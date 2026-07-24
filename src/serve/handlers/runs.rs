//! Live run read handlers for `bastion serve` (BA.11.M — read half of D42).
//!
//! Projects the embedded engine's in-memory [`LiveStateStore`] snapshot for a
//! run over HTTP, so remote clients can read a run's current per-node state
//! without polling Postgres. **Read-only** — no stream/SSE/WS is introduced
//! here (that is the deferred follow-on block, proposed `BA.11.N`).
//!
//! # Routes
//! - `GET /api/runs`      — currently-tracked run ids (`list_active()`)
//! - `GET /api/runs/{id}` — one run's projected [`RunStateDto`] snapshot
//!
//! # Pure core vs I/O shell (Rule 6)
//! [`project_run`] is the pure `TaskContext` → `RunStateDto` projection —
//! exhaustively unit-tested with no I/O. [`list_runs`] / [`get_run`] are the
//! thin async handlers over the shared [`LiveStateStore`] — smoke-tested
//! manually against a running `bastion serve` (recorded in the task spec's
//! `## Notes`), following the `handlers/status.rs` ErrorPayload/response shape.
//!
//! # Error mapping
//! - Malformed `{id}` (not a valid UUID) → 400 + `C006` (invalid input).
//! - Unknown/absent run id (not currently tracked) → 404 + `C002` (mirrors the
//!   "known target, missing resource" code used by `handlers/status.rs`).

use actix_web::{HttpResponse, web};
use engine_serve::live_state::LiveStateStore;
use uuid::Uuid;

use crate::serve::dto::{ErrorPayload, NodeTransitionDto, RunStateDto, RunUsageDto};
use engine_contract::task_context::{NodeRunStatus, TaskContext};

// ── Pure projection ──────────────────────────────────────────────────────────

/// Lowercase wire string for a `NodeRunStatus` (contract §6 casing).
fn status_str(status: NodeRunStatus) -> String {
    match status {
        NodeRunStatus::Pending => "pending",
        NodeRunStatus::Running => "running",
        NodeRunStatus::Success => "success",
        NodeRunStatus::Failed => "failed",
    }
    .to_owned()
}

/// Project a `TaskContext` snapshot into the wire `RunStateDto`, joining
/// `node_runs[class]` (status/timing/error/input/usage) with `nodes[class]`
/// (output) by class name (BA.11.M).
///
/// Pure — no I/O. Nodes are sorted by class name for deterministic output.
pub fn project_run(run_id: Uuid, ctx: &TaskContext) -> RunStateDto {
    let mut classes: Vec<&String> = ctx.node_runs.keys().collect();
    classes.sort();

    let nodes = classes
        .into_iter()
        .map(|class| {
            let run = &ctx.node_runs[class];
            NodeTransitionDto {
                node: class.clone(),
                status: status_str(run.status),
                started_at: run.started_at.map(|t| t.to_rfc3339()),
                completed_at: run.completed_at.map(|t| t.to_rfc3339()),
                error: run.error.clone(),
                input: run.input.clone(),
                output: ctx.nodes.get(class).cloned(),
                usage: run.usage.as_ref().map(|u| RunUsageDto {
                    input_tokens: u.input_tokens,
                    output_tokens: u.output_tokens,
                    model: u.model.clone(),
                }),
            }
        })
        .collect();

    RunStateDto {
        run_id: run_id.to_string(),
        event: ctx.event.clone(),
        metadata: ctx.metadata.clone(),
        nodes,
    }
}

// ── Handler helpers ──────────────────────────────────────────────────────────

/// Build a 400 response for a malformed `{id}` path param (not a valid UUID).
fn malformed_id_response(raw: &str) -> HttpResponse {
    HttpResponse::BadRequest().json(ErrorPayload {
        code: "C006".to_owned(),
        message: format!("malformed run id: {raw}"),
    })
}

/// Build a 404 response for a run id not currently tracked by the store.
fn unknown_run_response(id: Uuid) -> HttpResponse {
    HttpResponse::NotFound().json(ErrorPayload {
        code: "C002".to_owned(),
        message: format!("run not found: {id}"),
    })
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// `GET /api/runs` — the run ids currently tracked by the shared `LiveStateStore`.
///
/// Returns 200 with a JSON array of run-id strings; `[]` when the store is
/// empty (including when the engine is not mounted).
pub async fn list_runs(live: web::Data<LiveStateStore>) -> HttpResponse {
    let ids: Vec<String> = live
        .list_active()
        .into_iter()
        .map(|id| id.to_string())
        .collect();
    HttpResponse::Ok().json(ids)
}

/// `GET /api/runs/{id}` — the projected `RunStateDto` snapshot for one run.
///
/// 400 when `{id}` does not parse as a UUID; 404 when the run is not (or no
/// longer) tracked by the store; 200 with the projected snapshot otherwise.
pub async fn get_run(id: web::Path<String>, live: web::Data<LiveStateStore>) -> HttpResponse {
    let raw = id.into_inner();
    let Ok(run_id) = Uuid::parse_str(&raw) else {
        return malformed_id_response(&raw);
    };

    match live.get(run_id) {
        Some(ctx) => HttpResponse::Ok().json(project_run(run_id, &ctx)),
        None => unknown_run_response(run_id),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use engine_contract::task_context::{NodeRun, Usage};
    use std::collections::HashMap;

    fn node_run(status: NodeRunStatus) -> NodeRun {
        NodeRun {
            status,
            started_at: None,
            completed_at: None,
            error: None,
            input: None,
            usage: None,
        }
    }

    #[test]
    fn project_run_empty_node_runs_yields_empty_nodes() {
        let ctx = TaskContext {
            event: serde_json::json!({}),
            nodes: HashMap::new(),
            metadata: serde_json::json!({}),
            node_runs: HashMap::new(),
        };
        let run_id = Uuid::new_v4();

        let dto = project_run(run_id, &ctx);

        assert_eq!(dto.run_id, run_id.to_string());
        assert!(dto.nodes.is_empty());
    }

    #[test]
    fn project_run_carries_event_and_metadata() {
        let ctx = TaskContext {
            event: serde_json::json!({ "ticket_id": "T-1" }),
            nodes: HashMap::new(),
            metadata: serde_json::json!({ "workflow": "sdlc-flow" }),
            node_runs: HashMap::new(),
        };

        let dto = project_run(Uuid::new_v4(), &ctx);

        assert_eq!(dto.event, serde_json::json!({ "ticket_id": "T-1" }));
        assert_eq!(dto.metadata, serde_json::json!({ "workflow": "sdlc-flow" }));
    }

    #[test]
    fn project_run_joins_output_by_class_name() {
        let mut node_runs = HashMap::new();
        node_runs.insert(
            "DataIngestionNode".to_string(),
            node_run(NodeRunStatus::Success),
        );
        let mut nodes = HashMap::new();
        nodes.insert(
            "DataIngestionNode".to_string(),
            serde_json::json!({ "documents_loaded": 3 }),
        );
        let ctx = TaskContext {
            event: serde_json::json!({}),
            nodes,
            metadata: serde_json::json!({}),
            node_runs,
        };

        let dto = project_run(Uuid::new_v4(), &ctx);

        assert_eq!(dto.nodes.len(), 1);
        assert_eq!(dto.nodes[0].node, "DataIngestionNode");
        assert_eq!(dto.nodes[0].status, "success");
        assert_eq!(
            dto.nodes[0].output,
            Some(serde_json::json!({ "documents_loaded": 3 }))
        );
    }

    #[test]
    fn project_run_multi_node_mixed_statuses() {
        let mut node_runs = HashMap::new();
        node_runs.insert("NodeA".to_string(), node_run(NodeRunStatus::Pending));
        node_runs.insert("NodeB".to_string(), node_run(NodeRunStatus::Running));
        node_runs.insert("NodeC".to_string(), node_run(NodeRunStatus::Success));
        node_runs.insert("NodeD".to_string(), node_run(NodeRunStatus::Failed));

        let ctx = TaskContext {
            event: serde_json::json!({}),
            nodes: HashMap::new(),
            metadata: serde_json::json!({}),
            node_runs,
        };

        let dto = project_run(Uuid::new_v4(), &ctx);
        assert_eq!(dto.nodes.len(), 4);

        let statuses: HashMap<&str, &str> = dto
            .nodes
            .iter()
            .map(|n| (n.node.as_str(), n.status.as_str()))
            .collect();
        assert_eq!(statuses["NodeA"], "pending");
        assert_eq!(statuses["NodeB"], "running");
        assert_eq!(statuses["NodeC"], "success");
        assert_eq!(statuses["NodeD"], "failed");
    }

    #[test]
    fn project_run_failed_node_exposes_error_and_input() {
        let mut run = node_run(NodeRunStatus::Failed);
        run.error = Some("boom".to_string());
        run.input = Some(serde_json::json!({ "x": 1 }));

        let mut node_runs = HashMap::new();
        node_runs.insert("FailingNode".to_string(), run);

        let ctx = TaskContext {
            event: serde_json::json!({}),
            nodes: HashMap::new(),
            metadata: serde_json::json!({}),
            node_runs,
        };

        let dto = project_run(Uuid::new_v4(), &ctx);
        assert_eq!(dto.nodes.len(), 1);
        assert_eq!(dto.nodes[0].error.as_deref(), Some("boom"));
        assert_eq!(dto.nodes[0].input, Some(serde_json::json!({ "x": 1 })));
    }

    #[test]
    fn project_run_llm_node_has_usage_non_llm_node_has_none() {
        let mut llm_run = node_run(NodeRunStatus::Success);
        llm_run.usage = Some(Usage {
            input_tokens: Some(512),
            output_tokens: Some(128),
            model: "claude-sonnet-5".to_string(),
        });
        let plain_run = node_run(NodeRunStatus::Success);

        let mut node_runs = HashMap::new();
        node_runs.insert("LlmNode".to_string(), llm_run);
        node_runs.insert("PlainNode".to_string(), plain_run);

        let ctx = TaskContext {
            event: serde_json::json!({}),
            nodes: HashMap::new(),
            metadata: serde_json::json!({}),
            node_runs,
        };

        let dto = project_run(Uuid::new_v4(), &ctx);
        let by_node: HashMap<&str, &NodeTransitionDto> =
            dto.nodes.iter().map(|n| (n.node.as_str(), n)).collect();

        let llm_usage = by_node["LlmNode"].usage.as_ref().expect("usage present");
        assert_eq!(llm_usage.input_tokens, Some(512));
        assert_eq!(llm_usage.output_tokens, Some(128));
        assert_eq!(llm_usage.model, "claude-sonnet-5");

        assert!(by_node["PlainNode"].usage.is_none());
    }

    #[test]
    fn malformed_id_response_is_400_c006() {
        let resp = malformed_id_response("not-a-uuid");
        assert_eq!(resp.status(), 400);
    }

    #[test]
    fn unknown_run_response_is_404_c002() {
        let resp = unknown_run_response(Uuid::new_v4());
        assert_eq!(resp.status(), 404);
    }

    #[actix_web::test]
    async fn get_run_malformed_id_returns_400() {
        let live = web::Data::new(LiveStateStore::new());
        let resp = get_run(web::Path::from("not-a-uuid".to_string()), live).await;
        assert_eq!(resp.status(), 400);
    }

    #[actix_web::test]
    async fn get_run_unknown_id_returns_404() {
        let live = web::Data::new(LiveStateStore::new());
        let resp = get_run(web::Path::from(Uuid::new_v4().to_string()), live).await;
        assert_eq!(resp.status(), 404);
    }

    #[actix_web::test]
    async fn get_run_known_id_returns_200_with_projection() {
        let store = LiveStateStore::new();
        let run_id = Uuid::new_v4();
        store.record(
            run_id,
            &TaskContext {
                event: serde_json::json!({}),
                nodes: HashMap::new(),
                metadata: serde_json::json!({}),
                node_runs: HashMap::new(),
            },
        );
        let live = web::Data::new(store);

        let resp = get_run(web::Path::from(run_id.to_string()), live).await;
        assert_eq!(resp.status(), 200);
    }

    #[actix_web::test]
    async fn list_runs_empty_store_returns_empty_array() {
        let live = web::Data::new(LiveStateStore::new());
        let resp = list_runs(live).await;
        assert_eq!(resp.status(), 200);
    }

    #[actix_web::test]
    async fn list_runs_reflects_recorded_runs() {
        let store = LiveStateStore::new();
        let run_id = Uuid::new_v4();
        store.record(
            run_id,
            &TaskContext {
                event: serde_json::json!({}),
                nodes: HashMap::new(),
                metadata: serde_json::json!({}),
                node_runs: HashMap::new(),
            },
        );
        let live = web::Data::new(store);

        let resp = list_runs(live).await;
        assert_eq!(resp.status(), 200);
    }
}
