// sessions/commands.rs — session verb handlers (list, attach, new, kill).
//
// Decision D4: this entire path is DB-free. No Config::load(), no Postgres pool.
// Decision D5: all verbs are synchronous blocking calls — no async/tokio coupling.
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

/// Attach to an existing tmux session, inheriting the terminal.
/// Blocks until the user detaches; then returns to the shell.
pub fn attach(session_name: &str) -> anyhow::Result<()> {
    match tmux::attach_session(session_name) {
        Ok(()) => Ok(()),
        Err(e) => apply_degradation("attach", session_name, e),
    }
}

/// Create a new detached tmux session, optionally in a given directory.
pub fn new(session_name: &str, dir: Option<&str>) -> anyhow::Result<()> {
    match tmux::new_session(session_name, dir) {
        Ok(()) => {
            println!("{}", format_created(session_name));
            Ok(())
        }
        Err(e) => apply_degradation("new", session_name, e),
    }
}

/// Kill (remove) a tmux session by name.
pub fn kill(session_name: &str) -> anyhow::Result<()> {
    match tmux::kill_session(session_name) {
        Ok(()) => {
            println!("{}", format_killed(session_name));
            Ok(())
        }
        Err(e) => apply_degradation("kill", session_name, e),
    }
}

/// Outcome of mapping a `TmuxError` to user-facing degradation.
#[derive(Debug, PartialEq)]
pub enum Degraded {
    /// Print this message; treat as success (graceful — tmux not installed / no server).
    Graceful(String),
    /// Print this message; propagate the original error.
    Fatal(String),
}

/// Map a `TmuxError` to its user-facing degradation for a given verb.
/// Pure logic, extracted from the handlers so it is unit-testable without
/// spawning tmux. `verb` is the CLI verb name (`attach` / `new` / `kill`).
pub fn degrade_tmux_error(verb: &str, session_name: &str, err: &TmuxError) -> Degraded {
    match err {
        TmuxError::NotInstalled => Degraded::Graceful(format!(
            "tmux not installed — install tmux to use `bastion {verb}`"
        )),
        TmuxError::NoServer => Degraded::Graceful("no tmux server running".to_string()),
        TmuxError::ExitError { stderr, .. } => match verb {
            "new" => Degraded::Fatal(format!("error creating session '{session_name}': {stderr}")),
            _ => Degraded::Fatal(format!("error: session '{session_name}' not found")),
        },
    }
}

/// Apply the degradation outcome for a tmux error: print the message and either
/// swallow (graceful) or propagate (fatal) the original error. Non-`TmuxError`
/// errors are propagated unchanged.
fn apply_degradation(verb: &str, session_name: &str, e: anyhow::Error) -> anyhow::Result<()> {
    if let Some(te) = e.downcast_ref::<TmuxError>() {
        match degrade_tmux_error(verb, session_name, te) {
            Degraded::Graceful(msg) => {
                println!("{msg}");
                return Ok(());
            }
            Degraded::Fatal(msg) => {
                println!("{msg}");
                return Err(e);
            }
        }
    }
    Err(e)
}

/// Pure formatting helpers — testable without I/O.
pub fn format_created(name: &str) -> String {
    format!("created session '{}'", name)
}

pub fn format_killed(name: &str) -> String {
    format!("killed session '{}'", name)
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

    #[test]
    fn format_created_contains_name() {
        let msg = format_created("my-session");
        assert!(
            msg.contains("my-session"),
            "expected session name in: {msg}"
        );
        assert!(msg.contains("created"), "expected 'created' in: {msg}");
    }

    #[test]
    fn format_killed_contains_name() {
        let msg = format_killed("old-session");
        assert!(
            msg.contains("old-session"),
            "expected session name in: {msg}"
        );
        assert!(msg.contains("killed"), "expected 'killed' in: {msg}");
    }

    // ── TmuxError degradation mapping (#1) ──────────────────────────────────────

    #[test]
    fn degrade_not_installed_is_graceful_with_verb() {
        // The verb name is interpolated into the hint, so test more than one verb.
        let attach = degrade_tmux_error("attach", "x", &TmuxError::NotInstalled);
        let kill = degrade_tmux_error("kill", "x", &TmuxError::NotInstalled);
        match attach {
            Degraded::Graceful(m) => assert!(m.contains("bastion attach"), "got: {m}"),
            other => panic!("expected Graceful, got {other:?}"),
        }
        match kill {
            Degraded::Graceful(m) => assert!(m.contains("bastion kill"), "got: {m}"),
            other => panic!("expected Graceful, got {other:?}"),
        }
    }

    #[test]
    fn degrade_no_server_is_graceful() {
        let d = degrade_tmux_error("new", "x", &TmuxError::NoServer);
        assert_eq!(d, Degraded::Graceful("no tmux server running".to_string()));
    }

    #[test]
    fn degrade_exit_error_for_new_is_fatal_with_stderr() {
        let err = TmuxError::ExitError {
            code: 1,
            stderr: "duplicate session: work".to_string(),
        };
        match degrade_tmux_error("new", "work", &err) {
            Degraded::Fatal(m) => {
                assert!(m.contains("error creating session 'work'"), "got: {m}");
                assert!(m.contains("duplicate session: work"), "got: {m}");
            }
            other => panic!("expected Fatal, got {other:?}"),
        }
    }

    #[test]
    fn degrade_exit_error_for_attach_and_kill_is_fatal_not_found() {
        let err = TmuxError::ExitError {
            code: 1,
            stderr: "can't find session: ghost".to_string(),
        };
        for verb in ["attach", "kill"] {
            match degrade_tmux_error(verb, "ghost", &err) {
                Degraded::Fatal(m) => {
                    assert!(m.contains("session 'ghost' not found"), "verb {verb}: {m}");
                }
                other => panic!("verb {verb}: expected Fatal, got {other:?}"),
            }
        }
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
