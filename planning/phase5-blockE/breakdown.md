---
type: TaskBreakdown
title: Phase 5 Block E — Session view in the TUI (breakdown)
description: Atomic, agent-executable sub-steps for the ratatui session dashboard, with Task 2 (render + event loop) fully decomposed.
---

# Task Breakdown — Phase 5 Block E — Session view in the TUI

## Source Spec
`planning/phase5-blockE/tasks.md`

## Goal
Add a `ratatui` session dashboard (reachable as `bastion` no-arg or `bastion tui`) that lists sessions with status + last line and binds `[a]` attach, `[n]` new, `[s]` inline send, `[k]` kill, `[q]` quit — built entirely on the Block A–D primitives.

## How to Use
Work top to bottom. Each sub-step is a single atomic action. Run the inline **Verify**
checks as you go — do not batch them at the end. Each check must pass before continuing.

---

## Steps

### Step 1: Session dashboard state model (`src/sessions/app.rs`)

#### 1.1 Create `src/sessions/app.rs` with the type skeleton
**File:** `src/sessions/app.rs` (new)
**Action:** create file with imports + type definitions (no logic yet).
- Module doc comment: state model for the session TUI; pure (no I/O, no DB — D4/D5); the event loop in `ui.rs` owns all I/O.
- Imports: `use crate::sessions::model::Session;` and `use crossterm::event::KeyCode;`.
- `#[derive(Debug, Clone, PartialEq)] pub enum InputKind { New, Send }`
- `#[derive(Debug, Clone, PartialEq)] pub enum Mode { Normal, Input(InputKind) }`
- `#[derive(Debug, Clone, PartialEq)] pub enum Action { Attach(String), New(String), Send { session: String, keys: String }, Kill(String), None }`
- `pub struct SessionApp { pub sessions: Vec<Session>, pub selected: usize, pub mode: Mode, pub input: String, pub status: Option<String>, pub should_quit: bool }`

#### 1.2 Register the module in `src/sessions/mod.rs`
**File:** `src/sessions/mod.rs`
**Action:** add `pub mod app;` to the existing `pub mod` block (alphabetical: before `pub mod commands;`).
> ⚠️ Shared-file note: `mod.rs` is also edited in 2.2. Steps 1 and 2 are sequential (`dependsOn`), so the two `pub mod` lines never merge-conflict — keep each edit to its own line.

#### 1.3 Implement constructor + navigation
**File:** `src/sessions/app.rs`
**Action:** `impl SessionApp`:
- `pub fn new(sessions: Vec<Session>) -> Self` — `selected: 0`, `mode: Mode::Normal`, `input: String::new()`, `status: None`, `should_quit: false`.
- `pub fn select_next(&mut self)` — no-op if `sessions.is_empty()`; else `selected = (selected + 1) % sessions.len()` (wrap).
- `pub fn select_prev(&mut self)` — no-op if empty; else wrap down (`if selected == 0 { len - 1 } else { selected - 1 }`).
- `pub fn selected_session(&self) -> Option<&Session>` — `self.sessions.get(self.selected)`.
- `pub fn set_sessions(&mut self, sessions: Vec<Session>)` — replace list; clamp `selected` to `len.saturating_sub(1)` when the new list is non-empty, set `selected = 0` when empty.

#### 1.4 Implement input-buffer editing
**File:** `src/sessions/app.rs`
**Action:** on `impl SessionApp`:
- `pub fn push_input(&mut self, c: char)` — `self.input.push(c)`.
- `pub fn backspace_input(&mut self)` — `self.input.pop()` (ignore result).
- `pub fn take_input(&mut self) -> String` — `std::mem::take(&mut self.input)` (returns the buffer and clears it).

#### 1.5 Implement the pure key→action mapping `on_key`
**File:** `src/sessions/app.rs`
**Action:** `pub fn on_key(&mut self, key: KeyCode) -> Action`. Binding decisions (document inline to resolve the `k` collision):
- **Normal mode** (`Mode::Normal`):
  - `Down` or `Char('j')` → `self.select_next()`, return `Action::None`.
  - `Up` or `Char('k')`? — **No**: `k` is reserved for kill. Use `Up` only (plus `Char('j')`/`Down` for next). Document: navigation is arrow-key + `j` (down); `k` is the kill verb, not nav-up. So `Up` → `select_prev()`, return `Action::None`.
  - `Char('a')` → if `selected_session()` is `Some(s)`, return `Action::Attach(s.name.clone())`; else `Action::None`.
  - `Char('n')` → `self.mode = Mode::Input(InputKind::New)`, clear `input`, return `Action::None`.
  - `Char('s')` → if a session is selected, `self.mode = Mode::Input(InputKind::Send)`, clear `input`; else set `status = Some("no session selected".into())`; return `Action::None`.
  - `Char('k')` → if `Some(s)`, return `Action::Kill(s.name.clone())`; else `Action::None`.
  - `Char('q')` → `self.should_quit = true`, return `Action::None`.
  - any other key → `Action::None`.
  - On any successful action dispatch, clear `self.status` first (transient).
- **Input mode** (`Mode::Input(kind)`):
  - `Char(c)` → `self.push_input(c)`, return `Action::None`.
  - `Backspace` → `self.backspace_input()`, return `Action::None`.
  - `Esc` → `self.mode = Mode::Normal`, clear `input`, return `Action::None`.
  - `Enter` → read `kind`; `let text = self.take_input(); self.mode = Mode::Normal;` then:
    - `InputKind::New` → return `Action::New(text)` (empty text → `Action::None` + `status = Some("session name required")`).
    - `InputKind::Send` → if a session is selected, return `Action::Send { session: name, keys: text }`; else `Action::None`.
  - any other key → `Action::None`.

#### 1.6 Add exhaustive unit tests
**File:** `src/sessions/app.rs`
**Action:** `#[cfg(test)] mod tests` with a `fn make_sessions(names: &[&str]) -> Vec<Session>` helper (build `Session { name, state: SessionState::Idle, window_count: 1, last_line: String::new() }`). Cover:
- `new_starts_at_zero_normal_mode`
- `select_next_wraps`, `select_prev_wraps`, `select_next_empty_is_noop`, `select_prev_empty_is_noop`
- `single_session_next_prev_stay_at_zero`
- `set_sessions_clamps_selected_when_list_shrinks`, `set_sessions_empty_resets_to_zero`
- `selected_session_returns_none_when_empty`
- `push_backspace_take_input_roundtrip`
- `on_key_j_and_down_advance`, `on_key_up_retreats`
- `on_key_a_returns_attach_with_selected_name`, `on_key_a_empty_list_is_none`
- `on_key_n_enters_new_input_mode`
- `on_key_s_enters_send_input_mode_when_selected`, `on_key_s_no_selection_sets_status`
- `on_key_k_returns_kill_with_selected_name`
- `on_key_q_sets_should_quit`
- `input_mode_char_appends`, `input_mode_backspace_pops`, `input_mode_esc_cancels_to_normal`
- `input_mode_enter_new_returns_new_action`, `input_mode_enter_new_empty_sets_status`
- `input_mode_enter_send_returns_send_action`, `input_mode_enter_send_no_selection_is_none`

**Verify:** `cargo test sessions::app` → all new tests pass; `cargo clippy -- -D warnings` → clean.

---

### Step 2: Ratatui render + event loop (`src/sessions/ui.rs`) — dependsOn 1

#### 2.1 Create `src/sessions/ui.rs` with module doc + imports
**File:** `src/sessions/ui.rs` (new)
**Action:** create file.
- Module doc: ratatui session dashboard; the thin I/O shell over the pure `SessionApp`; synchronous loop (D5), DB-free (D4 — no `Config::load`, no pool).
- Imports: `use crate::sessions::app::{Action, InputKind, Mode, SessionApp};`, `use crate::sessions::model::{Pane, parse_sessions, Session};`, `use crate::sessions::tmux::{self, TmuxError};`, `use crate::sessions::commands::{degrade_tmux_error, Degraded};`, `use anyhow::Result;`, plus crossterm (`event::{self, Event, KeyCode}`, `terminal::{enable_raw_mode, disable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen}`, `execute`) and ratatui (`Terminal`, `backend::CrosstermBackend`, `Frame`, `widgets::{Block, Borders, List, ListItem, ListState, Paragraph}`, `layout::{Layout, Constraint, Direction}`, `style::{Style, Modifier}`, `text::Line`).

#### 2.2 Register the module in `src/sessions/mod.rs`
**File:** `src/sessions/mod.rs`
**Action:** add `pub mod ui;` to the `pub mod` block (after `pub mod tmux;`). (Sequential with 1.2 via dependsOn — no conflict.)

#### 2.3 Pure render-string helpers
**File:** `src/sessions/ui.rs`
**Action:** add pure functions (no `Frame`, fully unit-testable):
- `pub fn session_row(s: &Session) -> String` — `format!("{:<20} {:<8} {}", s.name, s.state.as_str(), last)` where `last` is `"(no output)"` when `s.last_line` is empty (mirror `commands::render_sessions`).
- `pub fn footer_hint(mode: &Mode) -> String` — `Mode::Normal` → `"[a]ttach [n]ew [s]end [k]ill [q]uit  ↑/j move"`; `Mode::Input(InputKind::New)` → `"new session name (Enter=create, Esc=cancel): "`; `Mode::Input(InputKind::Send)` → `"send to selected (Enter=send, Esc=cancel): "`.
- `pub fn status_line(app: &SessionApp) -> String` — `app.status.clone().unwrap_or_default()`; in input mode, append the live `app.input` buffer to the footer prompt (compose `footer_hint(&app.mode)` + `app.input`).

#### 2.4 `draw` — the ratatui frame builder
**File:** `src/sessions/ui.rs`
**Action:** `fn draw(frame: &mut Frame, app: &SessionApp, list_state: &mut ListState)`:
- Vertical `Layout`: `[Constraint::Min(1), Constraint::Length(1)]` → list area + footer area.
- Build `List` from `app.sessions.iter().map(|s| ListItem::new(Line::from(session_row(s))))`, wrapped in a bordered `Block::default().title("sessions").borders(Borders::ALL)`, with `.highlight_style(Style::default().add_modifier(Modifier::REVERSED))`.
- `list_state.select(Some(app.selected))` when non-empty, else `select(None)`.
- Render the list with `frame.render_stateful_widget(list, areas[0], list_state)`.
- Footer: `Paragraph::new(status_line(app))` into `areas[1]`.
- When `app.sessions` is empty, render a `Paragraph` "no sessions — press [n] to create one" in the list area instead of the list.

#### 2.5 Action execution helper
**File:** `src/sessions/ui.rs`
**Action:** `fn execute_action(action: Action, app: &mut SessionApp)`:
- `Action::None` → return.
- `Action::New(name)` → `tmux::new_session(&name, None)`; on `Ok` set `app.status = Some(format!("created '{name}'"))`; on `Err` → `set_tmux_status(app, "new", &name, e)`.
- `Action::Send { session, keys }` → `tmux::send_keys(&session, &keys)`; on `Ok` set status `sent to '{session}'`; on `Err` → degrade.
- `Action::Kill(name)` → `tmux::kill_session(&name)`; on `Ok` status `killed '{name}'`; on `Err` → degrade. (Attach is handled in the loop, not here — it must suspend the terminal.)
- Helper `fn set_tmux_status(app, verb, name, e: anyhow::Error)`: if `e.downcast_ref::<TmuxError>()` is `Some(te)`, map via `degrade_tmux_error(verb, name, te)` → `Degraded::Graceful(m) | Degraded::Fatal(m)` both write `app.status = Some(m)` (the TUI never aborts on a per-action tmux error); else `app.status = Some(e.to_string())`.

#### 2.6 tmux poll → `Vec<Session>` (refresh)
**File:** `src/sessions/ui.rs`
**Action:** `fn poll_sessions() -> Vec<Session>`:
- `tmux::list_sessions_raw()` → on `Err` return `Vec::new()` (degraded; the empty-state UI covers it).
- `parse_sessions(&raw)`, then for each session call `tmux::capture_pane_raw(&s.name)`; on `Ok(out)` set `s.last_line = Pane::new(&s.name, out).last_line().to_string()`; on `Err` leave empty. (Same enrichment as `commands::run`.)

#### 2.7 The event loop `run` (I/O shell — manual smoke test, not unit-tested)
**File:** `src/sessions/ui.rs`
**Action:** `pub fn run() -> Result<()>`:
- Setup: `enable_raw_mode()?`; `execute!(stdout, EnterAlternateScreen)?`; build `Terminal::new(CrosstermBackend::new(stdout))?`. Wrap the loop body so teardown always runs (extract `fn run_inner(terminal, app) -> Result<()>` and run teardown after it, propagating its result — terminal must never be left in raw mode on the error path).
- Initialize `let mut app = SessionApp::new(poll_sessions());` and `let mut list_state = ListState::default();`.
- Loop:
  - `terminal.draw(|f| draw(f, &app, &mut list_state))?;`
  - `if event::poll(Duration::from_millis(REFRESH_MS))? {` (const `REFRESH_MS: u64 = 2000`, matching the 2s poll cadence used elsewhere) `if let Event::Key(k) = event::read()? {` dispatch:
    - `let action = app.on_key(k.code);`
    - `if let Action::Attach(name) = &action { ` suspend → attach → restore: `disable_raw_mode()?; execute!(terminal.backend_mut(), LeaveAlternateScreen)?;` then `let res = tmux::attach_session(name);` then re-enter `enable_raw_mode()?; execute!(terminal.backend_mut(), EnterAlternateScreen)?; terminal.clear()?;` and on `Err` route through `set_tmux_status(&mut app, "attach", name, e)`; then `app.set_sessions(poll_sessions());` `continue;` `}`
    - else `execute_action(action, &mut app);`
  - `} else { app.set_sessions(poll_sessions()); }` (timeout tick → refresh list).
  - `if app.should_quit { break; }`
- Teardown helper: `disable_raw_mode()?; execute!(stdout, LeaveAlternateScreen)?;`.

#### 2.8 Unit tests for the pure helpers
**File:** `src/sessions/ui.rs`
**Action:** `#[cfg(test)] mod tests`:
- `session_row_running_includes_name_state_lastline`
- `session_row_empty_lastline_shows_placeholder`
- `footer_hint_normal_lists_all_keys`
- `footer_hint_input_new_and_send_differ`
- `status_line_empty_when_no_status_normal`
- `status_line_input_mode_composes_prompt_and_buffer`
> Note: `draw`/`run`/`execute_action`/`poll_sessions` are the I/O shell — exercised by the Step 4 manual smoke test, not unit-tested (CLAUDE.md Rule 6).

**Verify:** `cargo test sessions::ui` → pure-helper tests pass; `cargo build --release` → compiles (loop wiring sound).

---

### Step 3: CLI wiring — no-arg + `bastion tui` entry (`src/cli.rs`, `src/main.rs`) — dependsOn 2

#### 3.1 Make the top-level subcommand optional + add `Tui`
**File:** `src/cli.rs`
**Action:**
- Change `pub command: Commands,` → `pub command: Option<Commands>,` (with `#[command(subcommand)]` retained — clap treats an `Option` subcommand as not-required, so bare `bastion` parses).
- Add a variant to `enum Commands`: `/// Launch the interactive session dashboard` then `Tui,`.

#### 3.2 Dispatch `None` and `Tui` to the dashboard
**File:** `src/main.rs`
**Action:** in `match cli.command`:
- Change the scrutinee handling: since `cli.command` is now `Option`, match on it. Add arms `None => sessions::ui::run(),` and `Some(Commands::Tui) => sessions::ui::run(),` and wrap the existing arms as `Some(Commands::Monitor { .. }) => ...` etc. (Simplest: `match cli.command { None | Some(Commands::Tui) => sessions::ui::run(), Some(cmd) => match cmd { ...existing arms... } }`.) Keep every existing arm's body unchanged. `sessions::ui::run()` is a sync call — no `.await`.

#### 3.3 Add CLI parse tests
**File:** `src/cli.rs`
**Action:** `#[cfg(test)] mod tests` with `use super::*;` and `use clap::Parser;`:
- `bare_bastion_parses_to_none` — `Cli::try_parse_from(["bastion"]).unwrap().command.is_none()`.
- `tui_subcommand_parses` — `matches!(Cli::try_parse_from(["bastion","tui"]).unwrap().command, Some(Commands::Tui))`.
- `existing_verb_still_parses` — `matches!(Cli::try_parse_from(["bastion","sessions"]).unwrap().command, Some(Commands::Sessions))`.

**Verify:** `cargo test cli` → 3 tests pass; `cargo run -- --help` → lists `tui`; `cargo run -- sessions` still works.

---

### Step 4: Validate

#### 4.1 Run the full gated suite
**Action:** run each Validation Command below in order; all must pass.

#### 4.2 Manual smoke test against live tmux + record in spec `## Notes`
**Action:** with a tmux server running, verify and record results in `planning/phase5-blockE/tasks.md` → `## Notes`:
- Launch via bare `bastion` and `bastion tui` — list renders with status + last line, refreshes on the timer.
- `n` → type a name → Enter creates a session (visible on next refresh); Esc cancels.
- `s` → type a command → Enter sends it to the selected session (`capture` confirms arrival).
- `k` → kills the selected session.
- arrow/`j` navigation moves the highlight.
- `a` → drops into a real tmux attach; `Ctrl-b d` returns to the dashboard cleanly.
- `q` → exits with the terminal restored (not stuck in raw mode).
- Run with Postgres stopped → the dashboard still works (D4).

**Verify:** all four Validation Commands green + `## Notes` updated with smoke-test results.

---

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
- **Shared file `src/sessions/mod.rs`:** edited in 1.2 (`pub mod app;`) and 2.2 (`pub mod ui;`). Step 2 `dependsOn` Step 1, so these serialize into different waves — no merge conflict. If this block is ever run with parallel waves, treat `mod.rs` as append-only.
- **`k` key collision:** `k` is the kill verb in Normal mode, so it is NOT bound to nav-up. Navigation is `Up`/`Down` arrows + `j` for down only. This is a deliberate deviation from common vim `j`/`k` paddling — documented in 1.5 to avoid an accidental kill on an up-press.
- **Reuse, don't reimplement:** `degrade_tmux_error` + `Degraded` already live in `src/sessions/commands.rs` (pub) and map every `TmuxError` variant to a user message — the TUI routes per-action errors through it into `app.status` instead of crashing. `Pane`/`parse_sessions` enrichment mirrors `commands::run`.
- **D5 (sync loop):** the TUI loop is plain blocking `std::process::Command` + crossterm polling — no tokio. `main.rs` dispatches it without `.await`, consistent with the other session verbs.
- **crossterm/ratatui versions:** `crossterm` 0.29, `ratatui` 0.30 (already in `Cargo.toml`). Confirm the `ListState`/`render_stateful_widget` and `EnterAlternateScreen` import paths against 0.30 during 2.1 — minor path shifts are possible across ratatui majors.
