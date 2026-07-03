---
type: TaskSpec
title: "Task Spec — Phase 15, Block BA.15.12 (mev/okf-core format convergence — okf-core side)"
description: Bastion-side okf-core extension for BA.15.12 — add a state.json serde schema + block-dependency graph model, reconcile the OkfFrontmatter model with mev's, and add a graph/edge-resolution model, so mev can repoint at okf-core and delete its dupes.
doc_id: 15-12-mev-okf-core-convergence-tasks
layer: [console, factory]
project: bastion
status: active
keywords: [okf-core, BA.15.12, state-schema, OkfFrontmatter, graph-resolution, mev]
related: [D15-mev-integration-cross-repo-path-dep, D16-ba15-12-scope-widened-graph-resolution]
---

# Task Spec — Phase 15, Block BA.15.12 (mev/okf-core format convergence)

**Status:** Done · **Last run:** 2026-07-03 (4/4 tasks PASS, review PASS)

## Goal
Extract into `okf-core` the three shared-format models mev still owns privately — a `state.json`
serde schema + block-dependency graph, a reconciled `OkfFrontmatter`, and a graph/edge-resolution
model — so mev's `brain/okf.rs`/`state.rs`/`graph.rs`/`graph_emit.rs` can later repoint at `okf-core`
and delete their duplicates.

## Scope decision (read first)
This block is cross-repo (D15/D16), but **only the bastion-side `okf-core` extension lands here**.
The mev-side half (delete dupes, add the `okf-core` path dep, repoint callers) is a **separate SDLC
run in `../mev`'s own repo** — that spec is already written at
`../mev/planning/ticket-ba15-12-okf-core-convergence/` (tasks.md + tasks.json) and is explicitly
`blocked` in mev's `status.md`/`state.json` waiting on *this* spec to ship (`okf-core` has no state
schema / graph model yet). The mev-side **context is already seeded** — the D15/D16 mirror decision
exists as `../mev/planning/decisions/D9-ba15-12-okf-core-convergence-mirror.md` — so no context-seed
task is needed here (this closes the `ba15-12-mev-context-seed` carryover in this repo's
`state.json`; clear it after this spec commits). The single-repo worktree model is the reason the two
halves are separate specs: a bastion worktree cannot touch `../mev`'s source tree.

**Deferred to the mev-side spec, not judged here:** `bastion validate-brain`/`bastion graph` output
parity with `mev` on the whole brain corpus. That end-to-end parity can only be asserted once mev has
actually repointed at these `okf-core` models — it is the mev-side ticket's acceptance bar (and a
brain-side v2 `state.json` re-seed coordination step). This spec's bar is narrower: the models exist,
compile, serde-round-trip against real fixtures, and match `../planning/state-schema.md` +
`GraphExport` version "2".

## Context Pointers
- **Plan:** `planning/master-plan.md` → *Block BA.15.12* (the What / Files / Interfaces / Out-of-scope).
- **Decisions:** `planning/decisions/D15-mev-integration-cross-repo-path-dep.md` (split origin, path-dep
  direction, acyclic `mev → okf-core`), `D16-ba15-12-scope-widened-graph-resolution.md` (graph module
  now in scope).
- **Canonical schema to match:** `planning/state-schema.md` — the `state.json` contract the new
  `okf-core` state model must serialize to/from byte-faithfully.
- **Source shapes to reconcile / mirror (mev, read-only reference — do not edit here):**
  - `../mev/src/brain/okf.rs:35` `OkfFrontmatter` — `serde_yaml`-based; `layer`/`keywords`/`related`
    are `Option<Vec<String>>` (vs `okf-core`'s non-Option `Vec<String>`) and it carries a
    `synced_from: Option<String>` field `okf-core` lacks today.
  - `../mev/src/brain/state.rs` — `StateLoadError`, `Block`, `Focus`, `Track`, `TrackBlock`,
    `Carryover`/`CarryoverScope`, `RepoRollup`, `Endpoint`, `CrossRepoEdge`, `TierEntry`, `Origin`,
    `Backlog`, `BlockedBy`, `StateFile`, `load_state`; graph: `StateNode`, `StateEdge`,
    `StateEdgeKind`, `StateGraph`, `build_state_graph`.
  - `../mev/src/brain/graph.rs:199` `EdgeResolution` / `:228` `resolve_edge` + `Edge`/`EdgeKind`/
    `Node`/`Graph`/`GraphArtifact`; `../mev/src/brain/graph_emit.rs:30` `GraphExport` (version "2") /
    `:55` `ExportedEdge` / `build_graph_export`.
- **Target crate:** `crates/okf-core/` — today `lib.rs` (13 lines) exports only `frontmatter` +
  `parse` (605 lines total). This spec adds `state`, `graph`, `graph_emit` modules and reconciles
  `frontmatter`.
- **Standing rules:** `CLAUDE.md` rules 1 (tests ship with every change), 6 (coverage bar — pure
  logic exhaustively unit-tested, error/degradation paths covered, thin I/O shells smoke-tested), 7
  (`bella-engine` unrelated here). **Extract the pure serde/model layer only** — mev's validation and
  derivation logic (`check_*` / `derive_*`) stays in mev, consuming these shared types.

## Step-by-Step Tasks
See `tasks.json` in this directory — the task list is defined there, not here.

## Acceptance Criteria
- `crates/okf-core/` gains a `state` module whose serde types round-trip (serialize → parse → equal)
  against real brain `state.json` fixtures and match every field/shape documented in
  `planning/state-schema.md`; the block-dependency graph types (`StateGraph` + `build_state_graph`)
  reproduce mev's node/edge structure for a multi-block fixture.
- `crates/okf-core::OkfFrontmatter` is reconciled to mev's shape: `layer`/`keywords`/`related`
  tolerate both present-list and absent forms, and a `synced_from` field is present; existing
  `serialize_frontmatter` output is unchanged for inputs that don't set the new field (no regression
  for current bastion callers).
- `crates/okf-core/` gains a graph/edge-resolution model exposing `resolve_edge` (with an
  `EdgeResolution` result) and a `GraphExport`/`ExportedEdge` emitter whose serialized form carries
  `version: "2"` and the same fields as mev's `build_graph_export`.
- Every new/changed pure function is unit-tested directly (arg/shape assertions, resolution branches,
  error variants), including the absent-field and malformed-input paths — not just happy paths.
- All four gated checks pass: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`,
  `cargo build --release`. Combined test count is not lower than before.
- No `../mev` source files are edited by this spec (the mev repoint is a separate downstream run).

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```
<!-- Standard bastion checks from planning/harness.json (validation.checks[]). -->

## Notes
- **Scope was narrowed from the handoff's Option-A plan during task-gen:** the handoff assumed the
  mev-side context still needed seeding, but `../mev` already carries `D9` (the D15/D16 mirror) and a
  written, `blocked` dedup ticket. So this spec is the pure okf-core extension (the handoff's Option
  B), and the `ba15-12-mev-context-seed` carryover in this repo's `state.json` is already satisfied —
  clear it after committing this spec.
- Follow-up after this ships: run mev's `planning/ticket-ba15-12-okf-core-convergence/` spec in the
  `../mev` repo (it unblocks once these models exist), then assert the corpus-wide
  `validate-brain`/`graph` parity + `combined test count` bar there.

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
