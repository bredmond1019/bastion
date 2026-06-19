---
type: Plan
title: bastion Master Plan
description: Strategic roadmap and phase specifications for bastion.
---

# bastion — Master Plan

*Living document. Created 2026-06-18.*

## The Goal, Stated Plainly

`bastion` is a personal Rust CLI that makes the agentic engineering stack observable and operable from a single terminal command. The problem it solves: when a Python orchestrator workflow fails at node 7 of 12, you currently piece together what happened from Celery logs, Redis state, and raw SQL across three terminal panes. `bastion monitor` collapses that into one command — a live graph where nodes go green or red in real time, and the selected node shows its full input, output, error trace, and token count in a side pane.

"Ready" means: `bastion monitor` works against the live Python orchestrator, showing at least two distinct workflow types as navigable TUI graphs with accurate real-time state. Secondary commands (`inspect`, `costs`, `run`, `validate`, `status`) are functional at whatever phase they ship.

## The Destination

A single binary — `bastion` — that is the terminal entry point for the entire personal engineering stack. You open one pane, run one command family, and know what your system is doing. Longer term: a credible example of custom observability tooling you can describe to engineering clients.

## Architecture / Design Overview

`bastion` reads from the **Python orchestrator's PostgreSQL** directly (read-only; no changes to the Python side). The TUI uses `ratatui` + `crossterm` for rendering and event handling. `petgraph` manages the DAG structure and topological layout. `tokio` drives the async event loop (DB poll + keyboard events). `reqwest` handles FastAPI calls for `bastion run` and future node re-run.

```
src/
├── main.rs          clap dispatch → subcommand modules
├── cli.rs           all subcommand + flag definitions
├── config.rs        DATABASE_URL / BASTION_API_URL from env
├── db/
│   ├── workflows.rs  queries: active runs, node states, inputs/outputs
│   └── costs.rs      queries: token usage aggregation
├── api/client.rs     reqwest: trigger runs, re-run nodes, health check
├── monitor/          live TUI (ratatui loop, petgraph layout, crossterm events)
│   ├── app.rs        state: selected run/node, should_quit
│   ├── graph.rs      WorkflowRun → petgraph DAG → grid positions
│   ├── ui.rs         ratatui render: two-pane layout
│   └── events.rs     tokio loop: keyboard + DB poll interval
├── inspect/          static post-mortem view (monitor minus polling)
├── validate/         markdown/MDX validation (mirrors markdown-engine-validator)
├── costs/            LLM spend summary (tabular stdout)
└── run/              workflow trigger (FastAPI) + stack status check
```

---

## Phase 0 — Foundation

### Block A — Foundation setup
- **What:** Verify the Rust toolchain compiles the scaffolded project. Implement `bastion status` end-to-end: connect to PostgreSQL and the FastAPI health endpoint, print a summary of what's reachable. Add a `.env.example`.
- **Why:** Proves the DB connection and HTTP client work before any TUI work starts. Useful as a pre-flight check from day one.
- **Build notes:** `config.rs` reads `DATABASE_URL` and `BASTION_API_URL` from env. `run::status()` calls `api::client::ApiClient::health()` and a test PostgreSQL query. Print a formatted table: `DB ✓`, `API ✓`, worker count, queue depth (or `unreachable` per service).
- **Acceptance criteria:** `cargo build` passes. `cargo test` passes. `bastion status` prints real health data against the running Python orchestrator.

---

## Phase 1 — `bastion monitor`

### Block A — DB queries + graph layout
- **What:** Implement `db::workflows` queries (list active runs, get run state, get node inputs/outputs/errors). Build `monitor::graph` — convert `WorkflowRun` into a `petgraph` DAG and compute a topological grid layout.
- **Why:** The data layer must be solid before any TUI rendering. Layout bugs are easier to debug in unit tests than inside a live TUI.
- **Acceptance criteria:** Unit tests cover graph construction, topological ordering, and position assignment. Queries return correct data against a real run.

### Block B — TUI render loop
- **What:** Implement `monitor::ui` (two-pane ratatui layout) and `monitor::events` (tokio loop with keyboard + DB poll). Wire through `monitor::app` state. `bastion monitor` enters the TUI and displays live workflow state.
- **Why:** The core deliverable.
- **Acceptance criteria:** `bastion monitor` renders a running workflow as a live graph. Arrow-key navigation moves the selected node. State updates within the poll interval. `q` exits cleanly.

---

## Phase 2 — Inspect + Costs

### Block A — `bastion inspect`
- **What:** Reuse monitor graph/UI code with polling disabled. Load a completed run by ID from PostgreSQL and render it as a static navigable graph.
- **Acceptance criteria:** `bastion inspect <run-id>` renders any completed run. Navigation works. Exiting returns to the shell cleanly.

### Block B — `bastion costs`
- **What:** Implement `db::costs` aggregation queries. `bastion costs --last 7d` prints a formatted table of workflow names, run counts, token totals, and estimated USD cost.
- **Acceptance criteria:** Output matches manual SQL queries against the same data. Handles `7d`, `30d`, `all` windows.

---

## Phase 3 — Run + Validate

### Block A — `bastion run`
- **What:** Implement `api::client::trigger_workflow`. `bastion run <workflow> [--args '{}'] [--monitor]` POSTs to FastAPI, prints the run ID, optionally drops into `bastion monitor` for that run.
- **Acceptance criteria:** Successfully triggers a workflow. `--monitor` flag works.

### Block B — `bastion validate`
- **What:** Port or shell-out to `markdown-engine-validator` logic. Scan a content directory, validate frontmatter, check links, report errors with file + line.
- **Acceptance criteria:** Detects known-bad frontmatter and broken links in test fixtures.

---

## Phase 4 — Polish

- SSE streaming from FastAPI instead of DB polling (if the Python side exposes it)
- Node re-run from TUI (`r` key → `api::client::rerun_node`)
- `~/.config/bastion/config.toml` support so DB URL isn't always an env var
- `bastion help` improvements; man page

---

## Quick Reference Sequence Table

| Phase | Block | What | Why | Role in destination |
|---|---|---|---|---|
| 0 | A | Scaffold + `bastion status` | DB/API connection validated | Prerequisite for everything |
| 1 | A | DB queries + graph layout | Data layer before TUI | Enables render loop |
| 1 | B | TUI render loop | Core feature | The primary deliverable |
| 2 | A | `bastion inspect` | Post-mortem graph view | Completes the monitoring story |
| 2 | B | `bastion costs` | LLM spend tracking | Operational awareness |
| 3 | A | `bastion run` | Workflow trigger | Closes the control loop |
| 3 | B | `bastion validate` | Content validation | Unifies the Rust tool surface |
| 4 | — | Polish | SSE, re-run, config, man page | Production-quality tooling |

---

*Sequenced by dependency and competence, not calendar. When life gets in the way, pick up
where you left off.*
