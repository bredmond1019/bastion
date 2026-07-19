//! Hub actor for the `bastion serve` WebSocket surface.
//!
//! [`Hub`] is a process-singleton actix actor that:
//! - Tracks per-connection topic subscriptions.
//! - Runs one shared sessions-list poll (interval ~`poll_secs`) → fan-out to
//!   all `sessions` subscribers via a [`ServerFrame`] message.
//! - Maintains ref-counted per-pane poll intervals started on first subscribe
//!   and stopped when the last subscriber leaves or disconnects.
//! - Uses [`crate::serve::poll::PaneCursor`] to emit `pane` frames only on
//!   diff (no-change captures are silently dropped).
//! - Drives the needs-input rising-edge debounce (via
//!   [`crate::serve::poll::sessions_needing_input`]) from the **sessions-list**
//!   poller — an always-on per-session pane scan — so a session transitioning
//!   into `Blocked` reaches `sessions` subscribers even with no pane
//!   subscribed. See `crate::serve::poll` for the pure decision core.
//!
//! # Pure helpers (unit-tested, Rule 6)
//! - [`ConnId`] + monotonic counter
//! - [`should_start_poll`] / [`should_stop_poll`] — poller lifecycle decisions
//! - [`crate::serve::poll::should_emit_needs_input`] /
//!   [`crate::serve::poll::sessions_needing_input`] — rising-edge debounce
//!
//! # I/O shell (smoke-tested, Rule 6)
//! - `Handler<Connect>` / `Handler<Disconnect>` — connection lifecycle
//! - `Handler<Subscribe>` / `Handler<Unsubscribe>` — topic management + poller
//!   start/stop (blocking tmux calls offloaded via `web::block`)

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use actix::SpawnHandle;
use actix::prelude::*;
use actix_web::web;

use crate::config::FileConfig;
use crate::detect::AgentState;
use crate::serve::dto::{EventPayload, PanePayload, SessionsPayload, Topic, WsFrame, WsFrameKind};
use crate::serve::handlers::status::collect_flow_states;
use crate::serve::poll::{
    FlowWatcher, PaneCursor, sessions_needing_input, sessions_snapshot, sessions_with_last_line,
    workflow_done_frame,
};
use crate::serve::status::detect as status_detect;
use crate::sessions::tmux;

// ── Connection id ─────────────────────────────────────────────────────────────

/// Monotonic per-connection id (process-global counter; avoids a uuid dep).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnId(pub u64);

static NEXT_CONN_ID: AtomicU64 = AtomicU64::new(1);

impl ConnId {
    /// Allocate the next connection id.
    pub fn next() -> Self {
        ConnId(NEXT_CONN_ID.fetch_add(1, Ordering::Relaxed))
    }
}

// ── Hub messages ──────────────────────────────────────────────────────────────

/// A server→client frame delivered to one connection actor, which writes it to
/// the socket.
#[derive(Message)]
#[rtype(result = "()")]
pub struct ServerFrame(pub WsFrame);

/// Register a new connection with the hub.
#[derive(Message)]
#[rtype(result = "()")]
pub struct Connect {
    pub id: ConnId,
    pub addr: Recipient<ServerFrame>,
}

/// Deregister a connection from the hub (on WS close or keep-alive timeout).
#[derive(Message)]
#[rtype(result = "()")]
pub struct Disconnect {
    pub id: ConnId,
}

/// Subscribe a connection to a topic.
#[derive(Message)]
#[rtype(result = "()")]
pub struct Subscribe {
    pub id: ConnId,
    pub topic: Topic,
}

/// Unsubscribe a connection from a topic.
#[derive(Message)]
#[rtype(result = "()")]
pub struct Unsubscribe {
    pub id: ConnId,
    pub topic: Topic,
}

// ── Hub actor ─────────────────────────────────────────────────────────────────

/// Central WebSocket hub actor.
///
/// One `Hub` runs per actix `System` (started inside `run_server` in Task 5).
/// All per-connection [`WsConn`](super::session::WsConn) actors hold an
/// `Addr<Hub>` and send [`Connect`] / [`Disconnect`] / [`Subscribe`] /
/// [`Unsubscribe`] messages.
pub struct Hub {
    /// All connected clients (id → recipient for [`ServerFrame`]).
    conns: HashMap<ConnId, Recipient<ServerFrame>>,
    /// Subscribers to the global `sessions` list topic.
    sessions_subs: HashSet<ConnId>,
    /// Subscribers per pane topic, keyed by session name.
    pane_subs: HashMap<String, HashSet<ConnId>>,
    /// Running pane-poll interval handles, keyed by session name.
    pane_handles: HashMap<String, SpawnHandle>,
    /// Per-pane diff cursor so only changed captures trigger a push.
    pane_cursors: HashMap<String, PaneCursor>,
    /// Last agent state seen per session, keyed by session name, for the
    /// sessions-list poller's needs-input rising-edge debounce.
    sessions_last_state: HashMap<String, AgentState>,
    /// Handle for the single shared sessions-list interval.
    sessions_handle: Option<SpawnHandle>,
    /// Poll cadence in seconds.
    poll_secs: u64,
    /// Workspace registry the flow-watch poller enumerates each cycle
    /// (BA.0.A). Wrapped in `Arc` so a cycle's blocking file reads (spawned
    /// via `web::block`) can hold a cheap clone without requiring
    /// [`FileConfig`] itself to implement `Clone`.
    registry: Arc<FileConfig>,
    /// Stateful non-terminal→terminal tracker driving the `workflow_done` WS
    /// push — shared across poll cycles (BA.0.A).
    flow_watcher: FlowWatcher,
}

impl Hub {
    /// Create a new hub with the given poll interval and workspace registry
    /// (the latter drives the always-on flow-watch poller, BA.0.A).
    pub fn new(poll_secs: u64, registry: FileConfig) -> Self {
        Self {
            conns: HashMap::new(),
            sessions_subs: HashSet::new(),
            pane_subs: HashMap::new(),
            pane_handles: HashMap::new(),
            pane_cursors: HashMap::new(),
            sessions_last_state: HashMap::new(),
            sessions_handle: None,
            poll_secs,
            registry: Arc::new(registry),
            flow_watcher: FlowWatcher::new(),
        }
    }

    /// Deliver `frame` to every connection in `ids`, skipping disconnected ones.
    fn fan_out(&self, ids: &HashSet<ConnId>, frame: WsFrame) {
        for id in ids {
            if let Some(addr) = self.conns.get(id) {
                addr.do_send(ServerFrame(frame.clone()));
            }
        }
    }

    /// Deliver `frame` to **every** connected client, regardless of topic
    /// subscription (BA.0.A). The `workflow_done` push is not
    /// subscription-gated — every `/ws` client receives it.
    fn broadcast_all(&self, frame: WsFrame) {
        for addr in self.conns.values() {
            addr.do_send(ServerFrame(frame.clone()));
        }
    }
}

impl Actor for Hub {
    type Context = Context<Self>;

    /// Start the always-on flow-watch poller (BA.0.A) — unlike the
    /// `sessions`/`pane` pollers, this one is not gated on subscribers: the
    /// `workflow_done` push has no dedicated subscribe topic, so the only
    /// way a client can observe it is for the hub to poll unconditionally
    /// from actor startup.
    fn started(&mut self, ctx: &mut Self::Context) {
        let interval = Duration::from_secs(self.poll_secs);
        ctx.run_interval(interval, |act, ctx| {
            let registry = act.registry.clone();
            // Move the watcher's state into the blocking closure and restore
            // it from the `.then` continuation — `FlowWatcher` isn't `Clone`
            // (its whole point is being the single mutable cursor across
            // cycles), so `mem::take` is how the file-read work moves to the
            // blocking pool without duplicating that state.
            let mut watcher = std::mem::take(&mut act.flow_watcher);
            let fut = web::block(move || {
                let frames = watch_cycle(&registry, &mut watcher);
                (watcher, frames)
            })
            .into_actor(act)
            .then(|result, act, _ctx| {
                // web::block returns Result<Result<T, E>, BlockingError>; here
                // the inner closure is infallible, so only the outer layer
                // can fail (thread-pool panic/shutdown).
                if let Ok((watcher, frames)) = result {
                    act.flow_watcher = watcher;
                    for frame in frames {
                        act.broadcast_all(frame);
                    }
                }
                actix::fut::ready(())
            });
            ctx.spawn(fut);
        });
    }
}

// ── Flow-watch poll cycle (BA.0.A) ────────────────────────────────────────────

/// One flow-watch poll cycle: for every registered workspace, enumerate its
/// `sdlc-flow-state.json` files via [`collect_flow_states`], feed them through
/// `watcher.observe`, and map each resulting [`crate::serve::dto::WorkflowDonePayload`]
/// through [`workflow_done_frame`].
///
/// Thin I/O shell (file reads via `collect_flow_states`) over the pure
/// `FlowWatcher::observe` / `workflow_done_frame` core — no actor messaging
/// happens here, so it is directly unit-testable against a fixture workspace
/// registered in a [`FileConfig`], driven through a shared `FlowWatcher`
/// across calls (Rule 6).
pub(crate) fn watch_cycle(registry: &FileConfig, watcher: &mut FlowWatcher) -> Vec<WsFrame> {
    let Some(workspaces) = registry.workspaces.as_ref() else {
        return Vec::new();
    };

    // Deterministic order (sorted names) so multi-workspace cycles are
    // reproducible in tests, matching `collect_flow_states`'s own ordering
    // convention.
    let mut names: Vec<&String> = workspaces.keys().collect();
    names.sort();

    let mut frames = Vec::new();
    for name in names {
        let root = &workspaces[name];
        let flows = collect_flow_states(root);
        for payload in watcher.observe(name, &flows) {
            frames.push(workflow_done_frame(&payload));
        }
    }
    frames
}

// ── Pure helpers (unit-tested) ────────────────────────────────────────────────

/// First subscriber to a pane → start its poller (`prev_count` is the count
/// *before* this subscribe, i.e. 0 → 1 transition).
pub fn should_start_poll(prev_count: usize) -> bool {
    prev_count == 0
}

/// Last subscriber left a pane → stop its poller (`new_count` is the count
/// *after* the unsubscribe/disconnect, i.e. 1 → 0 transition).
pub fn should_stop_poll(new_count: usize) -> bool {
    new_count == 0
}

// ── Helper: sessions-list poll tick result ────────────────────────────────────

/// Result of one sessions-list poll tick's blocking work: the session DTOs to
/// fan out, plus each session's raw pane capture (name, capture) so the tick's
/// `.then` closure can compute needs-input state (Gap 1) without a second
/// blocking round trip.
type SessionsTickResult = (Vec<crate::serve::dto::SessionDto>, Vec<(String, String)>);

// ── Helper: build a WsFrame from typed payload ────────────────────────────────

fn sessions_frame(sessions: Vec<crate::serve::dto::SessionDto>) -> WsFrame {
    WsFrame {
        kind: WsFrameKind::Sessions,
        payload: serde_json::to_value(SessionsPayload { sessions })
            .unwrap_or(serde_json::Value::Null),
    }
}

fn pane_frame(session: String, seq: u64, lines: Vec<String>) -> WsFrame {
    WsFrame {
        kind: WsFrameKind::Pane,
        payload: serde_json::to_value(PanePayload {
            session,
            seq,
            lines,
        })
        .unwrap_or(serde_json::Value::Null),
    }
}

fn event_needs_input_frame(session: String) -> WsFrame {
    WsFrame {
        kind: WsFrameKind::Event,
        payload: serde_json::to_value(EventPayload {
            session,
            event: "needs_input".to_owned(),
        })
        .unwrap_or(serde_json::Value::Null),
    }
}

// ── Handler: Connect ──────────────────────────────────────────────────────────

impl Handler<Connect> for Hub {
    type Result = ();

    fn handle(&mut self, msg: Connect, _ctx: &mut Context<Self>) {
        self.conns.insert(msg.id, msg.addr);
    }
}

// ── Handler: Disconnect ───────────────────────────────────────────────────────

impl Handler<Disconnect> for Hub {
    type Result = ();

    fn handle(&mut self, msg: Disconnect, ctx: &mut Context<Self>) {
        self.conns.remove(&msg.id);
        self.sessions_subs.remove(&msg.id);

        if self.sessions_subs.is_empty()
            && let Some(handle) = self.sessions_handle.take()
        {
            ctx.cancel_future(handle);
        }

        // Remove from each pane topic; stop poller when last subscriber leaves.
        let pane_names: Vec<String> = self.pane_subs.keys().cloned().collect();
        for name in pane_names {
            if let Some(subs) = self.pane_subs.get_mut(&name) {
                subs.remove(&msg.id);
                if should_stop_poll(subs.len()) {
                    self.pane_subs.remove(&name);
                    if let Some(handle) = self.pane_handles.remove(&name) {
                        ctx.cancel_future(handle);
                    }
                    self.pane_cursors.remove(&name);
                }
            }
        }
    }
}

// ── Handler: Subscribe ────────────────────────────────────────────────────────

impl Handler<Subscribe> for Hub {
    type Result = ();

    fn handle(&mut self, msg: Subscribe, ctx: &mut Context<Self>) {
        match msg.topic {
            Topic::Sessions => {
                self.sessions_subs.insert(msg.id);

                // Start the shared sessions poll on first subscriber.
                if self.sessions_handle.is_none() {
                    let interval = Duration::from_secs(self.poll_secs);
                    let handle = ctx.run_interval(interval, |act, ctx| {
                        if act.sessions_subs.is_empty() {
                            return;
                        }
                        let conns = act
                            .sessions_subs
                            .iter()
                            .filter_map(|id| act.conns.get(id).cloned())
                            .collect::<Vec<_>>();

                        // One blocking closure: list sessions, then capture each
                        // session's pane once so the per-session state feeds both
                        // the needs-input rising-edge check (this task) and the
                        // `last_line` fill-in (Gap 3, task 3).
                        let fut = web::block(|| -> anyhow::Result<SessionsTickResult> {
                            let raw = tmux::list_sessions_raw()?;
                            let sessions = sessions_snapshot(&raw);
                            let panes = sessions
                                .iter()
                                .filter_map(|s| {
                                    tmux::capture_pane_raw(&s.name)
                                        .ok()
                                        .map(|capture| (s.name.clone(), capture))
                                })
                                .collect();
                            Ok((sessions, panes))
                        })
                        .into_actor(act)
                        .then(move |result, act, _ctx| {
                            // web::block returns Result<Result<T, E>, BlockingError>
                            if let Ok(Ok((sessions, panes))) = result {
                                // Fill last_line from the per-session pane
                                // captures already taken above (Gap 3), reusing
                                // the same capture pass as the needs-input
                                // check below rather than capturing twice.
                                let sessions = sessions_with_last_line(sessions, &panes);
                                let frame = sessions_frame(sessions);
                                for addr in &conns {
                                    addr.do_send(ServerFrame(frame.clone()));
                                }

                                // Needs-input rising-edge debounce, scoped to the
                                // sessions subscribers (Gap 1): a session crossing
                                // into Blocked emits even with no pane subscribed.
                                let current: Vec<(String, AgentState)> = panes
                                    .iter()
                                    .map(|(name, capture)| {
                                        (name.clone(), status_detect::detect_state(capture))
                                    })
                                    .collect();
                                let crossing =
                                    sessions_needing_input(&act.sessions_last_state, &current);
                                for name in crossing {
                                    let event_frame = event_needs_input_frame(name);
                                    for addr in &conns {
                                        addr.do_send(ServerFrame(event_frame.clone()));
                                    }
                                }
                                act.sessions_last_state = current.into_iter().collect();
                            }
                            // Ignore tmux errors: best-effort delivery.
                            actix::fut::ready(())
                        });
                        ctx.spawn(fut);
                    });
                    self.sessions_handle = Some(handle);
                }
            }

            Topic::Pane(name) => {
                let prev_count = self.pane_subs.get(&name).map(|s| s.len()).unwrap_or(0);
                self.pane_subs
                    .entry(name.clone())
                    .or_default()
                    .insert(msg.id);

                if should_start_poll(prev_count) {
                    let interval = Duration::from_secs(self.poll_secs);
                    let pane_name = name.clone();
                    let handle = ctx.run_interval(interval, move |act, ctx| {
                        let subs = match act.pane_subs.get(&pane_name) {
                            Some(s) if !s.is_empty() => s
                                .iter()
                                .filter_map(|id| act.conns.get(id).cloned())
                                .collect::<Vec<_>>(),
                            _ => return,
                        };

                        // Clone pane_name for each interval tick so the FnMut
                        // closure can give an owned copy to the async then-closure.
                        let name_for_block = pane_name.clone();
                        let name_for_then = pane_name.clone();
                        let fut = web::block(move || tmux::capture_pane_raw(&name_for_block))
                            .into_actor(act)
                            .then(move |result, act, _ctx| {
                                // web::block returns Result<Result<T, E>, BlockingError>
                                if let Ok(Ok(capture)) = result {
                                    // Pane diff — push only on change. Needs-input
                                    // detection lives in the sessions-list poller
                                    // (Gap 1), not here, so it fires even with no
                                    // pane subscribed.
                                    let cursor =
                                        act.pane_cursors.entry(name_for_then.clone()).or_default();
                                    if let Some((seq, lines)) = cursor.observe(&capture) {
                                        let frame = pane_frame(name_for_then.clone(), seq, lines);
                                        for addr in &subs {
                                            addr.do_send(ServerFrame(frame.clone()));
                                        }
                                    }
                                }
                                actix::fut::ready(())
                            });
                        ctx.spawn(fut);
                    });
                    self.pane_handles.insert(name.clone(), handle);
                }
            }
        }
    }
}

// ── Handler: Unsubscribe ──────────────────────────────────────────────────────

impl Handler<Unsubscribe> for Hub {
    type Result = ();

    fn handle(&mut self, msg: Unsubscribe, ctx: &mut Context<Self>) {
        match msg.topic {
            Topic::Sessions => {
                self.sessions_subs.remove(&msg.id);
                if self.sessions_subs.is_empty()
                    && let Some(handle) = self.sessions_handle.take()
                {
                    ctx.cancel_future(handle);
                }
            }

            Topic::Pane(name) => {
                let new_count = if let Some(subs) = self.pane_subs.get_mut(&name) {
                    subs.remove(&msg.id);
                    subs.len()
                } else {
                    0
                };

                if should_stop_poll(new_count) {
                    self.pane_subs.remove(&name);
                    if let Some(handle) = self.pane_handles.remove(&name) {
                        ctx.cancel_future(handle);
                    }
                    self.pane_cursors.remove(&name);
                }
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serve::dto::WorkflowDonePayload;
    use std::sync::Mutex;

    // ── Tiny TempDir helper (mirrors handlers/status.rs's test fixture) ────

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "bastion-hub-test-{}-{}",
                std::process::id(),
                ConnId::next().0
            ));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write(path: &std::path::Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    fn flow_json(spec_slug: &str, status: &str) -> String {
        format!(
            r#"{{
  "spec_slug": "{spec_slug}",
  "branch": "{spec_slug}-flow",
  "status": "{status}",
  "current_task": 1,
  "started_at": "2026-07-19T00:00:00Z",
  "updated_at": "2026-07-19T00:00:00Z"
}}"#
        )
    }

    fn registry_with(name: &str, root: &std::path::Path) -> FileConfig {
        let mut workspaces = HashMap::new();
        workspaces.insert(name.to_string(), root.to_path_buf());
        FileConfig {
            workspaces: Some(workspaces),
            ..Default::default()
        }
    }

    // ── watch_cycle ──────────────────────────────────────────────────────

    #[test]
    fn watch_cycle_first_observation_emits_no_frames() {
        let tmp = TempDir::new();
        write(
            &tmp.path().join("planning/spec-a/sdlc/sdlc-flow-state.json"),
            &flow_json("spec-a", "running"),
        );
        let registry = registry_with("bastion", tmp.path());
        let mut watcher = FlowWatcher::new();

        let frames = watch_cycle(&registry, &mut watcher);
        assert!(
            frames.is_empty(),
            "first observation must never emit a frame"
        );
    }

    #[test]
    fn watch_cycle_running_to_done_emits_one_frame() {
        let tmp = TempDir::new();
        let flow_path = tmp.path().join("planning/spec-a/sdlc/sdlc-flow-state.json");
        write(&flow_path, &flow_json("spec-a", "running"));
        let registry = registry_with("bastion", tmp.path());
        let mut watcher = FlowWatcher::new();

        // First cycle: no transition yet.
        assert!(watch_cycle(&registry, &mut watcher).is_empty());

        // Second cycle: running → done.
        write(&flow_path, &flow_json("spec-a", "done"));
        let frames = watch_cycle(&registry, &mut watcher);
        assert_eq!(frames.len(), 1, "running→done must emit exactly one frame");
        assert_eq!(frames[0].kind, WsFrameKind::Event);
        assert_eq!(frames[0].payload["event"], "workflow_done");
        assert_eq!(frames[0].payload["repo"], "bastion");
        assert_eq!(frames[0].payload["spec_slug"], "spec-a");
        assert_eq!(frames[0].payload["status"], "done");
        assert_eq!(frames[0].payload["session"], "");
    }

    #[test]
    fn watch_cycle_running_to_blocked_emits_one_frame() {
        let tmp = TempDir::new();
        let flow_path = tmp.path().join("planning/spec-a/sdlc/sdlc-flow-state.json");
        write(&flow_path, &flow_json("spec-a", "running"));
        let registry = registry_with("bastion", tmp.path());
        let mut watcher = FlowWatcher::new();

        assert!(watch_cycle(&registry, &mut watcher).is_empty());

        write(&flow_path, &flow_json("spec-a", "blocked"));
        let frames = watch_cycle(&registry, &mut watcher);
        assert_eq!(
            frames.len(),
            1,
            "running→blocked must emit exactly one frame"
        );
        assert_eq!(frames[0].payload["status"], "blocked");
    }

    #[test]
    fn watch_cycle_unchanged_status_emits_no_frame() {
        let tmp = TempDir::new();
        let flow_path = tmp.path().join("planning/spec-a/sdlc/sdlc-flow-state.json");
        write(&flow_path, &flow_json("spec-a", "running"));
        let registry = registry_with("bastion", tmp.path());
        let mut watcher = FlowWatcher::new();

        assert!(watch_cycle(&registry, &mut watcher).is_empty());
        // Second cycle: still running, no change.
        assert!(watch_cycle(&registry, &mut watcher).is_empty());
    }

    #[test]
    fn watch_cycle_already_terminal_emits_no_further_frame() {
        let tmp = TempDir::new();
        let flow_path = tmp.path().join("planning/spec-a/sdlc/sdlc-flow-state.json");
        write(&flow_path, &flow_json("spec-a", "running"));
        let registry = registry_with("bastion", tmp.path());
        let mut watcher = FlowWatcher::new();

        assert!(watch_cycle(&registry, &mut watcher).is_empty());
        write(&flow_path, &flow_json("spec-a", "done"));
        assert_eq!(watch_cycle(&registry, &mut watcher).len(), 1);

        // Third cycle: still done — already terminal, no further frame.
        let frames = watch_cycle(&registry, &mut watcher);
        assert!(
            frames.is_empty(),
            "an already-terminal status must not re-emit a frame"
        );
    }

    #[test]
    fn watch_cycle_empty_registry_emits_no_frames() {
        let registry = FileConfig::default();
        let mut watcher = FlowWatcher::new();
        assert!(watch_cycle(&registry, &mut watcher).is_empty());
    }

    // ── Hub::broadcast_all (hub-level test) ─────────────────────────────

    /// Records every [`ServerFrame`] it receives — the test double standing
    /// in for a `WsConn` recipient.
    #[derive(Default)]
    struct RecorderActor {
        received: Arc<Mutex<Vec<WsFrame>>>,
    }

    impl Actor for RecorderActor {
        type Context = Context<Self>;
    }

    impl Handler<ServerFrame> for RecorderActor {
        type Result = ();

        fn handle(&mut self, msg: ServerFrame, _ctx: &mut Context<Self>) {
            self.received.lock().unwrap().push(msg.0);
        }
    }

    /// Test-only probe message: round-trips through the recorder's mailbox so
    /// the caller can await it and be sure every earlier `do_send` in the
    /// (single-threaded, FIFO) mailbox has already been processed.
    #[derive(Message)]
    #[rtype(result = "Vec<WsFrame>")]
    struct DrainReceived;

    impl Handler<DrainReceived> for RecorderActor {
        type Result = Vec<WsFrame>;

        fn handle(&mut self, _msg: DrainReceived, _ctx: &mut Context<Self>) -> Vec<WsFrame> {
            self.received.lock().unwrap().clone()
        }
    }

    #[actix_web::test]
    async fn broadcast_all_delivers_to_a_connection_with_no_topic_subscription() {
        let recorder = RecorderActor::default().start();

        let mut hub = Hub::new(2, FileConfig::default());
        let id = ConnId::next();
        // Connect the recorder without subscribing it to `sessions` or any
        // `pane` topic — the workflow_done push must still reach it.
        hub.conns.insert(id, recorder.clone().recipient());

        let payload = WorkflowDonePayload {
            repo: "bastion".to_string(),
            spec_slug: "spec-a".to_string(),
            status: "done".to_string(),
        };
        let frame = workflow_done_frame(&payload);
        hub.broadcast_all(frame.clone());

        let received = recorder.send(DrainReceived).await.unwrap();
        assert_eq!(
            received.len(),
            1,
            "an unsubscribed connection must still receive the broadcast frame"
        );
        assert_eq!(received[0], frame);
    }

    // ── should_start_poll ──────────────────────────────────────────────────

    #[test]
    fn should_start_poll_true_when_prev_count_is_zero() {
        assert!(
            should_start_poll(0),
            "first subscriber (count 0→1) must start the poller"
        );
    }

    #[test]
    fn should_start_poll_false_when_prev_count_is_nonzero() {
        assert!(
            !should_start_poll(1),
            "second subscriber (count 1→2) must not start a new poller"
        );
        assert!(
            !should_start_poll(5),
            "nth subscriber must not start a new poller"
        );
    }

    // ── should_stop_poll ───────────────────────────────────────────────────

    #[test]
    fn should_stop_poll_true_when_new_count_is_zero() {
        assert!(
            should_stop_poll(0),
            "last subscriber left (count 1→0) must stop the poller"
        );
    }

    #[test]
    fn should_stop_poll_false_when_new_count_is_nonzero() {
        assert!(
            !should_stop_poll(1),
            "still one subscriber remaining — must not stop the poller"
        );
        assert!(
            !should_stop_poll(2),
            "multiple remaining subscribers — must not stop the poller"
        );
    }

    // Note: `should_emit_needs_input` / `sessions_needing_input` rising-edge
    // coverage now lives in `crate::serve::poll`'s test module — that's the
    // pure-logic home per Rule 6 (these helpers moved there in Gap 1 so the
    // sessions-list poller and this module could both depend on them without
    // a cycle).

    // ── ConnId::next ───────────────────────────────────────────────────────

    #[test]
    fn conn_id_next_returns_strictly_increasing_ids() {
        let a = ConnId::next();
        let b = ConnId::next();
        assert!(
            b.0 > a.0,
            "ConnId::next must return strictly increasing ids; got a={} b={}",
            a.0,
            b.0
        );
    }

    #[test]
    fn conn_id_next_ids_are_distinct() {
        let ids: Vec<ConnId> = (0..10).map(|_| ConnId::next()).collect();
        let unique: HashSet<u64> = ids.iter().map(|c| c.0).collect();
        assert_eq!(
            ids.len(),
            unique.len(),
            "all ConnId::next() calls must produce distinct ids"
        );
    }
}
