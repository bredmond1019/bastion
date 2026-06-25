---
type: Decision
title: "D6: Skip malformed tmux output lines"
description: When parsing tmux list-sessions output, a malformed line is skipped with a stderr warning rather than aborting the whole listing — partial system state is more useful than no state.
---

# D6 — Skip malformed tmux output lines

**Decided:** 2026-06-21
**Status:** Accepted

## Decision

When parsing `tmux list-sessions` output, a line that does not match the expected
`-F`-formatted shape is **skipped with a warning to stderr** — it does not abort the listing or
propagate an error. `parse_sessions` returns every line it could parse; unparseable lines are
dropped, not fatal. (The spec left this open: "graceful skip or typed error — pick one and test
it.")

## Why

`list-sessions` output is **live operator-system state**, not a controlled data contract. A
single odd line (an unexpected tmux version's format quirk, a session name with the separator
character, etc.) should not blind the operator to every other running session — partial state is
more useful than none, especially when this is the surface reached from a phone over SSH to see
what is running. The stderr warning preserves the signal that something was unparseable without
breaking the happy path.

## Consequence

- The format string and field separator are named `const`s shared between `tmux.rs` and
  `model.rs`, so the parser and the producer cannot silently drift.
- This policy is parsing-layer only. It does **not** apply to tmux *invocation* failures
  (binary missing, no server) — those remain typed `TmuxError` variants surfaced as clear
  messages.

## Rejected Alternatives

- **Propagate a typed parse error and abort the listing:** rejected — one malformed line would
  hide all healthy sessions, the opposite of what an at-a-glance ops listing is for.

## Refs

Recorded from `planning/phase5-blockA` implement report (Decisions and Trade-offs). Parsing lives
in `src/sessions/model.rs`; relates to [D4](./D4-session-management-surface.md).
