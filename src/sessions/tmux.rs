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
/// Fields (tab-separated):
///   1. session_name
///   2. session_attached (1/0)
///   3. session_windows (count)
///   4. session_activity (epoch secs)
///   5. pane_current_command (foreground process name in the first pane)
///
/// State (running vs idle) is derived from field 5, not field 2.
pub const LIST_SESSIONS_FORMAT: &str = "#{session_name}\t#{session_attached}\t#{session_windows}\t#{session_activity}\t#{pane_current_command}";

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

/// Returns the argument list for:
///   tmux attach -t <session_name>
/// The first element is the `tmux` binary name.
pub fn attach_args(session_name: &str) -> Vec<String> {
    vec![
        "tmux".to_string(),
        "attach".to_string(),
        "-t".to_string(),
        session_name.to_string(),
    ]
}

/// Returns the argument list for:
///   tmux new-session -d -s <session_name> [-c <dir>]
/// The first element is the `tmux` binary name.
pub fn new_session_args(session_name: &str, dir: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "tmux".to_string(),
        "new-session".to_string(),
        "-d".to_string(),
        "-s".to_string(),
        session_name.to_string(),
    ];
    if let Some(d) = dir {
        args.push("-c".to_string());
        args.push(d.to_string());
    }
    args
}

/// Returns the argument list for:
///   tmux kill-session -t <session_name>
/// The first element is the `tmux` binary name.
pub fn kill_session_args(session_name: &str) -> Vec<String> {
    vec![
        "tmux".to_string(),
        "kill-session".to_string(),
        "-t".to_string(),
        session_name.to_string(),
    ]
}

/// Returns the argument list for a **literal** send-keys invocation:
///   tmux send-keys -t <session_name> -l -- <keys>
///
/// `-l` (literal) ensures the text is never interpreted as tmux key names
/// (e.g. a command containing `Enter`, `C-c`).  `--` prevents a command
/// starting with `-` from being parsed as a flag.  The `keys` value is a
/// single argv element — multi-word commands are passed verbatim.
///
/// The Enter keypress must be sent in a separate call (use `send_enter_args`)
/// because `-l` disables key-name lookup.
pub fn send_keys_args(session_name: &str, keys: &str) -> Vec<String> {
    vec![
        "tmux".to_string(),
        "send-keys".to_string(),
        "-t".to_string(),
        session_name.to_string(),
        "-l".to_string(),
        "--".to_string(),
        keys.to_string(),
    ]
}

/// Returns the argument list for sending an Enter keypress:
///   tmux send-keys -t <session_name> Enter
///
/// This is a separate invocation from `send_keys_args` because `-l` (literal)
/// disables key-name lookup, so `Enter` would be sent as the literal string
/// rather than the Return key.
pub fn send_enter_args(session_name: &str) -> Vec<String> {
    vec![
        "tmux".to_string(),
        "send-keys".to_string(),
        "-t".to_string(),
        session_name.to_string(),
        "Enter".to_string(),
    ]
}

/// Returns the argument list for a **named-key** send-keys invocation:
///   tmux send-keys -t <session_name> <key>
///
/// Unlike `send_keys_args`, this does **not** use `-l` or `--` so that tmux
/// resolves the key name (`Escape`, `Enter`, `Up`, `Down`, `Left`, `Right`,
/// `C-c`, etc.) rather than sending it as literal text.
///
/// Use this for control keys and special keys that cannot be sent with `-l`.
pub fn send_named_key_args(session_name: &str, key: &str) -> Vec<String> {
    vec![
        "tmux".to_string(),
        "send-keys".to_string(),
        "-t".to_string(),
        session_name.to_string(),
        key.to_string(),
    ]
}

/// Returns the argument list for sending a **sequence** of named keys in one
/// `send-keys` call:
///   tmux send-keys -t <session_name> <key1> <key2> ...
///
/// Each key name is appended as a separate argv element so tmux treats every
/// element as an independent key stroke to dispatch in order.  No `-l`/`--` is
/// used so that key-name lookup is active for every element.
pub fn send_named_keys_args(session_name: &str, keys: &[String]) -> Vec<String> {
    let mut args = vec![
        "tmux".to_string(),
        "send-keys".to_string(),
        "-t".to_string(),
        session_name.to_string(),
    ];
    for k in keys {
        args.push(k.clone());
    }
    args
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
    if classify_no_server(&stderr) {
        bail!(TmuxError::NoServer);
    }

    let code = output.status.code().unwrap_or(-1);
    bail!(TmuxError::ExitError { code, stderr });
}

/// True when tmux stderr indicates no server is running / reachable.
/// Pure classification logic, extracted from `run_tmux` so it is unit-testable
/// without spawning a tmux process.
pub fn classify_no_server(stderr: &str) -> bool {
    stderr.contains("no server running")
        || stderr.contains("error connecting to")
        || stderr.contains("No such file or directory")
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

/// Create a detached tmux session, optionally starting in `dir`.
pub fn new_session(session_name: &str, dir: Option<&str>) -> Result<()> {
    let args = new_session_args(session_name, dir);
    run_tmux(&args).context("new-session failed")?;
    Ok(())
}

/// Remove a tmux session.
pub fn kill_session(session_name: &str) -> Result<()> {
    let args = kill_session_args(session_name);
    run_tmux(&args).context("kill-session failed")?;
    Ok(())
}

/// Send `keys` literally to `session_name`, followed by an Enter keypress.
///
/// Two tmux invocations are made:
/// 1. `send-keys -t <session> -l -- <keys>` — sends the text literally.
/// 2. `send-keys -t <session> Enter` — sends the Return key.
///
/// An unknown session surfaces as `TmuxError::ExitError`.
pub fn send_keys(session_name: &str, keys: &str) -> Result<()> {
    let literal_args = send_keys_args(session_name, keys);
    run_tmux(&literal_args).context("send-keys (literal) failed")?;

    let enter_args = send_enter_args(session_name);
    run_tmux(&enter_args).context("send-keys (Enter) failed")?;

    Ok(())
}

/// Send a single named key (e.g. `Escape`, `Enter`, `Up`, `C-c`) to
/// `session_name`.
///
/// Unlike `send_keys`, this does **not** use `-l` so tmux resolves the key
/// name.  An unknown session surfaces as `TmuxError::ExitError`.
pub fn send_named_key(session_name: &str, key: &str) -> Result<()> {
    let args = send_named_key_args(session_name, key);
    run_tmux(&args).context("send-keys (named key) failed")?;
    Ok(())
}

/// Send a sequence of named keys to `session_name` in a single tmux call.
///
/// Each element of `keys` is treated as an independent tmux key name.  An
/// empty slice is a no-op (no tmux call is made).  An unknown session surfaces
/// as `TmuxError::ExitError`.
pub fn send_named_keys(session_name: &str, keys: &[String]) -> Result<()> {
    if keys.is_empty() {
        return Ok(());
    }
    let args = send_named_keys_args(session_name, keys);
    run_tmux(&args).context("send-keys (named keys sequence) failed")?;
    Ok(())
}

/// Attach to an existing tmux session, handing the terminal to tmux.
/// Blocks until the user detaches (Ctrl-b d), then returns control.
pub fn attach_session(session_name: &str) -> Result<()> {
    let args = attach_args(session_name);
    debug_assert!(!args.is_empty(), "args must not be empty");
    let (bin, rest) = args.split_first().expect("args must not be empty");

    let status = Command::new(bin).args(rest).status().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow::Error::new(TmuxError::NotInstalled)
        } else {
            anyhow::Error::new(e).context("failed to run tmux")
        }
    })?;

    if status.success() {
        return Ok(());
    }

    let code = status.code().unwrap_or(-1);
    // When the session does not exist tmux exits non-zero; we surface that as ExitError.
    bail!(TmuxError::ExitError {
        code,
        stderr: format!("can't find session: {session_name}")
    });
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
        assert!(LIST_SESSIONS_FORMAT.contains("#{pane_current_command}"));
    }

    #[test]
    fn field_sep_matches_format_separator() {
        // Verify the const separator agrees with what we put in the format string.
        assert!(LIST_SESSIONS_FORMAT.contains(FIELD_SEP));
    }

    #[test]
    fn attach_args_correct() {
        let args = attach_args("my-session");
        assert_eq!(args[0], "tmux");
        assert_eq!(args[1], "attach");
        assert_eq!(args[2], "-t");
        assert_eq!(args[3], "my-session");
        assert_eq!(args.len(), 4);
    }

    #[test]
    fn new_session_args_without_dir() {
        let args = new_session_args("work", None);
        assert_eq!(args[0], "tmux");
        assert_eq!(args[1], "new-session");
        assert_eq!(args[2], "-d");
        assert_eq!(args[3], "-s");
        assert_eq!(args[4], "work");
        assert_eq!(args.len(), 5);
    }

    #[test]
    fn new_session_args_with_dir() {
        let args = new_session_args("work", Some("/tmp"));
        assert_eq!(args[0], "tmux");
        assert_eq!(args[1], "new-session");
        assert_eq!(args[2], "-d");
        assert_eq!(args[3], "-s");
        assert_eq!(args[4], "work");
        assert_eq!(args[5], "-c");
        assert_eq!(args[6], "/tmp");
        assert_eq!(args.len(), 7);
    }

    #[test]
    fn kill_session_args_correct() {
        let args = kill_session_args("old-session");
        assert_eq!(args[0], "tmux");
        assert_eq!(args[1], "kill-session");
        assert_eq!(args[2], "-t");
        assert_eq!(args[3], "old-session");
        assert_eq!(args.len(), 4);
    }

    // ── send-keys arg construction ──────────────────────────────────────────────

    #[test]
    fn send_keys_args_simple_command() {
        let args = send_keys_args("work", "cargo build");
        assert_eq!(args[0], "tmux");
        assert_eq!(args[1], "send-keys");
        assert_eq!(args[2], "-t");
        assert_eq!(args[3], "work");
        assert_eq!(args[4], "-l");
        assert_eq!(args[5], "--");
        assert_eq!(args[6], "cargo build");
        assert_eq!(args.len(), 7);
    }

    #[test]
    fn send_keys_args_contains_literal_flag() {
        // -l must always be present so key-name tokens are never interpreted.
        let args = send_keys_args("work", "echo Enter");
        assert!(args.contains(&"-l".to_string()), "missing -l in: {args:?}");
    }

    #[test]
    fn send_keys_args_contains_double_dash() {
        // -- must always be present so a leading hyphen is not parsed as a flag.
        let args = send_keys_args("work", "--help");
        assert!(args.contains(&"--".to_string()), "missing -- in: {args:?}");
    }

    #[test]
    fn send_keys_args_command_with_tmux_key_token() {
        // A command containing "Enter" must be a single argv element after --.
        let args = send_keys_args("work", "echo Enter");
        assert_eq!(
            args[6], "echo Enter",
            "command must be a single argv element"
        );
        assert_eq!(args.len(), 7);
    }

    #[test]
    fn send_keys_args_command_with_leading_hyphen() {
        // A command starting with - must be after --, as a single argv element.
        let args = send_keys_args("work", "--help");
        assert_eq!(args[5], "--", "-- must precede the command");
        assert_eq!(args[6], "--help", "command must be a single argv element");
        assert_eq!(args.len(), 7);
    }

    #[test]
    fn send_enter_args_correct() {
        let args = send_enter_args("work");
        assert_eq!(args[0], "tmux");
        assert_eq!(args[1], "send-keys");
        assert_eq!(args[2], "-t");
        assert_eq!(args[3], "work");
        assert_eq!(args[4], "Enter");
        assert_eq!(args.len(), 5);
        // Must NOT contain -l — that would prevent Enter being treated as the Return key.
        assert!(
            !args.contains(&"-l".to_string()),
            "-l must not appear in enter args"
        );
    }

    // ── send_named_key_args / send_named_keys_args ─────────────────────────────

    #[test]
    fn send_named_key_args_single_key() {
        let args = send_named_key_args("work", "Escape");
        assert_eq!(args[0], "tmux");
        assert_eq!(args[1], "send-keys");
        assert_eq!(args[2], "-t");
        assert_eq!(args[3], "work");
        assert_eq!(args[4], "Escape");
        assert_eq!(args.len(), 5);
    }

    #[test]
    fn send_named_key_args_no_literal_flag() {
        // -l must NOT be present — named-key lookup must remain active.
        let args = send_named_key_args("work", "Enter");
        assert!(
            !args.contains(&"-l".to_string()),
            "-l must not appear in named-key args: {args:?}"
        );
    }

    #[test]
    fn send_named_key_args_no_double_dash() {
        // -- must NOT be present — it would prevent tmux from resolving the key name.
        let args = send_named_key_args("work", "Up");
        assert!(
            !args.contains(&"--".to_string()),
            "-- must not appear in named-key args: {args:?}"
        );
    }

    #[test]
    fn send_named_key_args_arrow_keys() {
        for key in ["Up", "Down", "Left", "Right"] {
            let args = send_named_key_args("sess", key);
            assert_eq!(args[4], key, "key element mismatch for {key}");
            assert_eq!(args.len(), 5);
        }
    }

    #[test]
    fn send_named_key_args_modifier_key() {
        // Hyphen-style modifiers like C-c must be passed through as-is.
        let args = send_named_key_args("sess", "C-c");
        assert_eq!(args[4], "C-c");
        assert_eq!(args.len(), 5);
        assert!(!args.contains(&"-l".to_string()));
        assert!(!args.contains(&"--".to_string()));
    }

    #[test]
    fn send_named_keys_args_multi_key_sequence() {
        let keys: Vec<String> = vec!["Escape".to_string(), "Up".to_string(), "Enter".to_string()];
        let args = send_named_keys_args("sess", &keys);
        assert_eq!(args[0], "tmux");
        assert_eq!(args[1], "send-keys");
        assert_eq!(args[2], "-t");
        assert_eq!(args[3], "sess");
        assert_eq!(args[4], "Escape");
        assert_eq!(args[5], "Up");
        assert_eq!(args[6], "Enter");
        assert_eq!(args.len(), 7);
    }

    #[test]
    fn send_named_keys_args_no_literal_flag() {
        let keys: Vec<String> = vec!["C-c".to_string(), "Enter".to_string()];
        let args = send_named_keys_args("sess", &keys);
        assert!(
            !args.contains(&"-l".to_string()),
            "-l must not appear in named-keys args: {args:?}"
        );
        assert!(
            !args.contains(&"--".to_string()),
            "-- must not appear in named-keys args: {args:?}"
        );
    }

    #[test]
    fn send_named_keys_args_single_element() {
        let keys: Vec<String> = vec!["Escape".to_string()];
        let args = send_named_keys_args("work", &keys);
        assert_eq!(args[4], "Escape");
        assert_eq!(args.len(), 5);
    }

    #[test]
    fn send_named_keys_args_empty_yields_base_only() {
        // Empty keys slice → only tmux send-keys -t <name>, no key elements appended.
        let args = send_named_keys_args("work", &[]);
        assert_eq!(args.len(), 4);
        assert_eq!(args[0], "tmux");
        assert_eq!(args[1], "send-keys");
        assert_eq!(args[2], "-t");
        assert_eq!(args[3], "work");
    }

    // ── stderr classification (#2) ──────────────────────────────────────────────

    #[test]
    fn classify_no_server_matches_no_server_running() {
        assert!(classify_no_server(
            "no server running on /tmp/tmux-501/default"
        ));
    }

    #[test]
    fn classify_no_server_matches_error_connecting() {
        assert!(classify_no_server(
            "error connecting to /tmp/tmux-501/default (No such file)"
        ));
    }

    #[test]
    fn classify_no_server_matches_no_such_file() {
        assert!(classify_no_server("No such file or directory"));
    }

    #[test]
    fn classify_no_server_rejects_unrelated_stderr() {
        assert!(!classify_no_server("duplicate session: work"));
        assert!(!classify_no_server("can't find session: nope"));
    }

    #[test]
    fn classify_no_server_rejects_empty() {
        assert!(!classify_no_server(""));
    }
}
