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

`bastion` is an **observer, never a writer** of the Python orchestrator. It reconstructs a live run
by merging **two sources**, joined on **node class name** (the identity key):

1. **DAG shape** — `GET /workflows/{type}/graph` (FastAPI) → `{nodes, edges}`. Fetched once per
   workflow type; this is the *only* source of edges and of nodes that haven't run yet.
2. **Live per-node state** — the orchestrator's **PostgreSQL** `events.task_context.node_runs`
   (read-only, polled). Every node is present as `pending` from the first write, then transitions
   `running → success|failed` with timing, token usage, input, and errors.

There are **no** relational `workflow_runs` / `node_states` tables — all run state is JSON in the
`events` table. The contract for all of this (table shape, `node_runs` fields, endpoints, status
strings) is the orchestrator-owned, versioned
[data contract](../docs/data-contract.md); bastion **pins** a version of it.

Read path is **Hybrid**: direct Postgres for the live poll now; a reserved orchestrator HTTP read
API (`GET /events/{id}`) is documented for later but not depended on. The TUI uses `ratatui` +
`crossterm` for rendering and event handling. `petgraph` manages the DAG structure and topological
layout. `tokio` drives the async event loop (DB poll + keyboard events). `reqwest` handles FastAPI
calls — the graph endpoint, `bastion run`, and future node re-run.

```
src/
├── main.rs          clap dispatch → subcommand modules
├── cli.rs           all subcommand + flag definitions
├── config.rs        DATABASE_URL / BASTION_API_URL from env
├── db/
│   ├── workflows.rs  parse events.task_context: active runs, node_runs, outputs
│   └── costs.rs      aggregate node_runs[*].usage token totals
├── api/client.rs     reqwest: workflow_graph (DAG), trigger run, health check
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
- **Build notes:** `config.rs` reads `DATABASE_URL` and `BASTION_API_URL` from env. `run::status()` calls `api::client::ApiClient::health()` and a test PostgreSQL query. Print a formatted table: `DB ✓`, `API ✓` (or `unreachable` per service). Worker count / queue depth live in Redis, which is out of bastion's configured scope — **scoped out** of `status` (see D2).
- **Acceptance criteria:** `cargo build` passes. `cargo test` passes. `bastion status` prints real health data against the running Python orchestrator.

---

## Phase 1 — `bastion monitor`

### Block A — DB queries + graph layout
- **What:** Implement `db::workflows` against the **`events` table** (not relational tables):
  list active runs (rows whose `node_runs` aren't all terminal), and parse one row's
  `task_context` into per-node state (`node_runs[name]` → status/timing/error/input/usage;
  `nodes[name]` → output). Add `api::client::workflow_graph(type)` for the DAG `{nodes, edges}`.
  Build `monitor::graph` — construct a `petgraph` DAG from the **API edges**, overlay live node
  state by **class-name join**, and compute a topological grid layout.
- **Why:** The data layer must be solid before any TUI rendering. Layout bugs are easier to debug
  in unit tests than inside a live TUI. Edges come from the API; status comes from the DB — keep
  the two sources explicit (see [data contract](../docs/data-contract.md) §2).
- **Acceptance criteria:** Unit tests cover the `node_runs` JSON → state parse (against a captured
  fixture), the graph-endpoint → edges parse, the class-name join, topological ordering, and
  position assignment. Status strings deserialize from `pending|running|success|failed`.

### Block B — TUI render loop
- **What:** Implement `monitor::ui` (two-pane ratatui layout) and `monitor::events` (tokio loop with keyboard + DB poll). Wire through `monitor::app` state. `bastion monitor` (no arg → auto-pick the active run) enters the TUI and displays live workflow state. Detail pane reads, per the [data contract](../docs/data-contract.md) §6: status/timing/error/input/tokens from `node_runs[name]`, output from `nodes[name]`, run input from `events.data`.
- **Why:** The core deliverable.
- **Acceptance criteria:** `bastion monitor` renders a running workflow as a live graph. Arrow-key navigation moves the selected node. State updates within the poll interval (the orchestrator persists at every node boundary, so each transition is observable). `q` exits cleanly.

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
- **What:** Implement `api::client::trigger_workflow`. `bastion run <workflow> [--args '{}'] [--monitor]` issues `POST /` with `{workflow_type, data}` (the orchestrator's generic dispatcher — see [data contract](../docs/data-contract.md) §7), prints the returned `task_id`, optionally drops into `bastion monitor` for that run.
- **Acceptance criteria:** Successfully triggers a workflow. `--monitor` flag works.

### Block B — `bastion validate`
- **What:** Port or shell-out to `markdown-engine-validator` logic. Scan a content directory, validate frontmatter, check links, report errors with file + line.
- **Acceptance criteria:** Detects known-bad frontmatter and broken links in test fixtures.

---

## Phase 4 — Polish

- SSE streaming from FastAPI instead of DB polling (orchestrator plan Phase 5 — the `on_progress` seam is reserved for it; not built yet)
- Node re-run from TUI (`r` key → `api::client::rerun_node`) — **requires new orchestrator support** (no re-run endpoint exists today; would be a contract addition)
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
