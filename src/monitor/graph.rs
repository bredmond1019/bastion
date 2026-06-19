// Converts WorkflowRun node state into a petgraph DAG and computes
// a left-to-right topological layout for ratatui canvas rendering.

use petgraph::graph::DiGraph;
use crate::db::workflows::NodeState;

pub struct GraphLayout {
    pub graph: DiGraph<String, ()>,
    /// (node_index, col, row) — col/row are grid positions for ratatui canvas
    pub positions: Vec<(usize, u16, u16)>,
}

pub fn build_layout(_nodes: &[NodeState]) -> GraphLayout {
    todo!("Phase 1: topological sort → assign col by depth, row by sibling index")
}
