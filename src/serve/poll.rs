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

use std::collections::HashMap;

use crate::serve::dto::{SessionDto, WorkflowDonePayload};
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
    /// [`WorkflowDonePayload`] for each flow that just transitioned from a
    /// non-terminal status to a terminal one (`"done"` or `"blocked"`).
    ///
    /// Always updates the internal map to the latest status for every flow
    /// passed in, regardless of whether an event was emitted.
    pub fn observe(&mut self, repo: &str, flows: &[FlowState]) -> Vec<WorkflowDonePayload> {
        let mut events = Vec::new();

        for flow in flows {
            let key = (repo.to_owned(), flow.spec_slug.clone());
            let prev = self.last_status.get(&key).map(String::as_str);

            if detect_transition(prev, flow).is_some() {
                events.push(WorkflowDonePayload {
                    repo: repo.to_owned(),
                    spec_slug: flow.spec_slug.clone(),
                    status: flow.status.clone(),
                });
            }

            self.last_status.insert(key, flow.status.clone());
        }

        events
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

    // ── FlowWatcher ───────────────────────────────────────────────────────

    fn flow(spec_slug: &str, status: &str) -> FlowState {
        FlowState {
            spec_slug: spec_slug.to_owned(),
            branch: format!("{spec_slug}-flow"),
            status: status.to_owned(),
            current_task: 1,
            started_at: "2026-06-30T00:00:00Z".to_owned(),
            updated_at: "2026-06-30T01:00:00Z".to_owned(),
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
        assert_eq!(events[0].repo, "bastion");
        assert_eq!(events[0].spec_slug, "phase11-blockD");
        assert_eq!(events[0].status, "done");
    }

    #[test]
    fn flow_watcher_running_to_blocked_emits_event() {
        let mut watcher = FlowWatcher::new();
        watcher.observe("bastion", &[flow("phase11-blockD", "running")]);
        let events = watcher.observe("bastion", &[flow("phase11-blockD", "blocked")]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].status, "blocked");
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
        assert_eq!(events_a[0].repo, "bastion");

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
        assert_eq!(events[0].spec_slug, "phase11-blockA");
    }
}
