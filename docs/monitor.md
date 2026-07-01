---
type: Reference
title: Live Monitor Surface
description: Operator reference for `bastion monitor` — the live two-pane TUI graph view of workflow execution (keybindings, layout, flags, poll cadence, degrade paths).
doc_id: monitor
layer: [console]
project: bastion
status: active
keywords: [monitor, TUI, workflow graph, live poll, ratatui, node states, orchestrator]
related: [inspect, data-contract, sessions, costs]
---

# Live Monitor

`bastion monitor` is the live view of the agentic stack: it renders a running workflow as a
navigable graph that updates in place as each node executes. It reads the Python orchestrator's
PostgreSQL directly as a read-only observer (it never writes — bastion **D2**) and overlays the
DAG shape fetched from the orchestrator's graph endpoint. This is the workflow-observability
surface; for tmux/session control see [sessions.md](sessions.md).

> **Needs the orchestrator stack up.** `monitor` reads live execution state, so the orchestrator's
> Postgres must be reachable at `DATABASE_URL` and there must be a workflow running. Bring the
> stack up from the `python-orchestration-system/` repo: `./scripts/dev.sh` (START) /
> `./scripts/dev.sh stop` (STOP) — starts Postgres + Redis + FastAPI `:8080` + Celery. Then
> trigger a workflow so there is an active run to observe.

## Usage

```bash
bastion monitor                      # auto-pick the active run and watch it live
bastion monitor --workflow-id <id>   # monitor a specific run by its events.id
```

| Flag | Meaning |
|---|---|
| `--workflow-id <ID>` (`-w`) | Watch a specific run (its `events.id`). Omit to auto-pick from the active runs. |

## Layout

A two-pane TUI:

- **Left — graph pane.** The workflow DAG is rendered as a structured, indented tree format using Ratatui list primitives and box-drawing characters (`├─`, `└─`). Node color reflects live `RunStatus`:
  - **yellow** `~` — running
  - **green** `+` — success
  - **red** `!` — failed
  - **gray** `.` — pending
  The status symbol accompanies each node so state is legible without color; the selected node is
  highlighted.
- **Right — detail pane.** For the selected node, per the [data contract](data-contract.md) §6:
  status, timing (`started_at` / `elapsed_secs`), `error` (if any), `model`, token counts
  (`tokens_in` / `tokens_out`), and truncated `input` / `output`. When no node is selected, the
  run input is shown.

## Keybindings

| Key(s) | Action |
|---|---|
| `↓` / `j` | Select next node |
| `↑` / `k` | Select previous node |
| `→` / `n` | Next run (when multiple are active) |
| `←` / `p` | Previous run |
| `q` / `Esc` / `Ctrl-C` | Quit — restores the shell cleanly |

There is no manual-refresh key: the view refreshes automatically on the poll tick (see below).

## Poll cadence

The monitor re-fetches run state on a fixed interval, set by `BASTION_POLL_INTERVAL` (seconds;
default `2`). Because the orchestrator now persists execution state at every node boundary
(orchestrator **D28** — incremental `task_context.node_runs`), each node transition becomes
visible within one poll interval without any manual refresh. A transient DB error during a poll
is surfaced as a banner rather than killing the TUI — the next successful tick clears it.

## Degrade paths

`monitor` never panics on a missing or unreachable backend; it prints a clear message and exits
(or enters the TUI degraded) instead:

| Situation | Behavior |
|---|---|
| `DATABASE_URL` misconfigured / missing | Prints `configuration error` and exits. |
| Active-runs query fails (DB unreachable) | Prints the error + `Is the Python orchestrator stack running? (./scripts/dev.sh)` and exits. |
| `--workflow-id` fetch fails | Prints `failed to fetch run '<id>'` and exits. |
| No active workflow runs | Prints `no active workflow runs found.` + `Trigger a workflow first, then re-run` and exits. |
| Graph endpoint unreachable | Enters the TUI without an initial layout; retries on the first poll tick. |

## Related

- [inspect.md](inspect.md) — static post-mortem view of a completed run (no polling); use this
  after a run finishes.
- [data-contract.md](data-contract.md) — the orchestrator field mappings the monitor reads.
- [sessions.md](sessions.md) — the unified operator console, where the monitor is embedded within the **Mission Control** tab.
