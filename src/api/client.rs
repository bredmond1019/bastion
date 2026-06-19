// HTTP client for the Python orchestrator FastAPI layer.
// Used by `bastion run` to trigger workflows and re-run nodes.

use anyhow::Result;

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

    pub async fn trigger_workflow(&self, _name: &str, _args: Option<serde_json::Value>) -> Result<String> {
        todo!("Phase 3: POST /workflows/{{name}}/run, return run_id")
    }

    pub async fn rerun_node(&self, _run_id: &str, _node_id: &str) -> Result<()> {
        todo!("Phase 4: POST /workflows/{{run_id}}/nodes/{{node_id}}/rerun")
    }

    pub async fn health(&self) -> Result<bool> {
        todo!("Phase 0: GET /health, return true if 200")
    }
}
