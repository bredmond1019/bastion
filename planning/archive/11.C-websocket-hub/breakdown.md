---
type: TaskBreakdown
title: "Breakdown — BA.11.C Task 4: hub + per-connection actors"
description: "Atomic sub-step decomposition of Task 4 (WebSocket hub actor + per-connection actor with topic subscriptions, ref-counted pane pollers, diff-and-push fan-out, send/send_key dispatch, and needs-input debounce) from the BA.11.C spec."
doc_id: 11-c-breakdown
layer: [console, surface]
project: bastion
status: archived
keywords: [breakdown, websocket, actix actor, hub, topic subscription, ref-count, watch, needs-input, debounce]
related: [11-c-websocket-hub, 11-c0-agent-state-detection, master-plan]
phase: 11
block: C
---

# Task Breakdown — BA.11.C: WebSocket hub + live pane + needs-input detection

## Source Spec
`planning/11.C-websocket-hub/tasks.md`

## Scope of this breakdown
This decomposes **Task 4 only** ("Hub + per-connection actors with topic subscriptions and poll fan-out"). Spec Tasks 1 (frame DTOs), 2 (needs-input adapter), 3 (pure poll core) are dependencies that must land first; Task 5 (route swap + v0.2 doc) and Task 6 (validate) run after. The APIs from Tasks 1–3 are referenced below by the names the spec assigns them.

## Goal
Replace the `/ws` echo actor with a real session hub — topic-based subscriptions (`sessions`, `pane:<name>`), background poll tasks fanning out diffed pane updates over `watch` channels, key/named-key send frames reusing the tmux substrate, and a `event{needs_input}` push driven by Block C₀'s `detect::detect()` manifest engine — and bump `docs/serve-api.md` to v0.2.

## How to Use
Work top to bottom. Each sub-step is a single atomic action. Run the inline **Verify** checks as you go — do not batch them at the end. The crate still routes `/ws` to the echo actor until Task 5, so `cargo build`/`cargo test` stay green throughout Task 4 (the new actors compile as soon as they are declared in 4.1; their pure helpers are tested in 4.6 and 4.9).

---

## Steps

### Step 4: Hub + per-connection actors with topic subscriptions and poll fan-out

#### 4.1 Declare the new WS submodules
**File:** `src/serve/ws/mod.rs`
**Action:** append two module declarations after the existing `pub mod echo;` (line 6):
```rust
pub mod server;
pub mod session;
```
`echo` stays declared (it is the route target until Task 5 swaps it; `#![allow(dead_code)]` is already set crate-wide in `src/main.rs`).

#### 4.2 Hub message types
**File:** `src/serve/ws/server.rs` (new)
**Action:** create the file; define the connection id and the actix message types the hub handles. Use an `AtomicU64` connection counter — **not `Uuid`** (`uuid` is not a dependency; do not add one).
```rust
use actix::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::serve::dto::{Topic, WsFrame};

/// Monotonic per-connection id (process-global counter; avoids a uuid dep).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnId(pub u64);

static NEXT_CONN_ID: AtomicU64 = AtomicU64::new(1);

impl ConnId {
    pub fn next() -> Self {
        ConnId(NEXT_CONN_ID.fetch_add(1, Ordering::Relaxed))
    }
}

/// A server→client frame delivered to one connection actor, which writes it to the socket.
#[derive(Message)]
#[rtype(result = "()")]
pub struct ServerFrame(pub WsFrame);

#[derive(Message)]
#[rtype(result = "()")]
pub struct Connect {
    pub id: ConnId,
    pub addr: Recipient<ServerFrame>,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct Disconnect {
    pub id: ConnId,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct Subscribe {
    pub id: ConnId,
    pub topic: Topic,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct Unsubscribe {
    pub id: ConnId,
    pub topic: Topic,
}
```

#### 4.3 Hub struct, state, and `Actor` impl
**File:** `src/serve/ws/server.rs`
**Action:** add the hub actor with its subscription maps and ref-counted pane-poller bookkeeping. Mirror `rag-engine-rs` `ChatServer` (`HashMap<id, Recipient>` + `do_send` fan-out), extended with topic sets and per-pane poll handles.
```rust
use actix::SpawnHandle;
use crate::detect::AgentState;

pub struct Hub {
    /// All connected clients.
    conns: HashMap<ConnId, Recipient<ServerFrame>>,
    /// Subscribers to the global `sessions` list topic.
    sessions_subs: HashSet<ConnId>,
    /// Subscribers per pane topic, keyed by session name.
    pane_subs: HashMap<String, HashSet<ConnId>>,
    /// Running pane-poll interval handles, keyed by session name (ref-counted via `pane_subs`).
    pane_handles: HashMap<String, SpawnHandle>,
    /// Per-pane diff cursor (Task 3) so only changed captures are pushed.
    pane_cursors: HashMap<String, crate::serve::poll::PaneCursor>,
    /// Last agent state seen per pane, for needs-input rising-edge debounce.
    pane_last_state: HashMap<String, AgentState>,
    /// Handle for the single shared sessions-list interval (started on first subscriber).
    sessions_handle: Option<SpawnHandle>,
    /// Poll cadence in seconds (from config `poll_interval_secs`).
    poll_secs: u64,
}

impl Hub {
    pub fn new(poll_secs: u64) -> Self {
        Self {
            conns: HashMap::new(),
            sessions_subs: HashSet::new(),
            pane_subs: HashMap::new(),
            pane_handles: HashMap::new(),
            pane_cursors: HashMap::new(),
            pane_last_state: HashMap::new(),
            sessions_handle: None,
            poll_secs,
        }
    }
}

impl Actor for Hub {
    type Context = Context<Self>;
}
```

#### 4.4 Pure refcount + debounce helpers
**File:** `src/serve/ws/server.rs`
**Action:** add the small pure functions that decide poller lifecycle and event emission — extracted so they unit-test without an actor (Rule 6).
```rust
/// First subscriber to a pane → start its poller. (count goes 0→1)
pub fn should_start_poll(prev_count: usize) -> bool {
    prev_count == 0
}

/// Last subscriber left a pane → stop its poller. (count goes 1→0)
pub fn should_stop_poll(new_count: usize) -> bool {
    new_count == 0
}

/// Needs-input rising edge: emit `event{needs_input}` only on the transition
/// INTO Blocked, not on every poll while still Blocked.
pub fn should_emit_needs_input(prev: Option<AgentState>, new: AgentState) -> bool {
    new == AgentState::Blocked && prev != Some(AgentState::Blocked)
}
```

#### 4.5 Hub handlers: Connect / Disconnect / Subscribe / Unsubscribe + pollers
**File:** `src/serve/ws/server.rs`
**Action:** implement the four `Handler`s and the interval pollers.
- `Handler<Connect>` → `self.conns.insert(msg.id, msg.addr)`.
- `Handler<Disconnect>` → remove from `conns` and from `sessions_subs`; for every pane set containing the id, remove it and if `should_stop_poll(new_count)` cancel `pane_handles[name]` (`ctx.cancel_future`), and drop `pane_cursors`/`pane_last_state` for that pane. If `sessions_subs` becomes empty, cancel `sessions_handle`.
- `Handler<Subscribe>` for `Topic::Sessions`: insert id into `sessions_subs`; if `sessions_handle.is_none()` start the shared sessions poll via `ctx.run_interval(Duration::from_secs(self.poll_secs), ...)` and store the handle. The interval body offloads `tmux::list_sessions_raw()` (blocking) through `actix_web::web::block(...)`, resolves it back into the actor with `.into_actor(self)`, builds `poll::sessions_snapshot(&raw)` → wraps in a `WsFrame{kind: Sessions, payload: SessionsPayload{..}}` → `do_send`s a `ServerFrame` to every id in `sessions_subs`.
- `Handler<Subscribe>` for `Topic::Pane(name)`: insert id into `pane_subs[name]`; if `should_start_poll(prev_count)` start a per-pane `run_interval` storing the handle in `pane_handles[name]`. The interval body offloads `tmux::capture_pane_raw(&name)` via `web::block`, then back in-actor: feed the capture to `pane_cursors[name].observe(&capture)` (Task 3) — on `Some((seq, lines))` push a `WsFrame{kind: Pane, payload: PanePayload{session, seq, lines}}` to that pane's subscribers; compute `let state = detect_state(&capture)` and if `should_emit_needs_input(pane_last_state.get(name).copied(), state)` push a `WsFrame{kind: Event, payload: EventPayload{session: name, event: "needs_input"}}`; update `pane_last_state[name] = state`.
  - Use `crate::serve::status::detect::needs_input(&capture)` for the boolean, and obtain the `AgentState` for the debounce from the same adapter — **add a thin `pub fn detect_state(pane: &str) -> AgentState` to Task 2's `status/detect.rs`** if not already present (note in Notes; it is a one-line passthrough over the same compiled manifest).
- `Handler<Unsubscribe>`: remove id from the topic set; on `Topic::Pane(name)` if `should_stop_poll(new_count)` cancel + remove the pane poller/cursor/last-state; on `Topic::Sessions` if now empty cancel `sessions_handle`.
- Blocking tmux calls **must** go through `web::block`/`spawn_blocking` so the actix Arbiter is never stalled (runtime model is fixed — see spec Context Pointers).

**Verify:** `cargo build` → exit 0 (echo still wired; new hub compiles).

#### 4.6 Hub pure-helper unit tests
**File:** `src/serve/ws/server.rs` (append `#[cfg(test)] mod tests`)
**Action:** test the pure helpers from 4.4 (no actor spin-up — Rule 6):
- `should_start_poll`: `should_start_poll(0)` true; `should_start_poll(1)` false.
- `should_stop_poll`: `should_stop_poll(0)` true; `should_stop_poll(2)` false.
- `should_emit_needs_input`: `(None, Blocked)` → true; `(Some(Working), Blocked)` → true; `(Some(Blocked), Blocked)` → false; `(Some(Blocked), Working)` → false; `(Some(Idle), Idle)` → false.
- `ConnId::next` returns strictly increasing ids across two calls.

**Verify:** `cargo test serve::ws::server` → all pass.

#### 4.7 Per-connection actor: ServerFrame receipt + lifecycle
**File:** `src/serve/ws/session.rs` (new)
**Action:** create the per-connection WS actor adapted from `rag-engine-rs` `ChatSession`. It holds its `ConnId` + the `Addr<Hub>`, registers on `started`, deregisters on `stopping`, and writes server frames to the socket.
```rust
use actix::prelude::*;
use actix::{ActorContext, AsyncContext};
use actix_http::ws::Item;
use actix_web_actors::ws;

use crate::serve::dto::{WsFrame, WsFrameKind};
use crate::serve::ws::server::{ConnId, Connect, Disconnect, Hub, ServerFrame, Subscribe, Unsubscribe};

pub struct WsConn {
    id: ConnId,
    hub: Addr<Hub>,
    continuation_buf: Option<Vec<u8>>,
}

impl WsConn {
    pub fn new(hub: Addr<Hub>) -> Self {
        Self { id: ConnId::next(), hub, continuation_buf: None }
    }
}

impl Actor for WsConn {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.hub.do_send(Connect { id: self.id, addr: ctx.address().recipient() });
    }

    fn stopping(&mut self, _ctx: &mut Self::Context) -> Running {
        self.hub.do_send(Disconnect { id: self.id });
        Running::Stop
    }
}

/// Hub → client: serialize the frame and write it to the socket.
impl Handler<ServerFrame> for WsConn {
    type Result = ();
    fn handle(&mut self, msg: ServerFrame, ctx: &mut Self::Context) {
        if let Ok(txt) = serde_json::to_string(&msg.0) {
            ctx.text(txt);
        }
    }
}
```

#### 4.8 Per-connection actor: inbound frame dispatch
**File:** `src/serve/ws/session.rs`
**Action:** implement `StreamHandler<Result<ws::Message, ws::ProtocolError>>`. Reuse the `echo.rs` Ping→pong / Close / Continuation-buffering handling verbatim in shape; route `Text` frames through a pure dispatch helper (4.9) then act:
- Parse `Text` into `WsFrame` (`serde_json::from_str`); on parse error, `ctx.text` an `Error` frame (`WsFrameKind::Error` + `ErrorPayload`).
- Match `frame.kind`:
  - `Subscribe` → deserialize `payload` as `SubscribePayload`, `parse_topic(&p.topic)` → on `Some(topic)` `hub.do_send(Subscribe{id, topic})` and reply with an ack frame; on `None` reply `Error`.
  - `Unsubscribe` → as above → `hub.do_send(Unsubscribe{..})`.
  - `Send` → deserialize `SendPayload{session, keys}`; offload via `web::block(move || { tmux::send_keys(&session, &keys)?; tmux::send_named_key(&session, "Enter") })` (**`send_keys` does NOT append Enter, and there is no `send_enter` shell — press Enter with `send_named_key(name, "Enter")`**); on error push an `Error` frame.
  - `SendKey` → deserialize `SendKeyPayload{session, key}`; `web::block(move || tmux::send_named_key(&session, &key))`; on error push `Error`.
  - `Echo`/`Sessions`/`Pane`/`Event`/`Error` inbound → ignore (server→client kinds; ignoring inbound is the documented behaviour).
- Wrap the `.into_actor(self)` future pattern from `chat_session.rs` for the async `web::block` results so errors can write back to `ctx`.

#### 4.9 Per-connection pure dispatch helper + tests
**File:** `src/serve/ws/session.rs`
**Action:** extract the pure parse/classify seam so dispatch is unit-testable without a socket (the `echo.rs` `echo_text` precedent):
```rust
/// Outcome of classifying one inbound text frame, before any I/O.
#[derive(Debug, PartialEq)]
pub enum Inbound {
    Subscribe(crate::serve::dto::Topic),
    Unsubscribe(crate::serve::dto::Topic),
    Send { session: String, keys: String },
    SendKey { session: String, key: String },
    Ignore,          // a server→client kind arrived inbound
    Invalid(String), // parse error / bad topic — message for the Error frame
}

pub fn classify_inbound(text: &str) -> Inbound { /* parse WsFrame, match kind, parse_topic */ }
```
Append `#[cfg(test)] mod tests` covering: a valid `subscribe` to `pane:work` → `Inbound::Subscribe(Topic::Pane("work"))`; `subscribe` to `sessions` → `Subscribe(Topic::Sessions)`; `subscribe` to `pane:` → `Invalid`; a `send` frame → `Inbound::Send{..}`; a `send_key` frame → `Inbound::SendKey{..}`; an inbound `pane`/`sessions` kind → `Ignore`; malformed JSON → `Invalid`.

**Verify (group):**
```
cargo fmt --check && cargo clippy -- -D warnings && cargo test serve::ws
```
→ exit 0, all `server`/`session` tests pass. (Live socket behaviour is smoke-tested in spec Task 6, not here.)

---

## Acceptance Criteria
<!-- Copied verbatim from the spec — Task 4 delivers the hub/actor engine; the
     websocat/smoke criteria are exercised in spec Task 6 after Task 5 wires the route. -->
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
- **`send_keys` does NOT append Enter**, and there is **no `send_enter` shell fn** (only `send_enter_args`, an argv builder). The spec's "`send_keys`+`send_enter`" wording resolves in code to: `tmux::send_keys(name, keys)` then `tmux::send_named_key(name, "Enter")` — `send_named_key` is the shell that resolves named keys. Block B's REST `send` handler calls only `send_keys` (no Enter); the WS `Send` frame is meant to *land a command*, so it adds the Enter press.
- **`uuid` is not a dependency** — the reference `ChatSession` uses `SessionId(Uuid)`, but adapting it here uses `ConnId(u64)` from a process-global `AtomicU64`. Do not add `uuid`.
- **`detect_state` passthrough:** Task 2's adapter exposes `needs_input(pane) -> bool`; the debounce in 4.5 needs the underlying `AgentState`. Add a one-line `pub fn detect_state(pane: &str) -> AgentState` to `src/serve/status/detect.rs` (same compiled-manifest `OnceLock`, returns `detect::detect(pane, &m).state`). This is an additive edit to Task 2's file — if Tasks 2 and 4 run as parallel waves it must be declared on Task 4's side or Task 2 should ship `detect_state` up front; under sequential `/sdlc-flow` it is a non-issue. Flag it on the Task 2 ↔ Task 4 boundary.
- **Disjoint ownership:** Task 4 owns `src/serve/ws/server.rs` + `src/serve/ws/session.rs` (new) and appends two lines to `src/serve/ws/mod.rs` (additive). It does **not** edit `src/serve/mod.rs` — the `/ws` route swap + hub boot is spec Task 5's sole responsibility (the one non-additive `serve/mod.rs` edit). The only cross-task file touch is the `detect_state` addition to Task 2's `status/detect.rs` noted above.
- **actix poll mechanism:** use `AsyncContext::run_interval(Duration, |act, ctx| …)` for both the shared sessions poll and per-pane polls; store the returned `SpawnHandle` and cancel with `ctx.cancel_future(handle)` when the last subscriber leaves. Resolve blocking `web::block(...)` futures back into the actor with `.into_actor(self).then(...)` (the `chat_session.rs` pattern) so results mutate hub state on the actor thread.
- **Rule 6:** the actor/poll layer is the thin I/O shell smoke-tested in Task 6; the extracted pure seams (`should_start_poll`/`should_stop_poll`/`should_emit_needs_input`/`ConnId::next` in `server.rs`, `classify_inbound` in `session.rs`) carry the unit coverage. Keep tmux/process calls out of these helpers.
