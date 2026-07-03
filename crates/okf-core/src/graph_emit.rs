//! Graph-export envelope shared with mev's `brain::graph_emit` (Phase 3B, Block R).
//!
//! Converts a built [`GraphArtifact`] (from mev's `build_graph`, or any other producer
//! of the shared [`crate::graph`] shapes) into a [`GraphExport`] — a canonical,
//! JSON-serializable envelope with a `version`/`root` header. Consumed by the
//! orchestrator to load nodes and edges into a Postgres edges table.
//!
//! Design principles (mirrored from mev):
//! - **Pure output** — [`build_graph_export`] does not write to disk or a DB.
//! - **No re-derivation** — nodes and edges are cloned straight from `artifact.graph`
//!   in walk order; nothing is re-walked or re-inferred here.
//! - **Deterministic leaves** — `leaves` is a sorted `Vec<String>` (from
//!   `artifact.leaf_keys`, a `HashSet`) so repeated runs over an unchanged corpus emit
//!   byte-identical output.

use std::path::Path;

use serde::Serialize;

use crate::graph::{EdgeKind, EdgeResolution, GraphArtifact, Node, resolve_edge};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// The complete graph-export envelope for a Brain corpus crawl.
///
/// Serialises to JSON for consumption by the orchestrator's Postgres edges loader.
#[derive(Debug, Serialize)]
pub struct GraphExport {
    /// Schema version — currently `"2"`.
    pub version: String,
    /// Display path of the HQ root used for the crawl.
    pub root: String,
    /// All graph nodes, in walk order.
    pub nodes: Vec<Node>,
    /// All graph edges, in walk order, carrying resolved target fields.
    pub edges: Vec<ExportedEdge>,
    /// `scope:stem` for every corpus file with no authored `doc_id`, sorted for
    /// deterministic output.
    pub leaves: Vec<String>,
}

/// One exported graph edge, augmented with the [`resolve_edge`] outcome.
///
/// `to_ref` stays raw as-authored in every case. `target_node_id`/`target_doc_id`
/// are both `Some` when the edge resolves to a real node, and both `None` when it
/// is dangling or resolves to a leaf (doc-id-less file).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ExportedEdge {
    /// The referring node's canonical `scope:doc_id`.
    pub from: String,
    /// The raw, as-authored `related:` reference (bare or qualified).
    pub to_ref: String,
    /// Edge type.
    pub kind: EdgeKind,
    /// Qualified `scope:doc_id` of the resolved target node, or `None` if the
    /// edge is dangling or targets a leaf.
    pub target_node_id: Option<String>,
    /// The resolved target node's authored `doc_id`, or `None` if the edge is
    /// dangling or targets a leaf.
    pub target_doc_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Build a [`GraphExport`] from a pre-built [`GraphArtifact`].
///
/// `root` is the HQ directory that was crawled; it is stored as a display string in the
/// envelope header and is not used to access the filesystem.
///
/// Nodes are cloned directly from `artifact.graph` (already deterministic walk order).
/// Each edge is resolved via [`resolve_edge`] (the same pure function mev's
/// `check_graph` uses) to populate `target_node_id`/`target_doc_id` — both `Some` on
/// `Resolved`, both `None` on `LeafTarget`/`Dangling`. `leaves` is `artifact.leaf_keys`
/// collected into a `Vec<String>` and sorted.
pub fn build_graph_export(root: &Path, artifact: &GraphArtifact) -> GraphExport {
    let mut leaves: Vec<String> = artifact.leaf_keys.iter().cloned().collect();
    leaves.sort();

    let edges: Vec<ExportedEdge> = artifact
        .graph
        .edges
        .iter()
        .map(|edge| {
            let (target_node_id, target_doc_id) = match resolve_edge(artifact, edge) {
                EdgeResolution::Resolved { node_id, doc_id } => (Some(node_id), Some(doc_id)),
                EdgeResolution::LeafTarget { .. } | EdgeResolution::Dangling { .. } => (None, None),
            };
            ExportedEdge {
                from: edge.from.clone(),
                to_ref: edge.to_ref.clone(),
                kind: edge.kind.clone(),
                target_node_id,
                target_doc_id,
            }
        })
        .collect();

    GraphExport {
        version: "2".to_string(),
        root: root.display().to_string(),
        nodes: artifact.graph.nodes.clone(),
        edges,
        leaves,
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{Edge, Graph};
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;

    fn node(scope: &str, doc_id: &str) -> Node {
        Node {
            id: format!("{scope}:{doc_id}"),
            scope: scope.to_string(),
            doc_id: doc_id.to_string(),
            rel: PathBuf::from(format!("{doc_id}.md")),
        }
    }

    fn artifact_from(
        nodes: Vec<Node>,
        edges: Vec<Edge>,
        leaf_keys: HashSet<String>,
    ) -> GraphArtifact {
        let mut node_map = HashMap::new();
        for (idx, n) in nodes.iter().enumerate() {
            node_map.insert(n.id.clone(), idx);
        }
        GraphArtifact {
            graph: Graph { nodes, edges },
            node_map,
            leaf_keys,
        }
    }

    #[test]
    fn maps_nodes_edges_and_sorted_leaves() {
        let nodes = vec![node("brain", "alpha"), node("brain", "beta")];
        let edges = vec![Edge {
            from: "brain:alpha".to_string(),
            to_ref: "beta".to_string(),
            kind: EdgeKind::Related,
        }];
        let mut leaf_keys = HashSet::new();
        leaf_keys.insert("brain:z-leaf".to_string());
        leaf_keys.insert("brain:a-leaf".to_string());
        let artifact = artifact_from(nodes, edges, leaf_keys);

        let root = Path::new("/hq");
        let export = build_graph_export(root, &artifact);

        assert_eq!(export.version, "2");
        assert_eq!(export.root, "/hq");
        assert_eq!(export.nodes.len(), 2);
        assert_eq!(export.edges.len(), 1);
        assert_eq!(export.edges[0].from, "brain:alpha");
        assert_eq!(export.edges[0].to_ref, "beta");
        assert_eq!(
            export.edges[0].target_node_id,
            Some("brain:beta".to_string())
        );
        assert_eq!(export.edges[0].target_doc_id, Some("beta".to_string()));
        assert_eq!(
            export.leaves,
            vec!["brain:a-leaf".to_string(), "brain:z-leaf".to_string()],
            "leaves must be sorted"
        );
    }

    #[test]
    fn dangling_and_leaf_edges_have_null_target_fields() {
        let nodes = vec![node("brain", "alpha")];
        let edges = vec![
            Edge {
                from: "brain:alpha".to_string(),
                to_ref: "missing".to_string(),
                kind: EdgeKind::Related,
            },
            Edge {
                from: "brain:alpha".to_string(),
                to_ref: "z-leaf".to_string(),
                kind: EdgeKind::Related,
            },
        ];
        let mut leaf_keys = HashSet::new();
        leaf_keys.insert("brain:z-leaf".to_string());
        let artifact = artifact_from(nodes, edges, leaf_keys);

        let export = build_graph_export(Path::new("/hq"), &artifact);

        let dangling = export
            .edges
            .iter()
            .find(|e| e.to_ref == "missing")
            .expect("dangling edge present");
        assert_eq!(dangling.target_node_id, None);
        assert_eq!(dangling.target_doc_id, None);

        let leaf = export
            .edges
            .iter()
            .find(|e| e.to_ref == "z-leaf")
            .expect("leaf-target edge present");
        assert_eq!(leaf.target_node_id, None);
        assert_eq!(leaf.target_doc_id, None);
    }

    #[test]
    fn empty_artifact_produces_empty_vecs() {
        let artifact = artifact_from(vec![], vec![], HashSet::new());
        let export = build_graph_export(Path::new("/hq"), &artifact);

        assert_eq!(export.version, "2");
        assert!(export.nodes.is_empty());
        assert!(export.edges.is_empty());
        assert!(export.leaves.is_empty());
    }

    #[test]
    fn graph_export_serializes_with_version_2_and_expected_fields() {
        let nodes = vec![node("brain", "my-doc")];
        let edges = vec![Edge {
            from: "brain:my-doc".to_string(),
            to_ref: "other".to_string(),
            kind: EdgeKind::Related,
        }];
        let artifact = artifact_from(nodes, edges, HashSet::new());
        let export = build_graph_export(Path::new("/hq"), &artifact);

        let json = serde_json::to_string(&export).expect("export must serialize to JSON");
        let value: serde_json::Value =
            serde_json::from_str(&json).expect("export JSON must be valid");

        assert_eq!(value["version"], "2");
        assert!(value.get("root").is_some());
        assert!(value.get("nodes").is_some());
        assert!(value.get("edges").is_some());
        assert!(value.get("leaves").is_some());
        assert!(value["edges"][0].get("target_node_id").is_some());
        assert!(value["edges"][0].get("target_doc_id").is_some());
    }
}
