// OKF corpus reader — pure, no I/O.
//
// Parses OKF-format markdown files into `BrainNode`/`BrainEdge` lists
// suitable for feeding into `BrainGraph::build`. All functions here are
// deterministic over their inputs and carry no filesystem or network calls.

use std::path::{Path, PathBuf};

// ── Shared types ──────────────────────────────────────────────────────────────

/// A node in the brain graph, derived from a single OKF markdown document.
///
/// `id` is the stable slug (frontmatter `title` slug or filename stem).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrainNode {
    /// Stable identifier — slugified frontmatter title, falling back to filename stem.
    pub id: String,
    /// Human-readable title from OKF frontmatter, or the filename stem if absent.
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

// ── Pure helpers ──────────────────────────────────────────────────────────────

/// Derive a slug from a human-readable title: lowercase, spaces to hyphens,
/// drop any character that is not alphanumeric, `-`, or `_`.
fn slugify(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .map(|c| if c == ' ' { '-' } else { c })
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

/// Extract the `title` value from OKF YAML frontmatter in `content`.
///
/// Rules (mirrors `frontmatter.rs`):
/// - First line must be exactly `---`.
/// - Scans for a `title: <value>` key inside the block until the closing `---`.
/// - Returns `None` if the block is absent, unterminated, or has no `title` field.
fn extract_title_from_frontmatter(content: &str) -> Option<String> {
    let mut lines = content.lines();

    // First line must be the opening fence.
    match lines.next() {
        Some(line) if line.trim_end() == "---" => {}
        _ => return None,
    }

    // Collect the title candidate while scanning; only return it if the closing
    // fence is reached (unterminated fence → None).
    let mut found_title: Option<String> = None;

    for line in lines {
        let trimmed = line.trim_end();
        if trimmed == "---" {
            // Closing fence — return whatever we found (may be None).
            return found_title;
        }
        if found_title.is_none() {
            found_title = trimmed
                .strip_prefix("title:")
                .map(|rest| rest.trim().to_string())
                .filter(|v| !v.is_empty());
        }
    }

    // No closing fence found — unterminated block.
    None
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Parse a single OKF document into a `BrainNode`.
///
/// The node `id` is derived by slugifying the frontmatter `title` field.
/// If the frontmatter block is absent or has no `title`, the filename stem is used
/// as both `id` and `title`.
///
/// Returns `None` only if `path` has no valid filename stem (which is practically
/// impossible for real paths).
pub fn parse_okf_node(content: &str, path: &Path) -> Option<BrainNode> {
    let stem = path.file_stem()?.to_string_lossy().to_string();

    let (id, title) = match extract_title_from_frontmatter(content) {
        Some(raw_title) => (slugify(&raw_title), raw_title),
        None => (stem.clone(), stem),
    };

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
/// - Edges whose `to` slug does not resolve to any known node id are **skipped**
///   (unresolved link targets are silently dropped, not recorded as errors here).
/// - Paths that fail to produce a node (no stem) are skipped.
///
/// Returns `(nodes, edges)`. `edges` only contains resolved references.
pub fn build_node_edge_lists(docs: &[(PathBuf, String)]) -> (Vec<BrainNode>, Vec<BrainEdge>) {
    // First pass: build nodes and collect the id set.
    let mut nodes: Vec<BrainNode> = Vec::new();
    for (path, content) in docs {
        if let Some(node) = parse_okf_node(content, path) {
            nodes.push(node);
        }
    }

    let known_ids: std::collections::HashSet<String> = nodes.iter().map(|n| n.id.clone()).collect();

    // Second pass: extract edges, filtering to resolved targets.
    let mut edges: Vec<BrainEdge> = Vec::new();
    for (path, content) in docs {
        // Resolve the from-node id for this document.
        let from_id = match parse_okf_node(content, path) {
            Some(n) => n.id,
            None => continue,
        };

        for target in extract_okf_links(content) {
            if known_ids.contains(&target) {
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

    // ── slugify ───────────────────────────────────────────────────────────────

    #[test]
    fn slugify_lowercases_and_replaces_spaces() {
        assert_eq!(slugify("Hello World"), "hello-world");
    }

    #[test]
    fn slugify_strips_special_chars() {
        assert_eq!(slugify("D20 — Data Contract"), "d20--data-contract");
    }

    #[test]
    fn slugify_preserves_hyphens_and_underscores() {
        assert_eq!(slugify("d20_contract-v2"), "d20_contract-v2");
    }

    #[test]
    fn slugify_empty_string() {
        assert_eq!(slugify(""), "");
    }

    // ── extract_title_from_frontmatter ────────────────────────────────────────

    #[test]
    fn extract_title_present() {
        let content = "---\ntype: Decision\ntitle: My Title\ndescription: Desc\n---\n# Body";
        assert_eq!(
            extract_title_from_frontmatter(content),
            Some("My Title".to_string())
        );
    }

    #[test]
    fn extract_title_no_frontmatter() {
        let content = "# No frontmatter here";
        assert_eq!(extract_title_from_frontmatter(content), None);
    }

    #[test]
    fn extract_title_unterminated_fence() {
        let content = "---\ntitle: Unterminated\n# no closing fence";
        assert_eq!(extract_title_from_frontmatter(content), None);
    }

    #[test]
    fn extract_title_missing_field() {
        let content = "---\ntype: Decision\ndescription: No title here\n---\n";
        assert_eq!(extract_title_from_frontmatter(content), None);
    }

    #[test]
    fn extract_title_empty_value() {
        let content = "---\ntitle: \ndescription: Desc\n---\n";
        assert_eq!(extract_title_from_frontmatter(content), None);
    }

    // ── parse_okf_node ────────────────────────────────────────────────────────

    #[test]
    fn parse_node_with_frontmatter_title() {
        let content = "---\ntype: Decision\ntitle: D3 — Use petgraph\ndescription: Desc\n---\n";
        let path = PathBuf::from("planning/decisions/d3.md");
        let node = parse_okf_node(content, &path).unwrap();
        assert_eq!(node.title, "D3 — Use petgraph");
        // id is slugified title
        assert!(node.id.starts_with("d3"));
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

    #[test]
    fn build_single_node_no_edges() {
        let docs = vec![make_doc("d3", "D3 — petgraph", "No links here.")];
        let (nodes, edges) = build_node_edge_lists(&docs);
        assert_eq!(nodes.len(), 1);
        assert!(edges.is_empty());
    }

    #[test]
    fn build_two_nodes_one_edge() {
        let docs = vec![
            make_doc("d3", "D3", "References [[d3--petgraph]]."), // self-ref slug test
            make_doc("d20", "D20 — Contract", "References [[d3--petgraph]]."),
        ];
        // d3's id = slugify("D3") = "d3"
        // d20's id = slugify("D20 — Contract") ≈ "d20--contract"
        // "d3--petgraph" is not a known id → edges dropped (unresolved)
        let (nodes, edges) = build_node_edge_lists(&docs);
        assert_eq!(nodes.len(), 2);
        // No resolved edges because "d3--petgraph" is not a node id.
        assert!(edges.is_empty());
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
