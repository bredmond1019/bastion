---
type: TaskBreakdown
title: "Breakdown — BA.11.C0 Task 1: detection engine core"
description: "Atomic sub-step decomposition of Task 1 (core types, manifest schema, gate matcher, region resolver, detect()) from the BA.11.C0 agent-state detection spec."
doc_id: 11-c0-breakdown
layer: [console, surface]
project: bastion
status: active
keywords: [breakdown, agent-state detection, manifest engine, gate matcher, region selector, detect, regex]
related: [11-c0-agent-state-detection, master-plan]
phase: 11
block: C0
---

# Task Breakdown — BA.11.C0: Agent-state detection manifest engine

## Source Spec
`planning/11.C0-agent-state-detection/tasks.md`

## Scope of this breakdown
This decomposes **Task 1 only** ("Detection engine — core types, manifest schema, gate matcher, region resolver, `detect()`"). Spec Task 2 (seed manifests + fixtures + golden tests) and Task 3 (Validate) are unchanged and run after Step 1 here completes.

## Goal
Build a pure, config-driven agent-state detection engine — per-agent TOML manifests compile into priority-ordered rules and a `detect(screen, manifest) -> AgentDetection` matcher classifies a captured pane region as `Idle | Working | Blocked | Unknown` with `visible_*` / `skip_state_update` flags — seeded with Claude and Pi manifests only, so adding any future agent is a new TOML and not new Rust.

## How to Use
Work top to bottom. Each sub-step is a single atomic action. Run the inline **Verify** checks as you go — do not batch them at the end. Each check must pass before continuing. The tree may not compile until sub-step 1.5 lands (`mod.rs` and `manifest.rs` reference each other); the first compile Verify is at the end of 1.5.

---

## Steps

### Step 1: Detection engine — core types, manifest schema, gate matcher, region resolver, `detect()`

#### 1.1 Add `regex` as a direct dependency
**File:** `Cargo.toml`
**Action:** edit line 27 region of `[dependencies]` — add, alphabetically near the other crates (e.g. after the `reqwest` line, line 19, or grouped with `serde`):
```toml
regex = "1.12.4"
```
Pin to `1.12.4` — the version already resolved in `Cargo.lock` (so no lockfile churn). `regex` is currently only a transitive dependency; this promotes it to direct.

**Verify:** `cargo tree -i regex --depth 0` → shows `regex v1.12.4` as a direct dep (or `grep '^regex' Cargo.toml` returns the line).

#### 1.2 Register the module
**File:** `src/main.rs`
**Action:** add a single line `mod detect;` to the alphabetical module block (lines 5–18), between `mod db;` (line 10) and `mod inspect;` (line 11):
```rust
mod db;
mod detect;
mod inspect;
```
This is an append-only one-line edit; no other code in `main.rs` changes.

#### 1.3 Create the engine public API + core types
**File:** `src/detect/mod.rs`
**Action:** create the file. Mirror the house pattern in `src/sessions/model.rs` (doc comments, `#[derive(Debug, Clone, PartialEq)]` enums, `as_str()` matchers, pure functions, inline `#[cfg(test)] mod tests`).

Contents:
```rust
// detect/mod.rs — pure, config-driven agent-state detection.
//
// Per-agent TOML manifests (see manifest.rs) compile into priority-ordered rules;
// `detect()` resolves each rule's screen region and evaluates its gate, returning
// the first matching rule's outcome. Clean-room reimplementation of the Herdr
// detect *pattern* (Herdr is AGPL-3.0 — reference only, no copied source).

pub mod manifest;

#[cfg(test)]
mod golden_tests; // slot owned by spec Task 2 (manifests + fixtures + golden tests)

use manifest::{CompiledManifest, resolve_region};
use serde::{Deserialize, Serialize};

/// Classified state of an agent session from its captured pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Idle,
    Working,
    Blocked,
    Unknown,
}

impl AgentState {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentState::Idle => "idle",
            AgentState::Working => "working",
            AgentState::Blocked => "blocked",
            AgentState::Unknown => "unknown",
        }
    }
}

/// Full detection outcome: the state plus the visibility/skip flags carried by
/// the matching rule. On no match: `Unknown` with every flag `false`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDetection {
    pub state: AgentState,
    pub visible_idle: bool,
    pub visible_blocker: bool,
    pub visible_working: bool,
    pub skip_state_update: bool,
}

impl AgentDetection {
    /// The no-match default.
    pub fn unknown() -> Self {
        Self {
            state: AgentState::Unknown,
            visible_idle: false,
            visible_blocker: false,
            visible_working: false,
            skip_state_update: false,
        }
    }
}

/// Evaluate `manifest`'s rules (already sorted descending by priority at compile
/// time) against `screen`. Returns the first matching rule's `AgentDetection`,
/// or `AgentDetection::unknown()` when no rule matches.
pub fn detect(screen: &str, manifest: &CompiledManifest) -> AgentDetection {
    for rule in &manifest.rules {
        let region = resolve_region(screen, &rule.region);
        if rule.gate.eval(&region) {
            return AgentDetection {
                state: rule.state,
                visible_idle: rule.visible_idle,
                visible_blocker: rule.visible_blocker,
                visible_working: rule.visible_working,
                skip_state_update: rule.skip_state_update,
            };
        }
    }
    AgentDetection::unknown()
}
```
> If `golden_tests` does not yet exist when you first compile, either land sub-step 1.3 with the `#[cfg(test)] mod golden_tests;` line commented out and uncomment it in spec Task 2, **or** create an empty `src/detect/golden_tests.rs` placeholder now (Task 2 fills it). Prefer the empty-placeholder approach so the slot is real and disjoint ownership holds.

#### 1.4 Create the manifest schema, compile, region resolver, and gate matcher
**File:** `src/detect/manifest.rs`
**Action:** create the file with the deserializable schema, the compiled forms, the typed error, the region resolver, and the recursive gate matcher.

Schema (serde `Deserialize`, externally tagged enums map cleanly onto TOML inline tables):
```rust
use crate::detect::AgentState;
use regex::Regex;
use serde::Deserialize;

/// Which slice of the screen a rule inspects. Defaults to `Whole`.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RegionSpec {
    #[default]
    Whole,
    LastLines { n: usize },
}

/// A gate is a matcher leaf or a boolean combinator over child gates.
/// Externally tagged: `{ contains = "x" }`, `{ all = [..] }`, `{ not = {..} }`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateSpec {
    Contains(String),
    Regex(String),
    LineRegex(String),
    Any(Vec<GateSpec>),
    All(Vec<GateSpec>),
    Not(Box<GateSpec>),
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuleSpec {
    #[serde(default)]
    pub region: RegionSpec,
    pub gate: GateSpec,
    #[serde(default)]
    pub priority: i32,
    pub state: AgentState,
    #[serde(default)]
    pub visible_idle: bool,
    #[serde(default)]
    pub visible_blocker: bool,
    #[serde(default)]
    pub visible_working: bool,
    #[serde(default)]
    pub skip_state_update: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub name: String,
    #[serde(default)]
    pub rules: Vec<RuleSpec>,
}
```

Typed error (use `thiserror`, already a dep):
```rust
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("manifest TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("invalid regex in manifest: {0}")]
    Regex(#[from] regex::Error),
}
```

Compiled forms (regexes precompiled; rules sorted descending by priority):
```rust
pub struct CompiledManifest {
    pub name: String,
    pub rules: Vec<CompiledRule>,
}

pub struct CompiledRule {
    pub region: RegionSpec,
    pub gate: CompiledGate,
    pub state: AgentState,
    pub visible_idle: bool,
    pub visible_blocker: bool,
    pub visible_working: bool,
    pub skip_state_update: bool,
}

pub enum CompiledGate {
    Contains(String),
    Regex(Regex),
    LineRegex(Regex),
    Any(Vec<CompiledGate>),
    All(Vec<CompiledGate>),
    Not(Box<CompiledGate>),
}
```

Functions:
- `pub fn parse_manifest(toml_src: &str) -> Result<Manifest, ManifestError>` — `toml::from_str`.
- `impl Manifest { pub fn compile(self) -> Result<CompiledManifest, ManifestError> }` — compile each gate, build `CompiledRule`s, then **`sort_by(|a, b| b.priority.cmp(&a.priority))`** (descending; stable sort preserves source order on ties).
- `fn compile_gate(g: &GateSpec) -> Result<CompiledGate, ManifestError>` — recursive; `Regex::new` for `Regex`/`LineRegex` (propagates `regex::Error`).
- `pub fn resolve_region(screen: &str, region: &RegionSpec) -> String`:
  - `Whole` → `screen.to_string()`
  - `LastLines { n }` → the last `n` lines joined with `\n` (if the screen has ≤ `n` lines, return the whole screen).
- `impl CompiledGate { pub fn eval(&self, region: &str) -> bool }` — recursive:
  - `Contains(s)` → `region.contains(s)`
  - `Regex(re)` → `re.is_match(region)`
  - `LineRegex(re)` → `region.lines().any(|l| re.is_match(l))`
  - `Any(v)` → `v.iter().any(|g| g.eval(region))`
  - `All(v)` → `v.iter().all(|g| g.eval(region))`
  - `Not(g)` → `!g.eval(region)`

#### 1.5 Add exhaustive unit tests for the engine (pure, no I/O)
**File:** `src/detect/manifest.rs` (append `#[cfg(test)] mod tests { … }`) — and `src/detect/mod.rs` for `detect()`-level + `AgentState::as_str` tests.
**Action:** cover every leaf, combinator, region, and degradation path. Build manifests from inline TOML strings (no filesystem). Suggested cases:

In `manifest.rs` `mod tests`:
- `contains_matches` / `contains_no_match` — `CompiledGate::Contains` positive + negative.
- `regex_matches` / `regex_no_match` — `Regex` leaf over a region.
- `line_regex_matches_one_line` — multi-line region where only one line matches; `line_regex_no_line_matches` — none match.
- `any_true_when_one_child_true`, `all_false_when_one_child_false`, `not_negates`.
- `nested_gate` — e.g. `All([ Contains, Not(Contains), Any([Regex, LineRegex]) ])` evaluated to a known boolean.
- `region_whole_returns_full_screen`; `region_last_lines_returns_tail` (screen of 5 lines, `n = 2` → last two joined); `region_last_lines_fewer_lines_than_n` (screen of 1 line, `n = 5` → whole screen).
- `compile_sorts_rules_descending_priority` — two rules priority 1 and 10 in source order 1-then-10; assert compiled order is 10-then-1.
- `compile_bad_regex_is_error` — gate `{ regex = "(" }` → `Manifest::compile()` returns `Err(ManifestError::Regex(_))`.
- `parse_malformed_toml_is_error` — invalid TOML → `Err(ManifestError::Toml(_))`.
- `parse_missing_required_state` — a rule TOML with no `state` field → parse/compile error (state is required).

In `mod.rs` `mod tests`:
- `as_str_roundtrip` — each `AgentState` variant maps to its string.
- `detect_returns_first_matching_rule_by_priority` — a 2-rule inline manifest where a lower-priority rule would also match; assert the higher-priority rule's `AgentDetection` (state + flags) is returned.
- `detect_no_match_returns_unknown` — manifest whose single rule cannot match the screen → `AgentDetection::unknown()`.
- `detect_carries_skip_state_update_flag` — matching rule with `skip_state_update = true` → flag set in the result.

**Verify:** `cargo test detect::` → all new tests pass (`0 failed`).

**Verify (group):**
```
cargo fmt --check && cargo clippy -- -D warnings && cargo build --release
```
→ exit 0, no warnings.

---

## Acceptance Criteria
<!-- Copied verbatim from the spec — Task 1 satisfies the engine-level subset; the
     fixture/golden criteria are satisfied by spec Task 2. -->
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
- **Resolved `regex` version is `1.12.4`** (from `Cargo.lock`) — pin to it in `Cargo.toml` to avoid lockfile churn.
- **Modules are declared in `src/main.rs`** (not a `lib.rs` — there is none). `mod detect;` goes in the alphabetical block between `db` and `inspect`.
- **House pattern** (from `src/sessions/model.rs`): enums derive `Debug, Clone, PartialEq` with an `as_str()` matcher; pure classification fns; inline `#[cfg(test)] mod tests`. `AgentState` additionally needs `Copy` + `Serialize`/`Deserialize` (serde is a direct dep) because it is both deserialized from manifest TOML and serialized into future serve frames.
- **TOML ↔ enum mapping:** `GateSpec` is externally tagged so a TOML inline table `{ all = [ { contains = "x" } ] }` deserializes directly; `RegionSpec` is internally tagged (`{ kind = "last_lines", n = 5 }`) with `Whole` as the `#[default]`. Confirm these serde shapes against the actual `claude.toml`/`pi.toml` authored in spec Task 2 — if a manifest needs a different region selector, add the variant here, not inline in the matcher.
- **Disjoint ownership within Task 1:** all four touched files are either new (`mod.rs`, `manifest.rs`) or append-only one-line edits (`main.rs`, `Cargo.toml`); no overlap with spec Task 2, which owns only `manifests/*.toml`, `fixtures/*`, and `golden_tests.rs`. The `#[cfg(test)] mod golden_tests;` declaration in `mod.rs` is the single coordination point — create the empty `golden_tests.rs` placeholder in Task 1 so the slot is real and Task 2 only fills it (no `mod.rs` edit in Task 2).
- **Purity / Rule 6:** the whole engine is pure (`&str` in, value out). Tests build manifests from inline TOML strings and screens from inline `&str` — no filesystem, no process spawn. Task 2's golden tests load real manifests/fixtures via `include_str!`, which is still compile-time, not runtime I/O.
