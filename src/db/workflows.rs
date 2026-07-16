// Reads run state from the orchestrator's PostgreSQL (read-only; observer, D2).
//
// There are NO relational `workflow_runs` / `node_states` tables. All state is
// JSON in the `events` table: parse `events.task_context.node_runs` (per-node
// status/timing/error/input/usage) and `events.task_context.nodes` (per-node
// output), joined to DAG edges from `GET /workflows/{type}/graph` by class name.
// Canonical contract: ../docs/data-contract.md (pinned v1.1.0).
//
// v1.1.0 adds two run-level `task_context.metadata` annotations that this module
// consumes directly (BA.7.C): `metadata.cancellation.cancelled` and
// `metadata.budget.halted` (+ `.reason`). Per the contract, consumers must read
// these `metadata` keys — never infer from `NodeRunStatus` — to distinguish a
// cancelled or budget-halted run from a plain `failed` run. `NodeRunStatus`
// (mirrored here as `RunStatus` for per-node values) stays exactly
// `pending|running|success|failed`; `Cancelled`/`BudgetHalted` are `RunStatus`
// variants used only at the run level, derived from `metadata`, never
// deserialized off an individual node's wire status.

use anyhow::{Context, Result};
use serde::Deserialize;
use sqlx::postgres::PgPoolOptions;

/// Return all workflow runs that have at least one non-terminal node (i.e., any
/// node whose status is `pending` or `running`). Read-only; never writes (D2).
///
/// Implementation note: contract v1.0.0 has no indexed status column on `events`,
/// so this function fetches all rows and filters in Rust after parsing.
pub async fn list_active_runs(db_url: &str) -> Result<Vec<WorkflowRun>> {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(db_url)
        .await
        .context("failed to connect to PostgreSQL")?;

    let rows =
        sqlx::query_as::<_, EventRow>("SELECT id::text, workflow_type, task_context FROM events")
            .fetch_all(&pool)
            .await
            .context("failed to query events table")?;

    let mut active = Vec::new();
    for row in rows {
        let run = parse_event_row(row)?;
        // Keep only runs that have at least one non-terminal node.
        let is_active = run
            .nodes
            .iter()
            .any(|n| matches!(n.status, RunStatus::Pending | RunStatus::Running));
        if is_active {
            active.push(run);
        }
    }
    Ok(active)
}

/// Load a single workflow run by its `events.id`. Read-only; never writes (D2).
pub async fn get_run_state(db_url: &str, run_id: &str) -> Result<WorkflowRun> {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(db_url)
        .await
        .context("failed to connect to PostgreSQL")?;

    let row = sqlx::query_as::<_, EventRow>(
        "SELECT id::text, workflow_type, task_context FROM events WHERE id = $1::uuid",
    )
    .bind(run_id)
    .fetch_one(&pool)
    .await
    .with_context(|| format!("no events row found for id '{run_id}'"))?;

    parse_event_row(row)
}

// ── internal helpers ──────────────────────────────────────────────────────────

/// Raw columns fetched from the `events` table.
#[derive(sqlx::FromRow)]
pub(crate) struct EventRow {
    pub(crate) id: String,
    pub(crate) workflow_type: String,
    pub(crate) task_context: Option<serde_json::Value>,
}

/// Parse one `EventRow` into a `WorkflowRun` using the Task-2 parsing layer.
pub(crate) fn parse_event_row(row: EventRow) -> Result<WorkflowRun> {
    let tc = row
        .task_context
        .unwrap_or_else(|| serde_json::json!({ "node_runs": {}, "nodes": {} }));
    let nodes = parse_task_context(&tc)
        .with_context(|| format!("failed to parse task_context for run '{}'", row.id))?;

    // Derive started_at as the minimum non-null started_at across all nodes.
    let started_at = nodes
        .iter()
        .filter_map(|n| n.started_at.as_deref())
        .min()
        .map(str::to_string);

    // `metadata` is absent on every run written before v1.1.0 (and tolerated as
    // absent by the contract going forward) — default to an empty object so
    // `derive_run_status` sees "no annotation" rather than erroring.
    let metadata = tc
        .get("metadata")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let (status, budget_halt) = derive_run_status(&nodes, &metadata);

    Ok(WorkflowRun {
        id: row.id,
        workflow_name: row.workflow_type,
        status,
        budget_halt,
        nodes,
        started_at,
        elapsed_secs: None, // requires wall-clock subtraction; deferred to display layer
    })
}

/// Parse `task_context` JSON (the `task_context` column from an `events` row)
/// into a `Vec<NodeState>`.
///
/// The join: for each key in `task_context.node_runs`, populate `NodeState`
/// fields from `node_runs[name]` (status, error, input, usage.*) and
/// `task_context.nodes[name]` (output). `depends_on` is populated by the
/// caller from the graph endpoint, not from `task_context`.
pub(crate) fn parse_task_context(task_context: &serde_json::Value) -> Result<Vec<NodeState>> {
    let node_runs = task_context
        .get("node_runs")
        .and_then(|v| v.as_object())
        .context("task_context missing node_runs object")?;

    let nodes_map = task_context.get("nodes").and_then(|v| v.as_object());

    let mut result = Vec::with_capacity(node_runs.len());

    for (name, run_val) in node_runs {
        let status: RunStatus = serde_json::from_value(
            run_val
                .get("status")
                .cloned()
                .context("node_run missing status field")?,
        )
        .with_context(|| format!("invalid status value for node '{name}'"))?;

        let error = run_val
            .get("error")
            .and_then(|v| v.as_str())
            .map(str::to_string);

        let input = run_val
            .get("input")
            .and_then(|v| if v.is_null() { None } else { Some(v.clone()) });

        // usage may be null for non-LLM nodes
        let usage = run_val.get("usage");
        let (tokens_in, tokens_out, model) = match usage.and_then(|u| u.as_object()) {
            Some(u) => {
                let ti = u.get("input_tokens").and_then(|v| v.as_u64());
                let to = u.get("output_tokens").and_then(|v| v.as_u64());
                let m = u.get("model").and_then(|v| v.as_str()).map(str::to_string);
                (ti, to, m)
            }
            None => (None, None, None),
        };

        let started_at = run_val
            .get("started_at")
            .and_then(|v| v.as_str())
            .map(str::to_string);

        // output comes from the parallel `nodes[name]` map, not from node_runs
        let output = nodes_map
            .and_then(|m| m.get(name.as_str()))
            .and_then(|node| node.get("output"))
            .and_then(|v| if v.is_null() { None } else { Some(v.clone()) });

        result.push(NodeState {
            id: name.clone(),
            name: name.clone(),
            status,
            depends_on: vec![],
            input,
            output,
            error,
            tokens_in,
            tokens_out,
            model,
            started_at,
            elapsed_secs: None, // derived from timestamps; not stored in contract v1.0.0
        });
    }

    Ok(result)
}

/// Reads `metadata.cancellation.cancelled` (contract v1.1.0 §5). Absent-tolerant:
/// a missing `cancellation` key, a `cancellation` that isn't an object, or a
/// missing/non-bool `cancelled` field all read as "not cancelled" rather than
/// erroring — this is deliberately permissive so a malformed or partial
/// `metadata` object degrades to the pre-v1.1.0 node-based derivation instead
/// of panicking or propagating an error.
fn metadata_is_cancelled(metadata: &serde_json::Value) -> bool {
    metadata
        .get("cancellation")
        .and_then(|c| c.get("cancelled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Reads `metadata.budget.halted` + `.reason` (contract v1.1.0 §5), mirroring
/// the shape engine-rs's `stamp_budget_halt` writes:
/// `{ "budget": { "halted": true, "reason": { "cap", "spent", "limit" } } }`.
///
/// Returns `None` whenever the run is not halted, or whenever `halted` is
/// present but the `reason` is missing or malformed (unknown `cap`, wrong
/// numeric type) — absent-tolerant by the same reasoning as
/// `metadata_is_cancelled`: a partial or malformed `metadata.budget` object
/// must not crash or falsely halt bastion's view of the run.
fn metadata_budget_halt(metadata: &serde_json::Value) -> Option<BudgetHalt> {
    let budget = metadata.get("budget")?;
    let halted = budget
        .get("halted")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !halted {
        return None;
    }
    let reason = budget.get("reason")?;
    let cap = reason.get("cap").and_then(|v| v.as_str())?;
    match cap {
        "max_total_tokens" => {
            let spent = reason.get("spent").and_then(|v| v.as_u64())?;
            let limit = reason.get("limit").and_then(|v| v.as_u64())?;
            Some(BudgetHalt::TotalTokens { spent, limit })
        }
        "max_cost_usd" => {
            let spent = reason.get("spent").and_then(|v| v.as_f64())?;
            let limit = reason.get("limit").and_then(|v| v.as_f64())?;
            Some(BudgetHalt::CostUsd { spent, limit })
        }
        _ => None,
    }
}

/// Derive an overall `RunStatus` from `task_context.metadata` (v1.1.0
/// annotations) and, failing those, the aggregate of node statuses (the
/// pre-v1.1.0 behavior, unchanged for every run that carries neither key).
///
/// Priority order:
/// 1. `metadata.budget.halted == true` (with a decodable `reason`) → `BudgetHalted`,
///    carrying the breached cap/spent/limit.
/// 2. `metadata.cancellation.cancelled == true`                    → `Cancelled`.
/// 3. Otherwise, the existing node-aggregate rules:
///    - any node `running`                        → `Running`
///    - any node `pending`, none `running`         → `Pending`
///    - all nodes terminal, at least one `failed`  → `Failed`
///    - all nodes terminal, all `success`          → `Success`
///
/// Budget is checked before cancellation: a run can only be halted by the
/// pre-dispatch budget gate before the operator has a chance to cancel it, so
/// on the rare run where both markers are present, the budget halt is the
/// more informative — and, timeline-wise, likely the causal — terminal state.
pub(crate) fn derive_run_status(
    nodes: &[NodeState],
    metadata: &serde_json::Value,
) -> (RunStatus, Option<BudgetHalt>) {
    if let Some(halt) = metadata_budget_halt(metadata) {
        return (RunStatus::BudgetHalted, Some(halt));
    }
    if metadata_is_cancelled(metadata) {
        return (RunStatus::Cancelled, None);
    }

    let mut has_running = false;
    let mut has_pending = false;
    let mut has_failed = false;

    for node in nodes {
        match node.status {
            RunStatus::Running => has_running = true,
            RunStatus::Pending => has_pending = true,
            RunStatus::Failed => has_failed = true,
            RunStatus::Success | RunStatus::Cancelled | RunStatus::BudgetHalted => {}
        }
    }

    let status = if has_running {
        RunStatus::Running
    } else if has_pending {
        RunStatus::Pending
    } else if has_failed {
        RunStatus::Failed
    } else {
        RunStatus::Success
    };
    (status, None)
}

/// Assembled per run from one `events` row. `status` is derived first from
/// `task_context.metadata`'s v1.1.0 cancellation/budget annotations and, absent
/// those, by aggregating `node_runs` (there is no top-level status column).
/// `budget_halt` carries the breach detail (cap/spent/limit) whenever
/// `status == RunStatus::BudgetHalted`; it is always `None` otherwise.
#[derive(Debug, Clone)]
pub struct WorkflowRun {
    pub id: String,
    pub workflow_name: String,
    pub status: RunStatus,
    pub budget_halt: Option<BudgetHalt>,
    pub nodes: Vec<NodeState>,
    pub started_at: Option<String>,
    pub elapsed_secs: Option<u64>,
}

/// Per-node values mirror the contract's `node_runs[*].status` strings and
/// deserialize directly from `pending|running|success|failed` — §6's
/// `NodeRunStatus` gains no new variants (contract v1.1.0).
///
/// `Cancelled` and `BudgetHalted` are **run-level-only** states: they are never
/// produced by deserializing an individual node's wire status (there is no
/// such string on the wire), only by [`derive_run_status`] reading
/// `task_context.metadata`'s v1.1.0 annotations. Keeping them in the same enum
/// as the per-node states lets `WorkflowRun.status` and `NodeState.status`
/// share one type, matching how the rest of this module already treats
/// run-level status as "the same shape as a node's, aggregated".
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Running,
    Success,
    Failed,
    Pending,
    /// Run-level only — derived from `metadata.cancellation.cancelled == true`.
    #[serde(skip_deserializing)]
    Cancelled,
    /// Run-level only — derived from `metadata.budget.halted == true`. Detail
    /// (cap/spent/limit) lives on `WorkflowRun.budget_halt`, not on this variant,
    /// so `RunStatus` stays a plain, comparable/copyable-shaped enum.
    #[serde(skip_deserializing)]
    BudgetHalted,
}

/// The breach detail behind `WorkflowRun.status == RunStatus::BudgetHalted`,
/// read from `metadata.budget.reason` (contract v1.1.0 §5). Mirrors
/// engine-rs's `engine_core::budget::BudgetHaltReason` wording so the Console
/// and the Engine describe the same breach identically (vendor-don't-depend,
/// D24 — not imported, deliberately kept in sync by convention).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BudgetHalt {
    TotalTokens { spent: u64, limit: u64 },
    CostUsd { spent: f64, limit: f64 },
}

impl BudgetHalt {
    /// The contract-friendly lowercase snake_case cap name, matching
    /// `metadata.budget.reason.cap` on the wire.
    pub fn cap_name(&self) -> &'static str {
        match self {
            BudgetHalt::TotalTokens { .. } => "max_total_tokens",
            BudgetHalt::CostUsd { .. } => "max_cost_usd",
        }
    }
}

/// Assembled per node by joining `node_runs[name]` (status/timing/error/input/
/// usage) + `nodes[name]` (output) + graph-endpoint edges (`depends_on`).
/// Not deserialized directly — the three sources are merged by class name.
#[derive(Debug, Clone)]
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

#[cfg(test)]
mod tests {
    use super::*;

    // Load fixture files at test time using include_str! (paths relative to this
    // source file). Keeps tests hermetic — no filesystem I/O at runtime.
    const IN_PROGRESS_FIXTURE: &str = include_str!("fixtures/in_progress_run.json");
    const COMPLETED_FIXTURE: &str = include_str!("fixtures/completed_run.json");
    const CANCELLED_FIXTURE: &str = include_str!("fixtures/cancelled_run.json");
    const BUDGET_HALTED_TOKENS_FIXTURE: &str =
        include_str!("fixtures/budget_halted_tokens_run.json");
    const BUDGET_HALTED_COST_FIXTURE: &str = include_str!("fixtures/budget_halted_cost_run.json");
    const MALFORMED_METADATA_FIXTURE: &str = include_str!("fixtures/malformed_metadata_run.json");

    // ── RunStatus deserialization ─────────────────────────────────────────────

    #[test]
    fn run_status_deserializes_pending() {
        let s: RunStatus = serde_json::from_str("\"pending\"").unwrap();
        assert_eq!(s, RunStatus::Pending);
    }

    #[test]
    fn run_status_deserializes_running() {
        let s: RunStatus = serde_json::from_str("\"running\"").unwrap();
        assert_eq!(s, RunStatus::Running);
    }

    #[test]
    fn run_status_deserializes_success() {
        let s: RunStatus = serde_json::from_str("\"success\"").unwrap();
        assert_eq!(s, RunStatus::Success);
    }

    #[test]
    fn run_status_deserializes_failed() {
        let s: RunStatus = serde_json::from_str("\"failed\"").unwrap();
        assert_eq!(s, RunStatus::Failed);
    }

    #[test]
    fn run_status_rejects_unknown_string() {
        let result: Result<RunStatus, _> = serde_json::from_str("\"unknown\"");
        assert!(result.is_err());
    }

    // ── in-progress fixture parsing ───────────────────────────────────────────

    #[test]
    fn in_progress_fixture_parses_node_count() {
        let tc: serde_json::Value = serde_json::from_str(IN_PROGRESS_FIXTURE).unwrap();
        let nodes = parse_task_context(&tc).unwrap();
        assert_eq!(nodes.len(), 5, "in-progress fixture has 5 nodes");
    }

    #[test]
    fn in_progress_fixture_has_mixed_statuses() {
        let tc: serde_json::Value = serde_json::from_str(IN_PROGRESS_FIXTURE).unwrap();
        let nodes = parse_task_context(&tc).unwrap();

        let statuses: Vec<&RunStatus> = nodes.iter().map(|n| &n.status).collect();
        assert!(
            statuses.contains(&&RunStatus::Success),
            "expected at least one success node"
        );
        assert!(
            statuses.contains(&&RunStatus::Running),
            "expected at least one running node"
        );
        assert!(
            statuses.contains(&&RunStatus::Pending),
            "expected at least one pending node"
        );
    }

    #[test]
    fn in_progress_fixture_derived_status_is_running() {
        let tc: serde_json::Value = serde_json::from_str(IN_PROGRESS_FIXTURE).unwrap();
        let nodes = parse_task_context(&tc).unwrap();
        assert_eq!(
            derive_run_status(&nodes, &serde_json::json!({})).0,
            RunStatus::Running
        );
    }

    #[test]
    fn in_progress_fixture_null_usage_produces_none_fields() {
        let tc: serde_json::Value = serde_json::from_str(IN_PROGRESS_FIXTURE).unwrap();
        let nodes = parse_task_context(&tc).unwrap();

        // DataIngestionNode has null usage in the fixture
        let data_node = nodes
            .iter()
            .find(|n| n.name == "DataIngestionNode")
            .expect("DataIngestionNode should be present");

        assert!(
            data_node.tokens_in.is_none(),
            "null usage → tokens_in must be None"
        );
        assert!(
            data_node.tokens_out.is_none(),
            "null usage → tokens_out must be None"
        );
        assert!(data_node.model.is_none(), "null usage → model must be None");
    }

    #[test]
    fn in_progress_fixture_non_null_usage_populated() {
        let tc: serde_json::Value = serde_json::from_str(IN_PROGRESS_FIXTURE).unwrap();
        let nodes = parse_task_context(&tc).unwrap();

        // EmbeddingNode has usage with input_tokens=512, output_tokens=0
        let embed_node = nodes
            .iter()
            .find(|n| n.name == "EmbeddingNode")
            .expect("EmbeddingNode should be present");

        assert_eq!(embed_node.tokens_in, Some(512));
        assert_eq!(embed_node.tokens_out, Some(0));
        assert_eq!(embed_node.model.as_deref(), Some("text-embedding-3-small"));
    }

    #[test]
    fn in_progress_fixture_output_joined_from_nodes_map() {
        let tc: serde_json::Value = serde_json::from_str(IN_PROGRESS_FIXTURE).unwrap();
        let nodes = parse_task_context(&tc).unwrap();

        // DataIngestionNode has non-null output in nodes[name]
        let data_node = nodes
            .iter()
            .find(|n| n.name == "DataIngestionNode")
            .expect("DataIngestionNode should be present");

        let output = data_node.output.as_ref().expect("should have output");
        assert_eq!(output["documents_loaded"], 3);
    }

    #[test]
    fn in_progress_fixture_null_output_is_none() {
        let tc: serde_json::Value = serde_json::from_str(IN_PROGRESS_FIXTURE).unwrap();
        let nodes = parse_task_context(&tc).unwrap();

        // LLMSummaryNode is running — output is null in nodes map
        let llm_node = nodes
            .iter()
            .find(|n| n.name == "LLMSummaryNode")
            .expect("LLMSummaryNode should be present");

        assert!(
            llm_node.output.is_none(),
            "null nodes[name].output should map to None"
        );
    }

    #[test]
    fn in_progress_fixture_pending_node_started_at_is_none() {
        let tc: serde_json::Value = serde_json::from_str(IN_PROGRESS_FIXTURE).unwrap();
        let nodes = parse_task_context(&tc).unwrap();

        let pending_node = nodes
            .iter()
            .find(|n| n.status == RunStatus::Pending)
            .expect("should have a pending node");

        assert!(
            pending_node.started_at.is_none(),
            "pending node started_at (null in fixture) should be None"
        );
    }

    // ── completed fixture parsing ─────────────────────────────────────────────

    #[test]
    fn completed_fixture_parses_node_count() {
        let tc: serde_json::Value = serde_json::from_str(COMPLETED_FIXTURE).unwrap();
        let nodes = parse_task_context(&tc).unwrap();
        assert_eq!(nodes.len(), 5, "completed fixture has 5 nodes");
    }

    #[test]
    fn completed_fixture_derived_status_is_failed() {
        let tc: serde_json::Value = serde_json::from_str(COMPLETED_FIXTURE).unwrap();
        let nodes = parse_task_context(&tc).unwrap();
        // Has both success and failed nodes → Failed
        assert_eq!(
            derive_run_status(&nodes, &serde_json::json!({})).0,
            RunStatus::Failed
        );
    }

    #[test]
    fn completed_fixture_failed_node_has_error_message() {
        let tc: serde_json::Value = serde_json::from_str(COMPLETED_FIXTURE).unwrap();
        let nodes = parse_task_context(&tc).unwrap();

        let validation_node = nodes
            .iter()
            .find(|n| n.name == "ValidationNode")
            .expect("ValidationNode should be present");

        assert_eq!(validation_node.status, RunStatus::Failed);
        assert!(
            validation_node.error.is_some(),
            "failed node must have an error string"
        );
        assert!(
            validation_node
                .error
                .as_deref()
                .unwrap()
                .contains("schema mismatch"),
            "error message should describe the failure"
        );
    }

    #[test]
    fn completed_fixture_success_node_llm_usage_populated() {
        let tc: serde_json::Value = serde_json::from_str(COMPLETED_FIXTURE).unwrap();
        let nodes = parse_task_context(&tc).unwrap();

        let llm_node = nodes
            .iter()
            .find(|n| n.name == "LLMSummaryNode")
            .expect("LLMSummaryNode should be present");

        assert_eq!(llm_node.status, RunStatus::Success);
        assert_eq!(llm_node.tokens_in, Some(2048));
        assert_eq!(llm_node.tokens_out, Some(256));
        assert_eq!(llm_node.model.as_deref(), Some("claude-3-5-haiku-20241022"));
    }

    #[test]
    fn completed_fixture_success_node_output_present() {
        let tc: serde_json::Value = serde_json::from_str(COMPLETED_FIXTURE).unwrap();
        let nodes = parse_task_context(&tc).unwrap();

        let llm_node = nodes
            .iter()
            .find(|n| n.name == "LLMSummaryNode")
            .expect("LLMSummaryNode should be present");

        let output = llm_node.output.as_ref().expect("should have output");
        assert!(output["summary"].is_string());
    }

    // ── derive_run_status edge cases ──────────────────────────────────────────

    #[test]
    fn derive_status_all_success() {
        let nodes = vec![
            make_node("A", RunStatus::Success),
            make_node("B", RunStatus::Success),
        ];
        assert_eq!(
            derive_run_status(&nodes, &serde_json::json!({})).0,
            RunStatus::Success
        );
    }

    #[test]
    fn derive_status_all_failed() {
        let nodes = vec![
            make_node("A", RunStatus::Failed),
            make_node("B", RunStatus::Failed),
        ];
        assert_eq!(
            derive_run_status(&nodes, &serde_json::json!({})).0,
            RunStatus::Failed
        );
    }

    #[test]
    fn derive_status_running_takes_priority_over_pending() {
        let nodes = vec![
            make_node("A", RunStatus::Success),
            make_node("B", RunStatus::Running),
            make_node("C", RunStatus::Pending),
        ];
        assert_eq!(
            derive_run_status(&nodes, &serde_json::json!({})).0,
            RunStatus::Running
        );
    }

    #[test]
    fn derive_status_pending_when_no_running() {
        let nodes = vec![
            make_node("A", RunStatus::Success),
            make_node("B", RunStatus::Pending),
        ];
        assert_eq!(
            derive_run_status(&nodes, &serde_json::json!({})).0,
            RunStatus::Pending
        );
    }

    #[test]
    fn derive_status_failed_when_mixed_terminal() {
        let nodes = vec![
            make_node("A", RunStatus::Success),
            make_node("B", RunStatus::Failed),
        ];
        assert_eq!(
            derive_run_status(&nodes, &serde_json::json!({})).0,
            RunStatus::Failed
        );
    }

    #[test]
    fn derive_status_running_takes_priority_over_failed() {
        // A node can be failed while another is still running (e.g. parallel branches)
        let nodes = vec![
            make_node("A", RunStatus::Failed),
            make_node("B", RunStatus::Running),
        ];
        assert_eq!(
            derive_run_status(&nodes, &serde_json::json!({})).0,
            RunStatus::Running
        );
    }

    #[test]
    fn parse_returns_error_on_missing_node_runs() {
        let bad_json = serde_json::json!({ "nodes": {} });
        let result = parse_task_context(&bad_json);
        assert!(result.is_err(), "should fail without node_runs key");
    }

    // ── metadata.cancellation / metadata.budget derivation (contract v1.1.0) ──
    //
    // Fixtures are hand-authored to mirror exactly what engine-rs's
    // `stamp_cancelled` (`crates/engine-core/src/cancellation.rs`) and
    // `stamp_budget_halt` (`crates/engine-core/src/workflow.rs`) write —
    // `{ "cancellation": { "cancelled": true, "at": <iso8601> } }` and
    // `{ "budget": { "halted": true, "reason": { "cap", "spent", "limit" } } }`
    // respectively — rather than a hand-authored guess at the shape.
    // engine-contract carries no fixture files of its own to source these from.

    #[test]
    fn cancelled_fixture_derives_cancelled_status() {
        let tc: serde_json::Value = serde_json::from_str(CANCELLED_FIXTURE).unwrap();
        let nodes = parse_task_context(&tc).unwrap();
        let metadata = tc.get("metadata").cloned().unwrap();
        let (status, budget_halt) = derive_run_status(&nodes, &metadata);
        assert_eq!(status, RunStatus::Cancelled);
        assert!(
            budget_halt.is_none(),
            "a cancelled run carries no budget_halt detail"
        );
    }

    #[test]
    fn cancelled_status_is_distinct_from_failed() {
        // A cancelled run's nodes (success/running/pending, no failed node)
        // would otherwise derive to `Running` — confirm the metadata
        // annotation overrides that, and that it is not conflated with
        // `Failed` either.
        assert_ne!(RunStatus::Cancelled, RunStatus::Failed);
        assert_ne!(RunStatus::Cancelled, RunStatus::Running);
    }

    #[test]
    fn budget_halted_tokens_fixture_derives_status_and_reason() {
        let tc: serde_json::Value = serde_json::from_str(BUDGET_HALTED_TOKENS_FIXTURE).unwrap();
        let nodes = parse_task_context(&tc).unwrap();
        let metadata = tc.get("metadata").cloned().unwrap();
        let (status, budget_halt) = derive_run_status(&nodes, &metadata);
        assert_eq!(status, RunStatus::BudgetHalted);
        assert_eq!(
            budget_halt,
            Some(BudgetHalt::TotalTokens {
                spent: 100_000,
                limit: 100_000,
            })
        );
        assert_eq!(budget_halt.unwrap().cap_name(), "max_total_tokens");
    }

    #[test]
    fn budget_halted_cost_fixture_derives_status_and_reason() {
        let tc: serde_json::Value = serde_json::from_str(BUDGET_HALTED_COST_FIXTURE).unwrap();
        let nodes = parse_task_context(&tc).unwrap();
        let metadata = tc.get("metadata").cloned().unwrap();
        let (status, budget_halt) = derive_run_status(&nodes, &metadata);
        assert_eq!(status, RunStatus::BudgetHalted);
        match budget_halt {
            Some(BudgetHalt::CostUsd { spent, limit }) => {
                assert!((spent - 5.75).abs() < f64::EPSILON);
                assert!((limit - 5.0).abs() < f64::EPSILON);
            }
            other => panic!("expected CostUsd halt reason, got {other:?}"),
        }
        assert_eq!(budget_halt.unwrap().cap_name(), "max_cost_usd");
    }

    #[test]
    fn budget_halted_is_distinct_from_failed_and_cancelled() {
        assert_ne!(RunStatus::BudgetHalted, RunStatus::Failed);
        assert_ne!(RunStatus::BudgetHalted, RunStatus::Cancelled);
    }

    #[test]
    fn neither_key_present_derives_exactly_as_before() {
        // The in-progress / completed fixtures carry no `metadata` key at
        // all (pre-v1.1.0 shape) — confirm the empty-object default used by
        // `parse_event_row` reproduces the unchanged node-aggregate result.
        let tc: serde_json::Value = serde_json::from_str(IN_PROGRESS_FIXTURE).unwrap();
        let nodes = parse_task_context(&tc).unwrap();
        let metadata = tc
            .get("metadata")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let (status, budget_halt) = derive_run_status(&nodes, &metadata);
        assert_eq!(status, RunStatus::Running);
        assert!(budget_halt.is_none());

        let tc: serde_json::Value = serde_json::from_str(COMPLETED_FIXTURE).unwrap();
        let nodes = parse_task_context(&tc).unwrap();
        let metadata = tc
            .get("metadata")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let (status, budget_halt) = derive_run_status(&nodes, &metadata);
        assert_eq!(status, RunStatus::Failed);
        assert!(budget_halt.is_none());
    }

    #[test]
    fn genuinely_failed_run_is_unaffected_by_absent_metadata() {
        let tc: serde_json::Value = serde_json::from_str(COMPLETED_FIXTURE).unwrap();
        let nodes = parse_task_context(&tc).unwrap();
        let (status, budget_halt) = derive_run_status(&nodes, &serde_json::json!({}));
        assert_eq!(status, RunStatus::Failed);
        assert!(budget_halt.is_none());
    }

    #[test]
    fn malformed_metadata_fixture_falls_back_to_node_derivation() {
        // `budget.halted == true` but no `reason` object, and a
        // `cancellation` value that isn't an object at all — both must be
        // treated as absent rather than erroring or panicking.
        let tc: serde_json::Value = serde_json::from_str(MALFORMED_METADATA_FIXTURE).unwrap();
        let nodes = parse_task_context(&tc).unwrap();
        let metadata = tc.get("metadata").cloned().unwrap();
        let (status, budget_halt) = derive_run_status(&nodes, &metadata);
        // DataIngestionNode success, LLMSummaryNode running -> Running by the
        // ordinary node-aggregate rules, exactly as if metadata were absent.
        assert_eq!(status, RunStatus::Running);
        assert!(budget_halt.is_none());
    }

    #[test]
    fn budget_halt_with_unknown_cap_is_ignored() {
        let metadata = serde_json::json!({
            "budget": {
                "halted": true,
                "reason": { "cap": "max_llm_calls", "spent": 5, "limit": 5 }
            }
        });
        assert!(metadata_budget_halt(&metadata).is_none());
    }

    #[test]
    fn budget_halt_false_is_not_halted() {
        let metadata = serde_json::json!({
            "budget": { "halted": false }
        });
        assert!(metadata_budget_halt(&metadata).is_none());
    }

    #[test]
    fn cancellation_false_is_not_cancelled() {
        let metadata = serde_json::json!({
            "cancellation": { "cancelled": false, "at": "2026-07-16T00:00:00Z" }
        });
        assert!(!metadata_is_cancelled(&metadata));
    }

    #[test]
    fn empty_metadata_object_is_absent_tolerant() {
        let metadata = serde_json::json!({});
        assert!(!metadata_is_cancelled(&metadata));
        assert!(metadata_budget_halt(&metadata).is_none());
    }

    #[test]
    fn run_status_cancelled_and_budget_halted_do_not_deserialize_from_wire() {
        // These are run-level-only, derived states — the wire never spells a
        // node status as "cancelled" or "budget_halted" (contract §6 keeps
        // `NodeRunStatus` at exactly pending|running|success|failed).
        assert!(serde_json::from_str::<RunStatus>("\"cancelled\"").is_err());
        assert!(serde_json::from_str::<RunStatus>("\"budget_halted\"").is_err());
    }

    // Helper: build a minimal NodeState for derive_run_status tests.
    fn make_node(name: &str, status: RunStatus) -> NodeState {
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

    // ── integration stubs (require a live DB; skipped in CI) ─────────────────
    //
    // Run with:
    //   BASTION_INTEGRATION_TEST=1 cargo test -- --ignored
    //
    // These stubs document the expected call shape for `list_active_runs` and
    // `get_run_state`. They are gated behind `#[ignore]` and a runtime env-var
    // check so they never execute in unit-test CI.

    #[tokio::test]
    #[ignore]
    async fn integration_list_active_runs_returns_vec() {
        if std::env::var("BASTION_INTEGRATION_TEST").is_err() {
            return;
        }
        let db_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
        let runs = list_active_runs(&db_url)
            .await
            .expect("list_active_runs should not error against a live DB");
        // Just confirm the return type is correct; count may be 0 in a clean DB.
        let _: Vec<WorkflowRun> = runs;
    }

    #[tokio::test]
    #[ignore]
    async fn integration_get_run_state_errors_on_missing_id() {
        if std::env::var("BASTION_INTEGRATION_TEST").is_err() {
            return;
        }
        let db_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
        // A UUID that will never exist in the events table.
        let result = get_run_state(&db_url, "00000000-0000-0000-0000-000000000000").await;
        assert!(
            result.is_err(),
            "get_run_state should return Err for an unknown id"
        );
    }
}
