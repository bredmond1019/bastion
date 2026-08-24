use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::observ::errors::ConsoleError;

/// Outcome of probing the orchestrator's `/health` endpoint.
/// Unreachable is a normal outcome (not an `Err`) so `bastion status` never fails on it.
#[derive(Debug, Clone, PartialEq)]
pub enum ApiStatus {
    Reachable { status: String, version: String },
    Unreachable(String),
}

/// Orchestrator `/health` body — `{ "status": ..., "version": ... }` (recon 2026-06-18).
#[derive(Debug, Deserialize)]
struct HealthBody {
    status: String,
    version: String,
}

/// Request body for `POST /` — the generic workflow dispatcher.
/// Serializes as `{ "workflow_type": "...", "data": {...} }`.
#[derive(Debug, Serialize)]
struct TriggerRequest {
    workflow_type: String,
    data: serde_json::Value,
}

/// Response body for `POST /` — `202 { "task_id": "...", "message": "..." }`
/// pre-v1.2.0, or `202 { "task_id": "...", "event_id": "...", "message": "..." }`
/// per the current data contract (v1.2.0). `event_id` is `#[serde(default)]`
/// so both shapes deserialize without error: an orchestrator response that
/// predates the `event_id` field (or the field simply being absent from a
/// given trigger path) leaves it `None` rather than failing to parse.
#[derive(Debug, Deserialize)]
struct TaskAccepted {
    task_id: String,
    #[serde(default)]
    event_id: Option<String>,
    #[allow(dead_code)]
    message: String,
}

/// The two ids a `trigger_workflow` call can hand back — `task_id` (the
/// Celery task id) and, when the response carries it (data contract v1.2.0),
/// `event_id` (the `events.id` row the engine/orchestrator actually writes
/// execution state under). Handoff callers (e.g. `run::trigger`'s
/// `--monitor` attach) should prefer `event_id` when present — `task_id` may
/// not equal `events.id`, so attaching `monitor::run` to `task_id` can watch
/// the wrong row when the two diverge.
#[derive(Debug, Clone, PartialEq)]
pub struct TriggerOutcome {
    pub task_id: String,
    pub event_id: Option<String>,
}

impl TriggerOutcome {
    /// The id `monitor::run`'s `--workflow-id` filter should attach to:
    /// `event_id` when the response carried one, falling back to `task_id`
    /// otherwise. Pure — no I/O.
    pub fn monitor_id(&self) -> &str {
        self.event_id.as_deref().unwrap_or(&self.task_id)
    }
}

/// Build a `TriggerRequest` from a workflow type and optional data payload.
/// A `None` data argument serializes as `"data": {}` (empty object), matching
/// the orchestrator's `data: dict` field.
/// Pure function — no I/O — so it is unit-testable without a live server.
fn trigger_body(
    workflow_type: impl Into<String>,
    data: Option<serde_json::Value>,
) -> TriggerRequest {
    TriggerRequest {
        workflow_type: workflow_type.into(),
        data: data.unwrap_or_else(|| serde_json::Value::Object(Default::default())),
    }
}

/// `GET /workflows/{type}/graph` body — the static DAG (data contract §7).
/// The only source of edges and of not-yet-run nodes; joined to live `node_runs`
/// state by node class name.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct WorkflowGraph {
    pub nodes: Vec<String>,
    pub edges: Vec<(String, String)>,
}

/// `202 { "run_id": "...", "status": "aborting" }` — the pinned success body
/// for `POST /events/{run_id}/abort` (data contract, Abort section).
#[derive(Debug, Deserialize)]
struct AbortAccepted {
    run_id: String,
    status: String,
}

/// Outcome of a `POST /events/{run_id}/abort` call — one variant per shape
/// the pinned contract defines for that endpoint. `NotFound` and
/// `Unauthorized` carry the `ConsoleError` (and so the `C0xx` code)
/// `run::abort` (task 5) renders; a connection/transport failure is not a
/// member of this enum — [`ApiClient::abort_run`] returns that as `Err`
/// instead, since the call never produced a pinned response to classify.
/// `Accepted` carries no `ConsoleError`: a `C0xx` code is by construction an
/// error/degradation signal (`src/observ/errors.rs`), and a successful abort
/// is neither.
#[derive(Debug)]
pub enum AbortOutcome {
    /// `202` — the run's cancellation token has been triggered.
    Accepted { run_id: String, status: String },
    /// `404` — unknown or already-finished run id.
    NotFound(ConsoleError),
    /// `401` — missing or bad `X-API-Key`.
    Unauthorized(ConsoleError),
}

/// Classify a `POST /events/{run_id}/abort` HTTP response into a typed
/// [`AbortOutcome`], per the pinned contract: `202` → accepted (with the
/// body decoded), `404` → unknown/finished run, `401` → bad/missing key.
/// A `202` whose body doesn't match the pinned shape, or any other status,
/// is a decode/contract-mismatch failure (`ConsoleError::SerializationError`
/// / `ConsoleError::Io`) rather than a normal outcome.
///
/// Pure — no I/O — so it is unit-testable against fixtures without a live
/// server (Rule 6); the `reqwest` send/receive in [`ApiClient::abort_run`]
/// is the thin shell over this.
fn classify_abort_response(status: u16, body: &str) -> Result<AbortOutcome, ConsoleError> {
    match status {
        202 => serde_json::from_str::<AbortAccepted>(body)
            .map(|accepted| AbortOutcome::Accepted {
                run_id: accepted.run_id,
                status: accepted.status,
            })
            .map_err(|e| {
                ConsoleError::SerializationError(format!(
                    "decoding 202 abort response body: {e} (body: {body})"
                ))
            }),
        404 => Ok(AbortOutcome::NotFound(ConsoleError::SessionNotFound(
            "run not found or already finished".to_string(),
        ))),
        401 => Ok(AbortOutcome::Unauthorized(ConsoleError::NotAuthenticated)),
        other => Err(ConsoleError::Io(format!(
            "unexpected abort response status {other} (body: {body})"
        ))),
    }
}

/// `202 { "run_id": "...", "event_id": "...", "status": "resuming", "resume_at": "..." }`
/// — the pinned success body for `POST /events/{run_id}/resume`, per
/// `../engine-rs/crates/engine-serve/src/resume.rs::resume_run`.
#[derive(Debug, Deserialize)]
struct ResumeAccepted {
    run_id: String,
    status: String,
    resume_at: String,
}

/// Outcome of a `POST /events/{run_id}/resume` call — one variant per shape
/// the pinned engine contract defines for that endpoint, mirroring
/// [`AbortOutcome`]'s construction. A connection/transport failure is not a
/// member of this enum — [`ApiClient::resume_run`] returns that as `Err`
/// instead, since the call never produced a pinned response to classify.
#[derive(Debug)]
pub enum ResumeOutcome {
    /// `202` — the run has been told to resume.
    Accepted {
        run_id: String,
        status: String,
        resume_at: String,
    },
    /// `404 {"error": "unknown or non-resumable run"}`.
    NotFound(ConsoleError),
    /// `401` — missing or bad `X-API-Key` (empty body).
    Unauthorized(ConsoleError),
    /// `409 {"error": "resume already in flight"}`.
    AlreadyResuming(ConsoleError),
    /// `422 {"error": ...}` — the tap could not be resolved to a resumable
    /// state (e.g. malformed payload / stale snapshot).
    Unresolvable(ConsoleError),
}

/// Pinned `{"error": "..."}` shape shared by the engine's `404`/`409`/`422`
/// resume responses.
#[derive(Debug, Deserialize)]
struct ResumeErrorBody {
    error: String,
}

/// Classify a `POST /events/{run_id}/resume` HTTP response into a typed
/// [`ResumeOutcome`], per the pinned engine contract (`resume.rs::resume_run`):
/// `202` → accepted (body decoded), `404` → unknown/non-resumable run,
/// `409` → resume already in flight, `401` → bad/missing key (empty body),
/// `422` → unresolvable. A `202` whose body doesn't match the pinned shape,
/// or any other status, is a decode/contract-mismatch failure
/// (`ConsoleError::SerializationError` / `ConsoleError::Io`) rather than a
/// normal outcome.
///
/// Pure — no I/O — so it is unit-testable against fixtures without a live
/// server (Rule 6); the `reqwest` send/receive in [`ApiClient::resume_run`]
/// is the thin shell over this, mirroring [`classify_abort_response`].
fn classify_resume_response(status: u16, body: &str) -> Result<ResumeOutcome, ConsoleError> {
    match status {
        202 => serde_json::from_str::<ResumeAccepted>(body)
            .map(|accepted| ResumeOutcome::Accepted {
                run_id: accepted.run_id,
                status: accepted.status,
                resume_at: accepted.resume_at,
            })
            .map_err(|e| {
                ConsoleError::SerializationError(format!(
                    "decoding 202 resume response body: {e} (body: {body})"
                ))
            }),
        404 => Ok(ResumeOutcome::NotFound(ConsoleError::SessionNotFound(
            body_error_message(body).unwrap_or_else(|| "unknown or non-resumable run".to_string()),
        ))),
        409 => Ok(ResumeOutcome::AlreadyResuming(ConsoleError::InvalidInput(
            body_error_message(body).unwrap_or_else(|| "resume already in flight".to_string()),
        ))),
        401 => Ok(ResumeOutcome::Unauthorized(ConsoleError::NotAuthenticated)),
        422 => Ok(ResumeOutcome::Unresolvable(ConsoleError::InvalidInput(
            body_error_message(body).unwrap_or_else(|| "unresolvable resume request".to_string()),
        ))),
        other => Err(ConsoleError::Io(format!(
            "unexpected resume response status {other} (body: {body})"
        ))),
    }
}

/// Best-effort decode of the pinned `{"error": "..."}` body shared by the
/// engine's `404`/`409`/`422` resume responses. Returns `None` (rather than
/// erroring) when the body doesn't match — the caller falls back to a fixed
/// message so a body-shape drift on an already-typed error status degrades
/// gracefully instead of blocking classification.
fn body_error_message(body: &str) -> Option<String> {
    serde_json::from_str::<ResumeErrorBody>(body)
        .ok()
        .map(|b| b.error)
}

pub struct ApiClient {
    base_url: String,
    client: reqwest::Client,
    /// The engine's `X-API-Key` secret (task 1's `engine_api_key`), used only
    /// by [`ApiClient::abort_run`]. `None` by default so the existing
    /// orchestrator-facing methods (`health`, `trigger_workflow`,
    /// `workflow_graph`), which never touch the engine, are unaffected.
    engine_api_key: Option<String>,
}

impl ApiClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.to_string(),
            client: reqwest::Client::new(),
            engine_api_key: None,
        }
    }

    /// Attach the engine's `X-API-Key` secret for [`ApiClient::abort_run`] to
    /// send. A separate builder (rather than a `new` parameter) so existing
    /// `ApiClient::new(base_url)` call sites — which only ever talk to the
    /// orchestrator health/trigger endpoints, never the engine — are
    /// unaffected.
    pub fn with_engine_api_key(mut self, key: Option<String>) -> Self {
        self.engine_api_key = key;
        self
    }

    /// Returns the abort URL for `run_id` — `POST /events/{run_id}/abort`
    /// (data contract, Abort section), served by `engine-serve`'s route
    /// table (embedded in `bastion serve`, task 2), never the orchestrator.
    fn abort_url(&self, run_id: &str) -> String {
        format!(
            "{}/events/{run_id}/abort",
            self.base_url.trim_end_matches('/')
        )
    }

    /// Call `POST /events/{run_id}/abort` with no body and the `X-API-Key`
    /// header, per the pinned wire shape. Per D25, this only triggers the
    /// abort — bastion never cancels a run itself, writes the `events` row,
    /// or touches Celery/Redis.
    ///
    /// A missing `engine_api_key` is a typed `ConfigError`, not an
    /// unauthenticated request. A connection/transport failure is an `Io`
    /// error. `202`/`404`/`401` classify via [`classify_abort_response`].
    pub async fn abort_run(&self, run_id: &str) -> Result<AbortOutcome, ConsoleError> {
        let key = self.engine_api_key.as_deref().ok_or_else(|| {
            ConsoleError::ConfigError(
                "engine_api_key not configured — set BASTION_ENGINE_API_KEY or config.toml's \
                 engine_api_key"
                    .to_string(),
            )
        })?;

        let url = self.abort_url(run_id);
        let resp = self
            .client
            .post(&url)
            .header("X-API-Key", key)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| {
                ConsoleError::Io(format!("connecting to engine abort endpoint at {url}: {e}"))
            })?;

        let status = resp.status().as_u16();
        let body = resp
            .text()
            .await
            .map_err(|e| ConsoleError::Io(format!("reading abort response body: {e}")))?;

        classify_abort_response(status, &body)
    }

    /// Returns the resume URL for `run_id` — `POST /events/{run_id}/resume`,
    /// served by `engine-serve`'s route table (embedded in `bastion serve`),
    /// mirroring [`ApiClient::abort_url`].
    fn resume_url(&self, run_id: &str) -> String {
        format!(
            "{}/events/{run_id}/resume",
            self.base_url.trim_end_matches('/')
        )
    }

    /// Call `POST /events/{run_id}/resume` with no body and the `X-API-Key`
    /// header, per the pinned wire shape — the headless question path's
    /// resume call (BA.21.C), issued after an operator tap resolves via
    /// `session_qa::headless::headless_resume_for`.
    ///
    /// A missing `engine_api_key` is a typed `ConfigError`, not an
    /// unauthenticated request. A connection/transport failure is an `Io`
    /// error. `202`/`404`/`409`/`401`/`422` classify via
    /// [`classify_resume_response`]. Thin shell over that pure classifier,
    /// exactly like [`ApiClient::abort_run`].
    pub async fn resume_run(&self, run_id: &str) -> Result<ResumeOutcome, ConsoleError> {
        let key = self.engine_api_key.as_deref().ok_or_else(|| {
            ConsoleError::ConfigError(
                "engine_api_key not configured — set BASTION_ENGINE_API_KEY or config.toml's \
                 engine_api_key"
                    .to_string(),
            )
        })?;

        let url = self.resume_url(run_id);
        let resp = self
            .client
            .post(&url)
            .header("X-API-Key", key)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| {
                ConsoleError::Io(format!(
                    "connecting to engine resume endpoint at {url}: {e}"
                ))
            })?;

        let status = resp.status().as_u16();
        let body = resp
            .text()
            .await
            .map_err(|e| ConsoleError::Io(format!("reading resume response body: {e}")))?;

        classify_resume_response(status, &body)
    }

    /// Returns the full health URL for the configured base URL.
    fn health_url(&self) -> String {
        format!("{}/health", self.base_url.trim_end_matches('/'))
    }

    /// Fetch a workflow's static DAG from `GET /workflows/{type}/graph`.
    /// Edges (and pending nodes) come only from here; live state comes from
    /// polling Postgres `node_runs`, joined by class name (data contract §2).
    pub async fn workflow_graph(&self, workflow_type: &str) -> Result<WorkflowGraph> {
        let url = format!(
            "{}/workflows/{workflow_type}/graph",
            self.base_url.trim_end_matches('/')
        );
        self.client
            .get(&url)
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .context("requesting workflow graph")?
            .error_for_status()
            .context("workflow graph endpoint returned an error status")?
            .json::<WorkflowGraph>()
            .await
            .context("decoding workflow graph body")
    }

    /// Returns the trigger URL for `POST /` — base URL with any trailing slash preserved as a
    /// single `/`, so both `http://host:8080` and `http://host:8080/` produce `http://host:8080/`.
    fn trigger_url(&self) -> String {
        format!("{}/", self.base_url.trim_end_matches('/'))
    }

    pub async fn trigger_workflow(
        &self,
        workflow_type: &str,
        data: Option<serde_json::Value>,
    ) -> Result<TriggerOutcome> {
        // Orchestrator's generic dispatcher: POST / with {workflow_type, data}
        // → 202 {task_id, event_id?, message} (data contract §7, event_id per
        // v1.2.0). Returns both ids — callers that only need `task_id` (the
        // pre-v1.2.0 shape) can ignore `event_id`.
        let url = self.trigger_url();
        let body = trigger_body(workflow_type, data);
        self.client
            .post(&url)
            .json(&body)
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .context("sending trigger request to orchestrator")?
            .error_for_status()
            .context("orchestrator trigger endpoint returned an error status (check workflow_type and data)")?
            .json::<TaskAccepted>()
            .await
            .context("decoding trigger response body")
            .map(|accepted| TriggerOutcome {
                task_id: accepted.task_id,
                event_id: accepted.event_id,
            })
    }

    pub async fn rerun_node(&self, _run_id: &str, _node_id: &str) -> Result<()> {
        // No orchestrator re-run endpoint exists today — this is a future
        // contract ADDITION the Python side must make first (data contract §7).
        anyhow::bail!("Phase 4: requires a new orchestrator re-run endpoint")
    }

    pub async fn health(&self) -> ApiStatus {
        let url = self.health_url();
        let resp = self
            .client
            .get(&url)
            .timeout(Duration::from_secs(2))
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => match r.json::<HealthBody>().await {
                Ok(body) => ApiStatus::Reachable {
                    status: body.status,
                    version: body.version,
                },
                Err(e) => ApiStatus::Unreachable(format!("invalid health body: {e}")),
            },
            Ok(r) => ApiStatus::Unreachable(format!("HTTP {}", r.status())),
            Err(e) => ApiStatus::Unreachable(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observ::errors::ErrorCode;

    #[test]
    fn api_status_reachable_equality() {
        let a = ApiStatus::Reachable {
            status: "ok".to_string(),
            version: "1.0.0".to_string(),
        };
        let b = ApiStatus::Reachable {
            status: "ok".to_string(),
            version: "1.0.0".to_string(),
        };
        assert_eq!(a, b);
    }

    #[test]
    fn api_status_unreachable_equality() {
        let a = ApiStatus::Unreachable("connection refused".to_string());
        let b = ApiStatus::Unreachable("connection refused".to_string());
        assert_eq!(a, b);
    }

    #[test]
    fn api_status_reachable_ne_unreachable() {
        let reachable = ApiStatus::Reachable {
            status: "ok".to_string(),
            version: "1.0.0".to_string(),
        };
        let unreachable = ApiStatus::Unreachable("error".to_string());
        assert_ne!(reachable, unreachable);
    }

    #[test]
    fn api_status_debug_contains_variant_name() {
        let s = format!("{:?}", ApiStatus::Unreachable("timeout".to_string()));
        assert!(s.contains("Unreachable"));
        assert!(s.contains("timeout"));

        let r = format!(
            "{:?}",
            ApiStatus::Reachable {
                status: "ok".to_string(),
                version: "0.1.0".to_string(),
            }
        );
        assert!(r.contains("Reachable"));
    }

    #[test]
    fn health_url_trailing_slash_stripped() {
        let client = ApiClient::new("http://localhost:8000/");
        assert_eq!(client.health_url(), "http://localhost:8000/health");
    }

    #[test]
    fn health_url_no_trailing_slash() {
        let client = ApiClient::new("http://localhost:8000");
        assert_eq!(client.health_url(), "http://localhost:8000/health");
    }

    // ── trigger_body ─────────────────────────────────────────────────────────

    #[test]
    fn trigger_body_some_data_serializes_correctly() {
        let data = serde_json::json!({"key": "value", "count": 42});
        let body = trigger_body("my_workflow", Some(data));
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["workflow_type"], "my_workflow");
        assert_eq!(json["data"]["key"], "value");
        assert_eq!(json["data"]["count"], 42);
    }

    #[test]
    fn trigger_body_none_data_serializes_as_empty_object() {
        let body = trigger_body("my_workflow", None);
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["workflow_type"], "my_workflow");
        assert_eq!(json["data"], serde_json::json!({}));
    }

    #[test]
    fn trigger_body_workflow_type_preserved() {
        let body = trigger_body("research_workflow", None);
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["workflow_type"], "research_workflow");
    }

    // ── trigger_url ───────────────────────────────────────────────────────────

    #[test]
    fn trigger_url_trailing_slash_stripped_and_readded() {
        let client = ApiClient::new("http://localhost:8080/");
        assert_eq!(client.trigger_url(), "http://localhost:8080/");
    }

    #[test]
    fn trigger_url_no_trailing_slash_appended() {
        let client = ApiClient::new("http://localhost:8080");
        assert_eq!(client.trigger_url(), "http://localhost:8080/");
    }

    // ── abort_url ─────────────────────────────────────────────────────────────

    #[test]
    fn abort_url_trailing_slash_stripped() {
        let client = ApiClient::new("http://localhost:8080/");
        assert_eq!(
            client.abort_url("run-123"),
            "http://localhost:8080/events/run-123/abort"
        );
    }

    #[test]
    fn abort_url_no_trailing_slash() {
        let client = ApiClient::new("http://localhost:8080");
        assert_eq!(
            client.abort_url("run-123"),
            "http://localhost:8080/events/run-123/abort"
        );
    }

    // ── classify_abort_response ─────────────────────────────────────────────────

    #[test]
    fn classify_202_accepted_decodes_run_id_and_status() {
        let body = r#"{"run_id": "abc-123", "status": "aborting"}"#;
        let outcome = classify_abort_response(202, body).expect("202 should classify");
        match outcome {
            AbortOutcome::Accepted { run_id, status } => {
                assert_eq!(run_id, "abc-123");
                assert_eq!(status, "aborting");
            }
            other => panic!("expected Accepted, got {other:?}"),
        }
    }

    #[test]
    fn classify_202_malformed_body_is_serialization_error() {
        let body = r#"{"not_run_id": "abc-123"}"#;
        let err = classify_abort_response(202, body).expect_err("malformed 202 should error");
        assert_eq!(err.code(), ErrorCode::SerializationError);
    }

    #[test]
    fn classify_202_non_json_body_is_serialization_error() {
        let err =
            classify_abort_response(202, "not json at all").expect_err("bad JSON should error");
        assert_eq!(err.code(), ErrorCode::SerializationError);
    }

    #[test]
    fn classify_404_is_not_found() {
        let outcome = classify_abort_response(404, "").expect("404 should classify");
        match outcome {
            AbortOutcome::NotFound(err) => assert_eq!(err.code(), ErrorCode::SessionNotFound),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn classify_401_is_unauthorized() {
        let outcome = classify_abort_response(401, "").expect("401 should classify");
        match outcome {
            AbortOutcome::Unauthorized(err) => {
                assert_eq!(err.code(), ErrorCode::NotAuthenticated)
            }
            other => panic!("expected Unauthorized, got {other:?}"),
        }
    }

    #[test]
    fn classify_unexpected_status_is_io_error() {
        let err = classify_abort_response(500, "boom").expect_err("500 should error");
        assert_eq!(err.code(), ErrorCode::IoError);
    }

    // ── TriggerOutcome::monitor_id — event_id vs task_id handoff ────────────────

    #[test]
    fn monitor_id_prefers_event_id_when_present() {
        let outcome = TriggerOutcome {
            task_id: "task-1".to_string(),
            event_id: Some("event-1".to_string()),
        };
        assert_eq!(outcome.monitor_id(), "event-1");
    }

    #[test]
    fn monitor_id_falls_back_to_task_id_when_event_id_absent() {
        let outcome = TriggerOutcome {
            task_id: "task-1".to_string(),
            event_id: None,
        };
        assert_eq!(outcome.monitor_id(), "task-1");
    }

    // ── TaskAccepted deserialization — event_id is optional (v1.2.0) ────────────

    #[test]
    fn task_accepted_decodes_without_event_id() {
        let body = r#"{"task_id": "t1", "message": "accepted"}"#;
        let accepted: TaskAccepted = serde_json::from_str(body).unwrap();
        assert_eq!(accepted.task_id, "t1");
        assert!(accepted.event_id.is_none());
    }

    #[test]
    fn task_accepted_decodes_with_event_id() {
        let body = r#"{"task_id": "t1", "event_id": "e1", "message": "accepted"}"#;
        let accepted: TaskAccepted = serde_json::from_str(body).unwrap();
        assert_eq!(accepted.task_id, "t1");
        assert_eq!(accepted.event_id.as_deref(), Some("e1"));
    }

    // ── abort_run — missing engine_api_key ──────────────────────────────────────

    #[tokio::test]
    async fn abort_run_without_engine_api_key_is_config_error() {
        let client = ApiClient::new("http://localhost:1");
        let err = client.abort_run("run-123").await.expect_err(
            "missing engine_api_key should be a typed error, not an unauthenticated call",
        );
        assert_eq!(err.code(), ErrorCode::ConfigError);
    }

    #[tokio::test]
    async fn abort_run_connection_failure_is_io_error() {
        // Port 1 refuses connections on any dev/CI machine (no listener) —
        // a deterministic transport failure without a live server.
        let client =
            ApiClient::new("http://127.0.0.1:1").with_engine_api_key(Some("key".to_string()));
        let err = client
            .abort_run("run-123")
            .await
            .expect_err("connection failure should be a typed error");
        assert_eq!(err.code(), ErrorCode::IoError);
    }

    // ── resume_url ────────────────────────────────────────────────────────────

    #[test]
    fn resume_url_trailing_slash_stripped() {
        let client = ApiClient::new("http://localhost:8080/");
        assert_eq!(
            client.resume_url("run-123"),
            "http://localhost:8080/events/run-123/resume"
        );
    }

    #[test]
    fn resume_url_no_trailing_slash() {
        let client = ApiClient::new("http://localhost:8080");
        assert_eq!(
            client.resume_url("run-123"),
            "http://localhost:8080/events/run-123/resume"
        );
    }

    // ── classify_resume_response ─────────────────────────────────────────────────

    #[test]
    fn classify_202_resume_accepted_decodes_fields() {
        let body = r#"{"run_id": "abc-123", "event_id": "ev-1", "status": "resuming", "resume_at": "2026-08-24T00:00:00Z"}"#;
        let outcome = classify_resume_response(202, body).expect("202 should classify");
        match outcome {
            ResumeOutcome::Accepted {
                run_id,
                status,
                resume_at,
            } => {
                assert_eq!(run_id, "abc-123");
                assert_eq!(status, "resuming");
                assert_eq!(resume_at, "2026-08-24T00:00:00Z");
            }
            other => panic!("expected Accepted, got {other:?}"),
        }
    }

    #[test]
    fn classify_202_resume_malformed_body_is_serialization_error() {
        let body = r#"{"not_run_id": "abc-123"}"#;
        let err = classify_resume_response(202, body).expect_err("malformed 202 should error");
        assert_eq!(err.code(), ErrorCode::SerializationError);
    }

    #[test]
    fn classify_resume_404_is_not_found() {
        let body = r#"{"error": "unknown or non-resumable run"}"#;
        let outcome = classify_resume_response(404, body).expect("404 should classify");
        match outcome {
            ResumeOutcome::NotFound(err) => assert_eq!(err.code(), ErrorCode::SessionNotFound),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn classify_resume_409_is_already_resuming() {
        let body = r#"{"error": "resume already in flight"}"#;
        let outcome = classify_resume_response(409, body).expect("409 should classify");
        match outcome {
            ResumeOutcome::AlreadyResuming(err) => {
                assert_eq!(err.code(), ErrorCode::InvalidInput)
            }
            other => panic!("expected AlreadyResuming, got {other:?}"),
        }
    }

    #[test]
    fn classify_resume_401_is_unauthorized() {
        let outcome = classify_resume_response(401, "").expect("401 should classify");
        match outcome {
            ResumeOutcome::Unauthorized(err) => {
                assert_eq!(err.code(), ErrorCode::NotAuthenticated)
            }
            other => panic!("expected Unauthorized, got {other:?}"),
        }
    }

    #[test]
    fn classify_resume_422_is_unresolvable() {
        let body = r#"{"error": "cannot resolve snapshot"}"#;
        let outcome = classify_resume_response(422, body).expect("422 should classify");
        match outcome {
            ResumeOutcome::Unresolvable(err) => assert_eq!(err.code(), ErrorCode::InvalidInput),
            other => panic!("expected Unresolvable, got {other:?}"),
        }
    }

    #[test]
    fn classify_resume_unexpected_status_is_io_error() {
        let err = classify_resume_response(500, "boom").expect_err("500 should error");
        assert_eq!(err.code(), ErrorCode::IoError);
    }

    // ── resume_run — missing engine_api_key ──────────────────────────────────────

    #[tokio::test]
    async fn resume_run_without_engine_api_key_is_config_error() {
        let client = ApiClient::new("http://localhost:1");
        let err = client.resume_run("run-123").await.expect_err(
            "missing engine_api_key should be a typed error, not an unauthenticated call",
        );
        assert_eq!(err.code(), ErrorCode::ConfigError);
    }

    #[tokio::test]
    async fn resume_run_connection_failure_is_io_error() {
        // Port 1 refuses connections on any dev/CI machine (no listener) —
        // a deterministic transport failure without a live server.
        let client =
            ApiClient::new("http://127.0.0.1:1").with_engine_api_key(Some("key".to_string()));
        let err = client
            .resume_run("run-123")
            .await
            .expect_err("connection failure should be a typed error");
        assert_eq!(err.code(), ErrorCode::IoError);
    }
}
