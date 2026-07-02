---
type: Handoff
created: 2026-07-02
---

# Handoff — Bastion Product packaging plan + OKF write-path head start

> **For the next agent:** Read this immediately after `/prime`. Delete this file once consumed.

## What we're doing and why
We're turning `bastion` from a personal CLI hardwired to Brandon's brain into a **self-contained,
open-source "agent OS"** others can adopt in one command (`bastion init` greenfield, `bastion assess`
diagnostic, `bastion adopt` later). Brandon confirmed he's happy to **consolidate** the sibling repos
(`mev`, `bella-engine`, `workflow-engine-rs`, `base-template`, optional Python `orchestrator`) into one
cargo workspace and collapse the three CLIs into **one `bastion` binary** (sub-tools become libraries).
The full sequenced roadmap is authored in **`planning/bastion-product/plan.md`** as bastion **Phase 15
(BA.15.0–BA.15.11)**. This session authored that plan and prototyped its linchpin (the OKF *write* path)
in-repo, deliberately avoiding the disruptive repo-move step until the design was locked.

## Completed this session
- **Authored `planning/bastion-product/plan.md`** — Phase 15 roadmap in bastion `PREFIX.PHASE.BLOCK[.TASK]`
  convention, 3 waves + 2 deferred blocks, with locked decisions (workspace-first, embedded templates,
  init+assess now / adopt deferred, Rust-first engine, unified CLI), critical-files table, and 6-point
  verification. Bakes in the two conventions Brandon specified: **`tasks.json` companion** to every
  `tasks.md` merged into `state.json` (BA.15.5) and the **`PREFIX.PHASE.BLOCK[.TASK]` naming engine** (BA.15.6).
- **Prototyped the OKF write path** in `src/okf/mod.rs` (new module, wired via `mod okf;` in `src/main.rs`):
  `OkfFrontmatter` model + `serialize_frontmatter()` (canonical field order, inline lists, conservative
  YAML-safe quoting). This is net-new — nothing in the stack could *emit* OKF frontmatter before (mev only
  validates). **18 unit tests, all green** (`cargo test okf::` → 44 passed incl. brain::okf); `rustfmt --check`
  clean; **0 clippy findings** on the module. This is the head start on **BA.15.1** (okf-core).
- Added 2 `carryover[]` entries to `state.json` and ran `mev emit-state --write` (0 errors).

## Remaining work
- **Decision point for next session:** either (a) start **BA.15.0** workspace consolidation (disruptive:
  `git subtree` the sibling repos into `crates/`), or (b) keep prototyping in-repo toward **BA.15.8
  `bastion init`** and lift into the workspace later. Brandon was asked which to start but stepped away —
  re-confirm before the repo moves.
- Before executing Phase 15: inject the Phase 15 track into `state.json` `tracks[]` (see carryover
  `bastion-product-blocks-untracked`), then `mev emit-state --write`.
- BA.15.7 needs the `toml` crate's serialize/`display` feature (currently `parse`-only in `Cargo.toml`).

## Durable State Updates
- `state.json` `carryover[]` — added `engine-fmt-red` (known_issue: repo-wide `cargo fmt --check` is red
  from pre-existing `src/engine/` code, not new work) and `bastion-product-blocks-untracked`
  (deferred: Phase 15 blocks authored but not yet in `tracks[]`).

## Open questions / choices
- **Which BA.15 entry point to start** (workspace consolidation vs. continue in-repo) — needs Brandon's call.
- Note: `planning/bastion-product/` here is the bastion-repo concept folder; the *brain* already uses a
  `bastion-product` name for the five-layer program — related but distinct. Confirm no naming confusion
  if/when this surfaces at the brain level.

## Context the next agent needs
- The full design rationale + exploration findings (CLI touch-points, mev/bella internals, template
  sources) were produced this session but live in the plan doc + this handoff, not a separate design file.
- Uncommitted changes from a *prior* in-flight effort (bella-engine dependency contract: `CLAUDE.md`
  rule #7, `D14-bella-engine-dependency-contract.md`, `decisions/index.md`, and a `sessions/ui.rs` theme
  swap to `Theme::mission_control()`) were present and left untouched — they get swept into this session's
  commit alongside the plan + okf module.

## First command after `/prime`
`cargo test okf::` (confirm the write-path prototype is green), then decide BA.15.0 vs BA.15.8.
