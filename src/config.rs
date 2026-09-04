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
    #[error("{0} is malformed: '{1}' is not a valid {2}")]
    MalformedBudgetValue(&'static str, String, &'static str),
    /// Exactly one of `BASTION_TELEGRAM_BOT_TOKEN` / `BASTION_TELEGRAM_CHAT_ID`
    /// is set — a half-configured Telegram transport is a typed config error,
    /// never a silent no-op. Names the *missing* var, never the one that was
    /// present (which may be the token).
    #[error(
        "{0} must also be set — Telegram transport needs both BOT_TOKEN and CHAT_ID or neither"
    )]
    IncompleteTelegramConfig(&'static str),
    /// The [`named_bot_config`] generalization of [`ConfigError::IncompleteTelegramConfig`]
    /// for an arbitrary `--bot <slug>`. Kept as a **sibling variant, not a
    /// widening of `IncompleteTelegramConfig`** — a per-slug var name is
    /// built at runtime, so it cannot be `&'static str`, but widening the
    /// existing variant would ripple past this task's file into every fixed
    /// (`telegram_config`, `code_sessions_bot_config`, `lane_bot_config`)
    /// construction site and their already-committed tests, plus
    /// `src/serve/handlers/notify.rs`'s match arm — well beyond
    /// `src/config.rs`. `lane_bot_config` still returns
    /// `IncompleteTelegramConfig` unchanged; it translates this variant back
    /// at its own boundary so 8a3ac96's tests pass unedited.
    #[error("{0} must also be set — bot transport needs both BOT_TOKEN and CHAT_ID or neither")]
    IncompleteNamedBotConfig(String),
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
#[derive(Debug, Clone, serde::Deserialize, Default, PartialEq)]
pub struct FileConfig {
    pub database_url: Option<String>,
    pub api_base_url: Option<String>,
    pub poll_interval: Option<u64>,
    /// Named workspace roots: `[workspaces]` TOML table → name → absolute path.
    pub workspaces: Option<HashMap<String, PathBuf>>,
    /// Default workspace name — used when `--workspace` is omitted.
    pub default_workspace: Option<String>,
    /// Optional `[theme]` section — selects a named UI theme preset (BA.14.0).
    /// Fully optional; absent entirely for existing configs, which parse unchanged.
    pub theme: Option<ThemeConfig>,
    /// Budget cap: total token ceiling across a run's node runs (BA.7.C).
    /// Absent by default — a run with no budget configured behaves exactly as it
    /// did before v1.1.0 of the data contract.
    pub max_total_tokens: Option<u64>,
    /// Budget cap: total USD-cost ceiling across a run's node runs (BA.7.C).
    /// Absent by default — same absent-tolerant contract as `max_total_tokens`.
    pub max_cost_usd: Option<f64>,
    /// `X-API-Key` secret for the engine's abort endpoint (BA.7.C). Distinct from
    /// `ServeConfig.token` (bastion serve's own `Authorization: Bearer` gate) —
    /// two different secrets, two different schemes, two different route groups.
    pub engine_api_key: Option<String>,
    /// The `[telegram_commands]` allow-list table (BA.ticket.telegram-command-router),
    /// keyed by command NAME with no leading `/`. Absent entirely for existing
    /// configs, which parse unchanged. Read once at `bastion serve` boot — adding
    /// a command is a config edit plus a restart, not a hot reload.
    pub telegram_commands: Option<HashMap<String, TelegramCommandEntry>>,
}

/// The `[theme]` TOML table.
///
/// ```toml
/// [theme]
/// name = "bastion"
/// ```
#[derive(Debug, Clone, serde::Deserialize, Default, PartialEq)]
pub struct ThemeConfig {
    /// Theme preset name — resolved via `ui_theme::theme_by_name`. Any value not
    /// recognized as a known preset falls back to the `bastion` default there.
    pub name: Option<String>,
}

// ── Telegram command router allow-list ─────────────────────────────────────

/// Where a [`TelegramCommandParam`] pulls its value from, out of the message
/// text that followed the command name.
///
/// Survey-driven (`planning/BA.ticket.telegram-command-router/workflow-payload-survey.md`):
/// every triggerable workflow's event is a flat JSON object, so the router's whole
/// job is filling one flat object from these four sources plus the fixed `data`
/// base.
#[derive(Debug, Clone, Copy, serde::Deserialize, PartialEq, Eq)]
pub enum TelegramParamSource {
    /// The whole remainder of the message as one JSON string — a company name,
    /// a URL, a paragraph of notes. Chosen so multi-word arguments survive.
    #[serde(rename = "rest")]
    Rest,
    /// Every whitespace-split token as a JSON array of strings — a shopping list.
    #[serde(rename = "args")]
    Args,
    /// The Nth whitespace-split token (see `index`) as a JSON string —
    /// positional, for multi-field commands like `LINKEDIN_POST`'s `since`/`until`.
    #[serde(rename = "arg")]
    Arg,
    /// Builds an `IngressEnvelope` (see [`TelegramSourceKind`]) and places it at `key`.
    #[serde(rename = "envelope")]
    Envelope,
}

/// Which `SourcePayload` variant an `envelope`-sourced param carries.
///
/// `CONTENT_PIPELINE`'s `SourceRouterNode::route` branches purely on this
/// variant (`Url` → `FetchArticleNode`, `VideoId` → `FetchTranscriptNode`) and
/// never inspects the URL's host — so an article command and a YouTube command
/// are the same config shape, distinguished only by this field.
#[derive(Debug, Clone, Copy, serde::Deserialize, PartialEq, Eq)]
pub enum TelegramSourceKind {
    /// A plain URL — routes to `FetchArticleNode`.
    #[serde(rename = "url")]
    Url,
    /// A YouTube video id, extracted from a pasted `youtube.com/watch?v=`,
    /// `youtu.be/`, `/shorts/` or `/embed/` link — routes to `FetchTranscriptNode`.
    #[serde(rename = "video_id")]
    VideoId,
    /// Plain text.
    #[serde(rename = "text")]
    Text,
}

/// Returns `true` — the default for [`TelegramCommandParam::required`].
fn default_true() -> bool {
    true
}

/// One parameter of a [`TelegramCommandEntry`]: which part of the message fills
/// which field of the dispatched workflow's event payload.
///
/// Applied, in list order, over the entry's fixed `data` base — a param wins on
/// key collision. `required` defaults to `true`; a required param with nothing
/// to fill it refuses the dispatch with a usage reply rather than guessing a
/// default or panicking.
#[derive(Debug, Clone, serde::Deserialize, PartialEq)]
pub struct TelegramCommandParam {
    /// The key this param fills in the dispatched workflow's flat event object.
    pub key: String,
    /// Where the value comes from.
    pub from: TelegramParamSource,
    /// Positional index into the whitespace-split argument tokens — required
    /// when `from = "arg"`, ignored otherwise.
    #[serde(default)]
    pub index: Option<usize>,
    /// Which `SourcePayload` variant to build — required when `from = "envelope"`,
    /// ignored otherwise.
    #[serde(default)]
    pub source_kind: Option<TelegramSourceKind>,
    /// Whether a missing value refuses the dispatch (`true`, the default) or is
    /// tolerated as absent.
    #[serde(default = "default_true")]
    pub required: bool,
}

/// One `[telegram_commands.<name>]` allow-list entry — a command name to the
/// workflow it dispatches, with no code change required to add another.
///
/// `chat_id` pinning happens at dispatch time, outside this config shape; this
/// struct only says what a recognized command does once authorised.
#[derive(Debug, Clone, serde::Deserialize, PartialEq)]
pub struct TelegramCommandEntry {
    /// The `workflow_type` string dispatched through the in-process `/events/`
    /// route (e.g. `"RESEARCH_AGENT"`, `"CONTENT_PIPELINE"`).
    pub workflow_type: String,
    /// Ordered list of parameters filling the dispatched event's payload from
    /// the message text. Empty by default — a command with no arguments (e.g.
    /// a fixed-`data`-only trigger) omits `params` entirely.
    #[serde(default)]
    pub params: Vec<TelegramCommandParam>,
    /// Fixed base object merged under the params — covers tuning knobs like
    /// `policy`/`profile`/`locale`, and required-but-not-argument fields like
    /// `RESEARCH_AGENT`'s `mode`. `None` when a command needs no fixed fields.
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

/// Resolve the active `ui_theme::Theme` from a parsed `FileConfig`.
///
/// - `[theme]` section absent → default (`ui_theme::theme_by_name("")`, i.e. `bastion`).
/// - `[theme].name` absent → same default.
/// - `[theme].name` present → looked up via `ui_theme::theme_by_name`, which itself
///   falls back to the `bastion` default for an unknown name. Never panics.
pub fn resolve_theme(file: &FileConfig) -> crate::ui_theme::Theme {
    let name = file
        .theme
        .as_ref()
        .and_then(|t| t.name.as_deref())
        .unwrap_or("");
    crate::ui_theme::theme_by_name(name)
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
    /// Budget cap: total token ceiling (BA.7.C). Absent-tolerant — `None` means
    /// no gate/alert applies and behavior is unchanged from before v1.1.0.
    pub max_total_tokens: Option<u64>,
    /// Budget cap: total USD-cost ceiling (BA.7.C). Same absent-tolerant contract.
    pub max_cost_usd: Option<f64>,
    /// `X-API-Key` secret for the engine's abort endpoint (BA.7.C). Distinct from
    /// `ServeConfig.token`; used by both `api::client` (sender) and the embedded
    /// engine's `AppState.api_key` (verifier).
    pub engine_api_key: Option<String>,
    /// Desktop-notification toggle (Part B) — `BASTION_NOTIFY`. Opt-out, not
    /// opt-in: defaults to `true` since the notification feature only exists
    /// because it was asked for. Checked at the `monitor::events` /
    /// `monitor::watch` call sites, not inside `notify::send_macos_notification`
    /// itself (which stays a pure, config-agnostic shell over `osascript`).
    pub notify_enabled: bool,
}

/// Default FastAPI base URL — orchestrator `/health` lives on port 8080
/// (recon 2026-06-18; the scaffold's old 8000 default was wrong).
///
/// Module-scoped (rather than an associated const) so [`resolve_api_base_url`] can be a
/// standalone pure function callable without a fully-constructed [`Config`].
pub(crate) const DEFAULT_API_URL: &str = "http://localhost:8080";

/// Resolve the API base URL from env (highest precedence), then the workspace registry file,
/// then [`DEFAULT_API_URL`]. Pure and infallible — no I/O, no `Result`, no panic path — so a
/// caller (e.g. `run::status()`) can resolve this independently of whether the rest of
/// [`Config::from_sources`] succeeded (BA.ticket.status-config-error-coupling).
pub(crate) fn resolve_api_base_url(
    env_api: Option<String>,
    file_api_base_url: Option<String>,
) -> String {
    env_api
        .or(file_api_base_url)
        .unwrap_or_else(|| DEFAULT_API_URL.to_string())
}

impl Config {
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
                std::env::var("BASTION_MAX_TOTAL_TOKENS").ok(),
                std::env::var("BASTION_MAX_COST_USD").ok(),
                std::env::var("BASTION_ENGINE_API_KEY").ok(),
                std::env::var("BASTION_NOTIFY").ok(),
            ),
            file_config,
        )
    }

    /// Merge env vars (highest precedence) with file config (middle) and built-in defaults
    /// (lowest). `DATABASE_URL` must be satisfied by at least one source.
    ///
    /// `env` is `(DATABASE_URL, BASTION_API_URL, BASTION_POLL_INTERVAL,
    /// BASTION_MAX_TOTAL_TOKENS, BASTION_MAX_COST_USD, BASTION_ENGINE_API_KEY,
    /// BASTION_NOTIFY)`.
    ///
    /// The three budget/key fields are absent-tolerant: `None` from both env and file is a
    /// valid, unchanged configuration (no gate, no alert). A present-but-unparseable
    /// `BASTION_MAX_TOTAL_TOKENS` or `BASTION_MAX_COST_USD` is a typed
    /// [`ConfigError::MalformedBudgetValue`], never a silent default — a value that fails to
    /// parse must not be treated the same as "no cap configured".
    ///
    /// `BASTION_NOTIFY` follows the same lenient-parse convention as
    /// `BASTION_POLL_INTERVAL`: an unparseable value silently falls back to the
    /// default (`true`, opt-out) rather than erroring — unlike the budget
    /// values, a bad notify toggle isn't a "must fail loudly" config mistake.
    #[allow(clippy::type_complexity)]
    pub fn from_sources(
        env: (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
        file: FileConfig,
    ) -> Result<Self, ConfigError> {
        let (env_db, env_api, env_poll, env_max_tokens, env_max_cost, env_engine_key, env_notify) =
            env;

        let database_url = env_db
            .or(file.database_url)
            .ok_or(ConfigError::MissingVar("DATABASE_URL"))?;

        let api_base_url = resolve_api_base_url(env_api, file.api_base_url);

        let poll_interval_secs = env_poll
            .and_then(|s| s.parse::<u64>().ok())
            .or(file.poll_interval)
            .unwrap_or(2);

        let max_total_tokens = match env_max_tokens {
            Some(s) => Some(s.parse::<u64>().map_err(|_| {
                ConfigError::MalformedBudgetValue(
                    "BASTION_MAX_TOTAL_TOKENS",
                    s.clone(),
                    "u64 token count",
                )
            })?),
            None => file.max_total_tokens,
        };

        let max_cost_usd = match env_max_cost {
            Some(s) => Some(s.parse::<f64>().map_err(|_| {
                ConfigError::MalformedBudgetValue(
                    "BASTION_MAX_COST_USD",
                    s.clone(),
                    "f64 USD amount",
                )
            })?),
            None => file.max_cost_usd,
        };

        let engine_api_key = env_engine_key.or(file.engine_api_key);

        let notify_enabled = env_notify
            .and_then(|s| s.parse::<bool>().ok())
            .unwrap_or(true);

        Ok(Self {
            database_url,
            api_base_url,
            poll_interval_secs,
            max_total_tokens,
            max_cost_usd,
            engine_api_key,
            notify_enabled,
        })
    }

    /// Pure parser — no env access, so unit tests can call it directly.
    /// Delegates to `from_sources` with an empty `FileConfig` and no budget/key/notify values.
    pub fn from_vars(
        database_url: Option<String>,
        api_base_url: Option<String>,
        poll_interval: Option<String>,
    ) -> Result<Self, ConfigError> {
        Self::from_sources(
            (
                database_url,
                api_base_url,
                poll_interval,
                None,
                None,
                None,
                None,
            ),
            FileConfig::default(),
        )
    }
}

// ── TelegramConfig (BA.18.B task 4) ─────────────────────────────────────────

/// A Telegram bot token. Wraps the raw `String` so the value can never be
/// printed by accident: `Debug` is hand-written to always render
/// `BotToken(<redacted>)`, never the token itself (`CLAUDE.md` non-negotiable
/// constraint 3 — the token must never appear in a log line or error
/// message).
#[derive(Clone, PartialEq, Eq)]
pub struct BotToken(String);

impl BotToken {
    #[must_use]
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    /// The raw token value, for use only at the point a request is actually
    /// built (e.g. interpolated into the Telegram API URL path). Callers
    /// must never log or `Debug`-print the returned `&str`.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for BotToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BotToken(<redacted>)")
    }
}

/// Resolved Telegram transport config: present only when both
/// `BASTION_TELEGRAM_BOT_TOKEN` and `BASTION_TELEGRAM_CHAT_ID` are set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramConfig {
    pub bot_token: BotToken,
    pub chat_id: String,
}

/// Resolve the optional Telegram transport config from the two env values.
///
/// - Both absent → `Ok(None)` — the transport is simply not configured, and
///   `run_server` starts exactly as it does today.
/// - Both present → `Ok(Some(TelegramConfig { .. }))`.
/// - Exactly one present → `Err(ConfigError::IncompleteTelegramConfig(missing))`,
///   naming the missing var — a half-configured transport must fail loudly,
///   never silently behave as "unconfigured".
/// - A present-but-empty-string value counts as absent, matching
///   `build_serve_config`'s treatment of an empty `BASTION_SERVE_TOKEN`.
///
/// Pure function — no I/O, no env access. Call from `load_telegram_config`
/// or tests directly.
pub fn telegram_config(
    bot_token_env: Option<String>,
    chat_id_env: Option<String>,
) -> Result<Option<TelegramConfig>, ConfigError> {
    let bot_token = bot_token_env.filter(|s| !s.is_empty());
    let chat_id = chat_id_env.filter(|s| !s.is_empty());

    match (bot_token, chat_id) {
        (None, None) => Ok(None),
        (Some(token), Some(chat_id)) => Ok(Some(TelegramConfig {
            bot_token: BotToken::new(token),
            chat_id,
        })),
        (Some(_), None) => Err(ConfigError::IncompleteTelegramConfig(
            "BASTION_TELEGRAM_CHAT_ID",
        )),
        (None, Some(_)) => Err(ConfigError::IncompleteTelegramConfig(
            "BASTION_TELEGRAM_BOT_TOKEN",
        )),
    }
}

/// Load [`TelegramConfig`] from `BASTION_TELEGRAM_BOT_TOKEN` /
/// `BASTION_TELEGRAM_CHAT_ID` + `.env` file. DB-free.
pub fn load_telegram_config() -> Result<Option<TelegramConfig>, ConfigError> {
    dotenvy::dotenv().ok();
    telegram_config(
        std::env::var("BASTION_TELEGRAM_BOT_TOKEN").ok(),
        std::env::var("BASTION_TELEGRAM_CHAT_ID").ok(),
    )
}

// ── CodeSessionsBotConfig (BA.20.C task 2) ──────────────────────────────────

/// Resolved CodeSessionsBot config: present only when both
/// `BASTION_CODESESSIONS_BOT_TOKEN` and `BASTION_CODESESSIONS_CHAT_ID` are
/// set.
///
/// **Deliberately distinct from [`TelegramConfig`]** — that pair configures
/// BastionBot's approve/reject gate transport; this pair configures
/// CodeSessionsBot, the session-QA bridge's bot (shared with the HQ chore's
/// `claude_session_notify.sh`). Two bots, two token pairs, never conflated.
/// CodeSessionsBot does not exist yet as of BA.20.C, so unset is the expected
/// state today — absence means the bridge is disabled, not an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeSessionsBotConfig {
    pub bot_token: BotToken,
    pub chat_id: String,
}

/// Resolve the optional CodeSessionsBot config from the two env values.
///
/// Mirrors [`telegram_config`]'s both-or-neither rule exactly (same
/// semantics, same typed error, different env var names): both absent is
/// `Ok(None)` (bridge disabled, not an error); both present resolves; exactly
/// one present is `Err(ConfigError::IncompleteTelegramConfig(missing))`
/// naming the missing var; a present-but-empty-string value counts as
/// absent.
///
/// Pure function — no I/O, no env access. Call from
/// `load_code_sessions_bot_config` or tests directly.
pub fn code_sessions_bot_config(
    bot_token_env: Option<String>,
    chat_id_env: Option<String>,
) -> Result<Option<CodeSessionsBotConfig>, ConfigError> {
    let bot_token = bot_token_env.filter(|s| !s.is_empty());
    let chat_id = chat_id_env.filter(|s| !s.is_empty());

    match (bot_token, chat_id) {
        (None, None) => Ok(None),
        (Some(token), Some(chat_id)) => Ok(Some(CodeSessionsBotConfig {
            bot_token: BotToken::new(token),
            chat_id,
        })),
        (Some(_), None) => Err(ConfigError::IncompleteTelegramConfig(
            "BASTION_CODESESSIONS_CHAT_ID",
        )),
        (None, Some(_)) => Err(ConfigError::IncompleteTelegramConfig(
            "BASTION_CODESESSIONS_BOT_TOKEN",
        )),
    }
}

/// Load [`CodeSessionsBotConfig`] from `BASTION_CODESESSIONS_BOT_TOKEN` /
/// `BASTION_CODESESSIONS_CHAT_ID` + `.env` file. DB-free.
pub fn load_code_sessions_bot_config() -> Result<Option<CodeSessionsBotConfig>, ConfigError> {
    dotenvy::dotenv().ok();
    code_sessions_bot_config(
        std::env::var("BASTION_CODESESSIONS_BOT_TOKEN").ok(),
        std::env::var("BASTION_CODESESSIONS_CHAT_ID").ok(),
    )
}

// ── PricescoutBotConfig (BA.ticket.pricescout-telegram-bot task 1) ─────────
//
// A THIRD inbound loop, on a dedicated `pricescout` token, for the family's
// `/shop` command — not a reuse of the `telegram` or `codesessions` tokens.
// Absence disables the loop, not an error (mirrors `CodeSessionsBotConfig`,
// not `LaneBotConfig`'s hard-error CLI contract): `bastion serve` must boot
// exactly as today when this pair is unset.
//
// Defined below [`named_bot_config`]/[`load_named_bot_config`] (which this
// is a thin alias over) rather than beside [`CodeSessionsBotConfig`] purely
// because the generic path it reuses is defined later in this file; the
// doc comment on `load_code_sessions_bot_config` above is what "beside"
// refers to.

/// Load [`BotCredentials`] for the `pricescout` bot from
/// `BASTION_PRICESCOUT_BOT_TOKEN` / `BASTION_PRICESCOUT_CHAT_ID` + `.env`
/// file. DB-free.
///
/// **Thin alias over [`load_named_bot_config`]** — unlike [`LaneBotConfig`],
/// the family's bot needs no distinct struct shape (nothing downstream
/// depends on a bespoke `PricescoutBotConfig` type), so this loader is a
/// direct pass-through to the generic per-slug path rather than a wrapper
/// that translates the error back to a bespoke shape. Both-absent is
/// `Ok(None)`; both-present resolves; exactly one present is
/// `Err(ConfigError::IncompleteNamedBotConfig(missing))` naming the missing
/// var.
pub fn load_pricescout_bot_config() -> Result<Option<BotCredentials>, ConfigError> {
    load_named_bot_config("pricescout")
}

// ── LaneBotConfig (BA.ticket.notify-operator-cli task 1) ───────────────────

/// Resolved LaneBot config: present only when both `BASTION_LANE_BOT_TOKEN`
/// and `BASTION_LANE_CHAT_ID` are set.
///
/// **A third bot, not a reuse of either existing pair.** `bastion serve`
/// already runs one `getUpdates` long-poll per bot token —
/// `NotifyPollLoop::run` for BastionBot and `SessionQaBridge::run_outbound`
/// for CodeSessionsBot. Telegram delivers each update to exactly ONE
/// `getUpdates` consumer per bot token, so a CLI polling either of those
/// tokens would steal the taps those loops exist to receive. LaneBot gives
/// `bastion notify` a stream nothing else consumes. Unlike CodeSessionsBot
/// (where absence disables a background bridge, not an error), absence of
/// this pair IS a hard error for `bastion notify send`/`ask` — those verbs
/// are invoked deliberately and must never silently no-op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneBotConfig {
    pub bot_token: BotToken,
    pub chat_id: String,
}

/// Resolve the optional LaneBot config from the two env values.
///
/// Mirrors [`code_sessions_bot_config`]'s both-or-neither rule exactly (same
/// semantics, same typed error, different env var names): both absent is
/// `Ok(None)`; both present resolves; exactly one present is
/// `Err(ConfigError::IncompleteTelegramConfig(missing))` naming the missing
/// var; a present-but-empty-string value counts as absent.
///
/// Pure function — no I/O, no env access. Call from `load_lane_bot_config`
/// or tests directly.
///
/// **Thin alias over [`named_bot_config`]** (BA.ticket.notify-operator-cli
/// task 1's generalization) — `lane_bot_config("lane", ..)` in every way
/// except that its `Err` is translated back to the pre-existing
/// `IncompleteTelegramConfig(&'static str)` shape so this function's
/// original signature and 8a3ac96's already-committed tests keep working
/// byte-for-byte unedited. The truth table itself lives in
/// `named_bot_config` — this function does not re-derive it.
pub fn lane_bot_config(
    bot_token_env: Option<String>,
    chat_id_env: Option<String>,
) -> Result<Option<LaneBotConfig>, ConfigError> {
    match named_bot_config("lane", bot_token_env, chat_id_env) {
        Ok(None) => Ok(None),
        Ok(Some(creds)) => Ok(Some(LaneBotConfig {
            bot_token: creds.bot_token,
            chat_id: creds.chat_id,
        })),
        Err(ConfigError::IncompleteNamedBotConfig(missing)) => {
            let static_name = if missing == "BASTION_LANE_BOT_TOKEN" {
                "BASTION_LANE_BOT_TOKEN"
            } else {
                "BASTION_LANE_CHAT_ID"
            };
            Err(ConfigError::IncompleteTelegramConfig(static_name))
        }
        Err(other) => Err(other),
    }
}

/// Load [`LaneBotConfig`] from `BASTION_LANE_BOT_TOKEN` /
/// `BASTION_LANE_CHAT_ID` + `.env` file. DB-free.
pub fn load_lane_bot_config() -> Result<Option<LaneBotConfig>, ConfigError> {
    dotenvy::dotenv().ok();
    lane_bot_config(
        std::env::var("BASTION_LANE_BOT_TOKEN").ok(),
        std::env::var("BASTION_LANE_CHAT_ID").ok(),
    )
}

// ── named_bot_config (BA.ticket.notify-operator-cli task 1) ────────────────
//
// `--bot <slug>` exists so the CLI infrastructure introduced by
// `bastion notify` is shared across bot credential pairs instead of
// hardcoded to one. Every bot in this repo already follows one env-name
// pattern — `BASTION_TELEGRAM_*` (BastionBot), `BASTION_CODESESSIONS_*`
// (CodeSessionsBot), `BASTION_LANE_*` (LaneBot) — so a fourth bot needs only
// a new env pair, never a code change. The third-bot/steal-hazard rationale
// documented on [`LaneBotConfig`] above still holds and is still invisible
// from this code: Telegram delivers each update to exactly one `getUpdates`
// consumer per bot token, so distinct slugs exist to give each concurrent
// poller (BastionBot's `NotifyPollLoop`, CodeSessionsBot's
// `SessionQaBridge`, and now an arbitrary `--bot` target) its own stream.

/// Resolved credentials for an arbitrary named bot slug.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotCredentials {
    /// The slug this pair was resolved for (e.g. `"lane"`, `"codesessions"`).
    pub slug: String,
    pub bot_token: BotToken,
    pub chat_id: String,
}

/// Derive the two env var names a bot slug's credentials live under:
/// `BASTION_<SLUG_UPPER>_BOT_TOKEN` / `BASTION_<SLUG_UPPER>_CHAT_ID`.
///
/// Pure — no I/O. `slug` is upper-cased verbatim (a hyphen in the slug
/// produces a hyphen in the derived name; this function only performs the
/// substitution, it does not validate that the result is a shell-legal env
/// var name).
pub fn bot_env_var_names(slug: &str) -> (String, String) {
    let upper = slug.to_uppercase();
    (
        format!("BASTION_{upper}_BOT_TOKEN"),
        format!("BASTION_{upper}_CHAT_ID"),
    )
}

/// Resolve the optional bot config for an arbitrary `slug` from the two env
/// values already read for it.
///
/// Carries the identical both-or-neither truth table as [`lane_bot_config`]
/// / [`code_sessions_bot_config`] / [`telegram_config`], generalized over
/// the slug: both absent → `Ok(None)`; both present → resolved; exactly one
/// present → `Err(ConfigError::IncompleteNamedBotConfig(missing))` naming
/// the missing var *for this slug* (built at runtime, per-slug — this is
/// why the error is a sibling variant rather than a widened
/// `IncompleteTelegramConfig`, see that variant's doc comment); a
/// present-but-empty-string value counts as absent.
///
/// Pure function — no I/O, no env access. Call from
/// `load_named_bot_config` or tests directly.
pub fn named_bot_config(
    slug: &str,
    bot_token_env: Option<String>,
    chat_id_env: Option<String>,
) -> Result<Option<BotCredentials>, ConfigError> {
    let bot_token = bot_token_env.filter(|s| !s.is_empty());
    let chat_id = chat_id_env.filter(|s| !s.is_empty());
    let (token_var, chat_var) = bot_env_var_names(slug);

    match (bot_token, chat_id) {
        (None, None) => Ok(None),
        (Some(token), Some(chat_id)) => Ok(Some(BotCredentials {
            slug: slug.to_string(),
            bot_token: BotToken::new(token),
            chat_id,
        })),
        (Some(_), None) => Err(ConfigError::IncompleteNamedBotConfig(chat_var)),
        (None, Some(_)) => Err(ConfigError::IncompleteNamedBotConfig(token_var)),
    }
}

/// Load [`BotCredentials`] for `slug` from its two derived env vars +
/// `.env` file. DB-free. Mirrors [`load_code_sessions_bot_config`] /
/// [`load_lane_bot_config`]'s shape.
pub fn load_named_bot_config(slug: &str) -> Result<Option<BotCredentials>, ConfigError> {
    dotenvy::dotenv().ok();
    let (token_var, chat_var) = bot_env_var_names(slug);
    named_bot_config(
        slug,
        std::env::var(&token_var).ok(),
        std::env::var(&chat_var).ok(),
    )
}

/// The bot slugs this repo knows the env-var pattern for. Adding a bot to
/// this list is the only code change a new bot ever needs beyond its env
/// pair — everything else in `named_bot_config` is already generic.
pub const KNOWN_BOT_SLUGS: &[&str] = &["telegram", "codesessions", "lane", "pricescout"];

/// Which of [`KNOWN_BOT_SLUGS`] have a COMPLETE credential pair present in
/// `env_snapshot`, in `KNOWN_BOT_SLUGS` order.
///
/// Takes the environment as an argument (rather than reading
/// `std::env::var` itself) so it is testable without touching real process
/// env. Feeds the unknown-slug error message that names "the slugs that DO
/// have a complete pair" (BA.ticket.notify-operator-cli task 5).
pub fn configured_bot_slugs(env_snapshot: &HashMap<String, String>) -> Vec<String> {
    KNOWN_BOT_SLUGS
        .iter()
        .filter(|slug| {
            let (token_var, chat_var) = bot_env_var_names(slug);
            let token = env_snapshot.get(&token_var).map(String::as_str);
            let chat = env_snapshot.get(&chat_var).map(String::as_str);
            token.is_some_and(|s| !s.is_empty()) && chat.is_some_and(|s| !s.is_empty())
        })
        .map(|slug| slug.to_string())
        .collect()
}

/// Walk from `start` upward toward the filesystem root, returning the first
/// existing `dir.join(target)` encountered, or `None` if never found.
///
/// Pure function — no environment or process-global access; the caller supplies
/// `start` explicitly. Used both by `walk_up_for` (cwd-anchored) and by `assess`
/// (path-anchored, per BA.15.9).
pub fn walk_up_from(start: &std::path::Path, target: &str) -> Option<PathBuf> {
    let mut curr = start;
    loop {
        let candidate = curr.join(target);
        if candidate.exists() {
            return Some(candidate);
        }
        curr = curr.parent()?;
    }
}

/// Helper: Walk up from the current directory looking for a target file/dir.
/// Returns the absolute path if found, or `PathBuf::from(target)` if it hits the root without finding it.
fn walk_up_for(target: &str) -> PathBuf {
    if let Ok(cwd) = std::env::current_dir()
        && let Some(found) = walk_up_from(&cwd, target)
    {
        return found;
    }
    PathBuf::from(target)
}

// ── Planning root ─────────────────────────────────────────────────────────────

/// Resolve the `planning/` directory root.
///
/// Precedence:
/// 1. `env_val` — value of `BASTION_PLANNING_ROOT` env var (if set and non-empty).
/// 2. Built-in default: `PathBuf::from("planning")` (relative to cwd).
///
/// Pure function — no I/O, no env access. Call from `load_planning_root()` or tests directly.
pub fn planning_root(env_val: Option<String>) -> PathBuf {
    env_val
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| walk_up_for("planning"))
}

/// Load the planning root from `BASTION_PLANNING_ROOT` env var + `.env` file.
///
/// **DB-free** — does not read or require `DATABASE_URL`.
pub fn load_planning_root() -> PathBuf {
    dotenvy::dotenv().ok();
    planning_root(std::env::var("BASTION_PLANNING_ROOT").ok())
}

/// Resolve the `brain.toml` file path.
///
/// Precedence:
/// 1. `env_val` — value of `BASTION_BRAIN_TOML` env var (if set and non-empty).
/// 2. Built-in default: `PathBuf::from("brain.toml")` (relative to cwd).
pub fn brain_toml_path(env_val: Option<String>) -> PathBuf {
    env_val
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| walk_up_for("brain.toml"))
}

/// Load the brain.toml path from `BASTION_BRAIN_TOML` env var + `.env` file.
pub fn load_brain_toml_path() -> PathBuf {
    dotenvy::dotenv().ok();
    brain_toml_path(std::env::var("BASTION_BRAIN_TOML").ok())
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

    // ─── resolve_api_base_url: pure precedence (no Config::load, no I/O) ──────

    #[test]
    fn resolve_api_base_url_env_wins_over_file() {
        let resolved = resolve_api_base_url(
            Some("http://env:8888".into()),
            Some("http://file:9000".into()),
        );
        assert_eq!(resolved, "http://env:8888");
    }

    #[test]
    fn resolve_api_base_url_file_wins_when_env_absent() {
        let resolved = resolve_api_base_url(None, Some("http://file:7777".into()));
        assert_eq!(resolved, "http://file:7777");
    }

    #[test]
    fn resolve_api_base_url_default_when_both_absent() {
        let resolved = resolve_api_base_url(None, None);
        assert_eq!(resolved, "http://localhost:8080");
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
                None,
                None,
                None,
                None,
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
        let c = Config::from_sources((None, None, None, None, None, None, None), file)
            .expect("should parse");
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
        let c = Config::from_sources((None, None, None, None, None, None, None), file)
            .expect("should parse");
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
        let c = Config::from_sources((None, None, None, None, None, None, None), file)
            .expect("should parse");
        assert_eq!(c.database_url, "postgres://file-only/db");
    }

    #[test]
    fn missing_database_url_from_both_sources_is_error() {
        let err = Config::from_sources(
            (None, None, None, None, None, None, None),
            FileConfig::default(),
        )
        .unwrap_err();
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

    // ─── parse_file: [telegram_commands] table ───────────────────────────────

    const TELEGRAM_COMMANDS_TOML: &str = r#"
[telegram_commands.research]
workflow_type = "RESEARCH_AGENT"
data = { mode = "company", profile = "thorough" }
params = [{ key = "company_name", from = "rest" }]

[telegram_commands.intake]
workflow_type = "DIAGNOSTIC_INTAKE"
params = [{ key = "notes", from = "rest" }]

[telegram_commands.article]
workflow_type = "CONTENT_PIPELINE"
params = [{ key = "envelope", from = "envelope", source_kind = "url" }]

[telegram_commands.yt]
workflow_type = "CONTENT_PIPELINE"
params = [{ key = "envelope", from = "envelope", source_kind = "video_id" }]

[telegram_commands.linkedin]
workflow_type = "LINKEDIN_POST"
params = [
  { key = "since", from = "arg", index = 0 },
  { key = "until", from = "arg", index = 1 },
]

[telegram_commands.shop]
workflow_type = "PRICE_SCOUT"
data = { region = "BR" }
params = [{ key = "items", from = "args" }]
"#;

    #[test]
    fn parse_file_telegram_commands_six_entry_example() {
        let fc = parse_file(TELEGRAM_COMMANDS_TOML).expect("valid telegram_commands should parse");
        let table = fc
            .telegram_commands
            .as_ref()
            .expect("[telegram_commands] should be present");
        assert_eq!(table.len(), 6);

        let research = &table["research"];
        assert_eq!(research.workflow_type, "RESEARCH_AGENT");
        assert_eq!(
            research.data,
            Some(serde_json::json!({ "mode": "company", "profile": "thorough" }))
        );
        assert_eq!(research.params.len(), 1);
        assert_eq!(research.params[0].key, "company_name");
        assert_eq!(research.params[0].from, TelegramParamSource::Rest);

        let linkedin = &table["linkedin"];
        assert_eq!(linkedin.workflow_type, "LINKEDIN_POST");
        assert_eq!(linkedin.params.len(), 2);
        assert_eq!(linkedin.params[0].key, "since");
        assert_eq!(linkedin.params[0].from, TelegramParamSource::Arg);
        assert_eq!(linkedin.params[0].index, Some(0));
        assert_eq!(linkedin.params[1].key, "until");
        assert_eq!(linkedin.params[1].from, TelegramParamSource::Arg);
        assert_eq!(linkedin.params[1].index, Some(1));

        let article = &table["article"];
        assert_eq!(article.params[0].from, TelegramParamSource::Envelope);
        assert_eq!(article.params[0].source_kind, Some(TelegramSourceKind::Url));

        let yt = &table["yt"];
        assert_eq!(yt.params[0].source_kind, Some(TelegramSourceKind::VideoId));

        let shop = &table["shop"];
        assert_eq!(shop.workflow_type, "PRICE_SCOUT");
        assert_eq!(shop.data, Some(serde_json::json!({ "region": "BR" })));
        assert_eq!(shop.params[0].from, TelegramParamSource::Args);
    }

    #[test]
    fn parse_file_telegram_commands_omitted_params_and_data_default() {
        let toml = r#"
[telegram_commands.status]
workflow_type = "STATUS_ONLY"
"#;
        let fc = parse_file(toml).expect("entry with no params/data should parse");
        let entry = &fc.telegram_commands.expect("table present")["status"];
        assert_eq!(entry.workflow_type, "STATUS_ONLY");
        assert!(entry.params.is_empty());
        assert!(entry.data.is_none());
    }

    #[test]
    fn parse_file_telegram_commands_required_defaults_true() {
        let toml = r#"
[telegram_commands.intake]
workflow_type = "DIAGNOSTIC_INTAKE"
params = [{ key = "notes", from = "rest" }]
"#;
        let fc = parse_file(toml).expect("valid TOML should parse");
        let entry = &fc.telegram_commands.expect("table present")["intake"];
        assert!(entry.params[0].required);
    }

    #[test]
    fn parse_file_no_telegram_commands_table_yields_none() {
        let toml = r#"database_url = "postgres://no-telegram/db""#;
        let fc = parse_file(toml).expect("TOML without [telegram_commands] should parse");
        assert!(fc.telegram_commands.is_none());
    }

    #[test]
    fn parse_file_telegram_commands_missing_workflow_type_is_malformed() {
        let toml = r#"
[telegram_commands.bad]
params = [{ key = "notes", from = "rest" }]
"#;
        let err = parse_file(toml).unwrap_err();
        assert!(matches!(err, ConfigError::MalformedFile(_)));
    }

    #[test]
    fn parse_file_telegram_commands_unknown_from_is_malformed() {
        let toml = r#"
[telegram_commands.bad]
workflow_type = "RESEARCH_AGENT"
params = [{ key = "notes", from = "not_a_real_source" }]
"#;
        let err = parse_file(toml).unwrap_err();
        assert!(matches!(err, ConfigError::MalformedFile(_)));
    }

    #[test]
    fn parse_file_telegram_commands_unknown_source_kind_is_malformed() {
        let toml = r#"
[telegram_commands.bad]
workflow_type = "CONTENT_PIPELINE"
params = [{ key = "envelope", from = "envelope", source_kind = "not_a_real_kind" }]
"#;
        let err = parse_file(toml).unwrap_err();
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

    // ─── parse_file / resolve_theme: [theme] section (BA.14.0) ────────────────

    #[test]
    fn parse_file_with_theme_name_round_trips() {
        let toml = r#"
[theme]
name = "bastion"
"#;
        let fc = parse_file(toml).expect("TOML with [theme] should parse");
        let theme = fc.theme.expect("[theme] should be present");
        assert_eq!(theme.name.as_deref(), Some("bastion"));
    }

    #[test]
    fn parse_file_without_theme_section_yields_none() {
        let toml = r#"database_url = "postgres://no-theme/db""#;
        let fc = parse_file(toml).expect("TOML without [theme] should parse");
        assert!(fc.theme.is_none());
    }

    #[test]
    fn parse_file_pre_existing_config_with_no_theme_still_deserializes() {
        // A config written before BA.14.0 — no [theme] section at all.
        let toml = r#"
database_url = "postgres://legacy/db"
default_workspace = "brain"

[workspaces]
brain = "/Users/alice/brain"
"#;
        let fc = parse_file(toml).expect("pre-existing config should still parse");
        assert!(fc.theme.is_none());
        assert_eq!(fc.database_url.as_deref(), Some("postgres://legacy/db"));
    }

    #[test]
    fn resolve_theme_with_known_name_selects_preset() {
        let fc = FileConfig {
            theme: Some(ThemeConfig {
                name: Some("bastion".to_string()),
            }),
            ..Default::default()
        };
        let theme = resolve_theme(&fc);
        assert_eq!(theme, crate::ui_theme::theme_by_name("bastion"));
    }

    #[test]
    fn resolve_theme_with_absent_section_falls_back_to_default() {
        let fc = FileConfig::default();
        let theme = resolve_theme(&fc);
        assert_eq!(theme, crate::ui_theme::theme_by_name(""));
    }

    #[test]
    fn resolve_theme_with_absent_name_falls_back_to_default() {
        let fc = FileConfig {
            theme: Some(ThemeConfig { name: None }),
            ..Default::default()
        };
        let theme = resolve_theme(&fc);
        assert_eq!(theme, crate::ui_theme::theme_by_name(""));
    }

    #[test]
    fn resolve_theme_with_unknown_name_falls_back_to_default() {
        let fc = FileConfig {
            theme: Some(ThemeConfig {
                name: Some("nonexistent-preset".to_string()),
            }),
            ..Default::default()
        };
        let theme = resolve_theme(&fc);
        assert_eq!(theme, crate::ui_theme::theme_by_name("nonexistent-preset"));
        assert_eq!(theme.name, "bastion");
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

    // ─── telegram_config (BA.18.B task 4) ────────────────────────────────────

    #[test]
    fn telegram_config_both_absent_is_none() {
        let cfg = telegram_config(None, None).expect("absent is not an error");
        assert_eq!(cfg, None);
    }

    #[test]
    fn telegram_config_both_present_resolves() {
        let cfg = telegram_config(Some("bot-token-value".into()), Some("chat-42".into()))
            .expect("both present should resolve")
            .expect("expected Some");
        assert_eq!(cfg.bot_token.expose(), "bot-token-value");
        assert_eq!(cfg.chat_id, "chat-42");
    }

    #[test]
    fn telegram_config_token_only_is_typed_error_naming_chat_id() {
        let err = telegram_config(Some("bot-token-value".into()), None).unwrap_err();
        assert_eq!(
            err,
            ConfigError::IncompleteTelegramConfig("BASTION_TELEGRAM_CHAT_ID")
        );
    }

    #[test]
    fn telegram_config_chat_id_only_is_typed_error_naming_bot_token() {
        let err = telegram_config(None, Some("chat-42".into())).unwrap_err();
        assert_eq!(
            err,
            ConfigError::IncompleteTelegramConfig("BASTION_TELEGRAM_BOT_TOKEN")
        );
    }

    #[test]
    fn telegram_config_empty_strings_treated_as_absent() {
        let cfg = telegram_config(Some(String::new()), Some(String::new()))
            .expect("both empty is treated as both absent");
        assert_eq!(cfg, None);
    }

    #[test]
    fn telegram_config_empty_token_with_present_chat_id_is_typed_error() {
        // Empty-string token is treated as absent, so this is the "token
        // missing, chat id present" case, not the reverse.
        let err = telegram_config(Some(String::new()), Some("chat-42".into())).unwrap_err();
        assert_eq!(
            err,
            ConfigError::IncompleteTelegramConfig("BASTION_TELEGRAM_BOT_TOKEN")
        );
    }

    #[test]
    fn bot_token_debug_never_contains_the_token_value() {
        let token = BotToken::new("super-secret-token-value-12345");
        let rendered = format!("{token:?}");
        assert!(
            !rendered.contains("super-secret-token-value-12345"),
            "BotToken Debug must never contain the raw token; got: {rendered}"
        );
        assert_eq!(rendered, "BotToken(<redacted>)");
    }

    #[test]
    fn bot_token_expose_returns_raw_value_for_request_construction() {
        let token = BotToken::new("raw-value");
        assert_eq!(token.expose(), "raw-value");
    }

    // ─── code_sessions_bot_config (BA.20.C task 2) ─────────────────────────────

    #[test]
    fn code_sessions_bot_config_both_absent_is_none() {
        let cfg = code_sessions_bot_config(None, None).expect("absent is not an error");
        assert_eq!(cfg, None);
    }

    #[test]
    fn code_sessions_bot_config_both_present_resolves() {
        let cfg =
            code_sessions_bot_config(Some("cs-bot-token-value".into()), Some("cs-chat-7".into()))
                .expect("both present should resolve")
                .expect("expected Some");
        assert_eq!(cfg.bot_token.expose(), "cs-bot-token-value");
        assert_eq!(cfg.chat_id, "cs-chat-7");
    }

    #[test]
    fn code_sessions_bot_config_token_only_is_typed_error_naming_chat_id() {
        let err = code_sessions_bot_config(Some("cs-bot-token-value".into()), None).unwrap_err();
        assert_eq!(
            err,
            ConfigError::IncompleteTelegramConfig("BASTION_CODESESSIONS_CHAT_ID")
        );
    }

    #[test]
    fn code_sessions_bot_config_chat_id_only_is_typed_error_naming_bot_token() {
        let err = code_sessions_bot_config(None, Some("cs-chat-7".into())).unwrap_err();
        assert_eq!(
            err,
            ConfigError::IncompleteTelegramConfig("BASTION_CODESESSIONS_BOT_TOKEN")
        );
    }

    #[test]
    fn code_sessions_bot_config_empty_strings_treated_as_absent() {
        let cfg = code_sessions_bot_config(Some(String::new()), Some(String::new()))
            .expect("both empty is treated as both absent");
        assert_eq!(cfg, None);
    }

    #[test]
    fn code_sessions_bot_config_empty_token_with_present_chat_id_is_typed_error() {
        // Empty-string token is treated as absent, so this is the "token
        // missing, chat id present" case, not the reverse.
        let err =
            code_sessions_bot_config(Some(String::new()), Some("cs-chat-7".into())).unwrap_err();
        assert_eq!(
            err,
            ConfigError::IncompleteTelegramConfig("BASTION_CODESESSIONS_BOT_TOKEN")
        );
    }

    #[test]
    fn code_sessions_bot_config_debug_never_contains_the_token_value() {
        let cfg = code_sessions_bot_config(
            Some("super-secret-cs-token-98765".into()),
            Some("cs-chat-7".into()),
        )
        .expect("both present should resolve")
        .expect("expected Some");
        let rendered = format!("{cfg:?}");
        assert!(
            !rendered.contains("super-secret-cs-token-98765"),
            "CodeSessionsBotConfig Debug must never contain the raw token; got: {rendered}"
        );
    }

    // ─── lane_bot_config (BA.ticket.notify-operator-cli task 1) ───────────────

    #[test]
    fn lane_bot_config_both_absent_is_none() {
        let cfg = lane_bot_config(None, None).expect("absent is not an error");
        assert_eq!(cfg, None);
    }

    #[test]
    fn lane_bot_config_both_present_resolves() {
        let cfg = lane_bot_config(
            Some("lane-bot-token-value".into()),
            Some("lane-chat-9".into()),
        )
        .expect("both present should resolve")
        .expect("expected Some");
        assert_eq!(cfg.bot_token.expose(), "lane-bot-token-value");
        assert_eq!(cfg.chat_id, "lane-chat-9");
    }

    #[test]
    fn lane_bot_config_token_only_is_typed_error_naming_chat_id() {
        let err = lane_bot_config(Some("lane-bot-token-value".into()), None).unwrap_err();
        assert_eq!(
            err,
            ConfigError::IncompleteTelegramConfig("BASTION_LANE_CHAT_ID")
        );
    }

    #[test]
    fn lane_bot_config_chat_id_only_is_typed_error_naming_bot_token() {
        let err = lane_bot_config(None, Some("lane-chat-9".into())).unwrap_err();
        assert_eq!(
            err,
            ConfigError::IncompleteTelegramConfig("BASTION_LANE_BOT_TOKEN")
        );
    }

    #[test]
    fn lane_bot_config_empty_strings_treated_as_absent() {
        let cfg = lane_bot_config(Some(String::new()), Some(String::new()))
            .expect("both empty is treated as both absent");
        assert_eq!(cfg, None);
    }

    #[test]
    fn lane_bot_config_empty_token_with_present_chat_id_is_typed_error() {
        // Empty-string token is treated as absent, so this is the "token
        // missing, chat id present" case, not the reverse.
        let err = lane_bot_config(Some(String::new()), Some("lane-chat-9".into())).unwrap_err();
        assert_eq!(
            err,
            ConfigError::IncompleteTelegramConfig("BASTION_LANE_BOT_TOKEN")
        );
    }

    #[test]
    fn lane_bot_config_debug_never_contains_the_token_value() {
        let cfg = lane_bot_config(
            Some("super-secret-lane-token-13579".into()),
            Some("lane-chat-9".into()),
        )
        .expect("both present should resolve")
        .expect("expected Some");
        let rendered = format!("{cfg:?}");
        assert!(
            !rendered.contains("super-secret-lane-token-13579"),
            "LaneBotConfig Debug must never contain the raw token; got: {rendered}"
        );
    }

    // ─── named_bot_config (BA.ticket.notify-operator-cli task 1) ──────────────

    #[test]
    fn named_bot_config_both_absent_is_none() {
        let cfg = named_bot_config("codesessions", None, None).expect("absent is not an error");
        assert_eq!(cfg, None);
    }

    #[test]
    fn named_bot_config_both_present_resolves() {
        let cfg = named_bot_config(
            "codesessions",
            Some("cs-bot-token-value".into()),
            Some("cs-chat-7".into()),
        )
        .expect("both present should resolve")
        .expect("expected Some");
        assert_eq!(cfg.slug, "codesessions");
        assert_eq!(cfg.bot_token.expose(), "cs-bot-token-value");
        assert_eq!(cfg.chat_id, "cs-chat-7");
    }

    #[test]
    fn named_bot_config_token_only_is_typed_error_naming_chat_id() {
        let err =
            named_bot_config("codesessions", Some("cs-bot-token-value".into()), None).unwrap_err();
        assert_eq!(
            err,
            ConfigError::IncompleteNamedBotConfig("BASTION_CODESESSIONS_CHAT_ID".to_string())
        );
    }

    #[test]
    fn named_bot_config_chat_id_only_is_typed_error_naming_bot_token() {
        let err = named_bot_config("codesessions", None, Some("cs-chat-7".into())).unwrap_err();
        assert_eq!(
            err,
            ConfigError::IncompleteNamedBotConfig("BASTION_CODESESSIONS_BOT_TOKEN".to_string())
        );
    }

    #[test]
    fn named_bot_config_empty_strings_treated_as_absent() {
        let cfg = named_bot_config("codesessions", Some(String::new()), Some(String::new()))
            .expect("both empty is treated as both absent");
        assert_eq!(cfg, None);
    }

    #[test]
    fn named_bot_config_is_not_lane_specific() {
        // A slug other than "lane" or one of the two pre-existing bots
        // proves the truth table lives in named_bot_config, not baked into
        // any one caller.
        let cfg = named_bot_config("carrierpigeon", Some("pigeon-token".into()), None).unwrap_err();
        assert_eq!(
            cfg,
            ConfigError::IncompleteNamedBotConfig("BASTION_CARRIERPIGEON_CHAT_ID".to_string())
        );
    }

    #[test]
    fn bot_env_var_names_uppercases_a_lowercase_slug() {
        let (token_var, chat_var) = bot_env_var_names("lane");
        assert_eq!(token_var, "BASTION_LANE_BOT_TOKEN");
        assert_eq!(chat_var, "BASTION_LANE_CHAT_ID");
    }

    #[test]
    fn bot_env_var_names_preserves_a_hyphen_in_the_slug() {
        let (token_var, chat_var) = bot_env_var_names("code-sessions");
        assert_eq!(token_var, "BASTION_CODE-SESSIONS_BOT_TOKEN");
        assert_eq!(chat_var, "BASTION_CODE-SESSIONS_CHAT_ID");
    }

    #[test]
    fn lane_bot_config_is_byte_identical_to_named_bot_config_lane_both_absent() {
        assert_eq!(lane_bot_config(None, None), Ok(None));
    }

    #[test]
    fn lane_bot_config_is_byte_identical_to_named_bot_config_lane_both_present() {
        let via_lane = lane_bot_config(
            Some("lane-bot-token-value".into()),
            Some("lane-chat-9".into()),
        )
        .expect("both present should resolve")
        .expect("expected Some");
        let via_named = named_bot_config(
            "lane",
            Some("lane-bot-token-value".into()),
            Some("lane-chat-9".into()),
        )
        .expect("both present should resolve")
        .expect("expected Some");
        assert_eq!(via_lane.bot_token.expose(), via_named.bot_token.expose());
        assert_eq!(via_lane.chat_id, via_named.chat_id);
    }

    #[test]
    fn lane_bot_config_is_byte_identical_to_named_bot_config_lane_token_only() {
        let via_lane = lane_bot_config(Some("lane-bot-token-value".into()), None).unwrap_err();
        assert_eq!(
            via_lane,
            ConfigError::IncompleteTelegramConfig("BASTION_LANE_CHAT_ID")
        );
    }

    #[test]
    fn lane_bot_config_is_byte_identical_to_named_bot_config_lane_chat_id_only() {
        let via_lane = lane_bot_config(None, Some("lane-chat-9".into())).unwrap_err();
        assert_eq!(
            via_lane,
            ConfigError::IncompleteTelegramConfig("BASTION_LANE_BOT_TOKEN")
        );
    }

    #[test]
    fn configured_bot_slugs_reports_only_complete_pairs() {
        let mut env = HashMap::new();
        env.insert(
            "BASTION_TELEGRAM_BOT_TOKEN".to_string(),
            "tg-token".to_string(),
        );
        env.insert(
            "BASTION_TELEGRAM_CHAT_ID".to_string(),
            "tg-chat".to_string(),
        );
        // codesessions: half-configured (token only) — must not count.
        env.insert(
            "BASTION_CODESESSIONS_BOT_TOKEN".to_string(),
            "cs-token".to_string(),
        );
        // lane: entirely absent.

        let configured = configured_bot_slugs(&env);
        assert_eq!(configured, vec!["telegram".to_string()]);
    }

    #[test]
    fn configured_bot_slugs_treats_empty_string_value_as_absent() {
        let mut env = HashMap::new();
        env.insert(
            "BASTION_LANE_BOT_TOKEN".to_string(),
            "lane-token".to_string(),
        );
        env.insert("BASTION_LANE_CHAT_ID".to_string(), String::new());

        let configured = configured_bot_slugs(&env);
        assert!(configured.is_empty());
    }

    #[test]
    fn configured_bot_slugs_empty_env_is_empty() {
        let env = HashMap::new();
        assert!(configured_bot_slugs(&env).is_empty());
    }

    // ─── pricescout bot slug (BA.ticket.pricescout-telegram-bot task 1) ───────

    #[test]
    fn known_bot_slugs_contains_pricescout() {
        assert!(KNOWN_BOT_SLUGS.contains(&"pricescout"));
    }

    #[test]
    fn known_bot_slugs_still_contains_telegram_codesessions_and_lane() {
        // Adding pricescout must not displace or reorder the existing slugs.
        assert!(KNOWN_BOT_SLUGS.contains(&"telegram"));
        assert!(KNOWN_BOT_SLUGS.contains(&"codesessions"));
        assert!(KNOWN_BOT_SLUGS.contains(&"lane"));
    }

    #[test]
    fn named_bot_config_pricescout_both_absent_is_none() {
        let cfg = named_bot_config("pricescout", None, None).expect("absent is not an error");
        assert_eq!(cfg, None);
    }

    #[test]
    fn named_bot_config_pricescout_both_present_resolves() {
        let cfg = named_bot_config(
            "pricescout",
            Some("ps-bot-token-value".into()),
            Some("ps-chat-42".into()),
        )
        .expect("both present should resolve")
        .expect("expected Some");
        assert_eq!(cfg.slug, "pricescout");
        assert_eq!(cfg.bot_token.expose(), "ps-bot-token-value");
        assert_eq!(cfg.chat_id, "ps-chat-42");
    }

    #[test]
    fn named_bot_config_pricescout_token_only_is_typed_error_naming_chat_id() {
        let err =
            named_bot_config("pricescout", Some("ps-bot-token-value".into()), None).unwrap_err();
        assert_eq!(
            err,
            ConfigError::IncompleteNamedBotConfig("BASTION_PRICESCOUT_CHAT_ID".to_string())
        );
    }

    #[test]
    fn named_bot_config_pricescout_chat_id_only_is_typed_error_naming_bot_token() {
        let err = named_bot_config("pricescout", None, Some("ps-chat-42".into())).unwrap_err();
        assert_eq!(
            err,
            ConfigError::IncompleteNamedBotConfig("BASTION_PRICESCOUT_BOT_TOKEN".to_string())
        );
    }

    #[test]
    fn pricescout_env_var_names_are_derived_correctly() {
        let (token_var, chat_var) = bot_env_var_names("pricescout");
        assert_eq!(token_var, "BASTION_PRICESCOUT_BOT_TOKEN");
        assert_eq!(chat_var, "BASTION_PRICESCOUT_CHAT_ID");
    }

    #[test]
    fn configured_bot_slugs_reports_pricescout_when_complete() {
        let mut env = HashMap::new();
        env.insert(
            "BASTION_PRICESCOUT_BOT_TOKEN".to_string(),
            "ps-token".to_string(),
        );
        env.insert(
            "BASTION_PRICESCOUT_CHAT_ID".to_string(),
            "ps-chat".to_string(),
        );

        let configured = configured_bot_slugs(&env);
        assert_eq!(configured, vec!["pricescout".to_string()]);
    }

    // ─── planning_root ────────────────────────────────────────────────────────

    #[test]
    fn planning_root_defaults_to_planning() {
        let root = planning_root(None);
        assert!(root.ends_with("planning"));
    }

    #[test]
    fn planning_root_env_val_overrides_default() {
        let root = planning_root(Some("/absolute/path/planning".into()));
        assert_eq!(root, PathBuf::from("/absolute/path/planning"));
    }

    #[test]
    fn planning_root_empty_env_val_falls_back_to_default() {
        let root = planning_root(Some(String::new()));
        assert!(root.ends_with("planning"));
    }

    #[test]
    fn planning_root_relative_env_val_is_preserved_as_given() {
        let root = planning_root(Some("../other/planning".into()));
        assert_eq!(root, PathBuf::from("../other/planning"));
    }

    // ─── brain_toml_path ──────────────────────────────────────────────────────

    #[test]
    fn brain_toml_path_defaults_to_brain_toml() {
        let root = brain_toml_path(None);
        assert!(root.ends_with("brain.toml"));
    }

    #[test]
    fn brain_toml_path_env_val_overrides_default() {
        let root = brain_toml_path(Some("/absolute/path/brain.toml".into()));
        assert_eq!(root, PathBuf::from("/absolute/path/brain.toml"));
    }

    // ─── budget caps + engine_api_key (BA.7.C task 1) ─────────────────────────

    #[allow(clippy::type_complexity)]
    fn budget_env(
        max_total_tokens: Option<&str>,
        max_cost_usd: Option<&str>,
        engine_api_key: Option<&str>,
    ) -> (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) {
        (
            Some("postgres://localhost/db".into()),
            None,
            None,
            max_total_tokens.map(String::from),
            max_cost_usd.map(String::from),
            engine_api_key.map(String::from),
            None,
        )
    }

    #[test]
    fn budget_neither_set_is_valid_and_unchanged() {
        // Absent-tolerant: no budget/key configured anywhere is a valid config.
        let c = Config::from_sources(budget_env(None, None, None), FileConfig::default())
            .expect("should parse");
        assert_eq!(c.max_total_tokens, None);
        assert_eq!(c.max_cost_usd, None);
        assert_eq!(c.engine_api_key, None);
    }

    #[test]
    fn budget_file_only_is_used() {
        let file = FileConfig {
            max_total_tokens: Some(100_000),
            max_cost_usd: Some(5.5),
            engine_api_key: Some("file-key".into()),
            ..Default::default()
        };
        let c = Config::from_sources(budget_env(None, None, None), file).expect("should parse");
        assert_eq!(c.max_total_tokens, Some(100_000));
        assert_eq!(c.max_cost_usd, Some(5.5));
        assert_eq!(c.engine_api_key.as_deref(), Some("file-key"));
    }

    #[test]
    fn budget_env_wins_over_file() {
        let file = FileConfig {
            max_total_tokens: Some(100_000),
            max_cost_usd: Some(5.5),
            engine_api_key: Some("file-key".into()),
            ..Default::default()
        };
        let c = Config::from_sources(
            budget_env(Some("50000"), Some("2.25"), Some("env-key")),
            file,
        )
        .expect("should parse");
        assert_eq!(c.max_total_tokens, Some(50_000));
        assert_eq!(c.max_cost_usd, Some(2.25));
        assert_eq!(c.engine_api_key.as_deref(), Some("env-key"));
    }

    #[test]
    fn budget_each_cap_set_independently_tokens_only() {
        let c = Config::from_sources(budget_env(Some("42"), None, None), FileConfig::default())
            .expect("should parse");
        assert_eq!(c.max_total_tokens, Some(42));
        assert_eq!(c.max_cost_usd, None);
    }

    #[test]
    fn budget_each_cap_set_independently_cost_only() {
        let c = Config::from_sources(budget_env(None, Some("1.5"), None), FileConfig::default())
            .expect("should parse");
        assert_eq!(c.max_total_tokens, None);
        assert_eq!(c.max_cost_usd, Some(1.5));
    }

    #[test]
    fn budget_malformed_max_total_tokens_is_typed_error_not_silent_default() {
        let err = Config::from_sources(
            budget_env(Some("not-a-number"), None, None),
            FileConfig::default(),
        )
        .unwrap_err();
        assert_eq!(
            err,
            ConfigError::MalformedBudgetValue(
                "BASTION_MAX_TOTAL_TOKENS",
                "not-a-number".into(),
                "u64 token count"
            )
        );
    }

    #[test]
    fn budget_malformed_max_cost_usd_is_typed_error_not_silent_default() {
        let err = Config::from_sources(
            budget_env(None, Some("not-a-float"), None),
            FileConfig::default(),
        )
        .unwrap_err();
        assert_eq!(
            err,
            ConfigError::MalformedBudgetValue(
                "BASTION_MAX_COST_USD",
                "not-a-float".into(),
                "f64 USD amount"
            )
        );
    }

    #[test]
    fn budget_malformed_negative_tokens_is_typed_error() {
        // u64 rejects negative values — must not silently coerce or default.
        let err = Config::from_sources(budget_env(Some("-5"), None, None), FileConfig::default())
            .unwrap_err();
        assert!(matches!(
            err,
            ConfigError::MalformedBudgetValue("BASTION_MAX_TOTAL_TOKENS", _, _)
        ));
    }

    #[test]
    fn budget_engine_api_key_distinct_from_serve_token() {
        // engine_api_key and ServeConfig.token are two different secrets/schemes —
        // constructing one must never populate or influence the other.
        let c = Config::from_sources(
            budget_env(None, None, Some("engine-secret")),
            FileConfig::default(),
        )
        .expect("should parse");
        assert_eq!(c.engine_api_key.as_deref(), Some("engine-secret"));

        let sc = build_serve_config(None, Some("serve-secret".into()), None, None).unwrap();
        assert_eq!(sc.token, "serve-secret");
        assert_ne!(c.engine_api_key.as_deref(), Some(sc.token.as_str()));
    }

    // ─── notify_enabled (BASTION_NOTIFY, Part B) ─────────────────────────────
    //
    // Mirrors `poll_interval_secs`'s own parsing tests: opt-out (defaults to
    // `true`), lenient parse (an unparseable value silently falls back to the
    // default rather than erroring — unlike the budget values above).

    #[test]
    fn notify_enabled_defaults_to_true_when_unset() {
        let c = Config::from_sources(
            (
                Some("postgres://localhost/db".into()),
                None,
                None,
                None,
                None,
                None,
                None,
            ),
            FileConfig::default(),
        )
        .expect("should parse");
        assert!(c.notify_enabled, "notify_enabled should default to true");
    }

    #[test]
    fn notify_enabled_env_false_disables() {
        let c = Config::from_sources(
            (
                Some("postgres://localhost/db".into()),
                None,
                None,
                None,
                None,
                None,
                Some("false".into()),
            ),
            FileConfig::default(),
        )
        .expect("should parse");
        assert!(!c.notify_enabled);
    }

    #[test]
    fn notify_enabled_env_true_enables() {
        let c = Config::from_sources(
            (
                Some("postgres://localhost/db".into()),
                None,
                None,
                None,
                None,
                None,
                Some("true".into()),
            ),
            FileConfig::default(),
        )
        .expect("should parse");
        assert!(c.notify_enabled);
    }

    #[test]
    fn notify_enabled_unparseable_value_falls_back_to_default() {
        // Lenient parse, matching BASTION_POLL_INTERVAL's own convention —
        // not a hard ConfigError like the budget values.
        let c = Config::from_sources(
            (
                Some("postgres://localhost/db".into()),
                None,
                None,
                None,
                None,
                None,
                Some("not-a-bool".into()),
            ),
            FileConfig::default(),
        )
        .expect("unparseable BASTION_NOTIFY should not error");
        assert!(
            c.notify_enabled,
            "unparseable value should silently fall back to the true default"
        );
    }

    #[test]
    fn from_vars_notify_enabled_defaults_to_true() {
        let c = Config::from_vars(Some("postgres://localhost/db".into()), None, None)
            .expect("should parse");
        assert!(c.notify_enabled);
    }

    // ─── walk_up_from (pure, path-anchored) ──────────────────────────────────

    #[test]
    fn walk_up_from_finds_target_in_start_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target_path = dir.path().join("brain.toml");
        std::fs::write(&target_path, "").expect("write target");

        let found = walk_up_from(dir.path(), "brain.toml");
        assert_eq!(found, Some(target_path));
    }

    #[test]
    fn walk_up_from_finds_target_several_levels_up() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target_path = dir.path().join("brain.toml");
        std::fs::write(&target_path, "").expect("write target");

        let nested = dir.path().join("a").join("b").join("c");
        std::fs::create_dir_all(&nested).expect("create nested dirs");

        let found = walk_up_from(&nested, "brain.toml");
        assert_eq!(found, Some(target_path));
    }

    #[test]
    fn walk_up_from_returns_none_when_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("a").join("b");
        std::fs::create_dir_all(&nested).expect("create nested dirs");

        // A target that certainly does not exist anywhere up this isolated tempdir tree.
        let found = walk_up_from(&nested, "definitely-not-a-real-target-file.toml");
        assert_eq!(found, None);
    }

    #[test]
    fn walk_up_from_prefers_nearest_match_over_ancestor_match() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root_target = dir.path().join("planning");
        std::fs::create_dir_all(&root_target).expect("create root target");

        let nested = dir.path().join("child");
        let nested_target = nested.join("planning");
        std::fs::create_dir_all(&nested_target).expect("create nested target");

        let found = walk_up_from(&nested, "planning");
        assert_eq!(found, Some(nested_target));
    }

    // ─── walk_up_for regression (planning_root / brain_toml_path unchanged) ──

    #[test]
    fn planning_root_still_resolves_via_walk_up_for_when_env_absent() {
        // Regression: planning_root(None) delegates to walk_up_for("planning"),
        // which now delegates to walk_up_from(cwd, "planning"). Behavior must be
        // unchanged: either it finds a real "planning" dir walking up from cwd,
        // or it falls back to the bare relative PathBuf::from("planning").
        let root = planning_root(None);
        assert_eq!(root.file_name().and_then(|n| n.to_str()), Some("planning"));
    }

    #[test]
    fn brain_toml_path_still_resolves_via_walk_up_for_when_env_absent() {
        let path = brain_toml_path(None);
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some("brain.toml")
        );
    }

    // ─── parse_file: budget + engine_api_key TOML keys ───────────────────────

    #[test]
    fn parse_file_budget_keys_round_trip() {
        let toml = r#"
database_url = "postgres://budget-test/db"
max_total_tokens = 250000
max_cost_usd = 12.75
engine_api_key = "toml-engine-key"
"#;
        let fc = parse_file(toml).expect("valid TOML with budget keys should parse");
        assert_eq!(fc.max_total_tokens, Some(250_000));
        assert_eq!(fc.max_cost_usd, Some(12.75));
        assert_eq!(fc.engine_api_key.as_deref(), Some("toml-engine-key"));
    }

    #[test]
    fn parse_file_without_budget_keys_yields_none() {
        let toml = r#"database_url = "postgres://no-budget/db""#;
        let fc = parse_file(toml).expect("TOML without budget keys should parse");
        assert!(fc.max_total_tokens.is_none());
        assert!(fc.max_cost_usd.is_none());
        assert!(fc.engine_api_key.is_none());
    }
}
