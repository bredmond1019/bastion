//! Pure fan-out core for pane-diff tracking and session-list snapshots.
//!
//! This module contains only **pure logic** — no tmux I/O, no process spawning,
//! no actor messaging. The I/O wiring (poll intervals, actor dispatch) lives in
//! Task 4 (`src/serve/ws/server.rs`).
//!
//! # Types
//! - [`diff_pane`] — detect whether a pane capture has changed.
//! - [`PaneCursor`] — stateful per-pane sequencer that emits payloads on diff.
//! - [`sessions_snapshot`] — convert raw `tmux list-sessions` output to
//!   [`SessionDto`]s for the `sessions` topic push.
//! - [`FlowWatcher`] — stateful non-terminal→terminal `sdlc-flow-state.json`
//!   transition tracker for the `workflow_done` WS push (BA.11.D).
//! - [`RunWatcher`] — stateful `LiveStateStore` run-status diff tracker for
//!   the `run_transition` WS push (BA.11.N).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use engine_contract::task_context::{NodeRunStatus, TaskContext};
use uuid::Uuid;

use crate::db::workflows::{self, NodeState, RunStatus};
use crate::detect::AgentState;
use crate::serve::dto::{
    RunTransitionPayload, SessionDto, WorkflowDonePayload, WsFrame, WsFrameKind,
};
use crate::serve::handlers::runs::run_status_str;
use crate::serve::status::flow::{FlowState, detect_transition};
use crate::sessions::model::parse_sessions;

// ── Pane diff ─────────────────────────────────────────────────────────────────

/// Return `true` when the pane capture has changed (or there is no previous
/// capture to compare against).
///
/// # Arguments
/// - `prev` — the last capture that was pushed, or `None` on first observation.
/// - `next` — the freshly captured pane output.
///
/// # Examples
/// ```
/// use crate::serve::poll::diff_pane;
/// assert!(diff_pane(None, "output"));        // no previous → always new
/// assert!(!diff_pane(Some("x"), "x"));       // same → no diff
/// assert!(diff_pane(Some("x"), "y"));        // changed → diff
/// ```
pub fn diff_pane(prev: Option<&str>, next: &str) -> bool {
    match prev {
        None => true,
        Some(p) => p != next,
    }
}

// ── Pane cursor ───────────────────────────────────────────────────────────────

/// Stateful per-pane sequencer.
///
/// Tracks the last known pane capture so successive identical captures do
/// not trigger pushes.  Each time the capture changes, `seq` is bumped and
/// the new line list is returned.
///
/// # Example
/// ```
/// let mut cursor = PaneCursor::default();
/// let first = cursor.observe("line1\nline2\n");
/// assert!(first.is_some()); // first observation always pushes (seq = 1)
///
/// let unchanged = cursor.observe("line1\nline2\n");
/// assert!(unchanged.is_none()); // identical capture → no push, seq stays 1
///
/// let changed = cursor.observe("line1\nline2\nline3\n");
/// assert!(changed.is_some()); // changed → push, seq = 2
/// ```
#[derive(Debug, Default)]
pub struct PaneCursor {
    /// The last pane capture that was pushed to subscribers.
    last: Option<String>,
    /// Monotonically increasing push counter, starts at 0 and is bumped
    /// *before* returning each payload (so the first push yields seq = 1).
    seq: u64,
}

impl PaneCursor {
    /// Observe a new pane capture.
    ///
    /// Returns `Some((seq, lines))` when the capture differs from the last
    /// push (or there is no previous capture), where `seq` is the new
    /// (bumped) sequence number and `lines` is the non-padding trailing lines
    /// of `capture`.
    ///
    /// Returns `None` when the capture is identical to the last push; in that
    /// case `seq` is **not** bumped.
    pub fn observe(&mut self, capture: &str) -> Option<(u64, Vec<String>)> {
        if !diff_pane(self.last.as_deref(), capture) {
            return None;
        }
        self.seq += 1;
        self.last = Some(capture.to_owned());
        let lines = extract_lines(capture);
        Some((self.seq, lines))
    }
}

/// Extract non-padding trailing lines from a raw pane capture.
///
/// Strips trailing blank lines, then returns all remaining lines as owned
/// strings.  Matches `Pane::last_lines(None)` semantics.
fn extract_lines(capture: &str) -> Vec<String> {
    let lines: Vec<&str> = capture.lines().collect();
    let end = lines
        .iter()
        .rposition(|l| !l.trim().is_empty())
        .map(|i| i + 1)
        .unwrap_or(0);
    lines[..end].iter().map(|l| l.to_string()).collect()
}

// ── Session snapshot ──────────────────────────────────────────────────────────

/// Build a [`SessionDto`] snapshot from raw `tmux list-sessions -F …` output.
///
/// This is the body of a `sessions` topic push.  Malformed lines are silently
/// skipped by [`parse_sessions`].  `last_line` on each `SessionDto` is empty
/// here because this is a pure function — pane capture (I/O) is handled by
/// the poll task in Task 4.
pub fn sessions_snapshot(raw: &str) -> Vec<SessionDto> {
    parse_sessions(raw).iter().map(SessionDto::from).collect()
}

/// One sweep's sessions list plus raw per-session pane captures — the shared
/// shape both the always-on
/// [`BlockedEdgePoller`](crate::serve::blocked_edge::BlockedEdgePoller)'s
/// tmux sweep and the WS hub's `sessions` topic push consume, so the two
/// never need to run independent sweeps on the same poll cadence.
pub type SessionsSweep = (Vec<SessionDto>, Vec<(String, String)>);

/// Mutex-guarded holder for the most recent [`SessionsSweep`].
///
/// Written by the always-on `BlockedEdgePoller` (the sole owner of the tmux
/// sweep) and read by the WS hub's `sessions` topic poll, so a `sessions`
/// subscriber never triggers a second, independent `list-sessions` +
/// per-session `capture-pane` sweep on the same cadence (BA.18.A review
/// fix — folding the two sweeps into one physical call).
pub type SharedSessionsSweep = Arc<Mutex<Option<SessionsSweep>>>;

/// Fill each [`SessionDto`]'s `last_line` from a per-session pane capture
/// (Gap 3), without touching [`sessions_snapshot`]'s empty-`last_line`
/// contract.
///
/// `panes` holds `(session_name, raw_pane_capture)` pairs — the same capture
/// pass the sessions-list poller already performs for the needs-input
/// rising-edge check (Gap 1/task 1), reused here so panes are captured once
/// per tick. Sessions with no matching entry in `panes` (e.g. the capture
/// failed for that tick) are left with an empty `last_line`, matching
/// `sessions_snapshot`'s existing behaviour for that session.
///
/// This is a pure sibling builder — the actual pane capture (I/O) happens in
/// the poll task's blocking closure; this function only combines the two
/// already-materialized values.
pub fn sessions_with_last_line(
    sessions: Vec<SessionDto>,
    panes: &[(String, String)],
) -> Vec<SessionDto> {
    let captures: HashMap<&str, &str> = panes
        .iter()
        .map(|(name, capture)| (name.as_str(), capture.as_str()))
        .collect();

    sessions
        .into_iter()
        .map(|mut dto| {
            if let Some(capture) = captures.get(dto.name.as_str()) {
                dto.last_line = crate::sessions::model::Pane::new(dto.name.clone(), *capture)
                    .last_line()
                    .to_owned();
            }
            dto
        })
        .collect()
}

// ── Needs-input detection ────────────────────────────────────────────────────

/// Needs-input rising edge: emit `event{needs_input}` only on the transition
/// INTO `Blocked`, not on every poll while still blocked.
pub fn should_emit_needs_input(prev: Option<AgentState>, new: AgentState) -> bool {
    new == AgentState::Blocked && prev != Some(AgentState::Blocked)
}

/// Return the names of every session in `current` whose `(prev, current)`
/// state pair satisfies [`should_emit_needs_input`] — i.e. the sessions that
/// just crossed into `Blocked` this tick.
///
/// `prev` is keyed by session name and holds the state observed on the
/// previous poll tick (sessions absent from `prev` are treated as having no
/// prior observation, matching `should_emit_needs_input(None, ..)`).
/// `current` is the full list of sessions observed this tick, in order.
///
/// This is the pure decision core for the sessions-list poller (the I/O shell
/// in `src/serve/ws/server.rs` captures each session's pane, computes its
/// `AgentState`, and calls this helper before fanning out `event{needs_input}`
/// frames).
pub fn sessions_needing_input(
    prev: &HashMap<String, AgentState>,
    current: &[(String, AgentState)],
) -> Vec<String> {
    current
        .iter()
        .filter(|(name, state)| should_emit_needs_input(prev.get(name).copied(), *state))
        .map(|(name, _)| name.clone())
        .collect()
}

/// Owned, off-actor home for the needs-input rising-edge's previous-state map.
///
/// `src/serve/ws/server.rs` previously kept this map as a `sessions_last_state`
/// field on the WS actor, which ties its lifetime to a WebSocket subscription:
/// with no subscriber the actor's `run_interval` handle is never installed, so
/// the map is never populated and the rising edge is never observed (BA.18.A).
///
/// [`NeedsInputTracker`] holds the exact same map and the exact same
/// [`sessions_needing_input`] decision — [`should_emit_needs_input`]'s
/// predicate is untouched — but as a small standalone value that can be
/// constructed once at server start and driven by an always-on poller
/// (Task 3), independent of whether any WebSocket client is subscribed. The
/// WS actor (Task 4) becomes a consumer of this tracker's output rather than
/// the owner of its state.
#[derive(Debug, Default)]
pub struct NeedsInputTracker {
    /// Session name → state observed on the previous poll tick.
    prev: HashMap<String, AgentState>,
}

impl NeedsInputTracker {
    /// Construct an empty tracker (no sessions observed yet).
    pub fn new() -> Self {
        Self::default()
    }

    /// Observe the current tick's `(session_name, state)` pairs, returning the
    /// names that just crossed into `Blocked` — via [`sessions_needing_input`],
    /// unchanged in meaning — and updating the internal map to `current` for
    /// the next tick.
    ///
    /// A session absent from `current` (e.g. it no longer exists) is dropped
    /// from the internal map rather than carried forward, matching
    /// `sessions_last_state = current.into_iter().collect()`'s previous
    /// actor-field behaviour.
    pub fn observe(&mut self, current: &[(String, AgentState)]) -> Vec<String> {
        let crossing = sessions_needing_input(&self.prev, current);
        self.prev = current.iter().cloned().collect();
        crossing
    }
}

// ── Flow watcher (BA.11.D) ──────────────────────────────────────────────────────

/// Stateful tracker for `sdlc-flow-state.json` transitions across observation
/// cycles, keyed by `(repo_name, spec_slug)`.
///
/// Wraps [`detect_transition`] (the pure per-flow comparison) with a map of
/// last-known statuses so a poll loop (Task 4 I/O wiring) can call
/// [`FlowWatcher::observe`] on every cycle and only get back payloads for
/// flows that just transitioned from non-terminal to terminal.
#[derive(Debug, Default)]
pub struct FlowWatcher {
    /// `(repo_name, spec_slug)` → last-observed `status` string.
    last_status: HashMap<(String, String), String>,
}

impl FlowWatcher {
    /// Construct an empty watcher (no flows observed yet).
    pub fn new() -> Self {
        Self::default()
    }

    /// Observe the current set of flow states for `repo`, returning a
    /// `(payload, event_name)` pair for each flow that just transitioned
    /// from a non-terminal status to a terminal one (`"done"`, `"blocked"`,
    /// or `"reconcile_failed"`).
    ///
    /// `event_name` is `detect_transition`'s returned event string —
    /// `"workflow_done"` for an ordinary `done`/`blocked` transition, or the
    /// distinct `"workflow_reconcile_failed"` for a transition into the
    /// gate-failure terminal state. `WorkflowDonePayload` itself is
    /// unchanged (it is `#[typeshare]`); the event name travels alongside it
    /// as a plain `String` rather than as a new payload field, so the wire
    /// DTO and `types/serve.ts` are untouched by this change.
    ///
    /// Always updates the internal map to the latest status for every flow
    /// passed in, regardless of whether an event was emitted.
    pub fn observe(
        &mut self,
        repo: &str,
        flows: &[FlowState],
    ) -> Vec<(WorkflowDonePayload, String)> {
        let mut events = Vec::new();

        for flow in flows {
            let key = (repo.to_owned(), flow.spec_slug.clone());
            let prev = self.last_status.get(&key).map(String::as_str);

            if let Some(event_name) = detect_transition(prev, flow) {
                events.push((
                    WorkflowDonePayload {
                        repo: repo.to_owned(),
                        spec_slug: flow.spec_slug.clone(),
                        status: flow.status.clone(),
                    },
                    event_name,
                ));
            }

            self.last_status.insert(key, flow.status.clone());
        }

        events
    }
}

/// Build the `event{workflow_done}` (or `event{workflow_reconcile_failed}`)
/// [`WsFrame`] for a transitioned flow.
///
/// Wire format (serve-api §11.5, flattened `EventPayload` + workflow fields):
/// `{ "session": "", "event": <event_name>, "repo": …, "spec_slug": …, "status": … }`
/// with `kind == "event"`. `session` is always the empty string — this event is
/// not scoped to a tmux session — and delivery is not subscription-gated (the
/// I/O shell in `src/serve/ws/server.rs` broadcasts it to every connection).
///
/// `event_name` is the string [`crate::serve::status::flow::detect_transition`]
/// returned for this transition — `"workflow_done"` for `done`/`blocked`,
/// `"workflow_reconcile_failed"` for a reconcile-failed gate. Callers must
/// not hard-code `"workflow_done"`; the caller (`watch_cycle`) carries the
/// name through from `FlowWatcher::observe` instead.
///
/// Pure function: no I/O, no actor messaging.
pub fn workflow_done_frame(payload: &WorkflowDonePayload, event_name: &str) -> WsFrame {
    WsFrame {
        kind: WsFrameKind::Event,
        payload: serde_json::json!({
            "session": "",
            "event": event_name,
            "repo": payload.repo,
            "spec_slug": payload.spec_slug,
            "status": payload.status,
        }),
    }
}

// ── Run watcher (BA.11.N) ────────────────────────────────────────────────────

/// Map `ctx.node_runs` into the minimal `NodeState` values
/// `db::workflows::derive_run_status` needs. Only `status` is load-bearing for
/// the aggregate logic; every other field is a cheap per-class default.
///
/// Mirrors `src/serve/handlers/runs.rs`'s private `node_states_from` (kept
/// separate rather than shared, since only `run_status_str` was widened for
/// this block — see that function's doc comment).
///
/// Pure — no I/O.
fn node_states_for_status(ctx: &TaskContext) -> Vec<NodeState> {
    ctx.node_runs
        .iter()
        .map(|(class, run)| NodeState {
            id: class.clone(),
            name: class.clone(),
            status: match run.status {
                NodeRunStatus::Pending => RunStatus::Pending,
                NodeRunStatus::Running => RunStatus::Running,
                NodeRunStatus::Success => RunStatus::Success,
                NodeRunStatus::Failed => RunStatus::Failed,
            },
            depends_on: vec![],
            input: None,
            output: None,
            error: None,
            tokens_in: None,
            tokens_out: None,
            model: None,
            started_at: None,
            elapsed_secs: None,
        })
        .collect()
}

/// Derive a run's aggregate wire status string from its `TaskContext`
/// snapshot, via the exact same `derive_run_status` + `run_status_str` pair
/// `GET /api/runs` uses, so the stream and the poll fallback can never
/// disagree (D17).
///
/// Pure — no I/O.
fn derive_status_str(ctx: &TaskContext) -> String {
    let nodes = node_states_for_status(ctx);
    let (status, _budget_halt) = workflows::derive_run_status(&nodes, &ctx.metadata);
    run_status_str(status)
}

/// Derive the wire status of a run **that has already gone lifecycle-terminal**
/// from its retained snapshot.
///
/// [`derive_status_str`] must not be used for this. `derive_run_status`
/// aggregates a *live* run and reports `Pending` whenever any node is still
/// pending — correct while a run is executing, but wrong once it is over,
/// because a finished workflow legitimately leaves every never-taken branch
/// `Pending` forever. A successful `SDLC_FLOW` run retains 15 `success` nodes
/// and 4 `pending` ones (the untaken router branches), so aggregating it
/// yields `"pending"` for a run that in fact succeeded — observed live
/// 2026-08-03 against run `3adfdd1b`, whose `/ws` terminal frame read
/// `status: "pending", terminal: true` while the engine's own readback said
/// `succeeded`.
///
/// This mirrors `engine-serve`'s `derive_terminal_status`
/// (`../engine-rs/crates/engine-serve/src/http.rs:295`) condition for
/// condition — it is `pub(crate)` there and so unreachable from bastion, and
/// D17 forbids an `engine-rs` change to expose it. Kept deliberately in the
/// same checked order, mapped onto bastion's own wire vocabulary
/// (`"success"`, not engine's `"succeeded"`) so this stream agrees with
/// `GET /api/runs` (§14.1) rather than with the engine's separate protocol:
///
/// 1. `metadata.cancellation.cancelled == true` -> `"cancelled"`
/// 2. `metadata.budget.halted == true` -> `"budget_halted"`
/// 3. `metadata.failure.failed == true` **or** any `node_runs[..]` failed -> `"failed"`
/// 4. otherwise -> `"success"`
///
/// `"suspended"` is deliberately unreachable here: a suspended run is still in
/// `LiveStateStore`'s live map, so it never reaches the disappearance edge
/// (D17 constraint 1).
///
/// Pure — no I/O.
fn derive_terminal_status_str(ctx: &TaskContext) -> String {
    let flag = |key: &str, field: &str| -> bool {
        ctx.metadata
            .get(key)
            .and_then(|v| v.get(field))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    };

    if flag("cancellation", "cancelled") {
        return run_status_str(RunStatus::Cancelled);
    }
    if flag("budget", "halted") {
        return run_status_str(RunStatus::BudgetHalted);
    }

    let any_node_failed = ctx
        .node_runs
        .values()
        .any(|run| run.status == NodeRunStatus::Failed);
    if flag("failure", "failed") || any_node_failed {
        return run_status_str(RunStatus::Failed);
    }

    run_status_str(RunStatus::Success)
}

/// Read `event.spec_slug` when present as a string. `None` when the key is
/// absent, non-string, or `event` is not an object.
///
/// Mirrors `src/serve/handlers/runs.rs`'s private `spec_slug_from_event`
/// (kept separate for the same reason as [`node_states_for_status`]).
///
/// Pure — no I/O.
fn spec_slug_from_event(event: &serde_json::Value) -> Option<String> {
    event
        .get("spec_slug")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

/// Stateful tracker for `LiveStateStore` run-status transitions across
/// observation cycles, keyed by run id.
///
/// Wraps [`derive_status_str`] with a map of last-known statuses so a poll
/// loop (`src/serve/ws/server.rs`'s `run_watch_cycle`) can call
/// [`RunWatcher::observe`] on every cycle and only get back payloads for runs
/// that just transitioned — either a live-map status change, or a
/// disappearance from the live map (D17's lifecycle-terminal edge).
#[derive(Debug, Default)]
pub struct RunWatcher {
    /// Run id → last-observed aggregate status string.
    last_status: HashMap<Uuid, String>,
}

impl RunWatcher {
    /// Construct an empty watcher (no runs observed yet).
    pub fn new() -> Self {
        Self::default()
    }

    /// Observe the current set of live runs, returning a
    /// [`RunTransitionPayload`] for every run that just transitioned.
    ///
    /// - `live` — every run id currently returned by `LiveStateStore::list_active()`,
    ///   paired with its latest `TaskContext` snapshot (the caller has already
    ///   done the store reads; this stays pure).
    /// - `terminal_lookup` — resolves a disappeared run id's retained snapshot
    ///   (`LiveStateStore::get_record(id).map(|r| r.snapshot)`, in the caller).
    ///   Returns `None` when the run was evicted from the bounded completed
    ///   ring before this cycle observed its disappearance.
    ///
    /// Two edges, both handled:
    /// 1. **Status change on a live run** — a previous status exists and
    ///    differs from the current one → emit `terminal: false`. Nothing is
    ///    emitted on the first observation of an id (no previous status) or
    ///    when the status is unchanged.
    /// 2. **Disappearance** — an id present in the cursor map but absent from
    ///    `live` has gone lifecycle-terminal. Its final status is resolved via
    ///    `terminal_lookup`; when that yields nothing, the last-known status is
    ///    carried instead of dropping the transition. Either way `terminal: true`
    ///    is emitted and the id is removed from the cursor map, so a later
    ///    re-appearance is treated as a fresh first observation.
    ///
    /// Always updates the internal map for every id passed in `live`,
    /// regardless of whether an event was emitted.
    pub fn observe(
        &mut self,
        live: &[(Uuid, TaskContext)],
        terminal_lookup: impl Fn(Uuid) -> Option<TaskContext>,
    ) -> Vec<RunTransitionPayload> {
        let mut events = Vec::new();
        let mut seen: std::collections::HashSet<Uuid> =
            std::collections::HashSet::with_capacity(live.len());

        for (run_id, ctx) in live {
            seen.insert(*run_id);
            let status = derive_status_str(ctx);
            let prev = self.last_status.get(run_id);

            if let Some(prev_status) = prev
                && prev_status != &status
            {
                events.push(RunTransitionPayload {
                    run_id: run_id.to_string(),
                    status: status.clone(),
                    terminal: false,
                    spec_slug: spec_slug_from_event(&ctx.event),
                });
            }

            self.last_status.insert(*run_id, status);
        }

        let disappeared: Vec<Uuid> = self
            .last_status
            .keys()
            .filter(|id| !seen.contains(id))
            .copied()
            .collect();

        for run_id in disappeared {
            let last_known = self
                .last_status
                .remove(&run_id)
                .expect("id came from last_status's own key set");

            let (status, spec_slug) = match terminal_lookup(run_id) {
                Some(ctx) => (
                    derive_terminal_status_str(&ctx),
                    spec_slug_from_event(&ctx.event),
                ),
                None => (last_known, None),
            };

            events.push(RunTransitionPayload {
                run_id: run_id.to_string(),
                status,
                terminal: true,
                spec_slug,
            });
        }

        events
    }
}

/// Build the `event{run_transition}` [`WsFrame`] for a transitioned run.
///
/// Wire format (serve-api §8.3, flattened `EventPayload` + run-transition
/// fields): `{ "session": "", "event": "run_transition", "run_id": …,
/// "status": …, "terminal": …, "spec_slug": … }` (`spec_slug` omitted when
/// `None`, matching [`RunTransitionPayload`]'s own `skip_serializing_if`).
/// `session` is always the empty string — this event is not scoped to a tmux
/// session — mirroring [`workflow_done_frame`].
///
/// Pure function: no I/O, no actor messaging.
pub fn run_transition_frame(payload: &RunTransitionPayload) -> WsFrame {
    let mut json = serde_json::json!({
        "session": "",
        "event": "run_transition",
        "run_id": payload.run_id,
        "status": payload.status,
        "terminal": payload.terminal,
    });
    if let Some(spec_slug) = &payload.spec_slug {
        json["spec_slug"] = serde_json::Value::String(spec_slug.clone());
    }
    WsFrame {
        kind: WsFrameKind::Event,
        payload: json,
    }
}

/// Build the `event{run_stream_status}` [`WsFrame`] pushed immediately when a
/// connection subscribes to the `runs` topic (D17 constraint 2).
///
/// Wire format (serve-api §8.3): `{ "session": "", "event":
/// "run_stream_status", "available": …, "reason": … }` (`reason` omitted when
/// `None`).
///
/// Pure function: no I/O, no actor messaging.
pub fn run_stream_status_frame(available: bool, reason: Option<&str>) -> WsFrame {
    let mut json = serde_json::json!({
        "session": "",
        "event": "run_stream_status",
        "available": available,
    });
    if let Some(reason) = reason {
        json["reason"] = serde_json::Value::String(reason.to_owned());
    }
    WsFrame {
        kind: WsFrameKind::Event,
        payload: json,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── diff_pane ──────────────────────────────────────────────────────────

    #[test]
    fn diff_pane_none_prev_is_always_true() {
        // No previous capture — any next content is considered "new".
        assert!(diff_pane(None, "some output"), "None prev must return true");
        assert!(
            diff_pane(None, ""),
            "None prev with empty next must return true"
        );
    }

    #[test]
    fn diff_pane_same_content_is_false() {
        let capture = "line1\nline2\n";
        assert!(
            !diff_pane(Some(capture), capture),
            "identical captures must return false"
        );
    }

    #[test]
    fn diff_pane_different_content_is_true() {
        assert!(
            diff_pane(Some("old output"), "new output"),
            "changed capture must return true"
        );
    }

    #[test]
    fn diff_pane_empty_prev_vs_nonempty_next() {
        assert!(
            diff_pane(Some(""), "something"),
            "empty prev vs non-empty next must return true"
        );
    }

    #[test]
    fn diff_pane_nonempty_prev_vs_empty_next() {
        assert!(
            diff_pane(Some("something"), ""),
            "non-empty prev vs empty next must return true"
        );
    }

    #[test]
    fn diff_pane_both_empty_is_false() {
        assert!(
            !diff_pane(Some(""), ""),
            "both empty must return false (no change)"
        );
    }

    // ── PaneCursor::observe ────────────────────────────────────────────────

    #[test]
    fn pane_cursor_first_observe_returns_seq_one() {
        let mut cursor = PaneCursor::default();
        let result = cursor.observe("line1\nline2\n");
        assert!(result.is_some(), "first observe must return Some");
        let (seq, _lines) = result.unwrap();
        assert_eq!(seq, 1, "first push must yield seq = 1");
    }

    #[test]
    fn pane_cursor_first_observe_returns_lines() {
        let mut cursor = PaneCursor::default();
        let result = cursor.observe("line1\nline2\n");
        let (_seq, lines) = result.unwrap();
        assert_eq!(lines, vec!["line1", "line2"]);
    }

    #[test]
    fn pane_cursor_unchanged_returns_none_and_does_not_bump_seq() {
        let mut cursor = PaneCursor::default();
        let first = cursor.observe("line1\nline2\n");
        assert!(first.is_some());
        let (seq_after_first, _) = first.unwrap();
        assert_eq!(seq_after_first, 1);

        // Same capture again — must return None, seq must stay 1.
        let second = cursor.observe("line1\nline2\n");
        assert!(second.is_none(), "identical capture must return None");
        // Observe a third time to confirm seq was not bumped on the None path.
        let third = cursor.observe("line1\nline2\nline3\n");
        assert!(third.is_some());
        let (seq_after_change, _) = third.unwrap();
        assert_eq!(
            seq_after_change, 2,
            "seq must be 2 after one unchanged + one changed observation"
        );
    }

    #[test]
    fn pane_cursor_changed_capture_bumps_seq() {
        let mut cursor = PaneCursor::default();
        cursor.observe("first capture\n");
        let result = cursor.observe("second capture\n");
        assert!(result.is_some(), "changed capture must return Some");
        let (seq, _) = result.unwrap();
        assert_eq!(seq, 2, "second distinct capture must yield seq = 2");
    }

    #[test]
    fn pane_cursor_changed_capture_returns_new_lines() {
        let mut cursor = PaneCursor::default();
        cursor.observe("old\n");
        let result = cursor.observe("new line a\nnew line b\n");
        let (_seq, lines) = result.unwrap();
        assert_eq!(lines, vec!["new line a", "new line b"]);
    }

    #[test]
    fn pane_cursor_strips_trailing_blank_lines() {
        let mut cursor = PaneCursor::default();
        // Capture with trailing blank lines — they must be stripped.
        let result = cursor.observe("line1\nline2\n\n\n");
        let (_seq, lines) = result.unwrap();
        assert_eq!(
            lines,
            vec!["line1", "line2"],
            "trailing blank lines must be stripped from the emitted lines"
        );
    }

    #[test]
    fn pane_cursor_empty_capture_emits_empty_lines() {
        let mut cursor = PaneCursor::default();
        // First observe with empty capture — it still pushes (no prev).
        let result = cursor.observe("");
        assert!(result.is_some());
        let (_seq, lines) = result.unwrap();
        assert!(lines.is_empty(), "empty capture must yield empty lines vec");
    }

    #[test]
    fn pane_cursor_multiple_changes_increment_seq_monotonically() {
        let mut cursor = PaneCursor::default();
        for i in 1u64..=5 {
            let capture = format!("output version {i}\n");
            let result = cursor.observe(&capture);
            assert!(result.is_some());
            let (seq, _) = result.unwrap();
            assert_eq!(seq, i, "seq must equal iteration number {i}");
        }
    }

    // ── sessions_snapshot ──────────────────────────────────────────────────

    /// Fixture matching the 5-field `tmux list-sessions` format:
    ///   name, attached, windows, activity_epoch, pane_current_command
    const FIXTURE_TWO_SESSIONS: &str = "\
main\t0\t3\t1718000000\tclaude\n\
background\t0\t1\t1718000100\tzsh\n";

    const FIXTURE_ONE_SESSION_RUNNING: &str = "work\t0\t2\t1718000200\tcargo\n";

    const FIXTURE_ONE_SESSION_IDLE: &str = "scratch\t0\t1\t1718000300\tzsh\n";

    #[test]
    fn sessions_snapshot_returns_correct_count() {
        let dtos = sessions_snapshot(FIXTURE_TWO_SESSIONS);
        assert_eq!(dtos.len(), 2, "must return one DTO per valid session line");
    }

    #[test]
    fn sessions_snapshot_maps_running_session_correctly() {
        let dtos = sessions_snapshot(FIXTURE_ONE_SESSION_RUNNING);
        assert_eq!(dtos.len(), 1);
        let dto = &dtos[0];
        assert_eq!(dto.name, "work");
        assert_eq!(dto.state, "running");
    }

    #[test]
    fn sessions_snapshot_maps_idle_session_correctly() {
        let dtos = sessions_snapshot(FIXTURE_ONE_SESSION_IDLE);
        assert_eq!(dtos.len(), 1);
        let dto = &dtos[0];
        assert_eq!(dto.name, "scratch");
        assert_eq!(dto.state, "idle");
    }

    #[test]
    fn sessions_snapshot_last_line_is_empty() {
        // parse_sessions does not do pane capture; last_line must be empty.
        let dtos = sessions_snapshot(FIXTURE_ONE_SESSION_RUNNING);
        assert_eq!(
            dtos[0].last_line, "",
            "last_line must be empty in a pure snapshot (no pane capture)"
        );
    }

    #[test]
    fn sessions_snapshot_with_two_sessions_preserves_order() {
        let dtos = sessions_snapshot(FIXTURE_TWO_SESSIONS);
        assert_eq!(dtos[0].name, "main");
        assert_eq!(dtos[0].state, "running"); // claude is not a shell → running
        assert_eq!(dtos[1].name, "background");
        assert_eq!(dtos[1].state, "idle"); // zsh → idle
    }

    // ── sessions_with_last_line ────────────────────────────────────────────

    #[test]
    fn sessions_with_last_line_fills_matching_session() {
        let dtos = sessions_snapshot(FIXTURE_ONE_SESSION_RUNNING);
        let panes = vec![(
            "work".to_owned(),
            "first\nsecond\nlast output line\n".to_owned(),
        )];
        let filled = sessions_with_last_line(dtos, &panes);
        assert_eq!(filled.len(), 1);
        assert_eq!(
            filled[0].last_line, "last output line",
            "matching session's last_line must be filled from its pane capture"
        );
    }

    #[test]
    fn sessions_with_last_line_ignores_trailing_blank_lines() {
        let dtos = sessions_snapshot(FIXTURE_ONE_SESSION_RUNNING);
        let panes = vec![("work".to_owned(), "real line\n\n\n".to_owned())];
        let filled = sessions_with_last_line(dtos, &panes);
        assert_eq!(
            filled[0].last_line, "real line",
            "trailing blank padding must be skipped, matching Pane::last_line"
        );
    }

    #[test]
    fn sessions_with_last_line_leaves_unmatched_session_empty() {
        let dtos = sessions_snapshot(FIXTURE_ONE_SESSION_RUNNING);
        // No pane entry for "work" at all.
        let panes: Vec<(String, String)> = vec![];
        let filled = sessions_with_last_line(dtos, &panes);
        assert_eq!(
            filled[0].last_line, "",
            "a session with no matching pane capture must keep last_line empty"
        );
    }

    #[test]
    fn sessions_with_last_line_handles_multi_session_selectively() {
        let dtos = sessions_snapshot(FIXTURE_TWO_SESSIONS);
        // Only "main" has a pane capture this tick; "background" does not.
        let panes = vec![("main".to_owned(), "hello world\n".to_owned())];
        let filled = sessions_with_last_line(dtos, &panes);
        let main = filled.iter().find(|d| d.name == "main").unwrap();
        let background = filled.iter().find(|d| d.name == "background").unwrap();
        assert_eq!(main.last_line, "hello world");
        assert_eq!(background.last_line, "");
    }

    #[test]
    fn sessions_with_last_line_empty_capture_yields_empty_last_line() {
        let dtos = sessions_snapshot(FIXTURE_ONE_SESSION_RUNNING);
        let panes = vec![("work".to_owned(), "".to_owned())];
        let filled = sessions_with_last_line(dtos, &panes);
        assert_eq!(
            filled[0].last_line, "",
            "an all-blank/empty capture must yield an empty last_line, not panic"
        );
    }

    #[test]
    fn sessions_snapshot_empty_input_returns_empty_vec() {
        let dtos = sessions_snapshot("");
        assert!(dtos.is_empty(), "empty raw input must yield empty vec");
    }

    #[test]
    fn sessions_snapshot_skips_malformed_lines() {
        // A malformed line (< 3 fields) must be silently skipped.
        let raw = "bad-line\nwork\t0\t2\t1718000200\tcargo\n";
        let dtos = sessions_snapshot(raw);
        // Only the valid line should survive.
        assert_eq!(dtos.len(), 1);
        assert_eq!(dtos[0].name, "work");
    }

    // ── should_emit_needs_input ────────────────────────────────────────────

    #[test]
    fn emit_needs_input_on_transition_from_none_to_blocked() {
        assert!(
            should_emit_needs_input(None, AgentState::Blocked),
            "first observation of Blocked (no prior state) must emit"
        );
    }

    #[test]
    fn emit_needs_input_on_transition_from_working_to_blocked() {
        assert!(
            should_emit_needs_input(Some(AgentState::Working), AgentState::Blocked),
            "Working->Blocked transition must emit"
        );
    }

    #[test]
    fn emit_needs_input_on_transition_from_idle_to_blocked() {
        assert!(
            should_emit_needs_input(Some(AgentState::Idle), AgentState::Blocked),
            "Idle->Blocked transition must emit"
        );
    }

    #[test]
    fn no_emit_when_already_blocked() {
        assert!(
            !should_emit_needs_input(Some(AgentState::Blocked), AgentState::Blocked),
            "Blocked->Blocked (no transition) must NOT emit"
        );
    }

    #[test]
    fn no_emit_when_transitioning_away_from_blocked() {
        assert!(
            !should_emit_needs_input(Some(AgentState::Blocked), AgentState::Working),
            "Blocked->Working must NOT emit"
        );
        assert!(
            !should_emit_needs_input(Some(AgentState::Blocked), AgentState::Idle),
            "Blocked->Idle must NOT emit"
        );
    }

    #[test]
    fn no_emit_for_non_blocked_states() {
        assert!(
            !should_emit_needs_input(None, AgentState::Idle),
            "None->Idle must NOT emit"
        );
        assert!(
            !should_emit_needs_input(Some(AgentState::Idle), AgentState::Working),
            "Idle->Working must NOT emit"
        );
        assert!(
            !should_emit_needs_input(Some(AgentState::Idle), AgentState::Idle),
            "Idle->Idle must NOT emit"
        );
    }

    // ── sessions_needing_input ──────────────────────────────────────────────

    #[test]
    fn sessions_needing_input_none_to_blocked_emits() {
        let prev = HashMap::new();
        let current = vec![("main".to_owned(), AgentState::Blocked)];
        let names = sessions_needing_input(&prev, &current);
        assert_eq!(names, vec!["main".to_owned()]);
    }

    #[test]
    fn sessions_needing_input_working_to_blocked_emits() {
        let mut prev = HashMap::new();
        prev.insert("main".to_owned(), AgentState::Working);
        let current = vec![("main".to_owned(), AgentState::Blocked)];
        let names = sessions_needing_input(&prev, &current);
        assert_eq!(names, vec!["main".to_owned()]);
    }

    #[test]
    fn sessions_needing_input_idle_to_blocked_emits() {
        let mut prev = HashMap::new();
        prev.insert("main".to_owned(), AgentState::Idle);
        let current = vec![("main".to_owned(), AgentState::Blocked)];
        let names = sessions_needing_input(&prev, &current);
        assert_eq!(names, vec!["main".to_owned()]);
    }

    #[test]
    fn sessions_needing_input_blocked_to_blocked_does_not_emit() {
        let mut prev = HashMap::new();
        prev.insert("main".to_owned(), AgentState::Blocked);
        let current = vec![("main".to_owned(), AgentState::Blocked)];
        let names = sessions_needing_input(&prev, &current);
        assert!(names.is_empty(), "already-Blocked must not re-emit");
    }

    #[test]
    fn sessions_needing_input_away_from_blocked_does_not_emit() {
        let mut prev = HashMap::new();
        prev.insert("main".to_owned(), AgentState::Blocked);
        let current = vec![("main".to_owned(), AgentState::Working)];
        let names = sessions_needing_input(&prev, &current);
        assert!(names.is_empty(), "Blocked->Working must not emit");
    }

    #[test]
    fn sessions_needing_input_multi_session_emits_only_crossing_names() {
        let mut prev = HashMap::new();
        prev.insert("alpha".to_owned(), AgentState::Working); // crosses
        prev.insert("beta".to_owned(), AgentState::Blocked); // already blocked
        prev.insert("gamma".to_owned(), AgentState::Idle); // stays idle
        // "delta" absent from prev (first observation) and crosses too.
        let current = vec![
            ("alpha".to_owned(), AgentState::Blocked),
            ("beta".to_owned(), AgentState::Blocked),
            ("gamma".to_owned(), AgentState::Idle),
            ("delta".to_owned(), AgentState::Blocked),
        ];
        let names = sessions_needing_input(&prev, &current);
        assert_eq!(names, vec!["alpha".to_owned(), "delta".to_owned()]);
    }

    // ── NeedsInputTracker ────────────────────────────────────────────────────

    #[test]
    fn needs_input_tracker_first_observe_none_to_blocked_emits() {
        let mut tracker = NeedsInputTracker::new();
        let current = vec![("main".to_owned(), AgentState::Blocked)];
        let crossing = tracker.observe(&current);
        assert_eq!(crossing, vec!["main".to_owned()]);
    }

    #[test]
    fn needs_input_tracker_stays_blocked_does_not_reemit() {
        let mut tracker = NeedsInputTracker::new();
        let current = vec![("main".to_owned(), AgentState::Blocked)];
        let first = tracker.observe(&current);
        assert_eq!(first, vec!["main".to_owned()]);

        let second = tracker.observe(&current);
        assert!(
            second.is_empty(),
            "already-blocked session must not re-emit on the next tick"
        );
    }

    #[test]
    fn needs_input_tracker_reblock_after_resolve_emits_twice() {
        let mut tracker = NeedsInputTracker::new();
        let blocked = vec![("main".to_owned(), AgentState::Blocked)];
        let resolved = vec![("main".to_owned(), AgentState::Working)];

        let first = tracker.observe(&blocked);
        assert_eq!(first, vec!["main".to_owned()]);

        let cleared = tracker.observe(&resolved);
        assert!(cleared.is_empty());

        let second = tracker.observe(&blocked);
        assert_eq!(
            second,
            vec!["main".to_owned()],
            "blocking again after resolving must emit a second time"
        );
    }

    #[test]
    fn needs_input_tracker_can_be_seeded_before_any_emission() {
        // Simulates seed-before-emit (Task 3): a tracker constructed and
        // observed once at boot with already-blocked sessions must not report
        // them as new crossings on that seed pass's own return value being
        // discarded, and must not re-report them on the next real tick either.
        let mut tracker = NeedsInputTracker::new();
        let already_blocked = vec![("main".to_owned(), AgentState::Blocked)];

        // Seed pass: caller discards the return value.
        let _ = tracker.observe(&already_blocked);

        // Next tick, still blocked: must not emit (matches boot-time seed
        // semantics where the seed pass's crossings are never surfaced).
        let next = tracker.observe(&already_blocked);
        assert!(
            next.is_empty(),
            "a session already blocked at seed time must not re-emit while still blocked"
        );
    }

    #[test]
    fn needs_input_tracker_multi_session_emits_only_crossing_names() {
        let mut tracker = NeedsInputTracker::new();
        tracker.observe(&[
            ("alpha".to_owned(), AgentState::Working),
            ("beta".to_owned(), AgentState::Blocked),
            ("gamma".to_owned(), AgentState::Idle),
        ]);

        let crossing = tracker.observe(&[
            ("alpha".to_owned(), AgentState::Blocked), // crosses
            ("beta".to_owned(), AgentState::Blocked),  // already blocked
            ("gamma".to_owned(), AgentState::Idle),    // stays idle
            ("delta".to_owned(), AgentState::Blocked), // first observation, crosses
        ]);

        assert_eq!(crossing, vec!["alpha".to_owned(), "delta".to_owned()]);
    }

    // ── FlowWatcher ───────────────────────────────────────────────────────

    fn flow(spec_slug: &str, status: &str) -> FlowState {
        FlowState {
            spec_slug: spec_slug.to_owned(),
            branch: format!("{spec_slug}-flow"),
            status: status.to_owned(),
            current_task: 1,
            started_at: "2026-06-30T00:00:00Z".to_owned(),
            updated_at: "2026-06-30T01:00:00Z".to_owned(),
            run_id: None,
        }
    }

    #[test]
    fn flow_watcher_first_observation_emits_no_events() {
        let mut watcher = FlowWatcher::new();
        let events = watcher.observe("bastion", &[flow("phase11-blockD", "running")]);
        assert!(events.is_empty(), "first observation must never emit");
    }

    #[test]
    fn flow_watcher_unchanged_status_emits_no_events() {
        let mut watcher = FlowWatcher::new();
        watcher.observe("bastion", &[flow("phase11-blockD", "running")]);
        let events = watcher.observe("bastion", &[flow("phase11-blockD", "running")]);
        assert!(events.is_empty(), "unchanged status must not emit");
    }

    #[test]
    fn flow_watcher_running_to_done_emits_event() {
        let mut watcher = FlowWatcher::new();
        watcher.observe("bastion", &[flow("phase11-blockD", "running")]);
        let events = watcher.observe("bastion", &[flow("phase11-blockD", "done")]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0.repo, "bastion");
        assert_eq!(events[0].0.spec_slug, "phase11-blockD");
        assert_eq!(events[0].0.status, "done");
        assert_eq!(events[0].1, "workflow_done");
    }

    #[test]
    fn flow_watcher_running_to_blocked_emits_event() {
        let mut watcher = FlowWatcher::new();
        watcher.observe("bastion", &[flow("phase11-blockD", "running")]);
        let events = watcher.observe("bastion", &[flow("phase11-blockD", "blocked")]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0.status, "blocked");
        assert_eq!(events[0].1, "workflow_done");
    }

    #[test]
    fn flow_watcher_running_to_reconcile_failed_emits_distinct_event() {
        let mut watcher = FlowWatcher::new();
        watcher.observe("bastion", &[flow("phase11-blockD", "running")]);
        let events = watcher.observe("bastion", &[flow("phase11-blockD", "reconcile_failed")]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0.status, "reconcile_failed");
        assert_eq!(events[0].1, "workflow_reconcile_failed");
    }

    #[test]
    fn flow_watcher_done_to_done_emits_no_events() {
        let mut watcher = FlowWatcher::new();
        watcher.observe("bastion", &[flow("phase11-blockD", "done")]);
        let events = watcher.observe("bastion", &[flow("phase11-blockD", "done")]);
        assert!(
            events.is_empty(),
            "already-terminal status must not re-emit"
        );
    }

    #[test]
    fn flow_watcher_tracks_multiple_repos_and_specs_independently() {
        let mut watcher = FlowWatcher::new();
        watcher.observe("bastion", &[flow("phase11-blockD", "running")]);
        watcher.observe("bella", &[flow("phase11-blockD", "running")]);

        // Same spec_slug, different repo — transitions must be tracked independently.
        let events_a = watcher.observe("bastion", &[flow("phase11-blockD", "done")]);
        assert_eq!(events_a.len(), 1);
        assert_eq!(events_a[0].0.repo, "bastion");

        let events_b = watcher.observe("bella", &[flow("phase11-blockD", "running")]);
        assert!(
            events_b.is_empty(),
            "bella's flow is still running, unaffected by bastion's transition"
        );
    }

    #[test]
    fn flow_watcher_multiple_flows_in_one_observation() {
        let mut watcher = FlowWatcher::new();
        watcher.observe(
            "bastion",
            &[
                flow("phase11-blockA", "running"),
                flow("phase11-blockB", "running"),
            ],
        );
        let events = watcher.observe(
            "bastion",
            &[
                flow("phase11-blockA", "done"),
                flow("phase11-blockB", "running"),
            ],
        );
        assert_eq!(events.len(), 1, "only the transitioned flow should emit");
        assert_eq!(events[0].0.spec_slug, "phase11-blockA");
    }

    // ── workflow_done_frame ──────────────────────────────────────────────────

    #[test]
    fn workflow_done_frame_has_event_kind() {
        let payload = WorkflowDonePayload {
            repo: "bastion".to_owned(),
            spec_slug: "phase11-blockD".to_owned(),
            status: "done".to_owned(),
        };
        let frame = workflow_done_frame(&payload, "workflow_done");
        assert_eq!(frame.kind, WsFrameKind::Event);
    }

    #[test]
    fn workflow_done_frame_payload_fields_are_flattened_and_exact() {
        let payload = WorkflowDonePayload {
            repo: "bastion".to_owned(),
            spec_slug: "phase11-blockD".to_owned(),
            status: "done".to_owned(),
        };
        let frame = workflow_done_frame(&payload, "workflow_done");

        assert_eq!(frame.payload["session"], serde_json::json!(""));
        assert_eq!(frame.payload["event"], serde_json::json!("workflow_done"));
        assert_eq!(frame.payload["repo"], serde_json::json!("bastion"));
        assert_eq!(
            frame.payload["spec_slug"],
            serde_json::json!("phase11-blockD")
        );
        assert_eq!(frame.payload["status"], serde_json::json!("done"));

        // Exactly these five fields — nothing extra, nothing missing.
        let obj = frame
            .payload
            .as_object()
            .expect("payload must be an object");
        assert_eq!(obj.len(), 5, "payload must have exactly 5 fields");
    }

    #[test]
    fn workflow_done_frame_reflects_blocked_status() {
        let payload = WorkflowDonePayload {
            repo: "bella".to_owned(),
            spec_slug: "some-spec".to_owned(),
            status: "blocked".to_owned(),
        };
        let frame = workflow_done_frame(&payload, "workflow_done");
        assert_eq!(frame.payload["status"], serde_json::json!("blocked"));
        assert_eq!(frame.payload["repo"], serde_json::json!("bella"));
        assert_eq!(frame.payload["spec_slug"], serde_json::json!("some-spec"));
    }

    #[test]
    fn workflow_done_frame_carries_reconcile_failed_event_name() {
        let payload = WorkflowDonePayload {
            repo: "bastion".to_owned(),
            spec_slug: "some-spec".to_owned(),
            status: "reconcile_failed".to_owned(),
        };
        let frame = workflow_done_frame(&payload, "workflow_reconcile_failed");
        assert_eq!(
            frame.payload["event"],
            serde_json::json!("workflow_reconcile_failed")
        );
        assert_ne!(frame.payload["event"], serde_json::json!("workflow_done"));
        assert_eq!(
            frame.payload["status"],
            serde_json::json!("reconcile_failed")
        );
    }

    // ── RunWatcher (BA.11.N) ──────────────────────────────────────────────────

    use engine_contract::task_context::NodeRun;

    fn node_run(status: NodeRunStatus) -> NodeRun {
        NodeRun {
            status,
            started_at: None,
            completed_at: None,
            error: None,
            input: None,
            usage: None,
        }
    }

    /// Build a `TaskContext` with one node `"NodeA"` at `status`, an optional
    /// `spec_slug` on `event`, and empty `metadata` (no suspension/cancel/budget
    /// annotation) — the plain node-aggregate path.
    fn ctx_with_node_status(status: NodeRunStatus, spec_slug: Option<&str>) -> TaskContext {
        let mut node_runs = HashMap::new();
        node_runs.insert("NodeA".to_owned(), node_run(status));
        let event = match spec_slug {
            Some(slug) => serde_json::json!({ "spec_slug": slug }),
            None => serde_json::json!({}),
        };
        TaskContext {
            event,
            nodes: HashMap::new(),
            metadata: serde_json::json!({}),
            node_runs,
        }
    }

    /// A `TaskContext` whose `metadata` marks the run suspended — the
    /// `derive_run_status` branch that must win over the node-aggregate
    /// fallback regardless of node statuses (D17 constraint 1).
    fn ctx_suspended() -> TaskContext {
        let mut node_runs = HashMap::new();
        node_runs.insert("NodeA".to_owned(), node_run(NodeRunStatus::Pending));
        TaskContext {
            event: serde_json::json!({}),
            nodes: HashMap::new(),
            metadata: serde_json::json!({ "suspension": { "suspended": true } }),
            node_runs,
        }
    }

    #[test]
    fn run_watcher_first_observation_emits_nothing() {
        let mut watcher = RunWatcher::new();
        let run_id = Uuid::new_v4();
        let ctx = ctx_with_node_status(NodeRunStatus::Pending, None);

        let events = watcher.observe(&[(run_id, ctx)], |_| None);

        assert!(
            events.is_empty(),
            "first observation of an id must emit nothing"
        );
    }

    #[test]
    fn run_watcher_unchanged_status_emits_nothing() {
        let mut watcher = RunWatcher::new();
        let run_id = Uuid::new_v4();
        let ctx1 = ctx_with_node_status(NodeRunStatus::Pending, None);
        let ctx2 = ctx_with_node_status(NodeRunStatus::Pending, None);

        watcher.observe(&[(run_id, ctx1)], |_| None);
        let events = watcher.observe(&[(run_id, ctx2)], |_| None);

        assert!(events.is_empty(), "unchanged status must emit nothing");
    }

    #[test]
    fn run_watcher_pending_to_running_emits_transition() {
        let mut watcher = RunWatcher::new();
        let run_id = Uuid::new_v4();
        let pending = ctx_with_node_status(NodeRunStatus::Pending, Some("phase11-blockN"));
        let running = ctx_with_node_status(NodeRunStatus::Running, Some("phase11-blockN"));

        watcher.observe(&[(run_id, pending)], |_| None);
        let events = watcher.observe(&[(run_id, running)], |_| None);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].run_id, run_id.to_string());
        assert_eq!(events[0].status, "running");
        assert!(!events[0].terminal);
        assert_eq!(events[0].spec_slug.as_deref(), Some("phase11-blockN"));
    }

    #[test]
    fn run_watcher_running_to_suspended_emits_terminal_false() {
        let mut watcher = RunWatcher::new();
        let run_id = Uuid::new_v4();
        let running = ctx_with_node_status(NodeRunStatus::Running, None);
        let suspended = ctx_suspended();

        watcher.observe(&[(run_id, running)], |_| None);
        let events = watcher.observe(&[(run_id, suspended)], |_| None);

        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].status, "suspended",
            "D17 constraint 1: suspended status must be reported"
        );
        assert!(
            !events[0].terminal,
            "D17 constraint 1: a suspended run must never be reported terminal"
        );
    }

    #[test]
    fn run_watcher_disappearance_with_record_emits_derived_status_terminal_true() {
        let mut watcher = RunWatcher::new();
        let run_id = Uuid::new_v4();
        let running = ctx_with_node_status(NodeRunStatus::Running, Some("phase11-blockN"));

        watcher.observe(&[(run_id, running)], |_| None);

        let record_ctx = ctx_with_node_status(NodeRunStatus::Success, Some("phase11-blockN"));
        let events = watcher.observe(&[], |id| {
            if id == run_id {
                Some(record_ctx.clone())
            } else {
                None
            }
        });

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].run_id, run_id.to_string());
        assert_eq!(
            events[0].status, "success",
            "disappearance must read the final status back from the record"
        );
        assert!(events[0].terminal);
        assert_eq!(events[0].spec_slug.as_deref(), Some("phase11-blockN"));
    }

    /// A retained snapshot shaped like a real completed `SDLC_FLOW` run:
    /// mostly `success` nodes plus the never-taken router branches, which stay
    /// `Pending` forever. `metadata` carries the extra keys a live run really
    /// has, to prove none of them are mistaken for a cancellation/budget/
    /// failure marker.
    fn ctx_finished_with_untaken_branches() -> TaskContext {
        let mut node_runs = HashMap::new();
        for name in ["SetupWorktreeNode", "ImplementTaskNode", "WrapUpNode"] {
            node_runs.insert(name.to_owned(), node_run(NodeRunStatus::Success));
        }
        for name in [
            "GenerateTasksNode",
            "ReviewRouterNode",
            "IncrementAttemptNode",
        ] {
            node_runs.insert(name.to_owned(), node_run(NodeRunStatus::Pending));
        }
        TaskContext {
            event: serde_json::json!({ "spec_slug": "smoke-sdlc-flow" }),
            nodes: HashMap::new(),
            metadata: serde_json::json!({ "run_id": "3adfdd1b", "run_telemetry": {} }),
            node_runs,
        }
    }

    #[test]
    fn run_watcher_finished_run_with_untaken_branches_reports_success_not_pending() {
        // Regression, found by live testing 2026-08-03 against real run
        // `3adfdd1b` (smoke-sdlc-flow): the engine's own readback said
        // `succeeded` while the `/ws` terminal frame said `pending`, because
        // `derive_run_status` aggregates a *live* run and any still-`Pending`
        // node dominates. A finished workflow legitimately leaves every
        // never-taken branch `Pending` forever, so the terminal edge must use
        // `derive_terminal_status_str` instead.
        let mut watcher = RunWatcher::new();
        let run_id = Uuid::new_v4();

        watcher.observe(
            &[(run_id, ctx_with_node_status(NodeRunStatus::Running, None))],
            |_| None,
        );

        let finished = ctx_finished_with_untaken_branches();
        let events = watcher.observe(&[], |_| Some(finished.clone()));

        assert_eq!(events.len(), 1);
        assert!(events[0].terminal);
        assert_eq!(
            events[0].status, "success",
            "a finished run whose untaken branches are still Pending must report success, \
             not the live-aggregate `pending` (observed live on run 3adfdd1b)"
        );
        // The live-aggregate path is what produced the bug — pin the contrast
        // so a future refactor cannot quietly route the terminal edge back
        // through it.
        assert_eq!(
            derive_status_str(&finished),
            "pending",
            "the live aggregate still reports pending for this same snapshot — \
             that is exactly why the terminal edge must not use it"
        );
    }

    #[test]
    fn derive_terminal_status_str_covers_the_engine_derivation_table() {
        // Mirrors engine-serve's `derive_terminal_status` table, in order.
        let mut failed = ctx_finished_with_untaken_branches();
        failed.node_runs.insert(
            "ImplementTaskNode".to_owned(),
            node_run(NodeRunStatus::Failed),
        );
        assert_eq!(
            derive_terminal_status_str(&failed),
            "failed",
            "any failed node must beat the success default"
        );

        let mut failure_marker = ctx_finished_with_untaken_branches();
        failure_marker.metadata = serde_json::json!({ "failure": { "failed": true } });
        assert_eq!(
            derive_terminal_status_str(&failure_marker),
            "failed",
            "metadata.failure.failed must be honoured even with no failed node"
        );

        let mut cancelled = ctx_finished_with_untaken_branches();
        cancelled.metadata = serde_json::json!({
            "cancellation": { "cancelled": true },
            "failure": { "failed": true },
        });
        assert_eq!(
            derive_terminal_status_str(&cancelled),
            "cancelled",
            "cancellation is checked first and must win over the failure marker"
        );

        let mut halted = ctx_finished_with_untaken_branches();
        halted.metadata = serde_json::json!({
            "budget": { "halted": true },
            "failure": { "failed": true },
        });
        assert_eq!(
            derive_terminal_status_str(&halted),
            "budget_halted",
            "budget halt is checked before the failure marker"
        );

        assert_eq!(
            derive_terminal_status_str(&ctx_finished_with_untaken_branches()),
            "success",
            "none of the above must fall through to success"
        );
    }

    #[test]
    fn run_watcher_disappearance_without_record_emits_last_known_status_terminal_true() {
        let mut watcher = RunWatcher::new();
        let run_id = Uuid::new_v4();
        let running = ctx_with_node_status(NodeRunStatus::Running, None);

        watcher.observe(&[(run_id, running)], |_| None);
        let events = watcher.observe(&[], |_| None);

        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].status, "running",
            "an evicted record must fall back to the last-known status, not drop the transition"
        );
        assert!(events[0].terminal);
    }

    #[test]
    fn run_watcher_reappearance_after_disappearance_is_a_fresh_first_observation() {
        let mut watcher = RunWatcher::new();
        let run_id = Uuid::new_v4();
        let running = ctx_with_node_status(NodeRunStatus::Running, None);

        watcher.observe(&[(run_id, running.clone())], |_| None);
        let disappearance_events = watcher.observe(&[], |_| None);
        assert_eq!(disappearance_events.len(), 1);

        // The id re-appears (e.g. a new run reusing... in practice a fresh
        // uuid, but the watcher's cursor was cleared either way) — treated as
        // a brand-new first observation, emitting nothing.
        let reappearance_events = watcher.observe(&[(run_id, running)], |_| None);
        assert!(
            reappearance_events.is_empty(),
            "a re-appearing id after disappearance must be a fresh first observation"
        );
    }

    #[test]
    fn run_watcher_disappearance_removes_id_so_it_cannot_refire() {
        let mut watcher = RunWatcher::new();
        let run_id = Uuid::new_v4();
        let running = ctx_with_node_status(NodeRunStatus::Running, None);

        watcher.observe(&[(run_id, running)], |_| None);
        let first = watcher.observe(&[], |_| None);
        assert_eq!(first.len(), 1);

        // A second empty cycle must not re-emit the same disappearance.
        let second = watcher.observe(&[], |_| None);
        assert!(second.is_empty());
    }

    // ── run_transition_frame ─────────────────────────────────────────────────

    #[test]
    fn run_transition_frame_has_event_kind_and_flattened_fields() {
        let payload = RunTransitionPayload {
            run_id: "11111111-1111-1111-1111-111111111111".to_owned(),
            status: "running".to_owned(),
            terminal: false,
            spec_slug: Some("phase11-blockN".to_owned()),
        };
        let frame = run_transition_frame(&payload);

        assert_eq!(frame.kind, WsFrameKind::Event);
        assert_eq!(frame.payload["session"], serde_json::json!(""));
        assert_eq!(frame.payload["event"], serde_json::json!("run_transition"));
        assert_eq!(
            frame.payload["run_id"],
            serde_json::json!("11111111-1111-1111-1111-111111111111")
        );
        assert_eq!(frame.payload["status"], serde_json::json!("running"));
        assert_eq!(frame.payload["terminal"], serde_json::json!(false));
        assert_eq!(
            frame.payload["spec_slug"],
            serde_json::json!("phase11-blockN")
        );

        let obj = frame
            .payload
            .as_object()
            .expect("payload must be an object");
        assert_eq!(obj.len(), 6, "payload must have exactly 6 fields");
    }

    #[test]
    fn run_transition_frame_omits_absent_spec_slug() {
        let payload = RunTransitionPayload {
            run_id: "22222222-2222-2222-2222-222222222222".to_owned(),
            status: "success".to_owned(),
            terminal: true,
            spec_slug: None,
        };
        let frame = run_transition_frame(&payload);

        assert_eq!(frame.payload["terminal"], serde_json::json!(true));
        let obj = frame
            .payload
            .as_object()
            .expect("payload must be an object");
        assert!(
            !obj.contains_key("spec_slug"),
            "spec_slug must be absent, not null, when unknown"
        );
        assert_eq!(obj.len(), 5, "payload must have exactly 5 fields");
    }

    // ── run_stream_status_frame ──────────────────────────────────────────────

    #[test]
    fn run_stream_status_frame_available_omits_reason() {
        let frame = run_stream_status_frame(true, None);

        assert_eq!(frame.kind, WsFrameKind::Event);
        assert_eq!(frame.payload["session"], serde_json::json!(""));
        assert_eq!(
            frame.payload["event"],
            serde_json::json!("run_stream_status")
        );
        assert_eq!(frame.payload["available"], serde_json::json!(true));

        let obj = frame
            .payload
            .as_object()
            .expect("payload must be an object");
        assert!(!obj.contains_key("reason"));
        assert_eq!(obj.len(), 3, "payload must have exactly 3 fields");
    }

    #[test]
    fn run_stream_status_frame_unavailable_carries_reason() {
        let frame = run_stream_status_frame(false, Some("DATABASE_URL not set"));

        assert_eq!(frame.payload["available"], serde_json::json!(false));
        assert_eq!(
            frame.payload["reason"],
            serde_json::json!("DATABASE_URL not set")
        );

        let obj = frame
            .payload
            .as_object()
            .expect("payload must be an object");
        assert_eq!(obj.len(), 4, "payload must have exactly 4 fields");
    }
}
