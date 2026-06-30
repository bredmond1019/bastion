---
type: TaskSpec
title: "Task Spec — BA.11.C0: Agent-state detection manifest engine"
description: "A pure, config-driven agent-state detection engine reimplemented clean-room from Herdr's detect pattern: per-agent TOML manifests (region selector + contains/regex/line_regex matchers + any/all/not gates + priority + visible_* flags) compile into rules; detect(screen, manifest) evaluates them in priority order. Seeded with Claude + Pi manifests only."
doc_id: 11-c0-agent-state-detection
layer: [console, surface]
project: bastion
status: active
keywords: [agent-state detection, manifest engine, TOML, gate matcher, region selector, agent-agnostic, detect]
related: [master-plan, serve-api, sessions]
phase: 11
block: C0
---

# Task Spec — BA.11.C0: Agent-state detection manifest engine

**Status:** PASSED (3/3 tasks) · **Last run:** 2026-06-30 UTC

## Goal
Build a pure, config-driven agent-state detection engine — per-agent TOML manifests compile into priority-ordered rules and a `detect(screen, manifest) -> AgentDetection` matcher classifies a captured pane region as `Idle | Working | Blocked | Unknown` with `visible_*` / `skip_state_update` flags — seeded with Claude and Pi manifests only, so adding any future agent is a new TOML and not new Rust.

## Context Pointers
- **Plan:** `planning/master-plan.md` → *Phase 11 — BastionUI Console API* → *BA.11.C0 — Agent-state detection manifest engine (prework for Block C; agent-agnostic seam)*. This block **gates BA.11.C**: Block C's needs-input detector consumes `detect::detect()` (`Blocked && visible_blocker` → `needs_input`) instead of inline Claude-coupled heuristics.
- **Licensing constraint (hard):** Reimplement **clean-room** from Herdr's `src/detect/` *pattern* — Herdr is **AGPL-3.0, reference only** (master-plan decision D-x). Do **not** copy Herdr source; mirror the design (manifest schema → compiled rules → priority matcher) in original Rust.
- **Existing substrate (verified):**
  - **Modules are declared in `src/main.rs`** (alphabetical list, lines 5–18: `api`/`brain`/`cli`/`config`/`costs`/`db`/`inspect`/…). The new module slots in as `mod detect;` between `mod db;` and `mod inspect;`.
  - `src/sessions/claude_state.rs` is a workspace-**trust** observer only — it is **not** agent-state detection. This block fills a genuine gap: bastion has no agent-state detection today.
  - `src/sessions/model.rs` — `Pane::last_lines(Option<usize>)` is the kind of captured-pane text this engine consumes (a `&str` screen). No coupling required; `detect()` takes a plain `&str`.
  - `src/serve/dto.rs` / the `tmux.rs` construction-vs-execution split are the house **pure-logic-without-I/O** template (CLAUDE.md Rule 6).
- **Dependencies:** `toml` (`{ version = "0.8", default-features = false, features = ["parse"] }`) and `serde` are **direct** deps already. **`regex` is NOT a direct dependency** — it appears in `Cargo.lock` only transitively. Task 1 must add `regex` to `[dependencies]` in `Cargo.toml` (the master-plan's "no new deps" note is inaccurate on this point). No other new deps.
- **Standing rules (`CLAUDE.md`):** Rule 1 (tests ship with every change), Rule 2 (OKF frontmatter on new `.md` under `docs/`/`planning/` — this spec carries it; no new doc files added here), Rule 6 (coverage bar — pure logic exhaustively unit-tested **without I/O**, error/degradation paths covered, not just happy paths). This whole engine is pure: load manifests/fixtures via `include_str!` so tests touch no filesystem.

## Step-by-Step Tasks

### 1. Detection engine — core types, manifest schema, gate matcher, region resolver, `detect()`
- **Add `regex` as a direct dependency** in `Cargo.toml` `[dependencies]` (it is currently only transitive in `Cargo.lock`). Pin to the version already resolved in the lockfile.
- Register the module: add `mod detect;` to `src/main.rs` (alphabetical, between `mod db;` and `mod inspect;`).
- Create `src/detect/mod.rs` — the public API and core types:
  - `AgentState` enum: `Idle | Working | Blocked | Unknown` (derive `Debug, Clone, Copy, PartialEq, Eq`; `serde::Serialize` for downstream serve frames).
  - `AgentDetection { state: AgentState, visible_idle: bool, visible_blocker: bool, visible_working: bool, skip_state_update: bool }` (derive `Debug, Clone, PartialEq, Eq`).
  - `pub fn detect(screen: &str, manifest: &CompiledManifest) -> AgentDetection` — resolve the rule's `region` over `screen`, evaluate compiled rules in **descending priority** order, return the first matching rule's `AgentDetection`; on no match return `AgentState::Unknown` with all flags `false`.
  - Pre-declare the golden-test submodule slot so Task 2 owns only its own file: `#[cfg(test)] mod golden_tests;`.
- Create `src/detect/manifest.rs` — schema, compile, region resolver, gate matcher:
  - `Manifest` (deserialized from TOML): an agent `name` + a list of `Rule`s. `Rule` carries a `region` selector, a `gate` (the matcher tree), a `priority: i32`, and the outcome flags `state` + `visible_idle`/`visible_blocker`/`visible_working`/`skip_state_update`.
  - **Region selector** (`region`): implement the variants the seed manifests need — at minimum `whole` (entire screen) and `last_lines = N` (the final N lines). Add others only if a seed manifest requires them. A pure `resolve_region(screen, &Region) -> String`.
  - **Matchers** (leaf predicates): `contains = "…"` (substring), `regex = "…"` (regex over the resolved region), `line_regex = "…"` (regex tested per line, true if any line matches). Regexes are **compiled once** at manifest-compile time.
  - **Gate combinators** (recursive): `any` (OR), `all` (AND), `not` (negation) over child gates/matchers; a recursive `eval_gate(&Gate, region: &str) -> bool`.
  - `CompiledManifest` + `Manifest::compile() -> Result<CompiledManifest, …>` that precompiles every regex and surfaces a typed error on a bad pattern or malformed schema.
- **Tests (exhaustive, pure — Rule 6), in `manifest.rs`/`mod.rs` `#[cfg(test)]`:**
  - each matcher type (`contains`, `regex`, `line_regex`) — positive and negative;
  - `any`/`all`/`not` combinators including a **nested** gate;
  - each region selector (`whole`, `last_lines`) over a multi-line fixture string;
  - priority ordering (higher-priority rule wins when two match);
  - no-rule-matches → `Unknown` + all flags false;
  - `skip_state_update` flag is carried through;
  - `compile()` **error path** — a malformed regex / malformed TOML returns a typed error (degradation path, not just happy path).
- **Owns:** `src/detect/mod.rs`, `src/detect/manifest.rs` (new), `src/main.rs` (append-only one-line `mod detect;`), `Cargo.toml` (append-only `regex` dep). **No dependencies.**

### 2. PASSED Seed Claude + Pi manifests, captured-pane fixtures, golden tests
- Author `src/detect/manifests/claude.toml`:
  - a **`Blocked` + `visible_blocker`** rule matching Claude's prompt/approval box (e.g. the bordered input box / "Do you want to proceed?" affordance) at high priority;
  - a `Working` rule for Claude's active/spinner state ("esc to interrupt" / working indicator);
  - an `Idle` rule for the resting prompt.
- Author `src/detect/manifests/pi.toml`:
  - a **`Working`** rule matching Pi's `Working...` indicator;
  - idle/blocked rules as the fixtures require.
- Create fixtures under `src/detect/fixtures/` (captured pane snippets, plain text): at minimum `claude_blocked.txt` (a real prompt-box capture) and `pi_working.txt` (a `Working...` capture); add idle fixtures as needed for the manifests' other rules.
- Create `src/detect/golden_tests.rs` (the slot pre-declared in Task 1): load each manifest and fixture via `include_str!` (**no filesystem I/O**), `compile()`, run `detect()`, and assert:
  - `claude.toml` over `claude_blocked.txt` → `state == Blocked` **and** `visible_blocker == true`;
  - `pi.toml` over `pi_working.txt` → `state == Working`;
  - one round-trip proving a **new agent manifest is added with zero engine-code change** (this task adds only TOML + fixtures + this test file — no edits to `mod.rs`/`manifest.rs` logic; call that out in a test comment).
- **Owns:** `src/detect/manifests/claude.toml`, `src/detect/manifests/pi.toml`, `src/detect/fixtures/*`, `src/detect/golden_tests.rs` (all new). **Depends on:** Task 1.

### 3. PASSED Validate
- Run the Validation Commands listed below and confirm all pass.
- This block is pure (no process/Postgres/HTTP I/O), so there is no separate runtime shell to smoke-test — the golden tests in Task 2 exercise the full `detect()` path over real captured fixtures. Record the final test count in `## Notes`.

## Acceptance Criteria
- `src/detect/` exposes `detect(screen: &str, manifest: &CompiledManifest) -> AgentDetection` returning `AgentState` ∈ {`Idle`,`Working`,`Blocked`,`Unknown`} plus `visible_idle`/`visible_blocker`/`visible_working`/`skip_state_update`.
- A captured Claude prompt-box fixture run through `claude.toml` yields `Blocked` + `visible_blocker == true` (golden test).
- A Pi `Working...` fixture run through `pi.toml` yields `Working` (golden test).
- The `any`/`all`/`not` gate combinators (including a nested gate) and each `region` selector are unit-tested directly.
- `contains` / `regex` / `line_regex` matchers each have positive and negative unit tests.
- Priority ordering, no-match → `Unknown`, `skip_state_update` carry-through, and the `compile()` error path (malformed regex/TOML → typed error) are all unit-tested.
- Adding the Pi manifest required **no change to engine code** (`mod.rs`/`manifest.rs`) — only a new TOML + fixture + test (demonstrated by Task 2's file ownership and asserted in a golden test).
- `regex` is a direct dependency in `Cargo.toml`; `mod detect;` is registered in `src/main.rs`.
- All gated checks pass (`cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`).

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
Task 3 validation run — 2026-06-30. All four gated checks pass:
- `cargo fmt --check` — clean
- `cargo clippy -- -D warnings` — clean
- `cargo test` — 812 total tests pass (37 in `detect::` module: 30 unit tests in manifest.rs/mod.rs, 7 golden tests in golden_tests.rs)
- `cargo build --release` — clean

This block is pure (no process/Postgres/HTTP I/O); the golden tests in Task 2 fully exercise the `detect()` path over real captured fixtures — no separate runtime smoke-test required.

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
- 2026-06-30 [task 2] Added a cross-agent isolation golden test (claude_blocked through pi manifest → Unknown) not required by the spec; validates the extensibility/non-bleed claim asserted in the acceptance criteria and called out in a test comment.
