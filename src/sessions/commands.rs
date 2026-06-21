// sessions/commands.rs — `bastion sessions` list verb.
//
// Decision D4: this entire path is DB-free. No Config::load(), no Postgres pool.
// tmux is the only data source.

use crate::sessions::model::{Pane, Session, parse_sessions};
use crate::sessions::tmux::{self, TmuxError};

/// Entry point for `bastion sessions`.
/// Gathers data from tmux and prints a plain-text table.
pub fn run() -> anyhow::Result<()> {
    let raw = match tmux::list_sessions_raw() {
        Ok(r) => r,
        Err(e) => {
            // Graceful degradation: missing binary or no server → human message, no panic.
            if let Some(te) = e.downcast_ref::<TmuxError>() {
                match te {
                    TmuxError::NotInstalled => {
                        println!("tmux not installed — install tmux to use `bastion sessions`");
                        return Ok(());
                    }
                    TmuxError::NoServer => {
                        println!("no tmux server running");
                        return Ok(());
                    }
                    TmuxError::ExitError { .. } => {}
                }
            }
            return Err(e);
        }
    };

    let mut sessions = parse_sessions(&raw);

    // Enrich each session with its last pane line.
    for session in sessions.iter_mut() {
        match tmux::capture_pane_raw(&session.name) {
            Ok(output) => {
                let pane = Pane::new(&session.name, output);
                session.last_line = pane.last_line().to_string();
            }
            Err(_) => {
                // Non-fatal: last line stays empty.
                session.last_line = String::new();
            }
        }
    }

    print!("{}", render_sessions(&sessions));
    Ok(())
}

/// Pure render function: `&[Session]` → formatted String.
/// No I/O so it can be unit-tested against fixture data.
pub fn render_sessions(sessions: &[Session]) -> String {
    if sessions.is_empty() {
        return "no sessions\n".to_string();
    }

    let mut out = String::new();
    // Header
    out.push_str(&format!(
        "{:<20}  {:<8}  {}\n",
        "SESSION", "STATE", "LAST OUTPUT"
    ));
    out.push_str(&"-".repeat(70));
    out.push('\n');

    for s in sessions {
        let last = if s.last_line.is_empty() {
            "(no output)"
        } else {
            &s.last_line
        };
        out.push_str(&format!(
            "{:<20}  {:<8}  {}\n",
            s.name,
            s.state.as_str(),
            last,
        ));
    }

    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::model::SessionState;

    fn make_session(name: &str, state: SessionState, last_line: &str) -> Session {
        Session {
            name: name.to_string(),
            state,
            window_count: 1,
            last_line: last_line.to_string(),
        }
    }

    #[test]
    fn render_empty_sessions_shows_no_sessions() {
        let out = render_sessions(&[]);
        assert!(out.contains("no sessions"));
    }

    #[test]
    fn render_single_running_session() {
        let sessions = vec![make_session("main", SessionState::Running, "cargo build")];
        let out = render_sessions(&sessions);
        assert!(out.contains("main"));
        assert!(out.contains("running"));
        assert!(out.contains("cargo build"));
    }

    #[test]
    fn render_single_idle_session_with_empty_last_line() {
        let sessions = vec![make_session("scratch", SessionState::Idle, "")];
        let out = render_sessions(&sessions);
        assert!(out.contains("scratch"));
        assert!(out.contains("idle"));
        assert!(out.contains("(no output)"));
    }

    #[test]
    fn render_multiple_sessions() {
        let sessions = vec![
            make_session("main", SessionState::Running, "cargo test"),
            make_session("bg", SessionState::Idle, ""),
        ];
        let out = render_sessions(&sessions);
        assert!(out.contains("main"));
        assert!(out.contains("running"));
        assert!(out.contains("cargo test"));
        assert!(out.contains("bg"));
        assert!(out.contains("idle"));
    }

    /// Architectural guarantee: the sessions code path does not call Config::load()
    /// and does not open a Postgres pool. We verify this by calling the pure
    /// render/parse functions directly with DATABASE_URL intentionally absent from
    /// the environment, and confirming they do not panic or return a config error.
    #[test]
    fn sessions_render_path_requires_no_database_url() {
        // Remove DATABASE_URL from the environment for this test.
        // (In CI it may never be set; either way, these functions must not care.)
        // Safety: single-threaded test; no other thread reads this env var.
        unsafe { std::env::remove_var("DATABASE_URL") };

        // These are the only functions on the sessions command path that
        // process data; neither should require config.
        let sessions = crate::sessions::model::parse_sessions(
            "work\t1\t2\t1718000000\nscratch\t0\t1\t1718000001\n",
        );
        let out = render_sessions(&sessions);

        assert!(out.contains("work"));
        assert!(out.contains("scratch"));
        // No config error was raised — the test reaching this assertion is the proof.
    }
}
