---
type: TaskSpec
title: Phase 5 Block B — attach / new / kill (session lifecycle)
description: Task spec for the bastion session lifecycle verbs — attach, new, kill — built on the Phase 5 Block A tmux wrapper.
---

# Task Spec — Phase 5, Block B

## Goal
Add the session-lifecycle verbs `bastion attach <session>`, `bastion new <session> [--dir PATH]`, and `bastion kill <session>` — create, enter, and dispose tmux sessions.

## Context Pointers
- **Plan:** `planning/master-plan.md` → *Phase 5 — Session Management* → *Block B — `bastion attach` / `new` / `kill`*.
- **Decisions:** D4 (sessions surface is DB-free — no `Config::load()`, no Postgres pool), D5 (session verbs are synchronous blocking `std::process::Command` calls — no async/tokio coupling), D6 (malformed tmux lines are skipped, not fatal). All in `planning/decisions/`.
- **Existing code to extend (Block A patterns to follow):**
  - `src/sessions/tmux.rs` — pure command *construction* (returns `Vec<String>`) separated from *execution* (`run_tmux`); `TmuxError` enum with `NotInstalled`/`NoServer`/`ExitError`.
  - `src/sessions/commands.rs` — verb entry points with graceful `TmuxError` degradation to human messages.
  - `src/cli.rs` — `Commands` subcommand enum.
  - `src/main.rs` — sync dispatch for the DB-free sessions path (e.g. `Commands::Sessions => sessions::run()`).
- **Standing rules (`CLAUDE.md`):** every block ships with tests; OKF frontmatter on markdown; sessions path stays DB-free (lazy pool, D4); all gated checks (`planning/harness.json`) must pass.

## Step-by-Step Tasks

### 1. tmux command construction + execution for attach / new / kill
**Owns:** `src/sessions/tmux.rs` (only).
- Add pure construction functions mirroring the Block A style (first element is `"tmux"`):
  - `attach_args(session_name: &str) -> Vec<String>` → `tmux attach -t <session>`.
  - `new_session_args(session_name: &str, dir: Option<&str>) -> Vec<String>` → `tmux new-session -d -s <session>`, appending `-c <dir>` when `dir` is `Some`.
  - `kill_session_args(session_name: &str) -> Vec<String>` → `tmux kill-session -t <session>`.
- Add execution helpers that reuse `run_tmux` for the non-interactive verbs:
  - `new_session(session_name, dir) -> Result<()>` and `kill_session(session_name) -> Result<()>` (discard stdout; map errors as Block A does).
- Add an **interactive** attach helper `attach_session(session_name) -> Result<()>` that hands the terminal to tmux: build args via `attach_args`, run with `std::process::Command::status()` so the child **inherits the parent's stdio** and the call blocks until the user detaches, then returns to the shell. Map `NotFound` → `TmuxError::NotInstalled` consistent with `run_tmux`. (Do **not** use `exec()`-style process replacement — `.status()` returns control cleanly on detach and avoids skipping process teardown.)
- Ensure unknown/missing-session failures surface as `TmuxError::ExitError` (tmux exits non-zero with a "can't find session" stderr) so callers can present a clear message.
- **Tests (pure, no live tmux):** assert exact arg vectors for `attach_args`, `new_session_args` with and without `dir`, and `kill_session_args` (name, order, length) — same shape as the existing `*_args_correct` tests.

### 2. CLI verbs + dispatch wiring + command handlers
**Owns:** `src/sessions/commands.rs`, `src/cli.rs`, `src/main.rs`. **dependsOn: 1** (calls the new `tmux` functions; serialized to avoid touching `tmux.rs` concurrently).
- In `src/sessions/commands.rs`, add three entry points following the `run()` graceful-degradation pattern (downcast to `TmuxError`, print a human message for `NotInstalled`/`NoServer`, return `Err` otherwise):
  - `pub fn attach(session_name: &str) -> anyhow::Result<()>` → calls `tmux::attach_session`.
  - `pub fn new(session_name: &str, dir: Option<&str>) -> anyhow::Result<()>` → calls `tmux::new_session`; on success print a confirmation (e.g. `created session '<name>'`).
  - `pub fn kill(session_name: &str) -> anyhow::Result<()>` → calls `tmux::kill_session`; on success print a confirmation (e.g. `killed session '<name>'`).
  - On `ExitError` (e.g. unknown session), surface a clear message naming the session rather than a raw tmux dump.
- In `src/cli.rs`, add three subcommands to the `Commands` enum:
  - `Attach { session: String }`
  - `New { session: String, #[arg(long)] dir: Option<PathBuf> }`
  - `Kill { session: String }`
- In `src/main.rs`, wire the dispatch arms in the DB-free sessions style (sync, no `Config::load()`, no pool):
  - `Commands::Attach { session } => sessions::commands::attach(&session)`
  - `Commands::New { session, dir } => sessions::commands::new(&session, dir.as_deref().and_then(|p| p.to_str()))`
  - `Commands::Kill { session } => sessions::commands::kill(&session)`
- Re-export or path as needed; keep `sessions::run` for the existing list verb intact.
- **Tests:** unit-test the handler logic that does not spawn tmux — e.g. confirmation-message formatting helpers (extract a pure `format_created(name)` / `format_killed(name)` if useful so they are testable without I/O), mirroring how `render_sessions` is tested in isolation. Do not require a live tmux server in CI.

### 3. Validate
- Run the Validation Commands listed below and confirm all pass.
- Manually smoke-test against a real tmux server (outside CI): `bastion new tmp --dir /tmp` creates a detached session in `/tmp`; `bastion sessions` lists it; `bastion attach tmp` enters and returns cleanly on `Ctrl-b d`; `bastion kill tmp` removes it; `bastion kill nope` gives a clear unknown-session error.

## Acceptance Criteria
- `bastion attach <session>` execs into `tmux attach -t <session>`, inherits the terminal, and returns to the shell on detach.
- `bastion new <session>` creates a detached session via `tmux new-session -d -s`; `--dir PATH` adds `-c PATH` and the session starts in that directory.
- `bastion kill <session>` removes the session via `tmux kill-session -t`.
- Unknown/bad session names produce a clear, human-readable error (not a raw tmux stderr dump), and missing tmux / no server degrade gracefully like the `sessions` verb.
- Command-construction logic for all three verbs is unit-tested without spawning tmux; the sessions path remains DB-free (no `Config::load()`, no Postgres pool).
- All gated checks pass.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
<!-- filled in as work happens -->
