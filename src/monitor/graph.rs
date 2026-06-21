// Builds a petgraph DAG and a left-to-right topological layout for ratatui.
//
// Edges come from the orchestrator's graph endpoint (api::client::WorkflowGraph),
// NOT from node state — `node_runs` carries no edges. Live per-node status is
// overlaid by joining `nodes` to the graph on class name (data contract §2).

use crate::api::client::WorkflowGraph;
use crate::db::workflows::{NodeState, RunStatus};
use petgraph::algo::toposort;
use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::HashMap;

pub struct GraphLayout {
    pub graph: DiGraph<String, ()>,
    /// (node_index, col, row) — col/row are grid positions for ratatui canvas
    pub positions: Vec<(usize, u16, u16)>,
    /// Live run status per node name, overlaid from `NodeState` by class name.
    pub node_states: HashMap<String, RunStatus>,
}

/// `graph` supplies the DAG shape (nodes + edges); `nodes` supplies live state
/// for the nodes that have run, joined to `graph.nodes` by class name.
pub fn build_layout(graph: &WorkflowGraph, nodes: &[NodeState]) -> GraphLayout {
    // 1. Build DiGraph: add vertices for every node mentioned in edges.
    let mut node_indices: HashMap<String, NodeIndex> = HashMap::new();
    let mut digraph: DiGraph<String, ()> = DiGraph::new();

    for (from, to) in &graph.edges {
        if !node_indices.contains_key(from) {
            let idx = digraph.add_node(from.clone());
            node_indices.insert(from.clone(), idx);
        }
        if !node_indices.contains_key(to) {
            let idx = digraph.add_node(to.clone());
            node_indices.insert(to.clone(), idx);
        }
    }

    for (from, to) in &graph.edges {
        let from_idx = node_indices[from];
        let to_idx = node_indices[to];
        digraph.add_edge(from_idx, to_idx, ());
    }

    // Add any nodes from WorkflowGraph.nodes not already present (isolated or
    // not-yet-run nodes that have no edges in the current graph snapshot).
    for node_name in &graph.nodes {
        if !node_indices.contains_key(node_name) {
            let idx = digraph.add_node(node_name.clone());
            node_indices.insert(node_name.clone(), idx);
        }
    }

    // 2. Topological sort; fall back to insertion order on cycle (should not
    //    happen with a well-formed orchestrator DAG).
    let topo_order: Vec<NodeIndex> = match toposort(&digraph, None) {
        Ok(order) => order,
        Err(_) => digraph.node_indices().collect(),
    };

    // 3. Compute depth (column) for each node.
    //    depth[n] = max(depth[pred] + 1 for all predecessors) or 0 for roots.
    let node_count = digraph.node_count();
    let mut depth: Vec<u16> = vec![0; node_count];

    for &node_idx in &topo_order {
        let pred_max = digraph
            .neighbors_directed(node_idx, petgraph::Direction::Incoming)
            .map(|pred| depth[pred.index()])
            .max();
        depth[node_idx.index()] = pred_max.map(|d| d + 1).unwrap_or(0);
    }

    // 4. Assign row positions within each column in toposort order.
    let mut col_row_counter: HashMap<u16, u16> = HashMap::new();
    let mut positions: Vec<(usize, u16, u16)> = Vec::with_capacity(node_count);

    for &node_idx in &topo_order {
        let col = depth[node_idx.index()];
        let row_counter = col_row_counter.entry(col).or_insert(0);
        let row = *row_counter;
        *row_counter += 1;
        positions.push((node_idx.index(), col, row));
    }

    // 5. Overlay live status by joining `nodes` to the graph by class name.
    let node_states: HashMap<String, RunStatus> = nodes
        .iter()
        .map(|n| (n.name.clone(), n.status.clone()))
        .collect();

    GraphLayout {
        graph: digraph,
        positions,
        node_states,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::client::WorkflowGraph;
    use crate::db::workflows::{NodeState, RunStatus};

    // ── helper ────────────────────────────────────────────────────────────────

    fn make_graph(nodes: Vec<&str>, edges: Vec<(&str, &str)>) -> WorkflowGraph {
        WorkflowGraph {
            nodes: nodes.into_iter().map(str::to_string).collect(),
            edges: edges
                .into_iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
        }
    }

    fn make_node_state(name: &str, status: RunStatus) -> NodeState {
        NodeState {
            id: name.to_string(),
            name: name.to_string(),
            status,
            depends_on: vec![],
            input: None,
            output: None,
            error: None,
            tokens_in: None,
            tokens_out: None,
            model: None,
            started_at: None,
            elapsed_secs: None,
        }
    }

    // ── linear chain ──────────────────────────────────────────────────────────

    #[test]
    fn linear_chain_produces_three_distinct_columns() {
        // A → B → C
        let graph = make_graph(vec!["A", "B", "C"], vec![("A", "B"), ("B", "C")]);
        let layout = build_layout(&graph, &[]);

        let mut cols: Vec<u16> = layout.positions.iter().map(|&(_, col, _)| col).collect();
        cols.sort_unstable();
        cols.dedup();
        assert_eq!(
            cols,
            vec![0, 1, 2],
            "linear chain must span columns 0, 1, 2"
        );
    }

    #[test]
    fn linear_chain_columns_match_depth() {
        // A → B → C: A at depth 0, B at depth 1, C at depth 2
        let graph = make_graph(vec!["A", "B", "C"], vec![("A", "B"), ("B", "C")]);
        let layout = build_layout(&graph, &[]);

        let name_to_idx: HashMap<String, usize> = layout
            .graph
            .node_indices()
            .map(|i| (layout.graph[i].clone(), i.index()))
            .collect();

        let pos_by_idx: HashMap<usize, (u16, u16)> = layout
            .positions
            .iter()
            .map(|&(idx, col, row)| (idx, (col, row)))
            .collect();

        let a_col = pos_by_idx[&name_to_idx["A"]].0;
        let b_col = pos_by_idx[&name_to_idx["B"]].0;
        let c_col = pos_by_idx[&name_to_idx["C"]].0;

        assert!(a_col < b_col, "A must be left of B");
        assert!(b_col < c_col, "B must be left of C");
        assert_eq!(a_col, 0);
        assert_eq!(b_col, 1);
        assert_eq!(c_col, 2);
    }

    // ── diamond DAG ───────────────────────────────────────────────────────────

    #[test]
    fn diamond_dag_correct_depth_assignments() {
        // A→B, A→C, B→D, C→D
        let graph = make_graph(
            vec!["A", "B", "C", "D"],
            vec![("A", "B"), ("A", "C"), ("B", "D"), ("C", "D")],
        );
        let layout = build_layout(&graph, &[]);

        let name_to_idx: HashMap<String, usize> = layout
            .graph
            .node_indices()
            .map(|i| (layout.graph[i].clone(), i.index()))
            .collect();

        let pos_by_idx: HashMap<usize, (u16, u16)> = layout
            .positions
            .iter()
            .map(|&(idx, col, row)| (idx, (col, row)))
            .collect();

        let a_col = pos_by_idx[&name_to_idx["A"]].0;
        let b_col = pos_by_idx[&name_to_idx["B"]].0;
        let c_col = pos_by_idx[&name_to_idx["C"]].0;
        let d_col = pos_by_idx[&name_to_idx["D"]].0;

        assert_eq!(a_col, 0, "A is at depth 0");
        assert_eq!(b_col, 1, "B is at depth 1");
        assert_eq!(c_col, 1, "C is at depth 1 (parallel with B)");
        assert_eq!(d_col, 2, "D is at depth 2 (depends on both B and C)");
    }

    #[test]
    fn diamond_dag_b_and_c_share_column_different_rows() {
        let graph = make_graph(
            vec!["A", "B", "C", "D"],
            vec![("A", "B"), ("A", "C"), ("B", "D"), ("C", "D")],
        );
        let layout = build_layout(&graph, &[]);

        let name_to_idx: HashMap<String, usize> = layout
            .graph
            .node_indices()
            .map(|i| (layout.graph[i].clone(), i.index()))
            .collect();

        let pos_by_idx: HashMap<usize, (u16, u16)> = layout
            .positions
            .iter()
            .map(|&(idx, col, row)| (idx, (col, row)))
            .collect();

        let b_col = pos_by_idx[&name_to_idx["B"]].0;
        let c_col = pos_by_idx[&name_to_idx["C"]].0;
        let b_row = pos_by_idx[&name_to_idx["B"]].1;
        let c_row = pos_by_idx[&name_to_idx["C"]].1;

        assert_eq!(b_col, c_col, "B and C must be in the same column");
        assert_ne!(b_row, c_row, "B and C must occupy different rows");
    }

    // ── isolated node ─────────────────────────────────────────────────────────

    #[test]
    fn isolated_node_position_is_col0_row0() {
        let graph = make_graph(vec!["Alone"], vec![]);
        let layout = build_layout(&graph, &[]);

        assert_eq!(layout.positions.len(), 1);
        let (_idx, col, row) = layout.positions[0];
        assert_eq!(col, 0, "isolated node must be in column 0");
        assert_eq!(row, 0, "isolated node must be in row 0");
    }

    #[test]
    fn empty_graph_produces_empty_positions() {
        let graph = make_graph(vec![], vec![]);
        let layout = build_layout(&graph, &[]);
        assert!(layout.positions.is_empty());
    }

    // ── live-state overlay ────────────────────────────────────────────────────

    #[test]
    fn overlay_assigns_correct_status_to_named_node() {
        let graph = make_graph(vec!["NodeA", "NodeB"], vec![("NodeA", "NodeB")]);
        let live_nodes = vec![
            make_node_state("NodeA", RunStatus::Success),
            make_node_state("NodeB", RunStatus::Running),
        ];
        let layout = build_layout(&graph, &live_nodes);

        assert_eq!(
            layout.node_states.get("NodeA"),
            Some(&RunStatus::Success),
            "NodeA should have Success status"
        );
        assert_eq!(
            layout.node_states.get("NodeB"),
            Some(&RunStatus::Running),
            "NodeB should have Running status"
        );
    }

    #[test]
    fn overlay_missing_node_has_no_status() {
        // A node in the graph that has not run yet has no entry in node_states
        let graph = make_graph(vec!["NodeA", "NodeB"], vec![("NodeA", "NodeB")]);
        let live_nodes = vec![make_node_state("NodeA", RunStatus::Success)];
        let layout = build_layout(&graph, &live_nodes);

        assert!(
            layout.node_states.get("NodeB").is_none(),
            "NodeB has not run and should have no status entry"
        );
    }

    #[test]
    fn overlay_all_four_statuses_round_trip() {
        let graph = make_graph(
            vec!["A", "B", "C", "D"],
            vec![("A", "B"), ("A", "C"), ("A", "D")],
        );
        let live_nodes = vec![
            make_node_state("A", RunStatus::Success),
            make_node_state("B", RunStatus::Failed),
            make_node_state("C", RunStatus::Running),
            make_node_state("D", RunStatus::Pending),
        ];
        let layout = build_layout(&graph, &live_nodes);

        assert_eq!(layout.node_states["A"], RunStatus::Success);
        assert_eq!(layout.node_states["B"], RunStatus::Failed);
        assert_eq!(layout.node_states["C"], RunStatus::Running);
        assert_eq!(layout.node_states["D"], RunStatus::Pending);
    }

    // ── node count ────────────────────────────────────────────────────────────

    #[test]
    fn positions_contains_one_entry_per_node() {
        // A→B, C is isolated (not in edges)
        let graph = make_graph(vec!["A", "B", "C"], vec![("A", "B")]);
        let layout = build_layout(&graph, &[]);
        assert_eq!(layout.positions.len(), 3, "one position entry per node");
    }

    #[test]
    fn isolated_node_added_from_graph_nodes_list() {
        // Only edges reference A and B; C is in graph.nodes but not edges
        let graph = make_graph(vec!["A", "B", "C"], vec![("A", "B")]);
        let layout = build_layout(&graph, &[]);

        let node_names: Vec<String> = layout
            .graph
            .node_indices()
            .map(|i| layout.graph[i].clone())
            .collect();
        assert!(
            node_names.contains(&"C".to_string()),
            "C must be in the DiGraph"
        );
    }
}
