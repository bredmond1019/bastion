use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

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

pub struct ApiClient {
    base_url: String,
    client: reqwest::Client,
}

impl ApiClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.to_string(),
            client: reqwest::Client::new(),
        }
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
}
