---
type: Reference
title: overview — Kanban Board for This Repo
description: Reference for `bastion overview` — a read-only Kanban TUI over this repo's planning/state.json focus queues (now / next / blocked).
doc_id: overview
layer: [console]
project: bastion
status: active
keywords: [overview, kanban, state.json, focus queues, TUI, planning]
related: [momentum, commands, config]
---

# overview — Kanban Board for This Repo

`bastion overview` shows one repo's work as a three-column Kanban board in the terminal:
**In Progress**, **Up Next**, **Blocked**. It reads a single file — `planning/state.json` — and
writes nothing back. It is the "what am I on right now" board; for the same view across every
repo at once, use [`bastion momentum`](momentum.md).

## Quickstart

```bash
cd core/bastion      # or any repo that has a planning/state.json
bastion overview     # opens the board; press q to quit
```

| Must exist first | If it is missing |
|---|---|
| A `planning/` directory found by walking up from the current directory | The command exits with `Failed to read ".../state.json"` — `cd` into a repo that has one, or set `BASTION_PLANNING_ROOT`. |
| `state.json` inside it, with `repo`, `updated`, and a `focus` object | The command exits with `Failed to parse state.json`. The file is generated — regenerate it with `bastion emit-state --write` rather than hand-editing. |

`BASTION_PLANNING_ROOT` overrides the walk-up search and points at a planning directory
directly. See [config.md](config.md#environment-variables).

## What it reads

Exactly four fields of [`state.json`](brainval.md). Everything else in the file is ignored.

| Field | Used for |
|---|---|
| `repo` | The header line. |
| `updated` | The header line's `(updated …)` stamp. |
| `focus.now[]` | The **In Progress** column. |
| `focus.next[]` | The **Up Next** column. |
| `focus.blocked[]` | The **Blocked** column. |

Each entry in those three arrays renders as two lines — its `id` in accent colour, then its
`title` indented beneath. An entry's optional `repo` field is parsed but not displayed here;
that is what the cross-repo [`momentum`](momentum.md) view is for.

## Keys

| Key | Action |
|---|---|
| `q` | Quit and restore the terminal. |

There is no scrolling, no selection, and no editing. The board is a snapshot taken when the
command starts — it does not poll, so re-run it after `emit-state` to see new work.

## Read-only by design

`focus.now` / `next` / `blocked` are **derived** fields. `/log-work` and `mev emit-state` write
them; bastion only reads them (brain decision **D25** — "bastion triggers, the Engine executes").
Do not treat this board as a place to change state.

## See also

- [momentum.md](momentum.md) — the same queues across every registered workspace.
- [brainval.md](brainval.md) — `bastion emit-state`, which generates `state.json`'s derived fields.
- [commands.md](commands.md) — every bastion subcommand in one table.
