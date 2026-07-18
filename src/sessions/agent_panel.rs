// sessions/agent_panel.rs — pure builder for the always-visible cross-space
// "agents · priority" strip (BA.13.1.2).
//
// No I/O, no theme access here: rows carry state only, colors are applied at
// render time (`sessions/ui.rs`) from the runtime theme (BA.14.0).

use crate::detect::AgentState;
use crate::monitor::app::session_urgency;
use crate::sessions::model::Session;

/// One row of the agent panel: a session's display label plus its detected
/// `AgentState`, sorted by urgency (Blocked/needs-input first).
#[derive(Debug, Clone, PartialEq)]
pub struct AgentPanelRow {
    pub label: String,
    pub agent_state: AgentState,
}

impl AgentPanelRow {
    fn from_session(session: &Session) -> Self {
        Self {
            label: session.name.clone(),
            agent_state: session.agent_state,
        }
    }
}

/// Build one `AgentPanelRow` per session, sorted by `session_urgency`
/// (Blocked/needs-input first, then Working/Running, then Idle/Unknown).
pub fn agent_panel_rows(sessions: &[Session]) -> Vec<AgentPanelRow> {
    let mut ordered: Vec<&Session> = sessions.iter().collect();
    ordered.sort_by_key(|s| session_urgency(s));
    ordered
        .iter()
        .map(|s| AgentPanelRow::from_session(s))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::model::SessionState;

    fn make_session(name: &str, agent_state: AgentState, state: SessionState) -> Session {
        Session {
            name: name.to_string(),
            state,
            window_count: 1,
            foreground_cmd: String::new(),
            last_line: String::new(),
            agent_state,
            cwd: String::new(),
        }
    }

    #[test]
    fn empty_slice_produces_no_rows() {
        let rows = agent_panel_rows(&[]);
        assert!(rows.is_empty());
    }

    #[test]
    fn one_row_per_session() {
        let sessions = vec![
            make_session("a", AgentState::Idle, SessionState::Idle),
            make_session("b", AgentState::Working, SessionState::Idle),
            make_session("c", AgentState::Blocked, SessionState::Idle),
        ];
        let rows = agent_panel_rows(&sessions);
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn sorted_blocked_before_working_before_idle() {
        let sessions = vec![
            make_session("idle", AgentState::Idle, SessionState::Idle),
            make_session("working", AgentState::Working, SessionState::Idle),
            make_session("blocked", AgentState::Blocked, SessionState::Idle),
        ];
        let rows = agent_panel_rows(&sessions);
        assert_eq!(rows[0].label, "blocked");
        assert_eq!(rows[1].label, "working");
        assert_eq!(rows[2].label, "idle");
    }

    #[test]
    fn row_carries_label_and_agent_state() {
        let sessions = vec![make_session("s1", AgentState::Unknown, SessionState::Idle)];
        let rows = agent_panel_rows(&sessions);
        assert_eq!(rows[0].label, "s1");
        assert_eq!(rows[0].agent_state, AgentState::Unknown);
    }

    #[test]
    fn running_session_state_sorts_mid_like_working() {
        let sessions = vec![
            make_session("idle", AgentState::Idle, SessionState::Idle),
            make_session("running", AgentState::Idle, SessionState::Running),
            make_session("blocked", AgentState::Blocked, SessionState::Idle),
        ];
        let rows = agent_panel_rows(&sessions);
        assert_eq!(rows[0].label, "blocked");
        assert_eq!(rows[1].label, "running");
        assert_eq!(rows[2].label, "idle");
    }
}
