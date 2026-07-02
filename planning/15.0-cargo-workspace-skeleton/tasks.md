---
type: TaskSpec
title: "Task Spec — Phase 15, Block BA.15.0 (Cargo workspace skeleton)"
description: Decompose BA.15.0 — introduce a root Cargo workspace and relocate the bastion crate under crates/bastion/ without changing behavior.
doc_id: 15.0-cargo-workspace-skeleton
layer: [engine, meta]
project: bastion
status: draft
keywords: [cargo-workspace, crate-layout, packaging, bastion-product, phase-15]
related: [bastion-product-plan, master-plan]
---

# Task Spec — Phase 15, Block BA.15.0 (Cargo workspace skeleton)

**Status:** Done · **Last run:** 2026-07-02 (/sdlc-flow, PASS)

## Goal
Introduce a root `[workspace]` and move today's `bastion` sources under `crates/bastion/` — building the
physical container the later Phase 15 convergence crates (`okf-core`, `mev-core`, templates) will live in,
with the existing binary and test suite unchanged and green.

## Context Pointers
- **Block definition:** `planning/master-plan.md` → *Phase 15 → Block BA.15.0* (the authoritative What /
  Files / Out-of-scope / Acceptance criteria).
- **Executable detail:** `planning/bastion-product/plan.md` → *Wave 1 → BA.15.0*, the *Critical files*
  table (`root Cargo.toml; move src/ → crates/bastion/src/; member Cargo.toml`s`), and *Verification* step 1.
- **Repo rules:** `CLAUDE.md` standing rules — rule 1 (every behavior change ships tests; here the bar is
  *no test regression* since this block moves code without changing behavior), rule 6 (coverage bar), and
  rule 7 (`bella-engine` is an unpinned cross-repo path dep — keep it a path dep, do **not** add
  `default-features = false`).
- **Current layout:** package manifest is root `Cargo.toml` (edition 2024); sources in `src/`; four path
  deps to sibling repos — `bella-engine` (`../bella/...`) and three `workflow-engine-*`
  (`../../portfolio/workflow-engine-rs/...`). Relocating the manifest to `crates/bastion/Cargo.toml` adds
  two directory levels, so each path dep gains two `../` segments.
- **Move is include-safe:** `include_str!`/`include_bytes!` are resolved relative to the source file, and
  the `CARGO_MANIFEST_DIR`-relative fixture joins (`src/validate/report.rs`, `src/brain/code_graph.rs`)
  stay valid because the fixtures move with the tree and the manifest dir becomes `crates/bastion`.

## Step-by-Step Tasks
See `tasks.json` in this directory — the task list is defined there, not here.

## Acceptance Criteria
- Root `Cargo.toml` is a **virtual workspace manifest** (`[workspace]` with `members = ["crates/bastion"]`
  and an edition-2024-compatible `resolver`); it no longer declares the `bastion` `[package]`.
- The bastion package manifest lives at `crates/bastion/Cargo.toml` and its sources at
  `crates/bastion/src/` (moved with `git mv`, preserving history); no `src/` tree remains at repo root.
- The four sibling path dependencies (`bella-engine`, `workflow-engine-core`/`-mcp`/`-nodes`) are repointed
  to the new relative depth and still resolve; `bella-engine` remains a path dep **without**
  `default-features = false` (rule 7).
- `cargo build --release` builds the workspace; the `bastion` binary and the **full existing test suite**
  pass unchanged (same tests, all green) — no behavior moved into new crates (out of scope).
- In-repo layout references (`CLAUDE.md` directory map, and `AGENT.md`/`GEMINI.md` if they mirror it) point
  at `crates/bastion/src/…` rather than the old `src/…`.
- All gated checks pass.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
cargo run -- --help
```
<!-- Standard project checks from planning/harness.json (validation.checks[]); `cargo run -- --help`
     added as a spec-specific smoke test that the relocated binary still dispatches. -->

## Notes
<filled in as work happens>

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
- 2026-07-02 [task 1] SDLC worktrees live one directory level deeper than a standard checkout
  (`core/bastion/trees/<spec>/...`), which breaks any relative path dep reaching outside
  `core/bastion/`. Rather than deepening the committed path-dep strings (which would break
  resolution from a standard non-worktree checkout after merge), the fix was an untracked local
  symlink outside the worktree (`core/bastion/portfolio -> agentic-portfolio/portfolio`),
  mirroring the existing `trees/bella -> ../../bella` pattern already used for `bella-engine`.
  The committed `Cargo.toml` path depths are unchanged from the spec's Context Pointers.
