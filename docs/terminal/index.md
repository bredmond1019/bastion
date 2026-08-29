---
type: Index
title: Terminal Sessions — Docs
description: Index of bastion's tmux session-control surface — the verbs, the Claude Code workflow, and the agent-state detection engine behind the Working/Idle/Blocked labels.
doc_id: bastion-docs-terminal-index
layer: [console]
project: bastion
status: active
keywords: [tmux, sessions, claude code, agent state, index]
related: [bastion-cli-docs-index, commands, tuning]
---

# Terminal Sessions

This surface manages the long-running tmux sessions that hold Claude Code and other persistent
work. It is **database-free by design** (bastion decision D4): every verb here works with the
orchestrator stack down, Postgres stopped, and no network — which is why it is the surface you
reach for from a phone over SSH.

bastion *manages* these sessions. It does not run Claude Code itself; tmux holds the process.

| File | What it covers |
|---|---|
| [sessions.md](sessions.md) | The TUI dashboard and every verb: `sessions` · `attach` · `new` · `kill` · `send` · `capture` · `ask` |
| [claude-code-workflow.md](claude-code-workflow.md) | Walkthrough: launch Claude Code inside a session and drive it, including from a phone |
| [detect.md](detect.md) | The agent-state detection engine behind the Working / Idle / Blocked labels — a library surface, **not** a subcommand |

**Cross-cutting knobs** — log verbosity and the degradation posture these verbs share are in
[tuning.md](../tuning.md).
