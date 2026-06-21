---
type: TaskSpec
title: Phase 5 Block D — bastion capture
description: Read tmux pane output non-interactively — last-N-lines trimming, CLI verb, and graceful degradation, built on the Block A capture primitives.
---

# Task Spec — Phase 5, Block D

## Goal
Implement `bastion capture <session> [--lines N]` → `tmux capture-pane -p -t <session>`, printing the last N lines so a session's recent output can be checked without attaching.

## Context Pointers
- **Plan:** `planning/master-plan.md` → *Phase 5 — Session Management* → *Block D — `bastion capture`* (lines ~185–190).
- **Decisions:** D4 (sessions surface is DB-free), D5 (session verbs are synchronous blocking `std::process::Command` calls — no async/tokio coupling), D6 (malformed tmux output handled gracefully, not fatally).
- **Existing primitives to reuse (do not re-create):**
  - `src/sessions/tmux.rs` — `capture_pane_args(session)` and `capture_pane_raw(session)` already exist (added in Block A for the `sessions` last-line feature). The `TmuxError` taxonomy (`NotInstalled` / `NoServer` / `ExitError`) and `run_tmux` are already in place.
  - `src/sessions/model.rs` — `Pane` already owns pane-output line parsing (`Pane::new`, `Pane::last_line`). The `--lines` tail logic belongs here, alongside `last_line`.
  - `src/sessions/commands.rs` — verb handlers (`send` / `new` / `kill`) plus the shared `apply_degradation` / `degrade_tmux_error` graceful-degradation path and the pure `format_*` helpers.
  - `src/cli.rs` — `Commands` enum (clap subcommands).
  - `src/main.rs` — dispatch match arm; sessions verbs stay on the sync, DB-free path (no `Config::load()`, no `.await`).
- **Standing rules (CLAUDE.md):** every block ships with tests (rule 1); maintain OKF frontmatter (rule 2); the Coverage bar (rule 6) — pure trimming/parsing is exhaustively unit-tested against fixtures, error/degradation paths are tested explicitly, and the thin I/O wrapper is manually smoke-tested with the result recorded in Notes.

## Step-by-Step Tasks

### 1. Pane tail-trimming logic (model.rs)
- **Files owned:** `src/sessions/model.rs` only.
- Add a pure tail accessor to `Pane`, mirroring the existing `last_line`: `pub fn last_lines(&self, n: Option<usize>) -> Vec<String>` (or an equivalent returning the trailing lines as owned `String`s in original order).
  - `tmux capture-pane -p` pads output with trailing blank lines up to the pane height — **drop trailing blank/whitespace-only lines first**, then take the last `n` lines.
  - `None` ⇒ return all lines (after trailing-blank trimming); `Some(n)` ⇒ return at most the last `n` non-padding lines. `Some(0)` returns an empty `Vec`.
- If a small formatting helper is needed to join the lines for printing (e.g. `format_capture(lines: &[String]) -> String`), keep it pure; it may live here or in `commands.rs` (Task 2) — pick one owner and name it in that task's files to avoid an overlap.
- Unit tests (against inline fixtures, no live tmux): output with more lines than `n`, fewer than `n`, exactly `n`; `Some(0)`; `None` (all lines); trailing blank-line padding is stripped; empty / all-blank input yields an empty result; line order is preserved (oldest→newest).

### 2. capture verb handler + CLI wiring
- **Files owned:** `src/sessions/commands.rs`, `src/cli.rs`, `src/main.rs`.
- **dependsOn:** Task 1 (uses `Pane::last_lines`).
- `src/cli.rs`: add a `Capture` variant to `Commands` — `session: String` and `#[arg(long)] lines: Option<usize>`. Add a doc comment so it shows in `--help`.
- `src/sessions/commands.rs`: add `pub fn capture(session_name: &str, lines: Option<usize>) -> anyhow::Result<()>` mirroring `send`/`kill`: call `tmux::capture_pane_raw`, build a `Pane`, take `pane.last_lines(lines)`, print them (one per line); on `Err` route through the existing `apply_degradation("capture", session_name, e)`. Confirm `degrade_tmux_error`'s default (non-`new`) branch yields the right "session 'x' not found" message for the `capture` verb (no match-arm change expected — verify with a test).
- `src/main.rs`: add the `Commands::Capture { session, lines }` dispatch arm calling `sessions::commands::capture`. Keep it on the sync, DB-free path (no `.await`, no `Config::load()`).
- Unit tests: a `degrade_tmux_error("capture", …, ExitError)` case asserting the not-found message; if `format_capture` lives here, assert it joins lines correctly and handles the empty slice.

### 3. Validate
- Run the Validation Commands listed below and confirm all pass.
- Manually smoke-test: `cargo run -- capture --help` shows the verb and `--lines`; against a live tmux server, `bastion capture <session>` prints recent pane output, `bastion capture <session> --lines 5` bounds it to the last 5 lines, and an unknown session errors clearly (no panic). Record the result in Notes (Coverage bar rule 6).

## Acceptance Criteria
- `bastion capture <session>` prints the recent pane output for a session.
- `--lines N` bounds the output to the last N lines; trailing blank padding from `capture-pane` is not printed; line order is preserved.
- The tail-trimming logic is pure and exhaustively unit-tested against fixtures (more/fewer/exactly N, `Some(0)`, `None`, blank padding, empty input).
- An unknown/bad session name produces a clear error (not a panic), routed through the existing graceful-degradation path; `NotInstalled` / `NoServer` degrade gracefully.
- The capture path runs with Postgres stopped (DB-free, D4) and is fully synchronous (D5).
- All gated checks (`cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`) pass; new tests cover the trimming logic and the capture-verb degradation case.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
<!-- filled in as work happens -->
