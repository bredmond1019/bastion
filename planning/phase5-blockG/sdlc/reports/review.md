---
type: sdlc/review-report
phase: phase5-blockG
date: 2026-06-21
---

# Review Report — phase5-blockG

**Date:** 2026-06-21
**Spec:** planning/phase5-blockG/tasks.md
**Scope:** Full spec
**Verdict:** PASS

## Acceptance Criteria Check

| Criterion | Status | Evidence |
|---|---|---|
| `bastion ask` implements brain contract v0.1.0 exactly: flags, trigger wording, `<out>.done` marker, exit semantics (0 only when complete; non-zero with stderr on timeout/failure) | MET | `src/cli.rs:91-110` — all 6 flags with correct names and defaults; `src/sessions/ask.rs:98-108` — trigger wording matches contract verbatim; `src/sessions/ask.rs:87-91` — done_path appends `.done`; `src/main.rs:73-76` — errors map to non-zero via `anyhow` + stderr `eprintln!` |
| Ensures session + Claude are up (create + launch when cold, skip launch when `classify_state` Running); sends only the fixed trigger keystrokes | MET | `src/sessions/ask.rs:166-203` — cold path creates session, sends launch_cmd, calls `wait_for_claude`; warm path checks `classify_state != Running` before launching; trigger sent via single `send_keys` call; `wait_for_claude` uses `classify_state` to detect any non-shell foreground process (covers version-string naming like "2.1.185") |
| Untrusted `--dir` fails fast with clear message; trust is read-only (no write to `~/.claude.json`) | MET | `src/sessions/ask.rs:155-161` — trust pre-flight checks `trust_status(dir)` and returns `AskError::UntrustedDir` immediately; reuses Block F read-only `trust_status`; smoke test Scenario 4 confirms exit 1 with clear stderr message and no session created |
| Pure logic (`done_path`, `trigger_text`, poll-bound, new `*_args`) exhaustively unit-tested without I/O; timeout path has explicit test; I/O shell smoke-tested and recorded in `## Notes` | MET | `src/sessions/ask.rs:280-508` — 21 pure-unit tests covering all pure helpers and all AskError display paths; `pure_helpers_require_no_database_url` confirms no I/O dependency; `planning/phase5-blockG/tasks.md` `## Notes` records all 5 required scenarios (cold start, warm reuse, timeout, untrusted dir, unknown dir) plus D4/D5 verification and cleanup |
| DB-free (D4) and synchronous (D5) — no `Config::load()`, no pool, no `.await` on this path | MET | `src/sessions/ask.rs:12-17` — imports only `crate::sessions::*`, `std::path`, `std::thread`, `std::time`; no `tokio`, `async`, `Config`, or pool anywhere in ask.rs; `src/main.rs:57-77` — dispatch branch is synchronous (no `.await`); `pure_helpers_require_no_database_url` test removes DATABASE_URL and confirms all pure helpers succeed |
| All gated checks pass; test baseline increases with new cases | MET | Fresh run: fmt exit 0, clippy exit 0, test 206 passed 0 failed exit 0, build --release exit 0; 21 new `sessions::ask::tests::*` cases + 4 new `cli::tests::ask_*` cases added by this block |

## Fresh Test Results

**cargo fmt --check**
```
(no output, exit 0)
```
Result: PASSED

**cargo clippy -- -D warnings**
```
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.16s
```
Result: PASSED

**cargo test**
```
running 208 tests
test sessions::ask::tests::ask_error_launch_message_contains_session_and_timeout ... ok
test sessions::ask::tests::ask_error_timeout_message_contains_timeout_and_out ... ok
test sessions::ask::tests::ask_error_tmux_message_contains_op ... ok
test sessions::ask::tests::ask_error_untrusted_dir_message_contains_dir ... ok
test sessions::ask::tests::done_path_preserves_parent_directory ... ok
test sessions::ask::tests::done_path_simple_filename ... ok
test sessions::ask::tests::done_path_with_extension ... ok
test sessions::ask::tests::done_path_without_extension ... ok
test sessions::ask::tests::has_session_args_correct ... ok
test sessions::ask::tests::has_session_args_uses_provided_name ... ok
test sessions::ask::tests::poll_plan_180s_500ms ... ok
test sessions::ask::tests::poll_plan_fractional_rounds_up ... ok
test sessions::ask::tests::poll_plan_one_second_one_ms ... ok
test sessions::ask::tests::poll_plan_rounds_up ... ok
test sessions::ask::tests::poll_plan_zero_interval_returns_zero ... ok
test sessions::ask::tests::poll_plan_zero_timeout ... ok
test sessions::ask::tests::pure_helpers_require_no_database_url ... ok
test sessions::ask::tests::trigger_text_absolute_paths_present ... ok
test sessions::ask::tests::trigger_text_contains_done_marker_path ... ok
test sessions::ask::tests::trigger_text_contains_out_path ... ok
test sessions::ask::tests::trigger_text_contains_prompt_file_path ... ok
test sessions::ask::tests::trigger_text_contract_wording ... ok
test cli::tests::ask_all_flags_parse ... ok
test cli::tests::ask_missing_required_flags_fails ... ok
test cli::tests::ask_required_flags_parse ... ok
... (181 additional tests all ok) ...

test result: ok. 206 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 0.01s
```
Result: PASSED

**cargo build --release**
```
Finished `release` profile [optimized] target(s) in 0.13s
```
Result: PASSED

## Verdict: PASS

All six acceptance criteria are fully met and all four gating checks pass. The `bastion ask` subcommand is implemented exactly per the brain contract v0.1.0 (`docs/integrations/claude-code-llm-provider.md` §2): all required flags are present with correct names and defaults, trigger wording matches the contract verbatim, the `<out>.done` marker protocol is correctly implemented, and exit semantics (0 on success, non-zero with stderr diagnostics on failure) are correct. Pure helper functions (`done_path`, `trigger_text`, `poll_plan`, `has_session_args`) are exhaustively unit-tested in 21 cases without I/O, all four error variants have explicit display tests, and the I/O shell has been smoke-tested across all five required scenarios (cold start PONG, warm session reuse, timeout with diagnostics, untrusted dir fail-fast, unknown dir proceed) with results recorded in `## Notes`. The implementation correctly preserves D4 (DB-free) and D5 (synchronous) by importing only `std::` and `crate::sessions::*` with no Config, pool, async, or tokio dependencies. The fix applied in this pass (using `classify_state` rather than exact-string `"claude"` match) correctly handles Claude Code's process rename via `pthread_setname_np` and is robust to future version strings.

## Issues Found

None.

## Next Steps

Block G is complete. Per `planning/master-plan.md`, proceed to the next phase or block. The brain coordination doc (`docs/integrations/claude-code-llm-provider.md`) §3 matrix should be updated to reflect Block G as done, which unblocks the orchestrator's `CLAUDE_CODE_SESSION` provider implementation (item 4).
