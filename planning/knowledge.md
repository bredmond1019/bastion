---
type: Reference
title: Bastion Knowledge
description: Distilled, durable knowledge for Bastion — how it works, conventions, and an architecture digest.
doc_id: knowledge
layer: [factory]
project: bastion
status: active
keywords: [knowledge, conventions, architecture, semantic memory, durable]
related: [context, memory, planning-index]
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

- **Repo/workflow status REST surface (Phase 11 Block D, serve-api v0.3): `GET /repos`, `/repos/{name}/status`, `/repos/{name}/handoff`, `/repos/{name}/workflows`.** Backed by pure parsers in `src/serve/status/`: `parse_status` (D30 frontmatter scalars + `## Momentum` queue lines → `RepoStatus`), `read_handoff` (title + raw body from `handoff.md`), `parse_flow_state`/`is_terminal`/`detect_transition` (`sdlc-flow-state.json` → `FlowState`, terminal = `"done"`/`"blocked"`). `FlowWatcher` (`src/serve/poll.rs`) is a pure stateful `HashMap<(repo, spec_slug), String>` tracker whose `observe()` emits `WorkflowDonePayload`s on non-terminal→terminal transitions — reuses the existing `Event`/`EventPayload` WS frame pattern rather than adding a new `WsFrameKind` variant. The workspace registry (`FileConfig`) is loaded once at server startup and shared via `web::Data<FileConfig>` so handlers are unit-testable by injecting a fixture registry instead of real env vars.
  source: planning/archive/phase11-blockD/tasks.md · date: 2026-06-30 · supersedes: — · freshness: 2026-07-02

- **Config-driven runtime theme system (Phase 14 Block BA.14.0): one `Theme` shared by TUI chrome and the markdown view.** `src/ui_theme.rs` holds a process-wide `Theme` behind a `OnceLock` (`current_theme()`/`init_theme()`); named-color functions read the active theme instead of baked `rgb()`/`Color::` literals, which are now confined to the preset definitions (`theme_by_name(&str) -> Theme`, default/fallback preset `bastion`; only `bastion` is implemented today — `dark`/`light` are named-for-later, not built). A pure `to_bella_theme(Theme) -> bella_engine::Theme` mapping lets `render_with_edit` consume the same theme (both call sites in `src/sessions/ui.rs` were switched from the fixed `bella_engine::Theme::mission_control()`). `src/config.rs`'s `FileConfig` gained an optional `[theme]` section (`ThemeConfig { name }`) resolved via the pure `resolve_theme()`, defaulting to `bastion` when absent/unknown; existing configs with no `[theme]` still deserialize unchanged. `init_theme_from_config()` in `src/sessions/ui.rs::run()` is the thin I/O wrapper that wires config → runtime theme at TUI startup (untested directly per Rule 6 — it composes already-tested pure fns).
  source: planning/archive/14.0-config-driven-theme/sdlc/worklog.md · date: 2026-07-02 · supersedes: — · freshness: 2026-07-02

- **Persistent global agent panel: an always-on "agents · priority" strip under every `SelectedNode` (Phase 13 Block BA.13.1).** `session_urgency(&Session) -> u8` (lower = more urgent, Blocked/needs-input first) is a pure fn extracted from `build_mission_items` in `src/monitor/app.rs` and reused there — `build_mission_items`'s signature and Mission Control ordering are unchanged. `src/sessions/agent_panel.rs::agent_panel_rows(&[Session]) -> Vec<AgentPanelRow>` is a pure builder (session label + `AgentState` only, no theme/color fields) sorted by `session_urgency`; theme/color mapping happens only at render time in `src/sessions/ui.rs` via `agent_state_dot()` (reads `ui_theme::state_*_style()`, never literal colors). Strip height is computed by a pure `agent_panel_strip_height(row_count, frame_height)` — grows 3→7 lines with session count, shrinks toward (never below) 0 when vertical space is tight, so it never panics at small frame sizes.
  source: planning/archive/13.1-persistent-agent-panel/sdlc/worklog.md · date: 2026-07-02 · supersedes: — · freshness: 2026-07-02

- **`cargo clippy --all-targets -- -D warnings` is required to catch test-code-only lints; plain `cargo clippy` does not compile test targets.** Phase 13 Block BA.13.1's Task 4 hit a `clippy::collapsible_if` warning that only surfaced under `--all-targets`; the harness gate must include `--all-targets` (or run tests as part of the same pass) to be a true gate against test code.
  source: planning/archive/13.1-persistent-agent-panel/sdlc/worklog.md · date: 2026-07-02 · supersedes: — · freshness: 2026-07-02

- **The unified console's primary navigator is a spine-only sidebar, not top tabs (Phase 13 Block BA.13.0).** `src/brain/spaces.rs` exposes `SpineRow`/`SelectedNode` (`MissionControl` pinned first, `Hq`, selectable `Tier(name)`, `Space(...)`) and `spine_rows(&SpaceTree) -> Vec<SpineRow>` as a presentation layer over the unchanged `parse_space_tree`. The old `_root` tier is renamed `Hq` and the redundant standalone `brain` leaf is collapsed into it (data source: brain root `.`); `learn-ai`/`base-template` nest under `Hq`. `src/sessions/app.rs` tracks `selected_spine: usize` (wraps over **all** rows, headers included) with a derived `selected_node() -> SelectedNode`; all tab machinery (`tabs`, `active_tab_index`, `push_tab`/`close_tab`, `Tab`/`BackTab` keys) was removed — the markdown "open" (`t`) path became a transient `markdown_overlay: Option<PathBuf>` flag instead of a pushed tab. `src/sessions/ui.rs` routes the main area on `selected_node()`: `MissionControl` → `monitor::ui::render`, `Space`/`Hq` → Space Overview, `Tier` → `<tier>/planning/status.md` via a pure `tier_status_path(brain_root, tier)` helper, with a graceful empty-state degrade (no panic) when the tier's status file is absent.
  source: planning/archive/13.0-spine-primary-navigation/tasks.md · date: 2026-07-02 · supersedes: — · freshness: 2026-07-02

- **Mission Control unifies tmux sessions and orchestrator workflow runs into one list via `MissionItem` (Phase 12 Block E).** `src/monitor/app.rs::MissionItem` is `Session(sessions::model::Session) | Run(WorkflowRun)`. The pure `build_mission_items(sessions, runs) -> Vec<MissionItem>` orders needs-action sessions first, then running items, then idle/success — a standalone unit-tested merge function, not inline UI logic. `App` tracks a single `items: Vec<MissionItem>` + `selected: usize` instead of separate `selected_run`/`selected_node` fields. The `a`/`n`/`s`/`k` session key handlers act on the currently-selected `MissionItem::Session` when Mission Control is the active tab.
  source: planning/archive/12.e-mission-control-sessions/tasks.md · date: 2026-07-02 · supersedes: — · freshness: 2026-07-02

- **TUI `AppState` uses a dynamic `Vec<TabState>` + `active_tab_index`, not a static tab enum (Phase 12 Block A).** `TabState` variants: `SpaceOverview`, `MissionControl`, `MarkdownDocument(PathBuf)`. Layout math is a pure `compute_view(&self, area: Rect)` returning sidebar/main-content `Rect` boundaries, unit-tested without a real terminal. Mouse clicks map to `Action::SelectTab(usize)`. `bastion monitor`'s DAG is rendered as an indented tree (`├─`/`└─`) nested inside `TabState::MissionControl`, not a separate mode.
  source: planning/archive/12.a-unified-console/tasks.md · date: 2026-07-01 · supersedes: — · freshness: 2026-07-02

- **`suspend_and_attach(session_name)` (`src/sessions/tmux.rs`) prints a styled banner before `tmux attach`.** Banner text: `[ BASTION ] Attaching to Agent. Press Ctrl-b d to detach and return.` Terminal raw-mode/alt-screen state is suspended before attach and restored on detach — this is the pattern for any future "drop into an external terminal program" verb.
  source: planning/archive/12.a-unified-console/tasks.md · date: 2026-07-01 · supersedes: — · freshness: 2026-07-02

- **Agent-state detection engine (`src/detect/`, Phase 11 Block C₀) is a pure, config-driven manifest matcher.** `detect(screen: &str, manifest: &CompiledManifest) -> AgentDetection` resolves each `Rule`'s `region` selector (`whole` / `last_lines = N`) over the captured pane text, evaluates compiled gates (`contains`/`regex`/`line_regex` leaves; `any`/`all`/`not` combinators, regexes precompiled at `Manifest::compile()` time) in **descending priority** order, and returns the first match's `AgentState` (`Idle|Working|Blocked|Unknown`) + `visible_*`/`skip_state_update` flags. Adding a new agent (seeded: `claude.toml`, `pi.toml`) is a new TOML + fixture + golden test — zero engine-code change (D12).
  source: planning/archive/11.C0-agent-state-detection/tasks.md · date: 2026-06-30 · supersedes: — · freshness: 2026-07-02

- **WebSocket hub (Phase 11C, serve-api v0.2): topic subscriptions + ref-counted poll fan-out.** `src/serve/ws/server.rs` (hub actor, adapted from `rag-engine-rs` `ChatServer`) tracks per-connection topic subscriptions (`sessions`, `pane:<name>`). A shared `sessions`-list poll runs on `BASTION_POLL_INTERVAL` → `watch` channel → fan-out to all `sessions` subscribers. Per-session pane polls are **ref-counted**: started on the first `pane:<name>` subscribe, stopped on the last unsubscribe/disconnect. Each pane poll runs `PaneCursor::observe` (diff-and-push, only emits on change) and `status::needs_input` (Block C₀'s `detect::detect()` manifest engine), pushing `event{needs_input}` on the rising edge only (Blocked→… transition), not every poll tick.
  source: planning/archive/11.C-websocket-hub/tasks.md · date: 2026-06-30 · supersedes: — · freshness: 2026-07-02

- **`ConnId` is `u64` via `AtomicU64` counter, not `Uuid`.** `uuid` was not already a dependency, so the WS hub's per-connection ID uses an atomic counter instead of pulling in a new crate for a purely internal identifier.
  source: planning/archive/11.C-websocket-hub/sdlc/worklog.md · date: 2026-06-30 · supersedes: — · freshness: 2026-07-02

- **Session REST routes use `web::resource()` groupings, not bare `.route()`.** Bare `.route()` calls return 404 for an unregistered HTTP method on a registered path; `web::resource()` correctly returns 405 Method Not Allowed, which the Block B spec requires for the six `/api/sessions…` routes.
  source: planning/archive/11.B-session-rest/sdlc/worklog.md · date: 2026-06-26 · supersedes: — · freshness: 2026-07-02

- **`tmux_error_to_status` degradation mapping collapses `TmuxError::NotInstalled` and `NoServer` to the same HTTP/code pair.** Both map to `503` / `ErrorCode::C001` (BinaryNotFound) since both indicate tmux is unavailable at the system level; the pure helper downcasts `anyhow::Error` via `downcast_ref::<TmuxError>()` without disturbing `anyhow` propagation elsewhere in the handler chain.
  source: planning/archive/11.B-session-rest/sdlc/worklog.md · date: 2026-06-26 · supersedes: — · freshness: 2026-07-02

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
