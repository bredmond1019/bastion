---
type: Index
title: docs Router
description: Navigation index for bastion's user-facing documentation.
doc_id: docs-index
layer: [console]
project: bastion
status: active
keywords: [documentation, router, index, bastion docs, SDLC workflows]
related: [sessions, monitor, data-contract, config, brain, serve-api, validate]
---

# docs — Navigation

User-facing documentation for bastion. For internal strategy and progress, see `planning/`.

| Doc | Contents |
|---|---|
| [index.md](index.md) | This router |
| [sessions.md](sessions.md) | Session-control surface — TUI dashboard (`bastion` / `bastion tui`) + `sessions` / `attach` / `new` / `kill` / `send` / `capture` / `ask` verb reference + operator workflow |
| [monitor.md](monitor.md) | Live monitor surface — `bastion monitor`: two-pane graph TUI, keybindings, `--workflow-id` flag, poll cadence, degrade paths |
| [inspect.md](inspect.md) | Static post-mortem graph TUI — `bastion inspect <run-id>`: one-shot DB load, no polling, nodes colored by status |
| [costs.md](costs.md) | LLM spend summary — `bastion costs --last <window>`: per-workflow token totals and estimated USD cost for `7d`, `30d`, or `all` |
| [claude-code-workflow.md](claude-code-workflow.md) | Hands-on guide — use bastion to open a tmux session, launch Claude Code in it, and drive it (attach vs. send/capture), including from a phone |
| [run.md](run.md) | Workflow trigger — `bastion run <workflow> [--args '{}'] [--monitor]`: POST to orchestrator, print `task_id`, optional monitor hand-off |
| [validate.md](validate.md) | Content validation — `bastion validate <path>`: file discovery rules, `ValidationError`/`ErrorKind` types, submodule contracts, exit behaviour |
| [data-contract.md](data-contract.md) | Orchestrator field mappings — the execution state the monitor track reads (pinned to the orchestrator's contract) |
| [config.md](config.md) | Configuration reference — env vars, `~/.config/bastion/config.toml` format, and precedence rules |
| [observ.md](observ.md) | Observability module — C001–C014 error taxonomy, `CommandEvent` structured events, `emit_start`/`emit_outcome` tracing helpers, `classify_error()`, and `init_tracing` |
| [brain.md](brain.md) | OKF knowledge-graph queries — `bastion brain`: corpus discovery, `--dependents` / `--blast-radius` / `--lineage` modes, output format, degradation paths |
| [code.md](code.md) | Symbol-level code graph queries — `bastion code`: tree-sitter extraction, `--def` / `--refs` / `--dependents` modes, output format, degradation paths |
| [serve-api.md](serve-api.md) | HTTP + WebSocket API contract v0 — `bastion serve`: base URL, bearer-auth scheme, `GET /health`, `/ws` echo, and the frame envelope that `bastion-ui` pins against |

## SDLC workflows (docs/workflows/)

| Doc | Contents |
|---|---|
| [workflows/index.md](workflows/index.md) | Engine ladder overview + committed-state model |
| [workflows/sdlc-run.md](workflows/sdlc-run.md) | `sdlc-run` — full spec, in-place on main |
| [workflows/sdlc-task.md](workflows/sdlc-task.md) | `sdlc-task` — lean single-unit implement→test→fix→commit |
| [workflows/sdlc-flow.md](workflows/sdlc-flow.md) | `sdlc-flow` — shared worktree, per-task loop, one PR |
| [workflows/sdlc-block.md](workflows/sdlc-block.md) | `sdlc-block` — block-level roadmap orchestrator, branch train |
| [workflows/commands.md](workflows/commands.md) | Ad-hoc planning + utility commands reference |

## Internal context (planning/)

| Doc | Contents |
|---|---|
| [../planning/context.md](../planning/context.md) | Orientation + governing principles (read first) |
| [../planning/master-plan.md](../planning/master-plan.md) | Phase/block strategy and specifications |
| [../planning/status.md](../planning/status.md) | Current progress |
| [../planning/decisions/index.md](../planning/decisions/index.md) | Architectural decisions log |
