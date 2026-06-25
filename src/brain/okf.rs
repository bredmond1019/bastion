// OKF corpus reader — pure, no I/O.
//
// Parses OKF-format markdown files into `BrainNode`/`BrainEdge` lists
// suitable for feeding into `BrainGraph::build`. All functions here are
// deterministic over their inputs and carry no filesystem or network calls.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

// ── Shared types ──────────────────────────────────────────────────────────────

/// A node in the brain graph, derived from a single OKF markdown document.
///
/// `id` is the stable slug: the OKF `doc_id` frontmatter field when present,
/// otherwise the filename stem. This matches the convention that `[[link]]` targets
/// use short stable slugs (doc_id or stem), not slugified full titles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrainNode {
    /// Stable identifier — OKF `doc_id` frontmatter field, falling back to filename stem.
    pub id: String,
    /// Human-readable title from OKF frontmatter `title` field, or the filename stem if absent.
    pub title: String,
    /// Absolute or relative path to the source file.
    pub path: PathBuf,
}

/// A directed edge in the brain graph: `from` node id references `to` node id
/// via a `[[link]]` in the source document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrainEdge {
    pub from: String,
    pub to: String,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Parse a single OKF document into a `BrainNode`.
///
/// The node `id` is the OKF `doc_id` frontmatter field — the short, stable kebab-case
/// slug that `[[link]]` targets are expected to match. Falls back to the filename stem
/// when `doc_id` is absent or the file has no valid frontmatter.
///
/// The `title` comes from the frontmatter `title` field, falling back to the filename stem.
///
/// Returns `None` only if `path` has no valid filename stem (practically impossible for
/// real paths).
pub fn parse_okf_node(content: &str, path: &Path) -> Option<BrainNode> {
    let stem = path.file_stem()?.to_string_lossy().to_string();

    let fm = crate::validate::frontmatter::parse_frontmatter(content);

    let id = fm
        .as_ref()
        .and_then(|f| f.fields.get("doc_id"))
        .map(|(v, _)| v.clone())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| stem.clone());

    let title = fm
        .as_ref()
        .and_then(|f| f.fields.get("title"))
        .map(|(v, _)| v.clone())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| stem.clone());

    Some(BrainNode {
        id,
        title,
        path: path.to_path_buf(),
    })
}

/// Extract all `[[link]]` and `[[link|alias]]` targets from `content`.
///
/// - `[[slug]]` → returns `"slug"`.
/// - `[[slug|alias]]` → returns `"slug"` (the target slug, not the display alias).
/// - Only the slug portion before `|` (if present) is returned.
/// - No deduplication — callers may see the same slug multiple times if it appears
///   more than once in the document.
pub fn extract_okf_links(content: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut remaining = content;

    while let Some(open) = remaining.find("[[") {
        let after_open = &remaining[open + 2..];
        if let Some(close) = after_open.find("]]") {
            let inner = &after_open[..close];
            // Strip alias (`[[slug|alias]]` → slug).
            let slug = inner.split('|').next().unwrap_or(inner).trim();
            if !slug.is_empty() {
                links.push(slug.to_string());
            }
            remaining = &after_open[close + 2..];
        } else {
            // Unclosed `[[` — stop scanning.
            break;
        }
    }

    links
}

/// Build node and edge lists from in-memory `(path, content)` pairs.
///
/// - Each `(path, content)` pair produces exactly one `BrainNode` (if parseable).
/// - `[[link]]` targets in the content become `BrainEdge { from: node.id, to: slug }`.
/// - Duplicate `[[link]]` targets within a single document are deduplicated — a
///   document referencing the same node twice produces one edge, not two.
/// - Edges whose `to` slug does not resolve to any known node id are silently dropped.
/// - Paths that fail to produce a node (no stem) are skipped.
///
/// Returns `(nodes, edges)`. `edges` only contains resolved, deduplicated references.
pub fn build_node_edge_lists(docs: &[(PathBuf, String)]) -> (Vec<BrainNode>, Vec<BrainEdge>) {
    // First pass: parse nodes and record path→id so the edge pass doesn't re-parse.
    let mut nodes: Vec<BrainNode> = Vec::new();
    let mut id_by_path: HashMap<&PathBuf, String> = HashMap::new();
    for (path, content) in docs {
        if let Some(node) = parse_okf_node(content, path) {
            id_by_path.insert(path, node.id.clone());
            nodes.push(node);
        }
    }

    // Borrow ids from already-parsed nodes — no clone needed for the lookup set.
    let known_ids: HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();

    // Second pass: extract edges using cached ids; deduplicate [[link]] targets per doc
    // so a document that references the same node twice produces only one edge.
    let mut edges: Vec<BrainEdge> = Vec::new();
    for (path, content) in docs {
        let from_id = match id_by_path.get(path) {
            Some(id) => id,
            None => continue,
        };

        let unique_targets: HashSet<String> = extract_okf_links(content).into_iter().collect();

        for target in unique_targets {
            if known_ids.contains(target.as_str()) {
                edges.push(BrainEdge {
                    from: from_id.clone(),
                    to: target,
                });
            }
        }
    }

    (nodes, edges)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // ── parse_okf_node ────────────────────────────────────────────────────────

    #[test]
    fn parse_node_doc_id_takes_priority_over_stem() {
        let content =
            "---\ntype: Decision\ndoc_id: d3\ntitle: D3 — Use petgraph\ndescription: Desc\n---\n";
        let path = PathBuf::from("planning/decisions/D3-use-petgraph.md");
        let node = parse_okf_node(content, &path).unwrap();
        assert_eq!(node.id, "d3", "doc_id overrides the filename stem");
        assert_eq!(node.title, "D3 — Use petgraph");
        assert_eq!(node.path, path);
    }

    #[test]
    fn parse_node_falls_back_to_stem_when_no_doc_id() {
        let content = "---\ntype: Decision\ntitle: D3 — Use petgraph\ndescription: Desc\n---\n";
        let path = PathBuf::from("planning/decisions/d3.md");
        let node = parse_okf_node(content, &path).unwrap();
        // No doc_id → id is the filename stem, not a slugified title.
        assert_eq!(node.id, "d3");
        assert_eq!(node.title, "D3 — Use petgraph");
        assert_eq!(node.path, path);
    }

    #[test]
    fn parse_node_falls_back_to_stem_when_no_frontmatter() {
        let content = "# No frontmatter\n\nJust body text.";
        let path = PathBuf::from("docs/d20.md");
        let node = parse_okf_node(content, &path).unwrap();
        assert_eq!(node.id, "d20");
        assert_eq!(node.title, "d20");
        assert_eq!(node.path, path);
    }

    #[test]
    fn parse_node_empty_content_falls_back_to_stem() {
        let path = PathBuf::from("notes/empty.md");
        let node = parse_okf_node("", &path).unwrap();
        assert_eq!(node.id, "empty");
        assert_eq!(node.title, "empty");
    }

    // ── extract_okf_links ─────────────────────────────────────────────────────

    #[test]
    fn extract_links_plain_slug() {
        let content = "See [[d20]] for details.";
        assert_eq!(extract_okf_links(content), vec!["d20"]);
    }

    #[test]
    fn extract_links_alias_form() {
        let content = "See [[d20|data contract]] for details.";
        assert_eq!(extract_okf_links(content), vec!["d20"]);
    }

    #[test]
    fn extract_links_multiple() {
        let content = "Refs: [[d20]] and [[d21|session surface]].";
        assert_eq!(extract_okf_links(content), vec!["d20", "d21"]);
    }

    #[test]
    fn extract_links_no_links() {
        let content = "No wiki links here, just plain [markdown](url).";
        assert_eq!(extract_okf_links(content), Vec::<String>::new());
    }

    #[test]
    fn extract_links_unclosed_bracket_stops() {
        let content = "Good: [[d20]] then bad: [[unclosed";
        assert_eq!(extract_okf_links(content), vec!["d20"]);
    }

    #[test]
    fn extract_links_empty_content() {
        assert_eq!(extract_okf_links(""), Vec::<String>::new());
    }

    #[test]
    fn extract_links_duplicate_slugs() {
        let content = "[[d20]] referenced again [[d20]].";
        assert_eq!(extract_okf_links(content), vec!["d20", "d20"]);
    }

    #[test]
    fn extract_links_empty_brackets_skipped() {
        // `[[]]` has an empty slug after trimming.
        let content = "Bad: [[]] good: [[d20]].";
        assert_eq!(extract_okf_links(content), vec!["d20"]);
    }

    // ── build_node_edge_lists ─────────────────────────────────────────────────

    fn make_doc(stem: &str, title: &str, body: &str) -> (PathBuf, String) {
        let content =
            format!("---\ntype: Decision\ntitle: {title}\ndescription: Test fixture.\n---\n{body}");
        (PathBuf::from(format!("fixtures/{stem}.md")), content)
    }

    fn make_doc_with_id(stem: &str, doc_id: &str, body: &str) -> (PathBuf, String) {
        let content = format!(
            "---\ntype: Decision\ndoc_id: {doc_id}\ntitle: Fixture {stem}\ndescription: Test fixture.\n---\n{body}"
        );
        (PathBuf::from(format!("fixtures/{stem}.md")), content)
    }

    #[test]
    fn build_single_node_no_edges() {
        let docs = vec![make_doc("d3", "D3 — petgraph", "No links here.")];
        let (nodes, edges) = build_node_edge_lists(&docs);
        assert_eq!(nodes.len(), 1);
        assert!(edges.is_empty());
    }

    #[test]
    fn build_resolved_edge_with_doc_id() {
        // doc_id on each node; [[link]] target matches doc_id, not stem.
        let docs = vec![
            make_doc_with_id("long-filename-d3", "d3", ""),
            make_doc_with_id("long-filename-d20", "d20", "References [[d3]]."),
        ];
        let (nodes, edges) = build_node_edge_lists(&docs);
        assert_eq!(nodes.len(), 2);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, "d20");
        assert_eq!(edges[0].to, "d3");
    }

    #[test]
    fn build_duplicate_links_produce_single_edge() {
        // A document referencing the same target twice must produce only one edge.
        let docs = vec![
            (
                PathBuf::from("f/a.md"),
                "[[b]] appears twice [[b]].".to_string(),
            ),
            (PathBuf::from("f/b.md"), "Leaf.".to_string()),
        ];
        let (nodes, edges) = build_node_edge_lists(&docs);
        assert_eq!(nodes.len(), 2);
        assert_eq!(
            edges.len(),
            1,
            "duplicate [[b]] must produce exactly one edge"
        );
        assert_eq!(edges[0].from, "a");
        assert_eq!(edges[0].to, "b");
    }

    #[test]
    fn build_resolved_edges_using_stem_fallback() {
        // Use docs with no frontmatter so ids fall back to filename stems.
        let docs = vec![
            (
                PathBuf::from("fixtures/d3.md"),
                "References [[d20]] for details.".to_string(),
            ),
            (PathBuf::from("fixtures/d20.md"), "Leaf node.".to_string()),
        ];
        let (nodes, edges) = build_node_edge_lists(&docs);
        assert_eq!(nodes.len(), 2);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, "d3");
        assert_eq!(edges[0].to, "d20");
    }

    #[test]
    fn build_unresolved_link_is_skipped() {
        let docs = vec![(
            PathBuf::from("fixtures/d3.md"),
            "References [[nonexistent]].".to_string(),
        )];
        let (nodes, edges) = build_node_edge_lists(&docs);
        assert_eq!(nodes.len(), 1);
        assert!(
            edges.is_empty(),
            "unresolved link should be silently dropped"
        );
    }

    #[test]
    fn build_empty_corpus() {
        let docs: Vec<(PathBuf, String)> = vec![];
        let (nodes, edges) = build_node_edge_lists(&docs);
        assert!(nodes.is_empty());
        assert!(edges.is_empty());
    }

    #[test]
    fn build_chain_three_nodes() {
        // d3 → d20 → d21, all by stem fallback
        let docs = vec![
            (
                PathBuf::from("f/d3.md"),
                "[[d20]] is the contract.".to_string(),
            ),
            (PathBuf::from("f/d20.md"), "Depends on [[d21]].".to_string()),
            (PathBuf::from("f/d21.md"), "Leaf.".to_string()),
        ];
        let (nodes, edges) = build_node_edge_lists(&docs);
        assert_eq!(nodes.len(), 3);
        assert_eq!(edges.len(), 2);
        let froms: Vec<&str> = edges.iter().map(|e| e.from.as_str()).collect();
        assert!(froms.contains(&"d3"));
        assert!(froms.contains(&"d20"));
    }

    #[test]
    fn build_isolated_node_produces_no_edges() {
        let docs = vec![
            (
                PathBuf::from("f/d3.md"),
                "[[d20]] is the contract.".to_string(),
            ),
            (PathBuf::from("f/d20.md"), "Depends on [[d21]].".to_string()),
            (
                PathBuf::from("f/isolated.md"),
                "No outgoing links.".to_string(),
            ),
        ];
        let (nodes, edges) = build_node_edge_lists(&docs);
        assert_eq!(nodes.len(), 3);
        // d3→d20 edge resolves; d20→d21 does not (d21 not in corpus).
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, "d3");
        assert_eq!(edges[0].to, "d20");
    }
}
