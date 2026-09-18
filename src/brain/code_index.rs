//! OID-keyed sqlite cache for extracted code symbols and references.
//!
//! This module is the storage layer for the code-index cache (BA.ticket.code-index-cache,
//! task 2). It keys extracted `CodeSymbol`/`CodeRef` data on `(blob OID, language,
//! parser version)` so a `bastion code` query can answer from cache without re-parsing
//! unchanged blobs, at any commit, and self-heals (correct but slower) whenever the
//! cache is stale, missing, or partially populated.
//!
//! # Architecture
//!
//! **Pure layer** (unit-tested without a real git repo or sqlite file):
//! - [`parse_ls_tree_line`] / [`parse_ls_files_line`] parse one line of `git
//!   ls-tree`/`git ls-files -s` plumbing output into a `(path, blob_oid)` pair.
//! - [`symbol_kind_to_str`] / [`symbol_kind_from_str`] round-trip a [`SymbolKind`]
//!   through the cache's JSON storage columns.
//! - `encode_row` / `decode_row` turn `CodeSymbol`/`CodeRef` slices into the JSON
//!   strings actually written/read, separable from the `Connection` calls that do
//!   the writing/reading.
//!
//! **Thin I/O shell**:
//! - [`open_or_create_index`] opens (or creates) the sqlite file and its schema.
//! - [`cached_symbols`] / [`store_symbols`] read/write one cache row.
//! - [`assemble_blob_list`] shells out to `git` to list the blob set for the working
//!   tree, the index (`--staged`), or a historical `rev`.
//! - [`prune_unreachable`] deletes cache rows for blobs no ref can reach.
//! - [`index_stats`] reports row/blob counts for the `status` verb.
//!
//! # Self-healing
//! A missing, deleted, or corrupt index directory means every blob misses on
//! [`cached_symbols`] and the caller (task 3, `code_graph.rs`) falls back to parsing
//! it live and repopulating via [`store_symbols`] — correct, only slower.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::brain::code::{CodeRef, CodeSymbol, SymbolKind};

/// The extractor/parser version cache rows are keyed on, alongside `blob_oid`.
///
/// Bumping this constant orphans every previously cached row without a migration:
/// `cached_symbols` only matches rows whose stored `parser_version` equals this
/// value, so a bump makes every existing row silently stop matching — the next
/// query treats it as a miss, reparses live, and re-stores under the new version.
/// `prune_unreachable` then reclaims the orphaned rows once they are also
/// unreachable from any ref (see task 4's `--prune`).
pub const PARSER_VERSION: u32 = 1;

// ── Schema ──────────────────────────────────────────────────────────────────────

const SCHEMA_SQL: &str = "CREATE TABLE IF NOT EXISTS code_index (
    blob_oid TEXT NOT NULL,
    parser_version INTEGER NOT NULL,
    path TEXT NOT NULL,
    symbols_json TEXT NOT NULL,
    refs_json TEXT NOT NULL,
    PRIMARY KEY (blob_oid, parser_version)
)";

/// Cache row/blob counts for the `bastion code status` verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct IndexStats {
    /// Total cache rows across every `(blob_oid, parser_version)` pair.
    pub total_rows: usize,
    /// Distinct blob OIDs represented in the cache (may be indexed at more than
    /// one parser version each, e.g. mid-upgrade).
    pub distinct_blobs: usize,
}

// ── Pure: SymbolKind <-> storage string ──────────────────────────────────────────

/// Renders a [`SymbolKind`] as the lowercase string stored in the cache — mirrors
/// `SymbolKind`'s own `#[serde(rename_all = "lowercase")]` wire form exactly, kept
/// as a local, hand-written mapping rather than a `Deserialize` impl on
/// `SymbolKind` itself (out of this task's scope: `src/brain/code.rs`).
pub fn symbol_kind_to_str(kind: &SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Fn => "fn",
        SymbolKind::Struct => "struct",
        SymbolKind::Enum => "enum",
        SymbolKind::Trait => "trait",
        SymbolKind::Mod => "mod",
        SymbolKind::Impl => "impl",
        SymbolKind::Const => "const",
        SymbolKind::Static => "static",
        SymbolKind::TypeAlias => "typealias",
    }
}

/// Parses a stored kind string back into a [`SymbolKind`]. `None` on an unknown
/// string (a forward-incompatible cache row from a newer binary) — the caller
/// treats that the same as a cache miss.
pub fn symbol_kind_from_str(s: &str) -> Option<SymbolKind> {
    Some(match s {
        "fn" => SymbolKind::Fn,
        "struct" => SymbolKind::Struct,
        "enum" => SymbolKind::Enum,
        "trait" => SymbolKind::Trait,
        "mod" => SymbolKind::Mod,
        "impl" => SymbolKind::Impl,
        "const" => SymbolKind::Const,
        "static" => SymbolKind::Static,
        "typealias" => SymbolKind::TypeAlias,
        _ => return None,
    })
}

// ── Pure: JSON row encoding ───────────────────────────────────────────────────────

/// The JSON-serializable mirror of [`CodeSymbol`] actually written to/read from
/// the `symbols_json` column (`CodeSymbol` itself derives neither `Serialize` nor
/// `Deserialize` and is out of this task's file scope).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredSymbol {
    name: String,
    kind: String,
    path: String,
    line: usize,
}

/// The JSON-serializable mirror of [`CodeRef`] actually written to/read from the
/// `refs_json` column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredRef {
    name: String,
    path: String,
    line: usize,
}

/// Converts extracted symbols/refs into the JSON strings a cache row stores.
/// Pure — no I/O, no `Connection`. Unknown `SymbolKind` values cannot occur here
/// (every variant round-trips via [`symbol_kind_to_str`]), so this never fails.
fn encode_row(symbols: &[CodeSymbol], refs: &[CodeRef]) -> (String, String) {
    let stored_symbols: Vec<StoredSymbol> = symbols
        .iter()
        .map(|s| StoredSymbol {
            name: s.name.clone(),
            kind: symbol_kind_to_str(&s.kind).to_string(),
            path: s.path.display().to_string(),
            line: s.line,
        })
        .collect();
    let stored_refs: Vec<StoredRef> = refs
        .iter()
        .map(|r| StoredRef {
            name: r.name.clone(),
            path: r.path.display().to_string(),
            line: r.line,
        })
        .collect();
    (
        serde_json::to_string(&stored_symbols).unwrap_or_else(|_| "[]".to_string()),
        serde_json::to_string(&stored_refs).unwrap_or_else(|_| "[]".to_string()),
    )
}

/// Decodes a cache row's JSON columns back into `CodeSymbol`/`CodeRef` vectors.
/// A `StoredSymbol` whose `kind` string does not resolve via
/// [`symbol_kind_from_str`] is dropped rather than failing the whole row — this
/// can only happen if a future binary adds a `SymbolKind` variant this one does
/// not know, and dropping is strictly safer than misclassifying it.
fn decode_row(symbols_json: &str, refs_json: &str) -> (Vec<CodeSymbol>, Vec<CodeRef>) {
    let stored_symbols: Vec<StoredSymbol> = serde_json::from_str(symbols_json).unwrap_or_default();
    let stored_refs: Vec<StoredRef> = serde_json::from_str(refs_json).unwrap_or_default();

    let symbols = stored_symbols
        .into_iter()
        .filter_map(|s| {
            let kind = symbol_kind_from_str(&s.kind)?;
            Some(CodeSymbol {
                name: s.name,
                kind,
                path: PathBuf::from(s.path),
                line: s.line,
            })
        })
        .collect();

    let refs = stored_refs
        .into_iter()
        .map(|r| CodeRef {
            name: r.name,
            path: PathBuf::from(r.path),
            line: r.line,
        })
        .collect();

    (symbols, refs)
}

// ── Pure: git plumbing line parsing ───────────────────────────────────────────────

/// Parses one line of `git ls-tree -r <rev>` output (`"<mode> <type> <oid>\t<path>"`)
/// into `(path, blob_oid)`. Returns `None` for a non-blob entry (e.g. a `commit`
/// entry for a submodule) or a malformed line.
pub fn parse_ls_tree_line(line: &str) -> Option<(PathBuf, String)> {
    let (meta, path) = line.split_once('\t')?;
    let mut fields = meta.split_whitespace();
    let _mode = fields.next()?;
    let obj_type = fields.next()?;
    let oid = fields.next()?;
    if obj_type != "blob" {
        return None;
    }
    if path.is_empty() || oid.is_empty() {
        return None;
    }
    Some((PathBuf::from(path), oid.to_string()))
}

/// Parses one line of `git ls-files -s` output
/// (`"<mode> <oid> <stage>\t<path>"`) into `(path, blob_oid)`.
pub fn parse_ls_files_line(line: &str) -> Option<(PathBuf, String)> {
    let (meta, path) = line.split_once('\t')?;
    let mut fields = meta.split_whitespace();
    let _mode = fields.next()?;
    let oid = fields.next()?;
    let _stage = fields.next()?;
    if path.is_empty() || oid.is_empty() {
        return None;
    }
    Some((PathBuf::from(path), oid.to_string()))
}

// ── I/O shell: schema + row access ────────────────────────────────────────────────

/// Opens the sqlite index at `db_path`, creating the file, its parent directory,
/// and the schema if any of them are absent. Idempotent — a second call against
/// the same path is a no-op beyond opening the connection (`CREATE TABLE IF NOT
/// EXISTS` never resets existing data).
pub fn open_or_create_index(db_path: &Path) -> rusqlite::Result<Connection> {
    if let Some(parent) = db_path.parent() {
        // Best-effort: if this fails, `Connection::open` below will fail too and
        // surface the real error.
        let _ = std::fs::create_dir_all(parent);
    }
    let conn = Connection::open(db_path)?;
    conn.execute(SCHEMA_SQL, [])?;
    Ok(conn)
}

/// Looks up a cache row for `(blob_oid, parser_version)`. `None` on a miss —
/// either the blob has never been indexed, or it was indexed at a different
/// `parser_version` (a version bump orphans old rows: they simply stop matching
/// and eventually get pruned).
pub fn cached_symbols(
    conn: &Connection,
    blob_oid: &str,
    parser_version: u32,
) -> Option<(Vec<CodeSymbol>, Vec<CodeRef>)> {
    conn.query_row(
        "SELECT symbols_json, refs_json FROM code_index WHERE blob_oid = ?1 AND parser_version = ?2",
        params![blob_oid, parser_version],
        |row| {
            let symbols_json: String = row.get(0)?;
            let refs_json: String = row.get(1)?;
            Ok((symbols_json, refs_json))
        },
    )
    .ok()
    .map(|(symbols_json, refs_json)| decode_row(&symbols_json, &refs_json))
}

/// Stores (or replaces) the cache row for `(blob_oid, parser_version)`.
pub fn store_symbols(
    conn: &Connection,
    blob_oid: &str,
    parser_version: u32,
    path: &Path,
    symbols: &[CodeSymbol],
    refs: &[CodeRef],
) -> rusqlite::Result<()> {
    let (symbols_json, refs_json) = encode_row(symbols, refs);
    conn.execute(
        "INSERT OR REPLACE INTO code_index (blob_oid, parser_version, path, symbols_json, refs_json)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            blob_oid,
            parser_version,
            path.display().to_string(),
            symbols_json,
            refs_json
        ],
    )?;
    Ok(())
}

/// Deletes cache rows whose `blob_oid` is not present in `live_oids`. Returns the
/// number of rows deleted (not just distinct blobs — a blob cached at more than
/// one `parser_version` counts each row).
pub fn prune_unreachable(
    conn: &Connection,
    live_oids: &HashSet<String>,
) -> rusqlite::Result<usize> {
    let mut stmt = conn.prepare("SELECT DISTINCT blob_oid FROM code_index")?;
    let oids: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    drop(stmt);

    let mut deleted = 0usize;
    for oid in oids {
        if !live_oids.contains(&oid) {
            deleted += conn.execute("DELETE FROM code_index WHERE blob_oid = ?1", params![oid])?;
        }
    }
    Ok(deleted)
}

/// Reports cache row/blob counts for the `bastion code status` verb.
pub fn index_stats(conn: &Connection) -> rusqlite::Result<IndexStats> {
    let total_rows: usize =
        conn.query_row("SELECT COUNT(*) FROM code_index", [], |row| row.get(0))?;
    let distinct_blobs: usize = conn.query_row(
        "SELECT COUNT(DISTINCT blob_oid) FROM code_index",
        [],
        |row| row.get(0),
    )?;
    Ok(IndexStats {
        total_rows,
        distinct_blobs,
    })
}

// ── I/O shell: blob-list assembly via git plumbing ────────────────────────────────

/// Runs a `git` subcommand rooted at `repo_root` and returns its stdout as a
/// `String`. Returns an error (not a panic) on a non-zero exit or a `git`
/// binary that cannot be spawned at all.
fn run_git(repo_root: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .output()
        .map_err(|e| anyhow::anyhow!("failed to spawn git {:?}: {e}", args))?;
    if !output.status.success() {
        anyhow::bail!(
            "git {:?} failed (exit {:?}): {}",
            args,
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Assembles the `(path, blob_oid)` list a query should assemble its graph from.
///
/// - `rev` given: that commit's tree, via `git ls-tree -r <rev>` — answers for a
///   historical commit without checking it out.
/// - `staged` (and no `rev`): the git index's content, via `git ls-files -s` —
///   already gives each staged file's blob SHA directly, no hashing needed.
/// - neither: the working tree — tracked files' blob OIDs via `git ls-files -s`,
///   with each file `git diff --name-only` reports as dirty re-hashed via
///   `git hash-object` so its OID reflects working-tree content, not HEAD's.
pub fn assemble_blob_list(
    repo_root: &Path,
    rev: Option<&str>,
    staged: bool,
) -> anyhow::Result<Vec<(PathBuf, String)>> {
    if let Some(rev) = rev {
        let out = run_git(repo_root, &["ls-tree", "-r", rev])?;
        return Ok(out.lines().filter_map(parse_ls_tree_line).collect());
    }

    if staged {
        let out = run_git(repo_root, &["ls-files", "-s"])?;
        return Ok(out.lines().filter_map(parse_ls_files_line).collect());
    }

    // Working tree: start from the index's blob OIDs, then re-hash whatever
    // `git diff --name-only` reports as dirty against the working tree.
    let indexed = run_git(repo_root, &["ls-files", "-s"])?;
    let mut entries: Vec<(PathBuf, String)> =
        indexed.lines().filter_map(parse_ls_files_line).collect();

    let dirty = run_git(repo_root, &["diff", "--name-only"])?;
    let dirty_paths: HashSet<PathBuf> = dirty
        .lines()
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect();

    for (path, oid) in entries.iter_mut() {
        if dirty_paths.contains(path) {
            let abs = repo_root.join(&path);
            let hashed = run_git(
                repo_root,
                &["hash-object", abs.to_str().unwrap_or_default()],
            )?;
            let hashed = hashed.trim();
            if !hashed.is_empty() {
                *oid = hashed.to_string();
            }
        }
    }

    Ok(entries)
}

/// Reads the content of blob `oid` (at `path`, relative to `repo_root`) that a
/// query needs to parse on a cache miss.
///
/// Tries `git cat-file -p <oid>` first — this resolves for every blob already
/// written to the object database: every committed blob, and every staged blob
/// (`git add` writes the object immediately, before commit). Falls back to
/// reading `path` directly from the working tree when that fails, which is the
/// one case `assemble_blob_list` can produce an oid for that is **not** in the
/// object database: a dirty tracked file's oid comes from `git hash-object`
/// (no `-w`), which computes the hash without persisting the blob.
pub fn read_blob_content(repo_root: &Path, path: &Path, oid: &str) -> anyhow::Result<String> {
    if let Ok(content) = run_git(repo_root, &["cat-file", "-p", oid]) {
        return Ok(content);
    }
    let abs = repo_root.join(path);
    std::fs::read_to_string(&abs)
        .map_err(|e| anyhow::anyhow!("failed to read blob '{oid}' at '{}': {e}", abs.display()))
}

// ── Tests ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use tempfile::tempdir;

    // ── Pure: symbol kind round-trip ─────────────────────────────────────────────

    #[test]
    fn symbol_kind_round_trips_every_variant() {
        let variants = [
            SymbolKind::Fn,
            SymbolKind::Struct,
            SymbolKind::Enum,
            SymbolKind::Trait,
            SymbolKind::Mod,
            SymbolKind::Impl,
            SymbolKind::Const,
            SymbolKind::Static,
            SymbolKind::TypeAlias,
        ];
        for kind in variants {
            let s = symbol_kind_to_str(&kind);
            let back = symbol_kind_from_str(s).expect("must round-trip");
            assert_eq!(symbol_kind_to_str(&back), s);
        }
    }

    #[test]
    fn symbol_kind_from_str_unknown_is_none() {
        assert!(symbol_kind_from_str("bogus").is_none());
    }

    #[test]
    fn symbol_kind_typealias_matches_serde_rename() {
        // Must mirror SymbolKind's own #[serde(rename_all = "lowercase")].
        assert_eq!(symbol_kind_to_str(&SymbolKind::TypeAlias), "typealias");
    }

    // ── Pure: git plumbing line parsing ──────────────────────────────────────────

    #[test]
    fn parse_ls_tree_line_blob() {
        let line = "100644 blob e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\tsrc/main.rs";
        let (path, oid) = parse_ls_tree_line(line).unwrap();
        assert_eq!(path, PathBuf::from("src/main.rs"));
        assert_eq!(oid, "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
    }

    #[test]
    fn parse_ls_tree_line_non_blob_is_none() {
        let line = "160000 commit e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\tsub-module";
        assert!(parse_ls_tree_line(line).is_none());
    }

    #[test]
    fn parse_ls_tree_line_malformed_is_none() {
        assert!(parse_ls_tree_line("not a valid line").is_none());
        assert!(parse_ls_tree_line("").is_none());
    }

    #[test]
    fn parse_ls_files_line_ok() {
        let line = "100644 e69de29bb2d1d6434b8b29ae775ad8c2e48c5391 0\tsrc/lib.rs";
        let (path, oid) = parse_ls_files_line(line).unwrap();
        assert_eq!(path, PathBuf::from("src/lib.rs"));
        assert_eq!(oid, "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
    }

    #[test]
    fn parse_ls_files_line_malformed_is_none() {
        assert!(parse_ls_files_line("garbage").is_none());
    }

    // ── Pure: row encode/decode ───────────────────────────────────────────────────

    #[test]
    fn encode_decode_round_trips_symbols_and_refs() {
        let symbols = vec![CodeSymbol {
            name: "Foo".to_string(),
            kind: SymbolKind::Struct,
            path: PathBuf::from("src/foo.rs"),
            line: 10,
        }];
        let refs = vec![CodeRef {
            name: "Foo".to_string(),
            path: PathBuf::from("src/bar.rs"),
            line: 20,
        }];
        let (symbols_json, refs_json) = encode_row(&symbols, &refs);
        let (decoded_symbols, decoded_refs) = decode_row(&symbols_json, &refs_json);

        assert_eq!(decoded_symbols.len(), 1);
        assert_eq!(decoded_symbols[0].name, "Foo");
        assert_eq!(decoded_symbols[0].line, 10);
        assert_eq!(
            symbol_kind_to_str(&decoded_symbols[0].kind),
            symbol_kind_to_str(&SymbolKind::Struct)
        );
        assert_eq!(decoded_refs.len(), 1);
        assert_eq!(decoded_refs[0].name, "Foo");
        assert_eq!(decoded_refs[0].line, 20);
    }

    #[test]
    fn decode_row_drops_unknown_kind_symbol() {
        let symbols_json = r#"[{"name":"X","kind":"bogus","path":"a.rs","line":1}]"#;
        let refs_json = "[]";
        let (symbols, refs) = decode_row(symbols_json, refs_json);
        assert!(symbols.is_empty());
        assert!(refs.is_empty());
    }

    // ── I/O: schema + row access ──────────────────────────────────────────────────

    #[test]
    fn open_or_create_index_is_idempotent() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("nested").join("index.sqlite");

        let conn1 = open_or_create_index(&db_path).unwrap();
        store_symbols(
            &conn1,
            "oid1",
            1,
            Path::new("a.rs"),
            &[CodeSymbol {
                name: "A".to_string(),
                kind: SymbolKind::Fn,
                path: PathBuf::from("a.rs"),
                line: 1,
            }],
            &[],
        )
        .unwrap();
        drop(conn1);

        // Second open against the same path must not reset existing data.
        let conn2 = open_or_create_index(&db_path).unwrap();
        let found = cached_symbols(&conn2, "oid1", 1);
        assert!(found.is_some());
    }

    #[test]
    fn store_then_cached_round_trips() {
        let dir = tempdir().unwrap();
        let conn = open_or_create_index(&dir.path().join("index.sqlite")).unwrap();

        let symbols = vec![CodeSymbol {
            name: "Widget".to_string(),
            kind: SymbolKind::Struct,
            path: PathBuf::from("src/widget.rs"),
            line: 5,
        }];
        let refs = vec![CodeRef {
            name: "Widget".to_string(),
            path: PathBuf::from("src/main.rs"),
            line: 42,
        }];

        store_symbols(
            &conn,
            "abc123",
            1,
            Path::new("src/widget.rs"),
            &symbols,
            &refs,
        )
        .unwrap();

        let (got_symbols, got_refs) = cached_symbols(&conn, "abc123", 1).unwrap();
        assert_eq!(got_symbols.len(), 1);
        assert_eq!(got_symbols[0].name, "Widget");
        assert_eq!(got_refs.len(), 1);
        assert_eq!(got_refs[0].name, "Widget");
    }

    #[test]
    fn cached_symbols_unknown_blob_is_none() {
        let dir = tempdir().unwrap();
        let conn = open_or_create_index(&dir.path().join("index.sqlite")).unwrap();
        assert!(cached_symbols(&conn, "nonexistent", 1).is_none());
    }

    #[test]
    fn cached_symbols_stale_parser_version_is_none() {
        let dir = tempdir().unwrap();
        let conn = open_or_create_index(&dir.path().join("index.sqlite")).unwrap();
        store_symbols(&conn, "abc123", 1, Path::new("a.rs"), &[], &[]).unwrap();

        // Same blob, different (bumped) parser_version: must miss.
        assert!(cached_symbols(&conn, "abc123", 2).is_none());
        // Original version still hits.
        assert!(cached_symbols(&conn, "abc123", 1).is_some());
    }

    #[test]
    fn prune_unreachable_deletes_only_dead_rows_and_counts_correctly() {
        let dir = tempdir().unwrap();
        let conn = open_or_create_index(&dir.path().join("index.sqlite")).unwrap();

        store_symbols(&conn, "live-oid", 1, Path::new("a.rs"), &[], &[]).unwrap();
        store_symbols(&conn, "dead-oid", 1, Path::new("b.rs"), &[], &[]).unwrap();
        store_symbols(&conn, "dead-oid", 2, Path::new("b.rs"), &[], &[]).unwrap();

        let mut live: HashSet<String> = HashSet::new();
        live.insert("live-oid".to_string());

        let deleted = prune_unreachable(&conn, &live).unwrap();
        // Both dead-oid rows (parser_version 1 and 2) are deleted.
        assert_eq!(deleted, 2);
        assert!(cached_symbols(&conn, "live-oid", 1).is_some());
        assert!(cached_symbols(&conn, "dead-oid", 1).is_none());
        assert!(cached_symbols(&conn, "dead-oid", 2).is_none());
    }

    #[test]
    fn index_stats_counts_rows_and_distinct_blobs() {
        let dir = tempdir().unwrap();
        let conn = open_or_create_index(&dir.path().join("index.sqlite")).unwrap();

        store_symbols(&conn, "oid-a", 1, Path::new("a.rs"), &[], &[]).unwrap();
        store_symbols(&conn, "oid-b", 1, Path::new("b.rs"), &[], &[]).unwrap();
        store_symbols(&conn, "oid-b", 2, Path::new("b.rs"), &[], &[]).unwrap();

        let stats = index_stats(&conn).unwrap();
        assert_eq!(stats.total_rows, 3);
        assert_eq!(stats.distinct_blobs, 2);
    }

    // ── I/O: assemble_blob_list against a real temp git repo ─────────────────────

    /// Runs a git command against `dir`, panicking with stderr on failure — a
    /// test-only helper, not part of the module's public surface.
    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("failed to spawn git");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn init_repo(dir: &Path) {
        git(dir, &["init", "-q"]);
        git(dir, &["config", "user.email", "test@example.com"]);
        git(dir, &["config", "user.name", "Test"]);
    }

    #[test]
    fn assemble_blob_list_working_tree_returns_tracked_files() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        init_repo(root);
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        std::fs::write(root.join("b.rs"), "fn b() {}\n").unwrap();
        git(root, &["add", "a.rs", "b.rs"]);
        git(root, &["commit", "-q", "-m", "init"]);

        let entries = assemble_blob_list(root, None, false).unwrap();
        let paths: HashSet<PathBuf> = entries.iter().map(|(p, _)| p.clone()).collect();
        assert!(paths.contains(&PathBuf::from("a.rs")));
        assert!(paths.contains(&PathBuf::from("b.rs")));
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn assemble_blob_list_working_tree_dirty_file_reflects_working_content() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        init_repo(root);
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        git(root, &["add", "a.rs"]);
        git(root, &["commit", "-q", "-m", "init"]);

        let entries_before = assemble_blob_list(root, None, false).unwrap();
        let oid_before = entries_before
            .iter()
            .find(|(p, _)| p == &PathBuf::from("a.rs"))
            .map(|(_, oid)| oid.clone())
            .unwrap();

        // Dirty the tracked file without staging/committing.
        std::fs::write(root.join("a.rs"), "fn a() { /* changed */ }\n").unwrap();

        let entries_after = assemble_blob_list(root, None, false).unwrap();
        let oid_after = entries_after
            .iter()
            .find(|(p, _)| p == &PathBuf::from("a.rs"))
            .map(|(_, oid)| oid.clone())
            .unwrap();

        assert_ne!(
            oid_before, oid_after,
            "dirty file's OID must reflect working-tree content, not HEAD's"
        );
    }

    #[test]
    fn assemble_blob_list_staged_reflects_index_not_working_tree() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        init_repo(root);
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        git(root, &["add", "a.rs"]);
        git(root, &["commit", "-q", "-m", "init"]);

        // Stage a change, then dirty the working tree further without re-staging.
        std::fs::write(root.join("a.rs"), "fn a() { /* staged */ }\n").unwrap();
        git(root, &["add", "a.rs"]);
        let staged_entries = assemble_blob_list(root, None, true).unwrap();
        let staged_oid = staged_entries
            .iter()
            .find(|(p, _)| p == &PathBuf::from("a.rs"))
            .map(|(_, oid)| oid.clone())
            .unwrap();

        std::fs::write(root.join("a.rs"), "fn a() { /* working tree only */ }\n").unwrap();
        let staged_entries_again = assemble_blob_list(root, None, true).unwrap();
        let staged_oid_again = staged_entries_again
            .iter()
            .find(|(p, _)| p == &PathBuf::from("a.rs"))
            .map(|(_, oid)| oid.clone())
            .unwrap();

        assert_eq!(
            staged_oid, staged_oid_again,
            "--staged must reflect the index, not further working-tree edits"
        );
    }

    #[test]
    fn assemble_blob_list_rev_returns_historical_tree_regardless_of_worktree_state() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        init_repo(root);
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        git(root, &["add", "a.rs"]);
        git(root, &["commit", "-q", "-m", "init"]);
        let rev_out = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        let rev = String::from_utf8_lossy(&rev_out.stdout).trim().to_string();

        let historical = assemble_blob_list(root, Some(&rev), false).unwrap();
        let historical_oid = historical
            .iter()
            .find(|(p, _)| p == &PathBuf::from("a.rs"))
            .map(|(_, oid)| oid.clone())
            .unwrap();

        // Mutate the working tree after taking the rev; the historical answer
        // must be unaffected and must not require checking anything out.
        std::fs::write(root.join("a.rs"), "fn a() { /* mutated after rev */ }\n").unwrap();

        let historical_again = assemble_blob_list(root, Some(&rev), false).unwrap();
        let historical_oid_again = historical_again
            .iter()
            .find(|(p, _)| p == &PathBuf::from("a.rs"))
            .map(|(_, oid)| oid.clone())
            .unwrap();

        assert_eq!(historical_oid, historical_oid_again);

        // And it must differ from the now-dirty working tree's answer.
        let working = assemble_blob_list(root, None, false).unwrap();
        let working_oid = working
            .iter()
            .find(|(p, _)| p == &PathBuf::from("a.rs"))
            .map(|(_, oid)| oid.clone())
            .unwrap();
        assert_ne!(historical_oid, working_oid);
    }

    // ── read_blob_content ─────────────────────────────────────────────────────────

    #[test]
    fn read_blob_content_committed_blob_via_cat_file() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        init_repo(root);
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        git(root, &["add", "a.rs"]);
        git(root, &["commit", "-q", "-m", "init"]);

        let entries = assemble_blob_list(root, None, false).unwrap();
        let (path, oid) = entries
            .iter()
            .find(|(p, _)| p == &PathBuf::from("a.rs"))
            .unwrap();

        let content = read_blob_content(root, path, oid).unwrap();
        assert_eq!(content, "fn a() {}\n");
    }

    #[test]
    fn read_blob_content_dirty_blob_falls_back_to_working_tree() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        init_repo(root);
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        git(root, &["add", "a.rs"]);
        git(root, &["commit", "-q", "-m", "init"]);

        // Dirty the file without staging — its hash-object oid is never written
        // to the object database, so `git cat-file -p` on it must fail and the
        // working-tree fallback must kick in.
        std::fs::write(root.join("a.rs"), "fn a() { /* dirty */ }\n").unwrap();
        let entries = assemble_blob_list(root, None, false).unwrap();
        let (path, oid) = entries
            .iter()
            .find(|(p, _)| p == &PathBuf::from("a.rs"))
            .unwrap();

        let content = read_blob_content(root, path, oid).unwrap();
        assert_eq!(content, "fn a() { /* dirty */ }\n");
    }
}
