---
type: Decision
title: D2 — bastion's observability consumer contract (depends on orchestrator D28)
description: bastion is a read-only consumer of orchestrator execution state; the live monitor depends on the orchestrator persisting node-level state incrementally, which the orchestrator owns on its own merits.
---

# D2 — bastion's observability consumer contract

**Decided:** bastion reads orchestrator execution state and never reaches back into the Python
side (Governing Principle 4 — observer, never writer). The data it consumes comes from the
`events` table: `workflow_type`, the `data`/`task_context` JSON, and — once orchestrator **D28**
lands — a per-node status/timing envelope (`task_context.node_runs`), token usage, and a workflow
graph-introspection endpoint. bastion parses `task_context` to reconstruct the DAG; there are no
relational run/node tables to join.

**Why:** Reconnaissance before Phase 0 Block A established two things. First, the orchestrator's
`/health` is minimal (`{status, version}` on port 8080) and worker/queue metrics live in Redis,
which bastion is not configured to reach — so **Block A `bastion status` is scoped to DB + API
reachability only**; Redis-backed metrics are deferred. Second, and load-bearing for Phase 1:
the orchestrator currently persists `task_context` only once, at the end of a run, so a live
monitor would have no intermediate state to poll. The fix is the orchestrator's to make
(incremental node-level persistence) and it is justified on the orchestrator's own merits — see
orchestrator DECISIONS **D28** and plan
`python-orchestration-system/planning/plans/incremental-execution-observability.md`.

**Consequence for the sequence:** bastion Phase 1 (`bastion monitor`) is **gated** on that plan's
Phase 1 (node-boundary persistence + status envelope) landing. Until then, Phase 0 (`status`) and
the static `inspect`/`costs` reads that work off terminal `task_context` are unblocked. bastion's
roadmap is otherwise unchanged.

**Rejected:**
- *Having bastion write or trigger orchestrator-side persistence* — violates the observer
  principle; the orchestrator must expose state on its own merits with no consumer coupling.
- *Inferring live progress from the single terminal write* — impossible; there is no mid-run
  state to read. Hence the upstream dependency.

**Refs:** orchestrator D28 + `incremental-execution-observability.md`; bastion
`planning/status.md` deviation log (2026-06-18); supersedes nothing.
