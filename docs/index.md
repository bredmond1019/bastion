---
type: Index
title: bastion CLI — Docs Index
description: Index of operator reference documentation for the bastion CLI tool — surfaces, verbs, configuration, and the observability/control layer.
doc_id: bastion-cli-docs-index
layer: [console]
project: bastion
status: active
keywords: [bastion, cli, operator reference, index, docs]
related: [bastion-product-ownership, bastion-product-architecture]
---

# bastion CLI — Docs

Operator reference for the `bastion` CLI tool. For system architecture and naming conventions,
see [`Bastion/docs/architecture.md`](file://~/agentic-portfolio)
and [`Bastion/docs/ownership.md`](file://~/agentic-portfolio).

## Surfaces

| File | What it covers |
|---|---|
| [monitor.md](monitor.md) | Live two-pane TUI graph view of workflow execution |
| [inspect.md](inspect.md) | Static post-mortem TUI graph view of a completed run |
| [run.md](run.md) | Trigger a workflow via the FastAPI generic dispatcher |
| [sessions.md](sessions.md) | tmux session-control verbs (list / attach / new / kill / send / capture) |
| [detect.md](detect.md) | Pure agent-state detection engine — TOML manifest schema, gate types, `detect()` API |
| [claude-code-workflow.md](claude-code-workflow.md) | Hands-on walkthrough: spin up a tmux session, launch Claude Code, drive it |

## Knowledge graph

| File | What it covers |
|---|---|
| [brain.md](brain.md) | `bastion brain` — OKF corpus discovery, graph construction, structural queries |
| [code.md](code.md) | `bastion code` — tree-sitter symbol/reference/dependents lookup over Rust source |
| [validate.md](validate.md) | `bastion validate` — Markdown/MDX content validation |

## Infrastructure

| File | What it covers |
|---|---|
| [config.md](config.md) | Configuration: env vars, config file, built-in defaults |
| [costs.md](costs.md) | `bastion costs` — LLM spend summary surface |
| [observ.md](observ.md) | Structured error taxonomy (C001-C014), event tracing, logging init |
| [serve-api.md](serve-api.md) | HTTP + WebSocket API contract for `bastion serve` (v0.1) |
| [data-contract.md](data-contract.md) | bastion's pinned view of the orchestrator data contract |
