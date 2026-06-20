// Reads run state from the orchestrator's PostgreSQL (read-only; observer, D2).
//
// There are NO relational `workflow_runs` / `node_states` tables. All state is
// JSON in the `events` table: parse `events.task_context.node_runs` (per-node
// status/timing/error/input/usage) and `events.task_context.nodes` (per-node
// output), joined to DAG edges from `GET /workflows/{type}/graph` by class name.
// Canonical contract: ../docs/data-contract.md (pinned v1.0.0).

use anyhow::Result;
use serde::Deserialize;

pub async fn list_active_runs(_db_url: &str) -> Result<Vec<WorkflowRun>> {
    todo!("Phase 1: scan `events` for rows whose node_runs aren't all terminal")
}

pub async fn get_run_state(_db_url: &str, _run_id: &str) -> Result<WorkflowRun> {
    todo!("Phase 1: load one `events` row, parse task_context into a WorkflowRun")
}

/// Assembled per run from one `events` row. `status` is derived by aggregating
/// `node_runs` (there is no top-level status column in contract v1.0.0).
#[derive(Debug)]
pub struct WorkflowRun {
    pub id: String,
    pub workflow_name: String,
    pub status: RunStatus,
    pub nodes: Vec<NodeState>,
    pub started_at: Option<String>,
    pub elapsed_secs: Option<u64>,
}

/// Mirrors the contract's `node_runs[*].status` strings. Deserializes directly
/// from `pending|running|success|failed`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Running,
    Success,
    Failed,
    Pending,
}

/// Assembled per node by joining `node_runs[name]` (status/timing/error/input/
/// usage) + `nodes[name]` (output) + graph-endpoint edges (`depends_on`).
/// Not deserialized directly — the three sources are merged by class name.
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
    pub model: Option<String>,
    pub started_at: Option<String>,
    pub elapsed_secs: Option<u64>,
}
