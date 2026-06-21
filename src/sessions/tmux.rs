// sessions/tmux.rs — thin wrapper over `std::process::Command` → the tmux CLI.
// Decision D4: sessions surface uses no DB; driven entirely by tmux process output.
//
// Design: command *construction* (pure, returns args Vec) is separated from
// command *execution* (does I/O) so construction can be unit-tested without
// spawning a real tmux process.

use anyhow::{Context, Result, bail};
use std::process::Command;

// ── Format strings ────────────────────────────────────────────────────────────

/// Format string used with `tmux list-sessions -F`.
/// Fields (tab-separated): session_name, attached (1/0), window count, activity (epoch secs).
pub const LIST_SESSIONS_FORMAT: &str =
    "#{session_name}\t#{session_attached}\t#{session_windows}\t#{session_activity}";

/// Separator between fields in LIST_SESSIONS_FORMAT output.
pub const FIELD_SEP: char = '\t';

// ── Command construction (pure) ───────────────────────────────────────────────

/// Returns the argument list for:
///   tmux list-sessions -F <LIST_SESSIONS_FORMAT>
/// The first element is the `tmux` binary name.
pub fn list_sessions_args() -> Vec<String> {
    vec![
        "tmux".to_string(),
        "list-sessions".to_string(),
        "-F".to_string(),
        LIST_SESSIONS_FORMAT.to_string(),
    ]
}

/// Returns the argument list for:
///   tmux capture-pane -p -t <session_name>
/// The first element is the `tmux` binary name.
pub fn capture_pane_args(session_name: &str) -> Vec<String> {
    vec![
        "tmux".to_string(),
        "capture-pane".to_string(),
        "-p".to_string(),
        "-t".to_string(),
        session_name.to_string(),
    ]
}

// ── Execution ─────────────────────────────────────────────────────────────────

/// Errors produced by this module.
#[derive(Debug, thiserror::Error)]
pub enum TmuxError {
    #[error("tmux binary not found — is tmux installed?")]
    NotInstalled,
    #[error("no tmux server running")]
    NoServer,
    #[error("tmux error (exit {code}): {stderr}")]
    ExitError { code: i32, stderr: String },
}

/// Execute a tmux command (args[0] = "tmux", args[1..] = subcommand + flags).
/// Returns the captured stdout on success.
pub fn run_tmux(args: &[String]) -> Result<String> {
    debug_assert!(!args.is_empty(), "args must not be empty");
    let (bin, rest) = args.split_first().expect("args must not be empty");

    let output = Command::new(bin).args(rest).output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow::Error::new(TmuxError::NotInstalled)
        } else {
            anyhow::Error::new(e).context("failed to run tmux")
        }
    })?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        return Ok(stdout);
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    // tmux exits 1 with this stderr when no server is running.
    if stderr.contains("no server running")
        || stderr.contains("error connecting to")
        || stderr.contains("No such file or directory")
    {
        bail!(TmuxError::NoServer);
    }

    let code = output.status.code().unwrap_or(-1);
    bail!(TmuxError::ExitError { code, stderr });
}

/// List all tmux sessions; returns raw formatted output lines.
pub fn list_sessions_raw() -> Result<String> {
    let args = list_sessions_args();
    run_tmux(&args).context("list-sessions failed")
}

/// Capture the last-pane output of the given session; returns raw text.
pub fn capture_pane_raw(session_name: &str) -> Result<String> {
    let args = capture_pane_args(session_name);
    run_tmux(&args).context("capture-pane failed")
}

// ── Tests (pure, no live tmux) ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_sessions_args_correct() {
        let args = list_sessions_args();
        assert_eq!(args[0], "tmux");
        assert_eq!(args[1], "list-sessions");
        assert_eq!(args[2], "-F");
        assert_eq!(args[3], LIST_SESSIONS_FORMAT);
        assert_eq!(args.len(), 4);
    }

    #[test]
    fn capture_pane_args_correct() {
        let args = capture_pane_args("my-session");
        assert_eq!(args[0], "tmux");
        assert_eq!(args[1], "capture-pane");
        assert_eq!(args[2], "-p");
        assert_eq!(args[3], "-t");
        assert_eq!(args[4], "my-session");
        assert_eq!(args.len(), 5);
    }

    #[test]
    fn list_sessions_format_contains_required_fields() {
        assert!(LIST_SESSIONS_FORMAT.contains("#{session_name}"));
        assert!(LIST_SESSIONS_FORMAT.contains("#{session_attached}"));
        assert!(LIST_SESSIONS_FORMAT.contains("#{session_windows}"));
        assert!(LIST_SESSIONS_FORMAT.contains("#{session_activity}"));
    }

    #[test]
    fn field_sep_matches_format_separator() {
        // Verify the const separator agrees with what we put in the format string.
        assert!(LIST_SESSIONS_FORMAT.contains(FIELD_SEP));
    }
}
