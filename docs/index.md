---
type: Index
title: docs Router
description: Navigation index for bastion's user-facing documentation.
---

# docs — Navigation

User-facing documentation for bastion. For internal strategy and progress, see `planning/`.

| Doc | Contents |
|---|---|
| [index.md](index.md) | This router |
| [sessions.md](sessions.md) | Session-control surface — TUI dashboard (`bastion` / `bastion tui`) + `sessions` / `attach` / `new` / `kill` / `send` / `capture` / `ask` verb reference + operator workflow |
| [monitor.md](monitor.md) | Live monitor surface — `bastion monitor`: two-pane graph TUI, keybindings, `--workflow-id` flag, poll cadence, degrade paths |
| [inspect.md](inspect.md) | Static post-mortem graph TUI — `bastion inspect <run-id>`: one-shot DB load, no polling, nodes colored by status |
| [claude-code-workflow.md](claude-code-workflow.md) | Hands-on guide — use bastion to open a tmux session, launch Claude Code in it, and drive it (attach vs. send/capture), including from a phone |
| [data-contract.md](data-contract.md) | Orchestrator field mappings — the execution state the monitor track reads (pinned to the orchestrator's contract) |

## Internal context (planning/)

| Doc | Contents |
|---|---|
| [../planning/context.md](../planning/context.md) | Orientation + governing principles (read first) |
| [../planning/master-plan.md](../planning/master-plan.md) | Phase/block strategy and specifications |
| [../planning/status.md](../planning/status.md) | Current progress |
| [../planning/decisions/index.md](../planning/decisions/index.md) | Architectural decisions log |
