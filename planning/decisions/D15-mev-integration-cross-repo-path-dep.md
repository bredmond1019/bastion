---
type: Decision
title: "D15: mev integration is a cross-repo path dep, not a workspace merge — and BA.15.2 splits into a bastion-side CLI block + a deferred mev-side convergence block"
description: mev stays its own repo and is consumed by bastion as an unpinned Cargo path dependency (like bella-engine); mev's internals are NOT refactored. BA.15.2 is split — the bastion-side CLI unification (calling mev's existing public API) is scoped now; the risky mev-side dedup (drop OKF/state dupes for okf-core) is deferred as BA.15.12.
doc_id: D15-mev-integration-cross-repo-path-dep
layer: [console]
project: bastion
status: active
keywords: [mev, mev-core, cross-repo dependency, path dependency, okf-core, BA.15.2, block split, workspace]
related: [D14-bella-engine-dependency-contract, master-plan, bastion-product-plan]
---

# D15 — mev integration: cross-repo path dep + BA.15.2 split

**Date:** 2026-07-03
**Status:** Accepted
**Supersedes:** —
**Builds on:** D14 (bella-engine cross-repo path-dep discipline — same shape, applied to mev); the
Phase 15 program in `planning/bastion-product/plan.md` + `master-plan.md` (Block BA.15.2).

## Context

BA.15.2 as originally written ("Unify the CLI; mev becomes `mev-core`") bundled three separable moves:
(1) a bastion-side CLI surface that folds mev's + bella's commands into the `bastion` binary; (2) a
mev-side refactor that drops mev's own OKF/`state.json` struct definitions in favor of `okf-core`; and
(3) an implicit workspace decision about *how* mev enters the build.

Exploration surfaced three load-bearing facts:

- **mev is a mature, separate git repo** (`core/mev`) and is *already* a library — it exports
  `validate_brain`, `emit_state`, `manifest_brain`, `visualize_brain`, `graph_brain` as top-level
  `pub fn`s. "Convert mev to a library" is essentially already done.
- **The "dupes" are large and divergent, not trivial.** mev's `brain/okf.rs` (899 lines) has a *different*
  `OkfFrontmatter` (list `layer`, `serde_yaml`-based, coupled to `brain.toml` vocab) than the hand-rolled
  `okf-core` model from BA.15.1; mev's `brain/state.rs` is **5,383 lines** of canonical state schema +
  graph + emit engine, whereas `okf-core` has *zero* state code today (the state schema was deferred out
  of BA.15.1). Unifying either carries whole-brain-corpus parity risk.
- **BA.15.0 did not absorb siblings into one workspace.** Contrary to plan.md's original "one workspace
  via `git subtree add`" framing, BA.15.0 kept `bella-engine` / `workflow-engine-*` as external Cargo
  **path deps** and never touched mev. The code precedent is now cross-repo path deps, not a monorepo.
- **Operational reality:** the graph + `state.json` validation we rely on daily lives in mev and *works*.
  bastion's own value is mission-control, workflow interaction, coding sessions, and `serve` — not a
  second implementation of mev's format engines.

## Decision

1. **mev is consumed as a cross-repo Cargo path dependency**, exactly like `bella-engine`
   (`mev = { path = "../mev" }` in `crates/bastion/Cargo.toml`). mev stays its own repo with its own
   binary and its own CI. **`core/` is the de-facto workspace boundary**, not `core/bastion/`. We do
   **not** `git subtree`-absorb mev into bastion's workspace, and we do **not** vendor/fork it (D24's
   vendor pattern does not apply here — mev is consumed live, not harvested).

2. **mev's internals are not refactored to reach the CLI-unification goal.** The bastion-side block
   calls mev's *existing* public API unchanged. mev's working graph/state validation is treated as a
   stable cross-repo contract (same discipline as D14 for bella-engine), not a thing to be re-plumbed.

3. **BA.15.2 is split:**
   - **BA.15.2 (redefined) — Unify the CLI (bastion-side).** Add `bastion` subcommands
     `validate-brain` / `emit-state` / `manifest` / `graph` that call mev via the path dep, plus
     `bastion view` / `edit` over `bella-engine`, following the existing declare→name→dispatch, DB-free
     pattern. No mev or bella source changes. **Scoped now** (`planning/15.2-unify-cli-bastion-side/`).
   - **BA.15.12 (new) — mev/okf-core format convergence.** *Deferred, not scoped.* Drop mev's OKF/state
     dupes in favor of `okf-core`; requires `okf-core` to first gain a state schema + a reconciled OKF
     model (the prerequisite that BA.15.1 explicitly deferred). This is the higher-risk convergence and
     is only undertaken when there is appetite to touch mev's working internals. The original BA.15.2
     acceptance criterion ("the four duplicate implementations deleted; parity on the whole brain
     corpus") migrates here.

4. **`bin-shims/` are dropped from scope.** They existed to preserve standalone `mev`/`bella` binaries
   under the absorb-into-workspace model. Under the cross-repo path-dep model those binaries already
   build from their own repos, so no shims are needed.

## Consequences

- The dependency shape is a clean crate DAG: `bastion → mev` and `bastion → okf-core`; `bastion → bella-engine`.
  mev does **not** depend on `okf-core` until BA.15.12 (if ever), so no cross-repo cycle is introduced now.
- The CLI-unification value (one `bastion` binary that drives brain validation, state emission, manifest,
  and graph) lands immediately and cheaply, with zero risk to mev's proven format engines.
- The "single implementation of each format" north-star is *not* achieved by BA.15.2 alone — it waits on
  the deferred BA.15.12. That is an accepted trade: correctness of the working tool over premature dedup.
- `okf-core`'s state-schema extraction + OKF-model reconciliation are re-homed as prerequisites of
  BA.15.12, not of any currently-scoped block.
