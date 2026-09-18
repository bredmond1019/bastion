//! BA.ticket.code-index-cache task 5: binary-level contract tests for the
//! content-addressed code index (`bastion code index` / `query --json` /
//! `status`).
//!
//! Invokes the ACTUAL COMPILED BINARY (`CARGO_BIN_EXE_bastion`) as a separate
//! process against real temporary git repositories — the cache-hit/miss/
//! reparse behavior, the historical-`--rev` path, and the self-healing path
//! are only observable at that process boundary (they depend on `git`
//! plumbing, a real sqlite file on disk, and the CLI's own root resolution),
//! not from `src/brain/code_index.rs`'s or `src/brain/code_graph.rs`'s own
//! unit tests, which call the pure/thin-I/O functions directly and never go
//! through `clap` or `main`'s dispatch.
//!
//! Every scenario builds its own isolated temp git repo (`tempfile::tempdir`)
//! — no shared mutable fixture state between tests — and asserts through the
//! `status`/`query --json` JSON envelopes' counters (`hits`/`misses`/
//! `reparses`, `IndexStats`), never by parsing stderr/log output or by timing.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

// ── git test helpers (mirrors src/brain/code_index.rs's own test helpers) ──────

/// Runs a git command against `dir`, panicking with stderr on failure.
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

/// Runs a git command against `dir` and returns trimmed stdout, panicking on failure.
fn git_output(dir: &Path, args: &[&str]) -> String {
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
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn init_repo(dir: &Path) {
    git(dir, &["init", "-q"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    git(dir, &["config", "user.name", "Test"]);
}

fn write_file(dir: &Path, name: &str, content: &str) {
    std::fs::write(dir.join(name), content).expect("write fixture file");
}

fn commit_all(dir: &Path, msg: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", msg]);
}

/// Mirrors the binary's own default-path computation (`default_code_index_db_path`
/// in `src/main.rs`) exactly, so tests derive the same path the binary itself
/// resolves for a plain (non-worktree) repo, without depending on that private
/// function directly.
///
/// Since BA.ticket.code-index-cache task 6, every call site resolves the db
/// path through `resolve_code_index_db_path_for_repo`, which layers an
/// optional `[code]` config-table override on top of this same default — none
/// of these tests write a config file, so `registry.code` is always `None` and
/// the effective path always falls back to this default, keeping this helper
/// accurate.
fn db_path_for(root: &Path) -> PathBuf {
    let common = git_output(root, &["rev-parse", "--git-common-dir"]);
    let common_path = PathBuf::from(&common);
    let common_abs = if common_path.is_absolute() {
        common_path
    } else {
        root.join(common_path)
    };
    common_abs.join("bastion-code").join("index.sqlite")
}

// ── bastion CLI invocation helpers ──────────────────────────────────────────────

fn bastion_cmd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bastion"))
}

/// Runs `bastion code status --json --root <root>`, warming the cache as a
/// side effect (same as any other query would), and returns the parsed
/// `CodeStatusReport` JSON.
fn run_status_json(root: &Path) -> Value {
    let output = bastion_cmd()
        .args(["code", "status", "--json", "--root"])
        .arg(root)
        .output()
        .expect("failed to run bastion code status");
    assert!(
        output.status.success(),
        "bastion code status --json failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("status output must be valid JSON")
}

/// Runs `bastion code index --root <root>` (optionally `--rev`/`--staged`/
/// `--prune`) and returns the raw stdout (human-readable text — the `index`
/// verb has no `--json` form).
fn run_index(root: &Path, extra_args: &[&str]) -> String {
    let mut cmd = bastion_cmd();
    cmd.arg("code").arg("index").arg("--root").arg(root);
    cmd.args(extra_args);
    let output = cmd.output().expect("failed to run bastion code index");
    assert!(
        output.status.success(),
        "bastion code index failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Runs `bastion code query --json --root <root>` (optionally `--rev`), piping
/// `stdin_lines` (each already valid single-line JSON, no trailing newline) to
/// stdin, and returns one parsed JSON value per stdout line, in order.
fn run_query_batch(root: &Path, extra_args: &[&str], stdin_lines: &[&str]) -> Vec<Value> {
    let mut cmd = bastion_cmd();
    cmd.args(["code", "query", "--json", "--root"])
        .arg(root)
        .args(extra_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("failed to spawn bastion code query");
    {
        let stdin = child.stdin.as_mut().expect("query child has stdin");
        for line in stdin_lines {
            writeln!(stdin, "{line}").expect("write query line to stdin");
        }
    }
    let output = child
        .wait_with_output()
        .expect("failed to wait on bastion code query");
    assert!(
        output.status.success(),
        "bastion code query --json failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("each query result line must be valid JSON"))
        .collect()
}

fn as_usize(v: &Value, field: &str) -> usize {
    v.get(field)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("expected field '{field}' as u64 in {v:?}")) as usize
}

// ── Scenario 1: cache hit on a second identical query ───────────────────────────

#[test]
fn second_identical_query_hits_cache_with_zero_reparses() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    init_repo(root);
    write_file(root, "a.rs", "fn a() {}\n");
    commit_all(root, "init");

    let first = run_status_json(root);
    assert_eq!(
        as_usize(&first, "hits"),
        0,
        "cold cache must have zero hits"
    );
    assert_eq!(as_usize(&first, "misses"), 1);
    assert_eq!(as_usize(&first, "reparses"), 1);

    let second = run_status_json(root);
    assert_eq!(
        as_usize(&second, "hits"),
        1,
        "second identical query must answer from cache"
    );
    assert_eq!(
        as_usize(&second, "misses"),
        0,
        "second identical query must reparse zero unchanged blobs"
    );
    assert_eq!(as_usize(&second, "reparses"), 0);
}

// ── Scenario 2: editing one file reparses exactly that blob ─────────────────────

#[test]
fn editing_one_file_reparses_only_that_blob() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    init_repo(root);
    write_file(root, "a.rs", "fn a() {}\n");
    write_file(root, "b.rs", "fn b() {}\n");
    commit_all(root, "init");

    let warm = run_status_json(root);
    assert_eq!(as_usize(&warm, "misses"), 2, "both blobs start uncached");
    assert_eq!(as_usize(&warm, "reparses"), 2);

    // Edit exactly one tracked file, leaving it dirty (uncommitted).
    write_file(root, "a.rs", "fn a() { /* changed */ }\n");

    let after_edit = run_status_json(root);
    assert_eq!(
        as_usize(&after_edit, "hits"),
        1,
        "the untouched sibling blob (b.rs) must still hit"
    );
    assert_eq!(
        as_usize(&after_edit, "misses"),
        1,
        "only the edited blob (a.rs) must miss"
    );
    assert_eq!(as_usize(&after_edit, "reparses"), 1);
}

// ── Scenario 3: `--rev` answers a historical commit without checkout ────────────

#[test]
fn rev_query_answers_historical_commit_without_mutating_working_tree() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    init_repo(root);
    write_file(root, "a.rs", "fn foo() {}\n");
    commit_all(root, "add foo");
    let old_rev = git_output(root, &["rev-parse", "HEAD"]);

    write_file(root, "a.rs", "fn bar() {}\n");
    commit_all(root, "replace foo with bar");

    // Historical query at `old_rev`: foo must be found there.
    let historical = run_query_batch(root, &["--rev", &old_rev], &[r#"{"def":"foo"}"#]);
    assert_eq!(historical.len(), 1);
    let historical_results = historical[0]["results"]
        .as_array()
        .expect("results must be an array");
    assert_eq!(
        historical_results.len(),
        1,
        "foo must be found at the historical rev: {historical:?}"
    );

    // Current (HEAD) query: foo no longer exists.
    let current = run_query_batch(root, &[], &[r#"{"def":"foo"}"#]);
    assert_eq!(current.len(), 1);
    let current_results = current[0]["results"]
        .as_array()
        .expect("results must be an array");
    assert!(
        current_results.is_empty(),
        "foo must be absent at HEAD: {current:?}"
    );

    // The rev query must never have checked anything out — the working tree
    // still holds the post-replace content.
    let working_tree_content =
        std::fs::read_to_string(root.join("a.rs")).expect("read working tree file");
    assert_eq!(working_tree_content, "fn bar() {}\n");
}

// ── Scenario 4: bumping the parser version orphans old rows ─────────────────────

#[test]
fn stale_parser_version_row_is_orphaned_and_reparsed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    init_repo(root);
    write_file(root, "a.rs", "fn a() {}\n");
    commit_all(root, "init");

    // Warm the cache once.
    let warm = run_status_json(root);
    assert_eq!(as_usize(&warm, "misses"), 1);
    assert_eq!(as_usize(&warm, "reparses"), 1);

    // Directly corrupt the cached row's parser_version, simulating what a
    // parser-version bump does to every previously cached row: the row is
    // still present, but no longer matches the version a query looks up.
    let db_path = db_path_for(root);
    let conn = bastion::brain::code_index::open_or_create_index(&db_path)
        .expect("open the warmed index directly");
    let blob_list = bastion::brain::code_index::assemble_blob_list(root, None, false)
        .expect("assemble blob list");
    let (_path, oid) = blob_list
        .iter()
        .find(|(p, _)| p == &PathBuf::from("a.rs"))
        .expect("a.rs must be in the blob list");

    assert!(
        bastion::brain::code_index::cached_symbols(
            &conn,
            oid,
            bastion::brain::code_index::PARSER_VERSION
        )
        .is_some(),
        "row must be cached at the current parser version before corruption"
    );

    conn.execute(
        "UPDATE code_index SET parser_version = ?1 WHERE blob_oid = ?2",
        rusqlite::params![
            bastion::brain::code_index::PARSER_VERSION as i64 + 1000,
            oid
        ],
    )
    .expect("corrupt the row's parser_version");
    drop(conn);

    // The row no longer matches the current parser version...
    let conn2 =
        bastion::brain::code_index::open_or_create_index(&db_path).expect("reopen the index");
    assert!(
        bastion::brain::code_index::cached_symbols(
            &conn2,
            oid,
            bastion::brain::code_index::PARSER_VERSION
        )
        .is_none(),
        "a stale parser_version row must not match a current-version lookup"
    );
    drop(conn2);

    // ...so the next query treats it as a miss and reparses it.
    let after_corruption = run_status_json(root);
    assert_eq!(
        as_usize(&after_corruption, "hits"),
        0,
        "orphaned row must not count as a hit"
    );
    assert_eq!(as_usize(&after_corruption, "misses"), 1);
    assert_eq!(as_usize(&after_corruption, "reparses"), 1);

    // And it re-caches under the current version, so the next query hits again.
    let rewarmed = run_status_json(root);
    assert_eq!(as_usize(&rewarmed, "hits"), 1);
    assert_eq!(as_usize(&rewarmed, "misses"), 0);
}

// ── Scenario 5: `index --prune` removes rows unreachable from any ref ───────────

#[test]
fn prune_removes_rows_for_blobs_unreachable_from_any_ref() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    init_repo(root);
    write_file(root, "a.rs", "fn a() {}\n");
    commit_all(root, "init");
    let main_branch = git_output(root, &["rev-parse", "--abbrev-ref", "HEAD"]);

    // Introduce a unique blob on a throwaway branch, then delete the branch
    // (not just reset it) so the commit — and its unique blob — becomes
    // unreachable from any ref. `git rev-list --objects --all` (the live-OID
    // computation `--prune` uses) only walks refs, never the reflog, so this
    // is genuinely unreachable for that purpose.
    git(root, &["checkout", "-q", "-b", "throwaway"]);
    write_file(root, "unique.rs", "fn unique_fn() {}\n");
    commit_all(root, "unique blob");
    let unique_oid = git_output(root, &["rev-parse", "HEAD:unique.rs"]);

    // Warm the cache so the unique blob's row actually exists to be pruned.
    run_index(root, &[]);

    git(root, &["checkout", "-q", &main_branch]);
    git(root, &["branch", "-D", "throwaway"]);

    let db_path = db_path_for(root);
    let conn_before = bastion::brain::code_index::open_or_create_index(&db_path)
        .expect("open index before prune");
    assert!(
        bastion::brain::code_index::cached_symbols(
            &conn_before,
            &unique_oid,
            bastion::brain::code_index::PARSER_VERSION
        )
        .is_some(),
        "unique blob's row must exist before pruning"
    );
    drop(conn_before);

    let index_output = run_index(root, &["--prune"]);
    assert!(
        index_output.contains("pruned"),
        "expected a 'pruned N unreachable row(s)' line: {index_output}"
    );

    let conn_after =
        bastion::brain::code_index::open_or_create_index(&db_path).expect("open index after prune");
    assert!(
        bastion::brain::code_index::cached_symbols(
            &conn_after,
            &unique_oid,
            bastion::brain::code_index::PARSER_VERSION
        )
        .is_none(),
        "unreachable blob's row must be pruned"
    );

    // a.rs's blob is still reachable from `main_branch` and must survive.
    let a_blob_list = bastion::brain::code_index::assemble_blob_list(root, None, false)
        .expect("assemble blob list on main");
    let (_path, a_oid) = a_blob_list
        .iter()
        .find(|(p, _)| p == &PathBuf::from("a.rs"))
        .expect("a.rs must be in the blob list");
    assert!(
        bastion::brain::code_index::cached_symbols(
            &conn_after,
            a_oid,
            bastion::brain::code_index::PARSER_VERSION
        )
        .is_some(),
        "a.rs's still-reachable blob row must NOT be pruned"
    );
}

// ── Scenario 6: `query --json` batches many queries in one process ──────────────

#[test]
fn query_batch_returns_one_result_per_line_in_order() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    init_repo(root);
    write_file(root, "a.rs", "fn foo() {}\n");
    write_file(root, "b.rs", "fn bar() { foo(); }\n");
    commit_all(root, "init");

    let results = run_query_batch(
        root,
        &[],
        &[r#"{"def":"foo"}"#, r#"{"def":"bar"}"#, r#"{"refs":"foo"}"#],
    );

    assert_eq!(results.len(), 3, "one result line per stdin query line");

    assert_eq!(results[0]["query"], "def");
    assert_eq!(results[0]["name"], "foo");
    assert_eq!(
        results[0]["results"].as_array().expect("array").len(),
        1,
        "foo's def must be found: {:?}",
        results[0]
    );

    assert_eq!(results[1]["query"], "def");
    assert_eq!(results[1]["name"], "bar");
    assert_eq!(
        results[1]["results"].as_array().expect("array").len(),
        1,
        "bar's def must be found: {:?}",
        results[1]
    );

    assert_eq!(results[2]["query"], "refs");
    assert_eq!(results[2]["name"], "foo");
    assert_eq!(
        results[2]["results"].as_array().expect("array").len(),
        1,
        "foo's one call site (in bar) must be found: {:?}",
        results[2]
    );
}

// ── Scenario 7: self-healing after deleting the index mid-run ───────────────────

#[test]
fn deleting_the_index_directory_self_heals_on_the_next_query() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    init_repo(root);
    write_file(root, "a.rs", "fn a() {}\n");
    commit_all(root, "init");

    let warm = run_status_json(root);
    assert_eq!(as_usize(&warm, "misses"), 1);
    assert_eq!(as_usize(&warm, "reparses"), 1);
    assert_eq!(as_usize(&warm, "total_rows"), 1);

    // Delete the whole index directory (not just the file) mid-run.
    let db_path = db_path_for(root);
    let index_dir = db_path.parent().expect("db path has a parent dir");
    assert!(index_dir.exists(), "index dir must exist before deletion");
    std::fs::remove_dir_all(index_dir).expect("delete the index directory");
    assert!(!index_dir.exists());

    // The very next query must still answer correctly — self-healing,
    // never erroring — repopulating the cache from scratch.
    let healed = run_status_json(root);
    assert_eq!(
        as_usize(&healed, "hits"),
        0,
        "a freshly deleted index has nothing to hit"
    );
    assert_eq!(as_usize(&healed, "misses"), 1);
    assert_eq!(as_usize(&healed, "reparses"), 1);
    assert_eq!(as_usize(&healed, "total_rows"), 1);

    // And the cache is warm again for the query after that.
    let rewarmed = run_status_json(root);
    assert_eq!(as_usize(&rewarmed, "hits"), 1);
    assert_eq!(as_usize(&rewarmed, "misses"), 0);
}
