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

/// Response body for `POST /` — `202 { "task_id": "...", "message": "..." }`.
#[derive(Debug, Deserialize)]
struct TaskAccepted {
    task_id: String,
    #[allow(dead_code)]
    message: String,
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
    ) -> Result<String> {
        // Orchestrator's generic dispatcher: POST / with {workflow_type, data}
        // → 202 {task_id, message} (data contract §7). Returns the task_id.
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
            .map(|accepted| accepted.task_id)
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
}
