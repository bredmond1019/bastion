// sessions/ask.rs — `bastion ask` implementation.
//
// Performs a single Claude Code "turn" against an interactive tmux session:
//   ensure session + Claude → send trigger → wait for done-marker → exit.
//
// Decision D4: DB-free. No Config::load(), no Postgres pool.
// Decision D5: synchronous blocking — no tokio/async.
// Decision D6: malformed tmux output is skipped with a warning, not fatal.
//
// Contract: brain doc `docs/integrations/claude-code-llm-provider.md` §2 (v0.1.0).

use crate::sessions::claude_state::{TrustStatus, trust_status};
use crate::sessions::model::{SessionState, classify_state, parse_sessions};
use crate::sessions::tmux;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Polling interval for the done-marker wait loop (milliseconds).
pub const POLL_INTERVAL_MS: u64 = 500;

/// Readiness-wait timeout when launching Claude into a fresh session (seconds).
pub const READINESS_TIMEOUT_SECS: u64 = 30;

/// Polling interval for the Claude-readiness wait (milliseconds).
pub const READINESS_POLL_MS: u64 = 500;

// ── Args struct ───────────────────────────────────────────────────────────────

/// Arguments for the `ask` turn — mirrors the clap `Commands::Ask` fields.
/// Passed by `main.rs` after extracting from the parsed CLI struct.
pub struct AskArgs {
    pub session: String,
    pub prompt_file: PathBuf,
    pub out: PathBuf,
    pub dir: Option<PathBuf>,
    pub timeout_secs: u64,
    pub launch_cmd: String,
}

// ── Error type ────────────────────────────────────────────────────────────────

/// Errors produced by `ask`.
#[derive(Debug, thiserror::Error)]
pub enum AskError {
    /// `--dir` is explicitly marked Untrusted in `~/.claude.json`; Claude would
    /// stall on the one-time workspace-trust prompt.
    #[error(
        "directory '{0}' is untrusted (hasTrustDialogAccepted=false in ~/.claude.json); \
         Claude would stall on the workspace-trust prompt — open the directory in Claude \
         interactively once to accept trust, then retry"
    )]
    UntrustedDir(String),

    /// A tmux command failed.
    #[error("tmux error during {op}: {source}")]
    Tmux {
        op: String,
        #[source]
        source: anyhow::Error,
    },

    /// Claude did not become ready within the readiness budget.
    #[error(
        "Claude did not become ready in session '{session}' within {timeout_secs}s after launch"
    )]
    Launch { session: String, timeout_secs: u64 },

    /// `--out` was not written within `--timeout` seconds.
    #[error("timed out after {timeout_secs}s waiting for '{out}'; captured pane:\n{pane_output}")]
    Timeout {
        timeout_secs: u64,
        out: String,
        pane_output: String,
    },
}

// ── Pure helpers ──────────────────────────────────────────────────────────────

/// Derive the done-marker path from `out`: append `.done` to the full filename.
///
/// Examples:
/// - `/tmp/answer.json`  → `/tmp/answer.json.done`
/// - `/tmp/answer`       → `/tmp/answer.done`
pub fn done_path(out: &Path) -> PathBuf {
    let mut name = out.file_name().unwrap_or_default().to_os_string();
    name.push(".done");
    out.with_file_name(name)
}

/// Build the fixed trigger text sent to Claude.
///
/// Contract (v0.1.0): the exact wording from
/// `docs/integrations/claude-code-llm-provider.md` §2 — flag names and marker
/// filename must match verbatim.
pub fn trigger_text(prompt_file: &Path, out: &Path) -> String {
    let done = done_path(out);
    format!(
        "Read {} and follow its instructions exactly. \
         Write your complete answer to {}. \
         When finished, create an empty file {}",
        prompt_file.display(),
        out.display(),
        done.display(),
    )
}

/// Pure computation of the maximum number of poll attempts.
///
/// `timeout_secs * 1000 / interval_ms`, rounding up so that a fractional
/// remainder still gets one more attempt.
pub fn poll_plan(timeout_secs: u64, interval_ms: u64) -> usize {
    if interval_ms == 0 {
        return 0;
    }
    let total_ms = timeout_secs.saturating_mul(1000);
    total_ms.div_ceil(interval_ms) as usize
}

/// Returns the argument list for `tmux has-session -t <name>`.
/// Exits 0 if the session exists, 1 if not.
pub fn has_session_args(name: &str) -> Vec<String> {
    vec![
        "tmux".to_string(),
        "has-session".to_string(),
        "-t".to_string(),
        name.to_string(),
    ]
}

/// Check whether a tmux session with `name` currently exists.
/// Returns `true` if it exists, `false` otherwise (including when tmux is not
/// installed or no server is running).
pub fn has_session(name: &str) -> bool {
    let args = has_session_args(name);
    tmux::run_tmux(&args).is_ok()
}

// ── I/O shell ─────────────────────────────────────────────────────────────────

/// Run a single Claude Code "turn" against an interactive tmux session.
///
/// Steps:
///   1. Trust pre-flight — fail fast if `--dir` is explicitly Untrusted.
///   2. Ensure session + Claude — create session and/or launch Claude when cold,
///      skip launch when `classify_state` already reports `claude` running.
///   3. Send the trigger — the only keystrokes sent.
///   4. Wait for completion — poll `done_path(--out)` up to `--timeout`; on
///      found, remove the marker and return `Ok(())`; on timeout, capture
///      the pane and return `AskError::Timeout`.
pub fn ask(args: AskArgs) -> Result<(), AskError> {
    // ── 1. Trust pre-flight ──────────────────────────────────────────────────
    if let Some(ref dir) = args.dir {
        let dir_str = dir.to_string_lossy();
        if trust_status(&dir_str) == TrustStatus::Untrusted {
            return Err(AskError::UntrustedDir(dir_str.into_owned()));
        }
    }

    // ── 2. Ensure session + Claude ───────────────────────────────────────────
    let dir_str: Option<String> = args.dir.as_ref().map(|p| p.to_string_lossy().into_owned());

    if !has_session(&args.session) {
        // Create a new detached session.
        tmux::new_session(&args.session, dir_str.as_deref()).map_err(|e| AskError::Tmux {
            op: "new-session".to_string(),
            source: e,
        })?;

        // Launch Claude.
        tmux::send_keys(&args.session, &args.launch_cmd).map_err(|e| AskError::Tmux {
            op: "send-keys (launch)".to_string(),
            source: e,
        })?;

        // Wait for Claude to become the foreground process.
        wait_for_claude(&args.session, READINESS_TIMEOUT_SECS, READINESS_POLL_MS)?;
    } else {
        // Session exists — check whether Claude is already the foreground process.
        let foreground = foreground_cmd_for(&args.session);
        if classify_state(&foreground) != SessionState::Running || foreground.trim() != "claude" {
            // Session exists but Claude is not running — launch it.
            tmux::send_keys(&args.session, &args.launch_cmd).map_err(|e| AskError::Tmux {
                op: "send-keys (launch into existing session)".to_string(),
                source: e,
            })?;
            wait_for_claude(&args.session, READINESS_TIMEOUT_SECS, READINESS_POLL_MS)?;
        }
        // else: Claude is already running → skip launch.
    }

    // ── 3. Send the trigger ──────────────────────────────────────────────────
    let trigger = trigger_text(&args.prompt_file, &args.out);
    tmux::send_keys(&args.session, &trigger).map_err(|e| AskError::Tmux {
        op: "send-keys (trigger)".to_string(),
        source: e,
    })?;

    // ── 4. Wait for completion ───────────────────────────────────────────────
    let done = done_path(&args.out);
    let max_attempts = poll_plan(args.timeout_secs, POLL_INTERVAL_MS);

    for _ in 0..max_attempts {
        if done.exists() {
            // Marker found — remove it and return success.
            let _ = std::fs::remove_file(&done);
            return Ok(());
        }
        thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
    }

    // Timed out — capture the pane for diagnostics.
    let pane_output = tmux::capture_pane_raw(&args.session)
        .unwrap_or_else(|_| "(capture-pane failed)".to_string());

    Err(AskError::Timeout {
        timeout_secs: args.timeout_secs,
        out: args.out.display().to_string(),
        pane_output,
    })
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Poll `list-sessions` until the target session's foreground command is `claude`,
/// or until `timeout_secs` elapses.
fn wait_for_claude(session: &str, timeout_secs: u64, interval_ms: u64) -> Result<(), AskError> {
    let max_attempts = poll_plan(timeout_secs, interval_ms);

    for _ in 0..max_attempts {
        let foreground = foreground_cmd_for(session);
        if foreground.trim() == "claude" {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(interval_ms));
    }

    Err(AskError::Launch {
        session: session.to_string(),
        timeout_secs,
    })
}

/// Return the foreground pane command for `session` by parsing `list-sessions`.
/// Returns an empty string if the session is not found or output is malformed.
fn foreground_cmd_for(session: &str) -> String {
    let Ok(raw) = tmux::list_sessions_raw() else {
        return String::new();
    };
    parse_sessions(&raw)
        .into_iter()
        .find(|s| s.name == session)
        .map(|s| s.foreground_cmd)
        .unwrap_or_default()
}

// ── Tests (pure, no live tmux) ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use std::path::PathBuf;

    // ── done_path ─────────────────────────────────────────────────────────────

    #[test]
    fn done_path_with_extension() {
        let out = PathBuf::from("/tmp/answer.json");
        let done = done_path(&out);
        assert_eq!(done, PathBuf::from("/tmp/answer.json.done"));
    }

    #[test]
    fn done_path_without_extension() {
        let out = PathBuf::from("/tmp/answer");
        let done = done_path(&out);
        assert_eq!(done, PathBuf::from("/tmp/answer.done"));
    }

    #[test]
    fn done_path_preserves_parent_directory() {
        let out = PathBuf::from("/home/user/project/out.txt");
        let done = done_path(&out);
        assert_eq!(done, PathBuf::from("/home/user/project/out.txt.done"));
    }

    #[test]
    fn done_path_simple_filename() {
        let out = PathBuf::from("/var/tmp/result.md");
        let done = done_path(&out);
        assert_eq!(done, PathBuf::from("/var/tmp/result.md.done"));
    }

    // ── trigger_text ──────────────────────────────────────────────────────────

    #[test]
    fn trigger_text_contains_prompt_file_path() {
        let prompt = PathBuf::from("/tmp/prompt.txt");
        let out = PathBuf::from("/tmp/answer.json");
        let text = trigger_text(&prompt, &out);
        assert!(
            text.contains("/tmp/prompt.txt"),
            "trigger should contain prompt path: {text}"
        );
    }

    #[test]
    fn trigger_text_contains_out_path() {
        let prompt = PathBuf::from("/tmp/prompt.txt");
        let out = PathBuf::from("/tmp/answer.json");
        let text = trigger_text(&prompt, &out);
        assert!(
            text.contains("/tmp/answer.json"),
            "trigger should contain out path: {text}"
        );
    }

    #[test]
    fn trigger_text_contains_done_marker_path() {
        let prompt = PathBuf::from("/tmp/prompt.txt");
        let out = PathBuf::from("/tmp/answer.json");
        let text = trigger_text(&prompt, &out);
        assert!(
            text.contains("/tmp/answer.json.done"),
            "trigger should contain done marker path: {text}"
        );
    }

    #[test]
    fn trigger_text_contract_wording() {
        let prompt = PathBuf::from("/tmp/p.txt");
        let out = PathBuf::from("/tmp/o.json");
        let text = trigger_text(&prompt, &out);
        assert!(
            text.contains("Read "),
            "trigger must start with 'Read': {text}"
        );
        assert!(
            text.contains("follow its instructions exactly"),
            "trigger must contain 'follow its instructions exactly': {text}"
        );
        assert!(
            text.contains("Write your complete answer to"),
            "trigger must contain 'Write your complete answer to': {text}"
        );
        assert!(
            text.contains("create an empty file"),
            "trigger must contain 'create an empty file': {text}"
        );
    }

    #[test]
    fn trigger_text_absolute_paths_present() {
        let prompt = PathBuf::from("/absolute/prompt.txt");
        let out = PathBuf::from("/absolute/out.json");
        let text = trigger_text(&prompt, &out);
        // Both the prompt and out paths must appear as absolute paths.
        assert!(
            text.contains("/absolute/prompt.txt"),
            "absolute prompt path missing: {text}"
        );
        assert!(
            text.contains("/absolute/out.json"),
            "absolute out path missing: {text}"
        );
    }

    // ── poll_plan ─────────────────────────────────────────────────────────────

    #[test]
    fn poll_plan_rounds_up() {
        // 1s / 500ms = 2 exactly
        assert_eq!(poll_plan(1, 500), 2);
    }

    #[test]
    fn poll_plan_fractional_rounds_up() {
        // 1s / 300ms = 3.33... → 4
        assert_eq!(poll_plan(1, 300), 4);
    }

    #[test]
    fn poll_plan_180s_500ms() {
        // 180s / 500ms = 360 attempts
        assert_eq!(poll_plan(180, 500), 360);
    }

    #[test]
    fn poll_plan_zero_timeout() {
        assert_eq!(poll_plan(0, 500), 0);
    }

    #[test]
    fn poll_plan_zero_interval_returns_zero() {
        // Guard against divide-by-zero.
        assert_eq!(poll_plan(60, 0), 0);
    }

    #[test]
    fn poll_plan_one_second_one_ms() {
        // 1s / 1ms = 1000 attempts
        assert_eq!(poll_plan(1, 1), 1000);
    }

    // ── has_session_args ──────────────────────────────────────────────────────

    #[test]
    fn has_session_args_correct() {
        let args = has_session_args("my-session");
        assert_eq!(args[0], "tmux");
        assert_eq!(args[1], "has-session");
        assert_eq!(args[2], "-t");
        assert_eq!(args[3], "my-session");
        assert_eq!(args.len(), 4);
    }

    #[test]
    fn has_session_args_uses_provided_name() {
        let args = has_session_args("ask-smoke");
        assert_eq!(args[3], "ask-smoke");
    }

    // ── AskError display ──────────────────────────────────────────────────────

    #[test]
    fn ask_error_untrusted_dir_message_contains_dir() {
        let err = AskError::UntrustedDir("/some/untrusted/dir".to_string());
        let msg = err.to_string();
        assert!(
            msg.contains("/some/untrusted/dir"),
            "error message should contain the dir: {msg}"
        );
        assert!(
            msg.contains("untrusted"),
            "error message should mention 'untrusted': {msg}"
        );
    }

    #[test]
    fn ask_error_timeout_message_contains_timeout_and_out() {
        let err = AskError::Timeout {
            timeout_secs: 60,
            out: "/tmp/answer.json".to_string(),
            pane_output: "some pane output".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("60"), "should contain timeout: {msg}");
        assert!(
            msg.contains("/tmp/answer.json"),
            "should contain out path: {msg}"
        );
    }

    #[test]
    fn ask_error_launch_message_contains_session_and_timeout() {
        let err = AskError::Launch {
            session: "ask-smoke".to_string(),
            timeout_secs: 30,
        };
        let msg = err.to_string();
        assert!(msg.contains("ask-smoke"), "should contain session: {msg}");
        assert!(msg.contains("30"), "should contain timeout: {msg}");
    }

    #[test]
    fn ask_error_tmux_message_contains_op() {
        let err = AskError::Tmux {
            op: "new-session".to_string(),
            source: anyhow!("tmux exited with code 1"),
        };
        let msg = err.to_string();
        assert!(msg.contains("new-session"), "should contain op name: {msg}");
    }

    // ── DB-free guarantee ─────────────────────────────────────────────────────

    /// Architectural guarantee: none of the pure functions on this module require
    /// a DATABASE_URL. This test removes it and calls every pure helper.
    #[test]
    fn pure_helpers_require_no_database_url() {
        // Safety: single-threaded test; no other thread reads this env var.
        unsafe { std::env::remove_var("DATABASE_URL") };

        let prompt = PathBuf::from("/tmp/prompt.txt");
        let out = PathBuf::from("/tmp/answer.json");

        // These must not panic or return a config error.
        let _ = done_path(&out);
        let _ = trigger_text(&prompt, &out);
        let _ = poll_plan(180, 500);
        let _ = has_session_args("test-session");
        // No assertion needed beyond "this line is reached".
    }
}
