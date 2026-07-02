# Task Spec — Phase 11, Block D

**Status:** Done · **Last run:** 2026-06-30 (PASS)

## Goal
Expose repo/workflow status reads over the `bastion serve` API — `GET /repos`, `GET /repos/{name}/status`, `GET /repos/{name}/handoff`, `GET /repos/{name}/workflows` — backed by pure parsers for `status.md`, `handoff.md`, and `sdlc-flow-state.json`, with a `workflow_done` event pushed over the WebSocket hub when a flow-state file transitions from `running` to `done|blocked`.

## Context Pointers
- **Block definition:** `planning/master-plan.md` — `### BA.11.D` (lines 1004–1029)
- **Standing rules:** `GEMINI.md` / `CLAUDE.md` — pure-logic / I/O split (Rule 6), tests ship with every change (Rule 1), OKF frontmatter on new docs (Rule 2)
- **Existing serve surface:** `src/serve/mod.rs` (routing), `src/serve/dto.rs` (DTOs), `src/serve/handlers/` (handler modules), `src/serve/poll.rs` (pane-diff pure core), `src/serve/ws/server.rs` (Hub actor)
- **Workspace registry:** `src/config.rs` — `load_workspace_registry()` → `FileConfig { workspaces: Option<HashMap<String, PathBuf>> }`
- **Serve-api contract:** `docs/serve-api.md` (currently v0.2 — this block bumps to v0.3)
- **Out of scope:** Engine/orchestrator run state from Postgres (Block G); Flutter rendering; writing/mutating any files (read-only)

## Step-by-Step Tasks

### 1. Pure `status.md` parser (`src/serve/status/repo.rs`)
- Create `src/serve/status/repo.rs` with a pure parser that extracts structured data from a `status.md` file's content string.
- Parse the YAML frontmatter to extract `now`, `next`, `blocked` scalars (the D30 frontmatter scalars).
- Parse the body `## Momentum` section to extract the five queue lines (`now`, `next`, `blocked`, `improve`, `recurring`).
- Define a `RepoStatus` struct holding the parsed fields: repo name, now/next/blocked frontmatter scalars, has-handoff flag (passed in by caller), and the momentum queue lines.
- Add a `parse_status(content: &str) -> Option<RepoStatus>` pure function (returns `None` on malformed/absent frontmatter).
- Create fixture files under `src/serve/status/fixtures/` (at least: a well-formed `status.md` fixture, a malformed fixture with missing frontmatter, a fixture with empty momentum section).
- Write exhaustive unit tests: happy-path parsing, missing frontmatter → `None`, empty momentum → empty strings, round-trip through serde for `RepoStatus`.
- Register `repo` submodule in `src/serve/status/mod.rs`.

**Primary files:** `src/serve/status/repo.rs` (new), `src/serve/status/mod.rs` (modified — add `pub mod repo;`), `src/serve/status/fixtures/` (new fixtures)

### 2. Pure `handoff.md` reader + `sdlc-flow-state.json` parser (`src/serve/status/handoff.rs`, `src/serve/status/flow.rs`)
- Create `src/serve/status/handoff.rs` with a `read_handoff(content: &str) -> Option<HandoffInfo>` pure function that extracts the title (from frontmatter `title:` or the `# Handoff —` heading) and the raw markdown body. `HandoffInfo` is a simple struct: `{ title: String, body: String }`. Returns `None` on empty/unparseable input.
- Create `src/serve/status/flow.rs` with:
  - A `FlowState` struct matching the `sdlc-flow-state.json` shape: `spec_slug`, `branch`, `status` (string), `current_task`, `started_at`, `updated_at`.
  - `parse_flow_state(content: &str) -> Option<FlowState>` — pure JSON parse into the struct.
  - `is_terminal(status: &str) -> bool` — returns `true` for `"done"` or `"blocked"`.
  - `detect_transition(prev_status: Option<&str>, current: &FlowState) -> Option<String>` — returns `Some(event_name)` (e.g. `"workflow_done"`) when transitioning from a non-terminal to a terminal status; `None` otherwise.
- Add fixture files: at least one valid `sdlc-flow-state.json` fixture (borrow shape from `planning/archive/phase6-blockA/sdlc/sdlc-flow-state.json`), one malformed fixture, a minimal valid `handoff.md` fixture.
- Write exhaustive unit tests for both modules: happy paths, malformed input, transition detection edge cases (already-terminal → no event, non-terminal → terminal → event, `None` prev → no event).
- Register both submodules in `src/serve/status/mod.rs`.

**Primary files:** `src/serve/status/handoff.rs` (new), `src/serve/status/flow.rs` (new), `src/serve/status/mod.rs` (modified — add `pub mod handoff; pub mod flow;`), `src/serve/status/fixtures/` (new fixtures)

### 3. [~] DTO layer + poll extension (`src/serve/dto.rs`, `src/serve/poll.rs`)
- Add new DTOs to `src/serve/dto.rs`:
  - `RepoSummaryDto` — `{ name: String, now: String, has_handoff: bool }` (element of `GET /repos` array).
  - `RepoStatusDto` — full status shape returned by `GET /repos/{name}/status` (mirrors `RepoStatus` from Task 1, serializable).
  - `WorkflowStateDto` — serializable projection of `FlowState` from Task 2, returned as element of `GET /repos/{name}/workflows`.
  - `WorkflowDonePayload` — `{ repo: String, spec_slug: String, status: String }` for the `event{workflow_done}` WS push.
- Add a new `WsFrameKind::WorkflowDone` variant (or reuse `Event` with a discriminated `event` field — match the existing `EventPayload` pattern from BA.11.C; prefer the existing `Event` + `EventPayload` pattern with `event: "workflow_done"` and add the extra fields to the payload JSON).
- Extend `src/serve/poll.rs` with a pure `FlowWatcher` struct:
  - Holds a `HashMap<String, String>` mapping `(repo_name, spec_slug)` → last-known status.
  - `observe(repo: &str, flows: &[FlowState]) -> Vec<WorkflowDonePayload>` — compares current statuses against the map, emits payloads for any non-terminal → terminal transitions, updates the map.
- Write unit tests for all new DTOs (serde round-trip, missing fields) and for `FlowWatcher` (first observation → no events, status unchanged → no events, running→done → event, running→blocked → event, done→done → no event).

**Primary files:** `src/serve/dto.rs` (modified), `src/serve/poll.rs` (modified)

### 4. [~] REST handlers + route wiring + serve-api docs (`src/serve/handlers/status.rs`, `src/serve/handlers/mod.rs`, `src/serve/mod.rs`, `docs/serve-api.md`)
- Create `src/serve/handlers/status.rs` implementing:
  - `list_repos()` → `GET /repos` — calls `load_workspace_registry()`, iterates entries, for each: reads `{root}/planning/status.md` (parse via Task 1's parser), checks `{root}/planning/handoff.md` existence. Returns `Vec<RepoSummaryDto>`. Degrades gracefully: unreadable status.md → skip repo or return partial with empty `now`.
  - `get_repo_status(name)` → `GET /repos/{name}/status` — resolves workspace root from registry, reads + parses `planning/status.md`, returns `RepoStatusDto`. 404 on unknown name.
  - `get_repo_handoff(name)` → `GET /repos/{name}/handoff` — reads `planning/handoff.md`, returns raw markdown body as JSON `{ title, body }`. 404 when absent.
  - `get_repo_workflows(name)` → `GET /repos/{name}/workflows` — globs `planning/*/sdlc/sdlc-flow-state.json` under the repo root, parses each via Task 2's parser, returns `Vec<WorkflowStateDto>`. Empty array when none found.
- Register `pub mod status;` in `src/serve/handlers/mod.rs`.
- Wire the four new routes into the `/api` protected scope in `src/serve/mod.rs` (and mirror in `build_app()` test helper).
- Bump `docs/serve-api.md` version to v0.3 — document all four new endpoints, request/response shapes, error codes, and the `workflow_done` event in the WebSocket events section.
- Write actix-web integration tests in `src/serve/mod.rs::tests` for: auth enforcement on all four routes (missing token → 401), 404 on unknown repo name, and basic wiring (200 response structure).

**Primary files:** `src/serve/handlers/status.rs` (new), `src/serve/handlers/mod.rs` (modified), `src/serve/mod.rs` (modified — routes + test helper), `docs/serve-api.md` (modified)

### 5. [x] Validate
- Run the Validation Commands listed below and confirm all pass.
- Confirm the test count has increased beyond the 908 baseline.
- Verify `docs/serve-api.md` header says v0.3 and documents all four new endpoints.
- Smoke-test: inspect each handler's error paths in the tests (unknown workspace → 404, missing status.md → graceful degrade).

## Acceptance Criteria
- `GET /repos` returns every workspace registry repo with its `now` focus line and `has_handoff` flag.
- `GET /repos/{name}/status` returns the full parsed `status.md` content for a known workspace; returns 404 for an unknown name.
- `GET /repos/{name}/handoff` returns the handoff title + body when `planning/handoff.md` exists; returns 404 when absent.
- `GET /repos/{name}/workflows` returns parsed `sdlc-flow-state.json` entries under the repo; returns empty array when none exist.
- `FlowWatcher::observe()` detects a `running→done` transition and produces a `workflow_done` payload; no event on unchanged or already-terminal status.
- Pure parsers (`parse_status`, `read_handoff`, `parse_flow_state`, `detect_transition`) are exhaustively fixture-tested without I/O.
- All four REST routes reject missing/wrong bearer tokens with 401.
- `docs/serve-api.md` is bumped to v0.3 with all new endpoints and the `workflow_done` event documented.
- All gated checks pass (`cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`).

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
Task 5 validation run (2026-06-30): `cargo fmt --check` clean, `cargo clippy -- -D warnings` clean,
`cargo test` — 973 passed, 0 failed, 3 ignored (baseline was 908), `cargo build --release` clean.
`docs/serve-api.md` frontmatter title confirms "serve-api contract v0.3" and documents all four new
endpoints (`GET /repos`, `GET /repos/{name}/status`, `GET /repos/{name}/handoff`,
`GET /repos/{name}/workflows`) plus the `workflow_done` WS event. Handler error-path coverage
(401 on missing auth, 404 on unknown repo, graceful degrade on missing `status.md`) verified present
in `src/serve/mod.rs::tests` from Task 4.

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
- 2026-06-30 [task 4] Goal states a `workflow_done` event is "pushed over the WebSocket hub" when a flow-state file transitions; Task 4 shipped only the pure `FlowWatcher::observe()` (Task 3) and the REST read endpoints — it does not wire `FlowWatcher` into the live `Hub` actor to actually emit `workflow_done` over `/ws`. The WS push wiring is deferred to a later block (documented as such in `docs/serve-api.md`).
