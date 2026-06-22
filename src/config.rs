use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum ConfigError {
    #[error("{0} must be set (point to the Python orchestrator's PostgreSQL)")]
    MissingVar(&'static str),
    #[error("config file is malformed: {0}")]
    MalformedFile(String),
}

/// Fields mirroring env vars — all optional; used as the fallback layer beneath env vars.
/// Unknown keys are silently ignored (no `deny_unknown_fields`).
#[derive(Debug, serde::Deserialize, Default, PartialEq)]
pub struct FileConfig {
    pub database_url: Option<String>,
    pub api_base_url: Option<String>,
    pub poll_interval: Option<u64>,
}

/// Parse TOML `contents` into a `FileConfig`.
/// Empty string returns `FileConfig::default()`.
/// Malformed TOML returns `ConfigError::MalformedFile`.
pub fn parse_file(contents: &str) -> Result<FileConfig, ConfigError> {
    if contents.trim().is_empty() {
        return Ok(FileConfig::default());
    }
    toml::from_str(contents).map_err(|e| ConfigError::MalformedFile(e.to_string()))
}

/// Resolve `$XDG_CONFIG_HOME/bastion/config.toml`, falling back to
/// `$HOME/.config/bastion/config.toml`. Returns `None` when neither is set.
/// Pure function — reads only the two supplied env values, no I/O.
pub fn config_path(xdg_config_home: Option<String>, home: Option<String>) -> Option<PathBuf> {
    if let Some(xdg) = xdg_config_home {
        Some(PathBuf::from(xdg).join("bastion").join("config.toml"))
    } else {
        home.map(|h| {
            PathBuf::from(h)
                .join(".config")
                .join("bastion")
                .join("config.toml")
        })
    }
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

        // Resolve optional config file — absent or unreadable → silently degrade.
        // Present but malformed → propagate ConfigError::MalformedFile.
        let file_config = match config_path(
            std::env::var("XDG_CONFIG_HOME").ok(),
            std::env::var("HOME").ok(),
        ) {
            Some(path) => match std::fs::read_to_string(&path) {
                Ok(contents) => parse_file(&contents)?,
                Err(_) => FileConfig::default(),
            },
            None => FileConfig::default(),
        };

        Self::from_sources(
            (
                std::env::var("DATABASE_URL").ok(),
                std::env::var("BASTION_API_URL").ok(),
                std::env::var("BASTION_POLL_INTERVAL").ok(),
            ),
            file_config,
        )
    }

    /// Merge env vars (highest precedence) with file config (middle) and built-in defaults
    /// (lowest). `DATABASE_URL` must be satisfied by at least one source.
    pub fn from_sources(
        env: (Option<String>, Option<String>, Option<String>),
        file: FileConfig,
    ) -> Result<Self, ConfigError> {
        let (env_db, env_api, env_poll) = env;

        let database_url = env_db
            .or(file.database_url)
            .ok_or(ConfigError::MissingVar("DATABASE_URL"))?;

        let api_base_url = env_api
            .or(file.api_base_url)
            .unwrap_or_else(|| Self::DEFAULT_API_URL.to_string());

        let poll_interval_secs = env_poll
            .and_then(|s| s.parse::<u64>().ok())
            .or(file.poll_interval)
            .unwrap_or(2);

        Ok(Self {
            database_url,
            api_base_url,
            poll_interval_secs,
        })
    }

    /// Pure parser — no env access, so unit tests can call it directly.
    /// Delegates to `from_sources` with an empty `FileConfig`.
    pub fn from_vars(
        database_url: Option<String>,
        api_base_url: Option<String>,
        poll_interval: Option<String>,
    ) -> Result<Self, ConfigError> {
        Self::from_sources(
            (database_url, api_base_url, poll_interval),
            FileConfig::default(),
        )
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ─── from_vars (backward-compat) ──────────────────────────────────────────

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

    // ─── from_sources: precedence ─────────────────────────────────────────────

    #[test]
    fn env_wins_over_file() {
        let file = FileConfig {
            database_url: Some("postgres://from-file/db".into()),
            api_base_url: Some("http://file:9000".into()),
            poll_interval: Some(10),
        };
        let c = Config::from_sources(
            (
                Some("postgres://from-env/db".into()),
                Some("http://env:8888".into()),
                Some("3".into()),
            ),
            file,
        )
        .expect("should parse");
        assert_eq!(c.database_url, "postgres://from-env/db");
        assert_eq!(c.api_base_url, "http://env:8888");
        assert_eq!(c.poll_interval_secs, 3);
    }

    #[test]
    fn file_fills_gap_env_omits() {
        let file = FileConfig {
            database_url: Some("postgres://file/db".into()),
            api_base_url: Some("http://file:7777".into()),
            poll_interval: Some(15),
        };
        let c = Config::from_sources((None, None, None), file).expect("should parse");
        assert_eq!(c.database_url, "postgres://file/db");
        assert_eq!(c.api_base_url, "http://file:7777");
        assert_eq!(c.poll_interval_secs, 15);
    }

    #[test]
    fn builtin_default_applies_when_both_omit_api_and_poll() {
        let file = FileConfig {
            database_url: Some("postgres://default-test/db".into()),
            api_base_url: None,
            poll_interval: None,
        };
        let c = Config::from_sources((None, None, None), file).expect("should parse");
        assert_eq!(c.api_base_url, "http://localhost:8080");
        assert_eq!(c.poll_interval_secs, 2);
    }

    #[test]
    fn database_url_satisfied_by_file_alone() {
        let file = FileConfig {
            database_url: Some("postgres://file-only/db".into()),
            api_base_url: None,
            poll_interval: None,
        };
        let c = Config::from_sources((None, None, None), file).expect("should parse");
        assert_eq!(c.database_url, "postgres://file-only/db");
    }

    #[test]
    fn missing_database_url_from_both_sources_is_error() {
        let err = Config::from_sources((None, None, None), FileConfig::default()).unwrap_err();
        assert_eq!(err, ConfigError::MissingVar("DATABASE_URL"));
    }

    // ─── parse_file ───────────────────────────────────────────────────────────

    #[test]
    fn parse_file_empty_string_returns_default() {
        let fc = parse_file("").expect("empty string should parse");
        assert_eq!(fc, FileConfig::default());
    }

    #[test]
    fn parse_file_whitespace_only_returns_default() {
        let fc = parse_file("   \n  ").expect("whitespace-only should parse");
        assert_eq!(fc, FileConfig::default());
    }

    #[test]
    fn parse_file_valid_toml() {
        let toml = r#"
database_url = "postgres://toml/db"
api_base_url = "http://toml:9999"
poll_interval = 7
"#;
        let fc = parse_file(toml).expect("valid TOML should parse");
        assert_eq!(fc.database_url.as_deref(), Some("postgres://toml/db"));
        assert_eq!(fc.api_base_url.as_deref(), Some("http://toml:9999"));
        assert_eq!(fc.poll_interval, Some(7));
    }

    #[test]
    fn parse_file_partial_toml() {
        let toml = r#"database_url = "postgres://partial/db""#;
        let fc = parse_file(toml).expect("partial TOML should parse");
        assert_eq!(fc.database_url.as_deref(), Some("postgres://partial/db"));
        assert!(fc.api_base_url.is_none());
        assert!(fc.poll_interval.is_none());
    }

    #[test]
    fn parse_file_unknown_keys_ignored() {
        let toml = r#"
database_url = "postgres://unknown-key/db"
unknown_future_key = "ignored"
"#;
        let fc = parse_file(toml).expect("unknown keys should be ignored");
        assert_eq!(
            fc.database_url.as_deref(),
            Some("postgres://unknown-key/db")
        );
    }

    #[test]
    fn parse_file_malformed_toml_returns_typed_error() {
        let bad_toml = "database_url = [not valid toml";
        let err = parse_file(bad_toml).unwrap_err();
        assert!(matches!(err, ConfigError::MalformedFile(_)));
    }

    // ─── config_path ──────────────────────────────────────────────────────────

    #[test]
    fn config_path_xdg_set() {
        let path = config_path(Some("/custom/xdg".into()), Some("/home/user".into()));
        assert_eq!(path, Some(PathBuf::from("/custom/xdg/bastion/config.toml")));
    }

    #[test]
    fn config_path_only_home_set() {
        let path = config_path(None, Some("/home/user".into()));
        assert_eq!(
            path,
            Some(PathBuf::from("/home/user/.config/bastion/config.toml"))
        );
    }

    #[test]
    fn config_path_neither_set() {
        let path = config_path(None, None);
        assert!(path.is_none());
    }

    #[test]
    fn config_path_xdg_takes_precedence_over_home() {
        let path = config_path(Some("/xdg".into()), Some("/home".into()));
        assert!(path.unwrap().starts_with("/xdg"));
    }
}
