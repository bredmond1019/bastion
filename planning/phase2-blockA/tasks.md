---
type: TaskSpec
title: Phase 2 Block A — bastion inspect
description: Static post-mortem graph view — load a completed run by ID and render it as a static navigable graph by reusing the monitor TUI with polling disabled.
---

# Task Spec — Phase 2, Block A

## Goal
Reuse the monitor graph/UI code with polling disabled: load a completed run by ID from PostgreSQL and render it as a static navigable graph (`bastion inspect <run-id>`).

## Context Pointers
- **Plan:** `planning/master-plan.md` Phase 2 → Block A (`bastion inspect`, master-plan.md:115–117) and the Architecture overview (the two-source read model: DAG edges from `GET /workflows/{type}/graph`, live state from `events.task_context.node_runs`).
- **Reuse, do not re-implement** (per the consumed handoff): the difference from `monitor` is *purely the absence of the poll loop*. Load once, render, navigate, exit.
  - `db::workflows::get_run_state(db_url, run_id)` — already loads any run by `events.id` regardless of status (`src/db/workflows.rs:46`). No new query needed.
  - `monitor::graph::build_layout(&graph, &run.nodes)` — DAG layout (`src/monitor/graph.rs`).
  - `monitor::ui::render(frame, app)` — two-pane render; pure helpers `status_color` / `status_symbol` / `format_node_detail` / `build_graph_lines` (`src/monitor/ui.rs`).
  - `monitor::app::App` — state model + clamped node/run navigation (`src/monitor/app.rs`).
  - `api::client::ApiClient::workflow_graph(type)` — DAG `{nodes, edges}` (`src/api/client.rs`).
- **The inspect surface lives in `src/inspect/mod.rs`** — currently a `todo!()` stub. CLI is already wired: `Commands::Inspect { run_id }` (`src/cli.rs:24`) and `main.rs:34` already call `inspect::run(run_id).await`. No CLI changes needed.
- **Track / decisions:** inspect is on the **gated Postgres (monitor) track**, so **async/tokio is allowed** here (D5's synchronous-no-tokio rule applies only to `sessions/`). It is an **observer** — read-only against the orchestrator's Postgres (D2). D2 gate is lifted (orchestrator D28 landed).
- **Standing rules:** `CLAUDE.md` — Rule 1 (tests ship with every block), Rule 6 (Coverage bar: pure logic exhaustively unit-tested without I/O; thin I/O shell smoke-tested and recorded in `## Notes`).

## Step-by-Step Tasks

### 1. Expose the shared TUI shell so inspect reuses it instead of re-implementing it
- In `src/monitor/events.rs`, widen the visibility of the three pieces inspect needs to **`pub(crate)`**, with **no behavior change** to `monitor`:
  - `setup_terminal()` and `restore_terminal()` — the crossterm raw-mode + alternate-screen lifecycle (single source of truth for terminal setup/teardown).
  - `handle_key(app, key)` — the pure key→navigation mapping. Reusing it keeps `monitor` and `inspect` navigation identical (no drift); for a single-run inspect view the run-navigation arms are harmless no-ops.
- Do not move the functions or change their signatures — visibility only. Leave all existing `monitor::events` tests intact and passing.
- **Files owned:** `src/monitor/events.rs`.

### 2. Implement the static inspect render loop and `inspect::run`
- Depends on Task 1 (uses the now-`pub(crate)` `setup_terminal` / `restore_terminal` / `handle_key`).
- In `src/inspect/mod.rs`, replace the `todo!()`:
  - **Pure seam (unit-tested):** add `fn build_inspect_app(run: WorkflowRun, graph: Option<&WorkflowGraph>) -> App` that constructs the `App` for a single fetched run — sets `app.layout` (via `build_layout` when a graph is present, `None` otherwise) and `app.replace_runs(vec![run])`. No I/O; assert it element-by-element.
  - **`async fn run(run_id: String) -> Result<()>`** — the thin I/O shell, mirroring `monitor::run`'s degrade posture (never panic; print a clear message and `return Ok(())` on handled failure):
    1. `Config::load()` — on error, print the `DATABASE_URL` guidance and return Ok.
    2. `get_run_state(&config.database_url, &run_id)` — on error (e.g. unknown id), print `bastion inspect: no run found for '<id>'` (+ the `./scripts/dev.sh` hint) and return Ok.
    3. `api_client.workflow_graph(&run.workflow_name)` — on error, fall back to `None` (render nodes without edges) with a non-fatal note, exactly as `monitor::run` does.
    4. `build_inspect_app(run, graph.as_ref())`, then enter the **static** loop.
  - **Static loop** `fn run_static_loop(app: &mut App) -> Result<()>` — reuse `monitor::events::{setup_terminal, restore_terminal}` and `monitor::ui::render`; loop drawing once per iteration and blocking on `crossterm::event::read()` for keyboard events dispatched through `monitor::events::handle_key`; break on `app.should_quit`. **No `tokio::select!`, no poll interval, no DB re-fetch.** Always restore the terminal on exit (best-effort), even on error.
- Render any run the ID points to (post-mortem snapshot); do not reject a still-active run — `inspect` simply shows a static view.
- **Files owned:** `src/inspect/mod.rs`.

### 3. Validate
- Run the Validation Commands below and confirm all pass.
- **Smoke test (Rule 6 — thin I/O shell):** with the orchestrator stack up (`./scripts/dev.sh` in `../python-orchestration-system`) and at least one *completed* run in `events`, run `bastion inspect <run-id>`; confirm the static graph renders, ↑/↓ (and j/k) move the selected node with the detail pane updating, the DAG shows edges, and `q` / Esc returns cleanly to the shell. Record the observation (or "deferred — stack not up") in `## Notes`. This is the natural moment to also clear the deferred `bastion monitor` live smoke test (`planning/phase1-blockB/tasks.md`), since one bring-up covers both.

## Acceptance Criteria
- `bastion inspect <run-id>` renders a completed run as a static navigable graph (nodes colored by status, two-pane layout, detail pane for the selected node).
- Arrow-key (and j/k) navigation moves the selected node; the detail pane reflects the selection. `q` / Esc / Ctrl-C exits cleanly back to the shell.
- **No polling:** the inspect loop performs exactly one DB load and never re-queries — verified by the absence of a poll interval / `tokio::select!` timer arm in `run_static_loop`.
- Unknown run id, missing `DATABASE_URL`, and an unreachable graph endpoint each degrade gracefully (clear message, no panic, terminal restored).
- `build_inspect_app` is exhaustively unit-tested (layout present when a graph is supplied; `None` when absent; single run installed; node count preserved). The thin I/O shell is smoke-tested with the result recorded in `## Notes`.
- `monitor` behavior is unchanged: all pre-existing `monitor::events` tests pass without modification.
- All gated checks pass (baseline 265 tests stays green; net new tests added).

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
<!-- filled in as work happens -->
