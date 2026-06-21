---
type: TaskSpec
title: Phase 5 Block E — Session view in the TUI
description: A ratatui session dashboard (bastion / bastion tui) that lists sessions and binds attach/new/send/kill/quit to keys, built on the Block A–D primitives.
---

# Task Spec — Phase 5, Block E

## Goal
Add a `ratatui` session dashboard (reachable as `bastion` no-arg or `bastion tui`) that lists sessions with status + last line and binds `[a]` attach, `[n]` new, `[s]` inline send, `[k]` kill, `[q]` quit — built entirely on the Block A–D primitives.

## Context Pointers
- **Plan:** `planning/master-plan.md` → *Phase 5 — Session Management* → **Block E — session view in the TUI** (lines 192–200); source-tree note designates `sessions/ui.rs` for "ratatui session view (Block E)" (line 70).
- **Reused primitives (do not modify their behavior):**
  - `src/sessions/tmux.rs` — `list_sessions_raw`, `capture_pane_raw`, `attach_session`, `new_session`, `kill_session`, `send_keys`, `TmuxError`.
  - `src/sessions/model.rs` — `Session`, `SessionState`, `Pane`, `parse_sessions`.
  - `src/sessions/commands.rs` — `Degraded` / `degrade_tmux_error` (reuse for in-TUI error messaging).
- **TUI deps already in `Cargo.toml`:** `ratatui` 0.30, `crossterm` 0.29.
- **CLAUDE.md standing rules:** Rule 1 (tests ship with every block), Rule 6 (Coverage bar — pure logic separated from I/O and tested exhaustively; error/degradation paths tested; thin I/O shell smoke-tested and recorded in `## Notes`). OKF frontmatter on every markdown file.
- **Decisions:** D4 (sessions surface is DB-free — the TUI must open **no** Postgres pool / `Config::load()`), D5 (session verbs are synchronous blocking `std::process::Command`; the session TUI loop is likewise sync, no tokio coupling).

## Step-by-Step Tasks

### 1. Session dashboard state model (`src/sessions/app.rs`)
- Create `src/sessions/app.rs` and register it with `pub mod app;` in `src/sessions/mod.rs`.
- Define a pure `SessionApp` state struct holding: `sessions: Vec<Session>`, `selected: usize`, `mode: Mode`, `input: String`, `status: Option<String>` (transient status/error line), `should_quit: bool`.
- Define `enum Mode { Normal, Input(InputKind) }` and `enum InputKind { New, Send }` for the inline `[n]`/`[s]` prompts, and an `enum Action { Attach(String), New(String), Send { session, keys }, Kill(String), None }` that the event loop will execute.
- Implement pure methods, each unit-tested element-by-element:
  - `select_next()` / `select_prev()` — clamp/wrap navigation; no-op on empty list.
  - `selected_session() -> Option<&Session>`.
  - `set_sessions(Vec<Session>)` — refresh while keeping `selected` in range (clamp when the list shrinks).
  - `push_input(char)` / `backspace_input()` / `take_input() -> String` — input-buffer editing for the inline prompts.
  - `on_key(KeyCode) -> Action` — the pure key→action mapping: in `Normal`, `a/n/s/k/q` and Up/Down/`j`/`k`-nav map to the right transitions (note `k` is "kill" only in Normal but the nav binding is Up/Down + `j`/down; document the chosen binding to avoid the `k` collision); in `Input`, printable chars edit the buffer, Enter commits to the corresponding `Action`, Esc cancels back to `Normal`.
- **Owns:** `src/sessions/app.rs` (new), `src/sessions/mod.rs` (add `pub mod app;` line).
- Exhaustively cover navigation bounds (empty, single, wrap), `set_sessions` clamp, input editing, and every `on_key` branch incl. Esc-cancel and Enter-commit for both `InputKind`s.

### 2. Ratatui render + event loop (`src/sessions/ui.rs`)
- **dependsOn: 1** (uses `SessionApp`, `Mode`, `Action`).
- Create `src/sessions/ui.rs` and register it with `pub mod ui;` in `src/sessions/mod.rs`.
- Pure render helpers (unit-tested, no `Frame`): e.g. `session_row(&Session) -> String` (or a `ratatui::text::Line` builder fed by a pure string fn), a `footer_hint(&Mode) -> String` that renders the key legend / active prompt, and a `status_line(&SessionApp) -> String`. Assert their output strings directly.
- I/O shell (smoke-tested, not unit-tested — record results in `## Notes`):
  - `run() -> anyhow::Result<()>` — enter raw mode + alternate screen via crossterm, build the `ratatui` terminal, loop: draw, poll a crossterm key event with a refresh timeout (re-poll tmux via `list_sessions_raw` + per-session `capture_pane_raw`, reusing `parse_sessions` / `Pane`), feed the key to `SessionApp::on_key`, then execute the returned `Action`.
  - Action execution reuses the Block A–D tmux fns: `New`→`new_session`, `Send`→`send_keys`, `Kill`→`kill_session`; map any `TmuxError` through `degrade_tmux_error` into the app's `status` line rather than crashing the loop.
  - `Attach`: **suspend the TUI** (leave raw mode + alternate screen, restore the terminal), call `tmux::attach_session`, then **re-enter** the TUI and refresh on return — `a` must drop into a real tmux attach and come back cleanly.
  - On `should_quit`, tear down the terminal (leave alternate screen, disable raw mode) and return `Ok(())`. Ensure teardown also runs on the error path so the terminal is never left in raw mode.
- Keep the loop synchronous (D5) — no tokio. Keep it DB-free (D4) — no `Config::load()`, no pool.
- **Owns:** `src/sessions/ui.rs` (new), `src/sessions/mod.rs` (add `pub mod ui;` line — note this file is also appended to by task 1; the `dependsOn` serializes the two edits).

### 3. CLI wiring — no-arg + `bastion tui` entry (`src/cli.rs`, `src/main.rs`)
- **dependsOn: 2** (dispatches into `sessions::ui::run`).
- In `src/cli.rs`: make the dashboard reachable two ways without breaking existing verbs:
  - Add a `Tui` subcommand variant (`/// Launch the interactive session dashboard`).
  - Make `Cli.command` an `Option<Commands>` so bare `bastion` (no subcommand) is valid and resolves to the dashboard. Confirm `bastion --help` and every existing verb still parse.
- In `src/main.rs`: dispatch `None` and `Some(Commands::Tui)` to `sessions::ui::run()` (sync call, consistent with the other session verbs; no `.await`). Keep all existing match arms unchanged.
- **Owns:** `src/cli.rs`, `src/main.rs`.
- Add a `clap`-level test (e.g. `Cli::command()` / `try_parse_from`) asserting that bare `bastion`, `bastion tui`, and a sample existing verb (`bastion sessions`) all parse to the expected variant.

### 4. Validate
- Run the Validation Commands listed below and confirm all pass.
- Manually smoke-test the TUI against a live tmux server and record the results in `## Notes`: launch via bare `bastion` and `bastion tui`; verify the list renders with status + last line and refreshes; exercise `n` (new), `s` (inline send), `k` (kill), arrow/`j` navigation, `a` (attach → `Ctrl-b d` detach → returns to the dashboard cleanly), and `q` (exits with the terminal restored, not stuck in raw mode). Confirm it runs with Postgres stopped (D4).

## Acceptance Criteria
- Bare `bastion` and `bastion tui` both launch the session dashboard; all pre-existing verbs (`status`, `sessions`, `attach`, `new`, `kill`, `send`, `capture`, monitor-track verbs) still parse and behave unchanged.
- The dashboard lists live tmux sessions with status + last line and refreshes on a timer.
- Selection works and the documented keys act on the selected session: `a` drops into a real tmux attach and returns cleanly; `n` creates; `s` sends inline; `k` kills; `q` exits with the terminal restored.
- tmux errors (unknown session, no server, tmux not installed) surface as an in-TUI status message via `degrade_tmux_error` without crashing the loop.
- The TUI opens no Postgres pool / `Config::load()` and runs with Postgres stopped (D4); the loop is synchronous (D5).
- Pure logic (state transitions, `on_key` mapping, render-string helpers) is exhaustively unit-tested incl. error/degradation branches; the I/O shell (`ui::run`) is manually smoke-tested with results recorded in `## Notes`.
- All gated checks pass.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
<!-- filled in as work happens: manual smoke-test results, deferred-choice rationale -->
