// sessions/commands.rs — session verb handlers (list, attach, new, kill).
//
// Decision D4: this entire path is DB-free. No Config::load(), no Postgres pool.
// Decision D5: all verbs are synchronous blocking calls — no async/tokio coupling.
// tmux is the only data source.

use crate::sessions::claude_state::{TrustStatus, trust_status};
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
///
/// After session creation, prints a one-line trust pre-flight for the
/// resolved directory (reads `~/.claude.json` as a read-only observer).
/// The trust check is advisory: Unknown is acceptable and never blocks
/// or fails session creation.
pub fn new(session_name: &str, dir: Option<&str>) -> anyhow::Result<()> {
    match tmux::new_session(session_name, dir) {
        Ok(()) => {
            println!("{}", format_created(session_name));
            if let Some(d) = dir {
                let status = trust_status(d);
                println!("{}", format_trust(status, d));
            }
            Ok(())
        }
        Err(e) => apply_degradation("new", session_name, e),
    }
}

/// Send `keys` to the named tmux session, followed by Enter.
pub fn send(session_name: &str, keys: &str) -> anyhow::Result<()> {
    match tmux::send_keys(session_name, keys) {
        Ok(()) => {
            println!("{}", format_sent(session_name, keys));
            Ok(())
        }
        Err(e) => apply_degradation("send", session_name, e),
    }
}

/// Capture the last N lines of pane output for the named session.
/// Prints one line per line; trailing blank padding from `capture-pane -p` is excluded.
pub fn capture(session_name: &str, lines: Option<usize>) -> anyhow::Result<()> {
    match tmux::capture_pane_raw(session_name) {
        Ok(output) => {
            let pane = Pane::new(session_name, output);
            let captured = pane.last_lines(lines);
            print!("{}", format_capture(&captured));
            Ok(())
        }
        Err(e) => apply_degradation("capture", session_name, e),
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

pub fn format_sent(session: &str, keys: &str) -> String {
    format!("sent to '{}': {}", session, keys)
}

/// Join captured lines for printing. Each line ends with a newline; empty slice → empty string.
pub fn format_capture(lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    lines.iter().map(|l| format!("{l}\n")).collect()
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
        "{:<20}  {:<20}  {}\n",
        "SESSION", "STATE", "LAST OUTPUT"
    ));
    out.push_str(&"-".repeat(70));
    out.push('\n');

    for s in sessions {
        let state_col = format_state_col(s);
        let last = if s.last_line.is_empty() {
            "(no output)"
        } else {
            &s.last_line
        };
        out.push_str(&format!("{:<20}  {:<20}  {}\n", s.name, state_col, last,));
    }

    out
}

/// Pure helper: format the STATE column for a session row.
/// Running sessions show "running (cmd)"; idle sessions show "idle".
pub fn format_state_col(s: &Session) -> String {
    use crate::sessions::model::SessionState;
    match s.state {
        SessionState::Running if !s.foreground_cmd.is_empty() => {
            format!("running ({})", s.foreground_cmd)
        }
        SessionState::Running => "running".to_string(),
        SessionState::Idle => "idle".to_string(),
    }
}

/// Pure helper: format the trust pre-flight line for `bastion new --dir`.
pub fn format_trust(status: TrustStatus, _dir: &str) -> String {
    match status {
        TrustStatus::Trusted => "trust: trusted".to_string(),
        TrustStatus::Untrusted => {
            "trust: untrusted (Claude will prompt on first launch)".to_string()
        }
        TrustStatus::Unknown => "trust: unknown".to_string(),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::claude_state::TrustStatus;
    use crate::sessions::model::SessionState;

    fn make_session(name: &str, state: SessionState, last_line: &str) -> Session {
        Session {
            name: name.to_string(),
            state,
            window_count: 1,
            foreground_cmd: String::new(),
            last_line: last_line.to_string(),
        }
    }

    fn make_session_with_cmd(
        name: &str,
        state: SessionState,
        foreground_cmd: &str,
        last_line: &str,
    ) -> Session {
        Session {
            name: name.to_string(),
            state,
            window_count: 1,
            foreground_cmd: foreground_cmd.to_string(),
            last_line: last_line.to_string(),
        }
    }

    #[test]
    fn render_empty_sessions_shows_no_sessions() {
        let out = render_sessions(&[]);
        assert!(out.contains("no sessions"));
    }

    #[test]
    fn render_single_running_session_shows_command() {
        let sessions = vec![make_session_with_cmd(
            "main",
            SessionState::Running,
            "cargo",
            "",
        )];
        let out = render_sessions(&sessions);
        assert!(out.contains("main"), "row: {out}");
        assert!(out.contains("running (cargo)"), "row: {out}");
    }

    #[test]
    fn render_running_session_without_cmd_shows_running() {
        let sessions = vec![make_session("main", SessionState::Running, "")];
        let out = render_sessions(&sessions);
        assert!(out.contains("running"), "row: {out}");
    }

    #[test]
    fn render_single_idle_session_shows_idle() {
        let sessions = vec![make_session("scratch", SessionState::Idle, "")];
        let out = render_sessions(&sessions);
        assert!(out.contains("scratch"), "row: {out}");
        assert!(out.contains("idle"), "row: {out}");
        assert!(out.contains("(no output)"), "row: {out}");
    }

    #[test]
    fn render_multiple_sessions() {
        let sessions = vec![
            make_session_with_cmd("main", SessionState::Running, "cargo", "cargo test"),
            make_session("bg", SessionState::Idle, ""),
        ];
        let out = render_sessions(&sessions);
        assert!(out.contains("main"), "row: {out}");
        assert!(out.contains("running (cargo)"), "row: {out}");
        assert!(out.contains("cargo test"), "row: {out}");
        assert!(out.contains("bg"), "row: {out}");
        assert!(out.contains("idle"), "row: {out}");
    }

    // ── format_state_col ──────────────────────────────────────────────────────

    #[test]
    fn format_state_col_running_with_cmd() {
        let s = make_session_with_cmd("s", SessionState::Running, "claude", "");
        assert_eq!(format_state_col(&s), "running (claude)");
    }

    #[test]
    fn format_state_col_running_no_cmd() {
        let s = make_session("s", SessionState::Running, "");
        assert_eq!(format_state_col(&s), "running");
    }

    #[test]
    fn format_state_col_idle() {
        let s = make_session("s", SessionState::Idle, "");
        assert_eq!(format_state_col(&s), "idle");
    }

    // ── format_trust ──────────────────────────────────────────────────────────

    #[test]
    fn format_trust_trusted() {
        let msg = format_trust(TrustStatus::Trusted, "/some/dir");
        assert_eq!(msg, "trust: trusted");
    }

    #[test]
    fn format_trust_untrusted_contains_hint() {
        let msg = format_trust(TrustStatus::Untrusted, "/some/dir");
        assert!(msg.contains("trust: untrusted"), "got: {msg}");
        assert!(msg.contains("Claude will prompt"), "got: {msg}");
    }

    #[test]
    fn format_trust_unknown() {
        let msg = format_trust(TrustStatus::Unknown, "/some/dir");
        assert_eq!(msg, "trust: unknown");
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

    #[test]
    fn degrade_exit_error_for_send_is_fatal_not_found() {
        let err = TmuxError::ExitError {
            code: 1,
            stderr: "can't find session: ghost".to_string(),
        };
        match degrade_tmux_error("send", "ghost", &err) {
            Degraded::Fatal(m) => {
                assert!(m.contains("session 'ghost' not found"), "got: {m}");
            }
            other => panic!("expected Fatal, got {other:?}"),
        }
    }

    #[test]
    fn format_sent_contains_session_and_command() {
        let msg = format_sent("work", "cargo build --release");
        assert!(msg.contains("work"), "expected session name in: {msg}");
        assert!(
            msg.contains("cargo build --release"),
            "expected command in: {msg}"
        );
    }

    // ── capture verb degradation ─────────────────────────────────────────────

    #[test]
    fn degrade_exit_error_for_capture_is_fatal_not_found() {
        let err = TmuxError::ExitError {
            code: 1,
            stderr: "can't find session: ghost".to_string(),
        };
        match degrade_tmux_error("capture", "ghost", &err) {
            Degraded::Fatal(m) => {
                assert!(m.contains("session 'ghost' not found"), "got: {m}");
            }
            other => panic!("expected Fatal, got {other:?}"),
        }
    }

    #[test]
    fn degrade_not_installed_for_capture_is_graceful() {
        match degrade_tmux_error("capture", "any", &TmuxError::NotInstalled) {
            Degraded::Graceful(m) => assert!(m.contains("bastion capture"), "got: {m}"),
            other => panic!("expected Graceful, got {other:?}"),
        }
    }

    // ── format_capture ────────────────────────────────────────────────────────

    #[test]
    fn format_capture_joins_lines_with_newline() {
        let lines = vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()];
        let out = format_capture(&lines);
        assert_eq!(out, "alpha\nbeta\ngamma\n");
    }

    #[test]
    fn format_capture_empty_slice_returns_empty_string() {
        let out = format_capture(&[]);
        assert_eq!(out, "");
    }

    #[test]
    fn format_capture_single_line_has_trailing_newline() {
        let lines = vec!["only".to_string()];
        let out = format_capture(&lines);
        assert_eq!(out, "only\n");
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
        // 5-field format: name, attached, windows, activity, pane_current_command
        let sessions = crate::sessions::model::parse_sessions(
            "work\t1\t2\t1718000000\tcargo\nscratch\t0\t1\t1718000001\tzsh\n",
        );
        let out = render_sessions(&sessions);

        assert!(out.contains("work"));
        assert!(out.contains("scratch"));
        // No config error was raised — the test reaching this assertion is the proof.
    }
}
