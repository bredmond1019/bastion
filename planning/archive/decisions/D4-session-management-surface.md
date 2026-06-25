---
type: Decision
title: D4 — bastion absorbs tmux session management (process-control surface)
description: tmux session management ships as modules inside the bastion binary rather than a separate tool; bastion becomes the single operator shell with two surfaces — workflow observability (Postgres) and process/session control (tmux).
---

# D4 — bastion absorbs tmux session management

**Decided:** bastion gains a second surface — **process / session control** — for managing the
tmux sessions on the Mac Mini that hold long-running processes (primarily Claude Code sessions).
It ships as **modules inside the existing binary** (`sessions/` beside `monitor/`), exposing a
`bastion sessions` command family (`sessions`, `attach`, `new`, `send`, `capture`, `kill`) and,
later, a session view in the TUI. This was previously sketched as a standalone tool (working name
`brain`); that idea is dropped — the name collided with the company-brain repo, and the tool's
charter ("operator interface that grows into the client-facing appliance shell") was already
bastion's. There is one operator shell, and it is bastion. Recorded cross-repo as brain **D21**.

**Why:** bastion is already defined as the ops shell that unifies all personal tooling. A second
tool making the same claim would fork the portfolio narrative. bastion is also already shaped for
this: it is a single-crate binary whose subcommands are plain modules, and it already depends on
`clap`, `ratatui`, and `crossterm` — the tmux layer adds **no new dependencies** (only
`std::process::Command`). Folding in is faster to ship (no new repo, no duplicated TUI/CLI
harness) and gives one binary to install and invoke from any device (SSH over Tailscale).

**The two surfaces (scope line):**
- *Workflow observability* (`monitor`, `inspect`, `costs`, `run`) — reads the orchestrator's
  **PostgreSQL** state. Gated by D2 (orchestrator D28 incremental persistence).
- *Process / session control* (`status`, `sessions`, …) — shells out to **tmux** and the OS.
  Depends on neither Postgres nor the orchestrator, and is therefore **ungated**.

**Constraints:**
- **The Postgres pool must open lazily / on demand.** Session commands and the session TUI must
  work with zero DB connectivity (Postgres down, or invoked from a context that never touches it).
  The DB gate on `monitor` must not leak onto the tmux commands. This is the one real engineering
  commitment — if `main` currently opens the pool eagerly, Block A makes it lazy.
- **bastion manages, it does not run Claude Code.** It creates, inspects, and controls the tmux
  sessions that contain Claude Code — never drives the sessions themselves.
- **Plain modules first, not a workspace.** Implement inside the existing crate. Split into a
  Cargo workspace later only if the tmux layer needs to be separately publishable.
- **Earn the next command.** Build order is incremental (Phase 5 Blocks A→E); each verb ships only
  when reached for. Guards against bastion becoming a kitchen sink.

**Consequence for the sequence:** A new **Phase 5 — Session Management** track is added to
`master-plan.md`. It is an **independent, ungated track** — not sequenced behind the monitor work,
and can be worked at any time (including before the gated monitor phases complete).

**Rejected:**
- *A separate `brain`/standalone repo* — forks the operator-shell narrative; duplicates the
  TUI/CLI scaffolding; two binaries to install; a probable future merge anyway.
- *Splitting into a Cargo workspace up front* — premature; the modules share the crate cleanly
  and the tmux layer has no standalone-publish requirement today.

**Refs:** brain D21 (`docs/decisions/D21-bastion-session-management.md`); brain
`docs/projects/bastion.md`; aligns with brain D5 (Rust track = personal ops CLI / appliance
shell) and D14 (operator shell is portfolio depth, not a separate product). Supersedes nothing.
