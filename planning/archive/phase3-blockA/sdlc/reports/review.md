---
type: ReviewReport
title: Phase 3 Block A — bastion run
---

# Review Report — phase3-blockA

**Date:** 2026-06-22
**Spec:** planning/phase3-blockA/tasks.md
**Scope:** Full spec
**Verdict:** PASS

## Acceptance Criteria Check

| Criterion | Status | Evidence |
|---|---|---|
| `bastion run <workflow>` issues `POST /` with `{ "workflow_type": <workflow>, "data": {...} }` and prints the returned `task_id` | MET | `src/api/client.rs:104–126` (`trigger_workflow` posts to `trigger_url()` with `trigger_body`); `src/run/mod.rs:55–69` (`trigger` calls client and prints via `format_trigger_success`) |
| `--args '{"k":1}'` forwarded as `data` object; omitting `--args` sends `data: {}`; malformed `--args` fails fast with clear error and no panic | MET | `src/run/mod.rs:19–34` (`parse_args`); `src/api/client.rs:40–48` (`trigger_body` defaults `None` to `{}`); tests `parse_args_none_returns_none`, `parse_args_valid_object_returns_some`, `parse_args_malformed_json_returns_err`, `trigger_body_none_data_serializes_as_empty_object` |
| `--monitor` drops into `bastion monitor` filtered to the triggered run after printing the `task_id` | MET | `src/run/mod.rs:65–68`: `print!` then `monitor::run(Some(task_id)).await?` |
| Unreachable orchestrator or `422` produces a clear error message, not a panic | MET | `src/api/client.rs:119–125`: `.error_for_status().context(...)` propagates errors; `src/run/mod.rs:62–64`: `.with_context(...)` adds orchestrator hint; all error paths return `Err`, no panics |
| Pure logic (`trigger_body`, `parse_args`, output formatter) is unit-tested element-by-element including malformed-args and default-data paths; all gated checks pass; test baseline grows from 302 | MET | 14 new tests added (6 in `api::client::tests`, 13 in `run::tests`); baseline grew from 302 to 316; all 4 gating checks pass |

## Fresh Test Results

```
cargo fmt --check
(no output — clean)
EXIT: 0

cargo clippy -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.16s
EXIT: 0

cargo test
Finished `test` profile [unoptimized + debuginfo] target(s) in 0.16s
Running unittests src/main.rs (target/debug/deps/bastion-...)

test result: ok. 316 passed; 0 failed; 3 ignored; 0 measured; 0 filtered out; finished in 0.01s
EXIT: 0

cargo build --release
Finished `release` profile [optimized] target(s) in 0.13s
EXIT: 0
```

All 4 gating checks passed (fmt, clippy, test, build).

## Verdict: PASS

All 5 acceptance criteria are fully satisfied and all 4 gating checks pass with exit 0. The implementation correctly adds `trigger_body` and `trigger_url` pure helpers in `src/api/client.rs` with proper serialization (None data → `{}`), implements `parse_args` and `format_trigger_success` pure functions in `src/run/mod.rs`, and wires the async `trigger` function to load config, call the API, print the task_id, and optionally hand off to the monitor. Error paths use `anyhow` context throughout — no panics. The test count grew from 302 to 316, with element-by-element coverage of all pure functions including all non-object JSON variants and both URL trailing-slash cases. The live smoke test is appropriately deferred (requires the orchestrator stack) and documented in the implement report's Notes section.

## Issues Found

None.

## Next Steps

- The deferred live smoke test (trigger real workflow, confirm task_id, test --monitor, test 422 for unknown workflow, test malformed --args) should be folded in with the deferred smoke tests for costs, inspect, and monitor on the same orchestrator stack bring-up (per the spec §"Step 3: Validate").
- Proceed to phase3-blockB or the next block in the master plan sequence.
