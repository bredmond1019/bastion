---
type: Reference
title: Configuration
description: How bastion reads its configuration — env vars, config file, and built-in defaults.
doc_id: config
layer: [console]
project: bastion
status: active
keywords: [configuration, environment variables, config file, workspace registry, precedence, TOML, theme]
related: [observ, serve-api, brain, sessions]
---

# Configuration

bastion resolves configuration from three layers, in descending precedence:

1. **Environment variables** (highest precedence)
2. **`~/.config/bastion/config.toml`** (or `$XDG_CONFIG_HOME/bastion/config.toml`)
3. **Built-in defaults** (lowest precedence)

## Global CLI flags

These flags appear before the subcommand and apply to every invocation:

| Flag | Short | Default | Description |
|---|---|---|---|
| `--verbose` | `-v` | `false` | Raise log verbosity to DEBUG (default: INFO). Repeated use is accepted but has no additional effect. |
| `--json-logs` | — | `false` | Emit structured JSON log lines to stderr instead of human-readable text. Useful for log aggregators or piping into `jq`. |

Both flags are declared `global = true` in clap, so they work before or after any subcommand.

The flags are consumed by `observ::init_tracing(verbose, json_logs)`, called once at the top of `main()` before dispatch. The `RUST_LOG` environment variable overrides the level set by `--verbose` when both are present.

## Environment variables

| Variable | Required | Default | Description |
|---|---|---|---|
| `DATABASE_URL` | Yes (unless set in config file) | — | PostgreSQL URL for the Python orchestrator's database |
| `BASTION_API_URL` | No | `http://localhost:8080` | FastAPI orchestrator base URL |
| `BASTION_POLL_INTERVAL` | No | `2` | Monitor poll cadence in seconds |
| `BASTION_SERVE_ADDR` | No | `0.0.0.0:4317` | Bind address for `bastion serve` |
| `BASTION_SERVE_TOKEN` | Yes (for `bastion serve`) | — | Bearer token enforced on all protected routes; also settable via `--token` |
| `BASTION_PLANNING_ROOT` | No | `planning/` | Root directory for planning state and harnesses |
| `BASTION_BRAIN_TOML` | No | `brain.toml` | Path to the workspace definition registry |
| `BASTION_MAX_TOTAL_TOKENS` | No | — (no cap) | Budget cap (BA.7.C): total token ceiling. Absent-tolerant — no cap configured is a valid, unchanged config. A present-but-unparseable value is a fatal `ConfigError::MalformedBudgetValue`, never a silent default. |
| `BASTION_MAX_COST_USD` | No | — (no cap) | Budget cap (BA.7.C): total USD-cost ceiling. Same absent-tolerant / malformed-is-fatal contract as `BASTION_MAX_TOTAL_TOKENS`. |
| `BASTION_ENGINE_API_KEY` | No (required to use `bastion abort` / engine routes) | — | `X-API-Key` secret for the engine's abort endpoint. **Distinct from `BASTION_SERVE_TOKEN`** — two different secrets, two different schemes, two different route groups: this key is sent by `api::client` and checked by the embedded engine's `AppState.api_key`; `BASTION_SERVE_TOKEN` gates bastion serve's own session/status routes. Never reuse one for the other. |

## Config file

bastion looks for a TOML config file at:

1. `$XDG_CONFIG_HOME/bastion/config.toml` (if `$XDG_CONFIG_HOME` is set)
2. `~/.config/bastion/config.toml` (fallback)

A missing or unreadable file is silently ignored — bastion degrades to built-in defaults.
A present but malformed TOML file is a fatal error.

### Example `~/.config/bastion/config.toml`

```toml
database_url   = "postgres://postgres:postgres@localhost:5432/postgres"
api_base_url   = "http://localhost:8080"
poll_interval  = 2

default_workspace = "brain"

[workspaces]
brain    = "/Users/alice/brain"
client-a = "/Users/alice/clients/client-a/notes"

# Budget caps (BA.7.C) — both optional, absent-tolerant.
max_total_tokens = 1000000
max_cost_usd     = 25.00

# X-API-Key for the engine's abort endpoint (BA.7.C) — distinct from BASTION_SERVE_TOKEN.
engine_api_key = "<engine-api-key>"
```

All keys are optional. Unknown keys are ignored (forward-compatible).

### `[theme]` section

Selects a named UI theme preset for the TUI console (`bastion tui`), applied to both the chrome
and the `bella-engine` markdown view.

```toml
[theme]
name = "bastion"
```

`name` is optional; an absent `[theme]` section, an absent `name`, or an unrecognized name all
fall back to the built-in `bastion` preset — never a parse error or panic. Currently `bastion` is
the only implemented preset.

## Workspace registry

The `[workspaces]` table and `default_workspace` key support named corpus roots for
`bastion brain`. They have no effect on the observability track (monitor, costs, inspect).

| Key | Type | Description |
|---|---|---|
| `default_workspace` | `String` | Name of the workspace used when `--workspace` is not supplied on the CLI. |
| `[workspaces]` | `HashMap<String, PathBuf>` | Maps short names to absolute corpus root paths. |

`bastion brain` resolves the effective corpus root with the following precedence:

1. `--root <DIR>` on the CLI (explicit override; highest priority).
2. `--workspace <NAME>` (alias `--knowledge-dir`) — looks up `NAME` in `[workspaces]`.
3. `default_workspace` in the config file — resolved from `[workspaces]`.
4. Built-in default: current directory (`.`).

An unknown name in step 2 or 3 is a fatal error (`ConfigError::UnknownWorkspace`).

## Budget caps + engine API key (BA.7.C)

Three new, fully optional keys back the cost-budget-alerts-abort block:

| Key | Env var | Type | Description |
|---|---|---|---|
| `max_total_tokens` | `BASTION_MAX_TOTAL_TOKENS` | `Option<u64>` | Budget cap: total token ceiling for a run. |
| `max_cost_usd` | `BASTION_MAX_COST_USD` | `Option<f64>` | Budget cap: total USD-cost ceiling for a run. |
| `engine_api_key` | `BASTION_ENGINE_API_KEY` | `Option<String>` | `X-API-Key` secret for the engine's abort endpoint. |

All three follow the same env-over-file precedence as every other key, and all three are
**absent-tolerant**: with none configured, behavior is unchanged from before the v1.1.0 data
contract (no gate, no alert, no authenticated abort). `engine_api_key` is a distinct secret
from `ServeConfig.token` (the `Authorization: Bearer` gate `bastion serve` enforces on its own
session/status routes) — one authenticates operator access to bastion serve, the other
authenticates bastion's own outbound calls to the engine's abort endpoint. Never conflate or
reuse one for the other.

A `BASTION_MAX_TOTAL_TOKENS` or `BASTION_MAX_COST_USD` value that is present but fails to parse
as its numeric type (e.g. `BASTION_MAX_TOTAL_TOKENS=not-a-number`) is a fatal
`ConfigError::MalformedBudgetValue` — it is never silently treated as "no cap configured".

## Precedence rules

An environment variable **always wins** over the config file for the same key.
The config file fills any gap the environment does not set.
Built-in defaults apply only when both the environment and file omit a value.

`DATABASE_URL` is the only required value — it must appear in at least one source.

## Public API (`crates/bastion/src/config.rs`)

### `ConfigError`

| Variant | Description |
|---|---|
| `MissingVar(&'static str)` | Required env var not set. |
| `MalformedFile(String)` | Config file present but not valid TOML. |
| `UnknownWorkspace(String)` | Named workspace not found in the `[workspaces]` registry. |
| `NoWorkspaceRegistry` | `--workspace` used but no `[workspaces]` table exists in the config file. |
| `MissingServeToken` | `bastion serve` started without a bearer token (neither `--token` nor `BASTION_SERVE_TOKEN` set, or either resolved to an empty string). |
| `MalformedBudgetValue(&'static str, String, &'static str)` | A budget env var (`BASTION_MAX_TOTAL_TOKENS` or `BASTION_MAX_COST_USD`) was set but failed to parse as its expected numeric type. Carries the variable name, the offending value, and the expected type. Never silently defaults to "no cap". |

### `FileConfig`

Struct that mirrors the config-file keys. All fields are optional; constructed by
`parse_file` or `load_workspace_registry`.

| Field | Type | Description |
|---|---|---|
| `database_url` | `Option<String>` | PostgreSQL URL. |
| `api_base_url` | `Option<String>` | FastAPI base URL. |
| `poll_interval` | `Option<u64>` | Monitor poll cadence (seconds). |
| `workspaces` | `Option<HashMap<String, PathBuf>>` | Named corpus root paths. |
| `default_workspace` | `Option<String>` | Default workspace name. |
| `theme` | `Option<ThemeConfig>` | Optional `[theme]` section — see [`[theme]` section](#theme-section) above. |
| `max_total_tokens` | `Option<u64>` | Budget cap (BA.7.C): total token ceiling. |
| `max_cost_usd` | `Option<f64>` | Budget cap (BA.7.C): total USD-cost ceiling. |
| `engine_api_key` | `Option<String>` | `X-API-Key` secret for the engine's abort endpoint (BA.7.C). Distinct from `ServeConfig.token`. |

### `ThemeConfig`

| Field | Type | Description |
|---|---|---|
| `name` | `Option<String>` | Theme preset name, resolved via `ui_theme::theme_by_name`. |

### `resolve_theme`

```rust
pub fn resolve_theme(file: &FileConfig) -> crate::ui_theme::Theme
```

Pure function (no I/O). Resolves the active theme from a parsed `FileConfig`: an absent
`[theme]` section or `name`, or an unrecognized name, all fall back to the `bastion` default
via `ui_theme::theme_by_name`. Never panics.

### `resolve_workspace_root`

```rust
pub fn resolve_workspace_root(
    explicit_root: Option<PathBuf>,
    workspace_name: Option<&str>,
    file: &FileConfig,
) -> Result<PathBuf, ConfigError>
```

Pure function (no I/O). Applies the four-level precedence described above and returns
the effective corpus root. Returns `ConfigError::UnknownWorkspace` for an unrecognised
workspace name.

### `load_workspace_registry`

```rust
pub fn load_workspace_registry(
    xdg_config_home: Option<String>,
    home: Option<String>,
) -> Result<FileConfig, ConfigError>
```

Reads the config file (DB-free path, no `DATABASE_URL` required). Returns
`FileConfig::default()` when the file is absent or unreadable; returns
`ConfigError::MalformedFile` on parse errors.

### `ServeConfig`

DB-free configuration struct for `bastion serve`. Does not require `DATABASE_URL`.

| Field | Type | Description |
|---|---|---|
| `addr` | `String` | Bind address (e.g. `"0.0.0.0:4317"`). Default: `0.0.0.0:4317`. |
| `token` | `String` | Bearer token enforced by `BearerAuthMiddleware` on all protected routes. Mandatory and non-empty — absence or empty string is `ConfigError::MissingServeToken`. |

### `build_serve_config`

```rust
pub fn build_serve_config(
    addr_flag: Option<String>,
    token_flag: Option<String>,
    addr_env: Option<String>,
    token_env: Option<String>,
) -> Result<ServeConfig, ConfigError>
```

Pure function (no I/O). Merges CLI flags (highest precedence) over env vars (middle) over
the built-in address default (`0.0.0.0:4317`). Returns `ConfigError::MissingServeToken`
when neither `token_flag` nor `token_env` is provided, or when the resolved token is an
empty string (e.g. `BASTION_SERVE_TOKEN=` in the environment).

### `load_serve_config`

```rust
pub fn load_serve_config(
    addr_flag: Option<String>,
    token_flag: Option<String>,
) -> Result<ServeConfig, ConfigError>
```

I/O wrapper around `build_serve_config`. Reads `BASTION_SERVE_ADDR` and
`BASTION_SERVE_TOKEN` from the environment (after loading `.env` via `dotenvy`) and
delegates to `build_serve_config`. DB-free — does not touch `DATABASE_URL`.
