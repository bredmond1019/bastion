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
```

All keys are optional. Unknown keys are ignored (forward-compatible).

## Precedence rules

An environment variable **always wins** over the config file for the same key.
The config file fills any gap the environment does not set.
Built-in defaults apply only when both the environment and file omit a value.

`DATABASE_URL` is the only required value — it must appear in at least one source.
