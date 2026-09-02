---
type: Reference
title: bastion ⇄ Orchestrator Data Contract (Consumer)
description: bastion's pinned view of the orchestrator's versioned data contract — how each contract field maps to bastion's Rust types. The canonical contract lives in engine-rs (D78).
doc_id: data-contract
layer: [console, engine]
project: bastion
status: active
keywords: [data contract, orchestrator, node_runs, field mappings, v1.7.0, cancellation, budget gate]
related: [monitor, costs, inspect, run]
---

# Data Contract (Consumer View)

**Pinned Contract Version: 1.10.0**

The **canonical, authoritative** contract is owned by engine-rs per
[D78](file:///Users/brandon/Dev/agentic-portfolio/docs/decisions/D78-engine-rs-owns-the-data-contract.md)
(2026-08-21): `engine-rs/docs/data-contract.md`. This file is bastion's *consumer* view — it pins
the version bastion is built against and maps each contract field to bastion's Rust types.

> bastion is an **observer, never a writer** of the `events` row itself (D2) — it never opens a
> write connection to PostgreSQL. As of the canonical contract's v1.1.0, bastion may *trigger* a
> write the orchestrator/engine performs on its own behalf (`POST /events/{run_id}/abort`, per
> brain decision D25: "bastion triggers, the Engine executes") without becoming a writer itself —
> see the canonical doc's §3 for the reconciled prose. When the canonical contract bumps, re-pin
> the version here and update the mappings. The `/log-work` checklist prompts this.

## Quickstart

This page is a **pin**, not a tutorial — there is nothing here to run. Use it for one of two
things:

| You are… | Go to |
|---|---|
| checking which version of the canonical contract this repo is built against | the **Pinned Contract Version** line at the top of this page |
| reacting to the canonical contract bumping | [Re-pin checklist](#re-pin-checklist-when-the-canonical-contract-bumps) |

The canonical document is owned by another repo (named just above). **Never edit the mappings
here to describe new upstream behaviour without bumping the pinned version** — a mapping that
silently describes a newer contract than the pin claims is the failure this file exists to
prevent.

---

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

The canonical contract's **v1.7.0** adds a fourth run-level annotation, `metadata.completion` —
`{ "completion": { "completed": true } }`, stamped by `Workflow.run` when the node walk ends
normally. **This one is not cosmetic for bastion, and it is not yet consumed.**
`db::workflows::derive_run_status` (`src/db/workflows.rs`) aggregates `node_runs` with the rule
`has_running → Running`, else `has_pending → Pending`, else `has_failed → Failed`, else `Success`.
The orchestrator seeds *every* node in the DAG `pending` before the walk starts (canonical §2), so
a branch the router never takes is still `pending` when the run finishes — which means **bastion
currently reports every completed run of a branching workflow as `RunStatus::Pending`, forever.**
Confirmed live on `DOCUMENT_QA`, whose `AbstainNode` never runs on an answered query.

This is the same defect the canonical repo fixed on its own read side by adding this marker; the
Rust half is unfixed. The fix is to check `metadata.completion.completed` before the `has_pending`
arm (and after the cancellation/budget/suspension arms, matching the canonical's precedence).
Deliberately **not** done in this re-pin — it is a behaviour change to a function five surfaces
read, so it wants its own ticket and its own tests, not a doc-sync commit.

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
| 1.4.0 | 2026-07-27 | Re-pin from 1.3.0 to 1.4.0 (`OR.Q2`). Registers the canonical's v1.4.0 additions: `GET /recall`, `GET /walk`, `GET /pulse` (canonical § 7 HTTP surface) — the read half of the D51 HTTP adapter whose write half (`POST /ingest/*`) landed in v1.3.0, thin authenticated adapters over the orchestrator's `app/brain/` read core (`retrieval.recall` / `graph.walk` / `pulse.pulse`). No mapping or Rust-type change here: bastion's monitor/inspect/costs surfaces read `events`/`task_context` only (§ Read paths above) and have no corpus-query or health-snapshot use case today, so none of the three routes is called or consumed by any bastion Rust type; wiring bastion to any of them (e.g. a `bastion recall`/`bastion pulse` subcommand) is future work, not this re-pin. |
| 1.5.0 | 2026-08-01 | Re-pin from 1.4.0 to 1.5.0 (`OR.ticket.corpus-reconcile`, orchestrator-side). Registers the canonical's v1.5.0 addition: `POST /ingest/proposal` and `POST /ingest/artifact` gain an optional `authored_at: datetime \| null` field, threaded to the written `brain_documents` rows. Purely additive and backward-compatible — omitting the field (or sending `null`) preserves the pre-existing `datetime.now()` fallback exactly. No mapping or Rust-type change here: bastion is a **read-only Postgres observer** (§ above) with no artifacts to ingest, so it calls neither route and consumes neither. |
| 1.6.0 | 2026-08-01 | Re-pin from 1.5.0 to 1.6.0 (`OR.K2`, orchestrator-side). Registers the canonical's v1.6.0 change to `GET /recall`'s response: **`score` polarity is flipped — higher is now always better** on every path (`1.0` for an exact-id match, `1.0 - cosine distance` for semantic, unchanged fused similarity for hybrid), where the exact-id/semantic paths previously returned a raw cosine *distance* (`0.0` for an exact-id match, lower-is-better). `via`'s vocabulary widens from `exact-id \| semantic \| hybrid` to also include `structural \| keyword \| memory` (the hybrid path's per-candidate provenance, previously collapsed to a bare `"hybrid"`). Field names, types, and the `q`/`limit`/`hybrid` query params are unchanged, so the canonical flags it Minor — but **any consumer that sorts or thresholds on `score` must re-verify its comparison direction**, because an old-polarity comparison ranks results backwards with no error. No mapping or Rust-type change here: bastion calls none of the corpus-read routes today (§ 1.4.0 row above), so nothing in `db::`/`api::client` compares a `score`. Whenever a `bastion recall` surface is wired, it must be built against 1.6.0 semantics — higher-is-better, and `via` may be any of the six values. |
| 1.7.0 | 2026-08-13 | Re-pin from 1.6.0 to 1.7.0. Registers the canonical's v1.7.0 addition: the run-level `metadata.completion` annotation (§ Run-level `metadata` annotations above), stamped by `Workflow.run` on its normal exit path and used by the canonical's `derive_status` to report `succeeded` despite a leftover `pending` node. **Unlike the previous four re-pins, this one records a real bastion-side defect rather than an inert addition.** `db::workflows::derive_run_status` aggregates `node_runs` with `has_pending → Pending`, and the orchestrator seeds every node in the DAG `pending` before the walk starts — so the branch a router never takes keeps a finished run looking unfinished, and **bastion reports every completed run of a branching workflow as `RunStatus::Pending` forever** (confirmed live on `DOCUMENT_QA`, whose `AbstainNode` never runs on an answered query). Consuming the marker is the fix, and is deliberately left to its own ticket: `derive_run_status` backs five surfaces and a behaviour change there needs its own tests, not a doc-sync commit. No mapping or Rust-type change in this re-pin. |
| 1.8.0 | 2026-08-23 | Re-pin from 1.7.0 to 1.8.0 (`OR.3.A`). Registers the canonical's v1.8.0 addition: `POST /ingest/artifact` (canonical § 7 HTTP surface) gains six optional `LearningArtifact` fields — `channel_type`, `source_ref`, `summary`, `digest_markdown`, `entities`, `language` — and relaxes `doc_type`/`content` from required to optional, with route-side fallbacks (`content` <- `digest_markdown`, `doc_type` <- `"learning_artifact"`). No mapping or Rust-type change here: bastion is a **read-only Postgres observer** (§ above) with no artifacts to ingest, so it never calls the ingest routes and consumes none of this addition. **This 1.8.0 pinned synapse's own 1.8.0 lineage row, not engine-rs's** — see the 1.10.0 row below. |
| 1.10.0 | 2026-09-02 | Re-pin from 1.8.0 to 1.10.0 (`EN.14.D`, engine-rs-side). **This is not a one-step bump.** Per D78 (2026-08-21) the canonical contract moved from synapse (the former orchestrator) to `engine-rs/docs/data-contract.md`; the frontmatter `description` and the canonical-ownership line above, which still said "the Python repo"/"owned by the orchestrator," are corrected in this same change. Registers engine-rs's canonical v1.10.0 addition: `model: String` and `started_at: Option<String>` on `ClaudeSession` — engine-rs's own ledger-entry type for the `claude_sessions[]` array, additive and not part of any shape bastion reads. **Because bastion's prior 1.8.0 pin was synapse-lineage** (`OR.3.A`, `/ingest/artifact`'s `LearningArtifact` fields), this jump ALSO silently picks up engine-rs's own 1.8.0 — `campaign_id` and `GET /campaigns/{id}` (`EN.11.E`, 2026-08-21) — which bastion has never registered as consumed. **Stating this explicitly so this doc-sync commit does not imply otherwise: bastion does not read `campaign_id` or call `GET /campaigns/{id}` as of this re-pin** — no mapping or Rust-type change accompanies this row, and wiring either remains open, separate work. |
