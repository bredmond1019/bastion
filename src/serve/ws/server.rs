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
//! - Consumes `event{needs_input}` crossings from the always-on
//!   [`crate::serve::blocked_edge::BlockedEdgePoller`] (BA.18.A task 4) via
//!   [`BlockedEdgeCrossed`], instead of computing the rising edge itself.
//!   The hub owns no previous-state map of its own any more: the poller runs
//!   from server boot, independent of subscribers, and is the sole place the
//!   needs-input transition is decided. The hub's only job here is fan-out to
//!   whichever `sessions` subscribers are connected *right now* — so an
//!   unsubscribe/resubscribe cycle can never replay a crossing the hub itself
//!   never stored.
//!
//! # Pure helpers (unit-tested, Rule 6)
//! - [`ConnId`] + monotonic counter
//! - [`should_start_poll`] / [`should_stop_poll`] — poller lifecycle decisions
//! - [`crate::serve::poll::should_emit_needs_input`] /
//!   [`crate::serve::poll::sessions_needing_input`] — rising-edge debounce,
//!   now driven exclusively by [`crate::serve::blocked_edge::BlockedEdgePoller`]
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
use engine_contract::task_context::TaskContext;
use engine_serve::live_state::LiveStateStore;
use uuid::Uuid;

use crate::config::FileConfig;
use crate::serve::dto::{EventPayload, PanePayload, SessionsPayload, Topic, WsFrame, WsFrameKind};
use crate::serve::handlers::status::collect_flow_states;
use crate::serve::poll::{
    FlowWatcher, PaneCursor, RunWatcher, SharedSessionsSweep, run_stream_status_frame,
    run_transition_frame, sessions_snapshot, sessions_with_last_line, workflow_done_frame,
};
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

/// One or more sessions just crossed into `Blocked` this poll tick, as
/// decided by the always-on [`crate::serve::blocked_edge::BlockedEdgePoller`]
/// (BA.18.A task 3) — the sole owner of the needs-input rising-edge
/// computation. The hub is purely a consumer: on receipt it fans an
/// `event{needs_input}` frame out to whichever `sessions` subscribers are
/// connected right now (BA.18.A task 4). Sent unconditionally, subscriber or
/// not — [`Handler<BlockedEdgeCrossed>`] is a no-op when `sessions_subs` is
/// empty, which is exactly what "consumer, not owner" means: the poller never
/// needs to know whether anyone is listening.
#[derive(Message)]
#[rtype(result = "()")]
pub struct BlockedEdgeCrossed {
    pub sessions: Vec<String>,
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
    /// Subscribers to the `runs` topic (BA.11.N) — a run-level aggregate
    /// status push, subscription-gated (unlike the always-on flow-watch
    /// poller above).
    runs_subs: HashSet<ConnId>,
    /// Handle for the single shared runs-poll interval, started on first
    /// `runs` subscriber and stopped on last (mirrors `sessions_handle`).
    runs_handle: Option<SpawnHandle>,
    /// Stateful `LiveStateStore` run-status diff tracker — shared across
    /// poll cycles so a status is only pushed on real transition.
    run_watcher: RunWatcher,
    /// Clone of the process's `LiveStateStore` (D17) — the hub reads this
    /// in-memory on every runs-poll cycle; no `web::block` needed since
    /// reads never touch disk or the network.
    live: LiveStateStore,
    /// Whether the engine is mounted (so `LiveStateStore` is ever written),
    /// plus the reason when it is not — pushed to every new `runs`
    /// subscriber as a `run_stream_status` frame (D17 constraint 2).
    stream_available: (bool, Option<String>),
    /// The always-on `BlockedEdgePoller`'s shared sweep (BA.18.A review
    /// fix), when wired via [`Self::with_shared_sessions`]. When `Some`,
    /// the `sessions` topic poll reads from it instead of running its own
    /// independent tmux sweep, so a subscriber never doubles the tmux
    /// subprocess count per interval. `None` in every test in this module
    /// and whenever the poller could not start — the hub falls back to its
    /// own sweep so `sessions` delivery still works.
    shared_sessions: Option<SharedSessionsSweep>,
}

impl Hub {
    /// Create a new hub with the given poll interval, workspace registry
    /// (drives the always-on flow-watch poller, BA.0.A), live run-state
    /// store, and engine-mount availability verdict (both drive the `runs`
    /// topic, BA.11.N).
    pub fn new(
        poll_secs: u64,
        registry: FileConfig,
        live: LiveStateStore,
        stream_available: (bool, Option<String>),
    ) -> Self {
        Self {
            conns: HashMap::new(),
            sessions_subs: HashSet::new(),
            pane_subs: HashMap::new(),
            pane_handles: HashMap::new(),
            pane_cursors: HashMap::new(),
            sessions_handle: None,
            poll_secs,
            registry: Arc::new(registry),
            flow_watcher: FlowWatcher::new(),
            runs_subs: HashSet::new(),
            runs_handle: None,
            run_watcher: RunWatcher::new(),
            live,
            stream_available,
            shared_sessions: None,
        }
    }

    /// Wire the hub to read the always-on `BlockedEdgePoller`'s shared sweep
    /// on the `sessions` topic poll instead of running its own independent
    /// tmux sweep (BA.18.A review fix — folds the two 1+S sweeps that ran
    /// on the same cadence into one). Production wiring only
    /// (`src/serve/mod.rs::run_server`); every test in this module leaves
    /// this unset and exercises the fallback sweep.
    pub fn with_shared_sessions(mut self, shared: SharedSessionsSweep) -> Self {
        self.shared_sessions = Some(shared);
        self
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

// ── Runs poll cycle (BA.11.N) ─────────────────────────────────────────────────

/// One `runs`-topic poll cycle: read `live`'s current live-run set and feed
/// it through `watcher.observe`, mapping each resulting
/// [`crate::serve::dto::RunTransitionPayload`] through [`run_transition_frame`].
///
/// Thin I/O shell over the pure `RunWatcher::observe` / `run_transition_frame`
/// core (Task 2) — the only "I/O" here is `LiveStateStore`'s in-memory reads
/// (`list_active` / `get` / `get_record`), which is why this is called
/// directly from the actor's interval closure rather than offloaded via
/// `web::block` the way `watch_cycle`'s file reads are: there is no blocking
/// disk or network work to move off the actor's thread. Directly
/// unit-testable against a hand-seeded `LiveStateStore` with no actor
/// involved, mirroring `watch_cycle` (Rule 6).
pub(crate) fn run_watch_cycle(live: &LiveStateStore, watcher: &mut RunWatcher) -> Vec<WsFrame> {
    let live_pairs: Vec<(Uuid, TaskContext)> = live
        .list_active()
        .into_iter()
        .filter_map(|id| live.get(id).map(|ctx| (id, ctx)))
        .collect();

    watcher
        .observe(&live_pairs, |id| live.get_record(id).map(|r| r.snapshot))
        .iter()
        .map(run_transition_frame)
        .collect()
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

// ── Handler: BlockedEdgeCrossed ─────────────────────────────────────────────

impl Handler<BlockedEdgeCrossed> for Hub {
    type Result = ();

    /// Fan the crossing sessions out to current `sessions` subscribers only.
    /// No state is read or written here beyond `self.sessions_subs`/`conns`
    /// — the hub keeps no previous-state map of its own (task 4), so there is
    /// nothing to replay across an unsubscribe/resubscribe cycle.
    fn handle(&mut self, msg: BlockedEdgeCrossed, _ctx: &mut Context<Self>) {
        if self.sessions_subs.is_empty() {
            return;
        }
        for name in msg.sessions {
            self.fan_out(&self.sessions_subs, event_needs_input_frame(name));
        }
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

        // BA.11.N: a raw disconnect must release a `runs` subscription too —
        // not just an explicit `unsubscribe` — mirroring the sessions arm
        // immediately above.
        self.runs_subs.remove(&msg.id);
        if self.runs_subs.is_empty()
            && let Some(handle) = self.runs_handle.take()
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
                    let shared_sessions = self.shared_sessions.clone();
                    let handle = ctx.run_interval(interval, move |act, ctx| {
                        if act.sessions_subs.is_empty() {
                            return;
                        }
                        let conns = act
                            .sessions_subs
                            .iter()
                            .filter_map(|id| act.conns.get(id).cloned())
                            .collect::<Vec<_>>();

                        // BA.18.A review fix: when the always-on
                        // `BlockedEdgePoller` is wired in (production), read its
                        // shared sweep instead of running a second, independent
                        // `list_sessions_raw` + per-session `capture_pane_raw`
                        // sweep on the same cadence — that used to double the
                        // tmux subprocess count per interval whenever a
                        // `sessions` subscriber was present.
                        if let Some(shared) = &shared_sessions {
                            let snapshot = shared.lock().ok().and_then(|guard| guard.clone());
                            if let Some((sessions, panes)) = snapshot {
                                let sessions = sessions_with_last_line(sessions, &panes);
                                let frame = sessions_frame(sessions);
                                for addr in &conns {
                                    addr.do_send(ServerFrame(frame.clone()));
                                }
                            }
                            // No sweep yet (poller hasn't ticked): skip this
                            // cycle rather than falling back to a second sweep.
                            // Needs-input rising-edge crossings are not computed
                            // here either way — they arrive via
                            // `Handler<BlockedEdgeCrossed>` from the poller.
                            return;
                        }

                        // Fallback: no shared poller wired (e.g. it failed to
                        // start because neither `XDG_STATE_HOME` nor `HOME` is
                        // set) — run the hub's own sweep so `sessions` delivery
                        // still works.
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
                        .then(move |result, _act, _ctx| {
                            // web::block returns Result<Result<T, E>, BlockingError>
                            if let Ok(Ok((sessions, panes))) = result {
                                let sessions = sessions_with_last_line(sessions, &panes);
                                let frame = sessions_frame(sessions);
                                for addr in &conns {
                                    addr.do_send(ServerFrame(frame.clone()));
                                }
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

            Topic::Runs => {
                let prev_count = self.runs_subs.len();
                self.runs_subs.insert(msg.id);

                // D17 constraint 2: the new subscriber learns availability
                // at subscribe time, not by inferring it from silence — push
                // immediately, to this one connection only, regardless of
                // poller start/stop below.
                if let Some(addr) = self.conns.get(&msg.id) {
                    let (available, reason) = &self.stream_available;
                    let frame = run_stream_status_frame(*available, reason.as_deref());
                    addr.do_send(ServerFrame(frame));
                }

                // Start the shared runs-poll on first subscriber. Unlike the
                // sessions/pane pollers, this reads `LiveStateStore`
                // in-memory only, so the cycle runs directly in the interval
                // closure — no `web::block` offload is needed (see
                // `run_watch_cycle`'s doc comment).
                if should_start_poll(prev_count) {
                    let interval = Duration::from_secs(self.poll_secs);
                    let handle = ctx.run_interval(interval, |act, _ctx| {
                        if act.runs_subs.is_empty() {
                            return;
                        }
                        let frames = run_watch_cycle(&act.live, &mut act.run_watcher);
                        if frames.is_empty() {
                            return;
                        }
                        let subs = act.runs_subs.clone();
                        for frame in frames {
                            act.fan_out(&subs, frame);
                        }
                    });
                    self.runs_handle = Some(handle);
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

            Topic::Runs => {
                self.runs_subs.remove(&msg.id);
                if should_stop_poll(self.runs_subs.len())
                    && let Some(handle) = self.runs_handle.take()
                {
                    ctx.cancel_future(handle);
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

    // ── run_watch_cycle (BA.11.N) ────────────────────────────────────────

    use chrono::Utc;
    use engine_contract::task_context::{NodeRun, NodeRunStatus};

    fn node_run_fixture(status: NodeRunStatus) -> NodeRun {
        NodeRun {
            status,
            started_at: None,
            completed_at: None,
            error: None,
            input: None,
            usage: None,
        }
    }

    /// Build a `TaskContext` with one node `"NodeA"` at `status` and empty
    /// event/metadata — the plain node-aggregate path, mirroring
    /// `crate::serve::poll`'s own test fixture.
    fn run_ctx(status: NodeRunStatus) -> TaskContext {
        let mut node_runs = HashMap::new();
        node_runs.insert("NodeA".to_owned(), node_run_fixture(status));
        TaskContext {
            event: serde_json::json!({}),
            nodes: HashMap::new(),
            metadata: serde_json::json!({}),
            node_runs,
        }
    }

    #[test]
    fn run_watch_cycle_first_observation_emits_no_frames() {
        let live = LiveStateStore::new();
        let run_id = Uuid::new_v4();
        live.record(run_id, &run_ctx(NodeRunStatus::Pending));
        let mut watcher = RunWatcher::new();

        let frames = run_watch_cycle(&live, &mut watcher);
        assert!(
            frames.is_empty(),
            "first observation must never emit a frame"
        );
    }

    #[test]
    fn run_watch_cycle_status_change_emits_one_frame() {
        let live = LiveStateStore::new();
        let run_id = Uuid::new_v4();
        live.record(run_id, &run_ctx(NodeRunStatus::Pending));
        let mut watcher = RunWatcher::new();
        assert!(run_watch_cycle(&live, &mut watcher).is_empty());

        live.record(run_id, &run_ctx(NodeRunStatus::Running));
        let frames = run_watch_cycle(&live, &mut watcher);
        assert_eq!(
            frames.len(),
            1,
            "pending→running must emit exactly one frame"
        );
        assert_eq!(frames[0].kind, WsFrameKind::Event);
        assert_eq!(frames[0].payload["event"], "run_transition");
        assert_eq!(frames[0].payload["run_id"], run_id.to_string());
        assert_eq!(frames[0].payload["status"], "running");
        assert_eq!(frames[0].payload["terminal"], false);
    }

    #[test]
    fn run_watch_cycle_unchanged_status_emits_no_frame() {
        let live = LiveStateStore::new();
        let run_id = Uuid::new_v4();
        live.record(run_id, &run_ctx(NodeRunStatus::Running));
        let mut watcher = RunWatcher::new();
        assert!(run_watch_cycle(&live, &mut watcher).is_empty());

        // Second cycle: still running, no change.
        live.record(run_id, &run_ctx(NodeRunStatus::Running));
        assert!(run_watch_cycle(&live, &mut watcher).is_empty());
    }

    #[test]
    fn run_watch_cycle_disappearance_with_record_emits_terminal_frame() {
        let live = LiveStateStore::new();
        let run_id = Uuid::new_v4();
        live.record(run_id, &run_ctx(NodeRunStatus::Running));
        let mut watcher = RunWatcher::new();
        assert!(run_watch_cycle(&live, &mut watcher).is_empty());

        // The run goes lifecycle-terminal: `mark_terminal` moves it out of
        // the live map and into the completed ring with its final status.
        let final_ctx = run_ctx(NodeRunStatus::Success);
        live.mark_terminal(run_id, &final_ctx, "SDLC_FLOW", Utc::now(), Utc::now());

        let frames = run_watch_cycle(&live, &mut watcher);
        assert_eq!(
            frames.len(),
            1,
            "disappearance from list_active must emit exactly one frame"
        );
        assert_eq!(frames[0].payload["event"], "run_transition");
        assert_eq!(frames[0].payload["run_id"], run_id.to_string());
        assert_eq!(
            frames[0].payload["status"], "success",
            "final status must be read back via get_record, not the last live snapshot"
        );
        assert_eq!(frames[0].payload["terminal"], true);

        // The id is removed from the cursor map, so a further cycle emits
        // nothing further for it.
        assert!(run_watch_cycle(&live, &mut watcher).is_empty());
    }

    #[test]
    fn run_watch_cycle_suspended_run_emits_terminal_false() {
        // D17 constraint 1: a suspended run stays in `list_active()` (it is
        // not lifecycle-terminal), so it must flow through the status-change
        // edge with `terminal: false`, never the disappearance edge.
        let live = LiveStateStore::new();
        let run_id = Uuid::new_v4();
        live.record(run_id, &run_ctx(NodeRunStatus::Running));
        let mut watcher = RunWatcher::new();
        assert!(run_watch_cycle(&live, &mut watcher).is_empty());

        let mut suspended = run_ctx(NodeRunStatus::Pending);
        suspended.metadata = serde_json::json!({ "suspension": { "suspended": true } });
        live.record(run_id, &suspended);

        let frames = run_watch_cycle(&live, &mut watcher);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].payload["status"], "suspended");
        assert_eq!(
            frames[0].payload["terminal"], false,
            "a suspended run is still live — terminal must be false (D17 constraint 1)"
        );
    }

    #[test]
    fn run_watch_cycle_empty_store_emits_no_frames() {
        let live = LiveStateStore::new();
        let mut watcher = RunWatcher::new();
        assert!(run_watch_cycle(&live, &mut watcher).is_empty());
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

        let mut hub = Hub::new(
            2,
            FileConfig::default(),
            LiveStateStore::new(),
            (true, None),
        );
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

    // ── `runs` topic subscribe/unsubscribe/disconnect (BA.11.N) ────────────

    #[test]
    fn hub_new_starts_with_no_runs_poller() {
        // No subscriber yet — the shared poller must not be running.
        let hub = Hub::new(
            2,
            FileConfig::default(),
            LiveStateStore::new(),
            (true, None),
        );
        assert!(hub.runs_handle.is_none());
        assert!(hub.runs_subs.is_empty());
    }

    #[actix_web::test]
    async fn runs_subscribe_pushes_run_stream_status_available_true() {
        let hub = Hub::new(
            2,
            FileConfig::default(),
            LiveStateStore::new(),
            (true, None),
        )
        .start();
        let recorder = RecorderActor::default().start();
        let id = ConnId::next();

        hub.send(Connect {
            id,
            addr: recorder.clone().recipient(),
        })
        .await
        .unwrap();
        hub.send(Subscribe {
            id,
            topic: Topic::Runs,
        })
        .await
        .unwrap();

        let received = recorder.send(DrainReceived).await.unwrap();
        assert_eq!(
            received.len(),
            1,
            "subscribing to `runs` must immediately push exactly one run_stream_status frame"
        );
        assert_eq!(received[0].kind, WsFrameKind::Event);
        assert_eq!(received[0].payload["event"], "run_stream_status");
        assert_eq!(received[0].payload["available"], true);
        assert!(
            received[0].payload.get("reason").is_none(),
            "reason must be omitted when available"
        );
    }

    #[actix_web::test]
    async fn runs_subscribe_pushes_run_stream_status_unavailable_with_reason() {
        let hub = Hub::new(
            2,
            FileConfig::default(),
            LiveStateStore::new(),
            (false, Some("DATABASE_URL not set".to_owned())),
        )
        .start();
        let recorder = RecorderActor::default().start();
        let id = ConnId::next();

        hub.send(Connect {
            id,
            addr: recorder.clone().recipient(),
        })
        .await
        .unwrap();
        hub.send(Subscribe {
            id,
            topic: Topic::Runs,
        })
        .await
        .unwrap();

        let received = recorder.send(DrainReceived).await.unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].payload["available"], false);
        assert_eq!(received[0].payload["reason"], "DATABASE_URL not set");
    }

    #[actix_web::test]
    async fn runs_subscribe_status_frame_reaches_only_the_subscriber() {
        let hub = Hub::new(
            2,
            FileConfig::default(),
            LiveStateStore::new(),
            (true, None),
        )
        .start();
        let subscriber = RecorderActor::default().start();
        let bystander = RecorderActor::default().start();
        let sub_id = ConnId::next();
        let bystander_id = ConnId::next();

        hub.send(Connect {
            id: sub_id,
            addr: subscriber.clone().recipient(),
        })
        .await
        .unwrap();
        hub.send(Connect {
            id: bystander_id,
            addr: bystander.clone().recipient(),
        })
        .await
        .unwrap();
        hub.send(Subscribe {
            id: sub_id,
            topic: Topic::Runs,
        })
        .await
        .unwrap();

        let sub_received = subscriber.send(DrainReceived).await.unwrap();
        let bystander_received = bystander.send(DrainReceived).await.unwrap();
        assert_eq!(sub_received.len(), 1);
        assert!(
            bystander_received.is_empty(),
            "a connection not subscribed to `runs` must not receive the run_stream_status frame"
        );
    }

    #[actix_web::test]
    async fn runs_unsubscribe_and_disconnect_both_release_the_subscription() {
        let hub = Hub::new(
            2,
            FileConfig::default(),
            LiveStateStore::new(),
            (true, None),
        )
        .start();
        let recorder_a = RecorderActor::default().start();
        let recorder_b = RecorderActor::default().start();
        let id_a = ConnId::next();
        let id_b = ConnId::next();

        hub.send(Connect {
            id: id_a,
            addr: recorder_a.clone().recipient(),
        })
        .await
        .unwrap();
        hub.send(Connect {
            id: id_b,
            addr: recorder_b.clone().recipient(),
        })
        .await
        .unwrap();
        hub.send(Subscribe {
            id: id_a,
            topic: Topic::Runs,
        })
        .await
        .unwrap();
        hub.send(Subscribe {
            id: id_b,
            topic: Topic::Runs,
        })
        .await
        .unwrap();

        // Explicit unsubscribe releases id_a; a raw disconnect releases id_b.
        hub.send(Unsubscribe {
            id: id_a,
            topic: Topic::Runs,
        })
        .await
        .unwrap();
        hub.send(Disconnect { id: id_b }).await.unwrap();

        // Both connections had already received their one-time
        // run_stream_status frame; no further frames were pushed by
        // unsubscribe/disconnect themselves.
        let received_a = recorder_a.send(DrainReceived).await.unwrap();
        let received_b = recorder_b.send(DrainReceived).await.unwrap();
        assert_eq!(received_a.len(), 1);
        assert_eq!(received_b.len(), 1);
    }

    // ── Handler<BlockedEdgeCrossed> (BA.18.A task 4) ─────────────────────────
    //
    // The hub is a *consumer* of the always-on `BlockedEdgePoller`'s edge
    // decision, not the owner: these tests exercise only the fan-out — that
    // a `sessions` subscriber receives exactly the `needs_input` frames the
    // message names, that a non-subscriber does not, and that sending with
    // zero subscribers is a harmless no-op — never the rising-edge decision
    // itself (that predicate's coverage lives in `crate::serve::poll` and
    // `blocked_edge::poller`, unchanged by this task).

    #[actix_web::test]
    async fn blocked_edge_crossed_reaches_a_sessions_subscriber() {
        let hub = Hub::new(
            2,
            FileConfig::default(),
            LiveStateStore::new(),
            (true, None),
        )
        .start();
        let recorder = RecorderActor::default().start();
        let id = ConnId::next();

        hub.send(Connect {
            id,
            addr: recorder.clone().recipient(),
        })
        .await
        .unwrap();
        hub.send(Subscribe {
            id,
            topic: Topic::Sessions,
        })
        .await
        .unwrap();

        hub.send(BlockedEdgeCrossed {
            sessions: vec!["sess-a".to_owned(), "sess-b".to_owned()],
        })
        .await
        .unwrap();

        let received = recorder.send(DrainReceived).await.unwrap();
        assert_eq!(
            received.len(),
            2,
            "one needs_input frame per crossed session"
        );
        for frame in &received {
            assert_eq!(frame.kind, WsFrameKind::Event);
            assert_eq!(frame.payload["event"], "needs_input");
        }
        let sessions: HashSet<String> = received
            .iter()
            .map(|f| f.payload["session"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            sessions,
            HashSet::from(["sess-a".to_owned(), "sess-b".to_owned()])
        );
    }

    /// BA.18.A review fix: when the hub is wired with
    /// [`Hub::with_shared_sessions`], the `sessions` topic poll must read
    /// the pre-populated shared sweep rather than performing its own
    /// independent tmux sweep — this is the "fold the two 1+S sweeps into
    /// one" fix. Proven here by pre-populating the shared snapshot with a
    /// session name no real tmux server on this test host will ever have,
    /// then asserting the delivered `sessions` frame's content came from
    /// that snapshot (a real independent sweep would either error or
    /// return a completely different session set).
    #[actix_web::test]
    async fn sessions_subscribe_with_shared_sessions_reads_shared_sweep() {
        let shared: crate::serve::poll::SharedSessionsSweep = Arc::new(Mutex::new(Some((
            vec![crate::serve::dto::SessionDto {
                name: "ba18a-shared-fixture-session".to_owned(),
                state: "idle".to_owned(),
                last_line: String::new(),
            }],
            vec![(
                "ba18a-shared-fixture-session".to_owned(),
                "hello from shared sweep\n".to_owned(),
            )],
        ))));

        let hub = Hub::new(
            1,
            FileConfig::default(),
            LiveStateStore::new(),
            (true, None),
        )
        .with_shared_sessions(shared)
        .start();
        let recorder = RecorderActor::default().start();
        let id = ConnId::next();

        hub.send(Connect {
            id,
            addr: recorder.clone().recipient(),
        })
        .await
        .unwrap();
        hub.send(Subscribe {
            id,
            topic: Topic::Sessions,
        })
        .await
        .unwrap();

        // Wait for the 1s sessions-poll interval to tick at least once.
        actix_web::rt::time::sleep(Duration::from_millis(1200)).await;

        let received = recorder.send(DrainReceived).await.unwrap();
        let sessions_frames: Vec<&WsFrame> = received
            .iter()
            .filter(|f| f.kind == WsFrameKind::Sessions)
            .collect();
        assert_eq!(
            sessions_frames.len(),
            1,
            "exactly one sessions frame, built from the shared sweep, not a second independent sweep"
        );
        let payload = &sessions_frames[0].payload;
        assert_eq!(
            payload["sessions"][0]["name"], "ba18a-shared-fixture-session",
            "frame content must come from the pre-populated shared sweep"
        );
        assert_eq!(
            payload["sessions"][0]["last_line"], "hello from shared sweep",
            "last_line must be filled from the shared sweep's panes, not a fresh tmux capture"
        );
    }

    #[actix_web::test]
    async fn blocked_edge_crossed_does_not_reach_a_non_sessions_subscriber() {
        let hub = Hub::new(
            2,
            FileConfig::default(),
            LiveStateStore::new(),
            (true, None),
        )
        .start();
        let bystander = RecorderActor::default().start();
        let id = ConnId::next();

        hub.send(Connect {
            id,
            addr: bystander.clone().recipient(),
        })
        .await
        .unwrap();
        hub.send(Subscribe {
            id,
            topic: Topic::Runs,
        })
        .await
        .unwrap();

        hub.send(BlockedEdgeCrossed {
            sessions: vec!["sess-a".to_owned()],
        })
        .await
        .unwrap();

        let received = bystander.send(DrainReceived).await.unwrap();
        assert!(
            received
                .iter()
                .all(|f| f.payload.get("event") != Some(&serde_json::json!("needs_input"))),
            "a connection subscribed only to `runs` must not receive needs_input frames"
        );
    }

    #[actix_web::test]
    async fn blocked_edge_crossed_with_zero_subscribers_is_a_noop() {
        let hub = Hub::new(
            2,
            FileConfig::default(),
            LiveStateStore::new(),
            (true, None),
        )
        .start();

        // No Connect, no Subscribe — this must not panic or hang even with
        // nobody registered at all, matching the poller's own "nobody is
        // watching" case (the whole point of BA.18.A).
        hub.send(BlockedEdgeCrossed {
            sessions: vec!["sess-a".to_owned()],
        })
        .await
        .unwrap();
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
