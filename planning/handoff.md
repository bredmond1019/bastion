---
type: Handoff
created: 2026-06-21
---

# Handoff — Phase 5 Block F defined; ready to spec

> **For the next agent:** Read this immediately after `/prime`. Delete this file once consumed.

## What we're doing and why
Phase 5 (session/tmux control — the ungated track, D4) is the active work. Blocks A–E are
**complete and shipped**. This session ran a live end-to-end test of driving Claude Code through
bastion, which exposed two real gaps; we turned those into a newly-defined **Block F — session
activity indicator + Claude trust observer** (now written into `master-plan.md`). Block F is the
**clear next thing to build**: it is unblocked (unlike the monitor track's `phase1-blockB`, which is
gated on orchestrator D28), and it fixes flaws the live test surfaced. The next step is to spec it.

## Completed this session
- Ran `/sdlc-run phase5-blockE --from implement` → PASS in 1 attempt; shipped the ratatui session
  TUI dashboard (`src/sessions/app.rs` + `ui.rs`; bare `bastion` / `bastion tui` launch it). Recorded
  decisions **D7** (`k` = kill, not vim nav-up) and **D8** (`Attach` handled in the run loop, not
  `execute_action`). Authored `docs/claude-code-workflow.md` (genericized to `~/projects/*`,
  standardized launch on `claude --permission-mode bypassPermissions`). Commits through `9cb83a5`.
- **Live test of the Claude Code workflow** (created session → launched `claude --permission-mode
  bypassPermissions` → answered trust prompt → sent a prompt → captured the reply → killed session).
  It works. Two findings drove Block F:
  1. A **running Claude Code session reports `idle`** — `SessionState` is keyed on whether a *client
     is attached* (`session_attached`), not on what the pane is running. Misleading for the phone
     "is it still working?" check.
  2. **Hands-off `send`-launch stalls on Claude's one-time workspace-trust prompt** the first time
     per new directory (separate gate from `--permission-mode`).
- **Investigated both before deciding** (evidence in the session, reproducible):
  - tmux `#{pane_current_command}` works in `list-sessions -F` (probe returned the active pane's
    foreground command) → the basis for a real activity indicator.
  - Claude already persists trust per project in `~/.claude.json` →
    `projects["<abs-dir>"].hasTrustDialogAccepted` (bool). So a bastion-owned state store is
    **overkill and would drift**; read Claude's file as a read-only observer instead (same posture
    bastion takes toward the orchestrator's Postgres).
- **Added Block F to `master-plan.md`** — Phase 5 section + quick-reference table. (Uncommitted at
  the moment this handoff was written; the handoff `/commit` step will include it.)

## Remaining work
- **Spec and build Block F.** Recommended sequence:
  1. `/generate-tasks phase5-blockF` (the block definition now exists in `master-plan.md`).
  2. `/breakdown planning/phase5-blockF/tasks.md` if a task warrants it (the activity-indicator task
     touches the core `Session` model — likely a breakdown candidate).
  3. `/sdlc-run phase5-blockF` (or `/sdlc-block` if ≥2 tasks run in parallel — generate-tasks will
     recommend).
- Keep it to **one combined block, two tasks** (activity indicator; trust observer) — they share the
  render layer and are halves of one idea ("make the session list honest about real state").

## Open questions / choices
- **Claude exec name is not assumed.** Classify activity by *exclusion*: foreground command ∈
  `{zsh, bash, sh, fish}` → idle shell; anything else → running `<cmd>`. This sidesteps whether
  `claude` shows as `claude` or `node`. Confirm the shell set fits the host if needed.
- **`~/.claude.json` is Claude's internal schema** and can change between versions — the trust read
  must degrade gracefully (missing file/field → "trust: unknown"), never hard-depend on it, and
  **never write** to it.
- Otherwise clear to proceed — the design is settled per the discussion above.

## Context the next agent needs
- Preserve the sessions-surface invariants: **DB-free (D4)** — no `Config::load()`/no pool; and
  **synchronous (D5)** — no tokio. Reuse `degrade_tmux_error`/`Degraded` (`src/sessions/commands.rs`)
  for tmux error handling. Coverage bar (CLAUDE.md Rule 6): pure parsing/classification exhaustively
  unit-tested against fixtures; thin I/O shell smoke-tested and recorded in the spec `## Notes`.
- Likely file map for Block F: `sessions/tmux.rs` (format const gains `#{pane_current_command}`),
  `sessions/model.rs` (`Session.foreground_cmd`, rework `SessionState`, update `parse_session_line`),
  `sessions/claude_state.rs` *(new — trust observer, fixture-tested)*, `sessions/commands.rs` +
  `sessions/app.rs`/`ui.rs` (render the activity column + trust hint). Watch disjoint file ownership
  if running tasks in parallel — the two tasks overlap on `commands.rs`/`ui.rs` render code.
- Validation gate (`planning/harness.json`): `cargo fmt --check`, `cargo clippy -- -D warnings`,
  `cargo test`, `cargo build --release`. Test baseline: **145**.
- `phase1-blockB` (monitor render loop) remains BLOCKED on orchestrator D28 — do not start it without
  confirming that landed. Block F is the unblocked path.
- Brain-repo doc edits from last session may still be **uncommitted in `../`** (the parent
  `agentic-portfolio` repo) — `docs/projects/bastion.md` + `README.md`. Commit them there when convenient.

## First command after `/prime`
`/generate-tasks phase5-blockF`
