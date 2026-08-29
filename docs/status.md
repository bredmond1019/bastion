---
type: Reference
title: status — Stack Health Check
description: Reference for `bastion status` — a one-shot, non-TUI check of whether the Postgres events database and the orchestrator API are reachable.
doc_id: status
layer: [console]
project: bastion
status: active
keywords: [status, health check, reachability, postgres, orchestrator api, degradation]
related: [bastion-setup, run, monitor, commands]
---

# status — Stack Health Check

`bastion status` is the first thing to run when something else is not working. It probes the two
dependencies the observability commands need — the Postgres holding the `events` table, and the
orchestrator's HTTP API — and prints one line for each. It is a one-shot plain-text command:
no TUI, no polling, no writes.

## Quickstart

```bash
bastion status
```

```
Stack health
DB    reachable
API   reachable (status=ok, version=1.4.2)
```

Nothing needs to exist first. `status` is deliberately runnable in a broken environment — that
is the point of it. Missing configuration is reported as a result, not as a crash.

## What it probes

| Row | What is checked | Config it reads |
|---|---|---|
| `DB` | A connection to the Postgres `events` database. | `DATABASE_URL` |
| `API` | `GET /health` on the orchestrator's FastAPI. | `BASTION_API_URL`, defaulting to `http://localhost:8080` |

Both values come from the normal precedence chain — env var, then
`~/.config/bastion/config.toml`, then the built-in default. See [config.md](config.md).

## Reading the output

| Line | Meaning | What to do |
|---|---|---|
| `DB    reachable` | Postgres answered. | — |
| `DB    unreachable (DATABASE_URL not set)` | No database URL configured anywhere. **Not an error** — `status` treats `DATABASE_URL` as optional so it can still tell you about the API. | Set `DATABASE_URL`; see [setup.md](setup.md). |
| `DB    unreachable (<connection error>)` | Configured, but the connection failed. | Check the Postgres is up and the URL is right. |
| `API   reachable (status=…, version=…)` | The orchestrator answered its health route, and reported these. | — |
| `API   unreachable (<error>)` | Nothing answering at `BASTION_API_URL`. | Start the orchestrator stack; see [run.md](run.md). |

Any *other* configuration error — a malformed budget cap, for example — is fatal and exits
non-zero with the message, rather than being rendered as a row.

## See also

- [setup.md](setup.md) — provisioning the database and configuring bastion for the first time.
- [run.md](run.md) — triggering a workflow once the stack is healthy.
- [monitor.md](monitor.md) — watching one run live.
- [commands.md](commands.md) — every bastion subcommand in one table.
