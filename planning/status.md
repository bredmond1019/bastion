---
type: ProjectStatus
title: Bastion Status
description: Rollup status for Bastion sub-brain.
doc_id: bastion-status
layer: [meta]
status: active
timestamp: 2026-07-03T00:09:25Z
related: [context, master-plan, planning-index]
now: "BA.15.1 (spec 15.1-extract-okf-core) done — /sdlc-flow ran all 4 tasks to PASS, review PASS, docs patched. Status: Done."
next: "Pick the next Phase 15 block (BA.15.2 — mev -> mev-core, drop its dupes for okf-core), now unblocked by this extraction, or resume Phase 13/14 blocks per focus.next. See planning/handoff.md."
blocked: []
---

# Status — Bastion

## Momentum

> Working board — keep all five queues live. **Never end a meaningful session with every queue
> empty.** The headlines of **now / next / blocked** mirror the frontmatter scalars above.

- **now** — Full spec **15.1-extract-okf-core** (BA.15.1) done. `/sdlc-flow` ran all 4 tasks to
  PASS: Task 1 scaffolded the new `okf-core` workspace crate (empty lib.rs, serde dep) and wired
  it into the root workspace `members` and as a path dependency of `crates/bastion`; Task 2 moved
  the OKF frontmatter parser (`Frontmatter`, `ParseResult`, `extract_frontmatter`,
  `parse_frontmatter`) into `okf-core` as `pub` items with their tests, repointed `brain/okf.rs`
  to call `okf_core::parse_frontmatter` directly, and made `validate/frontmatter.rs` re-export
  the parser from `okf-core` while keeping `validate_frontmatter` and its tests unchanged; Task 3
  moved `OkfFrontmatter` + `serialize_frontmatter` (with all 18 serializer tests) into `okf-core`
  self-contained with zero bastion dependency, and deleted bastion's prototype
  `crates/bastion/src/okf/` module plus its `mod okf;` registration in `main.rs`; Task 4
  confirmed full validation (fmt/clippy -D warnings/test/release build) green with 1056 total
  tests (1029 bastion + 27 okf-core), no regressions. End review verdict: PASS (0 findings).
  Docs patched: `docs/okf.md`, `docs/index.md`.
- **next** — Pick the next Phase 15 block (BA.15.2 — `mev` → `mev-core`, drop its dupes for
  `okf-core`), now unblocked by this extraction, or resume Phase 13/14 blocks per `state.json`'s
  regenerated `focus.next` ordering. See `planning/handoff.md`.
- **blocked** — nothing blocked
- **improve** — `blank_code_spans` handles single-backtick inline spans only (fenced triple-backtick blocks out of scope); confirm `bastion validate` skips `trees/` if worktrees accumulate `.md` files; `status` config-file API URL not loaded when `DATABASE_URL` absent
- **recurring** — none yet

## Metrics

> Cheap, hand-maintained signals (leading + lagging). Do **not** push these into frontmatter —
> they are multi-valued and volatile.

- tasks completed / verified this period; intervention rate; retry rate; regression rate
- reusable assets created since last milestone
- days since last eval improvement; days since last new skill/workflow
- % of runs ending with an explicit next action
