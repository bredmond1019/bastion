---
type: TaskSpec
title: Phase 5 Block C — bastion send
description: Send keystrokes into a tmux session without attaching — tmux send-keys wrapper, CLI verb, and escaping correctness.
---

# Task Spec — Phase 5, Block C

## Goal
Implement `bastion send <session> <cmd>` → `tmux send-keys -t <session> <cmd> Enter`, so an action can be triggered in a session from the phone without a full attach.

## Context Pointers
- **Plan:** `planning/master-plan.md` → *Phase 5 — Session Management* → *Block C — `bastion send`* (lines ~179–183).
- **Decisions:** D4 (sessions surface is DB-free), D5 (session verbs are synchronous blocking `std::process::Command` calls — no async/tokio coupling), D6 (tmux-output handling).
- **Repo files to extend (follow the established Block A/B patterns):**
  - `src/sessions/tmux.rs` — pure arg-construction fns + a thin execution fn over `run_tmux`, plus the `TmuxError` taxonomy already defined there.
  - `src/sessions/commands.rs` — verb handler + the existing `degrade_tmux_error` / `apply_degradation` graceful-degradation path.
  - `src/cli.rs` — `Commands` enum (clap subcommands).
  - `src/main.rs` — dispatch match arm.
- **Standing rules:** every block ships with tests (CLAUDE.md rule 1); maintain OKF frontmatter; read-only against Postgres (N/A here — sessions surface is DB-free).

## Step-by-Step Tasks

### 1. tmux send-keys layer (pure args + literal escaping + execution)
- **Files owned:** `src/sessions/tmux.rs` only.
- Add a pure `send_keys_args(session_name: &str, keys: &str) -> Vec<String>` that builds the **literal** send-keys invocation: `tmux send-keys -t <session> -l -- <keys>`. Use `-l` (literal) so the command text is never interpreted as tmux key names (e.g. a command containing `Enter`, `C-c`, or a leading `-x`), and `--` so a command starting with `-` is not parsed as a flag.
- Add a pure `send_enter_args(session_name: &str) -> Vec<String>` that builds `tmux send-keys -t <session> Enter` (the separate Enter keypress — it cannot share the `-l` invocation, since `-l` disables key-name lookup).
- Add an execution fn `send_keys(session_name: &str, keys: &str) -> Result<()>` that runs the literal args, then the Enter args, via the existing `run_tmux`, with `.context(...)` on each. Reuse the existing `TmuxError` classification (`NotInstalled` / `NoServer` / `ExitError`) unchanged — an unknown session surfaces as `ExitError`.
- Unit tests (no live tmux): assert the exact arg vectors for `send_keys_args` and `send_enter_args`, including a multi-word command, a command containing a tmux key-like token (`"echo Enter"`), and a command with a leading hyphen (`"--help"`) — confirming `-l` and `--` are present and the command is a single argv element.

### 2. CLI verb + handler wiring
- **Files owned:** `src/cli.rs`, `src/main.rs`, `src/sessions/commands.rs`.
- **dependsOn:** Task 1 (calls `tmux::send_keys`).
- `src/cli.rs`: add a `Send` variant to `Commands` — `session: String` plus the command tokens. Capture the full multi-word command without requiring the user to quote: use a trailing var-arg `Vec<String>` (`#[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]`) so `bastion send work cargo build --release` is captured intact. Add a doc comment so it shows in `--help`.
- `src/sessions/commands.rs`: add `pub fn send(session_name: &str, keys: &str) -> anyhow::Result<()>` mirroring `new`/`kill`: call `tmux::send_keys`, on `Ok` print a confirmation via a new pure `format_sent(session, keys) -> String` helper, on `Err` route through the existing `apply_degradation("send", session_name, e)`. Confirm `degrade_tmux_error`'s default (non-`new`) branch yields the right "session not found" message for the `send` verb — extend its match only if the `send` wording needs to differ.
- `src/main.rs`: add the `Commands::Send { session, cmd }` dispatch arm; join the `Vec<String>` tokens with a single space into the literal command string and pass to `sessions::commands::send`. Keep it on the sync DB-free path (no `.await`, no `Config::load()`).
- Unit tests: `format_sent` contains the session name and the command text; add a `degrade_tmux_error("send", …, ExitError)` case asserting the not-found message.

### 3. Validate
- Run the Validation Commands listed below and confirm all pass.
- Manually confirm `cargo run -- send --help` shows the verb, and (if a tmux server is available) that `bastion send <session> '<cmd>'` lands the keystrokes in the target pane and errors clearly on an unknown session. Record the manual-smoke result in Notes.

## Acceptance Criteria
- `bastion send <session> <cmd>` sends `<cmd>` followed by Enter to the target tmux session; keystrokes arrive in the pane.
- Multi-word commands and commands containing tmux key-like tokens or a leading hyphen are sent **literally** (verified by `-l` + `--` in the constructed args) and are covered by unit tests.
- An unknown/bad session name produces a clear error (not a panic), routed through the existing graceful-degradation path.
- The send path runs with Postgres stopped (DB-free, D4) and is fully synchronous (D5).
- All gated checks (`cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`) pass; new tests are added for the arg construction, escaping, and formatting.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
<!-- filled in as work happens -->
