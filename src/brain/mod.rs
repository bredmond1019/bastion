// `bastion brain` — structural knowledge-graph queries over the OKF corpus.

pub mod graph;
pub mod okf;

use std::path::PathBuf;

use anyhow::Result;

use crate::brain::okf::BrainNode;
use crate::config::FileConfig;

// ── Query model ───────────────────────────────────────────────────────────────

/// The structural query to run against the brain corpus graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrainQuery {
    /// Direct dependents of the named node (incoming edges).
    Dependents(String),
    /// Transitive reverse reachability — everything affected by a change to the node.
    BlastRadius(String),
    /// Forward reachability — everything the node transitively references.
    Lineage(String),
}

// ── Pure helpers (unit-tested) ────────────────────────────────────────────────

/// Returns the greppable label prefix for output lines produced by this query.
pub fn query_label(query: &BrainQuery) -> &'static str {
    match query {
        BrainQuery::Dependents(_) => "dependent",
        BrainQuery::BlastRadius(_) => "blast-radius",
        BrainQuery::Lineage(_) => "lineage",
    }
}

/// Returns the target node id embedded in the query.
pub fn query_node_id(query: &BrainQuery) -> &str {
    match query {
        BrainQuery::Dependents(id) | BrainQuery::BlastRadius(id) | BrainQuery::Lineage(id) => id,
    }
}

/// Format a single result node as a greppable output line.
///
/// Format: `<label>: <id>\t<path>`
///
/// Each line is independently greppable by label (`grep "^dependent:"`) or by
/// node id (`grep "\td20"`).
pub fn format_result_line(label: &str, node: &BrainNode) -> String {
    format!("{}: {}\t{}", label, node.id, node.path.display())
}

// ── I/O shell ─────────────────────────────────────────────────────────────────

/// Entry point for `bastion brain`.
///
/// Thin I/O shell: resolves the effective corpus root from the workspace registry
/// (DB-free), discovers and reads markdown files, builds the OKF knowledge graph,
/// runs the requested query, and prints a greppable report.
///
/// Graceful degradation:
/// - Unknown workspace name → clear error message, non-zero exit.
/// - Unreadable root directory → prints a clear message and returns an error.
/// - Individual unreadable files → skipped with a warning on stderr.
/// - Unknown node id → prints a clear message and returns an error.
pub fn run(
    query: BrainQuery,
    explicit_root: Option<PathBuf>,
    workspace: Option<String>,
    registry: &FileConfig,
) -> Result<()> {
    // Pure: resolve the effective corpus root (no DB, no Config::load).
    let root = crate::config::resolve_workspace_root(explicit_root, workspace.as_deref(), registry)
        .map_err(anyhow::Error::from)?;

    // I/O: discover corpus files.
    let files = crate::validate::find_markdown_files(&root);
    if files.is_empty() {
        eprintln!(
            "brain: no markdown files found under '{}' — check --root or --workspace",
            root.display()
        );
        anyhow::bail!("empty corpus at '{}'", root.display());
    }

    // I/O: read files into (path, content) pairs; skip unreadable files with a warning.
    let mut docs: Vec<(PathBuf, String)> = Vec::new();
    for file in &files {
        match std::fs::read_to_string(file) {
            Ok(content) => docs.push((file.clone(), content)),
            Err(e) => {
                eprintln!("brain: skipping unreadable file '{}': {e}", file.display());
            }
        }
    }

    // Pure: build graph.
    let (nodes, edges) = okf::build_node_edge_lists(&docs);
    let g = graph::BrainGraph::build(nodes, edges);

    // Pure: run query.
    let label = query_label(&query);
    let node_id = query_node_id(&query);

    let results = match &query {
        BrainQuery::Dependents(id) => g.predecessors(id),
        BrainQuery::BlastRadius(id) => g.reachable_reverse(id),
        BrainQuery::Lineage(id) => g.reachable_forward(id),
    };

    match results {
        Ok(result_nodes) => {
            if result_nodes.is_empty() {
                println!("# no {label} results for '{node_id}'");
            } else {
                for node in &result_nodes {
                    println!("{}", format_result_line(label, node));
                }
            }
            Ok(())
        }
        Err(e) => anyhow::bail!("brain: {e}"),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_node(id: &str, path: &str) -> BrainNode {
        BrainNode {
            id: id.to_string(),
            title: id.to_string(),
            path: PathBuf::from(path),
        }
    }

    // ── query_label ───────────────────────────────────────────────────────────

    #[test]
    fn query_label_dependents() {
        assert_eq!(
            query_label(&BrainQuery::Dependents("d20".to_string())),
            "dependent"
        );
    }

    #[test]
    fn query_label_blast_radius() {
        assert_eq!(
            query_label(&BrainQuery::BlastRadius("d20".to_string())),
            "blast-radius"
        );
    }

    #[test]
    fn query_label_lineage() {
        assert_eq!(
            query_label(&BrainQuery::Lineage("d20".to_string())),
            "lineage"
        );
    }

    // ── query_node_id ─────────────────────────────────────────────────────────

    #[test]
    fn query_node_id_dependents() {
        assert_eq!(
            query_node_id(&BrainQuery::Dependents("d20".to_string())),
            "d20"
        );
    }

    #[test]
    fn query_node_id_blast_radius() {
        assert_eq!(
            query_node_id(&BrainQuery::BlastRadius("d21".to_string())),
            "d21"
        );
    }

    #[test]
    fn query_node_id_lineage() {
        assert_eq!(query_node_id(&BrainQuery::Lineage("d3".to_string())), "d3");
    }

    // ── format_result_line ────────────────────────────────────────────────────

    #[test]
    fn format_result_line_dependent() {
        let node = make_node("d20", "docs/decisions/d20.md");
        let line = format_result_line("dependent", &node);
        assert_eq!(line, "dependent: d20\tdocs/decisions/d20.md");
    }

    #[test]
    fn format_result_line_blast_radius() {
        let node = make_node("d3", "planning/decisions/d3.md");
        let line = format_result_line("blast-radius", &node);
        assert_eq!(line, "blast-radius: d3\tplanning/decisions/d3.md");
    }

    #[test]
    fn format_result_line_lineage() {
        let node = make_node("d4", "docs/d4.md");
        let line = format_result_line("lineage", &node);
        assert_eq!(line, "lineage: d4\tdocs/d4.md");
    }

    #[test]
    fn format_result_line_separates_fields_with_tab() {
        let node = make_node("abc", "some/path/abc.md");
        let line = format_result_line("dependent", &node);
        // Tab must separate the id+label part from the path.
        let parts: Vec<&str> = line.splitn(2, '\t').collect();
        assert_eq!(parts.len(), 2, "line must have exactly one tab separator");
        assert_eq!(parts[0], "dependent: abc");
        assert_eq!(parts[1], "some/path/abc.md");
    }
}
