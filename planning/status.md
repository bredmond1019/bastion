---
type: ProjectStatus
title: bastion Status
description: Current state and progress tracker for bastion.
---

# STATUS — Current State & Progress

**Last updated:** 2026-06-21 — phase5-blockB complete; attach/new/kill lifecycle verbs shipped; 79 tests passing, all gating checks green
**Current focus:** planning/phase5-blockC — `bastion send` (keystroke injection into tmux panes).

---

## How to Read / Update This File

- Status values: `Not started` · `In progress` · `Done` · `Blocked` · `Skipped`
- Keep `Current focus` and `Last updated` accurate; update as work happens.
- This file is **state only**. For what the work means, see `master-plan.md`.

---

## Progress Table

### Phase 0 — Foundation
| Block | What | Status | Notes |
|---|---|---|---|
| Block A | Foundation setup | Done | Both tasks merged (2026-06-20). Toolchain verified, config.rs reads DATABASE_URL + BASTION_API_URL with typed errors, .env.example added. Health probes (API + DB) implemented. `bastion status` command works offline and prints service reachability. All 17 tests pass; all gated checks green. |

### Phase 1 — Monitor
| Block | What | Status | Notes |
|---|---|---|---|
| Block A | DB queries + graph layout | Done | All tasks complete: test fixtures created (in-progress + completed run samples); node_runs JSON → NodeState parsing implemented with RunStatus deserialization; DB queries (list_active_runs, get_run_state) filled with sqlx; topological layout algorithm with grid position assignment verified against linear chains and diamond DAGs; all validation gates pass (cargo fmt, clippy, test, build --release). Cross-contract sync: v1.0.0 aligned (D3). |
| Block B | TUI render loop and event-driven updates | Not started | Next: implement ratatui TUI render loop and event-driven updates. |

### Phase 5 — Session Management (independent, ungated track — D4)
| Block | What | Status | Notes |
|---|---|---|---|
| Block A | `bastion sessions` + tmux wrapper + lazy DB pool | Done | sessions/ module shipped: tmux.rs (pure arg construction + typed errors), model.rs (Session/Pane + fixture parsing), commands.rs (list verb, graceful degradation, render), CLI wiring. DB-free guarantee enforced by architecture and locked in by test. 20 new tests; 73 total pass. All gating checks green. PASS in 1 review attempt. |
| Block B | `attach` / `new` / `kill` (lifecycle) | Done | attach/new/kill verbs shipped: pure arg-construction functions, interactive attach_session (.status()), new_session, kill_session; graceful degradation for NotInstalled/NoServer/ExitError; format_created/format_killed helpers unit-tested. PASS in 1 review attempt. Follow-up chore (2026-06-21) closed the error-path test gaps: extracted pure `classify_no_server` (tmux.rs) + `degrade_tmux_error`/`Degraded` (commands.rs); 9 new tests, 88 total, all gating checks green. **Deferred manual smoke test now COMPLETE** — verified live against tmux 3.6b: new (incl. `--dir` cwd), sessions list, kill (valid + unknown-session error), attach unknown-session error, and the interactive `attach`→`Ctrl-b d` detach round-trip (returns cleanly to shell). |
| Block C | `bastion send` (keystrokes) | Not started | |
| Block D | `bastion capture` (pane output) | Not started | |
| Block E | session view in the TUI | Not started | Built on Block A–D primitives. |

<!-- Add one sub-table per phase as the plan is fleshed out. -->

---

## Decisions & Deviations Log

*Record deviations from the plan and notable in-flight choices here. Promote durable ones to
`decisions/` via `/log-work`.*

- **2026-06-21 — bastion absorbs tmux session management; second surface added (D4).** What was
  sketched as a standalone tool (working name `brain`) is folded into bastion as modules instead:
  a `sessions/` module and a `bastion sessions` command family, with a session view in the TUI.
  Rationale: the standalone tool's charter ("operator interface that grows into the client
  appliance shell") was already bastion's, and bastion is shaped for it (single crate; already
  has `clap`/`ratatui`/`crossterm`; tmux needs no new deps). bastion now has two surfaces —
  workflow observability (Postgres, gated by D2) and process/session control (tmux, **ungated**).
  Added **Phase 5** (Blocks A–E) to `master-plan.md` as an independent track. One real constraint:
  the Postgres pool must become **lazy** so session commands run with zero DB. Cross-repo: brain
  **D21**; recorded as bastion **D4**.
- **2026-06-21 — Decisions D5–D6 promoted to registry (phase5-blockA deferred choices).** D5 (Session verbs are synchronous: tmux shell-outs are blocking `std::process::Command` calls, so no async ceremony or tokio coupling to the sessions/ surface) and D6 (Malformed `tmux list-sessions` output lines are skipped with stderr warning rather than aborting; partial system state is more useful than none). Both build on D4 and finalize the sessions/ surface contract.
- **2026-06-20 — Pinned the orchestrator data contract v1.0.0 (D3).** The orchestrator now publishes a single, versioned contract (`python-orchestration-system/docs/data-contract.md`) for the execution state bastion reads; bastion holds a consumer view (`docs/data-contract.md`) pinned to v1.0.0. Confirmed the **Hybrid** read path (direct Postgres for the live poll; reserved HTTP read API later) and the **two-source merge model** (DAG edges from `GET /workflows/{type}/graph`, live state from `events.task_context.node_runs`, joined by node **class name**). Realigned `master-plan.md` and the Phase-1 stub type defs to reality (no relational `workflow_runs`/`node_states` tables exist): `NodeState` gained `model`/`input`; `RunStatus` deserializes lowercase status strings; `ApiClient::workflow_graph()` added; `build_layout` now takes API edges. Orchestrator-side additions that complete the contract: per-node `input` + serializable output (orchestrator D30). `cargo fmt`/`clippy`/`test` (17) all green. Cross-repo: brain D20 / orchestrator D30. `/log-work` gained a contract sync-checklist step.
- **2026-06-18 — Pre-Block-A reconnaissance against the live orchestrator.** Read the
  python-orchestration-system to ground Block A. Findings: (1) orchestrator state is one `events`
  table with JSON `data` + `task_context` columns — no relational runs/nodes tables; the DAG is
  reconstructed by parsing `task_context`. (2) `/health` returns only `{status, version}` on port
  **8080** (not 8000 as the scaffold `.env.example` said); DB is `postgres`/`postgres`@5432, db
  name `postgres` (not `orchestrator_db`). Both config defaults to be corrected in Block A. (3)
  Worker count / queue depth live in Redis, out of bastion's configured scope → **Block A status
  scoped to DB + API only**; Redis-backed metrics deferred (see D2). (4) **Critical upstream
  dependency:** `task_context` is persisted only once, at the end of a run — so a live monitor has
  no intermediate state to read. The orchestrator owns the fix (incremental node-level
  persistence): orchestrator DECISIONS **D28** + plan `incremental-execution-observability.md`.
  bastion Phase 1 (monitor) is gated on that plan's Phase 1 landing. Recorded as bastion **D2**.
  Test path for Block A: stand up a local Postgres + apply the orchestrator migration for true
  end-to-end verification, plus unit tests for the unreachable/degraded path.

---

## Quick Self-Check

- Is `Current focus` accurate?
- Any `In progress` rows that are actually `Done`?
- Anything `Blocked` that needs surfacing?

---

*State only. For what things mean, see master-plan.md. For orientation, see context.md.*
