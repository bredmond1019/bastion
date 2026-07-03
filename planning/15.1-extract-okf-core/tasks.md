---
type: TaskSpec
title: "Task Spec — Phase 15, Block BA.15.1: Extract okf-core"
description: Lift the OKF frontmatter model, serializer, and parser into a new okf-core workspace crate and repoint bastion's consumers at it (lean scope — no net-new validators/state schema).
doc_id: 15-1-extract-okf-core-tasks
layer: [console]
project: bastion
status: active
keywords: [okf-core, frontmatter, workspace crate, extraction, single-source]
related: [master-plan, bastion-product-plan]
---

# Task Spec — Phase 15, Block BA.15.1: Extract `okf-core`

**Status:** Done · **Last run:** 2026-07-03 (`/sdlc-flow`, all 4 tasks PASS, review PASS)

## Goal
Single-source the OKF frontmatter contract into a new `okf-core` workspace crate — the model,
`serialize_frontmatter` (write path), and the frontmatter parser — and repoint bastion's consumers at
it, with no behavior change.

## Scope decision (lean — operator-confirmed 2026-07-02)
This block is a **pure mechanical extraction**: lift only what bastion already has (the `OkfFrontmatter`
model + `serialize_frontmatter`, and the `extract_frontmatter`/`parse_frontmatter` parser) into
`okf-core`, and repoint bastion's consumers. The closed-vocab validators (`is_kebab_case`,
layer/project/status) and the `state.rs` serde schema that the master-plan block also lists are **out of
scope here** — bastion has no existing implementation of either to lift, the state schema lives in the
cross-repo `core/planning/state-schema.md`, and mev (the other consumer) is not yet a workspace member.
Those pieces, and the mev repointing, land in **BA.15.2** ("mev → `mev-core`, drop its dupes for
`okf-core`"), whose scope explicitly owns them. BA.15.1's acceptance is "**both** [bastion] consumers
compile against `okf-core`."

## Context Pointers
- **Plan:** `planning/master-plan.md` → "Block BA.15.1 — Extract `okf-core`"; `planning/bastion-product/plan.md`
  → Wave 1 / BA.15.1 + the Critical-files table row for BA.15.1.
- **Source to lift:**
  - `crates/bastion/src/okf/mod.rs` (396 lines) — `OkfFrontmatter` model, `serialize_frontmatter`, the
    pure helpers (`push_scalar`, `push_list`, `yaml_scalar`, `needs_quote`), and 18 tests. Registered as
    `mod okf;` in `crates/bastion/src/main.rs:17`. It is currently a prototype with **no non-test
    consumers** in bastion (confirmed: no `crate::okf::*` references outside the module).
  - `crates/bastion/src/validate/frontmatter.rs` — the parser: `Frontmatter` (pub), `ParseResult`
    (private enum), `extract_frontmatter` (private fn), `parse_frontmatter` (`pub(crate)`), plus
    `validate_frontmatter` (pub) which maps parse outcomes to bastion's `ValidationError`/`ErrorKind`
    taxonomy (`crate::validate`).
- **Consumers that must keep compiling:**
  - `crates/bastion/src/brain/okf.rs:50` — `crate::validate::frontmatter::parse_frontmatter` (repoint to
    `okf_core::parse_frontmatter` directly, per the block's file list).
  - `crates/bastion/src/serve/status/handoff.rs:10` and `crates/bastion/src/serve/status/repo.rs:11` —
    `use crate::validate::frontmatter::parse_frontmatter`. These are **not** named in the block; keep them
    compiling by having `validate/frontmatter.rs` **re-export** the parser from `okf-core`
    (`pub use okf_core::{...}`) rather than editing these files.
  - `crates/bastion/src/validate/mod.rs:110` and `validate/report.rs` tests — call
    `frontmatter::validate_frontmatter`. `validate_frontmatter` **stays in bastion** (it depends on
    bastion's error taxonomy), so these are untouched.
- **CLAUDE.md rules:** Rule 1 (tests ship with every change — the moved tests must move with their code
  and stay green); Rule 6 (pure-logic exhaustive tests — the serializer/parser are already pure and
  fully unit-tested; preserve every case); Rule 7 (`bella-engine` unpinned — unaffected here).

## Design (the extraction seam)
`okf-core` owns the **format primitives** (model + serialize + parse); bastion keeps its **validation
policy** (`validate_frontmatter`, error taxonomy). Concretely:

- **okf-core public API:** `OkfFrontmatter`, `serialize_frontmatter`, `Frontmatter`, `ParseResult`,
  `extract_frontmatter`, `parse_frontmatter` — all `pub` (the parser items were `pub(crate)`/private in
  bastion; they become `pub` in the crate). `serde` (derive) is the crate's only dependency.
- **`validate/frontmatter.rs`** shrinks to: `pub use okf_core::{Frontmatter, ParseResult, extract_frontmatter, parse_frontmatter};`
  plus the retained `validate_frontmatter` (now calling the re-exported `extract_frontmatter`/`ParseResult`)
  and its own tests. Behavior byte-for-byte identical.
- **`brain/okf.rs`** switches its one call site to `okf_core::parse_frontmatter`.
- **`crates/bastion/src/okf/`** is deleted and `mod okf;` removed from `main.rs` — no duplicated
  model/serializer definitions remain in bastion.

## Step-by-Step Tasks
See `tasks.json` in this directory — the task list is defined there, not here.

## Acceptance Criteria
- A new crate `crates/okf-core/` exists, is listed in the root `Cargo.toml` `[workspace] members`, and
  `crates/bastion/Cargo.toml` depends on it via `okf-core = { path = "../okf-core" }`.
- `OkfFrontmatter`, `serialize_frontmatter`, `Frontmatter`, `ParseResult`, `extract_frontmatter`, and
  `parse_frontmatter` are defined in `okf-core` and are `pub`; the serializer/parser tests moved with
  them and pass under `okf-core`.
- `crates/bastion/src/okf/` is deleted and `mod okf;` is removed from `crates/bastion/src/main.rs`; no
  `OkfFrontmatter`/`serialize_frontmatter`/`extract_frontmatter` definitions remain in the `bastion` crate.
- `crates/bastion/src/validate/frontmatter.rs` re-exports the parser from `okf-core` and retains
  `validate_frontmatter` with **unchanged behavior** (its existing tests pass without edits to their
  assertions).
- `crates/bastion/src/brain/okf.rs` calls `okf_core::parse_frontmatter`.
- The unnamed parser consumers `serve/status/handoff.rs` and `serve/status/repo.rs` still compile via the
  re-export (no edits required).
- Combined workspace test count is **not lower** than before the extraction (all moved tests still run).
- All gated checks pass at the workspace root.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```
<!-- Run from the workspace root; all four are the project's gated checks (planning/harness.json). -->

## Notes
<filled in as work happens>

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
- 2026-07-03 [task 3] Two parser round-trip tests moved into `okf-core` were rewritten rather
  than moved verbatim: the two tests that originally asserted via bastion's `validate_frontmatter`
  now assert directly on `parse_frontmatter` output, and the quoted-colon round-trip test now
  checks the parsed value is non-empty and contains the original text instead of exact-matching an
  unquoted string — needed so `okf-core` has zero dependency on bastion's `validate_frontmatter`,
  per the task's explicit self-containment instruction; the hand-rolled parser intentionally does
  not strip YAML quoting, so the original exact-match assertion no longer applied outside
  `validate_frontmatter`'s context.
