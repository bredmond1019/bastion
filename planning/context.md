---
type: LocalContext
title: bastion Project Context
description: Core context, governing principles, and documentation router for bastion.
---

# CONTEXT — bastion

> **Read this first.** Stable orientation for bastion: *why* this body of work
> exists, the rules that govern how it is built, and a router to the rest of `planning/`.
> This file orients; it does not track. For state, open `status.md`. For why choices were
> made, open `decisions/`.

## What This Project Is

`bastion` is a personal Rust CLI that serves as the unified control panel for the agentic engineering stack. Its primary feature is `bastion monitor` — a live TUI that renders workflow execution as a navigable graph: nodes colored by state (pending / running / success / failed), a detail pane showing inputs, outputs, errors, and token counts for the selected node, and a 2-second poll against the Python orchestrator's PostgreSQL database.

Secondary subcommands round out the tool: `bastion inspect` (post-mortem graph view for completed runs), `bastion costs` (LLM spend aggregation), `bastion validate` (markdown/MDX content validation), `bastion run` (trigger workflows via FastAPI), and `bastion status` (quick stack health check). Long-term it becomes the single terminal entry point — `bastion <verb>` — for operating the entire personal engineering stack.

## Who Is Building It

Brandon Redmond — solo agentic systems engineer. Rust background: `rag-engine-rs` (Actix/pgvector/streaming), `claude-sdk-rs` (typed async SDK, 149 tests), `workflow-engine-rs` (graph-validated AI workflow engine, 717 tests). This project applies that Rust experience to personal ops tooling rather than portfolio artifacts. The TUI and observability angle is new ground; the graph and async patterns are not.

## The Document Set

| File | Role | Volatility | Read it when… |
|---|---|---|---|
| **context.md** | Orientation + router (read first) | Stable | You need to understand the project or find the right file |
| **status.md** | Current progress | Volatile | You need to know what's done / what's next |
| **master-plan.md** | Strategy + phase specifications | Semi-stable | You need to understand the sequence of work |
| **harness.json** | Validation/UI-test config the SDLC engines read | Semi-stable | You're adapting the pipeline to this stack |
| **decisions/** | Architectural decisions (atomic, append-only) | Append-only | You want to check a prior architectural choice |
| **index.md** | Navigation index for `planning/` | Stable | You need a map of the planning folder |
| **log.md** (root) | Dated narrative of work completed | Append-only | You want the chronological dev history |

## The Project Sequence at a Glance

<!-- Phase names only, one line each. The sequence is load-bearing; details live in
     master-plan.md. -->

- **Phase 0 — Foundation** — clap scaffold, config, DB connection, `bastion status`
- **Phase 1 — `bastion monitor`** — live TUI graph inspector (the core feature)
- **Phase 2 — `bastion inspect` + `bastion costs`** — post-mortem view + spend reporting
- **Phase 3 — `bastion run` + `bastion validate`** — workflow trigger + content validation
- **Phase 4 — Polish** — SSE streaming, in-TUI node re-run, config file, man page

## Governing Principles

<!-- 6–8 numbered rules that govern how this project is built. At minimum keep the first
     three; add project-specific architectural rules. -->

1. **Tests ship with every block.** No block is "done" until its core functionality is covered by automated tests.
2. **Just-in-time scope.** Build what the current block needs, not a speculative future.
3. **Sequence, not calendar.** Work is ordered by dependency and competence, not by dates.
4. **Read from the orchestrator's DB, don't modify it.** `bastion` is an observer. Never write to the Python orchestrator's tables.
5. **`todo!()` stubs are the contract.** Every stub carries a phase label and a one-line description of what fills it. Don't implement ahead of the phase.

## Fast Facts

- **Destination:** Personal ops CLI — `bastion <verb>` as the single terminal entry point for the agentic stack
- **Type:** infrastructure
- **Tech stack:** Rust — `clap`, `ratatui`, `crossterm`, `petgraph`, `tokio`, `sqlx` (PostgreSQL), `reqwest`, `serde`/`serde_json`, `dotenvy`, `anyhow`, `thiserror`
- **Key constraints:** Read-only access to the Python orchestrator's PostgreSQL; start Phase 0 only after Project A ships and writes real execution state
- **Started:** 2026-06-18

---

*This file orients; it does not track. For state, open status.md. For why choices were made,
open decisions/.*
