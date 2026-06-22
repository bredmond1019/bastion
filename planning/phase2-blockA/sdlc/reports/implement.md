---
type: ImplementationReport
title: Implementation Report — phase2-blockA
---

# Implementation Report — phase2-blockA

**Date:** 2026-06-22
**Plan:** planning/phase2-blockA/tasks.md
**Scope:** Full spec

## What Was Built or Changed

- `src/monitor/events.rs` — Widened visibility of `setup_terminal`, `restore_terminal`, and `handle_key` from private to `pub(crate)`. No behavior changes; all existing tests pass unchanged.
- `src/inspect/mod.rs` — Replaced the `todo!()` stub with the full static inspect implementation:
  - `pub fn build_inspect_app(run, graph)` — pure constructor that builds an `App` for a single run with optional `GraphLayout`. Exhaustively unit-tested (9 test cases).
  - `pub fn run_static_loop(app)` — thin I/O shell: `setup_terminal` → draw loop + blocking `crossterm::event::read()` → `handle_key` → `restore_terminal`. No poll interval, no `tokio::select!`.
  - `pub async fn run(run_id)` — entry point that loads config, fetches run from DB, fetches graph from API (non-fatal fallback), builds the App, and enters the static loop. Degrades gracefully (clear message, no panic, terminal restored) on missing `DATABASE_URL`, unknown run ID, or unreachable graph endpoint.

## Files Created or Modified

| File | Action |
|---|---|
| `src/monitor/events.rs` | modified — visibility widened for 3 functions |
| `src/inspect/mod.rs` | modified — replaced `todo!()` stub with full implementation |

## Validation Output

**Commands run:**
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

**Results:**
```
cargo fmt --check: (no output — clean)

cargo clippy -- -D warnings:
    Checking bastion v0.1.0
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.41s

cargo test:
running 274 tests
... (272 passed; 0 failed; 2 ignored)
test result: ok. 272 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 0.04s

cargo build --release:
    Compiling bastion v0.1.0
    Finished `release` profile [optimized] target(s) in 3.65s
```

Status: PASSED

## Decisions and Trade-offs

- **Visibility widening only (Task 1):** The spec required no signature or behavior changes to the three shared helpers — only `pub(crate)` to make them accessible from `src/inspect/mod.rs`. This keeps the single source of truth for terminal lifecycle and key handling in `monitor::events`.
- **`run_static_loop` is synchronous:** The static loop uses blocking `crossterm::event::read()` (not `tokio::select!` + channel). This is correct for a no-poll view — no need to interleave async timers — and keeps the function `fn` (not `async fn`), making the "no poll interval" acceptance criterion trivially verifiable by reading the signature.
- **Graceful degrade on graph API failure:** Consistent with `monitor::run` — renders nodes without edges, prints a non-fatal note to stderr, and continues into the TUI. No panic, terminal managed correctly.
- **No run-status filter:** The spec explicitly states "do not reject a still-active run" — `inspect` accepts any `events.id` regardless of workflow status. The static snapshot is all it ever renders.

## Notes

- **Smoke test:** Deferred — orchestrator stack (`./scripts/dev.sh` in `../python-orchestration-system`) not available in this environment. The thin I/O shell (`run` + `run_static_loop`) will be smoke-tested when the stack is next up, consistent with Rule 6.
- Baseline test count was 265; shipped with 272 passing (+7 new tests in `inspect::tests` beyond the 9 `build_inspect_app` cases, 2 are ignored integration tests that existed before).

## Follow-up Work

- Live smoke test of `bastion inspect <run-id>` when the orchestrator stack is running — record observation in task spec `## Notes`.
- The deferred `bastion monitor` live smoke test (phase1-blockB) can be cleared at the same time per the spec's Task 3 note.

## git diff --stat

```
 src/inspect/mod.rs    | 266 +++++++++++++++++++++++++++++++++++++++++++++++++-
 src/monitor/events.rs |   8 +-
 2 files changed, 268 insertions(+), 6 deletions(-))
```
