use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum ConfigError {
    #[error("{0} must be set (point to the Python orchestrator's PostgreSQL)")]
    MissingVar(&'static str),
}

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub api_base_url: String,
    // Used by Phase 1 monitor; present now to keep config loading complete.
    #[allow(dead_code)]
    pub poll_interval_secs: u64,
}

impl Config {
    /// Default FastAPI base URL — orchestrator `/health` lives on port 8080
    /// (recon 2026-06-18; the scaffold's old 8000 default was wrong).
    const DEFAULT_API_URL: &'static str = "http://localhost:8080";

    pub fn load() -> Result<Self, ConfigError> {
        dotenvy::dotenv().ok();
        Self::from_vars(
            std::env::var("DATABASE_URL").ok(),
            std::env::var("BASTION_API_URL").ok(),
            std::env::var("BASTION_POLL_INTERVAL").ok(),
        )
    }

    /// Pure parser — no env access, so unit tests can call it directly.
    pub fn from_vars(
        database_url: Option<String>,
        api_base_url: Option<String>,
        poll_interval: Option<String>,
    ) -> Result<Self, ConfigError> {
        let database_url = database_url.ok_or(ConfigError::MissingVar("DATABASE_URL"))?;
        let api_base_url = api_base_url.unwrap_or_else(|| Self::DEFAULT_API_URL.to_string());
        let poll_interval_secs = poll_interval.and_then(|s| s.parse().ok()).unwrap_or(2);
        Ok(Self {
            database_url,
            api_base_url,
            poll_interval_secs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_when_all_vars_present() {
        let c = Config::from_vars(
            Some("postgres://localhost/db".into()),
            Some("http://localhost:9000".into()),
            Some("5".into()),
        )
        .expect("should parse");
        assert_eq!(c.database_url, "postgres://localhost/db");
        assert_eq!(c.api_base_url, "http://localhost:9000");
        assert_eq!(c.poll_interval_secs, 5);
    }

    #[test]
    fn applies_defaults_for_optional_vars() {
        let c = Config::from_vars(Some("postgres://localhost/db".into()), None, None)
            .expect("should parse");
        assert_eq!(c.api_base_url, "http://localhost:8080");
        assert_eq!(c.poll_interval_secs, 2);
    }

    #[test]
    fn missing_database_url_is_typed_error_not_panic() {
        let err = Config::from_vars(None, None, None).unwrap_err();
        assert_eq!(err, ConfigError::MissingVar("DATABASE_URL"));
    }
}
