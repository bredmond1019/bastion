---
type: Index
title: Planning Boards — Docs
description: Index of bastion's read-only planning views — one repo's Kanban board and the cross-repo momentum rollup.
doc_id: bastion-docs-boards-index
layer: [console]
project: bastion
status: active
keywords: [boards, kanban, momentum, planning, index]
related: [bastion-cli-docs-index, commands, tuning]
---

# Planning Boards

Two commands that answer "what am I working on" by reading planning files already on disk. Both
are **read-only** — `/log-work` and `mev emit-state` write those files; bastion only renders them
(bastion decision D25). Neither needs a database.

The split is scope: one repo, or all of them.

| File | What it covers |
|---|---|
| [overview.md](overview.md) | `bastion overview` — **this** repo's `now` / `next` / `blocked` queues as a Kanban TUI, from `planning/state.json` |
| [momentum.md](momentum.md) | `bastion momentum` — the same queues plus `## Metrics`, across **every** registered workspace, from each `planning/status.md` |

**Which repos `momentum` sees** is the workspace registry — documented once in
[tuning.md](../tuning.md#corpus-and-workspace-roots).
