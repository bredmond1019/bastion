// Builds a petgraph DAG and a left-to-right topological layout for ratatui.
//
// Edges come from the orchestrator's graph endpoint (api::client::WorkflowGraph),
// NOT from node state — `node_runs` carries no edges. Live per-node status is
// overlaid by joining `nodes` to the graph on class name (data contract §2).

use crate::api::client::WorkflowGraph;
use crate::db::workflows::NodeState;
use petgraph::graph::DiGraph;

pub struct GraphLayout {
    pub graph: DiGraph<String, ()>,
    /// (node_index, col, row) — col/row are grid positions for ratatui canvas
    pub positions: Vec<(usize, u16, u16)>,
}

/// `graph` supplies the DAG shape (nodes + edges); `nodes` supplies live state
/// for the nodes that have run, joined to `graph.nodes` by class name.
pub fn build_layout(_graph: &WorkflowGraph, _nodes: &[NodeState]) -> GraphLayout {
    todo!("Phase 1: build DiGraph from graph.edges → topological col/row layout")
}
