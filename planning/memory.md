---
type: Reference
title: Bastion Memory
description: Repo-scoped durable memory for Bastion — episodic notes, preferences, superseded facts. Committed and portable.
doc_id: memory
layer: [factory]
project: bastion
status: active
keywords: [memory, episodic, preferences, durable, portable]
related: [knowledge, context, planning-index]
---

# Memory — Bastion

Repo-scoped **durable memory**: episodic notes, operator preferences, and superseded facts that
must survive a handoff and travel with the repo. Committed and portable — distinct from the global
`~/.claude/.../memory/` auto-memory (which is operator-level and stays on one machine).

Use this for project facts worth remembering across sessions. Promote durable "how it works"
knowledge to `knowledge.md`; promote settled choices to `decisions/`. Do not duplicate the global
auto-memory here.

## Notes

_Dated episodic entries — what was tried, what was decided in-flight, what to remember next time._

- **Cold-start race in `bastion ask`: `classify_state == Running` returns before Claude Code's TUI finishes initialization.** First cold-start smoke test timed out because the trigger was sent immediately after readiness was detected. Warm-session re-run (Claude TUI already initialized) succeeded. A short fixed delay post-readiness detection would close this gap; deferred as out-of-scope for Phase 5 Block G. Remember this when hardening `ask` reliability.
  source: planning/archive/phase5-blockG/tasks.md · date: 2026-06-21 · supersedes: — · freshness: 2026-06-27

- **Claude Code v2.1.185 sets `#{pane_current_command}` to its version string, not "claude".** The rename is done via `pthread_setname_np`. The original `foreground.trim() == "claude"` readiness check in `ask` never matched and caused every cold-start to timeout. Fixed with `classify_state == Running`. Track this across Claude Code upgrades — the process name may change again.
  source: planning/archive/decisions/D9-claude-readiness-via-classify-state.md · date: 2026-06-21 · supersedes: — · freshness: 2026-06-27

- **The "plain tokio await for actix-web" approach was disproven during Phase 11A spike.** It compiles and works for HTTP-only routes, but `actix-web-actors` WS actors silently fail or panic without an Arbiter. The correct runtime integration — dedicated OS thread with `actix_web::rt::System::new().block_on(...)` plus `tokio::task::spawn_blocking` in the dispatch arm — was settled in the runtime spike and must not be revisited without cause.
  source: planning/archive/11.A-serve-scaffold-and-api/tasks.md · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27

- **Phase 11A code review found 7 confirmed bugs after the block's initial PASS.** Bugs: empty token bypass (missing `is_empty()` check on `BASTION_SERVE_TOKEN`), WS continuation frames dropped in EchoActor (buffering removed prematurely), `/ws` route missing from `build_app()` (route wired but not mounted), `health()` handler not using type-safe `HealthResponse::ok()` constructor, misleading ping documentation, 401 response body used wrong integer format, unnecessary `String` allocation in auth middleware. All fixed before merge. Code review is essential even on green-test passes.
  source: log.md · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27

- **`bastion status` hard-errored on missing `DATABASE_URL` (regression caught during Phase 11B live testing).** The command returned a hard error instead of degrading gracefully. Fixed with a degradation path in `src/run/mod.rs` that completes with a "service unreachable" diagnostic. Inspect any new command path that touches DB for the same footgun.
  source: log.md · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27

- **`bastion validate --links` was flagging Rust identifiers in backticks as broken links (regression caught Phase 11 live testing).** e.g. `` `Result::Ok` ``, `` `async fn` `` were treated as URL targets and checked for headings. Fixed with backtick-span suppression in `src/validate/links.rs`. Any future link-extraction logic must exclude code spans first.
  source: log.md · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27

- **`bastion code --graph` was traversing `trees/` and `.git/worktrees/`, polluting the code graph.** Happened when scanning a multi-project Rust workspace. Exclusion filter added to `src/brain/code_graph.rs`. Remember to update the exclusion list if new non-source directories appear at workspace root.
  source: log.md · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27

- **Initial observability scaffolding assumed relational tables that don't exist in the orchestrator.** Stubs referenced `workflow_runs` / `node_states` tables. Recon before Phase 0 Block A established that all state is in one `events` JSON table. Corrected at D3. Before writing any new orchestrator query, re-read `docs/data-contract.md` — the real schema is non-obvious.
  source: planning/archive/decisions/D3-pin-data-contract.md · date: 2026-06-18 · supersedes: — · freshness: 2026-06-27

- **Phase 6B code review found double-print, missing `ConfigError::NoWorkspaceRegistry` variant, and `Config::load` doing a separate file read instead of delegating.** Added `NoWorkspaceRegistry` (distinguishes "no [workspaces] table" from "key absent in registry"), deduped `Config::load` to delegate to `load_workspace_registry`, removed the extra `eprintln!`. Pattern: config module grows subtly duplicated paths; audit on each extension.
  source: log.md · date: 2026-06-25 · supersedes: — · freshness: 2026-06-27

- **Phase 7A code review found keyword heuristics ordering bug in `classify_error()`.** Configuration errors were checked after tmux/process errors — wrong order for correct classification. Reordering matters: check more specific variants (typed `ConsoleError` downcast → `std::io::Error` downcast) before keyword heuristics, most-specific first.
  source: log.md · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27

- **Phase 5G `--dir` trust check behavior on unknown dirs (never-opened paths).** `trust_status` returns `Unknown` for directories with no entry in `~/.claude.json`; bastion proceeds past trust check and launches Claude. Only `Untrusted` (explicit `hasTrustDialogAccepted=false`) triggers fail-fast exit. Document this distinction for callers — `Unknown` is not safe, just indeterminate.
  source: planning/archive/phase5-blockG/tasks.md · date: 2026-06-21 · supersedes: — · freshness: 2026-06-27

- **tree-sitter ABI compatibility: 0.25/0.24 is the working pair.** `tree-sitter` crate 0.25 + `tree-sitter-rust` 0.24. A mismatch (both 0.25) caused an ABI error at link time during Phase 6C. Pin both versions before upgrading either.
  source: log.md · date: 2026-06-25 · supersedes: — · freshness: 2026-06-27

- **`workflow_done` is documented in `docs/serve-api.md` v0.3 as a live WebSocket event but is NOT actually wired to fire.** Phase 11 Block D shipped `FlowWatcher::observe()` (the pure detection logic) and the REST read endpoints, but explicitly deferred wiring `FlowWatcher` into the live `Hub` actor to actually push `workflow_done` over `/ws` — confirmed still true as of the 2026-07-02 archive pass (`FlowWatcher` is only constructed in `src/serve/poll.rs`'s own tests, never from `src/serve/ws/server.rs`). Anyone building on "the WS pushes `workflow_done`" should verify this gap is still open before relying on it; it is real unfinished work, not just stale documentation.
  source: planning/archive/phase11-blockD/tasks.md · date: 2026-06-30 · supersedes: — · freshness: 2026-07-02

- **`ui_theme::ACTIVE_THEME` is a process-wide `OnceLock` shared across the whole `cargo test` binary — do not mutate it mid-test via `init_theme()`.** Phase 14 Block BA.14.0's tests deliberately avoided calling `ui_theme::init_theme` inside unit tests to sidestep this hazard, asserting against whatever `current_theme()` resolves to at call time instead (deterministic and parallel-test-safe). Only one preset (`bastion`) exists today anyway, so there's no second preset to switch to for a true non-default-theme comparison — that's blocked on a future preset landing, not a test-writing gap.
  source: planning/archive/14.0-config-driven-theme/sdlc/worklog.md · date: 2026-07-02 · supersedes: — · freshness: 2026-07-02

- **BA.13.2 (mouse interactivity) was authored but never executed — shelved, not shipped.** The spec designs a pure `on_mouse(&mut self, kind, col, row) -> Option<Action>` dispatcher over `AppState`-tracked per-pane `Rect`s (`spine_area`/`browser_area`/`content_area`/`agent_panel_area`) routed via `bella_engine::geometry::point_in`/`body_pos`, explicitly deferring sub-tab click routing to BA.13.4 (not yet built either). No `sdlc/` work log exists and `tasks.md`'s own Notes were never filled in — confirm nothing in `src/sessions/app.rs`/`ui.rs` implements `on_mouse` before assuming this shelved design was picked up elsewhere. If mouse support is revisited, this spec's task breakdown (viewport Rect fields + dispatcher in Task 1, draw-time Rect population + event-arm wiring in Task 2) is the starting design, not a record of what was built.
  source: planning/archive/13.2-mouse-interactivity/tasks.md · date: 2026-07-02 · supersedes: — · freshness: 2026-07-02

- **Disjoint-ownership tasks in one sequential worktree can still break each other's compile even when each task's own file is correct.** During Phase 13 Block BA.13.0, Task 2 (`src/sessions/app.rs`) was fully correct in isolation, but the crate-wide `cargo build`/`cargo test` gate failed because Task 3's files (`src/sessions/ui.rs`, `src/sessions/tui_tests.rs`) still referenced the removed tab API. Since validation runs against the whole crate, not per-task, the agent made minimal compile-fixing adaptations to the not-yet-owned files to unblock the gate rather than leaving it red until Task 3 ran. Expect this whenever a spec's "owns: file X only" tasks share a compilation unit — a full-crate gate can force an earlier task to touch a later task's files just enough to compile, without doing that task's real work.
  source: planning/archive/13.0-spine-primary-navigation/sdlc/worklog.md · date: 2026-07-02 · supersedes: — · freshness: 2026-07-02

- **A worktree path-dependency depth bug in `Cargo.toml` was worked around with local-only, uncommitted symlinks** (`core/bastion/portfolio -> ../../portfolio`, `core/bastion/trees/bella -> ../../bella`) **during Phase 13 Block BA.13.0's Task 1.** The symlinks were deliberately left uncommitted to help downstream tasks in the same pipeline build/test; if a future worktree-based SDLC run hits unresolvable path-dependency errors for `bella-engine` or similar path deps, check for stale local symlinks like these before re-diagnosing from scratch.
  source: planning/archive/13.0-spine-primary-navigation/sdlc/worklog.md · date: 2026-07-02 · supersedes: — · freshness: 2026-07-02

- **Phase 12 Block D ("Mission Control: apply the console theme") is only half-shipped despite `state.json` marking `BA.12.D` closed.** Task 2 (border theming via `ui_theme::border_dim_style()`/`border_active_style()`) is present in `src/monitor/ui.rs`. Task 1 (retheme `status_color()` + error/banner spans off raw `ratatui::Color` literals onto `ui_theme::cyan()`/`sage()`/`rose()`/`muted()`) was **never done** — as of archiving, `status_color()` still returns `Color::Yellow`/`Green`/`Red`/`DarkGray` literally, its unit tests still assert those literals, and the error span at `src/monitor/ui.rs` (near the detail-pane render) still hardcodes `Color::Red`. If Mission Control theming is revisited, Task 1 of this spec is still outstanding work, not settled history — verify against the actual source before trusting a spec's "closed"/"Not started" header.
  source: planning/archive/12.d-mission-control-theme/tasks.md · date: 2026-07-02 · supersedes: — · freshness: 2026-07-02

- **Phase 12 Block C (Kanban rows) shipped in the code without its spec's `## Notes`/status ever being updated.** `src/overview/mod.rs`'s Kanban lane `Layout` uses `Direction::Vertical` with the same `Percentage(33)/Percentage(33)/Percentage(34)` split the spec called for, confirming the row-layout change landed — but `tasks.md` still read "Not started" and `## Notes` was never filled in with the manual smoke-test result. Lesson: a task spec's own status header can lag the actual code; verify against the source before assuming a "Not started" spec is truly undone.
  source: planning/archive/12.c-kanban-rows/tasks.md · date: 2026-07-02 · supersedes: — · freshness: 2026-07-02

- **Phase 12 Block A's Task 6 ("Agent State Manifest Engine") was never executed as written — it was superseded by the dedicated BA.11.C0 spec, which shipped `src/detect/manifest.rs` first.** The ad-hoc `12.a-unified-console/tasks.md` proposed the same module from scratch; Tasks 1–4 of that plan (tab engine, mouse events, DAG tree, suspend UX) shipped, but 5 (bella-engine integration) and 6 were left unmarked/not done, since the state-detection need was met by the C0 block instead. If reopening console work, check `src/detect/` for what already exists before re-planning a manifest engine.
  source: planning/archive/12.a-unified-console/tasks.md · date: 2026-07-01 · supersedes: — · freshness: 2026-07-02

- **Priority sort in `src/detect/manifest.rs` must use `sort_by_key(std::cmp::Reverse(...))`, not a `sort_by` closure, for descending order.** `clippy::unnecessary_sort_by` rejects the closure form as a lint error under the `-D warnings` gate. Reach for `Reverse` first on any future descending-sort-by-priority code in this codebase.
  source: planning/archive/11.C0-agent-state-detection/sdlc/worklog.md · date: 2026-06-30 · supersedes: — · freshness: 2026-07-02

- **`tmux::send_keys` already appends `Enter` internally (two tmux invocations: literal keys + Enter).** The WS hub's `Send` frame handler just calls `tmux::send_keys` directly — no separate Enter press is needed and no double-Enter risk. Any future caller of `send_keys` should not add its own trailing Enter.
  source: planning/archive/11.C-websocket-hub/sdlc/worklog.md · date: 2026-06-30 · supersedes: — · freshness: 2026-07-02

- **`web::block` handlers for session routes must capture owned `String`s via move closures, not references, to satisfy `Send + 'static` bounds.** References inside the closure body then borrow the moved values safely. Any future `web::block`-wrapped handler over tmux fns should follow this ownership pattern rather than trying to thread borrowed request data through.
  source: planning/archive/11.B-session-rest/sdlc/worklog.md · date: 2026-06-26 · supersedes: — · freshness: 2026-07-02

- **`send_named_keys` (plural) was dead code after Phase 11B review.** The plural variant was included preemptively in the tmux.rs spec but the session handlers only ever use the singular form. Removed before merge. Avoid pre-building plural/batch variants of session verbs until a handler actually needs them.
  source: log.md · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27

## Preferences

_Project-specific preferences (tooling, style, workflow) the operator has expressed._

- **Coverage bar (CLAUDE.md Rule 6):** pure logic is exhaustively unit-tested without I/O; error/degradation paths tested explicitly; thin I/O shells are manually smoke-tested and the result recorded in `## Notes` of the task spec. A green `cargo test` alone is not "done."
  source: CLAUDE.md · date: 2026-06-18 · supersedes: — · freshness: 2026-06-27

- **Validation gate runs `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release` in that order.** All four must pass before a block is closed. Source of truth: `planning/harness.json`.
  source: CLAUDE.md · date: 2026-06-18 · supersedes: — · freshness: 2026-06-27

---

*Episodic + portable. For durable "how it works" knowledge see `knowledge.md`.*
