// Structural query functions over a `BrainGraph`.
//
// All functions are pure (no I/O) — they delegate to the algorithm surface
// exposed by `BrainGraph` and return typed errors for unknown node ids.
//
// Phase 6 Block A — Task 3.

use crate::brain::graph::{BrainGraph, BrainGraphError};
use crate::brain::okf::BrainNode;

// ── Public query API ──────────────────────────────────────────────────────────

/// Return the direct dependents of `id` — the nodes that have an outgoing
/// `[[link]]` edge pointing **to** `id` (i.e. "what depends on `id`?").
///
/// This is a direct-edge query (not transitive). For transitive reverse
/// reachability use [`blast_radius`].
///
/// Returns `Err(BrainGraphError::UnknownNode)` when `id` is not in the graph.
pub fn dependents(graph: &BrainGraph, id: &str) -> Result<Vec<BrainNode>, BrainGraphError> {
    graph.predecessors(id)
}

/// Return every node that transitively depends on `id` — i.e. everything
/// whose correctness could be broken by a change to `id`.
///
/// Computed as the full reverse-BFS reachability set from `id` (all ancestors,
/// not just direct parents). The node `id` itself is NOT included.
///
/// Returns `Err(BrainGraphError::UnknownNode)` when `id` is not in the graph.
pub fn blast_radius(graph: &BrainGraph, id: &str) -> Result<Vec<BrainNode>, BrainGraphError> {
    graph.reachable_reverse(id)
}

/// Return the lineage of `id` — the chain of nodes that `id` transitively
/// references (outgoing edges, forward direction).
///
/// Computed as the full forward-DFS reachability set from `id`. The node `id`
/// itself is NOT included.
///
/// Returns `Err(BrainGraphError::UnknownNode)` when `id` is not in the graph.
pub fn lineage(graph: &BrainGraph, id: &str) -> Result<Vec<BrainNode>, BrainGraphError> {
    graph.reachable_forward(id)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::graph::BrainGraph;
    use crate::brain::okf::{BrainEdge, BrainNode};
    use std::path::PathBuf;

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn node(id: &str) -> BrainNode {
        BrainNode {
            id: id.to_string(),
            title: id.to_string(),
            path: PathBuf::from(format!("fixtures/{id}.md")),
        }
    }

    fn edge(from: &str, to: &str) -> BrainEdge {
        BrainEdge {
            from: from.to_string(),
            to: to.to_string(),
        }
    }

    /// Linear chain: a → b → c
    fn chain_graph() -> BrainGraph {
        BrainGraph::build(
            vec![node("a"), node("b"), node("c")],
            vec![edge("a", "b"), edge("b", "c")],
        )
    }

    /// Diamond DAG: a → b, a → c, b → d, c → d
    fn diamond_graph() -> BrainGraph {
        BrainGraph::build(
            vec![node("a"), node("b"), node("c"), node("d")],
            vec![
                edge("a", "b"),
                edge("a", "c"),
                edge("b", "d"),
                edge("c", "d"),
            ],
        )
    }

    /// Fixture-mirroring graph built from the known decision topology:
    ///   d3 → d20
    ///   d20 → d21, d20 → d3   (cycle between d3 and d20)
    ///   d21 → d20, d21 → d4
    ///   d4: leaf
    ///   unlinked: isolated
    fn fixture_graph() -> BrainGraph {
        BrainGraph::build(
            vec![
                node("d3"),
                node("d20"),
                node("d21"),
                node("d4"),
                node("unlinked"),
            ],
            vec![
                edge("d3", "d20"),
                edge("d20", "d21"),
                edge("d20", "d3"),
                edge("d21", "d20"),
                edge("d21", "d4"),
            ],
        )
    }

    // ── dependents ────────────────────────────────────────────────────────────

    #[test]
    fn dependents_single_result() {
        // In the chain a→b→c, b's dependents = [a].
        let g = chain_graph();
        let deps = dependents(&g, "b").unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].id, "a");
    }

    #[test]
    fn dependents_multiple_results() {
        // In the diamond a→b→d and a→c→d, d's dependents = [b, c].
        let g = diamond_graph();
        let deps = dependents(&g, "d").unwrap();
        let ids: Vec<&str> = deps.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"b"), "b depends on d");
        assert!(ids.contains(&"c"), "c depends on d");
        assert_eq!(deps.len(), 2);
    }

    #[test]
    fn dependents_source_node_has_none() {
        // a has no predecessors.
        let g = chain_graph();
        let deps = dependents(&g, "a").unwrap();
        assert!(deps.is_empty(), "source node has no dependents");
    }

    #[test]
    fn dependents_unknown_id_returns_error() {
        let g = chain_graph();
        let err = dependents(&g, "z").unwrap_err();
        assert_eq!(err, BrainGraphError::UnknownNode("z".to_string()));
    }

    #[test]
    fn dependents_fixture_d20_has_d3_and_d21() {
        // d3 → d20 and d21 → d20, so d20's direct dependents are d3 and d21.
        let g = fixture_graph();
        let deps = dependents(&g, "d20").unwrap();
        let ids: Vec<&str> = deps.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"d3"), "d3 references d20");
        assert!(ids.contains(&"d21"), "d21 references d20");
        assert_eq!(deps.len(), 2);
    }

    #[test]
    fn dependents_isolated_node_has_none() {
        let g = fixture_graph();
        let deps = dependents(&g, "unlinked").unwrap();
        assert!(deps.is_empty());
    }

    // ── blast_radius ──────────────────────────────────────────────────────────

    #[test]
    fn blast_radius_leaf_includes_full_chain() {
        // In the chain a→b→c, changing c affects b (direct) and a (transitive).
        let g = chain_graph();
        let br = blast_radius(&g, "c").unwrap();
        let ids: Vec<&str> = br.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"a"), "a is transitively affected");
        assert!(ids.contains(&"b"), "b is directly affected");
        assert!(!ids.contains(&"c"), "start node not included");
    }

    #[test]
    fn blast_radius_source_node_has_none() {
        // a has no predecessors, so nothing in the blast radius.
        let g = chain_graph();
        let br = blast_radius(&g, "a").unwrap();
        assert!(br.is_empty());
    }

    #[test]
    fn blast_radius_diamond_d_includes_all() {
        // d is the sink; changing it affects b, c (direct) and a (transitive).
        let g = diamond_graph();
        let br = blast_radius(&g, "d").unwrap();
        let ids: Vec<&str> = br.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"a"));
        assert!(ids.contains(&"b"));
        assert!(ids.contains(&"c"));
        assert!(!ids.contains(&"d"));
    }

    #[test]
    fn blast_radius_unknown_id_returns_error() {
        let g = chain_graph();
        let err = blast_radius(&g, "z").unwrap_err();
        assert_eq!(err, BrainGraphError::UnknownNode("z".to_string()));
    }

    #[test]
    fn blast_radius_fixture_d4_includes_transitive() {
        // d21 → d4, and d20 → d21, so blast_radius(d4) must include d21.
        // d20 → d21 and d3 → d20, d21 → d20 form a cycle, so BFS may also reach d20, d3.
        let g = fixture_graph();
        let br = blast_radius(&g, "d4").unwrap();
        let ids: Vec<&str> = br.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"d21"), "d21 directly depends on d4");
        // Transitive: d20→d21 and d3→d20, d21→d20 form a cycle reachable in reverse BFS.
        assert!(!ids.contains(&"d4"), "start node excluded");
    }

    #[test]
    fn blast_radius_isolated_node_has_none() {
        let g = fixture_graph();
        let br = blast_radius(&g, "unlinked").unwrap();
        assert!(br.is_empty());
    }

    // ── lineage ───────────────────────────────────────────────────────────────

    #[test]
    fn lineage_source_traces_full_chain() {
        // In a→b→c, lineage(a) = [b, c].
        let g = chain_graph();
        let lin = lineage(&g, "a").unwrap();
        let ids: Vec<&str> = lin.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"b"));
        assert!(ids.contains(&"c"));
        assert!(!ids.contains(&"a"), "start node not included");
    }

    #[test]
    fn lineage_leaf_is_empty() {
        let g = chain_graph();
        let lin = lineage(&g, "c").unwrap();
        assert!(lin.is_empty(), "leaf node has no outgoing lineage");
    }

    #[test]
    fn lineage_diamond_a_reaches_all() {
        // a → b, c, d.
        let g = diamond_graph();
        let lin = lineage(&g, "a").unwrap();
        let ids: Vec<&str> = lin.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"b"));
        assert!(ids.contains(&"c"));
        assert!(ids.contains(&"d"));
        assert!(!ids.contains(&"a"));
    }

    #[test]
    fn lineage_unknown_id_returns_error() {
        let g = chain_graph();
        let err = lineage(&g, "z").unwrap_err();
        assert_eq!(err, BrainGraphError::UnknownNode("z".to_string()));
    }

    #[test]
    fn lineage_fixture_d3_reaches_d20() {
        // d3 → d20, and d20 → d21, d20 → d3 (cycle), so forward DFS from d3 will reach d20.
        let g = fixture_graph();
        let lin = lineage(&g, "d3").unwrap();
        let ids: Vec<&str> = lin.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"d20"), "d3 directly links to d20");
        assert!(!ids.contains(&"d3"), "start node excluded");
    }

    #[test]
    fn lineage_fixture_d21_reaches_d4() {
        // d21 → d4 (leaf).
        let g = fixture_graph();
        let lin = lineage(&g, "d21").unwrap();
        let ids: Vec<&str> = lin.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"d4"), "d21 links to d4");
    }

    #[test]
    fn lineage_isolated_node_is_empty() {
        let g = fixture_graph();
        let lin = lineage(&g, "unlinked").unwrap();
        assert!(lin.is_empty());
    }
}
