//! Code-as-graph: build and query a symbol-level directed graph over Rust source.
//!
//! This module is the graph layer for the code surface (Block C, Task 2).
//!
//! # Architecture
//!
//! **Pure layer** (unit-tested):
//! - [`build_code_node_edge_lists`] maps `CodeSymbol`/`CodeRef` pairs from the
//!   extraction layer into `BrainNode`/`BrainEdge` lists consumable by `BrainGraph::build`.
//! - [`find_definition`] and [`find_references`] are plain slice queries.
//!
//! **Thin I/O shell** (`run_code`):
//! - Resolves the scan root from a workspace registry (DB-free).
//! - Walks `.rs` files, reads them, and delegates to the pure layer.
//! - Prints one greppable output line per result, mirroring `brain::run`.
//!
//! # From-id rule
//! Each `BrainEdge` produced by this module uses the **enclosing symbol's name** as
//! `from`. The "enclosing symbol" for a `CodeRef` at line L in file F is the last
//! symbol in F whose definition line is <= L (the innermost scope in a simple
//! top-down scan). Refs that precede all symbols in their file (e.g. module-level
//! `use` statements that appear before the first function) have no enclosing symbol
//! and are silently dropped — their `from` cannot resolve to a known symbol node.
//!
//! # Coverage note
//! Extraction scope: **Rust (.rs) files only**. Other languages in the scan root are
//! silently skipped by the file walker. Symbol kinds covered: Fn, Struct, Enum,
//! Trait, Mod, Impl.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::brain::code::{CodeRef, CodeSymbol};
use crate::brain::graph::BrainGraph;
use crate::brain::okf::{BrainEdge, BrainNode};
use crate::config::FileConfig;

// ── Query model ───────────────────────────────────────────────────────────────

/// The structural query to run against the code graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeQuery {
    /// Find the definition(s) of a named symbol.
    Def(String),
    /// Find all call sites / use imports for a named symbol.
    Refs(String),
    /// Find direct callers of a symbol (direct predecessors in the code graph).
    Dependents(String),
}

// ── Pure layer ────────────────────────────────────────────────────────────────

/// Build `BrainNode`/`BrainEdge` lists from extracted symbols and references.
///
/// Mapping rules:
/// - One `BrainNode` per `CodeSymbol`: `id = symbol.name`, `path = symbol.path`,
///   `title = symbol.name`.
/// - One `BrainEdge` per `CodeRef` that (a) has a known enclosing symbol in its
///   file and (b) references a symbol name that exists in the known-symbol set.
///   Refs to extern/std or unknown symbols are silently dropped.
///
/// **From-id rule:** the `from` id for a ref at line L in file F is the name of
/// the last symbol defined in F at or before line L. Refs with no preceding
/// symbol in their file are dropped (no resolvable from-id).
pub fn build_code_node_edge_lists(
    symbols: &[CodeSymbol],
    refs: &[CodeRef],
) -> (Vec<BrainNode>, Vec<BrainEdge>) {
    // Build nodes: one per symbol.
    let nodes: Vec<BrainNode> = symbols
        .iter()
        .map(|s| BrainNode {
            id: s.name.clone(),
            title: s.name.clone(),
            path: s.path.clone(),
        })
        .collect();

    // Set of known symbol names (for fast lookup when resolving ref targets).
    let known_ids: HashSet<&str> = symbols.iter().map(|s| s.name.as_str()).collect();

    // Build a per-file index: path → sorted Vec<(line, name)> for fast "enclosing
    // symbol" lookup. We sort ascending by line so we can binary-search.
    let mut file_symbol_index: std::collections::HashMap<&Path, Vec<(usize, &str)>> =
        std::collections::HashMap::new();
    for sym in symbols {
        file_symbol_index
            .entry(sym.path.as_path())
            .or_default()
            .push((sym.line, sym.name.as_str()));
    }
    for entries in file_symbol_index.values_mut() {
        entries.sort_by_key(|(line, _)| *line);
    }

    // Build edges: one per ref that resolves both its enclosing symbol (from) and
    // its target name (to). Deduplicate (from, to) pairs per the OKF convention.
    let mut seen_edges: HashSet<(String, String)> = HashSet::new();
    let mut edges: Vec<BrainEdge> = Vec::new();

    for r in refs {
        // Resolve `to`: must be a known symbol name.
        if !known_ids.contains(r.name.as_str()) {
            continue;
        }

        // Resolve `from`: last symbol in the same file with line <= ref.line.
        let enclosing = file_symbol_index.get(r.path.as_path()).and_then(|syms| {
            // Find the last entry with line <= r.line.
            let pos = syms.partition_point(|(line, _)| *line <= r.line);
            if pos == 0 {
                None // No symbol precedes this ref in its file.
            } else {
                Some(syms[pos - 1].1)
            }
        });

        let from = match enclosing {
            Some(name) => name,
            None => continue, // No enclosing symbol — drop the ref.
        };

        // Avoid self-loops and duplicate edges.
        if from == r.name.as_str() {
            continue;
        }
        let key = (from.to_string(), r.name.clone());
        if seen_edges.insert(key) {
            edges.push(BrainEdge {
                from: from.to_string(),
                to: r.name.clone(),
            });
        }
    }

    (nodes, edges)
}

/// Return all `CodeSymbol`s whose `name` matches `query_name`.
///
/// In practice, symbol names should be unique within a codebase, but if multiple
/// definitions exist (e.g. trait impls in separate files), all are returned.
pub fn find_definition<'a>(symbols: &'a [CodeSymbol], query_name: &str) -> Vec<&'a CodeSymbol> {
    symbols.iter().filter(|s| s.name == query_name).collect()
}

/// Return all `CodeRef`s whose `name` matches `query_name` (call sites + use imports).
pub fn find_references<'a>(refs: &'a [CodeRef], query_name: &str) -> Vec<&'a CodeRef> {
    refs.iter().filter(|r| r.name == query_name).collect()
}

/// Format a definition result as a greppable output line.
///
/// Format: `def: <name>\t<path>:<line>`
pub fn format_def_line(sym: &CodeSymbol) -> String {
    format!("def: {}\t{}:{}", sym.name, sym.path.display(), sym.line)
}

/// Format a reference result as a greppable output line.
///
/// Format: `ref: <name>\t<path>:<line>`
pub fn format_ref_line(r: &CodeRef) -> String {
    format!("ref: {}\t{}:{}", r.name, r.path.display(), r.line)
}

/// Format a dependent (caller) result as a greppable output line.
///
/// Format: `dependent: <id>\t<path>`
pub fn format_dependent_line(node: &BrainNode) -> String {
    format!("dependent: {}\t{}", node.id, node.path.display())
}

// ── File walker ───────────────────────────────────────────────────────────────

/// Walk `root` recursively and return all `.rs` files in deterministic (sorted) order.
///
/// Skip rules (mirror `validate::find_markdown_files`):
/// - Hidden directories (name starts with `.`)
/// - The `target/` directory (Rust build artefacts — typically large and irrelevant)
pub fn find_rust_files(root: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = Vec::new();
    collect_rs_files(root, &mut files);
    files.sort();
    files
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut children: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    children.sort();

    for path in children {
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || name == "target" {
                continue;
            }
            collect_rs_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

// ── I/O shell ─────────────────────────────────────────────────────────────────

/// Entry point for `bastion code`.
///
/// Thin I/O shell: resolves the scan root from the workspace registry (DB-free),
/// walks `.rs` files under that root, reads them (skipping unreadable files with
/// a stderr warning), runs extraction and graph construction, dispatches the query,
/// and prints a greppable report.
///
/// Graceful degradation mirrors `brain::run`:
/// - Unknown workspace name → clear error, non-zero exit.
/// - Unreadable root → clear error, non-zero exit.
/// - Individual unreadable files → skipped with a warning on stderr.
/// - Unknown symbol name → `# no <mode> results for '<name>'`.
pub fn run_code(
    query: CodeQuery,
    explicit_root: Option<PathBuf>,
    workspace: Option<String>,
    registry: &FileConfig,
) -> Result<()> {
    use crate::brain::code::{extract_refs, extract_symbols};

    // Resolve scan root (no DB, no Config::load).
    let root = crate::config::resolve_workspace_root(explicit_root, workspace.as_deref(), registry)
        .map_err(anyhow::Error::from)?;

    // Discover .rs files.
    let files = find_rust_files(&root);
    if files.is_empty() {
        eprintln!(
            "code: no .rs files found under '{}' — check --root or --workspace",
            root.display()
        );
        anyhow::bail!("empty source tree at '{}'", root.display());
    }

    // Read files; skip unreadable with a warning.
    let mut sources: Vec<(PathBuf, String)> = Vec::new();
    for file in &files {
        match std::fs::read_to_string(file) {
            Ok(content) => sources.push((file.clone(), content)),
            Err(e) => {
                eprintln!("code: skipping unreadable file '{}': {e}", file.display());
            }
        }
    }

    // Extract symbols and refs from all files.
    let mut all_symbols: Vec<CodeSymbol> = Vec::new();
    let mut all_refs: Vec<CodeRef> = Vec::new();
    for (path, content) in &sources {
        all_symbols.extend(extract_symbols(content, path));
        all_refs.extend(extract_refs(content, path));
    }

    // Build the graph for dependents queries.
    let (nodes, edges) = build_code_node_edge_lists(&all_symbols, &all_refs);
    let graph = BrainGraph::build(nodes, edges);

    match &query {
        CodeQuery::Def(name) => {
            let defs = find_definition(&all_symbols, name);
            if defs.is_empty() {
                println!("# no def results for '{name}'");
            } else {
                for sym in defs {
                    println!("{}", format_def_line(sym));
                }
            }
        }
        CodeQuery::Refs(name) => {
            let references = find_references(&all_refs, name);
            if references.is_empty() {
                println!("# no ref results for '{name}'");
            } else {
                for r in references {
                    println!("{}", format_ref_line(r));
                }
            }
        }
        CodeQuery::Dependents(name) => {
            match graph.predecessors(name) {
                Ok(callers) => {
                    if callers.is_empty() {
                        println!("# no dependent results for '{name}'");
                    } else {
                        for node in &callers {
                            println!("{}", format_dependent_line(node));
                        }
                    }
                }
                Err(_) => {
                    // Unknown symbol in the graph.
                    println!("# no dependent results for '{name}'");
                }
            }
        }
    }

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use crate::brain::code::{extract_refs, extract_symbols};

    // Fixture files (same fixtures as code.rs tests).
    //
    // Topology (symbols and edges the pure layer must produce):
    //
    //   Nodes (one per symbol):
    //     alpha         → lib.rs.fixture line 1
    //     beta          → lib.rs.fixture line 3
    //     Widget        → lib.rs.fixture line 5  (Struct)
    //     Widget        → lib.rs.fixture line 9  (Impl)   ← duplicate name, two nodes
    //     render        → lib.rs.fixture line 10
    //     main_consumer → consumer.rs.fixture line 5
    //     isolated_helper → util.rs.fixture line 1
    //
    //   Edges (enclosing-symbol rule):
    //     consumer.rs has refs:
    //       line 1: use alpha     → before any symbol in that file → DROPPED
    //       line 2: use beta      → before any symbol               → DROPPED
    //       line 3: use Widget    → before any symbol               → DROPPED
    //       line 6: call alpha    → enclosing = main_consumer (line 5) → edge main_consumer→alpha
    //       line 7: call beta     → enclosing = main_consumer         → edge main_consumer→beta
    //       line 10: call render  → enclosing = main_consumer         → edge main_consumer→render
    //
    //   Result: 3 edges; use-statement refs dropped (no enclosing symbol).
    //
    //   isolated_helper has no callers → no dependents.
    //   Widget appears twice (Struct + Impl) — both are included as nodes; dependents
    //   queries hit on the first match found by BrainGraph.

    const LIB_PATH: &str = "lib.rs";
    const CONSUMER_PATH: &str = "consumer.rs";
    const UTIL_PATH: &str = "util.rs";

    const LIB_FIXTURE: &str = include_str!("fixtures/code/lib.rs.fixture");
    const CONSUMER_FIXTURE: &str = include_str!("fixtures/code/consumer.rs.fixture");
    const UTIL_FIXTURE: &str = include_str!("fixtures/code/util.rs.fixture");

    /// Extract all symbols and refs from the three fixture files.
    fn fixture_symbols_and_refs() -> (Vec<CodeSymbol>, Vec<CodeRef>) {
        let mut syms = Vec::new();
        let mut refs = Vec::new();
        syms.extend(extract_symbols(LIB_FIXTURE, Path::new(LIB_PATH)));
        syms.extend(extract_symbols(CONSUMER_FIXTURE, Path::new(CONSUMER_PATH)));
        syms.extend(extract_symbols(UTIL_FIXTURE, Path::new(UTIL_PATH)));
        refs.extend(extract_refs(LIB_FIXTURE, Path::new(LIB_PATH)));
        refs.extend(extract_refs(CONSUMER_FIXTURE, Path::new(CONSUMER_PATH)));
        refs.extend(extract_refs(UTIL_FIXTURE, Path::new(UTIL_PATH)));
        (syms, refs)
    }

    // ── build_code_node_edge_lists: node count ────────────────────────────────

    #[test]
    fn node_count_matches_symbol_count() {
        let (syms, refs) = fixture_symbols_and_refs();
        let total_syms = syms.len();
        let (nodes, _) = build_code_node_edge_lists(&syms, &refs);
        assert_eq!(
            nodes.len(),
            total_syms,
            "one node per symbol: expected {total_syms}, got {}",
            nodes.len()
        );
    }

    #[test]
    fn nodes_include_alpha() {
        let (syms, refs) = fixture_symbols_and_refs();
        let (nodes, _) = build_code_node_edge_lists(&syms, &refs);
        assert!(
            nodes.iter().any(|n| n.id == "alpha"),
            "node 'alpha' must exist"
        );
    }

    #[test]
    fn nodes_include_main_consumer() {
        let (syms, refs) = fixture_symbols_and_refs();
        let (nodes, _) = build_code_node_edge_lists(&syms, &refs);
        assert!(
            nodes.iter().any(|n| n.id == "main_consumer"),
            "node 'main_consumer' must exist"
        );
    }

    #[test]
    fn nodes_include_isolated_helper() {
        let (syms, refs) = fixture_symbols_and_refs();
        let (nodes, _) = build_code_node_edge_lists(&syms, &refs);
        assert!(
            nodes.iter().any(|n| n.id == "isolated_helper"),
            "node 'isolated_helper' must exist"
        );
    }

    // ── build_code_node_edge_lists: edge topology ────────────────────────────

    #[test]
    fn edge_main_consumer_to_alpha() {
        let (syms, refs) = fixture_symbols_and_refs();
        let (_, edges) = build_code_node_edge_lists(&syms, &refs);
        assert!(
            edges
                .iter()
                .any(|e| e.from == "main_consumer" && e.to == "alpha"),
            "expected edge main_consumer→alpha; edges: {edges:?}"
        );
    }

    #[test]
    fn edge_main_consumer_to_beta() {
        let (syms, refs) = fixture_symbols_and_refs();
        let (_, edges) = build_code_node_edge_lists(&syms, &refs);
        assert!(
            edges
                .iter()
                .any(|e| e.from == "main_consumer" && e.to == "beta"),
            "expected edge main_consumer→beta; edges: {edges:?}"
        );
    }

    #[test]
    fn edge_main_consumer_to_render() {
        let (syms, refs) = fixture_symbols_and_refs();
        let (_, edges) = build_code_node_edge_lists(&syms, &refs);
        assert!(
            edges
                .iter()
                .any(|e| e.from == "main_consumer" && e.to == "render"),
            "expected edge main_consumer→render; edges: {edges:?}"
        );
    }

    #[test]
    fn use_statement_refs_do_not_produce_edges() {
        // use-statement refs appear at lines 1-3 in consumer, before main_consumer (line 5).
        // They have no enclosing symbol and must be dropped.
        let (syms, refs) = fixture_symbols_and_refs();
        let (_, edges) = build_code_node_edge_lists(&syms, &refs);
        // The only valid from-id for consumer refs is main_consumer.
        // If any edge has from != "main_consumer" from consumer.rs, that's unexpected.
        for edge in &edges {
            if edge.from != "main_consumer" {
                // Could be from lib.rs or util.rs symbols; should be none there.
                // Lib has no refs, util has no refs.
                panic!("unexpected edge from non-main_consumer: {edge:?}");
            }
        }
    }

    #[test]
    fn unresolved_ref_produces_no_edge() {
        // Create a ref to a symbol not in the symbol list → no edge.
        let syms = extract_symbols(LIB_FIXTURE, Path::new(LIB_PATH));
        let fake_ref = CodeRef {
            name: "extern_unknown".to_string(),
            path: PathBuf::from(LIB_PATH),
            line: 2,
        };
        let (_, edges) = build_code_node_edge_lists(&syms, &[fake_ref]);
        assert!(
            edges.is_empty(),
            "ref to unknown symbol must produce no edge"
        );
    }

    #[test]
    fn ref_with_no_enclosing_symbol_produces_no_edge() {
        // A ref at line 1 in a file with no symbol at line <= 1 must be dropped.
        let fake_sym = CodeSymbol {
            name: "my_fn".to_string(),
            kind: crate::brain::code::SymbolKind::Fn,
            path: PathBuf::from("a.rs"),
            line: 10, // symbol is at line 10
        };
        let fake_ref = CodeRef {
            name: "my_fn".to_string(), // target IS a known symbol
            path: PathBuf::from("a.rs"),
            line: 2, // ref is at line 2, BEFORE the symbol at line 10
        };
        let (_, edges) = build_code_node_edge_lists(&[fake_sym], &[fake_ref]);
        assert!(
            edges.is_empty(),
            "ref before any enclosing symbol must be dropped"
        );
    }

    #[test]
    fn duplicate_edges_are_deduplicated() {
        // Two refs to the same target from the same enclosing symbol should yield one edge.
        let syms = extract_symbols(LIB_FIXTURE, Path::new(LIB_PATH));
        let consumer_syms = extract_symbols(CONSUMER_FIXTURE, Path::new(CONSUMER_PATH));
        let all_syms: Vec<CodeSymbol> = syms.into_iter().chain(consumer_syms).collect();
        let refs = vec![
            CodeRef {
                name: "alpha".to_string(),
                path: PathBuf::from(CONSUMER_PATH),
                line: 6,
            },
            CodeRef {
                name: "alpha".to_string(),
                path: PathBuf::from(CONSUMER_PATH),
                line: 6, // exact duplicate
            },
        ];
        let (_, edges) = build_code_node_edge_lists(&all_syms, &refs);
        let mc_to_alpha = edges
            .iter()
            .filter(|e| e.from == "main_consumer" && e.to == "alpha")
            .count();
        assert_eq!(
            mc_to_alpha, 1,
            "duplicate ref must yield one edge, not {mc_to_alpha}"
        );
    }

    // ── find_definition ───────────────────────────────────────────────────────

    #[test]
    fn find_definition_returns_alpha() {
        let (syms, _) = fixture_symbols_and_refs();
        let defs = find_definition(&syms, "alpha");
        assert_eq!(defs.len(), 1, "exactly one def of alpha");
        assert_eq!(defs[0].path, PathBuf::from(LIB_PATH));
        assert_eq!(defs[0].line, 1);
    }

    #[test]
    fn find_definition_returns_main_consumer() {
        let (syms, _) = fixture_symbols_and_refs();
        let defs = find_definition(&syms, "main_consumer");
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].path, PathBuf::from(CONSUMER_PATH));
        assert_eq!(defs[0].line, 5);
    }

    #[test]
    fn find_definition_unknown_returns_empty() {
        let (syms, _) = fixture_symbols_and_refs();
        let defs = find_definition(&syms, "nonexistent");
        assert!(defs.is_empty(), "unknown symbol must return empty slice");
    }

    // ── find_references ───────────────────────────────────────────────────────

    #[test]
    fn find_references_alpha_returns_use_and_call() {
        let (_, refs) = fixture_symbols_and_refs();
        let r = find_references(&refs, "alpha");
        // Expect at least the use-statement (line 1) and the call (line 6).
        assert!(r.len() >= 2, "expected >= 2 refs to alpha, got {}", r.len());
    }

    #[test]
    fn find_references_unknown_returns_empty() {
        let (_, refs) = fixture_symbols_and_refs();
        let r = find_references(&refs, "nonexistent");
        assert!(r.is_empty());
    }

    // ── Graph-level: predecessors (direct dependents) ─────────────────────────

    #[test]
    fn predecessors_alpha_is_main_consumer() {
        let (syms, refs) = fixture_symbols_and_refs();
        let (nodes, edges) = build_code_node_edge_lists(&syms, &refs);
        let g = BrainGraph::build(nodes, edges);
        let preds = g.predecessors("alpha").expect("alpha must exist in graph");
        let ids: Vec<&str> = preds.iter().map(|n| n.id.as_str()).collect();
        assert!(
            ids.contains(&"main_consumer"),
            "main_consumer must be a direct caller of alpha; got {ids:?}"
        );
    }

    #[test]
    fn predecessors_beta_is_main_consumer() {
        let (syms, refs) = fixture_symbols_and_refs();
        let (nodes, edges) = build_code_node_edge_lists(&syms, &refs);
        let g = BrainGraph::build(nodes, edges);
        let preds = g.predecessors("beta").expect("beta must exist in graph");
        let ids: Vec<&str> = preds.iter().map(|n| n.id.as_str()).collect();
        assert!(
            ids.contains(&"main_consumer"),
            "main_consumer must call beta"
        );
    }

    #[test]
    fn isolated_helper_has_no_dependents() {
        let (syms, refs) = fixture_symbols_and_refs();
        let (nodes, edges) = build_code_node_edge_lists(&syms, &refs);
        let g = BrainGraph::build(nodes, edges);
        // isolated_helper may or may not be in the graph (it's a symbol node).
        // If present, its predecessors must be empty.
        if let Ok(preds) = g.predecessors("isolated_helper") {
            assert!(
                preds.is_empty(),
                "isolated_helper must have no callers; got {preds:?}"
            );
        }
    }

    #[test]
    fn reachable_reverse_render_includes_main_consumer() {
        let (syms, refs) = fixture_symbols_and_refs();
        let (nodes, edges) = build_code_node_edge_lists(&syms, &refs);
        let g = BrainGraph::build(nodes, edges);
        let blast = g
            .reachable_reverse("render")
            .expect("render must be in graph");
        let ids: Vec<&str> = blast.iter().map(|n| n.id.as_str()).collect();
        assert!(
            ids.contains(&"main_consumer"),
            "main_consumer is in the blast radius of render; got {ids:?}"
        );
    }

    // ── format helpers ────────────────────────────────────────────────────────

    #[test]
    fn format_def_line_shape() {
        use crate::brain::code::SymbolKind;
        let sym = CodeSymbol {
            name: "alpha".to_string(),
            kind: SymbolKind::Fn,
            path: PathBuf::from("lib.rs"),
            line: 1,
        };
        let line = format_def_line(&sym);
        assert_eq!(line, "def: alpha\tlib.rs:1");
    }

    #[test]
    fn format_ref_line_shape() {
        let r = CodeRef {
            name: "alpha".to_string(),
            path: PathBuf::from("consumer.rs"),
            line: 6,
        };
        let line = format_ref_line(&r);
        assert_eq!(line, "ref: alpha\tconsumer.rs:6");
    }

    #[test]
    fn format_dependent_line_shape() {
        let node = BrainNode {
            id: "main_consumer".to_string(),
            title: "main_consumer".to_string(),
            path: PathBuf::from("consumer.rs"),
        };
        let line = format_dependent_line(&node);
        assert_eq!(line, "dependent: main_consumer\tconsumer.rs");
    }

    // ── find_rust_files: basic behaviour ─────────────────────────────────────

    #[test]
    fn find_rust_files_empty_dir_returns_empty() {
        // Use a temp dir with no .rs files.
        let tmp = std::env::temp_dir().join("bastion_find_rust_files_test_empty");
        let _ = std::fs::create_dir_all(&tmp);
        let files = find_rust_files(&tmp);
        assert!(files.is_empty(), "empty dir must return no files");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn find_rust_files_result_is_sorted() {
        // src/ has many .rs files — result must be sorted.
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
        let files = find_rust_files(&src);
        let mut sorted = files.clone();
        sorted.sort();
        assert_eq!(files, sorted, "find_rust_files must return sorted paths");
    }

    #[test]
    fn find_rust_files_excludes_target_dir() {
        // Confirm no path contains "/target/" in the result from src/.
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
        let files = find_rust_files(&src);
        for f in &files {
            assert!(
                !f.components().any(|c| c.as_os_str() == "target"),
                "target/ must be excluded; got {f:?}"
            );
        }
    }
}
