---
type: Reference
title: bastion ⇄ Orchestrator Data Contract (Consumer)
description: bastion's pinned view of the orchestrator's versioned data contract — how each contract field maps to bastion's Rust types. The canonical contract lives in the Python repo.
doc_id: data-contract
layer: [console, engine]
project: bastion
status: active
keywords: [data contract, orchestrator, PostgreSQL, node_runs, field mappings, v1.3.0, cancellation, abort, budget gate, event read api, ingest]
related: [monitor, costs, inspect, run]
---

# Data Contract (Consumer View)

**Pinned Contract Version: 1.3.0**

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
- As of the canonical contract's v1.2.0, the orchestrator's own Python API now serves a read-only
  `GET /events/{event_id}` alternative to the direct-DB read above (`X-API-Key` gated, `404` for
  unknown/malformed ids, `200 {event_id, workflow_type, status, created_at, updated_at,
  task_context}` with `status` derived server-side — six values, precedence in the canonical
  doc's §7). Bastion does not consume this route yet — monitor/inspect/costs still read PostgreSQL
  directly — but it is no longer reserved-only; wiring bastion to it is a future `BA.*` block, not
  this re-pin.

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

### Run-level `metadata` annotations → `db::workflows::WorkflowRun`

The canonical contract's v1.1.0 §5 adds two structured `task_context.metadata` keys — a cancelled
run (`metadata.cancellation`) and a budget-halted run (`metadata.budget`), both spelled in
`metadata` rather than as new `NodeRunStatus` values (see the canonical doc for the full shape).
`BA.7.C` wires both into `db::workflows::derive_run_status`, which is now the sole source of
`WorkflowRun.status: RunStatus` (never inferred from `node_runs` alone once `metadata` is
present):

| Contract (`task_context.metadata`) | bastion |
|---|---|
| `metadata.budget.halted == true` (+ `.reason`) | `WorkflowRun.status == RunStatus::BudgetHalted`, detail on `WorkflowRun.budget_halt: Option<BudgetHalt>` |
| `metadata.budget.reason.cap` (`"max_total_tokens"` \| `"max_cost_usd"`) | `BudgetHalt::TotalTokens { .. }` \| `BudgetHalt::CostUsd { .. }` |
| `metadata.budget.reason.spent` / `.limit` | `BudgetHalt`'s `spent` / `limit` fields |
| `metadata.cancellation.cancelled == true` | `WorkflowRun.status == RunStatus::Cancelled` |

The canonical contract's v1.2.0 adds a third run-level annotation, `metadata.failure` — written
when a workflow raises inside the orchestrator's `process_incoming_event`, on a fresh session
that survives the enclosing transaction's rollback: `{ "failure": { "failed": true, "error":
"<ExcType>: <msg>", "at": "<iso8601>" } }`. This is not yet wired into
`db::workflows::derive_run_status` — bastion's existing per-node `RunStatus::Failed` derivation
(from `node_runs`) already covers the same terminal state for direct-DB reads; consuming
`metadata.failure` explicitly (e.g. to surface the top-level error message) is future work, not
this re-pin.

`RunStatus` gained the two run-level-only variants `Cancelled` and `BudgetHalted` alongside the
existing per-node-derived `Pending`/`Running`/`Success`/`Failed`. `derive_run_status` (`src/db/
workflows.rs`) checks `metadata.budget` before `metadata.cancellation` — a run can only be
budget-halted by the pre-dispatch gate before the operator gets a chance to cancel it, so on the
rare run carrying both markers the budget halt wins. Both reads are absent-tolerant: a run
written before v1.1.0 (no `metadata` key), or a `metadata.cancellation`/`metadata.budget` that
isn't well-formed, falls back to the pre-v1.1.0 node-based derivation unchanged — reading these
keys never turns a previously-valid run into a parse failure.

### Graph endpoint → edges

`GET /workflows/{type}/graph` → `{ "nodes": [str], "edges": [[from, to]] }`. Maps to
`api::client::workflow_graph()`; node names are class names; `edges` populate `NodeState.depends_on`
(and the `petgraph` DAG in `monitor::graph::build_layout`).

### Trigger → `api::client::trigger_workflow`

`POST /` with `{ "workflow_type": str, "data": object }` → `202 { "task_id": str, "event_id": str,
"message": str }`. The canonical contract's v1.2.0 adds `event_id` (the `events.id` row just
created) alongside the existing Celery `task_id` — it is the id to poll with the new
`GET /events/{event_id}` read route. `bastion` does not read `event_id` off the response yet;
consuming it is future work, not this re-pin.

### Abort → `api::client::abort_run`

`POST /events/{run_id}/abort` (no body) → `401` (bad/missing `X-API-Key`) \| `404` (unknown or
finished run) \| `202 { "run_id": str, "status": "aborting" }`. Consumed by
`api::client::ApiClient::abort_run`, the thin I/O shell behind the shipped `bastion abort <run>`
subcommand (**not** `bastion kill` — see the naming deviation in `planning/7.C-cost-budget-alerts-
abort/tasks.md` — `kill` stays the tmux session-kill verb). The endpoint itself is served by
`engine-serve`'s route table embedded into `bastion serve` (D48) — never by the Python
orchestrator, which has no abort endpoint and never will (D48 supersedes `OR.I`).

`api::client::classify_abort_response` maps each pinned response to a typed `AbortOutcome`:

| Response | `AbortOutcome` variant | `bastion abort` reports |
|---|---|---|
| `202 { run_id, status }` | `Accepted { run_id, status }` | `abort accepted: run <id> is now '<status>'` |
| `404` | `NotFound(ConsoleError::SessionNotFound)` | `abort failed: run '<id>' not found or already finished` (`C002`) |
| `401` | `Unauthorized(ConsoleError::NotAuthenticated)` | `abort failed: engine rejected the request` (`C012`) |
| connection failure / missing `engine_api_key` | `Err(ConsoleError::Io \| ConfigError)` | `abort failed: could not reach the engine` (`C009`/`C005`), pointing at `bastion serve` |

Every branch is unit-tested element-by-element in `src/run/abort.rs` (`render_outcome`) and
`src/api/client.rs` (`classify_abort_response`); the end-to-end path (real HTTP against a real
`engine-serve` `App`) is covered by the in-process integration test `tests/abort_contract.rs`.

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
| 1.1.0 | 2026-07-16 | Mapping change against the same pinned version (`BA.7.C`, no canonical bump) — the two v1.1.0 additions registered above are now consumed: the abort endpoint by `api::client::abort_run` / `bastion abort <run>`, served by `engine-serve` embedded in `bastion serve` (D48); the `metadata.cancellation` / `metadata.budget` annotations by `db::workflows::derive_run_status` into `WorkflowRun.status`/`budget_halt`. |
| 1.2.0 | 2026-07-24 | Re-pin from 1.1.0 to 1.2.0 (`OR.Y`). Registers the canonical's v1.2.0 additions, none yet consumed by bastion Rust types: the orchestrator's own `GET /events/{event_id}` read route (§ Monitor / Inspect above — no longer reserved), `event_id` on the `POST /events/` 202 body (§ Trigger above), and the `metadata.failure` run-level annotation (§ Run-level `metadata` annotations above). Wiring any of these into `db::workflows`/`api::client` is future work. |
| 1.3.0 | 2026-07-24 | Re-pin from 1.2.0 to 1.3.0 (`OR.Q`). Registers the canonical's v1.3.0 additions: `POST /ingest/proposal` and `POST /ingest/artifact` (canonical § 7 HTTP surface) — the ingest seam engine-rs hybrid workflows POST finished artifacts through into `brain_documents`. No mapping or Rust-type change here: `bastion` is a **read-only Postgres observer** (§ above) with no artifacts to ingest, so it never calls either route and consumes neither. |
