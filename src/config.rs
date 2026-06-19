use anyhow::{Context, Result};

pub struct Config {
    pub database_url: String,
    pub api_base_url: String,
    pub poll_interval_secs: u64,
}

impl Config {
    pub fn load() -> Result<Self> {
        dotenvy::dotenv().ok();

        let database_url = std::env::var("DATABASE_URL")
            .context("DATABASE_URL must be set (point to the Python orchestrator's PostgreSQL)")?;

        let api_base_url = std::env::var("BASTION_API_URL")
            .unwrap_or_else(|_| "http://localhost:8000".to_string());

        let poll_interval_secs = std::env::var("BASTION_POLL_INTERVAL")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(2);

        Ok(Self { database_url, api_base_url, poll_interval_secs })
    }
}
