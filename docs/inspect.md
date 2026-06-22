---
type: Reference
title: Static Inspect Surface
description: Operator reference for `bastion inspect <run-id>` — the static post-mortem TUI graph view of a completed (or in-progress snapshot) workflow run.
---

# Static Inspect

`bastion inspect <run-id>` renders a completed (or in-progress snapshot) workflow run as a
static, navigable graph TUI. It reuses the same two-pane layout and node-coloring logic as
`bastion monitor`, but performs **exactly one DB load** and never re-queries — there is no poll
interval and no background ticker. Use it for post-mortem analysis of a run that has already
finished. For live observation of an active run, see [monitor.md](monitor.md).

> **Needs the orchestrator stack up.** `inspect` reads the PostgreSQL `events` table and
> optionally the graph endpoint, so `DATABASE_URL` must be set and the orchestrator's Postgres
> must be reachable. Bring the stack up from the `python-orchestration-system/` repo:
> `./scripts/dev.sh` (START) / `./scripts/dev.sh stop` (STOP).

## Usage

```bash
bastion inspect <run-id>   # render a specific run (its events.id) as a static graph
```

| Argument | Meaning |
|---|---|
| `<run-id>` | The `events.id` of the run to inspect. Accepts any run status (completed, failed, or still-active snapshot). |

## Layout

Same two-pane TUI as `monitor`:

- **Left — graph pane.** Nodes placed on a topological left-to-right grid; color reflects the
  recorded `RunStatus` at the time of load:
  - **yellow** `~` — running
  - **green** `+` — success
  - **red** `!` — failed
  - **gray** `.` — pending
- **Right — detail pane.** For the selected node: status, timing, error (if any), model, token
  counts, and truncated input/output. When no node is selected, the run input is shown.

If the graph endpoint is unreachable, nodes are rendered without edges (non-fatal degradation).

## Keybindings

| Key(s) | Action |
|---|---|
| `↓` / `j` | Select next node |
| `↑` / `k` | Select previous node |
| `q` / `Esc` / `Ctrl-C` | Quit — restores the shell cleanly |

`→` / `←` / `n` / `p` (run switching) are inherited from the shared event handler but are
no-ops in inspect mode since only one run is loaded.

## Degrade paths

`inspect` never panics; it prints a clear message and exits instead:

| Situation | Behavior |
|---|---|
| `DATABASE_URL` misconfigured / missing | Prints `configuration error` + hint to set the env var and exits. |
| Run ID not found or DB unreachable | Prints `no run found for '<id>'` + orchestrator-stack hint and exits. |
| Graph endpoint unreachable | Prints `could not fetch workflow graph` + `Rendering nodes without edges.`; continues into the TUI without edge layout. |

## Key internals

| Symbol | Role |
|---|---|
| `build_inspect_app(run, graph)` | Pure function: builds the shared `App` with one run loaded and an optional `GraphLayout`. No I/O. Exhaustively unit-tested (9 cases). |
| `run_static_loop(app)` | Static event loop: draw → block on key → repeat; no poll timer. Restores the terminal on exit (best-effort). |
| `run(run_id)` | Async entry point. Loads config, fetches the run and graph, builds the app, enters `run_static_loop`. |

## Related

- [monitor.md](monitor.md) — live polling view of an active workflow run.
- [data-contract.md](data-contract.md) — the orchestrator field mappings both surfaces read.
- [sessions.md](sessions.md) — tmux session control surface.
