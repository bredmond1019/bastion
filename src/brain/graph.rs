// BrainGraph — petgraph-backed directed graph for the OKF knowledge corpus.
//
// Wraps `petgraph::graph::DiGraph<BrainNode, ()>` and exposes a Dgraph-free
// algorithm surface (shortest path, topological sort, DFS/BFS reachability).
// All public functions return typed errors rather than panicking.
//
// Phase 6 Block A — Task 2.

use std::collections::{HashMap, HashSet};

use petgraph::{
    Direction,
    algo::{astar, has_path_connecting, toposort},
    graph::{DiGraph, NodeIndex},
    visit::{Bfs, Dfs, Reversed},
};

use crate::brain::okf::{BrainEdge, BrainNode};

// ── Error type ────────────────────────────────────────────────────────────────

/// Errors returned by `BrainGraph` operations.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum BrainGraphError {
    /// A node id passed to a query does not exist in the graph.
    #[error("unknown node id: {0}")]
    UnknownNode(String),

    /// `toposort` detected a cycle in the graph.
    #[error("graph contains a cycle; topological sort is not possible")]
    CycleDetected,
}

// ── BrainGraph ────────────────────────────────────────────────────────────────

/// A directed graph over OKF brain nodes backed by `petgraph`.
///
/// Constructed once from `(Vec<BrainNode>, Vec<BrainEdge>)` and then queried
/// by node id string.
pub struct BrainGraph {
    /// The underlying petgraph directed graph.
    pub(crate) graph: DiGraph<BrainNode, ()>,
    /// Maps stable node id → petgraph `NodeIndex`.
    pub(crate) index: HashMap<String, NodeIndex>,
    /// Maps bare node title → all matching `NodeIndex` values.
    ///
    /// Used by [`predecessors_by_name`] for bare-name lookups in the code surface,
    /// where qualified ids (`file_stem::kind::name`) are stored in `index` but CLI
    /// queries arrive as bare symbol names.
    pub(crate) name_index: HashMap<String, Vec<NodeIndex>>,
}

impl BrainGraph {
    /// Build a `BrainGraph` from node and edge lists produced by `okf::build_node_edge_lists`.
    ///
    /// Edges whose `from` or `to` id does not correspond to a known node are silently skipped.
    pub fn build(nodes: Vec<BrainNode>, edges: Vec<BrainEdge>) -> Self {
        let mut graph: DiGraph<BrainNode, ()> = DiGraph::new();
        let mut index: HashMap<String, NodeIndex> = HashMap::new();
        let mut name_index: HashMap<String, Vec<NodeIndex>> = HashMap::new();

        for node in nodes {
            let id = node.id.clone();
            let title = node.title.clone();
            let idx = graph.add_node(node);
            index.insert(id, idx);
            name_index.entry(title).or_default().push(idx);
        }

        for edge in edges {
            if let (Some(&from_idx), Some(&to_idx)) = (index.get(&edge.from), index.get(&edge.to)) {
                graph.add_edge(from_idx, to_idx, ());
            }
        }

        Self {
            graph,
            index,
            name_index,
        }
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    fn resolve(&self, id: &str) -> Result<NodeIndex, BrainGraphError> {
        self.index
            .get(id)
            .copied()
            .ok_or_else(|| BrainGraphError::UnknownNode(id.to_string()))
    }

    // ── Traversal ─────────────────────────────────────────────────────────────

    /// Forward DFS reachability: all nodes reachable from `id` following
    /// outgoing edges (i.e. what `id` transitively points to).
    ///
    /// The start node is NOT included in the result.
    pub fn reachable_forward(&self, id: &str) -> Result<Vec<BrainNode>, BrainGraphError> {
        let start = self.resolve(id)?;
        let mut dfs = Dfs::new(&self.graph, start);
        // Advance past the start node.
        dfs.next(&self.graph);
        let mut result = Vec::new();
        while let Some(nx) = dfs.next(&self.graph) {
            result.push(self.graph[nx].clone());
        }
        Ok(result)
    }

    /// Reverse BFS reachability: all nodes that can reach `id` following
    /// incoming edges (i.e. everything that transitively depends on `id`).
    ///
    /// The start node is NOT included in the result.
    pub fn reachable_reverse(&self, id: &str) -> Result<Vec<BrainNode>, BrainGraphError> {
        let start = self.resolve(id)?;
        let reversed = Reversed(&self.graph);
        let mut bfs = Bfs::new(&reversed, start);
        // Advance past the start node.
        bfs.next(&reversed);
        let mut result = Vec::new();
        while let Some(nx) = bfs.next(&reversed) {
            result.push(self.graph[nx].clone());
        }
        Ok(result)
    }

    /// Direct predecessors (nodes with an outgoing edge to `id`).
    pub fn predecessors(&self, id: &str) -> Result<Vec<BrainNode>, BrainGraphError> {
        let target = self.resolve(id)?;
        let preds = self
            .graph
            .neighbors_directed(target, Direction::Incoming)
            .map(|nx| self.graph[nx].clone())
            .collect();
        Ok(preds)
    }

    /// Direct successors (nodes that `id` directly points to).
    pub fn successors(&self, id: &str) -> Result<Vec<BrainNode>, BrainGraphError> {
        let source = self.resolve(id)?;
        let succs = self
            .graph
            .neighbors_directed(source, Direction::Outgoing)
            .map(|nx| self.graph[nx].clone())
            .collect();
        Ok(succs)
    }

    /// Direct predecessors of all nodes whose **title** matches `name`.
    ///
    /// Unlike [`predecessors`], this accepts a bare symbol name and searches
    /// `name_index` — the multi-valued reverse map from `node.title`. Used by
    /// the code-graph surface where node ids are qualified (`lib::struct::Widget`)
    /// but CLI queries arrive as bare names (`Widget`).
    ///
    /// Returns an empty vec when no node has the given title (not an error).
    /// Deduplicates predecessor nodes when multiple targets share the same name.
    pub fn predecessors_by_name(&self, name: &str) -> Vec<BrainNode> {
        let Some(indices) = self.name_index.get(name) else {
            return vec![];
        };
        let mut seen: HashSet<NodeIndex> = HashSet::new();
        let mut result = Vec::new();
        for &target in indices {
            for nx in self.graph.neighbors_directed(target, Direction::Incoming) {
                if seen.insert(nx) {
                    result.push(self.graph[nx].clone());
                }
            }
        }
        result
    }

    // ── Shortest path ─────────────────────────────────────────────────────────

    /// Find the shortest path (minimum hop count) from `from_id` to `to_id`.
    ///
    /// Returns `Ok(Some(path))` where `path` is an ordered list of nodes from
    /// `from_id` to `to_id` inclusive, or `Ok(None)` if no path exists.
    /// Returns `Err(BrainGraphError::UnknownNode)` if either id is not in the graph.
    pub fn shortest_path(
        &self,
        from_id: &str,
        to_id: &str,
    ) -> Result<Option<Vec<BrainNode>>, BrainGraphError> {
        let from_idx = self.resolve(from_id)?;
        let to_idx = self.resolve(to_id)?;

        // A* with unit edge costs and no heuristic (equivalent to BFS/Dijkstra for unweighted).
        let result = astar(
            &self.graph,
            from_idx,
            |finish| finish == to_idx,
            |_e| 1u32,
            |_n| 0u32,
        );

        match result {
            Some((_cost, path)) => {
                let nodes = path.into_iter().map(|nx| self.graph[nx].clone()).collect();
                Ok(Some(nodes))
            }
            None => Ok(None),
        }
    }

    /// Returns `true` if there is any directed path from `from_id` to `to_id`.
    pub fn has_path(&self, from_id: &str, to_id: &str) -> Result<bool, BrainGraphError> {
        let from_idx = self.resolve(from_id)?;
        let to_idx = self.resolve(to_id)?;
        Ok(has_path_connecting(&self.graph, from_idx, to_idx, None))
    }

    // ── Topological sort ──────────────────────────────────────────────────────

    /// Return a topological ordering of all nodes (sources first).
    ///
    /// Returns `Err(BrainGraphError::CycleDetected)` if the graph has a cycle.
    pub fn toposort(&self) -> Result<Vec<BrainNode>, BrainGraphError> {
        toposort(&self.graph, None)
            .map(|order| order.into_iter().map(|nx| self.graph[nx].clone()).collect())
            .map_err(|_| BrainGraphError::CycleDetected)
    }

    // ── Accessors ─────────────────────────────────────────────────────────────

    /// Look up a node by id, returning `None` if not present.
    pub fn get_node(&self, id: &str) -> Option<&BrainNode> {
        self.index.get(id).map(|&nx| &self.graph[nx])
    }

    /// Total number of nodes.
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Total number of edges.
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
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

    /// Build a simple linear chain: a → b → c.
    fn chain_graph() -> BrainGraph {
        BrainGraph::build(
            vec![node("a"), node("b"), node("c")],
            vec![edge("a", "b"), edge("b", "c")],
        )
    }

    /// Build a diamond DAG: a → b, a → c, b → d, c → d.
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

    /// Build a graph with a cycle: a → b → c → a.
    fn cyclic_graph() -> BrainGraph {
        BrainGraph::build(
            vec![node("a"), node("b"), node("c")],
            vec![edge("a", "b"), edge("b", "c"), edge("c", "a")],
        )
    }

    // ── BrainGraph::build ─────────────────────────────────────────────────────

    #[test]
    fn build_empty_graph() {
        let g = BrainGraph::build(vec![], vec![]);
        assert_eq!(g.node_count(), 0);
        assert_eq!(g.edge_count(), 0);
    }

    #[test]
    fn build_nodes_only() {
        let g = BrainGraph::build(vec![node("a"), node("b")], vec![]);
        assert_eq!(g.node_count(), 2);
        assert_eq!(g.edge_count(), 0);
    }

    #[test]
    fn build_chain_graph() {
        let g = chain_graph();
        assert_eq!(g.node_count(), 3);
        assert_eq!(g.edge_count(), 2);
    }

    #[test]
    fn build_skips_edge_with_unknown_from() {
        let g = BrainGraph::build(vec![node("a"), node("b")], vec![edge("unknown", "a")]);
        assert_eq!(g.node_count(), 2);
        assert_eq!(g.edge_count(), 0, "edge with unknown from-id is dropped");
    }

    #[test]
    fn build_skips_edge_with_unknown_to() {
        let g = BrainGraph::build(vec![node("a"), node("b")], vec![edge("a", "unknown")]);
        assert_eq!(g.node_count(), 2);
        assert_eq!(g.edge_count(), 0, "edge with unknown to-id is dropped");
    }

    #[test]
    fn get_node_found_and_not_found() {
        let g = chain_graph();
        assert!(g.get_node("a").is_some());
        assert!(g.get_node("z").is_none());
    }

    // ── Traversal — forward/reverse reachability ───────────────────────────────

    #[test]
    fn reachable_forward_linear_chain() {
        let g = chain_graph();
        let reached = g.reachable_forward("a").unwrap();
        let ids: Vec<&str> = reached.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"b"), "b reachable from a");
        assert!(ids.contains(&"c"), "c reachable from a");
        assert!(!ids.contains(&"a"), "start node not in result");
    }

    #[test]
    fn reachable_forward_leaf_returns_empty() {
        let g = chain_graph();
        let reached = g.reachable_forward("c").unwrap();
        assert!(reached.is_empty(), "leaf node has no forward reachability");
    }

    #[test]
    fn reachable_reverse_linear_chain() {
        let g = chain_graph();
        let reached = g.reachable_reverse("c").unwrap();
        let ids: Vec<&str> = reached.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"a"), "a can reach c");
        assert!(ids.contains(&"b"), "b can reach c");
        assert!(!ids.contains(&"c"), "start node not in result");
    }

    #[test]
    fn reachable_forward_unknown_node_returns_error() {
        let g = chain_graph();
        assert_eq!(
            g.reachable_forward("z"),
            Err(BrainGraphError::UnknownNode("z".to_string()))
        );
    }

    #[test]
    fn reachable_reverse_unknown_node_returns_error() {
        let g = chain_graph();
        assert_eq!(
            g.reachable_reverse("z"),
            Err(BrainGraphError::UnknownNode("z".to_string()))
        );
    }

    #[test]
    fn reachable_reverse_diamond_includes_all_ancestors() {
        let g = diamond_graph();
        let reached = g.reachable_reverse("d").unwrap();
        let ids: Vec<&str> = reached.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"a"));
        assert!(ids.contains(&"b"));
        assert!(ids.contains(&"c"));
        assert!(!ids.contains(&"d"));
    }

    // ── Predecessors / successors ─────────────────────────────────────────────

    #[test]
    fn predecessors_middle_of_chain() {
        let g = chain_graph();
        let preds = g.predecessors("b").unwrap();
        assert_eq!(preds.len(), 1);
        assert_eq!(preds[0].id, "a");
    }

    #[test]
    fn successors_middle_of_chain() {
        let g = chain_graph();
        let succs = g.successors("b").unwrap();
        assert_eq!(succs.len(), 1);
        assert_eq!(succs[0].id, "c");
    }

    #[test]
    fn predecessors_unknown_node_returns_error() {
        let g = chain_graph();
        assert_eq!(
            g.predecessors("z"),
            Err(BrainGraphError::UnknownNode("z".to_string()))
        );
    }

    #[test]
    fn successors_unknown_node_returns_error() {
        let g = chain_graph();
        assert_eq!(
            g.successors("z"),
            Err(BrainGraphError::UnknownNode("z".to_string()))
        );
    }

    #[test]
    fn predecessors_source_node_is_empty() {
        let g = chain_graph();
        let preds = g.predecessors("a").unwrap();
        assert!(preds.is_empty());
    }

    #[test]
    fn successors_leaf_node_is_empty() {
        let g = chain_graph();
        let succs = g.successors("c").unwrap();
        assert!(succs.is_empty());
    }

    // ── Shortest path ─────────────────────────────────────────────────────────

    #[test]
    fn shortest_path_direct_edge() {
        let g = chain_graph();
        let path = g.shortest_path("a", "b").unwrap().unwrap();
        assert_eq!(path.len(), 2);
        assert_eq!(path[0].id, "a");
        assert_eq!(path[1].id, "b");
    }

    #[test]
    fn shortest_path_two_hops() {
        let g = chain_graph();
        let path = g.shortest_path("a", "c").unwrap().unwrap();
        assert_eq!(path.len(), 3);
        assert_eq!(path[0].id, "a");
        assert_eq!(path[2].id, "c");
    }

    #[test]
    fn shortest_path_no_route_returns_none() {
        let g = chain_graph();
        // c → a has no directed path.
        let path = g.shortest_path("c", "a").unwrap();
        assert!(path.is_none());
    }

    #[test]
    fn shortest_path_unknown_from_returns_error() {
        let g = chain_graph();
        assert_eq!(
            g.shortest_path("z", "a"),
            Err(BrainGraphError::UnknownNode("z".to_string()))
        );
    }

    #[test]
    fn shortest_path_unknown_to_returns_error() {
        let g = chain_graph();
        assert_eq!(
            g.shortest_path("a", "z"),
            Err(BrainGraphError::UnknownNode("z".to_string()))
        );
    }

    #[test]
    fn shortest_path_diamond_prefers_shorter() {
        // In a diamond a→b→d, a→c→d both paths have length 2; either is valid.
        let g = diamond_graph();
        let path = g.shortest_path("a", "d").unwrap().unwrap();
        assert_eq!(
            path.len(),
            3,
            "two hops is the shortest path in the diamond"
        );
        assert_eq!(path[0].id, "a");
        assert_eq!(path[2].id, "d");
    }

    // ── has_path ──────────────────────────────────────────────────────────────

    #[test]
    fn has_path_true() {
        let g = chain_graph();
        assert!(g.has_path("a", "c").unwrap());
    }

    #[test]
    fn has_path_false() {
        let g = chain_graph();
        assert!(!g.has_path("c", "a").unwrap());
    }

    #[test]
    fn has_path_unknown_node_returns_error() {
        let g = chain_graph();
        assert!(g.has_path("z", "a").is_err());
    }

    // ── Topological sort ──────────────────────────────────────────────────────

    #[test]
    fn toposort_linear_chain_sources_first() {
        let g = chain_graph();
        let order = g.toposort().unwrap();
        let ids: Vec<&str> = order.iter().map(|n| n.id.as_str()).collect();
        // In a chain a→b→c, a must come before b and b before c.
        let pos_a = ids.iter().position(|&s| s == "a").unwrap();
        let pos_b = ids.iter().position(|&s| s == "b").unwrap();
        let pos_c = ids.iter().position(|&s| s == "c").unwrap();
        assert!(pos_a < pos_b, "a before b in toposort");
        assert!(pos_b < pos_c, "b before c in toposort");
    }

    #[test]
    fn toposort_diamond_dag() {
        let g = diamond_graph();
        let order = g.toposort().unwrap();
        let ids: Vec<&str> = order.iter().map(|n| n.id.as_str()).collect();
        let pos_a = ids.iter().position(|&s| s == "a").unwrap();
        let pos_d = ids.iter().position(|&s| s == "d").unwrap();
        assert!(pos_a < pos_d, "source a before sink d");
    }

    #[test]
    fn toposort_cycle_returns_error() {
        let g = cyclic_graph();
        assert_eq!(g.toposort(), Err(BrainGraphError::CycleDetected));
    }

    #[test]
    fn toposort_empty_graph_returns_empty() {
        let g = BrainGraph::build(vec![], vec![]);
        let order = g.toposort().unwrap();
        assert!(order.is_empty());
    }

    // ── Fixture-derived corpus test ───────────────────────────────────────────

    /// Build a graph from the stem-based (no frontmatter) interpretation of the
    /// fixture files and assert that the known topology holds.
    ///
    /// Fixture topology (all by stem id, which matches [[link]] targets):
    ///   d3 → d20
    ///   d20 → d21, d20 → d3
    ///   d21 → d20, d21 → d4
    ///   d4: leaf
    ///   unlinked: isolated
    #[test]
    fn fixture_corpus_topology() {
        use crate::brain::okf::build_node_edge_lists;
        use std::path::PathBuf;

        // Use stem-fallback docs (no frontmatter) so ids == file stems,
        // matching the [[link]] targets used in the fixture content.
        let docs = vec![
            (
                PathBuf::from("src/brain/fixtures/d3.md"),
                "[[d20]] is the contract.".to_string(),
            ),
            (
                PathBuf::from("src/brain/fixtures/d20.md"),
                "Depends on [[d21]]. References [[d3]].".to_string(),
            ),
            (
                PathBuf::from("src/brain/fixtures/d21.md"),
                "See [[d20]]. Also [[d4]].".to_string(),
            ),
            (
                PathBuf::from("src/brain/fixtures/d4.md"),
                "Leaf.".to_string(),
            ),
            (
                PathBuf::from("src/brain/fixtures/unlinked.md"),
                "[[nonexistent]] is unresolved.".to_string(),
            ),
        ];
        let (nodes, edges) = build_node_edge_lists(&docs);
        let g = BrainGraph::build(nodes, edges);

        assert_eq!(g.node_count(), 5);
        // d3→d20, d20→d21, d20→d3, d21→d20, d21→d4 = 5 resolved edges;
        // unlinked→nonexistent is dropped.
        assert_eq!(g.edge_count(), 5);

        // d4 is a leaf: no successors, but has d21 as predecessor.
        let d4_succs = g.successors("d4").unwrap();
        assert!(d4_succs.is_empty());

        let d4_preds = g.predecessors("d4").unwrap();
        let pred_ids: Vec<&str> = d4_preds.iter().map(|n| n.id.as_str()).collect();
        assert!(pred_ids.contains(&"d21"));

        // unlinked is isolated.
        assert!(g.successors("unlinked").unwrap().is_empty());
        assert!(g.predecessors("unlinked").unwrap().is_empty());
    }
}
