use anyhow::Result;
use serde::Deserialize;
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

    pub async fn trigger_workflow(
        &self,
        _name: &str,
        _args: Option<serde_json::Value>,
    ) -> Result<String> {
        todo!("Phase 3: POST /workflows/{{name}}/run, return run_id")
    }

    pub async fn rerun_node(&self, _run_id: &str, _node_id: &str) -> Result<()> {
        todo!("Phase 4: POST /workflows/{{run_id}}/nodes/{{node_id}}/rerun")
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
}
