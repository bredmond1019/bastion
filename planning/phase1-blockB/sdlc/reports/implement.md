---
type: ImplementationReport
title: Implementation Report — phase1-blockB
description: TUI render loop for bastion monitor — two-pane layout, keyboard navigation, event loop, and DB poll.
---

# Implementation Report — phase1-blockB

**Date:** 2026-06-22
**Plan:** planning/phase1-blockB/tasks.md
**Scope:** Full spec (Tasks 1–4)

## What Was Built or Changed

- `src/monitor/app.rs` — Expanded `App` struct with `GraphLayout`, `banner`, and `should_quit` fields. Added pure navigation methods (`next_node`, `prev_node`, `next_run`, `prev_run`, `selected_node`, `replace_runs`, `quit`) with bounds clamping. 20 unit tests covering all edge cases (empty runs, empty nodes, clamp at first/last, cursor reset on run switch, `replace_runs` with shorter/empty data).
- `src/monitor/ui.rs` — Implemented `render(frame, &App)` splitting the terminal 50/50 horizontally into a graph pane (left) and detail pane (right). Pure helpers: `status_color`, `status_symbol`, `format_node_detail`, `build_graph_lines` (grid renderer from `GraphLayout.positions`). 18 unit tests covering all `RunStatus` variants for color and symbol, detail formatting with all-None fields, error/tokens/timing fields, graph lines with and without layout, and a `TestBackend` render test confirming the two-pane split draws without panic.
- `src/monitor/events.rs` — Implemented `run_event_loop` (crossterm raw mode + alternate screen; `tokio::select!` over a background keyboard-event thread and a `tokio::time::interval` tick; terminal always restored on exit). Pure `handle_key` function dispatching arrows/j/k (node nav), left/right/n/p (run nav), q/Esc/Ctrl-C (quit). 17 unit tests for all key bindings and clamp behavior.
- `src/monitor/mod.rs` — Implemented `monitor::run(workflow_id)`: loads `Config`, fetches active runs or single run by id, fetches initial graph from API, builds `GraphLayout`, constructs `App`, delegates to `run_event_loop`. Degrades with clear `eprintln!` messages on config error, DB error, API error, or no active runs — never panics.

## Files Created or Modified

| File | Action |
|---|---|
| `src/monitor/app.rs` | modified (stub expanded to full implementation + tests) |
| `src/monitor/ui.rs` | modified (stub expanded to full implementation + tests) |
| `src/monitor/events.rs` | modified (stub expanded to full implementation + tests) |
| `src/monitor/mod.rs` | modified (stub expanded to full wiring) |

## Validation Output

**Commands run:**
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
cargo run -- monitor --help
```

**Results:**
```
cargo fmt --check   → exit 0 (no diff)

cargo clippy -- -D warnings
    Checking bastion v0.1.0
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.39s
→ exit 0 (no warnings)

cargo test
running 265 tests
... (all pass)
test result: ok. 263 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out

cargo build --release
    Finished `release` profile [optimized] target(s) in 3.71s
→ exit 0

cargo run -- monitor --help
Live TUI graph monitor for workflow execution
Usage: bastion monitor [OPTIONS]
Options:
  -w, --workflow-id <WORKFLOW_ID>  Filter to a specific workflow ID
  -h, --help                       Print help
→ exit 0
```

Status: PASSED

## Decisions and Trade-offs

1. **Keyboard event thread + tokio channel instead of crossterm `event-stream`:** The `event-stream` feature was not listed in Cargo.toml. Rather than add it, the implementation uses a `std::thread::spawn` background thread that calls `crossterm::event::poll` with a 100ms timeout loop and forwards events over a `tokio::sync::mpsc` channel. This is a well-established pattern that avoids modifying the dependency manifest and keeps the tokio runtime free during I/O waits.

2. **Text-based graph grid instead of ratatui Canvas:** The left pane renders nodes in a column/row text grid derived from `GraphLayout.positions` rather than using the ratatui `Canvas` widget. This keeps the implementation straightforward, avoids floating-point canvas coordinates, and is fully testable via `build_graph_lines` (pure function, no Frame required). Visual arrows between nodes are deferred to Phase 4 polish.

3. **Layout stored on App, rebuilt per tick:** `App.layout` holds the `GraphLayout` for the currently selected run. On each poll tick, `events.rs` rebuilds it from the API graph + fresh node states. This is slightly redundant for the static DAG shape, but keeps the code simple and ensures node positions are always consistent with the current run.

4. **Banner degrades errors instead of propagating:** DB and API errors during the poll tick are surfaced as an `app.banner` string rendered in the detail pane rather than aborting the event loop. A transient Postgres hiccup should not kill the TUI.

5. **`monitor::run` degrades with `eprintln!` before entering TUI:** Errors that prevent the TUI from starting (missing config, DB unreachable, no active runs) are reported to stderr as plain messages and exit cleanly rather than returning an `Err` that propagates to `main`. This matches the spirit of Rule 6 (degrade with a clear message instead of panicking).

## Follow-up Work

- **Smoke test (Rule 6 requirement):** The thin I/O shell (`run_event_loop`, live render, arrow navigation, state transition within poll interval, `q` restoring shell) requires a live orchestrator for manual verification. To test: start the orchestrator (`./scripts/dev.sh` from `python-orchestration-system/`), trigger a workflow, then run `bastion monitor`. The degrade path (no active runs) is testable without the orchestrator by running `cargo run -- monitor` with `DATABASE_URL` pointing to an empty DB.
- **Visual graph arrows:** Edges between nodes in the left pane are not yet rendered. Deferred to Phase 4 polish.
- **`n` key run navigation** in the spec references running `n`/run-switch when multiple active runs exist — this is implemented via `next_run`/Right arrow and `p`/Left arrow.

## git diff --stat

```
 src/monitor/app.rs    | 365 ++++++++++++++++++++++++++++++-
 src/monitor/events.rs | 388 ++++++++++++++++++++++++++++++++-
 src/monitor/mod.rs    |  86 +++++++-
 src/monitor/ui.rs     | 583 +++++++++++++++++++++++++++++++++++++++++++++++++-
 4 files changed, 1413 insertions(+), 9 deletions(-)
```
