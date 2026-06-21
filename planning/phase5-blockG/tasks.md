---
type: TaskSpec
title: Phase 5 Block G — `bastion ask` (one Claude Code turn for an external caller)
description: Add a non-interactive `bastion ask` subcommand that runs a single Claude Code turn against an interactive tmux session via a prompt-file-in / answer-file-out protocol, so the Python orchestrator's CLAUDE_CODE_SESSION provider can run subscription-billed, observable LLM nodes.
---

# Task Spec — Phase 5, Block G

## Goal
Add `bastion ask` — one command that performs a single Claude Code "turn" against an interactive
session (ensure session + Claude up → send a fixed trigger → wait for the answer file) and exits, so an
external caller (the Python orchestrator) gets one stable command instead of choreographing raw
`send`/`capture`.

## Context Pointers
- **Plan:** `planning/master-plan.md` → *Phase 5 — Session Management* → *Block G — `bastion ask`*.
- **Cross-repo contract (load-bearing):** `agentic-portfolio/docs/integrations/claude-code-llm-provider.md`
  §2 defines the `bastion ask` v0.1.0 CLI surface, protocol, and exit contract. **This spec implements
  that contract verbatim** — flag names, the trigger wording, the `<out>.done` marker, and the exit
  semantics must match it. If you need to change the surface, bump §2 there in the same change.
- **Decisions to preserve:** D4 (sessions surface is **DB-free** — no `Config::load()`, no pool),
  D5 (session verbs are **synchronous** blocking `std::process::Command` — no async/tokio),
  D6 (malformed tmux output is skipped with a warning, not fatal).
- **Existing code to reuse (do not re-create):**
  - `src/sessions/tmux.rs` — pure `*_args()` constructors + `run_tmux`; `new_session`, `send_keys`
    (literal text + Enter), `capture_pane`, and a session-exists check (`has_session` / `list-sessions`
    parse). Reuse these; add a thin `*_args()` builder only if a needed tmux call has none.
  - `src/sessions/model.rs` — Block F `classify_state(foreground_cmd) -> SessionState` (to detect
    "claude already running" and skip launch).
  - `src/sessions/claude_state.rs` — Block F `trust_status(dir) -> TrustStatus` / `trust_for_dir`
    (to fail fast on an untrusted `--dir`).
  - `src/sessions/commands.rs` — the shared `degrade_tmux_error` / graceful-degradation helpers and the
    existing verb-handler pattern.
  - `src/cli.rs` — clap subcommand + flag definitions; `src/main.rs` — dispatch.
- **Standing rules (CLAUDE.md):** tests ship with the block (rule 1); OKF frontmatter on every md
  (rule 2); Coverage bar (rule 6) — pure command/arg/trigger construction and path derivation are
  exhaustively unit-tested without I/O; error/timeout paths get explicit cases; the thin I/O shell
  (process spawn + poll loop) is smoke-tested with results recorded in `## Notes`.

## Step-by-Step Tasks

### 1. CLI surface + dispatch for `ask`
- **Files owned:** `src/cli.rs`, `src/main.rs`.
- `cli.rs`: add an `Ask` subcommand matching the contract exactly:
  `--session <String>` (req), `--prompt-file <PathBuf>` (req), `--out <PathBuf>` (req),
  `--dir <PathBuf>` (opt), `--timeout <u64>` (opt, default `180`),
  `--launch-cmd <String>` (opt, default `"claude --permission-mode bypassPermissions"`).
- `main.rs`: dispatch `Commands::Ask { … }` → `sessions::ask::ask(...)`. Keep the DB pool **lazy** —
  `ask` must run with Postgres stopped (D4); do not touch the pool on this path.
- Unit test: clap parses a full `ask` invocation into the expected struct, including the defaults for
  `--timeout` and `--launch-cmd` when omitted.

### 2. Pure helpers — trigger, done-path, readiness/launch builders
- **Files owned:** `src/sessions/ask.rs` *(new — pure section)*, `src/sessions/mod.rs`.
- `mod.rs`: register `pub mod ask;`.
- `ask.rs` pure functions (no I/O), each unit-tested element-by-element:
  - `pub fn done_path(out: &Path) -> PathBuf` — derive `<out>.done` (append `.done` to the full file
    name; test with and without an extension).
  - `pub fn trigger_text(prompt_file: &Path, out: &Path) -> String` — the exact trigger wording from
    the contract: `Read <prompt-file> and follow its instructions exactly. Write your complete answer to
    <out>. When finished, create an empty file <out>.done`. Assert the rendered string contains both
    absolute paths and the marker filename.
  - `pub fn poll_plan(timeout_secs: u64, interval_ms: u64) -> usize` (or equivalent) — pure computation
    of the max poll attempts from the timeout/interval, so the loop bound is testable without sleeping.
  - Confirm reuse of `tmux::send_keys_args` / `send_enter_args` for the trigger (literal text then Enter);
    if `has_session` needs an args builder, add `pub fn has_session_args(name) -> Vec<String>` here and
    test it. No new behavior — just argument vectors asserted element-by-element.

### 3. The `ask` turn — thin I/O shell over the pure helpers
- **Files owned:** `src/sessions/ask.rs` *(I/O section)*.
- **dependsOn:** Tasks 1 and 2 (same new module + needs the CLI struct).
- Implement `pub fn ask(args: AskArgs) -> Result<(), AskError>` (synchronous, D5):
  1. **Trust pre-flight (fail fast):** if `--dir` is given, call Block F `trust_status(dir)`; on
     `Untrusted` return a clear `AskError` (Claude would stall on the one-time prompt) — do not launch.
     `Unknown`/`Trusted` proceed.
  2. **Ensure session + Claude:** if `!has_session(--session)` → `new_session(--session, --dir)`, then
     `send_keys(--launch-cmd)` + Enter; wait for readiness (poll `list-sessions` foreground command via
     Block F `classify_state` until it reads `claude`, bounded by a short readiness budget). If the
     session already exists and `classify_state` reports `claude` running, skip the launch.
  3. **Send the trigger** (from `trigger_text`) via `send_keys` + Enter — the only keystrokes sent.
  4. **Wait for completion:** poll `done_path(--out)` existence up to `--timeout` (bound from
     `poll_plan`); on found → remove the marker, return `Ok(())` (caller reads `--out`). On timeout →
     `capture_pane(--session)` and return `AskError::Timeout` carrying the diagnostics.
  - Define an `AskError` enum (`UntrustedDir`, `Tmux`, `Launch`, `Timeout`) reusing the existing
    `degrade_tmux_error` taxonomy where natural; `main`/dispatch maps it to a non-zero exit and prints
    diagnostics to **stderr** (exit-`0`-only-on-success contract).
- Note: bastion does **not** read or validate `--out` contents — it is payload-agnostic (the caller
  owns JSON vs markdown). It only guarantees the file exists and is complete (the marker was observed).

### 4. Validate
- Run the Validation Commands below and confirm all pass.
- Manual smoke test against a live tmux server + a Claude-trusted dir (Coverage bar, rule 6 — record in
  `## Notes`):
  - Write a tiny prompt file instructing "write the word PONG to <out>". Run
    `bastion ask --session ask-smoke --prompt-file <p> --out <o> --dir <trusted> --timeout 120` →
    `<o>` contains the expected answer, exit code `0`, `<o>.done` cleaned up, and `bastion sessions`
    shows `ask-smoke` as *running (claude)*.
  - Re-run reusing the warm session (no relaunch) → still succeeds.
  - A prompt that never writes the file → exits non-zero after `--timeout` with stderr diagnostics.
  - `--dir <never-opened-dir>` (untrusted/unknown): document the observed behavior (fail-fast on
    `Untrusted`; proceed on `Unknown`).
  - Confirm the whole path runs with Postgres stopped (D4) and synchronously (D5).
  - Clean up smoke sessions with `bastion kill`.

## Acceptance Criteria
- `bastion ask` implements the brain contract v0.1.0 (`docs/integrations/claude-code-llm-provider.md` §2)
  exactly: flags, trigger wording, `<out>.done` marker, and exit semantics (`0` only when `<out>` is
  complete; non-zero with stderr diagnostics on timeout/failure).
- Ensures the session + Claude are up (creating + launching when cold, skipping launch when Claude is
  already running via Block F `classify_state`); sends only the fixed trigger keystrokes.
- An untrusted `--dir` fails fast with a clear message (never stalls on Claude's trust prompt); trust
  is read-only (no write to `~/.claude.json`).
- Pure logic (`done_path`, `trigger_text`, poll-bound, any new `*_args`) is exhaustively unit-tested
  without I/O; the timeout path has an explicit test; the I/O shell is smoke-tested and recorded in
  `## Notes`.
- DB-free (D4) and synchronous (D5) preserved — no `Config::load()`, no pool, no `.await` on this path.
- All gated checks pass and the test baseline increases with the new cases.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes

### Manual smoke test — 2026-06-21 (Fix Pass 2)

Environment: macOS Darwin 24.6.0, tmux 3.6b, Claude Code v2.1.185, bastion release binary.

**Finding: process-name discovery fix (applied during smoke test)**

During smoke testing, we discovered that Claude Code v2.1.185 sets its process name via
`pthread_setname_np` (macOS `ucomm`) to its version string "2.1.185". tmux's
`#{pane_current_command}` uses `ucomm`, so it reports "2.1.185" — not "claude". The
original readiness check `foreground.trim() == "claude"` therefore never passed, causing
`wait_for_claude` to always timeout on cold starts. Fixed by replacing the exact-string
check with `classify_state(&foreground) == SessionState::Running` (any non-idle-shell
foreground process is taken as the signal that Claude is up). The warm-session check in
`ask()` was updated in the same way.

**Scenario 1 — cold start → PONG written → exit 0**

Note: the first cold-start run timed out (90s) because the trigger was sent immediately
after `classify_state` returned Running, before Claude Code's TUI was fully ready to
accept keyboard input. On a second attempt with the session already warm (Claude Code TUI
initialized), the full turn succeeded.

  - Session: `ask-smoke`, dir: `/Users/brandon/Dev/agentic-portfolio` (trusted)
  - Prompt: "Write the single word PONG to the output file."
  - Result: exit 0, `/tmp/bastion-smoke-out.txt` contained "PONG"
  - `.done` marker was found and removed by bastion (not present on exit)
  - Pane output confirmed Claude read the prompt, wrote the file, and created the marker.

**Scenario 2 — warm session reuse (no relaunch) → exit 0**

  - Re-ran immediately with same `--session ask-smoke` (Claude still running)
  - `classify_state == Running` → launch skipped, trigger sent directly
  - Result: exit 0, output "PONG", marker cleaned up. Confirmed via pane that Claude
    received the second trigger without relaunch.

**Scenario 3 — timeout → exit 1 with stderr diagnostics**

  - Same session, `--timeout 1` (too short for Claude to respond)
  - Result: exit 1, stderr printed "timed out after 1s waiting for '/tmp/bastion-smoke-out.txt'"
    plus full `capture-pane` output showing the trigger in Claude's input buffer and Claude
    beginning to process it.

**Scenario 4 — untrusted dir → fail fast, exit 1**

  - `--dir /Users/brandon` (hasTrustDialogAccepted=false in ~/.claude.json)
  - Result: exit 1, stderr: "directory '/Users/brandon' is untrusted ... open the directory
    in Claude interactively once to accept trust, then retry". No tmux session created.

**Scenario 4b — unknown dir → proceeds, no trust error**

  - `--dir /tmp/completely-new-dir` (no entry in ~/.claude.json → Unknown)
  - Result: proceeds past trust check, creates session, launches Claude. Timed out after 3s
    because no real prompt was processed, but the pane capture confirmed Claude Code was
    running in the session. No UntrustedDir error.

**Scenario 5 — D4/D5: DB-free and synchronous**

  - `pg_isready` not found on path (PostgreSQL client tools not installed); Postgres server
    status unknown. D4 is guaranteed structurally: `src/sessions/ask.rs` imports no
    `Config`, `Pool`, or `DATABASE_URL`; `cargo test` includes
    `pure_helpers_require_no_database_url` which removes `DATABASE_URL` and calls all pure
    helpers without error.
  - D5 confirmed: no `async`, `await`, `tokio`, or `.await` anywhere in `ask.rs` (grep
    verified). Execution is `std::process::Command` + `std::thread::sleep` poll loop.

**Cleanup**

  - Session `ask-smoke` killed with `bastion kill ask-smoke`.
  - Temp files `/tmp/bastion-smoke-*.txt` removed.
