---
type: Decision
title: "D5: Session verbs are synchronous"
description: The sessions/ surface is plain synchronous Rust — tmux shell-outs are blocking std::process::Command calls, so session verbs are not async and do not touch the tokio runtime.
---

# D5 — Session verbs are synchronous

**Decided:** 2026-06-21
**Status:** Accepted

## Decision

The `sessions/` surface is **synchronous**. `sessions::run()` (and the Block B–E verbs that
follow) are plain `fn`, not `async fn`. The `main.rs` dispatch arm calls `sessions::run()`
directly with no `.await`, sitting alongside the `async` workflow-observability arms — valid
because every match arm evaluates to the same type (`anyhow::Result<()>`).

## Why

The tmux layer is driven entirely by **blocking `std::process::Command`** calls (D4: no new
deps, no async process crate). There is no I/O concurrency to exploit — each verb shells out,
waits, and returns. Making these functions `async` would add `tokio` ceremony (and an implicit
runtime dependency) to a surface that is explicitly meant to run with zero orchestrator/DB
infrastructure. Keeping them sync also keeps the eventual SSH-from-phone path dependency-light.

## Consequence

- Phase 5 Blocks B–E (`attach`/`new`/`kill`/`send`/`capture`, session TUI) stay synchronous.
- If a future verb genuinely needs concurrency (e.g. fanning out `capture-pane` across many
  sessions), revisit with `std::thread` before reaching for async — async is not the default
  here.

## Rejected Alternatives

- **Make session verbs `async` for uniformity with the monitor arms:** rejected — uniformity for
  its own sake; adds runtime coupling to a deliberately infrastructure-free surface for no
  concurrency benefit.

## Refs

Recorded from `planning/phase5-blockA` implement report (Decisions and Trade-offs). Builds on
[D4](./D4-session-management-surface.md) (the two-surface split; tmux via `std::process::Command`).
