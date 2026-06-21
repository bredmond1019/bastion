---
type: Spec
title: Phase 1 Block A — DB queries + graph layout
description: Implement the data layer for bastion monitor: parse live run state from PostgreSQL events table, and build a petgraph DAG with topological grid layout.
---

# Phase 1 Block A — DB queries + graph layout

## Goal

Implement the data layer that `bastion monitor` depends on: parse live workflow run state from
the orchestrator's `events` table (read-only), expose typed Rust structs for node status/timing/
error/input/usage, and construct a `petgraph` DAG with a topological grid layout ready for the
ratatui render loop. All logic is covered by unit tests against captured fixtures before any TUI
work begins.

## Context Pointers

- `planning/context.md` — project orientation and governing principles
- `planning/master-plan.md` — Phase 1 Block A specification
- `docs/data-contract.md` — pinned contract v1.0.0 (field mappings bastion follows)
- `src/db/workflows.rs` — stubs for `list_active_runs` and `get_run_state`
- `src/api/client.rs` — `workflow_graph()` already implemented; `WorkflowGraph` type defined
- `src/monitor/graph.rs` — stub for `build_layout`
- `planning/decisions/D2-observability-consumer-contract.md` — read-only observer rule
- `planning/decisions/D3-pin-data-contract.md` — contract pinning decision

## Tasks

### 1. Add test fixtures for `events.task_context` JSON parsing

Create `src/db/fixtures/` containing at least two captured `task_context` JSON blobs:
one representing an in-progress run (mix of `pending`, `running`, and `success` nodes) and one
representing a completed run (all nodes `success` or `failed`). These are static `.json` files
used in tests; no real DB is required for unit tests. Depends on: none.

### 2. Implement `db::workflows` — `node_runs` JSON → `NodeState` parsing

In `src/db/workflows.rs`, add a private parsing layer that deserializes
`task_context.node_runs` and `task_context.nodes` from a `serde_json::Value` into
`Vec<NodeState>`. The join: for each key in `node_runs`, populate `NodeState` fields from
`node_runs[name]` (status, error, input, usage.input_tokens, usage.output_tokens, usage.model,
started_at) and from `nodes[name]` (output). `RunStatus` must deserialize from
`pending|running|success|failed` via `#[serde(rename_all = "lowercase")]`. Derive
`WorkflowRun.status` by aggregating node statuses: any `running` → `Running`; any `pending`
but none `running` → `Pending`; all terminal and any `failed` → `Failed`; all `success` →
`Success`. Write unit tests against the Task 1 fixtures covering: correct status derivation,
null `usage` fields becoming `None`, partial runs with mixed statuses, and all four `RunStatus`
deserialization variants. Depends on: 1.

### 3. Implement `db::workflows::list_active_runs` and `get_run_state`

Fill in the two `todo!()` stubs using `sqlx`. `list_active_runs` issues a single
`SELECT id, workflow_type, task_context, created_at FROM events` query, parses each row with
the Task 2 parsing layer, and filters out rows where all `node_runs` statuses are terminal
(`success` or `failed`). `get_run_state` loads one row by `id`. Both accept a `&str` DB URL
and create a short-lived `sqlx::PgPool` internally (Phase 1 scope; connection pooling is a
Phase 4 concern). Neither function modifies the database (D2). These functions are integration
points; unit tests use the fixture-based parsing layer from Task 2. Add at least one
`#[cfg(test)]` integration test stub (gated behind a `#[ignore]` attribute and a
`BASTION_INTEGRATION_TEST` env var) that documents the expected call shape. Depends on: 2.

### 4. Implement `monitor::graph::build_layout`

Fill in the `todo!()` stub in `src/monitor/graph.rs`. Construct a `petgraph::graph::DiGraph`
from `WorkflowGraph.edges` (node names are vertices; each edge tuple becomes a directed edge).
For each node in `WorkflowGraph.nodes` not already in the graph (pending nodes not yet in
`node_runs`), add it as an isolated vertex. Overlay live status from `nodes: &[NodeState]` by
joining on class name (node name string). Compute a topological column assignment using
`petgraph::algo::toposort`; nodes in the same topological depth share a column. Assign row
positions within a column in the order they appear in the toposort result. Store the result as
`positions: Vec<(usize, u16, u16)>` where the tuple is `(node_index_in_DiGraph, col, row)`.
Write unit tests covering: a linear three-node chain produces three distinct columns; a
diamond DAG (A→B, A→C, B→D, C→D) produces correct depth assignments; an isolated node
(no edges) has position (0, 0); a live-state overlay assigns the correct `RunStatus` to a
named node. Depends on: 2.

### 5. Validate — all gates pass

Run the full validation suite: `cargo fmt --check`, `cargo clippy -- -D warnings`,
`cargo test`, `cargo build --release`. All four must pass with zero errors. Depends on: 1, 2, 3, 4.

## Acceptance Criteria

- Unit tests cover `node_runs` JSON → `NodeState` parse against captured fixtures.
- All four `RunStatus` variants (`pending`, `running`, `success`, `failed`) deserialize correctly.
- `usage` being `null` in a fixture produces `None` for `tokens_in`, `tokens_out`, `model`.
- The class-name join in `build_layout` correctly overlays live `NodeState` onto graph nodes.
- Topological ordering and position assignment are verified by unit tests (linear chain, diamond DAG, isolated node).
- `list_active_runs` and `get_run_state` stubs are filled with `sqlx` queries; an `#[ignore]` integration stub documents the call shape.
- `cargo fmt --check` passes.
- `cargo clippy -- -D warnings` passes with zero warnings.
- `cargo test` passes (all non-`#[ignore]` tests green).
- `cargo build --release` passes.
- No writes to the orchestrator's PostgreSQL (D2 enforced).

## Validation Commands

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes

- `workflow_graph()` in `src/api/client.rs` is already implemented and tested; Task 4 consumes it
  but does not re-implement it.
- The `sqlx` DB calls in Task 3 will not execute in CI without a live DB; gate them with
  `#[ignore]`. The unit tests in Task 2 and 4 are the authoritative test coverage for this block.
- Phase 1 Block B (TUI render loop) depends entirely on this block being complete and tested.
- Connection pooling, caching, and SSE streaming are Phase 4 concerns — do not implement ahead of phase.
