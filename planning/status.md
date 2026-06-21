---
type: ProjectStatus
title: bastion Status
description: Current state and progress tracker for bastion.
---

# STATUS — Current State & Progress

**Last updated:** 2026-06-21 — phase1-blockA complete (all 5 tasks merged); phase1-blockB next
**Current focus:** phase1-blockB — TUI render loop and event-driven updates

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

<!-- Add one sub-table per phase as the plan is fleshed out. -->

---

## Decisions & Deviations Log

*Record deviations from the plan and notable in-flight choices here. Promote durable ones to
`decisions/` via `/log-work`.*

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
