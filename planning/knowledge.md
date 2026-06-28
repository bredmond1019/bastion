---
type: Reference
title: Bastion Knowledge
description: Distilled, durable knowledge for Bastion — how it works, conventions, and an architecture digest.
doc_id: knowledge
layer: [factory]
project: bastion
status: active
keywords: [knowledge, conventions, architecture, semantic memory, durable]
related: [context, status, memory, planning-index]
---

# Knowledge — Bastion

Distilled, **durable** project knowledge: how the system works, the conventions it follows, and an
architecture digest. This is *semantic memory* at repo scope — the things a new agent should read
to understand the project, kept current as the design settles.

Seed it from `context.md`, the decision record, and what you learn while building. Keep entries
durable (how things work), not episodic (what happened) — episodic notes go in `memory.md`, settled
choices go in `decisions/`. Each entry promoted from the cold archive tier carries provenance
(D35 format: claim · source · date · supersedes · freshness).

## How it works

_Architecture digest — the main components and how they fit together._

- **Two-surface architecture: workflow observability (Postgres) and process/session control (tmux).** These are independent tracks with no cross-coupling. The observability track (`monitor`, `inspect`, `costs`, `run`) reads the Python orchestrator's PostgreSQL. The session track (`sessions`, `attach`, `new`, `send`, `kill`, `ask`, TUI) shells out to tmux only. Session commands run with zero DB infrastructure.
  source: planning/archive/decisions/D4-session-management-surface.md · date: 2026-06-21 · supersedes: — · freshness: 2026-06-27

- **Single Rust crate; clap dispatch in `main.rs`.** Modules: `observ/`, `db/`, `api/`, `monitor/`, `inspect/`, `validate/`, `costs/`, `run/`, `sessions/`, `brain/`, `serve/`. Adding a workspace split was explicitly deferred — the crate is clean at module-per-surface and the tmux layer adds no new deps.
  source: planning/archive/decisions/D4-session-management-surface.md · date: 2026-06-21 · supersedes: — · freshness: 2026-06-27

- **Postgres pool is lazy / on-demand, never opened at startup.** Each `db::*` fn opens a short-lived `PgPoolOptions` pool on demand. Session commands never call `Config::load()` and never open a pool. The DB gate on `monitor` must not leak onto session verbs.
  source: planning/archive/decisions/D4-session-management-surface.md · date: 2026-06-21 · supersedes: — · freshness: 2026-06-27

- **bastion is a read-only observer of the orchestrator.** It reads the `events` table and the graph endpoint; it never writes or triggers orchestrator-side state. This is Governing Principle 4 — "observer, never writer."
  source: planning/archive/decisions/D2-observability-consumer-contract.md · date: 2026-06-18 · supersedes: — · freshness: 2026-06-27

- **Orchestrator data contract is pinned at v1.0.0.** A live run is reconstructed by merging two sources joined on node class name: DAG shape (nodes + edges, including pending nodes) from `GET /workflows/{type}/graph`, and per-node state (status, timing, error, input, token usage, output) from polling `events.task_context` in Postgres. There are no relational `workflow_runs`/`node_states` tables — all state is JSON in one `events` table. Read path is Hybrid: direct Postgres for live poll; the reserved `GET /events/{id}` HTTP API is documented but not depended on.
  source: planning/archive/decisions/D3-pin-data-contract.md · date: 2026-06-18 · supersedes: — · freshness: 2026-06-27

- **Phase 1 `bastion monitor` is gated on orchestrator D28 (incremental persistence).** The orchestrator currently persists `task_context` only once, at end of run — there is no mid-run state to poll. Phase 0 (`status`), Phase 2 (`inspect`/`costs`), and Phase 5 (sessions) are all unblocked.
  source: planning/archive/decisions/D2-observability-consumer-contract.md · date: 2026-06-18 · supersedes: — · freshness: 2026-06-27

- **`bastion serve` runs actix-web on a dedicated OS thread, separate from the tokio runtime.** `serve::run` is synchronous; the tokio dispatch arm calls it via `tokio::task::spawn_blocking`. Inside that thread, `actix_web::rt::System::new().block_on(...)` spins up the actix System and Arbiter needed by WebSocket actors. Plain-tokio-await works for HTTP routes but is not forward-safe for actix-web-actors WS actors.
  source: planning/archive/11.A-serve-scaffold-and-api/tasks.md · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27

- **`bastion serve` endpoint layout.** Default bind: `0.0.0.0:4317` (tailnet-reachable). `GET /health` is public (no auth). `/ws` upgrade and all `/api/*` routes require `Authorization: Bearer <BASTION_SERVE_TOKEN>`. Missing or wrong token returns 401. The token comparison is a pure `token_matches(header, expected) -> bool` function in `src/serve/auth.rs`.
  source: planning/archive/11.A-serve-scaffold-and-api/tasks.md · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27

- **Session REST API surface (Phase 11B).** Six routes under `/api/sessions` (auth required): `GET /sessions`, `GET /sessions/{name}/pane?lines=N`, `POST /sessions/{name}/send`, `POST /sessions/{name}/key`, `POST /sessions` (create), `DELETE /sessions/{name}` (kill). Named-key endpoint uses `tmux send-keys -t <name> <KeyName>` without `-l`/`--` flags, allowing tmux named-key resolution (Escape, Enter, arrows, C-c, etc.). Degradation mapping: not-installed/no-server → 503, unknown session → 404, other → 500.
  source: log.md · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27

- **`bastion ask` protocol (Phase 5G).** Sends a fixed trigger via `tmux send-keys` then polls for a `<out>.done` marker file. Trigger text: "Read <prompt-file> and follow its instructions exactly. Write your complete answer to <out>. When finished, create an empty file <out>.done". On finding the marker: removes it and returns exit 0. On timeout: captures the pane and exits non-zero with stderr diagnostics. Default launch command: `claude --permission-mode bypassPermissions`. Implements cross-repo contract at `docs/integrations/claude-code-llm-provider.md` §2. Trust pre-flight: fails fast on `Untrusted` dirs (never stalls on Claude's one-time prompt).
  source: planning/archive/phase5-blockG/tasks.md · date: 2026-06-21 · supersedes: — · freshness: 2026-06-27

- **Code-as-graph surface (Phase 6C).** Tree-sitter backed (`tree-sitter` + `tree-sitter-rust`, ABI-compatible pair 0.25/0.24). Pure `extract_symbols`/`extract_refs` functions. Walker skips hidden dirs, `target/`, `trees/`, and `.git/worktrees/`. Results exposed via `bastion code --def`/`--refs`/`--dependents`.
  source: log.md · date: 2026-06-25 · supersedes: — · freshness: 2026-06-27

- **OKF brain graph (Phase 6A).** `BrainGraph` wraps petgraph `DiGraph`. Node ids use filename stems (matching OKF wiki-link convention). `[[link]]` edges with unresolved targets are silently dropped at build time. DB-free and synchronous per D4/D5. Supports shortest path (A*), topological sort, DFS/BFS traversal, `--dependents`/`--blast-radius`/`--lineage` queries.
  source: log.md · date: 2026-06-25 · supersedes: — · freshness: 2026-06-27

- **Structured error taxonomy (Phase 7A).** C001–C014 vendored from `claude-sdk-rs` into `src/observ/errors.rs` — no crate dependency. `ErrorCode` Display yields `C001`…`C014`. `ConsoleError` + `ErrorContext` wrapper pairs a `ConsoleError` with the originating command. `is_recoverable()` covers: Timeout, RateLimitExceeded, StreamClosed, Io, ProcessError. Every dispatch arm emits start/outcome/duration structured events via `tracing`; `--json-logs` produces machine-parseable JSON; `--verbose` raises log verbosity.
  source: planning/archive/7-A-observability-and-control/tasks.md · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27

- **Multi-workspace brain configuration (Phase 6B).** `~/.config/bastion/config.toml` supports `workspaces`/`default_workspace` fields. `resolve_workspace_root` precedence: explicit `--root` > named `--workspace` > `default_workspace` in config > built-in `.`. `load_workspace_registry` degrades silently on absent files but propagates `ConfigError::MalformedFile` on broken TOML. `ConfigError::UnknownWorkspace` distinguishes "no [workspaces] table" from "key absent in registry."
  source: log.md · date: 2026-06-25 · supersedes: — · freshness: 2026-06-27

## Conventions

_Naming, patterns, and standing choices specific to this project._

- **tmux command construction/execution split.** Pure `*_args()` functions return `Vec<String>` (unit-testable without spawning tmux). A thin `run_tmux` executor does the I/O and maps `NotFound` + non-zero exit to typed `TmuxError` variants. Never mix construction and execution in one function.
  source: planning/archive/phase5-blockA/tasks.md · date: 2026-06-21 · supersedes: — · freshness: 2026-06-27

- **tmux format strings are named `const`s shared between producer and parser.** The `-F` format string and field separator live in constants so the parser and the command-builder cannot silently drift apart.
  source: planning/archive/decisions/D6-malformed-tmux-line-skip.md · date: 2026-06-21 · supersedes: — · freshness: 2026-06-27

- **Malformed tmux list-sessions output lines are skipped with a stderr warning, not fatal.** Partial session state is more useful than no state. Invocation failures (binary missing, no server) remain typed `TmuxError` variants.
  source: planning/archive/decisions/D6-malformed-tmux-line-skip.md · date: 2026-06-21 · supersedes: — · freshness: 2026-06-27

- **Session TUI keybindings: `k` is kill, not nav-up.** Navigation is Up/Down arrows plus `j` for down. There is deliberately no `k`-for-up binding. Single-key verb mnemonics (`a`ttach `n`ew `s`end `k`ill `q`uit) own their letters. Any future verb binding must check the legend first.
  source: planning/archive/decisions/D7-tui-keybindings-k-is-kill.md · date: 2026-06-21 · supersedes: — · freshness: 2026-06-27

- **`Action::Attach` is handled directly in the TUI run loop, not in `execute_action`.** Suspending and restoring the terminal requires the `ratatui::Terminal` handle, which lives in `run_inner` and is deliberately not passed into `execute_action`. `execute_action` covers `New`/`Send`/`Kill`/`None`; `Attach` is the one action handled inline.
  source: planning/archive/decisions/D8-attach-handled-in-run-loop.md · date: 2026-06-21 · supersedes: — · freshness: 2026-06-27

- **Session surface is synchronous (no tokio).** All tmux verbs use blocking `std::process::Command`. `sessions::run()` is a plain `fn`, not `async fn`. No tokio ceremony on an infrastructure-free surface. If a future verb needs concurrency, reach for `std::thread` before async.
  source: planning/archive/decisions/D5-sessions-synchronous.md · date: 2026-06-21 · supersedes: — · freshness: 2026-06-27

- **Claude readiness detection uses `classify_state == Running`, not an exact process-name match.** Claude Code renames its process to its version string via `pthread_setname_np`; `#{pane_current_command}` reports the version, not "claude". `classify_state` asks the inverse question: "is the foreground something other than an idle shell?" One source of truth for "is this session doing something": `IDLE_SHELLS` in `src/sessions/model.rs`.
  source: planning/archive/decisions/D9-claude-readiness-via-classify-state.md · date: 2026-06-21 · supersedes: — · freshness: 2026-06-27

- **Code graph node IDs are qualified: `{file_stem}::{kind}::{name}`.** Prevents silent collision when a file contains both `struct Widget` and `impl Widget`. `BrainNode.title` keeps the bare name for display. `BrainGraph.name_index` maps bare name → all nodes for CLI `--dependents` queries.
  source: planning/archive/decisions/D10-code-graph-qualified-node-ids.md · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27

- **serve config is DB-free.** `load_serve_config()` reads only `BASTION_SERVE_ADDR` (default `0.0.0.0:4317`) and mandatory `BASTION_SERVE_TOKEN`. Never calls `Config::load()`. Missing token is a typed error, never a silent empty default (empty string also rejected).
  source: planning/archive/11.A-serve-scaffold-and-api/tasks.md · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27

- **validate link extraction suppresses backtick spans.** Links are only extracted from non-code contexts, preventing false positives on Rust identifiers and keywords inside backticks (e.g. `` `Result::Ok` ``, `` `async fn` ``).
  source: log.md · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27

- **`bastion status` degrades gracefully when Postgres is unreachable.** The command completes with a "service unreachable" diagnostic rather than a hard error exit. This preserves the session-surface-always-works property of D4.
  source: log.md · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27

- **OKF project scaffolded from `base-template` commit `00ad2834`.** Every `.md` under `docs/` and `planning/` requires OKF frontmatter with `type`, `title`, `description` (required) and optional enriched fields per D27. Every new file to a directory requires updating that directory's `index.md`.
  source: planning/archive/decisions/D1-initial-okf.md · date: 2026-06-18 · supersedes: — · freshness: 2026-06-27

## Gotchas

_Non-obvious constraints, sharp edges, and hard-won lessons._

- **actix-web-actors WS actors require an actix System/Arbiter; plain tokio await is not sufficient.** Running `HttpServer::new(...).run().await` inside a tokio-spawned future works for HTTP only. WS actors fail silently or panic without the Arbiter. Solution: dedicated OS thread + `actix_web::rt::System::new().block_on(...)`.
  source: planning/archive/11.A-serve-scaffold-and-api/tasks.md · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27

- **The initial orchestrator data-contract assumption was wrong.** The initial stubs assumed relational `workflow_runs`/`node_states` tables. These do not exist — all state is JSON in one `events` table, edges live only in the graph endpoint. Corrected at D3 before Phase 1.
  source: planning/archive/decisions/D3-pin-data-contract.md · date: 2026-06-18 · supersedes: — · freshness: 2026-06-27

- **`bastion code --graph` must exclude `trees/` and `.git/worktrees/`** when scanning a workspace for Rust crates, or those directories pollute the code graph. Exclusion filter lives in `src/brain/code_graph.rs`.
  source: log.md · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27

---

*Durable knowledge. For episodic notes see `memory.md`; for the chronological narrative see the
root `log.md`.*
