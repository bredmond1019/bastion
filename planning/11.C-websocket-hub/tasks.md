---
type: TaskSpec
title: "Task Spec — BA.11.C: WebSocket hub + live pane + needs-input detection"
description: "Adapt the rag-engine-rs ChatServer/ChatSession actors into src/serve/ws/: topic subscriptions (sessions, pane:<name>), background poll tasks → watch channels → diff-and-push fan-out, and a needs-input event driven by Block C0's manifest engine (Blocked && visible_blocker). Extend the serve-api contract to v0.2."
doc_id: 11-c-websocket-hub
layer: [console, surface]
project: bastion
status: active
keywords: [websocket, actix actor, topic subscription, watch channel, live pane, needs-input, serve-api v0.2]
related: [11-c0-agent-state-detection, serve-api, master-plan, sessions]
phase: 11
block: C
---

# Task Spec — BA.11.C: WebSocket hub + live pane + needs-input detection

**Status:** Not started · **Last run:** never

## Goal
Replace the `/ws` echo actor with a real session hub — topic-based subscriptions (`sessions`, `pane:<name>`), background poll tasks fanning out diffed pane updates over `watch` channels, key/named-key send frames reusing the tmux substrate, and a `event{needs_input}` push driven by Block C₀'s `detect::detect()` manifest engine — and bump `docs/serve-api.md` to v0.2.

## Context Pointers
- **Plan:** `planning/master-plan.md` → *Phase 11 — BastionUI Console API* → *BA.11.C — WebSocket hub + live pane + "needs input" detection (prog. E)*. This is the killer BastionUI feature (brain D28): watch + alert + unblock from the phone, not just a polling viewer.
- **Reference to study before designing the hub:** `~/Dev/agentic-portfolio/Healthie/media_streams/` (production Ruby WS service) — port the patterns the block names: control/data topic split, `oneshot` subscription-confirm handshake, keep-alive checker → dead-connection cleanup, enum-keyed message dispatch, `Success`/`Failure` per-connection lifecycle. Also study `rag-engine-rs`'s `ChatServer`/`ChatSession` actors — `src/serve/ws/server.rs`/`session.rs` are adapted from them.
- **Consumed substrate (verified):**
  - **Block C₀ engine** (`src/detect/`, landed): `detect::manifest::parse_manifest(&str) -> Result<Manifest>`, `Manifest::compile() -> Result<CompiledManifest>`, `detect::detect(screen, &CompiledManifest) -> AgentDetection` with `AgentState::{Idle,Working,Blocked,Unknown}` + `visible_blocker`. The seed `src/detect/manifests/claude.toml` is loadable via `include_str!`. **Do not modify `src/detect/` — the engine is out of scope (it is Block C₀).**
  - **Existing WS scaffold** (`src/serve/ws/echo.rs`, Block A): the actix actor pattern — `impl Actor`/`StreamHandler` over `ws::Message`, explicit `Ping`→`pong`, fragmented-`Continuation` buffering, `ws::start(actor, &req, stream)` in the route handler. The hub/session actors follow this shape.
  - **Runtime model (do not change):** `src/serve/mod.rs::run` runs `actix_web::rt::System::new().block_on(run_server(...))` on a dedicated OS thread via `spawn_blocking`. The System provides the `Arbiter` actix actors need. New actors and poll tasks live inside this System; blocking tmux calls must be offloaded (`web::block` / `spawn_blocking`) so they never stall the Arbiter.
  - **Session substrate** (`src/sessions/`, Block B, all `pub`): `tmux::list_sessions_raw()`, `tmux::capture_pane_raw(name)`, `tmux::send_keys(name, keys)` + `tmux::send_enter(name)`, `tmux::send_named_key(name, key)`, `model::parse_sessions(raw) -> Vec<Session>`, `Pane::last_lines`. Reuse these — do not duplicate tmux logic (brain D21).
  - **Frame envelope** (`src/serve/dto.rs`, Block A): `WsFrame { kind: WsFrameKind, payload: serde_json::Value }`; `WsFrameKind { Echo, Error }`; `ErrorPayload { code, message }`. Block C extends this union.
- **Standing rules (`CLAUDE.md`):** Rule 1 (tests ship), Rule 2 (OKF frontmatter — only `docs/serve-api.md` changes, no new doc file; its `index.md` row already exists), Rule 6 (coverage bar — **the diff/seq + topic-parse + needs-input logic are pure functions, exhaustively unit-tested incl. error/degradation paths; the actor/poll I/O shell is smoke-tested and recorded in `## Notes`**).

## Step-by-Step Tasks

### 1. v0.2 frame schema + topic parsing (`src/serve/dto.rs`)
- Extend `WsFrameKind` (snake_case) with the client→server kinds `Subscribe`, `Unsubscribe`, `Send`, `SendKey` and the server→client kinds `Sessions`, `Pane`, `Event` (`Echo`/`Error` stay).
- Add payload structs (serde, `Debug, Clone, PartialEq`): `SubscribePayload { topic: String }`, `SendPayload { session: String, keys: String }`, `SendKeyPayload { session: String, key: String }`, `SessionsPayload { sessions: Vec<SessionDto> }`, `PanePayload { session: String, seq: u64, lines: Vec<String> }`, `EventPayload { session: String, event: String }` (the `needs_input` event string).
- Add a pure topic parser: `enum Topic { Sessions, Pane(String) }` + `fn parse_topic(s: &str) -> Option<Topic>` (`"sessions"` → `Sessions`; `"pane:<name>"` → `Pane(name)`, rejecting an empty name; anything else → `None`).
- Per Rule 6, unit-test (matching the existing `dto.rs` style): each new `WsFrameKind` serializes to its snake_case tag + round-trips; each payload round-trips + rejects a missing required field; `parse_topic` for `sessions`, `pane:work`, `pane:` (None), and an unknown topic (None).
- **Owns:** `src/serve/dto.rs` (append-only additions). **No dependencies.**

### 2. Needs-input detection adapter (`src/serve/status/`)
- Create `src/serve/status/mod.rs` (`pub mod detect;`) and `src/serve/status/detect.rs`.
- Load the Claude manifest once via `static` `OnceLock<CompiledManifest>`: `include_str!("../../detect/manifests/claude.toml")` → `parse_manifest` → `compile` (panic with a clear message if the seed manifest is malformed — it ships in-tree, so a failure is a build-time bug, asserted by a test).
- Expose the pure adapter `pub fn needs_input(pane: &str) -> bool` = `detect::detect(pane, manifest).state == AgentState::Blocked && detection.visible_blocker`, mapping `Blocked + visible_blocker` → the `needs_input` signal Block C emits as `event{needs_input}`.
- Create fixtures `src/serve/status/fixtures/needs_input.txt` (a Claude permission-prompt capture) and `no_input.txt` (a working/idle capture). Load via `include_str!`.
- Per Rule 6, unit-test: `needs_input(prompt_fixture) == true`; `needs_input(working_fixture) == false`; `needs_input("") == false`; a test that the embedded `claude.toml` parses+compiles (guards against a future manifest edit breaking the adapter).
- **Owns:** `src/serve/status/` (new dir). Adds the `pub mod status;` declaration to `src/serve/mod.rs` (append-only one line). **Depends on:** Block C₀ (landed). Carries no Task 1 dependency.

### 3. Pane diff + sequence + session-list snapshot logic (`src/serve/poll.rs`, pure core)
- Create `src/serve/poll.rs` housing the **pure** fan-out core (the I/O wiring lands in Task 4):
  - `fn diff_pane(prev: Option<&str>, next: &str) -> bool` — true when `next` differs from `prev` (or `prev` is `None`), i.e. there is something new to push. Keep it a pure comparison so the actor only emits `PanePayload` on change.
  - A `PaneCursor { last: Option<String>, seq: u64 }` (or equivalent) with `fn observe(&mut self, capture: &str) -> Option<(u64, Vec<String>)>` — when the capture changed, bump `seq`, store it, and return `(seq, lines)` for a `PanePayload`; return `None` when unchanged (no push).
  - `fn sessions_snapshot(raw: &str) -> Vec<SessionDto>` — `parse_sessions` → `SessionDto::from`, the body of the `sessions` topic push.
- Per Rule 6, unit-test exhaustively: `diff_pane` (None→true, same→false, changed→true); `PaneCursor::observe` (first capture pushes seq 1; unchanged returns None and does **not** bump seq; changed bumps to 2); `sessions_snapshot` over a fixture raw `list-sessions` string → expected `SessionDto`s. No tmux/process I/O in these tests.
- **Owns:** `src/serve/poll.rs` (new). Adds `pub mod poll;` to `src/serve/mod.rs` (append-only). **Depends on:** Task 1 (`SessionDto`/`PanePayload` shapes referenced by the snapshot/cursor return types).

### 4. Hub + per-connection actors with topic subscriptions and poll fan-out (`src/serve/ws/server.rs`, `src/serve/ws/session.rs`)
- Create `src/serve/ws/server.rs` (hub actor, adapted from `rag-engine-rs` `ChatServer`) and `src/serve/ws/session.rs` (per-connection actor, adapted from `ChatSession`); declare both in `src/serve/ws/mod.rs` (`pub mod server; pub mod session;`, append-only).
- Hub responsibilities: track per-connection topic subscriptions; one shared `sessions`-list poll (~2s, `BASTION_POLL_INTERVAL`) → `watch` channel → fan-out to all `sessions` subscribers; **ref-counted per-session pane polls** started on first `pane:<name>` subscribe and stopped on last unsubscribe/disconnect; each pane poll uses `PaneCursor` (Task 3) to push a `PanePayload` only on diff; on each pane capture, run `status::needs_input` (Task 2) and emit `event{needs_input}` (debounced to the rising edge — emit once per Blocked→… transition, not every poll).
- Per-connection actor: parse inbound `WsFrame`s (Task 1) — `Subscribe`/`Unsubscribe` (register/deregister topics, confirm via a `oneshot`-style ack as the Healthie reference does), `Send`/`SendKey` (offload `tmux::send_keys`+`send_enter` / `send_named_key` via `web::block`), forward server→client frames; explicit `Ping`→`pong` and `Close` handling as in `echo.rs`; keep-alive timeout → drop subscriptions + release pane-poll refcounts (the Healthie keep-alive-checker analogue).
- Blocking tmux calls run via `web::block`/`spawn_blocking` so the Arbiter is never stalled (runtime model is fixed — see Context Pointers).
- Per Rule 6: the actor/poll layer is the thin I/O shell — its pure inputs (frame parsing dispatch, refcount transitions, debounce edge logic) get unit tests where extractable (e.g. a pure `refcount` helper, a pure `should_emit_event(prev_state, new_state) -> bool` debounce fn); the live socket behaviour is smoke-tested in Task 6 and recorded in `## Notes`.
- **Owns:** `src/serve/ws/server.rs`, `src/serve/ws/session.rs` (new); `src/serve/ws/mod.rs` (append-only module decls). **Depends on:** Tasks 1, 2, 3.

### 5. Route swap + serve-api v0.2 contract + integration tests (`src/serve/mod.rs`, `docs/serve-api.md`)
- In `src/serve/mod.rs`: swap the `/ws` route from `ws::echo::ws_handler` to a new `ws::server`-backed upgrade handler that starts the per-connection actor with a hub `Addr`; start the hub actor inside `run_server` (within the actix System). Mirror the change in the `build_app()` test helper so test routing matches production. This is the one **non-additive** edit to `serve/mod.rs` (the module-decl lines from Tasks 2–3 are additive).
- Integration tests via `actix_web::test`: `/ws` still rejects missing/wrong bearer token with `401` (extend the existing `ws_scope_rejects_*` tests for the new handler); the upgrade succeeds with a valid token. (Live streaming/needs-input behaviour is smoke-tested, not asserted in-process — Rule 6.)
- Bump `docs/serve-api.md` to **v0.2**: document the topic model (`sessions`, `pane:<name>`), every client→server frame (`subscribe`/`unsubscribe`/`send`/`send_key`) and server→client frame (`sessions`/`pane`/`event`/`error`) with payload shapes, the `event{needs_input}` semantics, and the keep-alive/disconnect behaviour. Append a dated v0.1→v0.2 line to the doc's Amendment Log.
- **Owns:** `src/serve/mod.rs` (the route swap + hub boot), `docs/serve-api.md`. **Depends on:** Tasks 1–4.

### 6. Validate
- Run the Validation Commands listed below and confirm all pass.
- **Smoke-test the live I/O shell** against a running `bastion serve` (Rule 6), record in `## Notes`: with `websocat` (or equivalent) + a bearer token — subscribe to `pane:<name>` and confirm live `pane` pushes arrive as the session output changes; send a `send` frame and a `send_key` `Escape` frame and confirm they land in the tmux session; put a session on a permission prompt (e.g. a `claude` approval dialog) and confirm an `event{needs_input}` frame is pushed.

## Acceptance Criteria
- `websocat` (with a valid bearer token) subscribes to `pane:<name>` and receives live `pane` pushes as the session's output changes (smoke-tested, recorded in `## Notes`).
- Sending keys via a `send` frame and `Escape` via a `send_key` frame over the socket lands in the target tmux session (smoke-tested).
- A session sitting on a permission prompt produces an `event{needs_input}` frame, driven by `detect::detect(pane, claude.toml).state == Blocked && visible_blocker` (Block C₀), not inline literals.
- `/ws` rejects missing/wrong bearer token with `401`; a valid-token upgrade succeeds (asserted in-process).
- The pure logic is unit-tested without I/O: `diff_pane` + `PaneCursor` sequencing, `sessions_snapshot`, `parse_topic`, the `needs_input` adapter over prompt/working/idle fixtures, the needs-input debounce edge helper, and every new DTO's serde round-trip + missing-field case.
- `docs/serve-api.md` is at v0.2 documenting topics, all client/server frame kinds + payload shapes, `event{needs_input}` semantics, and disconnect behaviour, with an Amendment Log entry.
- The runtime model is unchanged (`System::new().block_on` on a dedicated thread); `src/detect/` is not modified.
- All gated checks pass (`cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`).

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes

**Smoke test — 2026-06-30 (Task 6)**

Environment: `BASTION_SERVE_TOKEN=smoketest123 cargo run --release -- serve --addr 127.0.0.1:7979`, one tmux session `test-smoke` running zsh.

1. **`/ws` auth gate**: `curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:7979/ws` → `401`. Confirmed missing-token rejection.

2. **`sessions` subscription**: `websocat ws://127.0.0.1:7979/ws -H "Authorization: Bearer smoketest123"` with `{"kind":"subscribe","payload":{"topic":"sessions"}}` → received `{"kind":"sessions","payload":{"sessions":[{"last_line":"","name":"test-smoke","state":"idle"}]}}`. Session list fan-out confirmed.

3. **`pane:<name>` subscription + live pushes**: subscribed to `pane:test-smoke`, ran `echo hello-from-smoke-test` in the session → received `{"kind":"pane","payload":{"lines":[...],"seq":1,"session":"test-smoke"}}` on first capture, then `{"kind":"pane","payload":{"lines":[...],"seq":2,"session":"test-smoke"}}` after the output changed. Diff-and-push confirmed.

4. **`send` frame**: sent `{"kind":"send","payload":{"session":"test-smoke","keys":"echo ws-send-test"}}` over the socket while subscribed to `pane:test-smoke` → the string `ws-send-test` appeared in the next `pane` push. Keys-over-WebSocket confirmed.

5. **`send_key` Escape**: sent `{"kind":"send_key","payload":{"session":"test-smoke","key":"Escape"}}` → key landed in tmux session (visible in pane capture). Named-key path confirmed.

6. **`event{needs_input}`**: ran `printf 'Do you want to proceed? (y/n): '` in the tmux session (matching the `claude.toml` `visible_blocker` rule), while subscribed to `pane:test-smoke` → received `{"kind":"event","payload":{"event":"needs_input","session":"test-smoke"}}`. Rising-edge debounce and Block C₀ detect integration confirmed.

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
