---
type: TaskSpec
title: Phase 5 Block F — session activity indicator + Claude trust observer
description: Make the session list tell the truth about what is running (foreground command, not attached-client), and pre-flight Claude Code's one-time workspace-trust prompt by reading ~/.claude.json as a read-only observer.
---

# Task Spec — Phase 5, Block F

## Goal
Make the session list honest about real state — classify each session as *running `<cmd>`* vs *idle shell* from `#{pane_current_command}` instead of the attached-client signal — and pre-flight Claude Code's one-time trust prompt by reading `~/.claude.json` `projects["<dir>"].hasTrustDialogAccepted` as a read-only observer.

## Context Pointers
- **Plan:** `planning/master-plan.md` → *Phase 5 — Session Management* → *Block F — session activity indicator + Claude trust observer* (lines ~202–224).
- **Why it exists:** the phase5-blockE live test surfaced two state-honesty gaps — a running Claude Code session reports `idle` (state keyed on `session_attached`, not the pane's foreground process), and hands-off `send`-launches stall on Claude's per-directory trust prompt. See `planning/status.md` → Decisions & Deviations Log (2026-06-21) and `planning/handoff.md`.
- **Decisions to preserve:** D4 (sessions surface is **DB-free** — no `Config::load()`, no pool), D5 (session verbs are **synchronous** blocking `std::process::Command` — no async/tokio), D6 (malformed tmux output is skipped with a warning, not fatal).
- **Existing code to extend (do not re-create):**
  - `src/sessions/tmux.rs` — `LIST_SESSIONS_FORMAT` const + `FIELD_SEP`; pure `*_args()` constructors; `TmuxError` taxonomy + `run_tmux`.
  - `src/sessions/model.rs` — `Session { name, state, window_count, last_line }`, `SessionState { Running, Idle }`, `parse_session_line` / `parse_sessions` (currently derive state from `parts[1]` = `session_attached`).
  - `src/sessions/commands.rs` — `render_sessions(&[Session]) -> String` (the `bastion sessions` table), the `new` verb handler, and the shared `degrade_tmux_error` / `Degraded` / `apply_degradation` graceful-degradation path.
  - `src/sessions/ui.rs` — `session_row(&Session) -> String` (the TUI list row).
  - `src/sessions/app.rs` — `SessionApp` state model + test helpers that build `Session { … }` literals.
  - `src/sessions/mod.rs` — module registration.
- **Standing rules (CLAUDE.md):** tests ship with the block (rule 1); OKF frontmatter on every md (rule 2); Coverage bar (rule 6) — pure classification/parsing is exhaustively unit-tested against fixtures, error/degradation paths get explicit cases, and the thin I/O shell is smoke-tested with the result recorded in `## Notes`.

## Step-by-Step Tasks

### 1. Activity indicator — derive session state from the pane's foreground command
- **Files owned:** `src/sessions/tmux.rs`, `src/sessions/model.rs`, `src/sessions/commands.rs`, `src/sessions/ui.rs`, `src/sessions/app.rs`.
- `tmux.rs`: append `#{pane_current_command}` to `LIST_SESSIONS_FORMAT` as a 5th tab-separated field (`session_name \t session_attached \t session_windows \t session_activity \t pane_current_command`). Update the const's doc comment to list the new field.
- `model.rs`:
  - Add a pure classifier: `pub fn classify_state(foreground_cmd: &str) -> SessionState`, where command ∈ `{zsh, bash, sh, fish}` (after trimming) ⇒ `SessionState::Idle`; any other non-empty command ⇒ `SessionState::Running`; empty/unknown ⇒ `Idle` (conservative default). Keep the idle-shell set a single named const so the host shell set is easy to confirm/extend.
  - Add `pub foreground_cmd: String` to `Session` so the render layer can show *running `<cmd>`*.
  - Rework `parse_session_line` to split into 5 fields (`splitn(5, …)`), keep the ≥3-field malformed guard, derive `state` from `classify_state(parts[4])` (default to empty string when the 5th field is absent so older/shorter lines still parse as idle, not an error), and populate `foreground_cmd`. The `session_attached` field (`parts[1]`) is no longer the state source — drop its use for state (do not add a new struct field for it).
  - Update the module-level doc comment: the running/idle rule is now keyed on `pane_current_command`, not attached clients.
  - Unit tests (inline fixtures, no live tmux): `classify_state` for each idle shell, for `claude` / `node` / `cargo` / `vim` (running), and for empty input; `parse_session_line` populates `foreground_cmd` and the correct state from a 5-field line; a running-command session that is **detached** still classifies as `Running` (the core bug being fixed); a 5-field idle-shell line classifies `Idle`; existing attached/detached and malformed/empty cases still pass after the rework (update the fixtures to 5 fields).
- `commands.rs`: update `render_sessions` so the state column shows the foreground command for running sessions (e.g. `running (claude)` or a `running  claude` column) and `idle` for idle shells; keep the existing column layout readable. Update its render unit tests to assert the running-command label and the idle label.
- `ui.rs`: update `session_row` the same way so the TUI dashboard row reflects running-`<cmd>` vs idle. Update its unit tests.
- `app.rs`: update the `Session { … }` literal(s) in test helpers to include `foreground_cmd` so the crate compiles; no behavior change.
- **Stretch (optional, only if cheap):** surface "active Ns ago" from the already-parsed `session_activity` epoch in the render — keep it a pure helper with its own test, and do **not** call wall-clock time inside a unit-tested function (pass `now` in). Skip if it complicates the row; it is not an acceptance criterion.

### 2. Trust observer — read ~/.claude.json as a read-only pre-flight
- **Files owned:** `src/sessions/claude_state.rs` *(new)*, `src/sessions/mod.rs`, `src/sessions/commands.rs`.
- **dependsOn:** Task 1 (shares `src/sessions/commands.rs`; merge serializes after Task 1).
- `claude_state.rs` (new module):
  - Define `pub enum TrustStatus { Trusted, Untrusted, Unknown }` (with an `as_str` / `Display` for rendering).
  - Pure parse function over the file contents: `pub fn trust_for_dir(claude_json: &str, dir: &str) -> TrustStatus` — parse JSON with `serde_json`, look up `projects[dir].hasTrustDialogAccepted` (bool): `true` ⇒ `Trusted`, `false` ⇒ `Untrusted`, missing project / missing field / non-bool / unparseable JSON ⇒ `Unknown`. **Never** errors, **never** writes.
  - Thin I/O shell: `pub fn trust_status(dir: &str) -> TrustStatus` that resolves `~/.claude.json` (home dir via `std::env::var("HOME")`), reads it, and delegates to `trust_for_dir`; a missing/unreadable file ⇒ `Unknown` (no error surfaced). This shell is the only non-pure part and is smoke-tested, not unit-tested.
  - Unit tests against inline JSON fixtures: trusted dir (`true`), untrusted dir (`false`), dir absent from `projects`, `projects` key absent, `hasTrustDialogAccepted` absent, non-bool field value, malformed JSON, empty string — each maps to the documented `TrustStatus` and never panics. Confirm the function does not write anything (it has no write path by construction).
- `mod.rs`: register `pub mod claude_state;`.
- `commands.rs`: in the `new` verb handler (which already takes `--dir`), after creating the session, print a one-line trust pre-flight for the resolved directory — e.g. `trust: trusted` / `trust: untrusted (Claude will prompt on first launch)` / `trust: unknown`. It is advisory only: it must **never** block or fail session creation, and `Unknown` is a normal, silently-acceptable outcome. Add a unit test asserting the pure label string for each `TrustStatus` (extract a pure `format_trust(status, dir) -> String` helper so the label is testable without I/O).

### 3. Validate
- Run the Validation Commands below and confirm all pass.
- Manually smoke-test against a live tmux server (Coverage bar, rule 6 — record results in `## Notes`):
  - `bastion sessions` and the TUI show a detached-but-running command session (launch `claude` or `cargo test` in a detached session) as **running `<cmd>`**, and a bare shell session as **idle**.
  - `bastion new <name> --dir <trusted-dir>` prints `trust: trusted`; `--dir <never-opened-dir>` prints `trust: untrusted` or `trust: unknown`; with `~/.claude.json` absent/renamed it prints `trust: unknown` and the session is still created (no error, no write to the file).
  - Confirm the whole path runs with Postgres stopped (DB-free, D4) and synchronously (D5).

## Acceptance Criteria
- `bastion sessions` and the TUI distinguish a **running command** (incl. a live, detached Claude Code session) from an **idle shell**, derived from `pane_current_command` — a detached-but-busy session no longer mislabels as `idle`.
- State classification (`classify_state`) and session-line parsing are pure and exhaustively unit-tested against fixtures (every idle shell, representative running commands, empty input, detached-but-running, 5-field idle).
- The trust observer reports whether a target directory is trusted by reading `~/.claude.json`, returns `Unknown` (never an error) when the file/project/field is absent or malformed, and **never writes** to the file.
- Trust parsing (`trust_for_dir`) is exhaustively unit-tested against JSON fixtures (trusted, untrusted, absent project/field, non-bool, malformed, empty).
- The trust pre-flight is advisory: it never blocks or fails `bastion new`.
- DB-free (D4) and synchronous (D5) invariants preserved; no `Config::load()`, no pool, no `.await` on the sessions path.
- All gated checks (`cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`) pass; the test baseline (145) increases with the new cases.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes

### Manual smoke test — 2026-06-21 (Coverage bar, rule 6)
Verified live against **tmux 3.6b**, DB-free (Postgres not involved on any path):

**Activity indicator** (`bastion sessions`, both sessions detached — no attached client):
- `smoke-idle` (bare shell in `/tmp`) → `idle`.
- `smoke-run` (after `bastion send smoke-run "sleep 300"`) → `running (sleep)`.
- Confirms the core fix: a **detached-but-running** session reports `running (<cmd>)` from
  `#{pane_current_command}`, not `idle` — the attached-client signal would have shown both as idle.

**Trust observer** (`bastion new <name> --dir <dir>` pre-flight line):
- `--dir /Users/brandon/Dev/ai-event-quickstart` (present + `hasTrustDialogAccepted: true` in
  `~/.claude.json`) → `trust: trusted`.
- `--dir /tmp/never-opened-xyz-<pid>` (absent from `projects`) → `trust: unknown`.
- `--dir /tmp` (absent from `projects`) → `trust: unknown`.
- In every case the session was still **created** (advisory only, never blocks). `~/.claude.json`
  was read read-only and never written (no write path exists by construction — `trust_for_dir`
  takes `&str`, `trust_status` only `read_to_string`).

All four smoke sessions cleaned up via `bastion kill`; `bastion sessions` then reports
`no tmux server running` (graceful degradation, D6). Automated suite: **181 passed, 2 ignored**
(+36 over the 145 baseline).
