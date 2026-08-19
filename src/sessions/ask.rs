// sessions/ask.rs — `bastion ask` implementation.
//
// Performs a single Claude Code "turn" against an interactive tmux session:
//   ensure session + Claude → send trigger → wait for done-marker → exit.
//
// Decision D4: DB-free. No Config::load(), no Postgres pool.
// Decision D5: synchronous blocking — no tokio/async.
// Decision D6: malformed tmux output is skipped with a warning, not fatal.
//
// Contract: brain doc `docs/integrations/claude-code-llm-provider.md` §2 (v0.2.0).

use crate::sessions::claude_state::{TrustStatus, trust_status};
use crate::sessions::model::{SessionState, classify_state, parse_sessions};
use crate::sessions::tmux;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime};
use uuid::Uuid;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Polling interval for the done-marker wait loop (milliseconds).
pub const POLL_INTERVAL_MS: u64 = 500;

/// Readiness-wait timeout when launching Claude into a fresh session (seconds).
pub const READINESS_TIMEOUT_SECS: u64 = 30;

/// Polling interval for the Claude-readiness wait (milliseconds).
pub const READINESS_POLL_MS: u64 = 500;

/// Number of consecutive "pane content is stable and non-empty" observations
/// required before `wait_for_claude` declares readiness.
///
/// A single non-shell foreground-process sample is not sufficient: Claude's
/// TUI keeps rendering (splash screen, MCP-auth check, entering alt-screen
/// mode) for a stretch *after* the process has already been exec'd, and a
/// command sent into that window is silently dropped (reproduced live: an
/// empty prompt persisted 20+ seconds after the old single-sample readiness
/// check declared success). Requiring the captured pane content to be
/// identical across `REQUIRED_STABLE_OBSERVATIONS` consecutive poll ticks
/// means the wait spans the entire duration the pane is still actively
/// re-rendering, not just one instant of it.
pub const REQUIRED_STABLE_OBSERVATIONS: usize = 2;

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

/// Generate a per-invocation nonce for the done-marker contract (v0.2.0).
///
/// Uses the simple (no-hyphen) hex form of a v4 UUID so the value is
/// filename-safe by construction and does not need escaping when embedded in
/// a path or a trigger string.
pub fn generate_nonce() -> String {
    Uuid::new_v4().simple().to_string()
}

/// Derive the nonce'd done-marker path from `out`: append `.{nonce}.done` to
/// the full filename, preserving the parent directory exactly.
///
/// Examples:
/// - `/tmp/answer.json`, `"abc"`  → `/tmp/answer.json.abc.done`
/// - `/tmp/answer`, `"abc"`       → `/tmp/answer.abc.done`
pub fn done_path(out: &Path, nonce: &str) -> PathBuf {
    let mut name = out.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(nonce);
    name.push(".done");
    out.with_file_name(name)
}

/// Derive the legacy (v0.1.0) bare done-marker path from `out`: append
/// `.done` to the full filename, with no nonce.
///
/// This is the dual-read-window form — task 4 accepts it alongside the
/// nonce'd `done_path` above for one release. Kept under this name so the
/// eventual removal at window close is a one-symbol grep.
///
/// Examples:
/// - `/tmp/answer.json`  → `/tmp/answer.json.done`
/// - `/tmp/answer`       → `/tmp/answer.done`
pub fn legacy_done_path(out: &Path) -> PathBuf {
    let mut name = out.file_name().unwrap_or_default().to_os_string();
    name.push(".done");
    out.with_file_name(name)
}

/// Build the fixed trigger text sent to Claude.
///
/// Contract (v0.2.0): the exact wording from
/// `docs/integrations/claude-code-llm-provider.md` §2 — flag names and marker
/// filename must match verbatim. The marker must be created containing
/// exactly the nonce text (not empty) — this is what lets the wait loop tell
/// this invocation's evidence apart from a stale marker left by a prior run.
pub fn trigger_text(prompt_file: &Path, out: &Path, nonce: &str) -> String {
    let done = done_path(out, nonce);
    format!(
        "Read {} and follow its instructions exactly. \
         Write your complete answer to {}. \
         When finished, create a file {} containing exactly the text {}",
        prompt_file.display(),
        out.display(),
        done.display(),
        nonce,
    )
}

/// Contract rule 2: does `marker_content` satisfy the nonce check?
///
/// True only when the content, trimmed of surrounding whitespace, equals
/// `nonce` exactly. Empty, whitespace-only, mismatched, and truncated
/// content are all false — a marker whose bytes do not prove *this*
/// invocation wrote it must never be treated as evidence the turn completed.
pub fn marker_satisfies(marker_content: &str, nonce: &str) -> bool {
    marker_content.trim() == nonce
}

/// Contract rule 4: does `out`'s mtime postdate the send?
///
/// Strictly greater-than — an mtime equal to `send_time` is NOT fresh. A
/// filesystem with coarse mtime granularity (e.g. 1s resolution on some
/// platforms/filesystems) could otherwise let a stale `--out` written in the
/// same tick as the send pass the check, which is the exact defect this
/// rule exists to close.
pub fn out_is_fresh(out_mtime: SystemTime, send_time: SystemTime) -> bool {
    out_mtime > send_time
}

/// Contract rules 2 and 4 composed: is this turn complete?
///
/// True only when the marker's content proves it belongs to this invocation
/// (rule 2) AND `--out`'s mtime postdates the send (rule 4). Used so the
/// wait loop's condition is one tested pure call rather than inline logic.
pub fn turn_is_complete(
    marker_content: &str,
    nonce: &str,
    out_mtime: SystemTime,
    send_time: SystemTime,
) -> bool {
    marker_satisfies(marker_content, nonce) && out_is_fresh(out_mtime, send_time)
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

/// Pure predicate: is this pane-content sample a "ready" observation?
///
/// A sample counts as ready when it is non-empty (a blank/whitespace-only
/// capture cannot be a settled prompt) *and* identical to the previous
/// sample — i.e. the pane stopped re-rendering between the two polls that
/// produced them. `previous == None` (first observation, or the streak was
/// just reset) can never count, since there is nothing yet to compare
/// against.
pub(crate) fn pane_is_stable_and_ready(previous: Option<&str>, current: &str) -> bool {
    if current.trim().is_empty() {
        return false;
    }
    match previous {
        Some(prev) => prev == current,
        None => false,
    }
}

/// Pure state transition for the readiness streak counter: bump on a ready
/// observation, reset to zero on anything else (non-shell-but-unstable pane,
/// or the foreground process reverting to a shell).
pub(crate) fn update_readiness_streak(is_ready_observation: bool, streak: usize) -> usize {
    if is_ready_observation {
        streak.saturating_add(1)
    } else {
        0
    }
}

// ── I/O shell ─────────────────────────────────────────────────────────────────

/// Run a single Claude Code "turn" against an interactive tmux session.
///
/// Steps:
///   1. Trust pre-flight — fail fast if `--dir` is explicitly Untrusted.
///   2. Ensure session + Claude — create session and/or launch Claude when cold,
///      skip launch when `classify_state` reports a non-shell process running
///      (covers `"claude"` as well as version-string names like `"2.1.185"`).
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

    ensure_session_with_claude(&args.session, dir_str.as_deref(), &args.launch_cmd)?;

    // ── 3. Send the trigger ──────────────────────────────────────────────────
    let nonce = generate_nonce();
    let trigger = trigger_text(&args.prompt_file, &args.out, &nonce);
    // Recorded immediately BEFORE send_keys, not after: a send that blocks
    // would otherwise stamp a time later than a fast reply's write and
    // reject a legitimate answer (contract rule 4).
    let send_time = SystemTime::now();
    tmux::send_keys(&args.session, &trigger).map_err(|e| AskError::Tmux {
        op: "send-keys (trigger)".to_string(),
        source: e.into(),
    })?;

    // ── 4. Wait for completion ───────────────────────────────────────────────
    let done = done_path(&args.out, &nonce);
    let max_attempts = poll_plan(args.timeout_secs, POLL_INTERVAL_MS);

    for _ in 0..max_attempts {
        // A marker that exists but fails the check is not an error and not
        // a success — keep polling until the timeout.
        if let Ok(marker_content) = std::fs::read_to_string(&done)
            && let Ok(out_mtime) = std::fs::metadata(&args.out).and_then(|m| m.modified())
            && turn_is_complete(&marker_content, &nonce, out_mtime, send_time)
        {
            // Marker satisfied — remove it and return success.
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

/// Ensure a tmux session named `session` exists and has Claude running as its
/// foreground process, launching it with `launch_cmd` if needed.
///
/// This is `ask()` steps 1–2 (ensure-session-with-claude), extracted so the
/// `bastion serve` quick-action handler (`mode:"spawn"`) can reuse the exact
/// same spawn/readiness mechanics without duplicating them. Behaviour:
///   - No session yet → create it (`tmux::new_session`), send `launch_cmd`,
///     wait for readiness via `wait_for_claude`.
///   - Session exists but Claude isn't the foreground process → send
///     `launch_cmd`, wait for readiness.
///   - Session exists and Claude is already running → no-op.
pub(crate) fn ensure_session_with_claude(
    session: &str,
    dir: Option<&str>,
    launch_cmd: &str,
) -> Result<(), AskError> {
    ensure_session_with_claude_with_timeout(
        session,
        dir,
        launch_cmd,
        READINESS_TIMEOUT_SECS,
        READINESS_POLL_MS,
    )
}

/// Same as [`ensure_session_with_claude`] but with the readiness-wait timeout
/// and poll interval as explicit parameters, so tests can exercise the full
/// create+launch+wait path without paying the full production timeout
/// (`READINESS_TIMEOUT_SECS`). Production callers go through the public
/// `ensure_session_with_claude` wrapper, which always uses the real
/// timeout/interval constants.
fn ensure_session_with_claude_with_timeout(
    session: &str,
    dir: Option<&str>,
    launch_cmd: &str,
    timeout_secs: u64,
    interval_ms: u64,
) -> Result<(), AskError> {
    if !has_session(session) {
        // Create a new detached session.
        tmux::new_session(session, dir).map_err(|e| AskError::Tmux {
            op: "new-session".to_string(),
            source: e.into(),
        })?;

        // Launch Claude.
        tmux::send_keys(session, launch_cmd).map_err(|e| AskError::Tmux {
            op: "send-keys (launch)".to_string(),
            source: e.into(),
        })?;

        // Wait for Claude to become the foreground process.
        wait_for_claude(session, timeout_secs, interval_ms)?;
    } else {
        // Session exists — check whether Claude is already the foreground process.
        // Use `classify_state` rather than checking for `"claude"` by name: modern
        // Claude Code renames its process to its version string (e.g. "2.1.185"),
        // so any non-idle-shell foreground command signals Claude is running.
        let foreground = foreground_cmd_for(session);
        if classify_state(&foreground) != SessionState::Running {
            // Session exists but Claude is not running — launch it.
            tmux::send_keys(session, launch_cmd).map_err(|e| AskError::Tmux {
                op: "send-keys (launch into existing session)".to_string(),
                source: e.into(),
            })?;
            wait_for_claude(session, timeout_secs, interval_ms)?;
        }
        // else: Claude is already running → skip launch.
    }

    Ok(())
}

/// Poll `list-sessions` until the target session's foreground command is a
/// non-shell process (i.e. `classify_state` returns `Running`) *and* the
/// captured pane content has settled — identical across
/// [`REQUIRED_STABLE_OBSERVATIONS`] consecutive poll ticks — or until
/// `timeout_secs` elapses.
///
/// We use `classify_state` rather than checking for the string `"claude"`
/// because Claude Code renames its process via `pthread_setname_np` to the
/// version string (e.g. `"2.1.185"`), so `#{pane_current_command}` in tmux
/// never shows `"claude"` when a modern Claude Code is running.  Any
/// non-idle-shell foreground command is a reliable signal that *a* process
/// has exec'd — but not that Claude's own TUI has finished starting up
/// (splash render, MCP-auth check, entering its raw-input/alt-screen mode)
/// and is actually reading stdin into its prompt box. A command sent into
/// that gap is silently dropped, not queued or errored. Requiring the
/// rendered pane content to stop changing across consecutive polls closes
/// that gap: as long as Claude's own startup is still redrawing the pane, no
/// two consecutive samples will match, so the streak keeps resetting until
/// startup is genuinely done — however long that takes, and however far
/// past the process-exec instant it runs.
pub(crate) fn wait_for_claude(
    session: &str,
    timeout_secs: u64,
    interval_ms: u64,
) -> Result<(), AskError> {
    let max_attempts = poll_plan(timeout_secs, interval_ms);
    let mut streak: usize = 0;
    let mut last_pane: Option<String> = None;

    for _ in 0..max_attempts {
        let foreground = foreground_cmd_for(session);
        if classify_state(&foreground) == SessionState::Running {
            let pane = tmux::capture_pane_raw(session).unwrap_or_default();
            let is_ready_observation = pane_is_stable_and_ready(last_pane.as_deref(), &pane);
            streak = update_readiness_streak(is_ready_observation, streak);
            last_pane = Some(pane);

            if streak >= REQUIRED_STABLE_OBSERVATIONS {
                return Ok(());
            }
        } else {
            streak = 0;
            last_pane = None;
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

    // ── generate_nonce ────────────────────────────────────────────────────────

    #[test]
    fn generate_nonce_is_non_empty_and_filename_safe() {
        let nonce = generate_nonce();
        assert!(!nonce.is_empty(), "nonce must not be empty");
        assert!(
            nonce.chars().all(|c| c.is_ascii_alphanumeric()),
            "nonce must be filename-safe (alphanumeric only): {nonce}"
        );
    }

    #[test]
    fn generate_nonce_two_calls_differ() {
        let a = generate_nonce();
        let b = generate_nonce();
        assert_ne!(a, b, "two generate_nonce() calls must not collide");
    }

    // ── done_path ─────────────────────────────────────────────────────────────

    #[test]
    fn done_path_with_extension() {
        let out = PathBuf::from("/tmp/answer.json");
        let done = done_path(&out, "abc");
        assert_eq!(done, PathBuf::from("/tmp/answer.json.abc.done"));
    }

    #[test]
    fn done_path_without_extension() {
        let out = PathBuf::from("/tmp/answer");
        let done = done_path(&out, "abc");
        assert_eq!(done, PathBuf::from("/tmp/answer.abc.done"));
    }

    #[test]
    fn done_path_preserves_parent_directory() {
        let out = PathBuf::from("/home/user/project/out.txt");
        let done = done_path(&out, "nonce123");
        assert_eq!(
            done,
            PathBuf::from("/home/user/project/out.txt.nonce123.done")
        );
    }

    #[test]
    fn done_path_simple_filename() {
        let out = PathBuf::from("/var/tmp/result.md");
        let done = done_path(&out, "xyz");
        assert_eq!(done, PathBuf::from("/var/tmp/result.md.xyz.done"));
    }

    #[test]
    fn done_path_places_nonce_between_filename_and_done() {
        let out = PathBuf::from("/tmp/answer.json");
        let done = done_path(&out, "abc");
        assert_eq!(
            done.file_name().unwrap().to_str().unwrap(),
            "answer.json.abc.done"
        );
    }

    // ── legacy_done_path ──────────────────────────────────────────────────────

    #[test]
    fn legacy_done_path_is_bare_v0_1_0_form() {
        let out = PathBuf::from("/tmp/answer.json");
        let done = legacy_done_path(&out);
        assert_eq!(done, PathBuf::from("/tmp/answer.json.done"));
    }

    #[test]
    fn legacy_done_path_preserves_parent_directory() {
        let out = PathBuf::from("/home/user/project/out.txt");
        let done = legacy_done_path(&out);
        assert_eq!(done, PathBuf::from("/home/user/project/out.txt.done"));
    }

    // ── trigger_text ──────────────────────────────────────────────────────────

    #[test]
    fn trigger_text_contains_prompt_file_path() {
        let prompt = PathBuf::from("/tmp/prompt.txt");
        let out = PathBuf::from("/tmp/answer.json");
        let text = trigger_text(&prompt, &out, "abc");
        assert!(
            text.contains("/tmp/prompt.txt"),
            "trigger should contain prompt path: {text}"
        );
    }

    #[test]
    fn trigger_text_contains_out_path() {
        let prompt = PathBuf::from("/tmp/prompt.txt");
        let out = PathBuf::from("/tmp/answer.json");
        let text = trigger_text(&prompt, &out, "abc");
        assert!(
            text.contains("/tmp/answer.json"),
            "trigger should contain out path: {text}"
        );
    }

    #[test]
    fn trigger_text_contains_nonced_done_marker_path() {
        let prompt = PathBuf::from("/tmp/prompt.txt");
        let out = PathBuf::from("/tmp/answer.json");
        let text = trigger_text(&prompt, &out, "abc123");
        assert!(
            text.contains("/tmp/answer.json.abc123.done"),
            "trigger should contain nonce'd done marker path: {text}"
        );
    }

    #[test]
    fn trigger_text_instructs_marker_content_equals_nonce() {
        let prompt = PathBuf::from("/tmp/prompt.txt");
        let out = PathBuf::from("/tmp/answer.json");
        let text = trigger_text(&prompt, &out, "thenonce");
        assert!(
            text.contains("thenonce"),
            "trigger should mention the nonce as required marker content: {text}"
        );
        assert!(
            text.contains("containing exactly the text"),
            "trigger should instruct the marker content requirement: {text}"
        );
    }

    #[test]
    fn trigger_text_contract_wording() {
        let prompt = PathBuf::from("/tmp/p.txt");
        let out = PathBuf::from("/tmp/o.json");
        let text = trigger_text(&prompt, &out, "n1");
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
            text.contains("containing exactly the text"),
            "trigger must instruct the marker to contain the nonce: {text}"
        );
    }

    #[test]
    fn trigger_text_absolute_paths_present() {
        let prompt = PathBuf::from("/absolute/prompt.txt");
        let out = PathBuf::from("/absolute/out.json");
        let text = trigger_text(&prompt, &out, "n2");
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

    // ── marker_satisfies ──────────────────────────────────────────────────────

    #[test]
    fn marker_satisfies_exact_match() {
        assert!(marker_satisfies("abc123", "abc123"));
    }

    #[test]
    fn marker_satisfies_empty_content_false() {
        assert!(!marker_satisfies("", "abc123"));
    }

    #[test]
    fn marker_satisfies_whitespace_only_false() {
        assert!(!marker_satisfies("   \n\t  ", "abc123"));
    }

    #[test]
    fn marker_satisfies_wrong_nonce_false() {
        assert!(!marker_satisfies("xyz789", "abc123"));
    }

    #[test]
    fn marker_satisfies_trims_surrounding_whitespace() {
        assert!(marker_satisfies("  abc123\n", "abc123"));
    }

    #[test]
    fn marker_satisfies_prefix_of_content_false() {
        // Content that merely starts with the nonce (e.g. truncated write on
        // top of a longer stale value) must not satisfy — the match is exact.
        assert!(!marker_satisfies("abc123extra", "abc123"));
    }

    // ── out_is_fresh ──────────────────────────────────────────────────────────

    #[test]
    fn out_is_fresh_true_when_strictly_after() {
        let send_time = SystemTime::now();
        let out_mtime = send_time + Duration::from_secs(1);
        assert!(out_is_fresh(out_mtime, send_time));
    }

    #[test]
    fn out_is_fresh_false_when_equal() {
        let t = SystemTime::now();
        assert!(!out_is_fresh(t, t));
    }

    #[test]
    fn out_is_fresh_false_when_before() {
        let send_time = SystemTime::now();
        let out_mtime = send_time - Duration::from_secs(1);
        assert!(!out_is_fresh(out_mtime, send_time));
    }

    // ── turn_is_complete ──────────────────────────────────────────────────────

    #[test]
    fn turn_is_complete_true_when_both_hold() {
        let send_time = SystemTime::now();
        let out_mtime = send_time + Duration::from_secs(1);
        assert!(turn_is_complete("abc123", "abc123", out_mtime, send_time));
    }

    #[test]
    fn turn_is_complete_false_when_marker_mismatched() {
        let send_time = SystemTime::now();
        let out_mtime = send_time + Duration::from_secs(1);
        assert!(!turn_is_complete("wrong", "abc123", out_mtime, send_time));
    }

    #[test]
    fn turn_is_complete_false_when_out_stale() {
        let send_time = SystemTime::now();
        let out_mtime = send_time - Duration::from_secs(1);
        assert!(!turn_is_complete("abc123", "abc123", out_mtime, send_time));
    }

    #[test]
    fn turn_is_complete_false_when_both_fail() {
        let send_time = SystemTime::now();
        let out_mtime = send_time - Duration::from_secs(1);
        assert!(!turn_is_complete("wrong", "abc123", out_mtime, send_time));
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
        // The old "single-threaded test" safety note was wrong — under
        // `cargo test` this is one thread of a shared process, and the bare
        // `remove_var` both raced concurrent readers and never restored the
        // value. See `crate::testsupport` for the crate-wide discipline.
        let env_lock = crate::testsupport::lock_env();
        let _database_url = crate::testsupport::EnvVarGuard::unset(&env_lock, "DATABASE_URL");

        let prompt = PathBuf::from("/tmp/prompt.txt");
        let out = PathBuf::from("/tmp/answer.json");

        // These must not panic or return a config error.
        let _ = done_path(&out, "nonce");
        let _ = trigger_text(&prompt, &out, "nonce");
        let _ = poll_plan(180, 500);
        let _ = has_session_args("test-session");
        // No assertion needed beyond "this line is reached".
    }

    // ── pane_is_stable_and_ready ──────────────────────────────────────────────

    #[test]
    fn pane_is_stable_and_ready_matching_nonempty_samples() {
        assert!(pane_is_stable_and_ready(Some("> "), "> "));
    }

    #[test]
    fn pane_is_stable_and_ready_differing_samples_not_ready() {
        assert!(!pane_is_stable_and_ready(
            Some("splash frame 1"),
            "splash frame 2"
        ));
    }

    #[test]
    fn pane_is_stable_and_ready_no_previous_not_ready() {
        // First observation (or right after a streak reset) has nothing to
        // compare against yet, so it can never count on its own.
        assert!(!pane_is_stable_and_ready(None, "> "));
    }

    #[test]
    fn pane_is_stable_and_ready_empty_current_not_ready() {
        assert!(!pane_is_stable_and_ready(Some(""), ""));
    }

    #[test]
    fn pane_is_stable_and_ready_whitespace_only_current_not_ready() {
        assert!(!pane_is_stable_and_ready(Some("   \n  "), "   \n  "));
    }

    #[test]
    fn pane_is_stable_and_ready_matching_multiline_samples() {
        let pane = "Welcome to Claude Code\n> ";
        assert!(pane_is_stable_and_ready(Some(pane), pane));
    }

    // ── update_readiness_streak ───────────────────────────────────────────────

    #[test]
    fn update_readiness_streak_increments_on_ready_observation() {
        assert_eq!(update_readiness_streak(true, 0), 1);
        assert_eq!(update_readiness_streak(true, 1), 2);
        assert_eq!(update_readiness_streak(true, 2), 3);
    }

    #[test]
    fn update_readiness_streak_resets_on_unready_observation() {
        assert_eq!(update_readiness_streak(false, 0), 0);
        assert_eq!(update_readiness_streak(false, 5), 0);
    }

    #[test]
    fn update_readiness_streak_saturates_instead_of_overflowing() {
        assert_eq!(update_readiness_streak(true, usize::MAX), usize::MAX);
    }

    #[test]
    fn required_stable_observations_reaches_ready_after_matching_streak() {
        // Mirrors the loop in `wait_for_claude`: two consecutive matching,
        // non-empty samples must cross REQUIRED_STABLE_OBSERVATIONS.
        let samples = ["splash 1", "splash 2", "> ", "> ", "> "];
        let mut streak = 0usize;
        let mut last: Option<&str> = None;
        let mut reached_ready_at = None;

        for (i, sample) in samples.iter().enumerate() {
            let is_ready = pane_is_stable_and_ready(last, sample);
            streak = update_readiness_streak(is_ready, streak);
            last = Some(sample);
            if streak >= REQUIRED_STABLE_OBSERVATIONS && reached_ready_at.is_none() {
                reached_ready_at = Some(i);
            }
        }

        assert_eq!(
            reached_ready_at,
            Some(4),
            "readiness should be declared once the streak of matching '> ' samples reaches \
             REQUIRED_STABLE_OBSERVATIONS (index 4 — the third consecutive '> ' sample)"
        );
    }

    // ── wait_for_claude (readiness timeout error branch) ─────────────────────
    //
    // A session name that has never existed can never have a foreground
    // process for `classify_state` to observe as `Running`, so
    // `foreground_cmd_for` deterministically returns "" (whether or not tmux
    // itself is installed — `list_sessions_raw`/`parse_sessions` degrade to
    // "not found" either way). This makes the `AskError::Launch` timeout
    // branch exercisable without a live tmux server.

    #[test]
    fn wait_for_claude_times_out_for_nonexistent_session() {
        let session = "bastion-test-wait-for-claude-nonexistent-session-xyz";
        // Small timeout/interval so the test stays fast: poll_plan(1, 200) = 5 attempts.
        let result = wait_for_claude(session, 1, 200);
        match result {
            Err(AskError::Launch {
                session: s,
                timeout_secs,
            }) => {
                assert_eq!(s, session);
                assert_eq!(timeout_secs, 1);
            }
            other => panic!("expected AskError::Launch, got {other:?}"),
        }
    }

    #[test]
    fn wait_for_claude_is_reachable_as_pub_crate() {
        // Compile-time guarantee (task 1 acceptance criterion): `wait_for_claude`
        // must be `pub(crate)` so `src/serve/` handlers can call it directly.
        // If this compiles, the visibility requirement holds.
        let _: fn(&str, u64, u64) -> Result<(), AskError> = wait_for_claude;
    }

    // ── ensure_session_with_claude (extracted helper) ────────────────────────

    #[test]
    fn ensure_session_with_claude_is_reachable_as_pub_crate() {
        // Compile-time guarantee (task 1 acceptance criterion): the extracted
        // ensure-session-with-claude helper must be `pub(crate)` so `src/serve/`
        // handlers can call it directly without duplicating the mechanics.
        let _: fn(&str, Option<&str>, &str) -> Result<(), AskError> = ensure_session_with_claude;
    }

    #[test]
    fn ensure_session_with_claude_propagates_launch_timeout_error() {
        // A cold, never-created session forces the "create + launch + wait"
        // path. `tmux::new_session`/`send_keys` either succeed (real tmux
        // present) or fail fast and get mapped to `AskError::Tmux` (no tmux
        // installed / no server); if they succeed, the subsequent readiness
        // wait against a launch command that starts nothing real times out
        // and maps to `AskError::Launch`. Either way this exercises the
        // helper's error-branch wiring end-to-end without asserting on a
        // live external process.
        //
        // Uses the timeout-parameterized variant with a small
        // timeout/interval (poll_plan(1, 200) = 5 attempts) so this test
        // stays fast instead of paying the full production
        // `READINESS_TIMEOUT_SECS` (30s) when the readiness wait is
        // exercised — same rationale as
        // `wait_for_claude_times_out_for_nonexistent_session` above.
        let session = "bastion-test-ensure-session-with-claude-xyz";
        let result = ensure_session_with_claude_with_timeout(session, None, "true", 1, 200);

        match result {
            Err(AskError::Launch { session: s, .. }) => assert_eq!(s, session),
            Err(AskError::Tmux { op, .. }) => {
                assert!(!op.is_empty(), "tmux error should carry an op label");
            }
            Ok(()) => {
                // A real tmux server was available and "true" happened to
                // leave a foreground process classify_state treats as
                // Running (unlikely, but not a correctness violation) —
                // clean up so repeated test runs stay hermetic.
            }
            other => panic!("unexpected result variant: {other:?}"),
        }

        // Best-effort cleanup: don't leak a real tmux session across test runs.
        let _ = tmux::kill_session(session);
    }
}
