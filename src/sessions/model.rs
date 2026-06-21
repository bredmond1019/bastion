// sessions/model.rs — Session / Pane types + pure parsing of tmux output.
//
// Parsing rule for running/idle indicator:
//   A session is considered "running" when it has at least one attached client
//   (session_attached == "1"). Otherwise it is "idle".
//   This is the most reliable signal available from `tmux list-sessions -F`
//   without querying individual pane process states.

use crate::sessions::tmux::{FIELD_SEP, LIST_SESSIONS_FORMAT};
use anyhow::{Result, bail};

/// Whether a session has an attached client.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionState {
    Running,
    Idle,
}

impl SessionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionState::Running => "running",
            SessionState::Idle => "idle",
        }
    }
}

/// Represents a single tmux session as reported by `list-sessions`.
#[derive(Debug, Clone)]
pub struct Session {
    pub name: String,
    pub state: SessionState,
    pub window_count: u32,
    /// Last non-blank line from `capture-pane -p`, empty string if none.
    pub last_line: String,
}

/// Lightweight pane capture: the raw text from `capture-pane -p -t <session>`.
/// The last non-blank line is extracted from this by `last_pane_line`.
#[derive(Debug, Clone)]
pub struct Pane {
    pub session_name: String,
    pub raw_output: String,
}

impl Pane {
    pub fn new(session_name: impl Into<String>, raw_output: impl Into<String>) -> Self {
        Self {
            session_name: session_name.into(),
            raw_output: raw_output.into(),
        }
    }

    /// Extract the last non-blank line from the captured pane output.
    pub fn last_line(&self) -> &str {
        self.raw_output
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
    }

    /// Return the trailing lines of pane output, after stripping trailing blank/whitespace-only
    /// lines that `tmux capture-pane -p` pads to fill the pane height.
    ///
    /// - `None`    → return all non-padding lines (oldest → newest).
    /// - `Some(n)` → return at most the last `n` non-padding lines.
    /// - `Some(0)` → empty Vec.
    pub fn last_lines(&self, n: Option<usize>) -> Vec<String> {
        if n == Some(0) {
            return Vec::new();
        }

        // Collect non-padding lines: strip trailing blank lines first.
        let lines: Vec<&str> = self.raw_output.lines().collect();
        let trimmed_end = lines
            .iter()
            .rposition(|l| !l.trim().is_empty())
            .map(|i| i + 1)
            .unwrap_or(0);
        let meaningful: &[&str] = &lines[..trimmed_end];

        match n {
            None => meaningful.iter().map(|l| l.to_string()).collect(),
            Some(count) => {
                let start = meaningful.len().saturating_sub(count);
                meaningful[start..].iter().map(|l| l.to_string()).collect()
            }
        }
    }
}

// ── Parsing ──────────────────────────────────────────────────────────────────

/// Parse a single `tmux list-sessions -F <LIST_SESSIONS_FORMAT>` output line
/// into a `Session` (without last-line, which requires a separate capture-pane call).
///
/// Returns `Err` for malformed lines so callers can choose to skip or propagate.
pub fn parse_session_line(line: &str) -> Result<Session> {
    // Expected field order matches LIST_SESSIONS_FORMAT:
    //   session_name  \t  session_attached  \t  session_windows  \t  session_activity
    let _ = LIST_SESSIONS_FORMAT; // Suppress unused-import lint; used in doc / test.

    let parts: Vec<&str> = line.splitn(4, FIELD_SEP).collect();
    if parts.len() < 3 {
        bail!(
            "malformed list-sessions line (expected ≥3 tab-separated fields): {:?}",
            line
        );
    }

    let name = parts[0].to_string();
    if name.is_empty() {
        bail!(
            "malformed list-sessions line: empty session name in {:?}",
            line
        );
    }

    let attached = parts[1].trim();
    let state = if attached == "1" {
        SessionState::Running
    } else {
        SessionState::Idle
    };

    let window_count: u32 = parts[2].trim().parse().unwrap_or(0);

    Ok(Session {
        name,
        state,
        window_count,
        last_line: String::new(), // filled in by commands.rs after capture-pane
    })
}

/// Parse the full multi-line output of `tmux list-sessions -F ...` into a
/// `Vec<Session>`. Malformed lines are skipped with a warning to stderr.
pub fn parse_sessions(output: &str) -> Vec<Session> {
    output
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| match parse_session_line(line) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("bastion: skipping malformed session line: {e}");
                None
            }
        })
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Fixtures ─────────────────────────────────────────────────────────────

    /// Fixture: two sessions — one attached (running), one detached (idle).
    const FIXTURE_TWO_SESSIONS: &str = "\
main\t1\t3\t1718000000\n\
background\t0\t1\t1718000100\n";

    /// Fixture: single session with an attached client.
    const FIXTURE_ATTACHED: &str = "work\t1\t2\t1718000200\n";

    /// Fixture: single session, no attached client.
    const FIXTURE_DETACHED: &str = "scratch\t0\t1\t1718000300\n";

    /// Fixture: malformed line (only one field).
    const FIXTURE_MALFORMED: &str = "bad-line-no-tabs\n";

    // ── Session line parsing ──────────────────────────────────────────────────

    #[test]
    fn parses_attached_session_as_running() {
        let s = parse_session_line("work\t1\t2\t1718000200").unwrap();
        assert_eq!(s.name, "work");
        assert_eq!(s.state, SessionState::Running);
        assert_eq!(s.window_count, 2);
    }

    #[test]
    fn parses_detached_session_as_idle() {
        let s = parse_session_line("scratch\t0\t1\t1718000300").unwrap();
        assert_eq!(s.name, "scratch");
        assert_eq!(s.state, SessionState::Idle);
        assert_eq!(s.window_count, 1);
    }

    #[test]
    fn parses_multiple_sessions() {
        let sessions = parse_sessions(FIXTURE_TWO_SESSIONS);
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].name, "main");
        assert_eq!(sessions[0].state, SessionState::Running);
        assert_eq!(sessions[1].name, "background");
        assert_eq!(sessions[1].state, SessionState::Idle);
    }

    #[test]
    fn malformed_line_is_skipped_not_panicked() {
        // The malformed line should be silently skipped; valid lines still parsed.
        let input = format!("{}\n{}", FIXTURE_MALFORMED.trim(), "good\t0\t1\t1718000000");
        let sessions = parse_sessions(&input);
        // Only the good line should be returned.
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].name, "good");
    }

    #[test]
    fn empty_output_yields_empty_vec() {
        let sessions = parse_sessions("");
        assert!(sessions.is_empty());
    }

    // ── Attached vs detached fixtures ─────────────────────────────────────────

    #[test]
    fn attached_fixture_is_running() {
        let sessions = parse_sessions(FIXTURE_ATTACHED);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].state, SessionState::Running);
    }

    #[test]
    fn detached_fixture_is_idle() {
        let sessions = parse_sessions(FIXTURE_DETACHED);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].state, SessionState::Idle);
    }

    // ── Pane / last-line ─────────────────────────────────────────────────────

    #[test]
    fn pane_last_line_returns_last_nonblank() {
        let pane = Pane::new("work", "first line\nsecond line\n\n");
        assert_eq!(pane.last_line(), "second line");
    }

    #[test]
    fn pane_last_line_empty_when_all_blank() {
        let pane = Pane::new("empty-session", "\n   \n\n");
        assert_eq!(pane.last_line(), "");
    }

    #[test]
    fn pane_last_line_single_line() {
        let pane = Pane::new("single", "only line");
        assert_eq!(pane.last_line(), "only line");
    }

    #[test]
    fn state_as_str() {
        assert_eq!(SessionState::Running.as_str(), "running");
        assert_eq!(SessionState::Idle.as_str(), "idle");
    }

    // ── Pane::last_lines ─────────────────────────────────────────────────────

    #[test]
    fn last_lines_none_returns_all_nonblank_trailing_stripped() {
        let pane = Pane::new("s", "line1\nline2\nline3\n\n   \n");
        let lines = pane.last_lines(None);
        assert_eq!(lines, vec!["line1", "line2", "line3"]);
    }

    #[test]
    fn last_lines_some_n_more_lines_than_n() {
        let pane = Pane::new("s", "a\nb\nc\nd\ne\n\n");
        let lines = pane.last_lines(Some(3));
        assert_eq!(lines, vec!["c", "d", "e"]);
    }

    #[test]
    fn last_lines_some_n_fewer_lines_than_n() {
        let pane = Pane::new("s", "x\ny\n\n\n");
        let lines = pane.last_lines(Some(10));
        assert_eq!(lines, vec!["x", "y"]);
    }

    #[test]
    fn last_lines_some_n_exactly_n_lines() {
        let pane = Pane::new("s", "p\nq\nr\n\n");
        let lines = pane.last_lines(Some(3));
        assert_eq!(lines, vec!["p", "q", "r"]);
    }

    #[test]
    fn last_lines_some_zero_returns_empty() {
        let pane = Pane::new("s", "a\nb\nc\n");
        let lines = pane.last_lines(Some(0));
        assert!(lines.is_empty());
    }

    #[test]
    fn last_lines_empty_input_returns_empty() {
        let pane = Pane::new("s", "");
        assert!(pane.last_lines(None).is_empty());
        assert!(pane.last_lines(Some(5)).is_empty());
    }

    #[test]
    fn last_lines_all_blank_returns_empty() {
        let pane = Pane::new("s", "\n   \n\t\n");
        assert!(pane.last_lines(None).is_empty());
        assert!(pane.last_lines(Some(3)).is_empty());
    }

    #[test]
    fn last_lines_order_is_preserved_oldest_newest() {
        let pane = Pane::new("s", "first\nsecond\nthird\n\n");
        let lines = pane.last_lines(Some(2));
        assert_eq!(lines, vec!["second", "third"]);
    }

    #[test]
    fn last_lines_trailing_blank_padding_stripped() {
        // Simulate tmux padding: content lines followed by many blank lines.
        let raw = "output1\noutput2\n\n\n\n\n\n\n\n";
        let pane = Pane::new("s", raw);
        let lines = pane.last_lines(None);
        assert_eq!(lines, vec!["output1", "output2"]);
    }
}
