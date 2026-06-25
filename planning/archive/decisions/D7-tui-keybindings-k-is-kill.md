---
type: Decision
title: "D7: Session TUI navigation is arrow-keys + j; k is kill-only"
description: In the session TUI Normal mode, k is bound to the kill verb, not vim-style nav-up — navigation is Up/Down arrows plus j for down, so an up-press can never trigger an accidental kill.
---

# D7 — Session TUI navigation is arrow-keys + `j`; `k` is kill-only

**Decided:** 2026-06-21
**Status:** Accepted

## Decision

In the session TUI (`src/sessions/app.rs`, `SessionApp::on_key`), the `k` key in **Normal mode**
is bound to the **kill verb** (kill the selected session), not vim-style navigation-up.
List navigation is the **`Up`/`Down` arrow keys plus `j` for down**; there is deliberately **no
`k`-for-up** binding. The single-key verb legend (`[a]ttach [n]ew [s]end [k]ill [q]uit`) takes
precedence over vim muscle memory.

## Why

The dashboard's verbs are single-letter mnemonics (`a`/`n`/`s`/`k`/`q`). `k` for "kill" is the
mnemonic an operator expects, and a vim-style `k`-for-up binding would directly collide with it on
the same key in the same mode. Of the two readings, the destructive one is the dangerous one:
binding `k` to nav-up would mean a reflexive up-press on the wrong session is an accidental
**kill**. Resolving the collision in favor of the verb — and providing `Up` (+ `j` downward) for
navigation — removes that footgun entirely. This surface is reached from a phone over SSH, where a
misfire is costly and hard to undo.

## Consequence

- `on_key` maps `Char('k')` → `Action::Kill(selected)` and `Up` → `select_prev()`; `j`/`Down` →
  `select_next()`. The asymmetry (`j` works, `k` does not) is intentional and documented inline.
- Any future verb that wants a vim-pair binding must check the single-key legend first — the verb
  mnemonics own their letters.

## Rejected Alternatives

- **Vim `j`/`k` for navigation, move kill to another key:** rejected — `k` is the natural, expected
  mnemonic for "kill"; remapping it to a less obvious key to satisfy vim symmetry trades clarity of
  a destructive verb for paddling convenience.

## Refs

Recorded from `planning/phase5-blockE` implement report. Binding lives in `src/sessions/app.rs`
(`SessionApp::on_key`). Builds on [D4](./D4-session-management-surface.md) and
[D5](./D5-sessions-synchronous.md) (the synchronous, infrastructure-free sessions surface).
