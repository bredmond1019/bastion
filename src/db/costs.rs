// Queries: token usage aggregation by workflow and time window.

use anyhow::Result;

pub async fn get_cost_summary(_db_url: &str, _window: &str) -> Result<Vec<WorkflowCost>> {
    todo!("Phase 2: aggregate token usage from the orchestrator's token log table")
}

#[derive(Debug)]
pub struct WorkflowCost {
    pub workflow_name: String,
    pub run_count: u64,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub estimated_usd: f64,
}
