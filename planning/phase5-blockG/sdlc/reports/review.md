---
type: Report
title: Review Report — phase5-blockG
---

# Review Report — phase5-blockG

**Date:** 2026-06-21
**Spec:** planning/phase5-blockG/tasks.md
**Scope:** Full spec
**Verdict:** PARTIAL

## Acceptance Criteria Check

| Criterion | Status | Evidence |
|---|---|---|
| `bastion ask` implements the brain contract v0.1.0 exactly: flags, trigger wording, `<out>.done` marker, and exit semantics (`0` only on success; non-zero with stderr on timeout/failure) | MET | `src/cli.rs:91-110` — all six flags match contract verbatim; `src/sessions/ask.rs:98-108` — trigger wording matches spec; `src/sessions/ask.rs:208-222` — exit-0-only-on-success; `src/main.rs:73-76` — stderr diagnostics on error |
| Ensures session + Claude are up (creating + launching when cold, skipping launch when Claude already running via Block F `classify_state`); sends only the fixed trigger keystrokes | MET | `src/sessions/ask.rs:165-199` — cold-start path creates session and launches; line 183 uses `classify_state` to skip launch when claude is foreground; line 196 sends only trigger text |
| An untrusted `--dir` fails fast with a clear message; trust is read-only (no write to `~/.claude.json`) | MET | `src/sessions/ask.rs:155-160` — calls `trust_status()` and returns `AskError::UntrustedDir` immediately; no write path exists in the module |
| Pure logic (`done_path`, `trigger_text`, poll-bound, any new `*_args`) is exhaustively unit-tested without I/O; the timeout path has an explicit test; the I/O shell is smoke-tested and recorded in `## Notes` | PARTIAL | Pure functions: 23 unit tests in `src/sessions/ask.rs:262-497` covering all pure helpers and all error variants. Timeout path tested via `ask_error_timeout_message_contains_timeout_and_out`. **GAP: the I/O shell smoke test has NOT been performed and recorded — `planning/phase5-blockG/tasks.md` `## Notes` still contains the placeholder text.** |
| DB-free (D4) and synchronous (D5) preserved — no `Config::load()`, no pool, no `.await` on this path | MET | `src/sessions/ask.rs` — no Config import, no async/tokio; `src/main.rs:57-77` — dispatches to `ask()` without `.await`; DB pool is never touched |
| All gated checks pass and the test baseline increases with the new cases | MET | All four gating checks pass (see Fresh Test Results); test count increased to 208 (was 183 before blockG; 25 new tests added in `sessions::ask::tests` and `cli::tests`) |

## Fresh Test Results

All four gating checks re-run from scratch against the current working tree:

**cargo fmt --check**
```
(no output, exit 0)
```
PASSED

**cargo clippy -- -D warnings**
```
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.16s
```
PASSED (exit 0)

**cargo test**
```
running 208 tests
... [all sessions::ask::tests::* passed — 23 tests] ...
... [all cli::tests::* passed including ask_required_flags_parse, ask_all_flags_parse, ask_missing_required_flags_fails] ...
test result: ok. 206 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 0.01s
```
PASSED (exit 0)

Note: the binary reports 208 lines but the summary line says 206 passed + 2 ignored = 208 total, matching the earlier test report.

**cargo build --release**
```
Finished `release` profile [optimized] target(s) in 0.14s
```
PASSED (exit 0)

## Verdict: PARTIAL

All four gating checks pass and five of the six acceptance criteria are fully met. The implementation correctly wires `bastion ask` with the brain contract v0.1.0 CLI surface, trigger wording, done-marker protocol, exit semantics, cold-start/warm-session logic, trust pre-flight, DB-free + synchronous invariants, and a strong pure-function unit-test suite. The sole gap is that the manual smoke test against a live tmux + Claude session — required by the acceptance criteria and CLAUDE.md rule 6 — has not been performed or recorded in `planning/phase5-blockG/tasks.md` `## Notes`. The placeholder text remains unchanged.

## Issues Found

- **Missing smoke test record (criterion 4, PARTIAL):** `planning/phase5-blockG/tasks.md` `## Notes` still contains the boilerplate placeholder `<filled in as work happens — record the manual smoke-test results here per Coverage bar rule 6>`. The spec requires: (a) write a tiny prompt file, run `bastion ask`, confirm exit 0 and answer written; (b) re-run to confirm warm-session skip; (c) timeout scenario exits non-zero with stderr diagnostics; (d) untrusted/unknown dir behavior documented; (e) confirm Postgres stopped (D4) and synchronous (D5). All five scenarios must be run and their results recorded in `## Notes` before this criterion is MET.

## Next Steps

1. With a live tmux server and a Claude-trusted directory available, run the five smoke-test scenarios listed in `planning/phase5-blockG/tasks.md` Task 4 (Validate).
2. Record the results — pass/fail, exit codes, observed output — in the `## Notes` section of `planning/phase5-blockG/tasks.md`.
3. Re-run this review. If all five scenarios pass and the notes are filled in, the verdict will upgrade to PASS.
