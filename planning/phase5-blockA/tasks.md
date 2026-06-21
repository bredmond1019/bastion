---
type: TaskSpec
title: Task Spec — Phase 5, Block A — bastion sessions (+ tmux wrapper + lazy DB pool)
description: Stand up the sessions/ module (tmux wrapper + Session/Pane model) and the `bastion sessions` list command, keeping the Postgres pool lazy so session commands run with zero DB.
---

# Task Spec — Phase 5, Block A — `bastion sessions` (+ tmux wrapper + lazy DB pool)

## Goal
Stand up the `sessions/` module (`tmux.rs` wrapper + `model.rs` `Session`/`Pane` types) and implement `bastion sessions` — list tmux sessions with last-line output and a running/idle indicator — while making the Postgres pool lazy so the command runs with zero DB connectivity (D4).

## Context Pointers
- **Plan:** `planning/master-plan.md` → *Phase 5 — Session Management* (track intro) and *Block A* (lines ~146–168), plus the `src/sessions/` directory map and the **Lazy DB pool (D4)** callout in *Architecture / Design Overview*.
- **Decision:** `planning/decisions/D4-session-management-surface.md` — the two-surface split and the one engineering commitment (lazy/on-demand Postgres; session path touches **no** DB).
- **Repo reality (verified):** `src/main.rs` does **not** open a shared pool eagerly — each `db::*` fn opens a short-lived `PgPoolOptions` pool on demand. The only DB coupling on the command path is `config::Config::load()`, which returns `ConfigError::MissingVar("DATABASE_URL")` when unset. The lazy-DB commitment therefore means: the `sessions` dispatch arm must **not** call `Config::load()` and must **not** open any pool.
- **CLAUDE.md standing rules:** every block ships with tests (Rule 1); OKF frontmatter on every markdown file (Rule 2); read-only observer of the orchestrator (Rule 4) — not in play here since this surface is tmux-only; no new dependencies (D4: `std::process::Command` only).
- **No new crate dependencies.** tmux is driven via `std::process::Command`. Do not add to `Cargo.toml`.

## Step-by-Step Tasks

### 1. tmux wrapper + sessions module scaffold
**Owns (primary files):** `src/sessions/tmux.rs` (new), `src/sessions/mod.rs` (new), `src/main.rs` (add `mod sessions;` only).
- Create `src/sessions/mod.rs` declaring `pub mod tmux;` (later tasks append `model` / `commands`). Keep it minimal — module declarations + a short header comment referencing D4.
- Create `src/sessions/tmux.rs`: a thin wrapper over `std::process::Command` → the `tmux` CLI. Provide:
  - A small function to build the argument vector for `list-sessions` with an explicit `-F` format string (so output parsing in Task 2 is deterministic). Keep the format string a named `const` so the model parser and the wrapper agree on field order/separator.
  - A function to build args for `capture-pane -p -t <session>` (used to fetch a session's last output line).
  - An executor that runs a built command and returns `Result<String>` (stdout), mapping a missing `tmux` binary (`ErrorKind::NotFound`) and a non-zero exit (no server running) into clear, typed/`anyhow` errors — never a panic.
  - Split **command construction** (pure, returns `Vec<String>` / args) from **execution** (does I/O) so construction is unit-testable without spawning tmux.
- In `src/main.rs`, add `mod sessions;` alongside the existing module declarations. Do **not** add a dispatch arm yet (Task 3 does that, after the CLI variant exists).
- **Tests:** unit tests asserting the constructed `list-sessions` and `capture-pane` argument vectors are exactly correct (binary name, flags, format string, target). No live tmux required.

### 2. Session / Pane model + tmux-output parsing  *(dependsOn: 1)*
**Owns (primary files):** `src/sessions/model.rs` (new), `src/sessions/mod.rs` (append `pub mod model;` — append-only).
- Create `src/sessions/model.rs` with `Session` (name, attached flag, window count, last-line output, running/idle indicator) and a `Pane` type as needed by the format chosen in Task 1.
- Implement pure parsing functions: one `list-sessions -F`-formatted line → `Session`, and the full multi-line output → `Vec<Session>`. Derive the running/idle indicator from the available tmux fields (e.g. attached state / activity) — document the rule in a comment.
- Append exactly one line (`pub mod model;`) to `src/sessions/mod.rs`. This is the only edit to that shared file in this task.
- **Tests:** parse captured `tmux list-sessions` / `capture-pane` output **fixtures** (inline `const` strings or a `src/sessions/fixtures/` dir owned by this task) into `Session`/`Pane`. Cover: multiple sessions, an attached vs detached session, a session with empty/blank last line, and a malformed line (graceful skip or typed error — pick one and test it). No live tmux in CI.

### 3. `bastion sessions` command + CLI wiring + lazy-DB guarantee  *(dependsOn: 1, 2)*
**Owns (primary files):** `src/sessions/commands.rs` (new), `src/cli.rs` (add `Sessions` variant), `src/sessions/mod.rs` (append `pub mod commands;` + public entry), `src/main.rs` (add dispatch arm).
- Add `src/sessions/commands.rs` implementing the `sessions` list verb: call the tmux wrapper (Task 1) → parse output (Task 2) → for each session fetch its last pane line via `capture-pane -p` → render a plain-text table (session name, running/idle indicator, last line). Reuse the `run::render_status` pattern: a **pure** render function (input: `&[Session]`) returning a `String`, plus a thin I/O entry that gathers data and prints it.
- Expose a public entry from `src/sessions/mod.rs` (e.g. `pub async fn run() -> Result<()>` or sync — sessions does no async I/O, so a plain `fn` is acceptable; keep the `main.rs` call site consistent). Append `pub mod commands;` to `mod.rs`.
- Add a `Sessions` variant to the `Commands` enum in `src/cli.rs` (`/// List tmux sessions with last-line output`).
- Add the dispatch arm in `src/main.rs`: `Commands::Sessions => sessions::run()...`. **This arm must not call `Config::load()` and must not open a Postgres pool** — the sessions path stays DB-free (D4). Confirm by inspection that nothing on this path references `config`/`db`.
- **Graceful degradation:** when tmux is absent or no server is running, print a clear human message (e.g. "no tmux server running" / "tmux not installed") and exit non-fatally — do not panic and do not surface a raw error trace.
- **Tests:** unit-test the pure render function against a `Vec<Session>` (including the empty case → a friendly "no sessions" line). Add a test asserting the sessions code path compiles/runs without `DATABASE_URL` set — e.g. call the render/parse path directly with env unset, demonstrating no `Config::load()` dependency. (The architectural guarantee is the deliverable; the test locks it in.)

### 4. Validate
- Run the Validation Commands listed below and confirm all pass.
- Manually smoke-test where a tmux server is available: `cargo run -- sessions` lists real sessions with last-line output and a running/idle indicator; rerun with `DATABASE_URL` unset and Postgres stopped to confirm it still works.

## Acceptance Criteria
- `bastion sessions` lists real tmux sessions, each with its last pane output line and a running/idle indicator.
- The command runs with **Postgres stopped and `DATABASE_URL` unset** — the sessions path never calls `Config::load()` and never opens a pool (verified by inspection + a test).
- tmux command **construction** (args for `list-sessions`, `capture-pane`) is unit-tested without spawning tmux.
- Captured `tmux` output **fixtures** parse into `Session`/`Pane` in unit tests (no live tmux required in CI), covering attached/detached, empty last line, and malformed input.
- Missing `tmux` binary or no running server produces a clear, non-panicking message.
- No new crate dependencies added to `Cargo.toml` (tmux driven via `std::process::Command`).
- All gated checks (`planning/harness.json` → `validation.checks[]`) pass.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
- Repo reality at authoring time: no eager pool in `main.rs`; the lazy-DB work is "keep the sessions path off `Config::load()` and off the pool," not "refactor an eager pool." If a later block introduces a shared eager pool, revisit this constraint.
- `ui.rs` (session TUI view) is **Block E**, not this block. `attach`/`new`/`kill` are **Block B**, `send` **C**, `capture` verb **D**. Block A ships only the `sessions` list verb (it may use `capture-pane` internally for last-line output, but not expose a `capture` command).
