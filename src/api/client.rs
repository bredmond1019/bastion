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

    pub async fn health(&self) -> ApiStatus {
        let url = format!("{}/health", self.base_url.trim_end_matches('/'));
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

    #[allow(dead_code)]
    pub async fn trigger_workflow(&self, _workflow_id: &str) -> Result<()> {
        todo!("Phase 3: trigger workflow via FastAPI")
    }

    #[allow(dead_code)]
    pub async fn rerun_node(&self, _workflow_id: &str, _node_id: &str) -> Result<()> {
        todo!("Phase 4: rerun node via FastAPI")
    }
}
