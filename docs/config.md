---
type: Reference
title: Configuration
description: How bastion reads its configuration — env vars, config file, and built-in defaults.
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
```

All keys are optional. Unknown keys are ignored (forward-compatible).

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
