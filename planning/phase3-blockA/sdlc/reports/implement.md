---
type: ImplementationReport
title: Phase 3 Block A — bastion run
---

# Implementation Report — phase3-blockA

**Date:** 2026-06-22
**Plan:** planning/phase3-blockA/tasks.md
**Scope:** Full spec

## What Was Built or Changed

- `src/api/client.rs`: Added private types `TriggerRequest` (Serialize) and `TaskAccepted` (Deserialize). Added private pure helper `trigger_body(workflow_type, data)` that normalises `None` → `data: {}`. Added private method `trigger_url()` for trailing-slash-normalised POST URL. Implemented `trigger_workflow`: POST `/`, 2s timeout, `.error_for_status()`, decode `TaskAccepted`, return `task_id`. Added 6 unit tests covering `trigger_body` (Some/None data, workflow_type field) and `trigger_url` (with and without trailing slash).
- `src/run/mod.rs`: Added public pure function `parse_args(args: Option<String>)` that returns `Ok(None)` for None, parses JSON strings, rejects non-objects with typed error messages. Added private pure helper `value_type_name` for human-readable type labels in error messages. Added public pure function `format_trigger_success` for greppable `task_id:` output line. Implemented `trigger`: load config, build client, parse args, call `trigger_workflow`, print result, optionally hand off to `monitor::run`. Added 13 unit tests covering all parse_args branches, all value_type_name variants, format_trigger_success shape, and the existing render_status tests.

## Files Created or Modified

| File | Action |
|---|---|
| src/api/client.rs | modified |
| src/run/mod.rs | modified |
| planning/phase3-blockA/sdlc/reports/implement.md | created |

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
cargo fmt --check
(no output — clean)

cargo clippy -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.57s

cargo test
test result: ok. 316 passed; 0 failed; 3 ignored; 0 measured; 0 filtered out; finished in 0.03s

cargo build --release
Finished `release` profile [optimized] target(s) in 3.81s
```

Status: PASSED

## Decisions and Trade-offs

- `trigger_body` is a private (module-level) function — not `pub` or `pub(crate)` — because `TriggerRequest` is also private and Clippy's `-D private-interfaces` rejects a more-visible function returning a less-visible type. The `mod tests` block (same file) can still call it directly, satisfying the spec's unit-test requirement.
- `trigger_url()` is a private method parallel to `health_url()`, keeping the URL-construction logic testable without touching the reqwest layer.
- The `--monitor` hand-off is a direct `monitor::run(Some(task_id)).await?` call; the task_id is printed before the monitor starts so it's always visible in the terminal's scroll-back even after the TUI takes over.
- Non-object JSON values passed as `--args` return a typed error (e.g. "got number") rather than attempting coercion, matching the orchestrator's expectation of a `data: dict`.

## Notes

**Deferred live smoke test** (needs `./scripts/dev.sh` in `../python-orchestration-system`):

1. Bring up the orchestrator stack.
2. Run `bastion run <valid_workflow_type>` and confirm the printed `task_id` matches the `202` body from the orchestrator.
3. Run `bastion run <valid_workflow_type> --monitor` and confirm the TUI opens filtered to that run.
4. Run `bastion run unknown_type` and confirm the `422` error surfaces cleanly with a legible message (not a panic).
5. Run `bastion run <wf> --args '{'` and confirm the malformed-JSON error is shown and the process exits non-zero.

This smoke test is to be folded in with the deferred smoke tests for costs, inspect, and monitor on the same stack bring-up (per the phase3-blockA spec §"Step 3: Validate").

## git diff --stat

```
 src/api/client.rs | 84 ++++++++++++++++++++++++++++++++++++++++++++++++++++--
 src/run/mod.rs    | 121 +++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++--
 2 files changed, 199 insertions(+), 6 deletions(-)
```
