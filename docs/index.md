---
type: Index
title: bastion CLI — Docs Index
description: Master index of the bastion CLI's operator documentation — grouped into six domain directories plus the capability catalogue, the cross-cutting tuning page, and the pinned consumer contracts.
doc_id: bastion-cli-docs-index
layer: [console]
project: bastion
status: active
keywords: [bastion, cli, operator reference, index, docs]
related: [commands, tuning, bastion-docs-workflows-index, bastion-docs-terminal-index, bastion-docs-boards-index, bastion-docs-knowledge-index, bastion-docs-serve-index, bastion-docs-operations-index, data-contract, carryover-contract, workspace-contract, brain-graph-output]
---

# bastion CLI — Docs

Operator reference for the `bastion` CLI. For how this binary fits the wider Bastion program —
and the **Bastion** (the system) vs **`bastion`** (this Console binary) naming split — see
`agentic-portfolio/core/docs/ownership.md`.

## Start here

| If you want to… | Read |
|---|---|
| see everything `bastion` can do, in one table | **[commands.md](commands.md)** — the capability catalogue |
| get it connected for the first time | [operations/setup.md](operations/setup.md) |
| find out why something is not working | [workflows/status.md](workflows/status.md), then [operations/config.md](operations/config.md) |
| change a knob and know what else it affects | [tuning.md](tuning.md) |

```bash
cargo install --path .   # from core/bastion/
bastion sessions         # a safe first command — no database needed
bastion --help           # the catalogue, from the binary
```

## The six domains

Each directory has its own `index.md` explaining what unifies it.

| Directory | What lives there |
|---|---|
| [workflows/](workflows/index.md) | One workflow run: trigger it, watch it, stop it, cost it out — `run` · `monitor` · `inspect` · `abort` · `costs` · `status` |
| [terminal/](terminal/index.md) | tmux session control and the Claude Code workflow — database-free, works with the stack down |
| [boards/](boards/index.md) | Read-only planning views — one repo's Kanban board, and the cross-repo momentum rollup |
| [knowledge/](knowledge/index.md) | Structural queries and validation over docs and Rust source — `brain` · `code` · `validate` · `assess` · the `mev` and `bella` pass-throughs |
| [serve/](serve/index.md) | The outward-facing surfaces — the `bastion serve` HTTP/WebSocket contract, and reaching the operator over Telegram |
| [operations/](operations/index.md) | Getting it running and reading what it says — setup, the full config reference, the error/logging spine |

## Two pages that cut across all six

| File | What it covers |
|---|---|
| [commands.md](commands.md) | **The capability catalogue** — every subcommand, derived from the code's dispatch table rather than from this index, so a command with no doc still appears |
| [tuning.md](tuning.md) | **How to tune it** — corpus roots, poll cadence, budget ceilings, the three secrets, logging, and the build-drift guard. Each mechanism is shared by several commands and is explained once here rather than inside whichever feature introduced it |

## Pinned contracts (consumer views)

These stay at the root of `docs/` on purpose: every consuming repo in the fleet carries
`docs/data-contract.md` and `docs/workspace-contract.md` at that exact path, and HQ's update
protocol names those paths when a canonical contract bumps.

Each pins the version bastion is built against and maps it to bastion's types. **Editing one
without bumping its version is the failure they exist to prevent.**

| File | Canonical owner |
|---|---|
| [data-contract.md](data-contract.md) | The orchestrator's `events` row contract |
| [carryover-contract.md](carryover-contract.md) | `mev`'s carryover triage ranking contract |
| [workspace-contract.md](workspace-contract.md) | The orchestrator's knowledge-workspace contract |

## Pinned contracts (bastion-owned)

Unlike the three above, this one is authored and owned *by this repo* — it is the contract a
consuming skill or script pins against, not a view of someone else's.

| File | Consumed by |
|---|---|
| [brain-graph-output.md](brain-graph-output.md) | The `bastion brain --json` / `bastion code --json` output shape — `base-template`'s `brain-graph` skill (block `BT.3.F`) |
