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

## Quickstart

```bash
# the minimum for the observability commands
export DATABASE_URL="postgres://user:pass@localhost:5432/db"
export BASTION_API_URL="http://localhost:8080"
bastion status            # confirms both are reachable

# optional: named corpus roots for `brain`, `code` and `momentum`
mkdir -p ~/.config/bastion
cat > ~/.config/bastion/config.toml <<'TOML'
default_workspace = "brain"

[workspaces]
brain = "/Users/you/Dev/agentic-portfolio"
TOML
```

Nothing here is required to run `bastion sessions`, `bastion view` or the brain-ops
pass-throughs — those read no configuration at all. Full variable table:
[Environment variables](#environment-variables). Which command needs what:
[commands.md § Quickstart](commands.md#quickstart).

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
| `BASTION_NOTIFY` | No | `true` | Desktop-notification toggle for `bastion monitor` (TUI and `--watch`) — opt-out, not opt-in. Parsed leniently: an unparseable value silently falls back to the default rather than erroring (unlike the budget values below). macOS-only regardless of setting — a no-op on other platforms. See [monitor.md](monitor.md#desktop-notifications). |
| `BASTION_SERVE_ADDR` | No | `0.0.0.0:4317` | Bind address for `bastion serve` |
| `BASTION_SERVE_TOKEN` | Yes (for `bastion serve`) | — | Bearer token enforced on all protected routes; also settable via `--token` |
| `BASTION_PLANNING_ROOT` | No | `planning/` | Root directory for planning state and harnesses |
| `BASTION_BRAIN_TOML` | No | `brain.toml` | Path to the workspace definition registry |
| `BASTION_MAX_TOTAL_TOKENS` | No | — (no cap) | Budget cap (BA.7.C): total token ceiling. Absent-tolerant — no cap configured is a valid, unchanged config. A present-but-unparseable value is a fatal `ConfigError::MalformedBudgetValue`, never a silent default. |
| `BASTION_MAX_COST_USD` | No | — (no cap) | Budget cap (BA.7.C): total USD-cost ceiling. Same absent-tolerant / malformed-is-fatal contract as `BASTION_MAX_TOTAL_TOKENS`. |
| `BASTION_ENGINE_API_KEY` | No (required to use `bastion abort` / engine routes) | — | `X-API-Key` secret for the engine's abort endpoint. **Distinct from `BASTION_SERVE_TOKEN`** — two different secrets, two different schemes, two different route groups: this key is sent by `api::client` and checked by the embedded engine's `AppState.api_key`; `BASTION_SERVE_TOKEN` gates bastion serve's own session/status routes. Never reuse one for the other. |
| `BASTION_ENGINE_HARNESS_PATH` | No | — (no schedule loop spawned) | Path to the `harness.json` whose `schedule` block configures the embedded engine's scheduled entries (`engine_serve::schedule::spawn_schedule_loop`). Absent, empty, or pointing at a path that does not exist/is not readable all resolve to `None` — the ordinary case today — and `bastion serve` starts with no schedule loop, logged at info, not error. A path that resolves but whose `schedule` block fails to parse is a distinct, louder `tracing::error!` outcome, but still does not abort startup. Env-var only — no `config.toml` key. |
| `BASTION_TELEGRAM_BOT_TOKEN` | No | — | Telegram bot token for the operator-notification transport (`BA.18.B`, [serve-api.md](serve-api.md#26-operator-notification-transport)). **Mini-plist-only** — the real value lives in `com.brandon.engine-serve.plist` on the Mac Mini and is never written to any tracked file, `.env`, test, or fixture in this repo. Both this and `BASTION_TELEGRAM_CHAT_ID` absent leaves the transport unconfigured (`bastion serve` boots unchanged); exactly one present is a typed `ConfigError::IncompleteTelegramConfig`. |
| `BASTION_TELEGRAM_CHAT_ID` | No | — | Operator's Telegram chat id the bot delivers to. Same Mini-plist-only, absent-tolerant, paired-with-the-token contract as `BASTION_TELEGRAM_BOT_TOKEN` above. |
| `BASTION_CODESESSIONS_BOT_TOKEN` | No | — | Bot token for CodeSessionsBot, the session-QA bridge's bot (`BA.20.C`, [serve-api.md](serve-api.md#27-session-qa-bridge)). **Deliberately distinct from `BASTION_TELEGRAM_BOT_TOKEN`** — that pair is BastionBot's approve/reject gate transport; this pair is a second bot, shared with the HQ chore's `claude_session_notify.sh`. CodeSessionsBot does not exist yet as of `BA.20.C`, so unset is the expected state today (bridge disabled). Both this and `BASTION_CODESESSIONS_CHAT_ID` absent leaves the bridge disabled; exactly one present is a typed `ConfigError::IncompleteTelegramConfig`. |
| `BASTION_CODESESSIONS_CHAT_ID` | No | — | The operator's Telegram chat id CodeSessionsBot delivers to. Same absent-tolerant, paired-with-the-token contract as `BASTION_CODESESSIONS_BOT_TOKEN` above. |
| `BASTION_LANE_BOT_TOKEN` | No | — | Bot token for LaneBot, the third bot `bastion notify send\|ask` (`BA.ticket.notify-operator-cli`, [notify.md](notify.md)) uses by default. **Deliberately distinct from `BASTION_TELEGRAM_BOT_TOKEN` and `BASTION_CODESESSIONS_BOT_TOKEN`** — `bastion serve` already runs one `getUpdates` long-poll per bot token (BastionBot's approve/reject gate, CodeSessionsBot's session-QA bridge), and Telegram hands each update to exactly one consumer, so a CLI polling either of those tokens would steal the taps those loops exist to receive. LaneBot's credentials are not provisioned yet (`operator-lanebot-credential`); both this and `BASTION_LANE_CHAT_ID` absent leaves `--bot lane` unconfigured — exactly one present is a typed `ConfigError::IncompleteTelegramConfig`. |
| `BASTION_LANE_CHAT_ID` | No | — | The operator's Telegram chat id LaneBot delivers to. Same absent-tolerant, paired-with-the-token contract as `BASTION_LANE_BOT_TOKEN` above. |
| `BASTION_FAIL_ON_BUILD_DRIFT` | No | `false` (falsy) | Opt-in hard-fail switch for build-provenance drift on `bastion emit-state --write` — see [brainval.md](brainval.md#--build-stamp-and-the-build-provenance-drift-guard). By default, drift between the running binary's compiled-in git SHA and the live source tree prints a loud stderr banner but still lets the write proceed; setting this env var to a truthy value (`1`, `true`, `yes`, `on`, case-insensitive) turns that same drift into a non-zero exit with nothing written, before `mev::emit_state` is called. Same effect as the `--fail-on-drift` flag on `emit-state` — either alone is sufficient, with no precedence to reason about. Exists so `scripts/routine.sh`'s unattended nightly run on the Mac Mini can refuse to write from a stale build without editing that HQ-owned script's arguments. |

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

The `[workspaces]` table and `default_workspace` key name the repos bastion can look at, so you
do not have to pass `--root` every time. Three commands read it:

- [`bastion brain`](brain.md) and [`bastion code`](code.md) — to resolve the corpus / source root
  to scan (both also accept `--root` and `--workspace`, which override it).
- [`bastion momentum`](momentum.md) — to know which repos to roll up. Unlike the other two, it
  reads **every** registered workspace and has no `--root` escape hatch, so an unregistered repo
  simply does not appear.

It has no effect on the observability track (monitor, costs, inspect).

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

## Operator-notification transport (`BA.18.B`)

Two fully optional env vars configure the Telegram operator-notification transport
(`src/serve/notify/`); see [serve-api.md §26](serve-api.md#26-operator-notification-transport)
for the full contract.

| Env var | Type | Description |
|---|---|---|
| `BASTION_TELEGRAM_BOT_TOKEN` | `Option<String>` | Telegram bot token. |
| `BASTION_TELEGRAM_CHAT_ID` | `Option<String>` | The operator's Telegram chat id. |

**Both are Mini-plist-only** — the real values live in the Mac Mini's
`com.brandon.engine-serve.plist` and never in `.env`, `.env.example` (beyond an empty
placeholder), a config-file example, a test fixture, or a log line anywhere in this repo. Read
directly from the environment via `src/config.rs`'s `load_telegram_config`, not through
`FileConfig`/`config.toml` — there is deliberately no `[telegram]` TOML section, so the token
cannot end up in a config file a developer might accidentally track or share.

Absent-tolerant as a pair: with neither set, `bastion serve` boots exactly as it did before this
block, logged at `info`. Exactly one set (token without chat id, or the reverse) is a fatal
`ConfigError::IncompleteTelegramConfig(&'static str)` naming the missing variable — never a silent
half-configuration. The token is held in a `BotToken` newtype whose `Debug` impl always renders
`BotToken(<redacted>)`.

**`BASTION_TELEGRAM_CHAT_ID` doubles as the approval-ledger identity.** When a tap is resolved,
`ticket-approve-and-run-seams` records a row attributing the decision to this chat id — the bot is
configured against a single chat and there is no operator identity to read. That is honest about
*which channel* approved something, but the ledger cannot distinguish two people with access to the
same chat. See [serve-api.md §26.9](serve-api.md#269-approve-and-run-resolution-v030-ticket-approve-and-run-seams)
for the ledger's XDG path resolution and the rest of the contract.

**Deployment trap:** the Mini's plist currently carries these as `TELEGRAM_BOT_TOKEN` /
`TELEGRAM_CHAT_ID`, **without** the `BASTION_` prefix `load_telegram_config` reads. Deployed as-is,
the transport logs "not configured" at `info` and stays silently off. Rename the plist entries — or
the reader — before relying on the Mini for operator notifications.

## Session-QA bridge bot (`BA.20.C`)

A second, fully optional env-var pair configures CodeSessionsBot, the session-QA bridge's bot
(`src/serve/session_qa/`); see
[serve-api.md §27](serve-api.md#27-session-qa-bridge) for
the full contract.

| Env var | Type | Description |
|---|---|---|
| `BASTION_CODESESSIONS_BOT_TOKEN` | `Option<String>` | CodeSessionsBot's Telegram bot token. |
| `BASTION_CODESESSIONS_CHAT_ID` | `Option<String>` | The operator's Telegram chat id CodeSessionsBot delivers to. |

**Deliberately distinct from the pair above** — those configure BastionBot (the approve/reject gate
transport); these configure CodeSessionsBot (the session-QA bridge, shared with the HQ chore's
`claude_session_notify.sh`). Two bots, two token pairs, never conflated. Read via
`load_code_sessions_bot_config` / the pure `code_sessions_bot_config`, mirroring
`load_telegram_config`'s both-or-neither rule exactly (same `ConfigError::IncompleteTelegramConfig`
error, reused rather than duplicated). CodeSessionsBot does not exist yet as of `BA.20.C`, so both
absent — the bridge disabled — is the expected state today. Same `BotToken` newtype, same redacted
`Debug`.

## LaneBot (`BA.ticket.notify-operator-cli`)

A third, fully optional env-var pair configures LaneBot, the default bot for `bastion notify
send|ask` (`src/notify_cli.rs`); see [notify.md](notify.md) for the full verb contract.

| Env var | Type | Description |
|---|---|---|
| `BASTION_LANE_BOT_TOKEN` | `Option<String>` | LaneBot's Telegram bot token. |
| `BASTION_LANE_CHAT_ID` | `Option<String>` | The operator's Telegram chat id LaneBot delivers to. |

**A third bot, not a reuse of either existing pair.** `bastion serve` already runs one
`getUpdates` long-poll per bot token — `NotifyPollLoop::run` for BastionBot (`telegram`) and
`SessionQaBridge::run_outbound` for CodeSessionsBot (`codesessions`) — and Telegram hands each
update to exactly one consumer. A CLI `notify ask` polling either of those tokens would
randomly steal the taps those loops exist to receive. LaneBot gives the CLI a stream nothing
else consumes. Read via `load_lane_bot_config` / the pure `lane_bot_config`, itself a thin
alias over the slug-generic `named_bot_config(slug, token_env, chat_env)` that also backs
`--bot <slug>` routing for `telegram` and `codesessions` — same both-or-neither truth table,
same `ConfigError::IncompleteTelegramConfig` error, reused rather than duplicated. LaneBot's
credentials are not provisioned yet, so both absent is the expected state today; `--bot lane`
is `bastion notify`'s default regardless. Same `BotToken` newtype, same redacted `Debug`.

## Precedence rules

An environment variable **always wins** over the config file for the same key.
The config file fills any gap the environment does not set.
Built-in defaults apply only when both the environment and file omit a value.

`DATABASE_URL` is the only required value — it must appear in at least one source.

## Public API (`src/config.rs`)

### `ConfigError`

| Variant | Description |
|---|---|
| `MissingVar(&'static str)` | Required env var not set. |
| `MalformedFile(String)` | Config file present but not valid TOML. |
| `UnknownWorkspace(String)` | Named workspace not found in the `[workspaces]` registry. |
| `NoWorkspaceRegistry` | `--workspace` used but no `[workspaces]` table exists in the config file. |
| `MissingServeToken` | `bastion serve` started without a bearer token (neither `--token` nor `BASTION_SERVE_TOKEN` set, or either resolved to an empty string). |
| `MalformedBudgetValue(&'static str, String, &'static str)` | A budget env var (`BASTION_MAX_TOTAL_TOKENS` or `BASTION_MAX_COST_USD`) was set but failed to parse as its expected numeric type. Carries the variable name, the offending value, and the expected type. Never silently defaults to "no cap". |
| `IncompleteTelegramConfig(&'static str)` | Exactly one of `BASTION_TELEGRAM_BOT_TOKEN` / `BASTION_TELEGRAM_CHAT_ID` was set (`BA.18.B`), **or** exactly one of `BASTION_CODESESSIONS_BOT_TOKEN` / `BASTION_CODESESSIONS_CHAT_ID` was set (`BA.20.C`), **or** exactly one of `BASTION_LANE_BOT_TOKEN` / `BASTION_LANE_CHAT_ID` was set (`BA.ticket.notify-operator-cli`, reusing this same variant via the slug-generic `named_bot_config`). Carries the name of the missing variable. Never silently treated as "transport/bridge/bot not configured" — that state is reserved for both absent. |

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
