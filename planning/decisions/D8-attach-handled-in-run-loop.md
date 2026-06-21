---
type: Decision
title: "D8: Attach is handled in the TUI run loop, not execute_action"
description: The session TUI's Attach action is executed directly in run_inner rather than the shared execute_action helper, because suspending/restoring the terminal requires the ratatui Terminal handle that execute_action does not hold.
---

# D8 — Attach is handled in the TUI run loop, not `execute_action`

**Decided:** 2026-06-21
**Status:** Accepted

## Decision

In the session TUI (`src/sessions/ui.rs`), the `Action::Attach` branch is executed **directly in
the event loop (`run_inner`)**, not in the shared `execute_action` helper that handles
`New`/`Send`/`Kill`. `execute_action` deliberately does **not** match `Attach`; the loop intercepts
that action before delegating the rest.

## Why

Attaching to tmux requires **suspending the TUI** — leaving raw mode and the alternate screen,
handing the terminal to `tmux attach`, then re-entering raw mode + alternate screen and clearing on
return. That teardown/restore needs the `ratatui::Terminal` handle (and `backend_mut()`), which
lives in the run loop and is not — and should not be — passed into `execute_action`. `execute_action`
is the pure-ish "fire a tmux verb and fold the result into `app.status`" helper for the
non-suspending verbs; threading the whole `Terminal` through it to accommodate the one action that
manipulates the screen would muddy that boundary. Keeping `Attach` in the loop keeps each piece
doing one job.

## Consequence

- `execute_action` covers `New`/`Send`/`Kill`/`None`; `Attach` is handled inline in `run_inner` with
  the suspend → `tmux::attach_session` → restore → refresh sequence.
- The split is a known asymmetry: a reader scanning `execute_action` for "where does attach happen"
  must look to the loop. Documented here and inline so it is not mistaken for an omission.

## Rejected Alternatives

- **Pass the `Terminal` into `execute_action` so it handles all actions uniformly:** rejected — it
  couples the verb-dispatch helper to the rendering backend for a single case, eroding the clean
  state-vs-I/O boundary the sessions surface maintains elsewhere.

## Refs

Recorded from `planning/phase5-blockE` implement report. Logic lives in `src/sessions/ui.rs`
(`run_inner` / `execute_action`). Builds on [D5](./D5-sessions-synchronous.md) (synchronous loop)
and the attach primitive established in Phase 5 Block B.
