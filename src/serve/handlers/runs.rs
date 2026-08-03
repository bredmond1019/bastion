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
use serde::Deserialize;
use uuid::Uuid;

use crate::config::FileConfig;
use crate::db::workflows::{self, NodeState, RunStatus};
use crate::serve::dto::{
    ErrorPayload, NodeTransitionDto, RepoWorkflowStateDto, RunStateDto, RunSummaryDto, RunUsageDto,
};
use crate::serve::handlers::status;
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

// ── RunSummaryDto projection (BA.11.T) ───────────────────────────────────────

/// Map an `engine_contract` `NodeRunStatus` onto the shared `db::workflows`
/// `RunStatus` for the four variants both enums carry. `RunStatus` also has
/// `Cancelled`/`BudgetHalted`, which are run-level-only and never produced
/// from a single node's wire status — so this mapping is total and exhaustive
/// over `NodeRunStatus`.
fn node_run_status_to_run_status(status: NodeRunStatus) -> RunStatus {
    match status {
        NodeRunStatus::Pending => RunStatus::Pending,
        NodeRunStatus::Running => RunStatus::Running,
        NodeRunStatus::Success => RunStatus::Success,
        NodeRunStatus::Failed => RunStatus::Failed,
    }
}

/// Map `ctx.node_runs` into the minimal `NodeState` values `derive_run_status`
/// needs. Only `status` is load-bearing for the aggregate logic; the rest are
/// cheap per-class defaults (`depends_on: vec![]`, everything else `None`)
/// except `started_at`, carried through as RFC3339 for parity with the wire
/// shape even though `derive_run_status` itself does not read it.
///
/// Pure — no I/O.
fn node_states_from(ctx: &TaskContext) -> Vec<NodeState> {
    ctx.node_runs
        .iter()
        .map(|(class, run)| NodeState {
            id: class.clone(),
            name: class.clone(),
            status: node_run_status_to_run_status(run.status),
            depends_on: vec![],
            input: None,
            output: None,
            error: None,
            tokens_in: None,
            tokens_out: None,
            model: None,
            started_at: run.started_at.map(|t| t.to_rfc3339()),
            elapsed_secs: None,
        })
        .collect()
}

/// Lowercase wire string for a `db::workflows::RunStatus` (contract §6
/// casing), mirroring [`status_str`]'s style for the run-level enum, which
/// additionally carries the three run-level-only variants.
///
/// `pub(crate)` (widened from private for BA.11.N) so `src/serve/poll.rs`'s
/// `RunWatcher` can reuse the exact same status-string derivation `GET
/// /api/runs` uses, rather than reimplementing it — the two must never
/// disagree.
pub(crate) fn run_status_str(status: RunStatus) -> String {
    match status {
        RunStatus::Pending => "pending",
        RunStatus::Running => "running",
        RunStatus::Success => "success",
        RunStatus::Failed => "failed",
        RunStatus::Cancelled => "cancelled",
        RunStatus::BudgetHalted => "budget_halted",
        RunStatus::Suspended => "suspended",
    }
    .to_owned()
}

/// Read `event.spec_slug` when present as a string. `None` when the key is
/// absent, non-string, or `event` is not an object.
///
/// Pure — no I/O.
fn spec_slug_from_event(event: &serde_json::Value) -> Option<String> {
    event
        .get("spec_slug")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

/// Derive `(started_at, updated_at)` from `ctx.node_runs[*]` timestamps.
///
/// `started_at` is the earliest non-null `started_at` across all tracked
/// nodes; `updated_at` is the latest non-null `started_at` **or**
/// `completed_at` across all tracked nodes. Both are `None` when
/// `node_runs` is empty or carries no timestamps at all (the window between
/// `post_events` registering the run and its first `on_progress` callback).
///
/// Pure — no I/O. Comparison is lexicographic over RFC3339 strings, which is
/// timestamp-order-preserving for a fixed offset representation (`to_rfc3339`
/// always renders UTC `DateTime<Utc>` with a `+00:00`/`Z`-equivalent offset).
fn run_timestamps(ctx: &TaskContext) -> (Option<String>, Option<String>) {
    let mut started_at: Option<String> = None;
    let mut updated_at: Option<String> = None;

    for run in ctx.node_runs.values() {
        if let Some(s) = run.started_at.map(|t| t.to_rfc3339()) {
            if started_at.as_deref().is_none_or(|cur| s.as_str() < cur) {
                started_at = Some(s.clone());
            }
            if updated_at.as_deref().is_none_or(|cur| s.as_str() > cur) {
                updated_at = Some(s);
            }
        }
        if let Some(c) = run.completed_at.map(|t| t.to_rfc3339())
            && updated_at.as_deref().is_none_or(|cur| c.as_str() > cur)
        {
            updated_at = Some(c);
        }
    }

    (started_at, updated_at)
}

/// Project a `TaskContext` snapshot into the wire `RunSummaryDto` (BA.11.T).
///
/// Reuses `db::workflows::derive_run_status` for `status` rather than
/// reimplementing the aggregate/cancellation/budget-halt priority order.
/// `workflow_type` is always `None` today — see `RunSummaryDto`'s doc comment
/// and the task spec's Notes for why.
///
/// `repo` is passed in already-resolved (via [`resolve_repo_for_run`]) rather
/// than resolved here, so this projection stays pure — the registry walk that
/// resolution requires is the caller's ([`list_runs`]'s) I/O concern, gated
/// behind `?with_repo=1` (A7).
///
/// Pure — no I/O.
pub fn project_run_summary(run_id: Uuid, ctx: &TaskContext, repo: Option<String>) -> RunSummaryDto {
    let nodes = node_states_from(ctx);
    let (status, _budget_halt) = workflows::derive_run_status(&nodes, &ctx.metadata);
    let (started_at, updated_at) = run_timestamps(ctx);

    RunSummaryDto {
        run_id: run_id.to_string(),
        workflow_type: None,
        status: run_status_str(status),
        spec_slug: spec_slug_from_event(&ctx.event),
        started_at,
        updated_at,
        repo,
    }
}

// ── run_id -> repo join (A7) ─────────────────────────────────────────────────

/// Deserialize a query-string flag as a bool, accepting `1`/`0` in addition to
/// `true`/`false` — `serde_urlencoded` (the wire format `web::Query` uses)
/// only recognizes the literal tokens `true`/`false` for a plain `bool`
/// field, which would reject the exact param this route documents and A7
/// pins (`?with_repo=1`, AC 5a — deliberately not `?with_repo=true`, to
/// stay consistent with other boolean query flags in this API).
fn bool_flag_from_str<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    match raw.as_str() {
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        other => Err(serde::de::Error::custom(format!(
            "invalid boolean flag value: {other:?} (expected 1/0 or true/false)"
        ))),
    }
}

/// `GET /api/runs` query params.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
pub struct RunsQuery {
    /// `?with_repo=1` opts into the `run_id -> repo` join ([`resolve_repo_for_run`])
    /// against every registered workspace's flow state (`collect_all_workflows`,
    /// A2). Defaults to `false` (mirrors `/api/board`'s `?graph=1`, A5): Task 1's
    /// measurement found the registry walk ~6x the unenriched baseline at 0
    /// active runs against the live HQ registry (23 repos), so an unopted poll
    /// (this route's hottest consumer, bastion-web's ~2-6s run rail) must not pay
    /// for it — see `planning/ticket-run-summary-repo-join/tasks.md`'s Notes.
    #[serde(default, deserialize_with = "bool_flag_from_str")]
    pub with_repo: bool,
}

/// Resolve the repo that owns `run_id` by an **exact match** on
/// `RepoWorkflowStateDto::run_id` — never substring, prefix, or spec-slug
/// similarity (A7: a wrong label is strictly worse than an absent one).
///
/// A flow state with `run_id: None` never matches any run, including a run
/// whose own id is (hypothetically) absent — absence must not match absence,
/// so this function only ever compares against a concrete `run_id` string.
///
/// **Ambiguity guard:** if more than one flow state reports the same
/// `run_id` (a misconfigured registry — two repos racing the same run id —
/// should not happen in practice, but the registry is untrusted input), the
/// **first** match in `workflows`' iteration order wins. `workflows` is
/// expected to be `collect_all_workflows`'s output, which is already ordered
/// `(repo name, then spec_slug)`; taking the first match therefore means the
/// alphabetically-first repo wins, deterministically, on every call — never
/// left to `HashMap` iteration order, which would make the response flap
/// between requests.
///
/// Pure — no I/O.
fn resolve_repo_for_run(run_id: &str, workflows: &[RepoWorkflowStateDto]) -> Option<String> {
    workflows
        .iter()
        .find(|w| w.run_id.as_deref() == Some(run_id))
        .map(|w| w.repo.clone())
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

/// `GET /api/runs[?with_repo=1]` — the projected `RunSummaryDto` for every
/// run currently tracked by the shared `LiveStateStore` (BA.11.T), optionally
/// enriched with `repo` via an exact `run_id` join against every registered
/// workspace's flow state (A7).
///
/// Returns 200 with a JSON array; `[]` when the store is empty (including
/// when the engine is not mounted). Any run id that races out of the store
/// between `list_active()` and `get()` (evicted by `mark_terminal`) is
/// silently dropped from the response rather than erroring.
///
/// **The registry walk (`collect_all_workflows`) only runs when `with_repo`
/// is set.** Without it, `workflows` stays an empty `Vec` and
/// [`resolve_repo_for_run`] trivially returns `None` for every run — the
/// unopted poll path (this route's hottest consumer, bastion-web's ~2-6s run
/// rail) pays nothing for the join it never asked for, per Task 1's
/// measurement and A7's Notes.
pub async fn list_runs(
    query: web::Query<RunsQuery>,
    live: web::Data<LiveStateStore>,
    registry: web::Data<FileConfig>,
) -> HttpResponse {
    let with_repo = query.with_repo;

    let active: Vec<(Uuid, TaskContext)> = live
        .list_active()
        .into_iter()
        .filter_map(|id| live.get(id).map(|ctx| (id, ctx)))
        .collect();

    // Thread-pool failure degrades to "no matches" rather than a 500 — `repo`
    // absence is always a valid outcome (A7), unlike the `/api/repos*`
    // routes' hard error mapping.
    let workflows: Vec<RepoWorkflowStateDto> = if with_repo {
        web::block(move || status::collect_all_workflows(&registry))
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let summaries: Vec<RunSummaryDto> = active
        .into_iter()
        .map(|(id, ctx)| {
            let repo = resolve_repo_for_run(&id.to_string(), &workflows);
            project_run_summary(id, &ctx, repo)
        })
        .collect();
    HttpResponse::Ok().json(summaries)
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
    async fn list_runs_empty_store_returns_200_empty_array() {
        let live = web::Data::new(LiveStateStore::new());
        let resp = list_runs(
            web::Query(RunsQuery::default()),
            live,
            web::Data::new(FileConfig::default()),
        )
        .await;
        assert_eq!(resp.status(), 200);

        let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json, serde_json::json!([]));
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

        let resp = list_runs(
            web::Query(RunsQuery::default()),
            live,
            web::Data::new(FileConfig::default()),
        )
        .await;
        assert_eq!(resp.status(), 200);
    }

    #[actix_web::test]
    async fn list_runs_spec_slug_bearing_run_returns_summary_with_real_slug_and_status() {
        let store = LiveStateStore::new();
        let run_id = Uuid::new_v4();
        let mut node_runs = HashMap::new();
        node_runs.insert(
            "PlanNode".to_string(),
            node_run_with_times(
                NodeRunStatus::Success,
                Some("2026-01-01T00:00:00Z"),
                Some("2026-01-01T00:05:00Z"),
            ),
        );
        store.record(
            run_id,
            &TaskContext {
                event: serde_json::json!({ "spec_slug": "11.T-run-summary-projection" }),
                nodes: HashMap::new(),
                metadata: serde_json::json!({}),
                node_runs,
            },
        );
        let live = web::Data::new(store);

        let resp = list_runs(
            web::Query(RunsQuery::default()),
            live,
            web::Data::new(FileConfig::default()),
        )
        .await;
        assert_eq!(resp.status(), 200);

        let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = json.as_array().expect("array body");
        assert_eq!(arr.len(), 1);
        assert_eq!(
            arr[0].get("spec_slug").and_then(serde_json::Value::as_str),
            Some("11.T-run-summary-projection")
        );
        assert_eq!(
            arr[0].get("status").and_then(serde_json::Value::as_str),
            Some("success")
        );
        assert_eq!(
            arr[0].get("run_id").and_then(serde_json::Value::as_str),
            Some(run_id.to_string().as_str())
        );
    }

    #[actix_web::test]
    async fn list_runs_run_without_spec_slug_omits_field_from_raw_json() {
        let store = LiveStateStore::new();
        let run_id = Uuid::new_v4();
        store.record(
            run_id,
            &TaskContext {
                event: serde_json::json!({ "ticket_id": "T-1" }),
                nodes: HashMap::new(),
                metadata: serde_json::json!({}),
                node_runs: HashMap::new(),
            },
        );
        let live = web::Data::new(store);

        let resp = list_runs(
            web::Query(RunsQuery::default()),
            live,
            web::Data::new(FileConfig::default()),
        )
        .await;
        assert_eq!(resp.status(), 200);

        let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = json.as_array().expect("array body");
        assert_eq!(arr.len(), 1);
        let entry = arr[0].as_object().expect("object entry");
        assert!(
            !entry.contains_key("spec_slug"),
            "spec_slug must be an absent key, not present as null: {entry:?}"
        );
        assert!(
            !entry.contains_key("workflow_type"),
            "workflow_type must be an absent key today: {entry:?}"
        );
    }

    #[actix_web::test]
    async fn list_runs_excludes_terminal_run() {
        let store = LiveStateStore::new();
        let run_id = Uuid::new_v4();
        let ctx = TaskContext {
            event: serde_json::json!({}),
            nodes: HashMap::new(),
            metadata: serde_json::json!({}),
            node_runs: HashMap::new(),
        };
        store.record(run_id, &ctx);
        let now = chrono::Utc::now();
        store.mark_terminal(run_id, &ctx, "SDLC_FLOW", now, now);
        let live = web::Data::new(store);

        let resp = list_runs(
            web::Query(RunsQuery::default()),
            live,
            web::Data::new(FileConfig::default()),
        )
        .await;
        assert_eq!(resp.status(), 200);

        let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json, serde_json::json!([]));
    }

    // ── RunSummaryDto projection (BA.11.T) ────────────────────────────────────

    fn ts(rfc3339: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(rfc3339)
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    fn node_run_with_times(
        status: NodeRunStatus,
        started_at: Option<&str>,
        completed_at: Option<&str>,
    ) -> NodeRun {
        NodeRun {
            status,
            started_at: started_at.map(ts),
            completed_at: completed_at.map(ts),
            error: None,
            input: None,
            usage: None,
        }
    }

    // -- node_states_from --

    #[test]
    fn node_states_from_empty_node_runs_yields_empty_vec() {
        let ctx = TaskContext {
            event: serde_json::json!({}),
            nodes: HashMap::new(),
            metadata: serde_json::json!({}),
            node_runs: HashMap::new(),
        };
        assert!(node_states_from(&ctx).is_empty());
    }

    #[test]
    fn node_states_from_single_node_maps_status_and_started_at() {
        let mut node_runs = HashMap::new();
        node_runs.insert(
            "NodeA".to_string(),
            node_run_with_times(NodeRunStatus::Success, Some("2026-01-01T00:00:00Z"), None),
        );
        let ctx = TaskContext {
            event: serde_json::json!({}),
            nodes: HashMap::new(),
            metadata: serde_json::json!({}),
            node_runs,
        };

        let states = node_states_from(&ctx);
        assert_eq!(states.len(), 1);
        let node = &states[0];
        assert_eq!(node.id, "NodeA");
        assert_eq!(node.name, "NodeA");
        assert_eq!(node.status, RunStatus::Success);
        assert!(node.depends_on.is_empty());
        assert!(node.input.is_none());
        assert!(node.output.is_none());
        assert!(node.error.is_none());
        assert!(node.tokens_in.is_none());
        assert!(node.tokens_out.is_none());
        assert!(node.model.is_none());
        assert_eq!(
            node.started_at.as_deref(),
            Some("2026-01-01T00:00:00+00:00")
        );
        assert!(node.elapsed_secs.is_none());
    }

    #[test]
    fn node_states_from_many_nodes_mixed_statuses() {
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

        let states = node_states_from(&ctx);
        assert_eq!(states.len(), 4);
        let by_id: HashMap<&str, &NodeState> = states.iter().map(|n| (n.id.as_str(), n)).collect();
        assert_eq!(by_id["NodeA"].status, RunStatus::Pending);
        assert_eq!(by_id["NodeB"].status, RunStatus::Running);
        assert_eq!(by_id["NodeC"].status, RunStatus::Success);
        assert_eq!(by_id["NodeD"].status, RunStatus::Failed);
    }

    // -- run_status_str --

    #[test]
    fn run_status_str_covers_all_seven_variants() {
        assert_eq!(run_status_str(RunStatus::Pending), "pending");
        assert_eq!(run_status_str(RunStatus::Running), "running");
        assert_eq!(run_status_str(RunStatus::Success), "success");
        assert_eq!(run_status_str(RunStatus::Failed), "failed");
        assert_eq!(run_status_str(RunStatus::Cancelled), "cancelled");
        assert_eq!(run_status_str(RunStatus::BudgetHalted), "budget_halted");
        assert_eq!(run_status_str(RunStatus::Suspended), "suspended");
    }

    // -- spec_slug_from_event --

    #[test]
    fn spec_slug_from_event_present_string() {
        let event = serde_json::json!({ "spec_slug": "11.T-run-summary-projection" });
        assert_eq!(
            spec_slug_from_event(&event),
            Some("11.T-run-summary-projection".to_string())
        );
    }

    #[test]
    fn spec_slug_from_event_absent_key() {
        let event = serde_json::json!({ "ticket_id": "T-1" });
        assert_eq!(spec_slug_from_event(&event), None);
    }

    #[test]
    fn spec_slug_from_event_non_string_value() {
        let event = serde_json::json!({ "spec_slug": 42 });
        assert_eq!(spec_slug_from_event(&event), None);
    }

    // -- run_timestamps --

    #[test]
    fn run_timestamps_empty_node_runs_yields_none_none() {
        let ctx = TaskContext {
            event: serde_json::json!({}),
            nodes: HashMap::new(),
            metadata: serde_json::json!({}),
            node_runs: HashMap::new(),
        };
        assert_eq!(run_timestamps(&ctx), (None, None));
    }

    #[test]
    fn run_timestamps_single_node_started_only() {
        let mut node_runs = HashMap::new();
        node_runs.insert(
            "NodeA".to_string(),
            node_run_with_times(NodeRunStatus::Running, Some("2026-01-01T00:00:00Z"), None),
        );
        let ctx = TaskContext {
            event: serde_json::json!({}),
            nodes: HashMap::new(),
            metadata: serde_json::json!({}),
            node_runs,
        };

        let (started, updated) = run_timestamps(&ctx);
        assert_eq!(started.as_deref(), Some("2026-01-01T00:00:00+00:00"));
        assert_eq!(updated.as_deref(), Some("2026-01-01T00:00:00+00:00"));
    }

    #[test]
    fn run_timestamps_single_node_started_and_completed() {
        let mut node_runs = HashMap::new();
        node_runs.insert(
            "NodeA".to_string(),
            node_run_with_times(
                NodeRunStatus::Success,
                Some("2026-01-01T00:00:00Z"),
                Some("2026-01-01T00:05:00Z"),
            ),
        );
        let ctx = TaskContext {
            event: serde_json::json!({}),
            nodes: HashMap::new(),
            metadata: serde_json::json!({}),
            node_runs,
        };

        let (started, updated) = run_timestamps(&ctx);
        assert_eq!(started.as_deref(), Some("2026-01-01T00:00:00+00:00"));
        assert_eq!(updated.as_deref(), Some("2026-01-01T00:05:00+00:00"));
    }

    #[test]
    fn run_timestamps_multi_node_min_max_across_started_and_completed() {
        let mut node_runs = HashMap::new();
        node_runs.insert(
            "NodeA".to_string(),
            node_run_with_times(
                NodeRunStatus::Success,
                Some("2026-01-01T00:02:00Z"),
                Some("2026-01-01T00:04:00Z"),
            ),
        );
        node_runs.insert(
            "NodeB".to_string(),
            node_run_with_times(NodeRunStatus::Running, Some("2026-01-01T00:00:00Z"), None),
        );
        node_runs.insert(
            "NodeC".to_string(),
            node_run_with_times(
                NodeRunStatus::Success,
                Some("2026-01-01T00:01:00Z"),
                Some("2026-01-01T00:10:00Z"),
            ),
        );
        let ctx = TaskContext {
            event: serde_json::json!({}),
            nodes: HashMap::new(),
            metadata: serde_json::json!({}),
            node_runs,
        };

        let (started, updated) = run_timestamps(&ctx);
        // Earliest started_at across all nodes is NodeB's 00:00:00.
        assert_eq!(started.as_deref(), Some("2026-01-01T00:00:00+00:00"));
        // Latest started_at-or-completed_at across all nodes is NodeC's completed_at 00:10:00.
        assert_eq!(updated.as_deref(), Some("2026-01-01T00:10:00+00:00"));
    }

    #[test]
    fn run_timestamps_node_with_no_timestamps_ignored() {
        let mut node_runs = HashMap::new();
        node_runs.insert("NodeA".to_string(), node_run(NodeRunStatus::Pending));
        let ctx = TaskContext {
            event: serde_json::json!({}),
            nodes: HashMap::new(),
            metadata: serde_json::json!({}),
            node_runs,
        };
        assert_eq!(run_timestamps(&ctx), (None, None));
    }

    // -- project_run_summary --

    #[test]
    fn project_run_summary_sdlc_flow_event_with_spec_slug() {
        let mut node_runs = HashMap::new();
        node_runs.insert(
            "PlanNode".to_string(),
            node_run_with_times(
                NodeRunStatus::Success,
                Some("2026-01-01T00:00:00Z"),
                Some("2026-01-01T00:05:00Z"),
            ),
        );
        let ctx = TaskContext {
            event: serde_json::json!({ "spec_slug": "11.T-run-summary-projection" }),
            nodes: HashMap::new(),
            metadata: serde_json::json!({}),
            node_runs,
        };
        let run_id = Uuid::new_v4();

        let dto = project_run_summary(run_id, &ctx, None);

        assert_eq!(dto.run_id, run_id.to_string());
        assert_eq!(dto.workflow_type, None);
        assert_eq!(dto.status, "success");
        assert_eq!(
            dto.spec_slug,
            Some("11.T-run-summary-projection".to_string())
        );
        assert_eq!(dto.started_at.as_deref(), Some("2026-01-01T00:00:00+00:00"));
        assert_eq!(dto.updated_at.as_deref(), Some("2026-01-01T00:05:00+00:00"));
    }

    #[test]
    fn project_run_summary_event_with_no_spec_slug_omits_field() {
        let ctx = TaskContext {
            event: serde_json::json!({ "ticket_id": "T-1" }),
            nodes: HashMap::new(),
            metadata: serde_json::json!({}),
            node_runs: HashMap::new(),
        };

        let dto = project_run_summary(Uuid::new_v4(), &ctx, None);

        assert_eq!(dto.spec_slug, None);
        assert_eq!(dto.status, "success");
        assert_eq!(dto.started_at, None);
        assert_eq!(dto.updated_at, None);
    }

    #[test]
    fn project_run_summary_cancelled_metadata_yields_cancelled_status() {
        let mut node_runs = HashMap::new();
        node_runs.insert("NodeA".to_string(), node_run(NodeRunStatus::Running));
        let ctx = TaskContext {
            event: serde_json::json!({}),
            nodes: HashMap::new(),
            metadata: serde_json::json!({ "cancellation": { "cancelled": true } }),
            node_runs,
        };

        let dto = project_run_summary(Uuid::new_v4(), &ctx, None);
        assert_eq!(dto.status, "cancelled");
    }

    #[test]
    fn project_run_summary_budget_halted_metadata_yields_budget_halted_status() {
        let mut node_runs = HashMap::new();
        node_runs.insert("NodeA".to_string(), node_run(NodeRunStatus::Running));
        let ctx = TaskContext {
            event: serde_json::json!({}),
            nodes: HashMap::new(),
            metadata: serde_json::json!({
                "budget": {
                    "halted": true,
                    "reason": {
                        "cap": "max_total_tokens",
                        "spent": 1000,
                        "limit": 500
                    }
                }
            }),
            node_runs,
        };

        let dto = project_run_summary(Uuid::new_v4(), &ctx, None);
        assert_eq!(dto.status, "budget_halted");
    }

    #[test]
    fn project_run_summary_suspended_metadata_yields_suspended_status() {
        let mut node_runs = HashMap::new();
        node_runs.insert("NodeA".to_string(), node_run(NodeRunStatus::Success));
        node_runs.insert("NodeB".to_string(), node_run(NodeRunStatus::Pending));
        let ctx = TaskContext {
            event: serde_json::json!({}),
            nodes: HashMap::new(),
            metadata: serde_json::json!({
                "suspension": { "suspended": true, "resume_at": "NodeB" }
            }),
            node_runs,
        };

        let dto = project_run_summary(Uuid::new_v4(), &ctx, None);
        assert_eq!(dto.status, "suspended");
    }

    #[test]
    fn project_run_summary_carries_resolved_repo() {
        let ctx = TaskContext {
            event: serde_json::json!({}),
            nodes: HashMap::new(),
            metadata: serde_json::json!({}),
            node_runs: HashMap::new(),
        };

        let dto = project_run_summary(Uuid::new_v4(), &ctx, Some("bastion".to_string()));
        assert_eq!(dto.repo, Some("bastion".to_string()));
    }

    // -- resolve_repo_for_run (A7) --

    fn workflow(repo: &str, run_id: Option<&str>) -> RepoWorkflowStateDto {
        RepoWorkflowStateDto {
            repo: repo.to_string(),
            spec_slug: "spec".to_string(),
            branch: "branch".to_string(),
            status: "running".to_string(),
            current_task: 1,
            started_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            run_id: run_id.map(str::to_owned),
        }
    }

    #[test]
    fn resolve_repo_for_run_exact_match_resolves() {
        let workflows = vec![
            workflow("bastion", Some("run-1")),
            workflow("bella", Some("run-2")),
        ];
        assert_eq!(
            resolve_repo_for_run("run-2", &workflows),
            Some("bella".to_string())
        );
    }

    #[test]
    fn resolve_repo_for_run_no_match_yields_none() {
        let workflows = vec![workflow("bastion", Some("run-1"))];
        assert_eq!(resolve_repo_for_run("run-9", &workflows), None);
    }

    #[test]
    fn resolve_repo_for_run_none_run_id_never_matches() {
        // A flow state that has never seen A1's `run_id` stamp must never
        // match a run -- absence must not match absence.
        let workflows = vec![workflow("bastion", None)];
        assert_eq!(resolve_repo_for_run("run-1", &workflows), None);
    }

    #[test]
    fn resolve_repo_for_run_empty_workflows_yields_none() {
        assert_eq!(resolve_repo_for_run("run-1", &[]), None);
    }

    #[test]
    fn resolve_repo_for_run_ambiguous_run_id_picks_first_deterministically() {
        // Two repos racing the same run_id (a misconfigured registry) --
        // resolution must be deterministic (first match in `workflows`'
        // order), never left to HashMap iteration order.
        let workflows = vec![
            workflow("alpha", Some("run-dup")),
            workflow("zeta", Some("run-dup")),
        ];
        assert_eq!(
            resolve_repo_for_run("run-dup", &workflows),
            Some("alpha".to_string())
        );

        // Same duplicate pair, reversed order in the slice: the guarantee is
        // "first in `workflows`' order", not "alphabetically first repo" --
        // callers get alphabetical-first for free only because
        // `collect_all_workflows` sorts by `(repo, spec_slug)` first.
        let reversed = vec![
            workflow("zeta", Some("run-dup")),
            workflow("alpha", Some("run-dup")),
        ];
        assert_eq!(
            resolve_repo_for_run("run-dup", &reversed),
            Some("zeta".to_string())
        );
    }

    // -- list_runs: ?with_repo=1 gate (A7) ---------------------------------------

    /// Minimal temp-dir helper that cleans up on drop (mirrors
    /// `handlers/status.rs`'s test helper).
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new() -> Self {
            static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let pid = std::process::id();
            let dir = std::env::temp_dir().join(format!("bastion_runs_handler_test_{pid}_{id}"));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Register one workspace (`repo-a`) with a real `sdlc-flow-state.json`
    /// on disk whose `run_id` is `run_id`, so a test can prove whether the
    /// registry walk that would find it actually ran.
    fn registry_with_flow_state(run_id: &str) -> (TempDir, FileConfig) {
        let tmp = TempDir::new();
        let flow_path = tmp.path().join("planning/spec-a/sdlc/sdlc-flow-state.json");
        std::fs::create_dir_all(flow_path.parent().unwrap()).unwrap();
        std::fs::write(
            &flow_path,
            serde_json::json!({
                "spec_slug": "spec-a",
                "branch": "spec-a-flow",
                "started_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z",
                "status": "running",
                "current_task": 1,
                "run_id": run_id,
            })
            .to_string(),
        )
        .unwrap();

        let mut workspaces = HashMap::new();
        workspaces.insert("repo-a".to_string(), tmp.path().to_path_buf());
        let registry = FileConfig {
            workspaces: Some(workspaces),
            ..Default::default()
        };
        (tmp, registry)
    }

    fn empty_ctx() -> TaskContext {
        TaskContext {
            event: serde_json::json!({}),
            nodes: HashMap::new(),
            metadata: serde_json::json!({}),
            node_runs: HashMap::new(),
        }
    }

    #[actix_web::test]
    async fn list_runs_without_with_repo_skips_the_registry_walk() {
        let run_id = Uuid::new_v4();
        let (_tmp, registry) = registry_with_flow_state(&run_id.to_string());

        let live = LiveStateStore::new();
        live.record(run_id, &empty_ctx());

        let resp = list_runs(
            web::Query(RunsQuery { with_repo: false }),
            web::Data::new(live),
            web::Data::new(registry),
        )
        .await;

        let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
        let summaries: Vec<RunSummaryDto> = serde_json::from_slice(&body).unwrap();
        assert_eq!(summaries.len(), 1);
        // A real flow state with a matching run_id exists on disk, but without
        // `?with_repo=1` the registry walk must never run: proven by the
        // absence of the join's effect. If the walk had run, `repo` would
        // have resolved to `Some("repo-a")` (see the sibling test below).
        assert_eq!(
            summaries[0].repo, None,
            "repo must stay absent when with_repo is unset, proving the registry walk did not run"
        );
    }

    #[actix_web::test]
    async fn list_runs_with_with_repo_resolves_the_join() {
        let run_id = Uuid::new_v4();
        let (_tmp, registry) = registry_with_flow_state(&run_id.to_string());

        let live = LiveStateStore::new();
        live.record(run_id, &empty_ctx());

        let resp = list_runs(
            web::Query(RunsQuery { with_repo: true }),
            web::Data::new(live),
            web::Data::new(registry),
        )
        .await;

        let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
        let summaries: Vec<RunSummaryDto> = serde_json::from_slice(&body).unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].repo, Some("repo-a".to_string()));
    }

    #[actix_web::test]
    async fn list_runs_with_with_repo_no_matching_flow_state_stays_absent() {
        let (_tmp, registry) = registry_with_flow_state("some-other-run-id");

        let live = LiveStateStore::new();
        live.record(Uuid::new_v4(), &empty_ctx());

        let resp = list_runs(
            web::Query(RunsQuery { with_repo: true }),
            web::Data::new(live),
            web::Data::new(registry),
        )
        .await;

        let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
        let summaries: Vec<RunSummaryDto> = serde_json::from_slice(&body).unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].repo, None);
    }
}
