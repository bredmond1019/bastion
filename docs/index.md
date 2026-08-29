---
type: Index
title: bastion CLI — Docs Index
description: Index of operator reference documentation for the bastion CLI tool — surfaces, verbs, configuration, and the observability/control layer.
doc_id: bastion-cli-docs-index
layer: [console]
project: bastion
status: active
keywords: [bastion, cli, operator reference, index, docs]
related: [commands, bastion-setup, monitor, brain, config, brainval, docview, workspace-contract, abort, assess, overview, momentum, status]
---

# bastion CLI — Docs

Operator reference for the `bastion` CLI. For how this binary fits the wider Bastion program —
and the **Bastion** (the system) vs **`bastion`** (this Console binary) naming split — see
`agentic-portfolio/core/docs/ownership.md`.

## Start here

| If you want to… | Read |
|---|---|
| see everything `bastion` can do, in one table | **[commands.md](commands.md)** — the capability catalogue |
| get it connected for the first time | [setup.md](setup.md) |
| find out why something is not working | [status.md](status.md), then [config.md](config.md) |

```bash
cargo install --path .   # from core/bastion/
bastion sessions         # a safe first command — no database needed
bastion --help           # the catalogue, from the binary
```

## Run and watch workflows

| File | What it covers |
|---|---|
| [run.md](run.md) | `bastion run` — trigger a workflow, gated by the pre-dispatch budget check |
| [monitor.md](monitor.md) | `bastion monitor` — live two-pane TUI graph of a running workflow |
| [inspect.md](inspect.md) | `bastion inspect` — static post-mortem graph of a finished run |
| [abort.md](abort.md) | `bastion abort` — stop a running workflow via the Engine's abort endpoint |
| [costs.md](costs.md) | `bastion costs` — LLM spend per workflow, with budget alerts |
| [status.md](status.md) | `bastion status` — one-shot check that the DB and API are reachable |

## Drive terminal sessions

| File | What it covers |
|---|---|
| [sessions.md](sessions.md) | The TUI dashboard and every tmux verb (`sessions`/`attach`/`new`/`kill`/`send`/`capture`/`ask`) |
| [claude-code-workflow.md](claude-code-workflow.md) | Walkthrough: launch Claude Code inside a session and drive it, including from a phone |
| [detect.md](detect.md) | The agent-state detection engine behind those Working/Idle/Blocked labels (library, not a subcommand) |

## See what is in flight

| File | What it covers |
|---|---|
| [overview.md](overview.md) | `bastion overview` — this repo's now/next/blocked queues as a Kanban TUI |
| [momentum.md](momentum.md) | `bastion momentum` — the same queues plus metrics, across every registered workspace |

## Query and validate the corpus

| File | What it covers |
|---|---|
| [brain.md](brain.md) | `bastion brain` — dependents / blast-radius / lineage over the OKF `[[link]]` graph |
| [code.md](code.md) | `bastion code` — symbol definition / references / dependents over Rust source |
| [validate.md](validate.md) | `bastion validate` — Markdown/MDX frontmatter and link validation |
| [assess.md](assess.md) | `bastion assess` — read-only coverage/readiness diagnostic, writes nothing |
| [brainval.md](brainval.md) | `bastion validate-brain` / `manifest` / `graph` / `emit-state` — the `mev` pass-throughs |
| [okf.md](okf.md) | `core/okf-core` — the shared frontmatter model, parser and serializer (library, not a subcommand) |
| [docview.md](docview.md) | `bastion view` / `edit` — the `bella` viewer pass-throughs |

## Serve and reach the operator

| File | What it covers |
|---|---|
| [serve-api.md](serve-api.md) | The pinned HTTP + WebSocket contract for `bastion serve` |
| [notify.md](notify.md) | `bastion notify send\|ask` — messages and gated questions over Telegram |

## Configure and operate

| File | What it covers |
|---|---|
| [setup.md](setup.md) | First-run setup: database provisioning, env vars, PATH |
| [config.md](config.md) | Every env var, config-file key, and the precedence between them |
| [observ.md](observ.md) | Error taxonomy (C001–C014), command events, logging init |

## Pinned contracts (consumer views)

Each pins the version bastion is built against and maps it to bastion's types. Editing one
without bumping its version is the failure they exist to prevent.

| File | Canonical owner |
|---|---|
| [data-contract.md](data-contract.md) | The orchestrator's `events` row contract |
| [carryover-contract.md](carryover-contract.md) | `mev`'s carryover triage ranking contract |
| [workspace-contract.md](workspace-contract.md) | The orchestrator's knowledge-workspace contract |
