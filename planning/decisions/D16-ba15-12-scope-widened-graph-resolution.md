---
type: Decision
title: "D16: BA.15.12 scope widened to include mev's graph.rs/graph_emit.rs resolve-edge surface"
description: BA.15.12 (mev/okf-core format convergence) as scoped by D15 covered only mev's brain/okf.rs + brain/state.rs. mev shipped MV.3B.V since then, adding a resolve-edge module (graph.rs + graph_emit.rs, 1,089 lines) with zero okf-core counterpart. BA.15.12 now explicitly includes reconciling and converging that surface too, not just okf.rs/state.rs.
doc_id: D16-ba15-12-scope-widened-graph-resolution
layer: [console, brain]
project: bastion
status: active
keywords: [mev, okf-core, BA.15.12, graph-resolution, scope-widening]
related: [D15-mev-integration-cross-repo-path-dep]
---

# D16 — BA.15.12 scope widened: include mev's graph.rs/graph_emit.rs resolve-edge surface

**Date:** 2026-07-03
**Status:** Accepted
**Supersedes:** —
**Builds on:** D15 (BA.15.2 split; BA.15.12 defined as the mev/okf-core format-convergence block).
Amends BA.15.12's scope only — does not reopen D15's decision to keep mev as a cross-repo path
dependency, and does not change BA.15.2 (already shipped).

## Context

D15 scoped BA.15.12 around a specific, then-current duplication: mev's `brain/okf.rs` (899 lines) and
`brain/state.rs` (5,383 lines) each re-implement a format `okf-core` also implements (`frontmatter.rs`
+ `parse.rs`, 605 lines) — ~6,282 duplicate lines, the number carried in `state.json`'s
`ba15-12-mev-context-seed` carryover and in `master-plan.md`'s BA.15.12 write-up.

Since D15 was written, mev shipped **MV.3B.V** (2026-07-03): a `resolve_edge(artifact, edge) ->
EdgeResolution` helper in `mev/src/brain/graph.rs` (807 lines), consumed by both `check_graph` and a
now-versioned `GraphExport` in `graph_emit.rs` (282 lines; `version` "1" → "2", `ExportedEdge` gained
nullable `target_node_id`/`target_doc_id`). `okf-core` has no `graph.rs`/state-graph equivalent at all
today. This 1,089-line module is exactly the same shape of problem D15 named (a format/logic
implementation that only exists in mev, with `okf-core` as bastion's designated convergence target) —
it just didn't exist yet when D15 was written. Leaving it out of BA.15.12 would mean the block ships
"one parser" for OKF/state but not for graph resolution, re-creating the same drift risk BA.15.12
exists to close.

## Decision

**BA.15.12's scope is widened to include `mev/src/brain/graph.rs` + `graph_emit.rs`, alongside the
original `okf.rs` + `state.rs`.** Concretely:

1. `okf-core`'s reconciliation target is no longer just a state schema + `OkfFrontmatter` model — it
   also needs a graph/edge-resolution model (`resolve_edge` / `EdgeResolution` / `ExportedEdge`,
   or whatever shape the reconciliation converges on) before mev's `graph.rs`/`graph_emit.rs` dupes
   can be dropped.
2. mev's `brain/graph.rs` + `brain/graph_emit.rs` are repointed at `okf-core` in the same pass as
   `okf.rs`/`state.rs`, in mev's own repo, as part of BA.15.12's cross-repo execution (unchanged from
   D15: BA.15.12 runs partly in mev's own repo).
3. The duplicate-line accounting BA.15.12 tracks grows from ~6,282 lines (`okf.rs` + `state.rs`) to
   ~7,371 lines (adding `graph.rs` + `graph_emit.rs`, 1,089 lines as of MV.3B.V — this will drift
   further if mev's `brain/` module keeps growing before BA.15.12 is executed; re-measure at
   `/generate-tasks` time rather than trusting this figure).
4. Everything else D15 decided is unchanged: mev stays a cross-repo path dependency (not absorbed or
   vendored), BA.15.2 is unaffected (already shipped), and BA.15.12 remains deferred/not-yet-tasked
   until `/generate-tasks` runs against it.

## Consequences

- `master-plan.md`'s BA.15.12 write-up, `bastion-product/plan.md`'s BA.15.12 summary + critical-files
  row, and `state.json`'s BA.15.12 block title are updated in the same pass as this decision to reflect
  the widened scope — see those files' current text rather than duplicating it here (this doc records
  *why* the scope changed, not the current spec, which will keep evolving).
- The `mv3bv-graph-resolution-surface` carryover in `state.json` is resolved by this decision (its
  `clears_when` condition — "the full mev port is scoped into a block" — is now met) and removed.
- BA.15.12's eventual `/generate-tasks` pass has a larger surface to plan against than D15 alone would
  have suggested; the mev-side context-seeding carryover (`ba15-12-mev-context-seed`) still applies and
  now additionally covers `graph.rs`/`graph_emit.rs`.
