//! Pure `sdlc-flow-state.json` parser + terminal-transition detection for the
//! `bastion serve` workflow-status surface.
//!
//! No I/O happens here — callers (Task 4 handlers, the poll loop in
//! `src/serve/poll.rs`) read the file / poll on a timer and pass the content
//! or parsed struct in.

use serde::{Deserialize, Serialize};

// ── Shared types ──────────────────────────────────────────────────────────────

/// Parsed view of a spec's `sdlc-flow-state.json`.
///
/// Only the fields the `bastion serve` status surface needs are modeled here
/// — the real file carries a much larger `tasks`/`review`/`docs`/`pr` shape,
/// but `serde(deny_unknown_fields)` is deliberately *not* set so those extra
/// fields are ignored rather than rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowState {
    pub spec_slug: String,
    pub branch: String,
    /// Raw status string, e.g. `"running"`, `"done"`, `"blocked"`.
    pub status: String,
    pub current_task: u32,
    pub started_at: String,
    pub updated_at: String,
}

// ── Pure parsing ──────────────────────────────────────────────────────────────

/// Parse `content` (the full text of a `sdlc-flow-state.json` file) into a
/// [`FlowState`]. Returns `None` on malformed/non-matching JSON.
pub fn parse_flow_state(content: &str) -> Option<FlowState> {
    serde_json::from_str(content).ok()
}

/// Returns `true` for the two terminal status strings (`"done"`, `"blocked"`),
/// `false` for anything else (e.g. `"running"`, `"pending"`).
pub fn is_terminal(status: &str) -> bool {
    matches!(status, "done" | "blocked")
}

/// Detect a non-terminal → terminal status transition.
///
/// Returns `Some("workflow_done")` when `prev_status` is `Some` and
/// non-terminal while `current.status` is terminal. Returns `None` in every
/// other case: no previous status (`prev_status` is `None`, i.e. first
/// observation), previous status already terminal, or current status still
/// non-terminal.
pub fn detect_transition(prev_status: Option<&str>, current: &FlowState) -> Option<String> {
    let prev = prev_status?;
    if is_terminal(prev) {
        return None;
    }
    if is_terminal(&current.status) {
        return Some("workflow_done".to_string());
    }
    None
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = include_str!("fixtures/flow_state_valid.json");
    const MALFORMED: &str = include_str!("fixtures/flow_state_malformed.json");

    fn flow_with_status(status: &str) -> FlowState {
        FlowState {
            spec_slug: "phase11-blockD".to_string(),
            branch: "phase11-blockD-flow".to_string(),
            status: status.to_string(),
            current_task: 2,
            started_at: "2026-06-30T00:00:00Z".to_string(),
            updated_at: "2026-06-30T01:00:00Z".to_string(),
        }
    }

    #[test]
    fn parses_valid_flow_state() {
        let state = parse_flow_state(VALID).expect("should parse");
        assert_eq!(state.spec_slug, "phase6-blockA");
        assert_eq!(state.branch, "phase6-blockA-flow");
        assert_eq!(state.status, "done");
        assert_eq!(state.current_task, 5);
        assert_eq!(state.started_at, "2026-06-25T18:30:59Z");
        assert_eq!(state.updated_at, "2026-06-25T19:02:33Z");
    }

    #[test]
    fn malformed_json_returns_none() {
        assert!(parse_flow_state(MALFORMED).is_none());
    }

    #[test]
    fn empty_input_returns_none() {
        assert!(parse_flow_state("").is_none());
    }

    #[test]
    fn missing_required_field_returns_none() {
        let content = r#"{"branch": "x", "status": "running"}"#;
        assert!(parse_flow_state(content).is_none());
    }

    #[test]
    fn is_terminal_matches_done_and_blocked_only() {
        assert!(is_terminal("done"));
        assert!(is_terminal("blocked"));
        assert!(!is_terminal("running"));
        assert!(!is_terminal("pending"));
        assert!(!is_terminal(""));
    }

    #[test]
    fn detect_transition_running_to_done_emits_event() {
        let current = flow_with_status("done");
        assert_eq!(
            detect_transition(Some("running"), &current),
            Some("workflow_done".to_string())
        );
    }

    #[test]
    fn detect_transition_running_to_blocked_emits_event() {
        let current = flow_with_status("blocked");
        assert_eq!(
            detect_transition(Some("running"), &current),
            Some("workflow_done".to_string())
        );
    }

    #[test]
    fn detect_transition_already_terminal_emits_no_event() {
        let current = flow_with_status("done");
        assert_eq!(detect_transition(Some("done"), &current), None);
        assert_eq!(detect_transition(Some("blocked"), &current), None);
    }

    #[test]
    fn detect_transition_still_running_emits_no_event() {
        let current = flow_with_status("running");
        assert_eq!(detect_transition(Some("running"), &current), None);
        assert_eq!(detect_transition(Some("pending"), &current), None);
    }

    #[test]
    fn detect_transition_no_prev_emits_no_event() {
        let current = flow_with_status("done");
        assert_eq!(detect_transition(None, &current), None);
    }
}
