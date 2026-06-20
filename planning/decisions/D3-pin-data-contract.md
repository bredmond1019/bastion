---
type: Decision
title: "D3: Pin the orchestrator's versioned data contract"
description: bastion pins v1.0.0 of the orchestrator-owned data contract, reads via a Hybrid path (direct Postgres now, reserved HTTP read API later), and merges two sources joined by node class name.
---

# D3 — Pin the orchestrator's versioned data contract

**Decided:** bastion pins **v1.0.0** of the orchestrator-owned data contract
(`python-orchestration-system/docs/data-contract.md`) and keeps a consumer view at
`bastion/docs/data-contract.md` mapping each contract field to its Rust types. A live run is
reconstructed by merging **two sources joined on node class name**: DAG shape (nodes + edges, incl.
pending nodes) from `GET /workflows/{type}/graph`, and live per-node state (status, timing, error,
input, token usage, output) from polling Postgres `events.task_context`. Read path is **Hybrid**:
direct Postgres for the live poll now; the orchestrator's reserved `GET /events/{id}` read API is
documented but not depended on.

**Why:** Pre-Block-A recon found bastion's scaffolded stubs assumed relational `workflow_runs` /
`node_states` tables that don't exist — all state is JSON in one `events` table, and edges live only
in the graph endpoint. Pinning a versioned, orchestrator-owned contract makes that explicit and lets
the two repos move together: when the contract bumps, bastion re-pins.

**Consequence for the sequence:** Phase 1 Block A (`db::workflows` + `monitor::graph`) is built
against this contract — parse `node_runs`, fetch the graph endpoint, join by class name. Stub type
defs were aligned now (`NodeState` gains `model`/`input`; `RunStatus` deserializes the lowercase
status strings; `ApiClient::workflow_graph()` added; `build_layout` takes API edges). The
`status` command's worker/queue metrics stay scoped out (Redis, per D2).

**Rejected:**
- *Keep the relational-table assumption* — it never matched reality; corrected here.
- *Derive the DAG from `node_runs` alone* — `node_runs` carries no edges; the graph endpoint is the
  only edge source.
- *Depend on an HTTP read API now* — Hybrid keeps direct Postgres for the cheap high-frequency poll.

**Refs:** orchestrator **D30** + `docs/data-contract.md` (v1.0.0); brain **D20**; extends bastion
**D2** (observability consumer contract), supersedes nothing.
