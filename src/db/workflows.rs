// Queries: list active runs, get run state, get node inputs/outputs/errors.
// Reads from the same PostgreSQL the Python orchestrator writes to.
// See planning/rust-project-ideas/plans/bastion.md for schema expectations.

use anyhow::Result;

pub async fn list_active_runs(_db_url: &str) -> Result<Vec<WorkflowRun>> {
    todo!("Phase 1: query workflow_runs table for active runs")
}

pub async fn get_run_state(_db_url: &str, _run_id: &str) -> Result<WorkflowRun> {
    todo!("Phase 1: query node_states for a specific run")
}

#[derive(Debug)]
pub struct WorkflowRun {
    pub id: String,
    pub workflow_name: String,
    pub status: RunStatus,
    pub nodes: Vec<NodeState>,
    pub started_at: Option<String>,
    pub elapsed_secs: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RunStatus {
    Running,
    Success,
    Failed,
    Pending,
}

#[derive(Debug)]
pub struct NodeState {
    pub id: String,
    pub name: String,
    pub status: RunStatus,
    pub depends_on: Vec<String>,
    pub input: Option<serde_json::Value>,
    pub output: Option<serde_json::Value>,
    pub error: Option<String>,
    pub tokens_in: Option<u64>,
    pub tokens_out: Option<u64>,
    pub started_at: Option<String>,
    pub elapsed_secs: Option<u64>,
}
