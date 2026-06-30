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
//! - Drives the needs-input rising-edge debounce via [`should_emit_needs_input`]
//!   to emit `event{needs_input}` only on the Blocked transition, not every
//!   poll tick while blocked.
//!
//! # Pure helpers (unit-tested, Rule 6)
//! - [`ConnId`] + monotonic counter
//! - [`should_start_poll`] / [`should_stop_poll`] — poller lifecycle decisions
//! - [`should_emit_needs_input`] — rising-edge debounce
//!
//! # I/O shell (smoke-tested, Rule 6)
//! - `Handler<Connect>` / `Handler<Disconnect>` — connection lifecycle
//! - `Handler<Subscribe>` / `Handler<Unsubscribe>` — topic management + poller
//!   start/stop (blocking tmux calls offloaded via `web::block`)

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use actix::SpawnHandle;
use actix::prelude::*;
use actix_web::web;

use crate::detect::AgentState;
use crate::serve::dto::{EventPayload, PanePayload, SessionsPayload, Topic, WsFrame, WsFrameKind};
use crate::serve::poll::{PaneCursor, sessions_snapshot};
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
    /// Last agent state seen per pane, for needs-input rising-edge debounce.
    pane_last_state: HashMap<String, AgentState>,
    /// Handle for the single shared sessions-list interval.
    sessions_handle: Option<SpawnHandle>,
    /// Poll cadence in seconds.
    poll_secs: u64,
}

impl Hub {
    /// Create a new hub with the given poll interval.
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

    /// Deliver `frame` to every connection in `ids`, skipping disconnected ones.
    fn fan_out(&self, ids: &HashSet<ConnId>, frame: WsFrame) {
        for id in ids {
            if let Some(addr) = self.conns.get(id) {
                addr.do_send(ServerFrame(frame.clone()));
            }
        }
    }
}

impl Actor for Hub {
    type Context = Context<Self>;
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

/// Needs-input rising edge: emit `event{needs_input}` only on the transition
/// INTO `Blocked`, not on every poll while still blocked.
pub fn should_emit_needs_input(prev: Option<AgentState>, new: AgentState) -> bool {
    new == AgentState::Blocked && prev != Some(AgentState::Blocked)
}

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
                    self.pane_last_state.remove(&name);
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
                        let subs = act.sessions_subs.clone();
                        let conns = act
                            .sessions_subs
                            .iter()
                            .filter_map(|id| act.conns.get(id).cloned())
                            .collect::<Vec<_>>();

                        let fut = web::block(tmux::list_sessions_raw).into_actor(act).then(
                            move |result, _act, _ctx| {
                                // web::block returns Result<Result<T, E>, BlockingError>
                                if let Ok(Ok(raw)) = result {
                                    let sessions = sessions_snapshot(&raw);
                                    let frame = sessions_frame(sessions);
                                    for addr in &conns {
                                        addr.do_send(ServerFrame(frame.clone()));
                                    }
                                }
                                // Ignore tmux errors: best-effort delivery.
                                let _ = subs; // keep alive
                                actix::fut::ready(())
                            },
                        );
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
                                    // Pane diff — push only on change.
                                    let cursor =
                                        act.pane_cursors.entry(name_for_then.clone()).or_default();
                                    if let Some((seq, lines)) = cursor.observe(&capture) {
                                        let frame = pane_frame(name_for_then.clone(), seq, lines);
                                        for addr in &subs {
                                            addr.do_send(ServerFrame(frame.clone()));
                                        }
                                    }

                                    // Needs-input rising-edge debounce.
                                    let state = status_detect::detect_state(&capture);
                                    let prev = act.pane_last_state.get(&name_for_then).copied();
                                    if should_emit_needs_input(prev, state) {
                                        let event_frame =
                                            event_needs_input_frame(name_for_then.clone());
                                        for addr in &subs {
                                            addr.do_send(ServerFrame(event_frame.clone()));
                                        }
                                    }
                                    act.pane_last_state.insert(name_for_then.clone(), state);
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
                    self.pane_last_state.remove(&name);
                }
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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

    // ── should_emit_needs_input ────────────────────────────────────────────

    #[test]
    fn emit_needs_input_on_transition_from_none_to_blocked() {
        // No previous state + Blocked → rising edge, must emit.
        assert!(
            should_emit_needs_input(None, AgentState::Blocked),
            "first observation of Blocked (no prior state) must emit"
        );
    }

    #[test]
    fn emit_needs_input_on_transition_from_working_to_blocked() {
        assert!(
            should_emit_needs_input(Some(AgentState::Working), AgentState::Blocked),
            "Working→Blocked transition must emit"
        );
    }

    #[test]
    fn emit_needs_input_on_transition_from_idle_to_blocked() {
        assert!(
            should_emit_needs_input(Some(AgentState::Idle), AgentState::Blocked),
            "Idle→Blocked transition must emit"
        );
    }

    #[test]
    fn no_emit_when_already_blocked() {
        // Already Blocked last tick → no emission (suppress repeated events).
        assert!(
            !should_emit_needs_input(Some(AgentState::Blocked), AgentState::Blocked),
            "Blocked→Blocked (no transition) must NOT emit"
        );
    }

    #[test]
    fn no_emit_when_transitioning_away_from_blocked() {
        assert!(
            !should_emit_needs_input(Some(AgentState::Blocked), AgentState::Working),
            "Blocked→Working must NOT emit"
        );
        assert!(
            !should_emit_needs_input(Some(AgentState::Blocked), AgentState::Idle),
            "Blocked→Idle must NOT emit"
        );
    }

    #[test]
    fn no_emit_for_non_blocked_states() {
        assert!(
            !should_emit_needs_input(None, AgentState::Idle),
            "None→Idle must NOT emit"
        );
        assert!(
            !should_emit_needs_input(Some(AgentState::Idle), AgentState::Working),
            "Idle→Working must NOT emit"
        );
        assert!(
            !should_emit_needs_input(Some(AgentState::Idle), AgentState::Idle),
            "Idle→Idle must NOT emit"
        );
    }

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
