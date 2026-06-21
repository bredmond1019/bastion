---
type: Report
title: Implementation Report — phase5-blockG
---

# Implementation Report — phase5-blockG

**Date:** 2026-06-21
**Plan:** planning/phase5-blockG/tasks.md
**Scope:** Full spec

## What Was Built or Changed

- `src/cli.rs` — Added `Ask` subcommand with all six flags matching the v0.1.0 contract verbatim (`--session`, `--prompt-file`, `--out`, `--dir`, `--timeout` default 180, `--launch-cmd` default `claude --permission-mode bypassPermissions`). Added `#[derive(Debug)]` to `Commands` (needed for test panic messages). Added four unit tests covering defaults, all-flags, and required-flag failures.
- `src/main.rs` — Dispatch arm for `Commands::Ask` constructs `AskArgs` and calls `sessions::ask::ask(...)`, mapping `AskError` to a non-zero exit with diagnostics on stderr. DB pool is not touched (D4 preserved).
- `src/sessions/mod.rs` — Registered `pub mod ask;`.
- `src/sessions/ask.rs` — New module (pure + I/O): `AskArgs` struct, `AskError` enum (`UntrustedDir`, `Tmux`, `Launch`, `Timeout`), pure helpers (`done_path`, `trigger_text`, `poll_plan`, `has_session_args`, `has_session`), I/O shell (`ask`), two private I/O helpers (`wait_for_claude`, `foreground_cmd_for`), and 23 unit tests covering all pure functions and all error variants.

## Files Created or Modified

| File | Action |
|---|---|
| src/cli.rs | modified |
| src/main.rs | modified |
| src/sessions/mod.rs | modified |
| src/sessions/ask.rs | created |
| planning/phase5-blockG/sdlc/reports/implement.md | created |

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
cargo fmt --check: (no output, exit 0)
cargo clippy -- -D warnings: Finished `dev` profile (exit 0)
cargo test: test result: ok. 206 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
cargo build --release: Finished `release` profile (exit 0)
```

Status: PASSED

## Decisions and Trade-offs

- **Trigger wording** is taken verbatim from the brain contract (`docs/integrations/claude-code-llm-provider.md` §2): "Read <prompt-file> and follow its instructions exactly. Write your complete answer to <out>. When finished, create an empty file <out>.done". The exact whitespace and phrasing is contract-locked.
- **Readiness polling** uses a 30-second budget (READINESS_TIMEOUT_SECS) with 500ms interval. The readiness check looks for the foreground command being exactly `claude`. Any other command (idle shell or something else) triggers a launch. This means if Claude finishes a previous task and returns to a shell, the next `ask` call will re-launch it — intentional: the session semantics guarantee Claude is running when the trigger is sent.
- **D4 preserved**: `ask` calls no `Config::load()`, opens no Postgres pool. The entire path is tmux process invocations only.
- **D5 preserved**: `ask` is synchronous blocking (`std::process::Command` + `std::thread::sleep` poll loop). No async/tokio on this path.
- **D6 preserved**: `foreground_cmd_for` uses `parse_sessions`, which skips malformed lines with a warning (model.rs D6 behavior).
- **`div_ceil`**: clippy flagged the manual ceiling-division; replaced with `u64::div_ceil` (stabilized in Rust 1.73, compiler here is 1.95).
- **`#[derive(Debug)]` on Commands**: added so test `panic!` messages can format the unexpected variant. This is a purely additive change with no behavioral impact.

## Follow-up Work

- Manual smoke test against a live tmux + Claude session (Coverage bar rule 6) — to be recorded in `planning/phase5-blockG/tasks.md` `## Notes` section by the operator after running with a real Claude session.
- The orchestrator's `BastionSessionBackend` (python-orchestration-system) can now implement the session-mode provider against this stable CLI contract (see brain doc §3 blockers matrix, item 4).

## git diff --stat

```
 planning/master-plan.md |  35 ++++++++++++++++
 planning/status.md      |   2 +
 src/cli.rs              | 113 +++++++++++++++++++++++++++++++++++++++++++++++-
 src/main.rs             |  22 ++++++++++
 src/sessions/mod.rs     |   1 +
 5 files changed, 172 insertions(+), 1 deletion(-)
```

(src/sessions/ask.rs is new and does not appear in the diff against HEAD — it was created during this session.)
