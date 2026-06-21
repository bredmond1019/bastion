---
type: TaskSpec
title: Chore — sessions error-path test coverage
description: Close the two untested error-handling gaps in the Phase 5 sessions surface — command-handler TmuxError degradation and run_tmux stderr classification — by extracting pure helpers and testing them.
---

# Chore: sessions error-path test coverage

## Metadata
prompt: `Add tests for sessions error-handling gaps: (#1) command-handler TmuxError degradation branches in src/sessions/commands.rs attach/new/kill, and (#2) run_tmux stderr classification in src/sessions/tmux.rs. Extract a pure classify_tmux_error(stderr) helper to make the NoServer/ExitError mapping testable, and make the handler error mapping testable.`

## Chore Description
The Phase 5 sessions surface has strong coverage of pure logic (arg construction, parsing,
rendering) but two pieces of real branching logic are untested:

- **#1 — Command-handler error degradation.** `attach` / `new` / `kill` in
  `src/sessions/commands.rs` each `match` on `TmuxError::{NotInstalled, NoServer, ExitError}`
  to decide the printed message and whether to return `Ok(())` (graceful) or `Err` (fatal).
  None of these branches are exercised.
- **#2 — `run_tmux` stderr classification.** `src/sessions/tmux.rs` maps tmux stderr containing
  `"no server running"` / `"error connecting to"` / `"No such file or directory"` to
  `TmuxError::NoServer`; this pure string logic is untested.

Both are testable today **without a live tmux server** if the branching is lifted out of the
I/O functions into pure helpers. This chore extracts those helpers and tests them. It does
**not** test the actual process-spawning execution helpers (`run_tmux`, `new_session`,
`kill_session`, `attach_session` I/O) — those remain covered by manual smoke testing, consistent
with the project's construction-vs-execution split (D5) and the DB-free sessions design (D4).

Constraints: sessions path stays DB-free (D4); no behavior change to the CLI output — this is a
pure refactor-for-testability plus new tests. All gated checks must stay green.

## Relevant Files
- `src/sessions/tmux.rs` — extract a pure `classify_no_server(stderr: &str) -> bool` helper from
  the inline stderr matching in `run_tmux`; `run_tmux` calls it. Add unit tests for the helper.
- `src/sessions/commands.rs` — extract a pure mapping of `(verb, session_name, &TmuxError)` →
  a degradation outcome (graceful message vs. fatal message), used by all three handlers. Add
  unit tests for the mapping. The three handlers (`attach`/`new`/`kill`) are rewritten to call a
  shared `apply_degradation` so their error behavior is identical and centralized.
- `CLAUDE.md` — standing rule: every change ships with tests; no emoji; OKF frontmatter.
- `planning/decisions/D4-session-management-surface.md`, `D5-sessions-synchronous.md` — confirm
  the refactor keeps the surface DB-free and synchronous (no new coupling).

### New Files
None — all changes are within existing `sessions/` files.

## Step by Step Tasks
IMPORTANT: Execute every step in order, top to bottom.

### 1. Extract `classify_no_server` in tmux.rs (#2)
- Add a pure function:
  ```rust
  /// True when tmux stderr indicates no server is running / reachable.
  pub fn classify_no_server(stderr: &str) -> bool {
      stderr.contains("no server running")
          || stderr.contains("error connecting to")
          || stderr.contains("No such file or directory")
  }
  ```
- Replace the inline `if stderr.contains(...) || ...` block in `run_tmux` with a call to
  `classify_no_server(&stderr)`. No behavior change.
- Add tests in the `tmux.rs` `tests` module:
  - each of the three known phrases returns `true` (one test per phrase, or a table test);
  - an unrelated stderr (e.g. `"duplicate session: work"`) returns `false`;
  - empty string returns `false`.

### 2. Extract degradation mapping in commands.rs (#1)
- Add a pure outcome type and mapping function:
  ```rust
  /// Outcome of mapping a TmuxError to user-facing degradation.
  #[derive(Debug, PartialEq)]
  pub enum Degraded {
      /// Print this message; treat as success (graceful).
      Graceful(String),
      /// Print this message; propagate the original error.
      Fatal(String),
  }

  /// Map a TmuxError to its user-facing degradation for a given verb.
  pub fn degrade_tmux_error(verb: &str, session_name: &str, err: &TmuxError) -> Degraded {
      match err {
          TmuxError::NotInstalled => Degraded::Graceful(format!(
              "tmux not installed — install tmux to use `bastion {verb}`"
          )),
          TmuxError::NoServer => Degraded::Graceful("no tmux server running".to_string()),
          TmuxError::ExitError { stderr, .. } => match verb {
              "new" => Degraded::Fatal(format!(
                  "error creating session '{session_name}': {stderr}"
              )),
              _ => Degraded::Fatal(format!("error: session '{session_name}' not found")),
          },
      }
  }
  ```
- Add a small private applier that keeps the existing `print!` + return contract:
  ```rust
  fn apply_degradation(verb: &str, session_name: &str, e: anyhow::Error) -> anyhow::Result<()> {
      if let Some(te) = e.downcast_ref::<TmuxError>() {
          match degrade_tmux_error(verb, session_name, te) {
              Degraded::Graceful(msg) => { println!("{msg}"); return Ok(()); }
              Degraded::Fatal(msg) => { println!("{msg}"); return Err(e); }
          }
      }
      Err(e)
  }
  ```
- Rewrite `attach` / `new` / `kill` error arms to delegate to `apply_degradation(verb, name, e)`,
  preserving the existing success-path messages (`format_created` / `format_killed`, and `attach`
  returning `Ok(())` silently on success). Verify the produced messages match the pre-refactor
  strings exactly (no user-visible change).

### 3. Add degradation mapping tests (#1)
- In the `commands.rs` `tests` module, add tests asserting `degrade_tmux_error` results:
  - `NotInstalled` → `Graceful` containing the verb name (test at least `attach` and `kill` so the
    `bastion {verb}` interpolation is covered);
  - `NoServer` → `Graceful("no tmux server running")`;
  - `ExitError` with `verb = "new"` → `Fatal` containing `error creating session` and the stderr;
  - `ExitError` with `verb = "attach"` (and/or `kill`) → `Fatal` containing `not found`.
- Construct `TmuxError::ExitError { code, stderr }` directly in tests (no tmux spawn needed).

### 4. Validate
- Run the Validation Commands listed below and confirm all pass.
- Confirm the new test count increased and the sessions suite is green:
  `cargo test sessions`.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
- This is a refactor-for-testability: the CLI's printed output and exit behavior must be
  byte-for-byte identical to the current implementation. The new tests pin that behavior.
- Out of scope (intentionally untested, per D5): the actual process-spawning paths
  (`run_tmux`, `new_session`, `kill_session`, `attach_session` I/O). These are verified by the
  deferred manual tmux smoke test, not in CI.
- `Degraded` and `degrade_tmux_error` / `classify_no_server` may need `pub` (or `pub(crate)`) to
  be reachable from the `#[cfg(test)]` module in the same file — `pub(crate)` is sufficient and
  keeps the surface tight; `#![allow(dead_code)]` in `main.rs` already covers any unused-warning
  during incremental build-out.
