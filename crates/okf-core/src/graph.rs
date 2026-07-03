//! Graph/edge-resolution model shared with mev's `brain::graph` (Phase 3, Block J).
//!
//! `okf-core` owns the **pure model + resolution primitives** mev's knowledge graph is
//! built from: [`Node`], [`Edge`], [`EdgeKind`], [`Graph`], [`GraphArtifact`], and
//! [`resolve_edge`]. mev's corpus-walking construction (`build_graph`) and
//! diagnostic-producing checks (`check_graph`) stay in mev — they depend on mev-only
//! types (`Corpus`, `BrainConfig`, `Diagnostic`) that have no place in this shared crate.
//! `resolve_edge` is extracted here because it is the pure resolution primitive both
//! `check_graph` and mev's `graph_emit::build_graph_export` consume, and because
//! `okf-core`'s own [`crate::graph_emit::build_graph_export`] needs it too.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use serde::Serialize;

// ---------------------------------------------------------------------------
// Graph model (D4 serializable, emittable artifact)
// ---------------------------------------------------------------------------

/// The kind of a directed edge in the knowledge graph.
///
/// `Related` is the only variant today, sourced from the `related:` frontmatter list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Related,
}

/// A directed edge in the knowledge graph.
///
/// `from` and `to_ref` are both *as-authored* (from `related:` entries); normalisation to
/// qualified `scope:doc_id` happens at resolution time via [`resolve_edge`].
#[derive(Debug, Clone, Serialize)]
pub struct Edge {
    /// Canonical id of the source node (`scope:doc_id`).
    pub from: String,
    /// The raw `related:` entry as authored (bare or `scope:doc_id`).
    pub to_ref: String,
    /// Edge type.
    pub kind: EdgeKind,
}

/// A graph node — a Brain corpus file with an authored `doc_id`.
#[derive(Debug, Clone, Serialize)]
pub struct Node {
    /// Canonical id: `scope:doc_id`.
    pub id: String,
    /// Owning scope slug (from the corpus registry).
    pub scope: String,
    /// Authored `doc_id` (location-independent frontmatter field).
    pub doc_id: String,
    /// Path of the file relative to the HQ crawl root.
    pub rel: PathBuf,
}

/// The serializable, emittable knowledge graph — the D4 artifact.
#[derive(Debug, Default, Serialize)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

/// All data required to resolve edges against a built graph: the serializable graph
/// artifact plus the lookup structures `resolve_edge` needs.
///
/// mev's `build_graph` populates this by walking a `Corpus`; `okf-core` does not own
/// that construction step (it depends on mev-only corpus types), only the shape and
/// the resolution logic that consumes it.
pub struct GraphArtifact {
    /// The serializable, emittable graph (D4 artifact).
    pub graph: Graph,
    /// `canonical_id → node index` for O(1) resolution.
    pub node_map: HashMap<String, usize>,
    /// `scope:stem` for every corpus file that has **no** authored `doc_id`.
    pub leaf_keys: HashSet<String>,
}

// ---------------------------------------------------------------------------
// Edge resolution
// ---------------------------------------------------------------------------

/// The outcome of resolving one edge's `to_ref` against a [`GraphArtifact`].
///
/// Produced by [`resolve_edge`] — the single source of truth for edge resolution,
/// shared by mev's `check_graph` (diagnostics) and [`crate::graph_emit::build_graph_export`]
/// (exported fields).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdgeResolution {
    /// The (qualified) `to_ref` resolves to a real node.
    Resolved {
        /// Qualified `scope:doc_id` of the target node.
        node_id: String,
        /// The target node's authored `doc_id`.
        doc_id: String,
    },
    /// The (qualified) `to_ref` resolves to a known leaf file (no `doc_id`).
    LeafTarget {
        /// The qualified `scope:doc_id`-shaped key used for the leaf lookup.
        qualified: String,
    },
    /// The (qualified) `to_ref` resolves to nothing in the corpus.
    Dangling {
        /// The qualified `scope:doc_id`-shaped key that could not be found.
        qualified: String,
    },
}

/// Resolve one edge's `to_ref` against the built [`GraphArtifact`].
///
/// Qualifies a bare `to_ref` (no `:`) to the referrer's own scope (looked up via
/// `edge.from` in `node_map`), then looks the qualified id up in `node_map`. Falls
/// back to classifying the qualified key as a known leaf (`leaf_keys`) or dangling.
///
/// Pure: takes only the artifact and one edge, returns an [`EdgeResolution`] — no
/// diagnostics are produced here.
pub fn resolve_edge(artifact: &GraphArtifact, edge: &Edge) -> EdgeResolution {
    // Determine the referrer's scope from the from-node.
    let from_scope: &str = artifact
        .node_map
        .get(edge.from.as_str())
        .and_then(|&idx| artifact.graph.nodes.get(idx))
        .map(|n| n.scope.as_str())
        .unwrap_or("");

    // Normalise to_ref: if it contains ':' it is already qualified; otherwise
    // qualify it within the referrer's scope.
    let qualified: String = if edge.to_ref.contains(':') {
        edge.to_ref.clone()
    } else {
        format!("{from_scope}:{}", edge.to_ref)
    };

    if let Some(&idx) = artifact.node_map.get(qualified.as_str())
        && let Some(node) = artifact.graph.nodes.get(idx)
    {
        return EdgeResolution::Resolved {
            node_id: qualified,
            doc_id: node.doc_id.clone(),
        };
    }

    // Not a node: check if it's a known leaf.
    if artifact.leaf_keys.contains(&qualified) {
        EdgeResolution::LeafTarget { qualified }
    } else {
        EdgeResolution::Dangling { qualified }
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal `GraphArtifact` directly (no corpus walk — that lives in mev).
    fn artifact_from(nodes: Vec<Node>, leaf_keys: HashSet<String>) -> GraphArtifact {
        let mut node_map = HashMap::new();
        for (idx, node) in nodes.iter().enumerate() {
            node_map.insert(node.id.clone(), idx);
        }
        GraphArtifact {
            graph: Graph {
                nodes,
                edges: Vec::new(),
            },
            node_map,
            leaf_keys,
        }
    }

    fn node(scope: &str, doc_id: &str) -> Node {
        Node {
            id: format!("{scope}:{doc_id}"),
            scope: scope.to_string(),
            doc_id: doc_id.to_string(),
            rel: PathBuf::from(format!("{doc_id}.md")),
        }
    }

    #[test]
    fn resolve_edge_returns_resolved_for_bare_ref_in_same_scope() {
        let artifact = artifact_from(
            vec![node("brain", "alpha"), node("brain", "beta")],
            HashSet::new(),
        );
        let edge = Edge {
            from: "brain:alpha".to_string(),
            to_ref: "beta".to_string(),
            kind: EdgeKind::Related,
        };
        assert_eq!(
            resolve_edge(&artifact, &edge),
            EdgeResolution::Resolved {
                node_id: "brain:beta".to_string(),
                doc_id: "beta".to_string(),
            }
        );
    }

    #[test]
    fn resolve_edge_returns_resolved_for_qualified_cross_scope_ref() {
        let artifact = artifact_from(
            vec![node("brain", "alpha"), node("mev", "target")],
            HashSet::new(),
        );
        let edge = Edge {
            from: "brain:alpha".to_string(),
            to_ref: "mev:target".to_string(),
            kind: EdgeKind::Related,
        };
        assert_eq!(
            resolve_edge(&artifact, &edge),
            EdgeResolution::Resolved {
                node_id: "mev:target".to_string(),
                doc_id: "target".to_string(),
            }
        );
    }

    #[test]
    fn resolve_edge_returns_leaf_target_for_known_leaf() {
        let mut leaf_keys = HashSet::new();
        leaf_keys.insert("brain:leaf-stem".to_string());
        let artifact = artifact_from(vec![node("brain", "alpha")], leaf_keys);
        let edge = Edge {
            from: "brain:alpha".to_string(),
            to_ref: "leaf-stem".to_string(),
            kind: EdgeKind::Related,
        };
        assert_eq!(
            resolve_edge(&artifact, &edge),
            EdgeResolution::LeafTarget {
                qualified: "brain:leaf-stem".to_string(),
            }
        );
    }

    #[test]
    fn resolve_edge_returns_dangling_for_missing_target() {
        let artifact = artifact_from(vec![node("brain", "alpha")], HashSet::new());
        let edge = Edge {
            from: "brain:alpha".to_string(),
            to_ref: "typo-nonexistent".to_string(),
            kind: EdgeKind::Related,
        };
        assert_eq!(
            resolve_edge(&artifact, &edge),
            EdgeResolution::Dangling {
                qualified: "brain:typo-nonexistent".to_string(),
            }
        );
    }

    #[test]
    fn resolve_edge_bare_ref_naming_other_scope_id_is_dangling() {
        // Bare refs are always qualified to the *from-node's* scope first; they do
        // NOT search across scopes, even if a same-named doc_id exists elsewhere.
        let artifact = artifact_from(
            vec![node("brain", "alpha"), node("mev", "mev-target")],
            HashSet::new(),
        );
        let edge = Edge {
            from: "brain:alpha".to_string(),
            to_ref: "mev-target".to_string(),
            kind: EdgeKind::Related,
        };
        assert_eq!(
            resolve_edge(&artifact, &edge),
            EdgeResolution::Dangling {
                qualified: "brain:mev-target".to_string(),
            }
        );
    }

    #[test]
    fn resolve_edge_unknown_from_node_qualifies_to_empty_scope() {
        // `from` not present in node_map → from_scope defaults to "" (graceful
        // degradation, matching mev's `unwrap_or("")`).
        let artifact = artifact_from(vec![node("brain", "alpha")], HashSet::new());
        let edge = Edge {
            from: "brain:ghost".to_string(),
            to_ref: "somewhere".to_string(),
            kind: EdgeKind::Related,
        };
        assert_eq!(
            resolve_edge(&artifact, &edge),
            EdgeResolution::Dangling {
                qualified: ":somewhere".to_string(),
            }
        );
    }

    #[test]
    fn graph_serializes_nodes_and_edges() {
        let graph = Graph {
            nodes: vec![node("brain", "alpha")],
            edges: vec![Edge {
                from: "brain:alpha".to_string(),
                to_ref: "beta".to_string(),
                kind: EdgeKind::Related,
            }],
        };
        let json = serde_json::to_string(&graph).expect("graph must serialize");
        assert!(json.contains("brain:alpha"));
        assert!(json.contains("\"to_ref\":\"beta\""));
        assert!(json.contains("\"kind\":\"related\""));
    }

    #[test]
    fn edge_kind_serializes_snake_case() {
        let json = serde_json::to_string(&EdgeKind::Related).unwrap();
        assert_eq!(json, "\"related\"");
    }
}
