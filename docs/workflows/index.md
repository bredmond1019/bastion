---
type: Index
title: Workflow Execution — Docs
description: Index of the bastion commands that trigger, watch, stop and cost out a workflow run.
doc_id: bastion-docs-workflows-index
layer: [console]
project: bastion
status: active
keywords: [workflows, execution, monitor, costs, index]
related: [bastion-cli-docs-index, commands, tuning, data-contract]
---

# Workflow Execution

The commands in this directory are about **one workflow run**: starting it, watching it, stopping
it, and finding out what it cost. All of them read the Postgres `events` table (`DATABASE_URL`),
except `status`, which exists to tell you whether that Postgres is even reachable.

Read [status.md](status.md) first when anything here fails to connect.

| File | What it covers |
|---|---|
| [run.md](run.md) | `bastion run` — trigger a workflow through the orchestrator's dispatcher, gated by the pre-dispatch budget check |
| [monitor.md](monitor.md) | `bastion monitor` — live two-pane TUI graph, or a headless `--watch` text loop |
| [inspect.md](inspect.md) | `bastion inspect` — static post-mortem graph of one finished run; one DB load, never re-queries |
| [abort.md](abort.md) | `bastion abort` — **destructive**; stop a running workflow via the Engine's abort endpoint |
| [costs.md](costs.md) | `bastion costs` — LLM token spend per workflow type, with budget-threshold alerts under `--watch` |
| [status.md](status.md) | `bastion status` — one-shot check that the database and the orchestrator API answer |

**Cross-cutting knobs** — poll cadence, budget ceilings, and the two API secrets are shared with
other surfaces and documented once in [tuning.md](../tuning.md).

**The row shape these read** is pinned in [data-contract.md](../data-contract.md).
