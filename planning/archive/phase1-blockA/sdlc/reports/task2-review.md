---
type: Report
subtype: Review
task: phase1-blockA-task2
---

# Review Report — phase1-blockA-task2

**Date:** 2026-06-21
**Spec:** planning/phase1-blockA/tasks.md
**Scope:** Task 2 — Implement `db::workflows` — `node_runs` JSON → `NodeState` parsing
**Verdict:** PASS

## Acceptance Criteria Check

| Criterion | Status | Evidence |
|---|---|---|
| Unit tests cover `node_runs` JSON → `NodeState` parse against captured fixtures | MET | 22 tests in `src/db/workflows.rs::tests`, covering both in_progress and completed fixtures |
| All four `RunStatus` variants (`pending`, `running`, `success`, `failed`) deserialize correctly | MET | `run_status_deserializes_pending/running/success/failed` — all 4 pass |
| `usage` being `null` produces `None` for `tokens_in`, `tokens_out`, `model` | MET | `in_progress_fixture_null_usage_produces_none_fields` passes; logic in `parse_task_context` matches `None` on null usage |
| class-name join in `build_layout` overlays live `NodeState` onto graph nodes | SKIP | Task 4 scope — `build_layout` is not in Task 2's step list |
| Topological ordering and position assignment verified by unit tests | SKIP | Task 4 scope |
| `list_active_runs` and `get_run_state` stubs filled with sqlx + `#[ignore]` integration stub | SKIP | Task 3 scope — stubs remain as `todo!()` placeholders, as expected for Task 2 |
| `cargo fmt --check` passes | MET | Exit 0 |
| `cargo clippy -- -D warnings` passes with zero warnings | MET | Exit 0 (42 tests compiled, 0 warnings) |
| `cargo test` passes (all non-`#[ignore]` tests green) | MET | 42 passed, 0 failed, 0 ignored |
| `cargo build --release` passes | MET | Exit 0 |
| No writes to the orchestrator's PostgreSQL (D2 enforced) | MET | `parse_task_context` and `derive_run_status` operate on in-memory `serde_json::Value` only; stubs do not execute any DB calls |

## Fresh Test Results

```
cargo fmt --check        → EXIT:0  PASS
cargo clippy -- -D warnings → EXIT:0  PASS (0 warnings)
cargo test               → EXIT:0  PASS (42 passed; 0 failed; 0 ignored)
cargo build --release    → EXIT:0  PASS
```

Test breakdown (Task 2 specific — 22 tests in `db::workflows::tests`):
- RunStatus deserialization: 5 tests (4 valid variants + 1 unknown-string rejection)
- in-progress fixture: 8 tests (node count, mixed statuses, derived Running status, null usage → None, non-null usage populated, output joined from nodes map, null output → None, pending started_at → None)
- completed fixture: 4 tests (node count, derived Failed status, failed node error message, LLM usage populated, success output present)
- `derive_run_status` edge cases: 6 tests (all success, all failed, running priority over pending, pending when no running, failed when mixed terminal, running priority over failed)
- error handling: 1 test (missing node_runs key → Err)

## Verdict: PASS

All Task 2 acceptance criteria are fully met. The `parse_task_context()` and `derive_run_status()` functions are correctly implemented in `src/db/workflows.rs` with 22 hermetic unit tests covering all required scenarios. All four gating validation commands pass with zero errors or warnings. Task 3 and Task 4 criteria are correctly scoped out (stubs remain as `todo!()` placeholders, as designed). No D2 violations — all new code is purely in-memory parsing with no DB writes.

## Issues Found

None.

## Next Steps

Task 3 is the natural next step: fill in `list_active_runs` and `get_run_state` with `sqlx` queries and add the `#[ignore]` integration test stub. The parsing layer from Task 2 is ready for Task 3 to consume.
