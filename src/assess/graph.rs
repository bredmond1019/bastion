//! Graph-readiness family for `bastion assess` (Phase 15, Block BA.15.9, task 4).
//!
//! Pure over already-read `(path, content)` pairs — no filesystem access happens here.
//! `okf-core` owns the graph *shape* (`GraphArtifact`, `Node`, `Edge`) and the
//! resolution primitive (`resolve_edge`) but not construction — mev builds its own
//! over a `Corpus`, and `assess` builds its own here over the markdown files
//! `assess::discover::resolve_context` found.
//!
//! Construction mirrors mev's `build_graph`: a file with a non-empty `doc_id` becomes
//! a [`okf_core::graph::Node`] (`id` = `scope:doc_id`); a file without one is a leaf,
//! tracked by `scope:<file stem>` in `leaf_keys` so a `related:` entry pointing at it
//! resolves to `LeafTarget` rather than `Dangling`. Every `related:` entry on a node
//! becomes an `Edge`, classified via `resolve_edge`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use okf_core::{
    Edge, EdgeKind, EdgeResolution, Graph, GraphArtifact, Node, ParseResult, extract_frontmatter,
    resolve_edge,
};
use serde::{Deserialize, Serialize};

/// A `related:` edge whose target resolves to neither a real node nor a known leaf
/// file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DanglingEdgeFinding {
    /// The file the dangling `related:` entry was authored on.
    pub file: String,
    /// The qualified (`scope:doc_id`-shaped) reference that failed to resolve.
    pub to_ref: String,
}

/// Graph-readiness family: how many nodes/edges the corpus authors, and where the
/// dangling `related:` references are.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphFamily {
    pub node_count: usize,
    pub edge_count: usize,
    pub resolved_count: usize,
    pub leaf_target_count: usize,
    pub dangling_count: usize,
    pub dangling_findings: Vec<DanglingEdgeFinding>,
}

/// Compute the graph-readiness family over already-read `(path, content)` pairs.
///
/// Pure: performs no filesystem access. `scope` is the corpus scope slug every
/// discovered file belongs to (assess only ever looks at one corpus per run, unlike
/// mev's multi-scope crawl); `docs` is expected to already be the set of markdown
/// files `assess::discover::resolve_context` found.
pub fn assess_graph(scope: &str, docs: &[(PathBuf, String)]) -> GraphFamily {
    let mut nodes: Vec<Node> = Vec::new();
    let mut node_map: HashMap<String, usize> = HashMap::new();
    let mut leaf_keys: HashSet<String> = HashSet::new();
    // (from canonical id, raw to_ref, source file display path) — collected in a
    // first pass so the node map is fully built before edges are resolved.
    let mut pending_edges: Vec<(String, String, String)> = Vec::new();

    for (path, content) in docs {
        let stem = file_stem(path);
        let doc_id = match extract_frontmatter(content) {
            ParseResult::Ok(fm) => fm
                .fields
                .get("doc_id")
                .map(|(value, _)| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .map(|doc_id| (doc_id, fm)),
            _ => None,
        };

        match doc_id {
            Some((doc_id, fm)) => {
                let canonical_id = format!("{scope}:{doc_id}");
                let idx = nodes.len();
                node_map.insert(canonical_id.clone(), idx);
                nodes.push(Node {
                    id: canonical_id.clone(),
                    scope: scope.to_string(),
                    doc_id,
                    rel: path.clone(),
                });

                if let Some((raw_related, _)) = fm.fields.get("related") {
                    let file = display_path(path);
                    for to_ref in parse_related_list(raw_related) {
                        pending_edges.push((canonical_id.clone(), to_ref, file.clone()));
                    }
                }
            }
            None => {
                leaf_keys.insert(format!("{scope}:{stem}"));
            }
        }
    }

    let mut edges = Vec::with_capacity(pending_edges.len());
    let mut edge_files = Vec::with_capacity(pending_edges.len());
    for (from, to_ref, file) in pending_edges {
        edges.push(Edge {
            from,
            to_ref,
            kind: EdgeKind::Related,
        });
        edge_files.push(file);
    }

    let artifact = GraphArtifact {
        graph: Graph { nodes, edges },
        node_map,
        leaf_keys,
    };

    let mut family = GraphFamily {
        node_count: artifact.graph.nodes.len(),
        edge_count: artifact.graph.edges.len(),
        ..Default::default()
    };

    for (edge, file) in artifact.graph.edges.iter().zip(edge_files.iter()) {
        match resolve_edge(&artifact, edge) {
            EdgeResolution::Resolved { .. } => family.resolved_count += 1,
            EdgeResolution::LeafTarget { .. } => family.leaf_target_count += 1,
            EdgeResolution::Dangling { qualified } => {
                family.dangling_findings.push(DanglingEdgeFinding {
                    file: file.clone(),
                    to_ref: qualified,
                });
            }
        }
    }
    family.dangling_count = family.dangling_findings.len();

    family
}

/// Parse a raw `related:` frontmatter value (as recovered by the hand-rolled OKF
/// parser, e.g. `[a, b]` or a bare single ref) into individual as-authored refs.
///
/// Pure string parsing — no YAML dependency, matching the house-style hand-rolled
/// parser/serializer this crate already uses elsewhere.
fn parse_related_list(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(trimmed);

    inner
        .split(',')
        .map(|item| unquote(item.trim()))
        .filter(|item| !item.is_empty())
        .collect()
}

/// Strip a matching pair of surrounding double quotes and undo the serializer's
/// backslash escaping, mirroring `okf_core::frontmatter`'s quoting rules.
fn unquote(value: &str) -> String {
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        value[1..value.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")
    } else {
        value.to_string()
    }
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(path: &str, content: &str) -> (PathBuf, String) {
        (PathBuf::from(path), content.to_string())
    }

    #[test]
    fn node_count_over_a_mixed_fixture() {
        let docs = [
            doc(
                "a.md",
                "---\ntype: Doc\ntitle: A\ndescription: D\ndoc_id: alpha\n---\n",
            ),
            doc(
                "b.md",
                "---\ntype: Doc\ntitle: B\ndescription: D\ndoc_id: beta\n---\n",
            ),
            doc("leaf.md", "# no frontmatter\n"),
        ];
        let family = assess_graph("brain", &docs);
        assert_eq!(family.node_count, 2);
        assert_eq!(family.edge_count, 0);
    }

    #[test]
    fn bare_to_ref_qualifies_to_referrers_own_scope_and_resolves() {
        let docs = [
            doc(
                "a.md",
                "---\ntype: Doc\ntitle: A\ndescription: D\ndoc_id: alpha\nrelated: [beta]\n---\n",
            ),
            doc(
                "b.md",
                "---\ntype: Doc\ntitle: B\ndescription: D\ndoc_id: beta\n---\n",
            ),
        ];
        let family = assess_graph("brain", &docs);
        assert_eq!(family.edge_count, 1);
        assert_eq!(family.resolved_count, 1);
        assert_eq!(family.dangling_count, 0);
        assert!(family.dangling_findings.is_empty());
    }

    #[test]
    fn related_entry_to_nonexistent_doc_id_is_classified_dangling() {
        let docs = [doc(
            "a.md",
            "---\ntype: Doc\ntitle: A\ndescription: D\ndoc_id: alpha\nrelated: [typo-nonexistent]\n---\n",
        )];
        let family = assess_graph("brain", &docs);
        assert_eq!(family.edge_count, 1);
        assert_eq!(family.dangling_count, 1);
        assert_eq!(family.resolved_count, 0);
        assert_eq!(family.leaf_target_count, 0);
        let finding = &family.dangling_findings[0];
        assert_eq!(finding.file, "a.md");
        assert_eq!(finding.to_ref, "brain:typo-nonexistent");
    }

    #[test]
    fn related_entry_to_a_leaf_file_is_leaf_target_not_dangling() {
        let docs = [
            doc(
                "a.md",
                "---\ntype: Doc\ntitle: A\ndescription: D\ndoc_id: alpha\nrelated: [leaf]\n---\n",
            ),
            doc("leaf.md", "# no frontmatter, no doc_id\n"),
        ];
        let family = assess_graph("brain", &docs);
        assert_eq!(family.edge_count, 1);
        assert_eq!(family.leaf_target_count, 1);
        assert_eq!(family.dangling_count, 0);
        assert!(family.dangling_findings.is_empty());
    }

    #[test]
    fn empty_corpus_yields_zero_nodes_and_edges_without_panicking() {
        let docs: [(PathBuf, String); 0] = [];
        let family = assess_graph("brain", &docs);
        assert_eq!(family.node_count, 0);
        assert_eq!(family.edge_count, 0);
        assert_eq!(family.resolved_count, 0);
        assert_eq!(family.leaf_target_count, 0);
        assert_eq!(family.dangling_count, 0);
    }

    #[test]
    fn multiple_related_entries_are_each_classified_independently() {
        let docs = [
            doc(
                "a.md",
                "---\ntype: Doc\ntitle: A\ndescription: D\ndoc_id: alpha\nrelated: [beta, ghost]\n---\n",
            ),
            doc(
                "b.md",
                "---\ntype: Doc\ntitle: B\ndescription: D\ndoc_id: beta\n---\n",
            ),
        ];
        let family = assess_graph("brain", &docs);
        assert_eq!(family.edge_count, 2);
        assert_eq!(family.resolved_count, 1);
        assert_eq!(family.dangling_count, 1);
        assert_eq!(family.dangling_findings[0].to_ref, "brain:ghost");
    }

    #[test]
    fn quoted_related_entry_is_unquoted_before_resolution() {
        let docs = [
            doc(
                "a.md",
                "---\ntype: Doc\ntitle: A\ndescription: D\ndoc_id: alpha\nrelated: [\"beta\"]\n---\n",
            ),
            doc(
                "b.md",
                "---\ntype: Doc\ntitle: B\ndescription: D\ndoc_id: beta\n---\n",
            ),
        ];
        let family = assess_graph("brain", &docs);
        assert_eq!(family.resolved_count, 1);
    }

    #[test]
    fn file_with_no_doc_id_is_a_leaf_not_a_node() {
        let docs = [doc(
            "leaf.md",
            "---\ntype: Doc\ntitle: L\ndescription: D\n---\n",
        )];
        let family = assess_graph("brain", &docs);
        assert_eq!(family.node_count, 0);
    }

    #[test]
    fn empty_related_list_produces_no_edges() {
        let docs = [doc(
            "a.md",
            "---\ntype: Doc\ntitle: A\ndescription: D\ndoc_id: alpha\nrelated: []\n---\n",
        )];
        let family = assess_graph("brain", &docs);
        assert_eq!(family.node_count, 1);
        assert_eq!(family.edge_count, 0);
    }
}
