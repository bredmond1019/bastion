---
type: Handoff
created: 2026-07-03
---

# Handoff — BA.15.2 shipped; scope BA.15.12 next (cross-repo)

> **For the next agent:** Read this immediately after `/prime`. Delete this file once consumed.

## What we're doing and why
`bastion` is mid-Phase-15 (BA.15, Bastion Product Packaging). The prior session shipped
**BA.15.2 (Unify the CLI, bastion-side)** — folding `mev`'s brain-ops commands
(`validate-brain`/`manifest`/`graph`/`emit-state`) and `bella`'s document viewer (`view`/`edit`)
into the `bastion` binary as thin pass-throughs, per the bastion-side split of D15. This
session was a follow-up scoping/verification pass (no code changes) confirming what's next:
**BA.15.12** (mev/okf-core format convergence) is the correctly-unblocked next Phase 15 block,
and it is **genuinely cross-repo** — half the work lands in `bastion` (extend `okf-core`), half
in `mev`'s own repo (delete its duplicate OKF/state code, repoint at `okf-core`). The critical
finding this session: **`mev`'s own planning docs have zero awareness of this work** — no D15
mirror, no `state.json`/`status.md` mention — so scoping it isn't just a `bastion`-side
`/generate-tasks` call.

## Completed this session
- Verified `BA.15.12` is correctly tracked and unblocked: confirmed in `planning/state.json`
  `tracks[]` (`status: "open"`, `depends_on: [BA.15.1, BA.15.2]`, both now closed), in
  `planning/decisions/D15-mev-integration-cross-repo-path-dep.md` (where it was split off from
  the original BA.15.2 scope), and in `planning/master-plan.md` §BA.15.12 /
  `planning/bastion-product/plan.md`.
- Produced a full line-level accounting of BA.15.2's diff (`08f9201..b5c75c7`) to confirm no
  `mev`/`bella` source was ever touched — every one of the 23 changed files sits under
  `core/bastion/`. Net-new code: `crates/bastion/src/brainval/mod.rs` (530 lines),
  `crates/bastion/src/docview/mod.rs` (228 lines), 4 new `Commands` enum variants in `cli.rs`,
  dispatch wiring in `main.rs`, one new `mev = { path = "../../../mev" }` dep line. Nothing was
  deleted or altered anywhere else.
- Investigated the actual size/shape of the BA.15.12 dedup: `mev/src/brain/okf.rs` (899 lines,
  its own `serde_yaml`-based `OkfFrontmatter` with list-valued `layer` — diverges from
  `okf-core`'s hand-rolled model) and `mev/src/brain/state.rs` (5,383 lines, a full `state.json`
  schema + block-dependency graph engine — `okf-core` has **zero** equivalent today, so this is
  net-new code in `okf-core`, not a mechanical move). ~6,282 duplicate/divergent lines total vs.
  612 lines currently in `okf-core`.
- Confirmed no dependency-cycle risk: `bastion → mev`, `bastion → okf-core` already exist; adding
  `mev → okf-core` (via `okf-core = { path = "../bastion/crates/okf-core" }` in mev's
  `Cargo.toml`) stays acyclic since `okf-core` depends on nothing back — D15 states this
  explicitly. `okf-core` does **not** need to move to its own repo.
- Added `carryover[]` entry `ba15-12-mev-context-seed` to `planning/state.json` (see Durable
  State Updates) capturing the mev-context gap, then ran `mev emit-state --write` (0 errors).

## Remaining work
- **Run `/generate-tasks` for BA.15.12 — but this is a two-repo scoping job, not one spec:**
  1. In **this repo** (`bastion`): scope the `okf-core` extension — add a `state.json` serde
     schema + block-dependency graph (mirroring `mev/src/brain/state.rs`'s shape) and reconcile
     `okf-core`'s `OkfFrontmatter` model with mev's (`serde_yaml`, list-valued `layer`) into one
     shared model. This is new code, not a copy — budget for genuine design work here.
  2. In **`/Users/brandon/Dev/agentic-portfolio/core/mev`**: this repo needs context seeded
     *before or during* task generation — it currently has no idea this work exists. At minimum:
     a decision doc there mirroring D15's shape (why mev is giving up its own OKF/state code for
     a shared crate, the path-dep direction, the parity requirement) plus a
     `planning/status.md`/plan entry. Then scope the actual dedup task(s): delete
     `brain/okf.rs` + `brain/state.rs`'s duplicate logic, add the `okf-core` path dep, repoint
     callers.
  3. Acceptance bar for both halves (from D15/master-plan, migrated into whatever task spec(s)
     get written): mev's dupes deleted; `bastion validate-brain` output byte-identical to `mev`'s
     own output on the **whole brain corpus**; combined test count not lower; gated checks green
     in **both** repos.
- Given the real size (~6,282 lines, a genuine model reconciliation — not mechanical), consider
  splitting into two task specs (one per repo) rather than one `/sdlc-flow` spanning both trees,
  since the harness's worktree model is single-repo.
- Alternative if BA.15.12 scoping stalls or there's no appetite yet (D15 explicitly says this is
  "the risky half," undertaken only when ready): resume Phase 13/14 blocks per `state.json`'s
  `focus.next` ordering (`BA.7.B`, `BA.11.E`, `BA.13.2`, `BA.13.3`, `BA.13.5`, `BA.14.1-3`, ...).

## Durable State Updates
- `state.json` `carryover[]`: added `ba15-12-mev-context-seed` (`kind: constraint`,
  `scope: {cross_repo: true}`) — states that `/generate-tasks` for BA.15.12 must seed context
  into `mev`'s own repo (decision doc + status/plan entry) before/while writing the mev-side task
  spec, not just author a `bastion`-side spec. `clears_when`: "BA.15.12's task spec(s) exist in
  both bastion and mev repos with mev-side context seeded." Delete this entry once that's done.
- `state.json` `focus`: regenerated via `mev emit-state --write` (0 errors) after the carryover
  edit — no block statuses changed this session (BA.15.2 was already closed last session).
- No `tracks[].blocks[]` or `tasks.json` changes this session — this was scoping/verification
  only, no task spec written yet.

## Open questions / choices
- **Does BA.15.12 get one task spec or two (one per repo)?** Not decided — flagged above as a
  recommendation (two specs, given the harness's single-repo worktree model), not a settled
  choice. Decide this when running `/generate-tasks`.
- Everything else about the *shape* of the work (dependency direction, no repo move, the parity
  acceptance bar) is already settled per D15 — no open question there.

## Context the next agent needs
- A visual dependency-graph artifact was produced this session showing the current
  (`bastion → mev` path dep + `bastion → bella` subprocess spawn) and planned
  (`mev → okf-core` new path dep, mev's dupes struck through) states — useful for onboarding
  whoever scopes BA.15.12, but it's a conversational artifact, not committed to the repo; regenerate
  from this handoff's "Completed this session" bullets if wanted again.
- `mev`'s CLI surface and output are explicitly **not** allowed to change as a side effect of
  BA.15.12 — that's the acceptance bar, not a nice-to-have (see Remaining work §3). Whoever scopes
  the mev-side task(s) should write that constraint directly into the mev-side task spec's
  acceptance criteria, not just rely on this handoff.

## First command after `/prime`
`/generate-tasks 15.12-mev-okf-core-dedup` — but read "Remaining work" above first: this needs a
plan for how to split (or sequence) work across `bastion` and `mev` before task-gen actually runs,
and the mev-side context-seeding has to happen alongside it.
