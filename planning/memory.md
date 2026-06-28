---
type: Reference
title: Bastion Memory
description: Repo-scoped durable memory for Bastion — episodic notes, preferences, superseded facts. Committed and portable.
doc_id: memory
layer: [factory]
project: bastion
status: active
keywords: [memory, episodic, preferences, durable, portable]
related: [knowledge, context, status, planning-index]
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
