---
type: Decision
title: "D9: Claude readiness is detected via classify_state == Running, not an exact process-name match"
description: The `bastion ask` cold-start readiness check treats the session as Claude-ready when classify_state reports Running, rather than matching the foreground process name against the literal "claude", because Claude Code renames its process to its version string.
---

# D9 — Claude readiness via `classify_state == Running`, not an exact `"claude"` match

**Decided:** 2026-06-21
**Status:** Accepted

## Decision

In the `bastion ask` cold-start path (`src/sessions/ask.rs`), readiness — "Claude Code is up in
the session, send the trigger" — is determined by `classify_state(foreground_cmd) == Running`
(i.e. the foreground command is not one of the known idle shells), **not** by matching the tmux
`#{pane_current_command}` against the literal string `"claude"`.

## Why

Claude Code (observed on **v2.1.185**) renames its own process after launch: its `ucomm` /
`#{pane_current_command}` becomes the **version string**, not `claude` (the rename is done via
`pthread_setname_np`). An exact `== "claude"` readiness check therefore never matches, so the poll
loop would run to its readiness budget and the turn would stall/fail even though Claude was running
fine. Keying off `classify_state == Running` is robust to whatever Claude names its process — it
asks the inverse question ("is the foreground something other than an idle shell?"), which is the
property the readiness gate actually cares about. It also reuses the Block F classifier already used
for the activity indicator (`IDLE_SHELLS`), so there is one source of truth for "is this session
doing something."

## Consequence

- `ask`'s readiness poll skips the launch when `classify_state` reports `Running` and otherwise
  waits for that transition; it does not depend on the foreground command being literally `claude`.
- This is resilient to future Claude Code process-name changes, but it is coarser: any non-idle
  foreground process reads as "ready." Acceptable for `ask`, whose sessions are dedicated to the
  Claude turn (the caller owns the session name).
- **Known follow-up (deferred, out of scope for Block G):** there is still a cold-start race — a
  short fixed delay after readiness detection would close the gap between `classify_state` returning
  `Running` and Claude Code's TUI finishing initialization. Noted, not yet implemented.

## Rejected Alternatives

- **Exact `#{pane_current_command} == "claude"` match:** rejected — broken by Claude Code's
  process-name rename to its version string; produces false "not ready" and stalls the turn.
- **Match against the version string:** rejected — brittle (changes every Claude Code release) and
  would need bastion to track Claude versions.

## Refs

Recorded from `planning/phase5-blockG` fix pass (commit `bd7190d`). Logic lives in
`src/sessions/ask.rs` (readiness poll) and reuses `classify_state` / `IDLE_SHELLS` from
`src/sessions/model.rs` (Block F). Builds on [D5](./D5-sessions-synchronous.md) (synchronous
sessions surface).
