---
type: TaskBreakdown
title: "Task Breakdown — Phase 15, Block BA.15.12 (mev/okf-core format convergence — okf-core side)"
description: Atomic, agent-executable sub-steps for extracting the state.json serde schema + block-dependency graph, the reconciled OkfFrontmatter, and the graph/edge-resolution model into okf-core.
doc_id: 15-12-mev-okf-core-convergence-breakdown
layer: [console, factory]
project: bastion
status: active
keywords: [okf-core, BA.15.12, state-schema, graph-resolution, breakdown]
related: [15-12-mev-okf-core-convergence-tasks, D15-mev-integration-cross-repo-path-dep, D16-ba15-12-scope-widened-graph-resolution]
---

# Task Breakdown — mev/okf-core format convergence (okf-core side)

## Source Spec
`planning/15.12-mev-okf-core-convergence/tasks.md`

## Goal
Extract into `okf-core` the three shared-format models mev still owns privately — a `state.json`
serde schema + block-dependency graph, a reconciled `OkfFrontmatter`, and a graph/edge-resolution
model — so mev's `brain/okf.rs`/`state.rs`/`graph.rs`/`graph_emit.rs` can later repoint at `okf-core`
and delete their duplicates.

## How to Use
Work top to bottom. Each sub-step is a single atomic action. Run the inline **Verify** checks as you
go — do not batch them at the end. Each check must pass before continuing.

**Extraction boundary (applies throughout):** move only the *pure serde/model + resolution
primitives*. mev's `build_graph`/`check_graph` (need `Corpus`) and `discover_state_files`/`check_*`/
`derive_*` (need `BrainConfig`/`Diagnostic`) **stay in mev** and are out of scope here. The mev source
files under `../mev/src/brain/` are **read-only reference** — do not edit them in this spec (that is
the separate downstream mev-side ticket). `okf-core` must not gain a dependency on `bastion`, `mev`,
`Corpus`, `BrainConfig`, or `Diagnostic`.

---

## Steps

### Step 1: okf-core — state.json serde schema + block-dependency graph model

#### 1.1 Add `serde_json` + `thiserror` to okf-core's manifest
**File:** `crates/okf-core/Cargo.toml`
**Action:** edit the `[dependencies]` table. Today it holds only
`serde = { version = "1", features = ["derive"] }`. Add:
```toml
serde_json = "1"
thiserror = "2"
```
(`serde_json` powers `load_state` deserialization; `thiserror` powers `StateLoadError`. Do **not**
add `serde_yaml` or `petgraph` — the state model is JSON and the graph builder is plain `Vec`-based.)

#### 1.2 Create `crates/okf-core/src/state.rs` — serde schema + loader
**File:** `crates/okf-core/src/state.rs`
**Action:** create the module. Port these items **verbatim in shape** from
`../mev/src/brain/state.rs` (reference only), preserving every `#[serde(...)]` attribute
(`rename`, `alias`, `default`, `tag`, `rename_all`, `skip`) exactly — byte-faithful serde is the
contract with `planning/state-schema.md`:

- `StateLoadError` (`../mev/src/brain/state.rs:41`) — `thiserror`-derived enum, variants `Io { path, source }` and `Parse { path, source }`. Use `std::io::Error` / `serde_json::Error` as the `#[source]` types.
- `BlockedBy` (`:69`) — `#[serde(tag = "type", rename_all = "snake_case")]` enum, variants `Block { repo, id, what? }` and `External { what }`.
- `Block` (`:99`) — fields `id` (`#[serde(alias = "block")]`), `title`, `status?`, `note?`, `repo?`, `blocked_by: Vec<BlockedBy>`.
- `Focus` (`:126`) — `now`/`next`/`blocked: Vec<Block>`, all `#[serde(default)]`; derive `Default`.
- `TrackBlock` (`:144`) — `id`, `title`, `status?`, `depends_on: Vec<BlockedBy>`, `wave: Option<i64>`, `origin: Option<Origin>`.
- `Track` (`:166`) — `title`, `blocks: Vec<TrackBlock>`.
- `RepoRollup` (`:180`) — `repo`, `tier?`, `now`/`next`/`blocked: Vec<Block>`.
- `Endpoint` (`:203`) — `repo`, `id` (`#[serde(alias = "block")]`).
- `CrossRepoEdge` (`:213`) — `from: Endpoint`, `to: Endpoint`, `note?`.
- `TierEntry` (`:229`) — `tier`, `rollup?`, `summary?`.
- `Origin` (`:246`) — `kind` (`#[serde(rename = "type")]`), `slug`.
- `Backlog` (`:265`) — `slug`, `title`, `repo`, `kind` (`#[serde(rename = "type")]`), `status`, `depends_on: Vec<BlockedBy>`, `block?`, `notes?`.
- `CarryoverScope` (`:294`) — `repo?`, `tier?`, `cross_repo: Option<bool>`.
- `Carryover` (`:305`) — `slug`, `scope: CarryoverScope`, `kind`, `text`, `related: Vec<BlockedBy>`, `clears_when?`, `created`.
- `StateFile` (`:337`) — top-level: `repo`, `kind`, `updated`, `focus: Focus`, `tracks: Vec<Track>`, `repos: Vec<RepoRollup>`, `cross_repo: Vec<CrossRepoEdge>`, `tiers: Vec<TierEntry>`, `note?`, `backlog: Vec<Backlog>`, `carryover: Vec<Carryover>`. Every collection `#[serde(default)]`; **no** `deny_unknown_fields`.
- `load_state(path: &Path) -> Result<StateFile, StateLoadError>` (`:380`) — read file → map read error to `Io`, `serde_json::from_str` → map to `Parse`.

Keep the same derives mev uses (`Debug, Clone, Deserialize, Serialize`, plus `PartialEq` on `BlockedBy`/`Endpoint`/`Origin`/`CarryoverScope`, `Default` on `Focus`).

#### 1.3 Add the block-dependency graph model to `crates/okf-core/src/state.rs`
**File:** `crates/okf-core/src/state.rs` (same file, append below the serde section)
**Action:** port from `../mev/src/brain/state.rs`:
- `StateSource` (`:400`) — `repo_slug: String`, `abs_path: PathBuf`, `expected_kind: &'static str`. Derive `Debug, Clone`. (This is the pure discovery record `build_state_graph` consumes; mev's `discover_state_files` that *produces* it stays in mev.)
- `StateEdgeKind` (`:826`) — `#[serde(rename_all = "snake_case")]` enum `BlockedBy | CrossRepo`; derive `Debug, Clone, PartialEq, Eq, Serialize`.
- `StateEdge` (`:840`) — `from: String`, `to_ref: String`, `kind: StateEdgeKind`, `source_path: PathBuf` (`#[serde(skip)]`). Derive `Debug, Clone, Serialize`.
- `StateNode` (`:857`) — `key`, `repo`, `id`, `title`, `source_path: PathBuf` (`#[serde(skip)]`). Derive `Debug, Clone, Serialize`.
- `StateGraph` (`:877`) — `nodes: Vec<StateNode>`, `edges: Vec<StateEdge>`; derive `Debug, Default, Serialize`.
- `build_state_graph(files: &[(StateSource, StateFile)]) -> StateGraph` (`:909`) — port the body verbatim: one `StateNode` per `tracks[].blocks[]` (key `"{repo}:{id}"`), one `BlockedBy` `StateEdge` per `{type:"block"}` `depends_on` entry (skip `External`), one `CrossRepo` `StateEdge` per brain-file `cross_repo[]` entry. Uses `src.repo_slug` / `src.abs_path` from `StateSource`.

Add `use std::path::{Path, PathBuf};` and `use serde::{Deserialize, Serialize};` at the top of the file.

#### 1.4 Wire the module into the crate root
**File:** `crates/okf-core/src/lib.rs`
**Action:** after the existing `mod parse;` line add `mod state;`, and after the existing
`pub use parse::{...};` line add:
```rust
pub use state::{
    Backlog, Block, BlockedBy, Carryover, CarryoverScope, CrossRepoEdge, Endpoint, Focus, Origin,
    RepoRollup, StateEdge, StateEdgeKind, StateFile, StateGraph, StateLoadError, StateNode,
    StateSource, TierEntry, Track, TrackBlock, build_state_graph, load_state,
};
```
(Append-only — do not reorder the existing `frontmatter`/`parse` lines. Step 3 also appends to this
file; task 3 `dependsOn` task 1 so these edits never merge concurrently.)

#### 1.5 Add fixtures + unit tests for the state module
**File:** `crates/okf-core/src/state.rs` (add a `#[cfg(test)] mod tests`)
**Action:** write these cases (mirror mev's own state tests where they exist, but assert against
okf-core's public API):
- `load_state_roundtrip_real_fixture` — embed a representative leaf `state.json` (a `kind:"project"` file with `focus.now`/`next`/`blocked`, a `tracks[]` phase with a `depends_on: [{type:"block",...},{type:"external",...}]` block, and a `carryover[]` entry) as a string constant; `serde_json::from_str::<StateFile>` it, then re-serialize with `serde_json::to_value` on both and assert equal (round-trip fidelity). Use a real shape drawn from `planning/state.json` / `planning/state-schema.md`.
- `load_state_brain_fixture` — a `kind:"brain"` file with `repos[]`, `cross_repo[]`, and `tiers[]`; assert the counts deserialize.
- `blocked_by_unknown_type_is_rejected` — `serde_json::from_str::<StateFile>` on a file whose `depends_on[]` entry has `"type":"bogus"` returns `Err` (proves the tagged enum rejects unknown kinds → `E_STATE_SCHEMA_BAD_BLOCKED_BY` upstream).
- `block_id_alias_reads_v1_key` — a block authored with `"block":"BA.1.A"` (v1) deserializes with `id == "BA.1.A"` (proves the `#[serde(alias = "block")]`).
- `load_state_missing_file_is_io_error` — `load_state(Path::new("/nonexistent/state.json"))` returns `Err(StateLoadError::Io { .. })`.
- `load_state_malformed_json_is_parse_error` — write `"{ not json"` to a `tempfile`, `load_state` it, assert `Err(StateLoadError::Parse { .. })`.
- `build_state_graph_nodes_and_edges` — construct two `(StateSource, StateFile)` pairs (repo `"a"` with block `X` depending on `{type:"block", repo:"b", id:"Y"}` plus an `{type:"external"}`; repo `"b"` with block `Y`); assert the graph has 2 nodes keyed `"a:X"`/`"b:Y"`, exactly one `BlockedBy` edge `a:X → b:Y`, and zero edges from the external dep.
- `build_state_graph_cross_repo_edge` — a brain file with one `cross_repo[]` entry yields one `CrossRepo` `StateEdge`.

If the tests need `tempfile`, add `tempfile = "3"` under a new `[dev-dependencies]` in
`crates/okf-core/Cargo.toml`.

**Verify:** `cargo test -p okf-core state::` → all state tests pass; `cargo clippy -p okf-core -- -D warnings` → clean.

---

### Step 2: okf-core — reconcile `OkfFrontmatter` with mev's model

> Read `crates/okf-core/src/frontmatter.rs` (the target) and `../mev/src/brain/okf.rs:35` (the
> reference). The only structural delta is: mev carries a `synced_from: Option<String>` field that
> okf-core lacks. mev's `Option<Vec<String>>` for `layer`/`keywords`/`related` vs okf-core's
> `Vec<String>` **both already tolerate an absent field** (okf-core via `#[serde(default)]`), so no
> change is needed there — an absent list deserializes to an empty `Vec`. Confirm this rather than
> reshaping to `Option<Vec<_>>` (reshaping would break every current bastion caller and the
> hand-rolled serializer).

#### 2.1 Add the `synced_from` field to `OkfFrontmatter`
**File:** `crates/okf-core/src/frontmatter.rs`
**Action:** in `struct OkfFrontmatter` (line 24), after the `related` field (line 42) add:
```rust
    /// Cross-repo sync watermark: the `synced_from` date the brain cache was last synced from.
    /// Presence-only; not format-checked here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced_from: Option<String>,
```
Do **not** emit `synced_from` from `serialize_frontmatter` (the hand-rolled write path at line 55) —
it is a read-side watermark, not part of the canonical authored block, and leaving it out keeps
existing serializer output byte-identical. Keep all edits inside `frontmatter.rs`; **do not touch
`lib.rs`** — `OkfFrontmatter` is already re-exported, and Steps 1/3 own `lib.rs`.

#### 2.2 Add regression + new-field tests
**File:** `crates/okf-core/src/frontmatter.rs` (extend the existing `#[cfg(test)] mod tests`)
**Action:** add:
- `synced_from_deserializes_when_present` — `serde_json::from_str::<OkfFrontmatter>(r#"{"synced_from":"2026-07-01"}"#)` yields `synced_from == Some("2026-07-01")`. (Add `use serde_json;` in the test module if absent; `serde_json` is now a normal dep from Step 1.1.)
- `synced_from_absent_defaults_to_none` — deserializing `{}` yields `synced_from == None` and does not error.
- `absent_lists_default_empty` — deserializing `{"type":"Doc"}` yields `layer`/`keywords`/`related` all empty `Vec` (confirms the absent-field tolerance the reconciliation relies on).
- `serialize_unchanged_with_synced_from_set` — build the existing `full()` fixture, additionally set `synced_from: Some("2026-07-01")`, and assert `serialize_frontmatter(&fm)` output is **identical** to the pre-existing `serialize_full_exact_output` expected string (i.e. `synced_from` never appears in serialized output → no regression for current callers).

**Verify:** `cargo test -p okf-core frontmatter::` → all pass, including the unchanged
`serialize_full_exact_output` and `serialize_default_emits_only_required_keys_empty`.

---

### Step 3: okf-core — graph/edge-resolution model + `GraphExport` v2 emitter

> Boundary: `build_graph` and `check_graph` (mev `graph.rs`) depend on `Corpus`/`Diagnostic` and stay
> in mev. Only the pure types + `resolve_edge` (which takes a pre-built `GraphArtifact`) move here,
> plus the whole of `graph_emit.rs` (which depends only on `GraphArtifact` + `resolve_edge`).

#### 3.1 Create `crates/okf-core/src/graph.rs` — graph types + `resolve_edge`
**File:** `crates/okf-core/src/graph.rs`
**Action:** port from `../mev/src/brain/graph.rs` (reference only):
- `EdgeKind` (`:46`) — `#[serde(rename_all = "snake_case")]` enum with the single variant `Related`; derive `Debug, Clone, PartialEq, Eq, serde::Serialize`.
- `Edge` (`:56`) — `from: String`, `to_ref: String`, `kind: EdgeKind`; derive `Debug, Clone, serde::Serialize`.
- `Node` (`:70`) — `id: String`, `scope: String`, `doc_id: String`, `rel: PathBuf`; derive `Debug, Clone, serde::Serialize`.
- `Graph` (`:86`) — `nodes: Vec<Node>`, `edges: Vec<Edge>`; derive `Debug, Default, serde::Serialize`.
- `GraphArtifact` (`:97`) — `graph: Graph`, `node_map: HashMap<String, usize>`, `leaf_keys: HashSet<String>`. All fields `pub` (tests and `graph_emit` construct/read them directly). No derive needed.
- `EdgeResolution` (`:199`) — enum `Resolved { node_id, doc_id } | LeafTarget { qualified } | Dangling { qualified }`; derive `Debug, Clone, PartialEq, Eq`.
- `resolve_edge(artifact: &GraphArtifact, edge: &Edge) -> EdgeResolution` (`:228`) — port the body verbatim: derive the referrer scope from `edge.from` via `node_map`→`graph.nodes`, qualify a bare `to_ref` as `"{from_scope}:{to_ref}"`, then classify as `Resolved` (in `node_map`), `LeafTarget` (in `leaf_keys`), or `Dangling`.

Add `use std::collections::{HashMap, HashSet};`, `use std::path::PathBuf;`. Do **not** port
`build_graph` or `check_graph`.

#### 3.2 Create `crates/okf-core/src/graph_emit.rs` — `GraphExport` v2 emitter
**File:** `crates/okf-core/src/graph_emit.rs`
**Action:** port `../mev/src/brain/graph_emit.rs` (reference only), repointing its imports from
`crate::brain::graph::{...}` to `crate::graph::{EdgeKind, EdgeResolution, GraphArtifact, Node, resolve_edge}`:
- `GraphExport` (`:30`) — `version: String`, `root: String`, `nodes: Vec<Node>`, `edges: Vec<ExportedEdge>`, `leaves: Vec<String>`; derive `Debug, Serialize`.
- `ExportedEdge` (`:55`) — `from`, `to_ref`, `kind: EdgeKind`, `target_node_id: Option<String>`, `target_doc_id: Option<String>`; derive `Debug, Clone, Serialize, PartialEq, Eq`.
- `build_graph_export(root: &Path, artifact: &GraphArtifact) -> GraphExport` (`:84`) — port verbatim: sorted `leaves` from `artifact.leaf_keys`; map each `artifact.graph.edges` entry through `resolve_edge` (`Resolved` → both target fields `Some`; `LeafTarget`/`Dangling` → both `None`); set `version: "2".to_string()`.

Add `use std::path::Path;` and `use serde::Serialize;`.

#### 3.3 Wire the two modules into the crate root
**File:** `crates/okf-core/src/lib.rs`
**Action:** append (after Step 1.4's `mod state;`): `mod graph;` and `mod graph_emit;`, and after
Step 1.4's `pub use state::{...};` block add:
```rust
pub use graph::{Edge, EdgeKind, EdgeResolution, Graph, GraphArtifact, Node, resolve_edge};
pub use graph_emit::{ExportedEdge, GraphExport, build_graph_export};
```
(Append-only. Task 3 `dependsOn` task 1, so this file is edited sequentially — never a concurrent
merge with Step 1.4.)

#### 3.4 Add unit tests for graph + graph_emit
**File:** `crates/okf-core/src/graph.rs` and `crates/okf-core/src/graph_emit.rs` (each an in-file
`#[cfg(test)] mod tests`)
**Action:** because `build_graph` is **not** in okf-core, tests must construct a `GraphArtifact` by
hand (a small helper that pushes `Node`s, fills `node_map` with `id → index`, and seeds `leaf_keys`).
Cover:
- `graph.rs`: `resolve_edge_resolved` (bare `to_ref` resolves within the referrer's scope to a real node → `Resolved { node_id, doc_id }`); `resolve_edge_qualified_ref` (a `scope:doc_id` `to_ref` resolves without re-qualifying); `resolve_edge_leaf_target` (`to_ref` in `leaf_keys` → `LeafTarget`); `resolve_edge_dangling` (`to_ref` in neither → `Dangling`).
- `graph_emit.rs`: `export_version_is_2_and_shape` (assert `version == "2"`, node/edge counts, and `edges[0].target_node_id`/`target_doc_id` are `Some` on a resolved edge); `dangling_and_leaf_edges_have_null_targets` (both target fields `None`); `empty_artifact_produces_empty_vecs`; `graph_export_serializes_with_v2_key` (serialize to `serde_json::Value`, assert `value["version"] == "2"` and `version`/`root`/`nodes`/`edges`/`leaves` keys present, and each `ExportedEdge` carries `target_node_id`/`target_doc_id` keys). These mirror mev's `graph_emit.rs` tests (`:157`–`:281`) but build the artifact directly instead of via `build_graph`.

**Verify:** `cargo test -p okf-core graph` → all graph + graph_emit tests pass; `cargo clippy -p okf-core -- -D warnings` → clean.

---

### Step 4: Validate

#### 4.1 Format gate
**Action:** run `cargo fmt --check` → exit 0.

#### 4.2 Lint gate
**Action:** run `cargo clippy -- -D warnings` → exit 0 (no warnings anywhere in the workspace).

#### 4.3 Test gate
**Action:** run `cargo test` → all pass. Confirm the workspace test count is **not lower** than the
pre-change baseline (the new okf-core `state`/`graph`/`graph_emit`/`frontmatter` tests are strictly
additive).

#### 4.4 Build gate
**Action:** run `cargo build --release` → exit 0.

#### 4.5 Boundary confirmation
**Action:** confirm `git status`/`git diff --stat` shows **no** files under `../mev/` were modified by
this spec, and that `crates/okf-core/Cargo.toml` gained only `serde_json` + `thiserror` (+ dev
`tempfile`) — no `bastion`/`mev`/`petgraph`/`serde_yaml` dependency crept in.

**Verify:** all four gated checks green; `git diff --stat -- ../mev` empty.

---

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

## Notes
- **Disjoint file ownership:** `crates/okf-core/src/lib.rs` is edited by both Step 1 (1.4) and Step 3
  (3.3), and `crates/okf-core/Cargo.toml` by Step 1 (1.1) and possibly Step 2/Step 1.5 (dev-dep). In
  `tasks.json` **task 3 `dependsOn` task 1** and task 2 owns only `frontmatter.rs`, so no two tasks
  edit `lib.rs`/`Cargo.toml` concurrently — the shared-file edits are all append-only. Task 2 (Step 2)
  is deliberately confined to `frontmatter.rs` to keep it independently parallelizable.
- **Reconciliation is smaller than the spec implies:** the only real `OkfFrontmatter` delta is the new
  `synced_from` field. okf-core's `#[serde(default)]` on `layer`/`keywords`/`related` already gives the
  absent-field tolerance mev's `Option<Vec<_>>` provides — do **not** reshape those to `Option<Vec<_>>`
  (it would break the hand-rolled serializer and every current bastion caller). This narrows Step 2 to
  a field addition + regression tests.
- **`build_graph`/`check_graph`/`discover_state_files`/`check_*`/`derive_*` do NOT move.** They depend
  on mev's `Corpus`, `BrainConfig`, and `Diagnostic`, which are out of scope. okf-core gets the pure
  data types + `resolve_edge` + `build_state_graph` + `build_graph_export` only. Because `build_graph`
  stays in mev, okf-core's graph tests must hand-construct a `GraphArtifact` (Step 3.4).
- **Downstream:** once this ships, mev's `planning/ticket-ba15-12-okf-core-convergence/` (already
  written, currently `blocked`) deletes mev's dupes, adds `okf-core = { path = "../bastion/crates/okf-core" }`,
  and repoints callers. The corpus-wide `validate-brain`/`graph` output-parity acceptance bar is
  asserted **there**, not here. Clear this repo's `ba15-12-mev-context-seed` carryover after this spec
  commits — the mev-side context (D9 mirror + ticket) already exists.
- **Standing rules applied:** CLAUDE.md rule 1 (tests ship with every change — Steps 1.5/2.2/3.4) and
  rule 6 (pure logic exhaustively tested without I/O: `build_state_graph`/`resolve_edge`/
  `build_graph_export` are pure and asserted directly; error paths `StateLoadError::Io`/`Parse` and
  the tagged-enum rejection covered; the only thin I/O shell — `load_state`'s `read_to_string` — is
  exercised via `tempfile` fixtures).
