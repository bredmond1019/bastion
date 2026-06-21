---
type: Report
title: Implementation Report — phase1-blockA-task3
---

# Implementation Report — phase1-blockA-task3

**Date:** 2026-06-21
**Plan:** planning/phase1-blockA/tasks.md
**Scope:** Task 3 — Implement `db::workflows::list_active_runs` and `get_run_state`

## What Was Built or Changed

- Filled the two `todo!()` stubs in `src/db/workflows.rs` with real `sqlx` queries against the orchestrator's `events` table (read-only, D2 enforced).
- Added an internal `EventRow` struct (derives `sqlx::FromRow`) mapping the three queried columns.
- Added a private `parse_event_row` helper that calls the Task-2 parsing layer (`parse_task_context` + `derive_run_status`) and derives `WorkflowRun.started_at` as the minimum non-null `started_at` across all nodes.
- `list_active_runs` fetches all rows, parses each, and retains only those with at least one `Pending` or `Running` node (per contract v1.0.0 — no indexed status column exists).
- `get_run_state` fetches a single row by `events.id` using a parameterised `$1` bind.
- Both functions create a short-lived `PgPoolOptions` pool with `max_connections(1)` (Phase 1 scope; pooling deferred to Phase 4).
- Added two `#[tokio::test] #[ignore]` integration stubs documenting the expected call shape; each is also gated behind `BASTION_INTEGRATION_TEST` at runtime.

## Files Created or Modified

| File | Action |
|---|---|
| src/db/workflows.rs | modified |
| planning/phase1-blockA/sdlc/reports/task3-implement.md | created |

## Validation Output

**Commands run:**
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```
**Result:** PASSED

## Decisions and Trade-offs

- **Short-lived pool per call:** The spec explicitly scopes connection pooling to Phase 4. Creating a new pool per call is fine for Phase 1 where `list_active_runs` and `get_run_state` are called infrequently (TUI poll interval is seconds).
- **Rust-side filtering for active runs:** Contract v1.0.0 has no indexed status column, so filtering must happen in Rust after JSON parsing. A future migration could add a materialized column but that is out of scope here.
- **`started_at` derivation:** The contract does not store a top-level run `started_at`; deriving it as the minimum node-level `started_at` is the least surprising behaviour and matches the data-contract doc's field mapping note.
- **`elapsed_secs` deferred:** Requires wall-clock subtraction that belongs in the display/render layer, not the DB layer. Left `None` in the parse layer as the existing stub comment stated.

## Follow-up Work

- Phase 4: replace short-lived pools with a shared `PgPool` passed from the application runtime.
- Phase 4: SSE streaming for `get_run_state` / live updates.
- Integration tests: run with a seeded DB (CI job or docker-compose fixture) once infrastructure is available.

## git diff --stat

```
 src/db/workflows.rs | 126 ++++++++++++++++++++++++++++++++++++++++++++++++++--
 1 file changed, 122 insertions(+), 4 deletions(-)
```
