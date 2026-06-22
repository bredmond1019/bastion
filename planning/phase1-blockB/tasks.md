---
type: TaskSpec
title: Task Spec — Phase 1, Block B (TUI render loop)
description: Implement the live ratatui monitor — two-pane layout, navigation, and an event-driven DB poll over the data layer shipped in Block A.
---

# Task Spec — Phase 1, Block B (TUI render loop)

## Goal
Implement `monitor::ui` (two-pane ratatui layout) and `monitor::events` (event loop with keyboard navigation + DB poll), wired through `monitor::app` so `bastion monitor` enters the TUI and displays live workflow state.

## Context Pointers
- **Plan:** `planning/master-plan.md` → Phase 1 → Block B (the `### Block B — TUI render loop` section). Acceptance: live graph renders, arrow-key navigation moves the selected node, state updates within the poll interval, `q` exits cleanly.
- **Data layer already shipped (Block A) — do NOT re-implement:**
  - `db::workflows::list_active_runs(db_url)` / `get_run_state(db_url, run_id)` → `Vec<WorkflowRun>` / `WorkflowRun` (`src/db/workflows.rs`).
  - `WorkflowRun { id, workflow_name, status, nodes: Vec<NodeState>, started_at, elapsed_secs }`; `NodeState { id, name, status, depends_on, input, output, error, tokens_in, tokens_out, model, started_at, elapsed_secs }`; `RunStatus { Running, Success, Failed, Pending }`.
  - `api::client::workflow_graph(type)` → `WorkflowGraph { nodes, edges }` (`src/api/client.rs`).
  - `monitor::graph::build_layout(&WorkflowGraph, &[NodeState]) -> GraphLayout { graph, positions: Vec<(usize,u16,u16)>, node_states: HashMap<String,RunStatus> }` (`src/monitor/graph.rs`).
- **Stubs to fill (the only files this block owns):** `src/monitor/app.rs`, `src/monitor/ui.rs`, `src/monitor/events.rs`, `src/monitor/mod.rs`. CLI is already wired (`Commands::Monitor { workflow_id }` in `src/cli.rs`; dispatched in `src/main.rs:33` as `monitor::run(workflow_id).await`) — do not touch `cli.rs`/`main.rs`.
- **Data contract:** `docs/data-contract.md` §6 (detail-pane fields: status/timing/error/input/tokens from `node_runs[name]`, output from `nodes[name]`, run input from `events.data`) and §2 (two-source merge by class name).
- **CLAUDE.md standing rules:** Rule 1 (tests ship with the block), Rule 6 (coverage bar — separate pure logic from I/O; pure formatting/navigation unit-tested exhaustively incl. bounds/empty cases; the thin I/O shell smoke-tested and recorded in `## Notes`). The monitor track is the gated Postgres surface — async/tokio is allowed here (D5 synchronous-only applies to the `sessions/` surface, **not** `monitor/`).

## Step-by-Step Tasks

### 1. App state model + navigation (owns `src/monitor/app.rs`)
- Expand `App` beyond the current minimal stub to hold what the render loop needs: the list of `WorkflowRun`s, the current `GraphLayout`, selected-run and selected-node cursors, an optional status/error banner string, and `should_quit`.
- Add **pure** navigation methods, each clamped to bounds and safe on empty input: `next_node` / `prev_node` (move the node cursor over the current run's `nodes`), `next_run` / `prev_run` (switch runs and reset the node cursor), `selected_run()`, `selected_node()` (→ `Option<&NodeState>`), and `quit()`.
- Add a pure `replace_runs(Vec<WorkflowRun>)` (or equivalent) that swaps in freshly-polled state while keeping the selection valid (clamp cursors if the new run/node counts shrank).
- **Tests (exhaustive, no I/O):** navigation clamps at first/last node and first/last run; empty-runs and empty-nodes return `None` and never panic; `replace_runs` preserves selection when possible and clamps when the new data is shorter.
- Keep this file free of ratatui draw calls and tokio — pure state only.

### 2. Two-pane render functions (owns `src/monitor/ui.rs`, depends on Task 1)
- Implement `render(frame, &App)`: a `Layout` splitting the frame into a **left graph pane** and a **right detail pane**.
  - Left pane: render each node from `GraphLayout.positions` at its `(col, row)` grid cell, label = node name, **colored by `RunStatus`** (e.g. running=yellow, success=green, failed=red, pending=gray), with the selected node visually highlighted.
  - Right pane: detail for the selected node per data contract §6 — status, timing (`started_at` / `elapsed_secs`), `error` (if any), `model`, token counts (`tokens_in` / `tokens_out`), truncated `input` and `output`. Render the run input / a placeholder when no node is selected.
- Factor the text/format/color decisions into **pure helpers** (e.g. `status_color(RunStatus)`, `status_symbol(RunStatus)`, `format_node_detail(&NodeState) -> Vec<Line>` / `-> String`) so they unit-test without a `Frame`.
- **Tests:** assert each pure helper directly (every `RunStatus` arm → expected color/symbol; detail formatting includes status/tokens/error and handles all-`None` optional fields). Optionally assert `render` against a `ratatui::backend::TestBackend` buffer for the two-pane split. No live terminal required.

### 3. Event loop + `monitor::run` wiring (owns `src/monitor/events.rs`, `src/monitor/mod.rs`, depends on Tasks 1 & 2)
- `events::run_event_loop(&mut App, poll_secs)`: enter crossterm raw mode + alternate screen; loop with `tokio::select!` over (a) keyboard events and (b) a `tokio::time::interval(poll_secs)` DB-poll tick. On key: arrows/`j`/`k` → `App` navigation, `q`/`Esc`/`Ctrl-C` → `quit`. On tick: re-fetch run state via the Block A queries and `App::replace_runs`; redraw after any change. **Always restore the terminal** (leave alternate screen + disable raw mode) on exit, including the error path.
- `monitor::run(workflow_id)`: load `Config` (DB URL + `BASTION_POLL_INTERVAL` via `src/config.rs`); fetch active runs (`list_active_runs`, or `get_run_state` when `workflow_id` is `Some`); auto-pick the active run when no id is given; fetch the run type's DAG via `api::client::workflow_graph`; `build_layout`; construct `App`; run the loop; return cleanly. Degrade with a clear message when no active runs exist or the DB/API is unreachable — never panic.
- This is the thin I/O shell (terminal + Postgres + HTTP): keep it a shell over the already-tested pure core. Per Rule 6, **manually smoke-test it and record the result in `## Notes`** (the parts that can't be unit-tested: live render, arrow navigation, a state transition appearing within the poll interval, `q` restoring the shell). Bring up the live orchestrator for this from the `python-orchestration-system/` repo: `./scripts/dev.sh` (START) / `./scripts/dev.sh stop` (STOP) — starts Postgres + Redis + FastAPI `:8080` + Celery; trigger a workflow so there's an active run to observe.

### 4. Validate
- Run the Validation Commands listed below and confirm all pass.
- Confirm `cargo run -- monitor --help` shows the command and the binary builds; record the smoke-test observations from Task 3 in `## Notes`.

## Acceptance Criteria
- `bastion monitor` renders a running workflow as a live two-pane graph (graph left, selected-node detail right).
- Arrow-key (and `j`/`k`) navigation moves the selected node; `n`/run-switch navigation works when multiple active runs exist.
- Node state updates within the poll interval (the orchestrator now persists at every node boundary — D28), with no manual refresh.
- `q` (and `Esc`/`Ctrl-C`) exits cleanly and restores the shell; the error/no-runs path degrades with a clear message instead of panicking.
- Pure navigation and formatting logic is unit-tested exhaustively, including bounds and all-`None` cases; the I/O shell smoke-test result is recorded in `## Notes`.
- All gated checks pass; the monitor stays a read-only observer (no writes to the orchestrator DB — D2).

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
cargo run -- monitor --help
```

## Notes
<!-- filled in as work happens; Rule 6 requires the Task 3 I/O smoke-test result here -->
