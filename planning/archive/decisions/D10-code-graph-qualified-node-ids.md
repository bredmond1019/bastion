---
type: Decision
title: "D10: Code graph uses qualified node IDs to prevent name collisions"
description: Code-as-graph BrainNodes use file_stem::kind::name as their id; BrainGraph gains a name_index for bare-name CLI queries.
doc_id: D10-code-graph-qualified-node-ids
status: active
keywords: [code-graph, brain, node-id, collision, petgraph, qualified-id]
related: [D4-session-management-surface]
---

# D10 — Code graph uses qualified node IDs to prevent name collisions

**Date:** 2026-06-26
**Supersedes:** none
**Builds on:** Phase 6 Block C (code-as-graph)

## Context

`build_code_node_edge_lists` originally set `BrainNode.id = symbol.name` (bare name). When a
file contains both `struct Widget` and `impl Widget`, `BrainGraph::build` calls
`index.insert(id, idx)` twice for the same key — the second overwrites the first. The struct
node exists in petgraph but is unreachable through the index; `--dependents Widget` silently
resolves only to the last-inserted Widget.

## Decision

**Node ids are qualified: `{file_stem}::{kind}::{name}`.**

Examples:
- `struct Widget` in `lib.rs` → `lib::struct::Widget`
- `impl Widget` in `lib.rs` → `lib::impl::Widget`
- `fn main_consumer` in `consumer.rs` → `consumer::fn::main_consumer`

`BrainNode.title` keeps the bare name for display.

`BrainGraph` gains a `name_index: HashMap<String, Vec<NodeIndex>>` populated from `node.title`
during `build`, and a `predecessors_by_name(&self, name: &str) -> Vec<BrainNode>` method that
aggregates predecessors across all nodes sharing a bare name. This is the entry point for CLI
`--dependents <bare_name>` queries.

`format_dependent_line` uses `node.title` (bare name) rather than `node.id` so CLI output
remains human-readable (`dependent: Widget\tlib.rs`, not `dependent: lib::struct::Widget\tlib.rs`).

## Alternatives considered

**Bare-name IDs with deduplication (last-wins):** Simpler — no graph changes, no ID format
change. But silently merges distinct definitions into one node (struct Widget and impl Widget
become one), losing edge fidelity. Rejected: the graph loses information without surfacing the
loss.

**Path + line number (`path:line:name`):** Unique, but unstable — every code move changes the
id. Qualified kind is more stable and sufficient to disambiguate all practical cases (`struct`
vs `impl` vs `trait` with the same name in the same file).

## Consequences

- All code-graph edges use qualified ids; the OKF brain graph is unaffected (its ids are
  doc_id strings, not affected by this module).
- `predecessors_by_name` is additive on `BrainGraph`; existing `predecessors(id)` method is
  unchanged and still used by the OKF brain path.
- The `name_index` adds O(nodes) memory overhead — negligible for typical codebases.
- CLI output format for `--dependents` is unchanged (bare name shown).
