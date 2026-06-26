use std::collections::HashMap;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum ConfigError {
    #[error("{0} must be set (point to the Python orchestrator's PostgreSQL)")]
    MissingVar(&'static str),
    #[error("config file is malformed: {0}")]
    MalformedFile(String),
    #[error("unknown workspace '{0}' — not found in [workspaces] registry")]
    UnknownWorkspace(String),
    #[error("no [workspaces] table in config — add [workspaces] to ~/.config/bastion/config.toml")]
    NoWorkspaceRegistry,
    #[error("BASTION_SERVE_TOKEN must be set — supply a bearer token via env or --token")]
    MissingServeToken,
}

// ── ServeConfig ───────────────────────────────────────────────────────────────

/// DB-free configuration for `bastion serve`.
///
/// Does NOT require `DATABASE_URL`. The token is mandatory — a missing token is
/// a typed [`ConfigError::MissingServeToken`], never a silent empty default.
#[derive(Debug, Clone, PartialEq)]
pub struct ServeConfig {
    /// Bind address (e.g. `"0.0.0.0:4317"`).
    pub addr: String,
    /// Bearer token that protected routes enforce.
    pub token: String,
}

impl ServeConfig {
    /// Default bind address — Tailscale-reachable, port 4317.
    const DEFAULT_ADDR: &'static str = "0.0.0.0:4317";
}

/// Build a [`ServeConfig`] by merging CLI flags (highest precedence) over env vars (middle)
/// over built-in defaults (lowest).
///
/// `addr_flag` and `token_flag` come from the CLI `--addr`/`--token` flags (may be `None`).
/// `addr_env` and `token_env` come from `BASTION_SERVE_ADDR` and `BASTION_SERVE_TOKEN`
/// respectively (may be `None` when not set).
///
/// **Pure function — no I/O, no env access.** Call from `load_serve_config` or tests directly.
///
/// # Errors
/// Returns [`ConfigError::MissingServeToken`] when neither flag nor env provides a token,
/// or when the resolved token is an empty string (e.g. `BASTION_SERVE_TOKEN=`).
pub fn build_serve_config(
    addr_flag: Option<String>,
    token_flag: Option<String>,
    addr_env: Option<String>,
    token_env: Option<String>,
) -> Result<ServeConfig, ConfigError> {
    let addr = addr_flag
        .or(addr_env)
        .unwrap_or_else(|| ServeConfig::DEFAULT_ADDR.to_string());

    let token = token_flag
        .or(token_env)
        .filter(|s| !s.is_empty())
        .ok_or(ConfigError::MissingServeToken)?;

    Ok(ServeConfig { addr, token })
}

/// Load [`ServeConfig`] from environment variables + `.env` file.
///
/// **DB-free** — does not read or require `DATABASE_URL`.
///
/// CLI flag values (from clap) should be passed in as `addr_flag` / `token_flag` and take
/// precedence over the env values read here.
///
/// # Errors
/// Returns [`ConfigError::MissingServeToken`] when neither `--token` nor `BASTION_SERVE_TOKEN`
/// is set.
pub fn load_serve_config(
    addr_flag: Option<String>,
    token_flag: Option<String>,
) -> Result<ServeConfig, ConfigError> {
    dotenvy::dotenv().ok();
    build_serve_config(
        addr_flag,
        token_flag,
        std::env::var("BASTION_SERVE_ADDR").ok(),
        std::env::var("BASTION_SERVE_TOKEN").ok(),
    )
}

/// Fields mirroring env vars — all optional; used as the fallback layer beneath env vars.
/// Unknown keys are silently ignored (no `deny_unknown_fields`).
///
/// The `[workspaces]` table maps short names to corpus root paths; `default_workspace`
/// names the entry used when neither `--root` nor `--workspace` is supplied.
#[derive(Debug, serde::Deserialize, Default, PartialEq)]
pub struct FileConfig {
    pub database_url: Option<String>,
    pub api_base_url: Option<String>,
    pub poll_interval: Option<u64>,
    /// Named workspace roots: `[workspaces]` TOML table → name → absolute path.
    pub workspaces: Option<HashMap<String, PathBuf>>,
    /// Default workspace name — used when `--workspace` is omitted.
    pub default_workspace: Option<String>,
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

/// Resolve the effective corpus root for `bastion brain`.
///
/// Precedence (highest → lowest):
/// 1. `explicit_root` — supplied via `--root <path>` (always wins).
/// 2. `workspace_name` — look up in `file.workspaces`; unknown name → typed error.
/// 3. `file.default_workspace` — resolve from registry; unknown name → typed error.
/// 4. Built-in default: `PathBuf::from(".")` (Block A behavior preserved).
///
/// Pure function — no I/O, no `DATABASE_URL` dependency.
pub fn resolve_workspace_root(
    explicit_root: Option<PathBuf>,
    workspace_name: Option<&str>,
    file: &FileConfig,
) -> Result<PathBuf, ConfigError> {
    // 1. Explicit --root wins.
    if let Some(root) = explicit_root {
        return Ok(root);
    }

    let registry = file.workspaces.as_ref();

    // 2. Named --workspace lookup.
    if let Some(name) = workspace_name {
        let Some(m) = registry else {
            return Err(ConfigError::NoWorkspaceRegistry);
        };
        return match m.get(name) {
            Some(path) => Ok(path.clone()),
            None => Err(ConfigError::UnknownWorkspace(name.to_string())),
        };
    }

    // 3. default_workspace from config.
    if let Some(ref default_name) = file.default_workspace {
        let Some(m) = registry else {
            return Err(ConfigError::NoWorkspaceRegistry);
        };
        return match m.get(default_name.as_str()) {
            Some(path) => Ok(path.clone()),
            None => Err(ConfigError::UnknownWorkspace(default_name.clone())),
        };
    }

    // 4. Built-in default.
    Ok(PathBuf::from("."))
}

/// Load **only** the workspace registry from the config file — DB-free.
///
/// Reads the config file identified by `config_path(xdg_config_home, home)`, parses it,
/// and returns the resulting `FileConfig` (which carries the workspace table).
///
/// Degradation contract:
/// - Config file absent or unreadable → returns `FileConfig::default()` (empty registry).
/// - Config file present but malformed → returns `ConfigError::MalformedFile`.
pub fn load_workspace_registry(
    xdg_config_home: Option<String>,
    home: Option<String>,
) -> Result<FileConfig, ConfigError> {
    match config_path(xdg_config_home, home) {
        Some(path) => match std::fs::read_to_string(&path) {
            Ok(contents) => parse_file(&contents),
            Err(_) => Ok(FileConfig::default()),
        },
        None => Ok(FileConfig::default()),
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

        // Absent or unreadable → silently degrade; malformed → propagate MalformedFile.
        let file_config = load_workspace_registry(
            std::env::var("XDG_CONFIG_HOME").ok(),
            std::env::var("HOME").ok(),
        )?;

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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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

    // ─── parse_file: [workspaces] table ──────────────────────────────────────

    #[test]
    fn parse_file_workspace_table_round_trips() {
        let toml = r#"
database_url = "postgres://ws/db"
default_workspace = "brain"

[workspaces]
brain = "/Users/alice/brain"
client-a = "/Users/alice/clients/a"
"#;
        let fc = parse_file(toml).expect("valid TOML with [workspaces] should parse");
        assert_eq!(fc.database_url.as_deref(), Some("postgres://ws/db"));
        assert_eq!(fc.default_workspace.as_deref(), Some("brain"));

        let ws = fc
            .workspaces
            .as_ref()
            .expect("[workspaces] should be present");
        assert_eq!(ws.get("brain"), Some(&PathBuf::from("/Users/alice/brain")));
        assert_eq!(
            ws.get("client-a"),
            Some(&PathBuf::from("/Users/alice/clients/a"))
        );
    }

    #[test]
    fn parse_file_missing_workspace_table_yields_none() {
        let toml = r#"database_url = "postgres://no-ws/db""#;
        let fc = parse_file(toml).expect("TOML without [workspaces] should parse");
        assert!(fc.workspaces.is_none());
        assert!(fc.default_workspace.is_none());
    }

    #[test]
    fn parse_file_empty_workspace_table_is_accepted() {
        let toml = "[workspaces]\n";
        let fc = parse_file(toml).expect("empty [workspaces] table should parse");
        // An empty TOML table deserialises to Some(empty map) or None depending on serde.
        // Either is acceptable — the resolver handles both.
        if let Some(ws) = &fc.workspaces {
            assert!(ws.is_empty());
        }
    }

    // ─── resolve_workspace_root ───────────────────────────────────────────────

    fn make_registry(entries: &[(&str, &str)]) -> FileConfig {
        let mut map = HashMap::new();
        for (name, path) in entries {
            map.insert(name.to_string(), PathBuf::from(path));
        }
        FileConfig {
            workspaces: Some(map),
            ..Default::default()
        }
    }

    #[test]
    fn resolve_explicit_root_wins_over_everything() {
        let mut fc = make_registry(&[("brain", "/registry/brain")]);
        fc.default_workspace = Some("brain".into());
        let result =
            resolve_workspace_root(Some(PathBuf::from("/explicit/root")), Some("brain"), &fc)
                .unwrap();
        assert_eq!(result, PathBuf::from("/explicit/root"));
    }

    #[test]
    fn resolve_named_workspace_hits_registry() {
        let fc = make_registry(&[("brain", "/repos/brain"), ("client-a", "/repos/client-a")]);
        let result = resolve_workspace_root(None, Some("client-a"), &fc).unwrap();
        assert_eq!(result, PathBuf::from("/repos/client-a"));
    }

    #[test]
    fn resolve_unknown_workspace_name_is_typed_error() {
        let fc = make_registry(&[("brain", "/repos/brain")]);
        let err = resolve_workspace_root(None, Some("missing"), &fc).unwrap_err();
        assert_eq!(err, ConfigError::UnknownWorkspace("missing".into()));
    }

    #[test]
    fn resolve_unknown_workspace_name_contains_the_name() {
        let fc = make_registry(&[]);
        let err = resolve_workspace_root(None, Some("ghost"), &fc).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("ghost"),
            "error message should include the unknown name"
        );
    }

    #[test]
    fn resolve_default_workspace_fallback() {
        let mut fc = make_registry(&[("brain", "/repos/brain")]);
        fc.default_workspace = Some("brain".into());
        let result = resolve_workspace_root(None, None, &fc).unwrap();
        assert_eq!(result, PathBuf::from("/repos/brain"));
    }

    #[test]
    fn resolve_default_workspace_unknown_is_typed_error() {
        let mut fc = make_registry(&[("brain", "/repos/brain")]);
        fc.default_workspace = Some("nonexistent".into());
        let err = resolve_workspace_root(None, None, &fc).unwrap_err();
        assert_eq!(err, ConfigError::UnknownWorkspace("nonexistent".into()));
    }

    #[test]
    fn resolve_no_config_returns_dot() {
        let fc = FileConfig::default();
        let result = resolve_workspace_root(None, None, &fc).unwrap();
        assert_eq!(result, PathBuf::from("."));
    }

    #[test]
    fn resolve_named_workspace_with_no_registry_is_no_registry_error() {
        // workspaces: None (no [workspaces] section) — distinct from an empty registry.
        let fc = FileConfig::default();
        let err = resolve_workspace_root(None, Some("brain"), &fc).unwrap_err();
        assert_eq!(err, ConfigError::NoWorkspaceRegistry);
    }

    #[test]
    fn resolve_default_workspace_with_no_registry_is_no_registry_error() {
        // default_workspace set but no [workspaces] table — should not say "not found in registry".
        let fc = FileConfig {
            default_workspace: Some("brain".into()),
            ..Default::default()
        };
        let err = resolve_workspace_root(None, None, &fc).unwrap_err();
        assert_eq!(err, ConfigError::NoWorkspaceRegistry);
    }

    #[test]
    fn resolve_registry_present_but_no_workspace_arg_and_no_default_returns_dot() {
        let fc = make_registry(&[("brain", "/repos/brain")]);
        // No --workspace, no default_workspace — fall through to built-in default.
        let result = resolve_workspace_root(None, None, &fc).unwrap();
        assert_eq!(result, PathBuf::from("."));
    }

    #[test]
    fn resolve_explicit_root_wins_even_with_no_registry() {
        let fc = FileConfig::default();
        let result = resolve_workspace_root(Some(PathBuf::from("/my/root")), None, &fc).unwrap();
        assert_eq!(result, PathBuf::from("/my/root"));
    }

    // ─── build_serve_config ───────────────────────────────────────────────────

    #[test]
    fn serve_config_flag_wins_over_env() {
        // CLI --addr and --token both override the env values.
        let sc = build_serve_config(
            Some("127.0.0.1:9000".into()),
            Some("flag-token".into()),
            Some("0.0.0.0:1111".into()),
            Some("env-token".into()),
        )
        .unwrap();
        assert_eq!(sc.addr, "127.0.0.1:9000");
        assert_eq!(sc.token, "flag-token");
    }

    #[test]
    fn serve_config_env_fills_gap_when_no_flags() {
        // Env values are used when CLI flags are absent.
        let sc = build_serve_config(
            None,
            None,
            Some("10.0.0.1:5000".into()),
            Some("env-secret".into()),
        )
        .unwrap();
        assert_eq!(sc.addr, "10.0.0.1:5000");
        assert_eq!(sc.token, "env-secret");
    }

    #[test]
    fn serve_config_default_addr_when_both_omit() {
        // Neither flag nor env provides addr → built-in default.
        let sc = build_serve_config(None, Some("tok".into()), None, None).unwrap();
        assert_eq!(sc.addr, "0.0.0.0:4317");
    }

    #[test]
    fn serve_config_flag_addr_with_env_token() {
        // Mixed: addr from flag, token from env.
        let sc = build_serve_config(
            Some("192.168.1.5:8080".into()),
            None,
            None,
            Some("env-tok".into()),
        )
        .unwrap();
        assert_eq!(sc.addr, "192.168.1.5:8080");
        assert_eq!(sc.token, "env-tok");
    }

    #[test]
    fn serve_config_missing_token_is_typed_error() {
        // Neither --token nor BASTION_SERVE_TOKEN → MissingServeToken.
        let err = build_serve_config(None, None, None, None).unwrap_err();
        assert_eq!(err, ConfigError::MissingServeToken);
    }

    #[test]
    fn serve_config_missing_token_error_message_is_descriptive() {
        let err = build_serve_config(None, None, None, None).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("BASTION_SERVE_TOKEN"),
            "error should mention the env var name; got: {msg}"
        );
    }

    #[test]
    fn serve_config_token_from_flag_alone_succeeds() {
        // Env is absent; CLI flag alone satisfies the mandatory token.
        let sc = build_serve_config(None, Some("only-flag-token".into()), None, None).unwrap();
        assert_eq!(sc.token, "only-flag-token");
        assert_eq!(sc.addr, "0.0.0.0:4317"); // default addr
    }

    #[test]
    fn serve_config_token_from_env_alone_succeeds() {
        // CLI flag absent; env alone satisfies the mandatory token.
        let sc = build_serve_config(None, None, None, Some("only-env-token".into())).unwrap();
        assert_eq!(sc.token, "only-env-token");
    }

    #[test]
    fn serve_config_empty_env_token_is_typed_error() {
        // BASTION_SERVE_TOKEN="" (set but empty) must be treated the same as absent.
        // An empty token would cause every protected request to return 401 with no
        // way to authenticate — the server must refuse to start.
        let err = build_serve_config(None, None, None, Some(String::new())).unwrap_err();
        assert_eq!(err, ConfigError::MissingServeToken);
    }

    #[test]
    fn serve_config_empty_flag_token_is_typed_error() {
        // --token "" (empty string from CLI) must also be rejected.
        let err = build_serve_config(None, Some(String::new()), None, None).unwrap_err();
        assert_eq!(err, ConfigError::MissingServeToken);
    }
}
