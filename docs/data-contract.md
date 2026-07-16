---
type: Reference
title: bastion ⇄ Orchestrator Data Contract (Consumer)
description: bastion's pinned view of the orchestrator's versioned data contract — how each contract field maps to bastion's Rust types. The canonical contract lives in the Python repo.
doc_id: data-contract
layer: [console, engine]
project: bastion
status: active
keywords: [data contract, orchestrator, PostgreSQL, node_runs, field mappings, v1.1.0, cancellation, abort, budget gate]
related: [monitor, costs, inspect, run]
---

# Data Contract (Consumer View)

**Pinned Contract Version: 1.1.0**

The **canonical, authoritative** contract is owned by the orchestrator:
`python-orchestration-system/docs/data-contract.md`. This file is bastion's *consumer* view — it
pins the version bastion is built against and maps each contract field to bastion's Rust types.

> bastion is an **observer, never a writer** of the `events` row itself (D2) — it never opens a
> write connection to PostgreSQL. As of the canonical contract's v1.1.0, bastion may *trigger* a
> write the orchestrator/engine performs on its own behalf (`POST /events/{run_id}/abort`, per
> brain decision D25: "bastion triggers, the Engine executes") without becoming a writer itself —
> see the canonical doc's §3 for the reconciled prose. When the canonical contract bumps, re-pin
> the version here and update the mappings. The `/log-work` checklist prompts this.

---

## Read paths (v1.x)

### Monitor / Inspect (Hybrid)

- Live monitor polls **PostgreSQL** `events.task_context` directly (read-only).
- The **DAG edges** come from `GET /workflows/{type}/graph` (HTTP) — the only source of edges and
  of not-yet-run nodes.
- Join the two on **node class name**.
- Reserved for later: an orchestrator HTTP read API (`GET /events/{id}`) — do not depend on it.

### Costs (DB-only)

- `db::costs::fetch_all_runs` issues `SELECT id, workflow_type, task_context FROM events` over
  **all** rows (active and completed), assembling each via `db::workflows::parse_event_row`
  (the same shared parse path as monitor/inspect — no duplicated JSON parsing).
- No graph endpoint is used. Token counts are **exact**, computed by `costs::tokens::count` (real
  `tiktoken` encoding) over each node's `input`/`output` text; `node_runs[*].usage.input_tokens` /
  `.output_tokens` are used only as a fallback when a node has no countable text or no `model`.
- Window filtering (`7d`, `30d`, `all`) is applied in pure Rust after the full-table fetch.

---

## Field mappings

### `events` row → `db::workflows::WorkflowRun`

| Contract (`events`) | bastion |
|---|---|
| `id` | `WorkflowRun.id` |
| `workflow_type` | `WorkflowRun.workflow_name` |
| `data` | run input (detail pane "run input") |
| `task_context.node_runs` | `WorkflowRun.nodes: Vec<NodeState>` |
| derived from `node_runs` aggregate | `WorkflowRun.status: RunStatus` |
| `task_context.node_runs[*].started_at` (min) | `WorkflowRun.started_at` |
| derived (now − started_at) | `WorkflowRun.elapsed_secs` |

**Active runs:** select rows whose `node_runs` values are not all terminal (`success`/`failed`).
There is no indexed status column in v1.0.0 — scan + parse.

### `node_runs[name]` (+ `nodes[name]`) → `db::workflows::NodeState`

| Contract | bastion |
|---|---|
| class name (the key) | `NodeState.id` / `NodeState.name` |
| `node_runs[name].status` (`pending\|running\|success\|failed`) | `NodeState.status: RunStatus` (serde-renamed lowercase) |
| `node_runs[name].error` | `NodeState.error` |
| `node_runs[name].input` | `NodeState.input` |
| `node_runs[name].usage.input_tokens` | `NodeState.tokens_in` |
| `node_runs[name].usage.output_tokens` | `NodeState.tokens_out` |
| `node_runs[name].usage.model` | `NodeState.model` |
| `node_runs[name].started_at` | `NodeState.started_at` |
| derived (completed_at − started_at) | `NodeState.elapsed_secs` |
| `nodes[name]` (output; look for `output` key) | `NodeState.output` |
| edges from `GET /workflows/{type}/graph` | `NodeState.depends_on` |

`RunStatus` must `#[serde(rename_all = "lowercase")]` (or per-variant rename) to deserialize the
contract's lowercase status strings. `usage` is **null** for non-LLM nodes → `tokens_*` / `model`
are `Option`. `input` is null unless the node is an LLM node.

### Run-level `metadata` annotations (new in v1.1.0, not yet consumed)

The canonical contract's v1.1.0 §5 adds two structured `task_context.metadata` keys — a cancelled
run (`metadata.cancellation`) and a budget-halted run (`metadata.budget`), both spelled in
`metadata` rather than as new `NodeRunStatus` values (see the canonical doc for the full shape).
**No bastion Rust type reads these yet** — `WorkflowRun.status: RunStatus` is currently derived
only from `node_runs`' aggregate, so a cancelled or budget-halted run today still reads through
bastion's existing status derivation (most likely as `running`, since not-yet-run nodes stay
`pending` and no node itself failed). Wiring `metadata.cancellation` / `metadata.budget` into
`WorkflowRun.status` (or a new field alongside it) is `BA.7.C`'s job, not this re-pin's — this pin
only registers that the field exists on the wire as of 1.1.0.

### Graph endpoint → edges

`GET /workflows/{type}/graph` → `{ "nodes": [str], "edges": [[from, to]] }`. Maps to
`api::client::workflow_graph()`; node names are class names; `edges` populate `NodeState.depends_on`
(and the `petgraph` DAG in `monitor::graph::build_layout`).

### Trigger → `api::client::trigger_workflow`

`POST /` with `{ "workflow_type": str, "data": object }` → `202 { "task_id": str, "message": str }`.

### Abort → not yet implemented (new in v1.1.0)

`POST /events/{run_id}/abort` (no body) → `401` (bad/missing `X-API-Key`) \| `404` (unknown or
finished run) \| `202 { "run_id": str, "status": "aborting" }`. **No `api::client` function calls
this yet** — `bastion kill` (`BA.7.C`, currently blocked) is the consumer that will add one.
Registered here so the wire shape is pinned before that work starts.

---

## Re-pin checklist (when the canonical contract bumps)

1. Read the canonical changelog; update the **Pinned Contract Version** above.
2. Update any changed field mappings here.
3. Update affected Rust types (`db::workflows`, `db::costs`, `api::client`, `monitor::graph`).
4. Note it in `planning/status.md`.

---

## Changelog (this pin)

| Pinned At | Date | Change |
|---|---|---|
| 1.0.0 | 2026-06-20 | Initial pin against canonical 1.0.0. |
| 1.1.0 | 2026-07-16 | Re-pin from 1.0.0 straight to 1.1.0, resolving known drift against the canonical 1.0.1 patch (no bastion-visible shape change in 1.0.1 — `POST /events/` auth only, and bastion never calls that endpoint). Registers the canonical's v1.1.0 additions: `POST /events/{run_id}/abort` (§ above) and the `metadata.cancellation` / `metadata.budget` run-level annotations (§ above) — both unconsumed by bastion Rust types today; wiring them up is `BA.7.C`'s job. |
