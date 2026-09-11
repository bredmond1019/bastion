// sessions/commands.rs — session verb handlers (list, attach, new, kill).
//
// Decision D4: this entire path is DB-free. No Config::load(), no Postgres pool.
// Decision D5: all verbs are synchronous blocking calls — no async/tokio coupling.
// tmux is the only data source.

use std::fs;
use std::path::Path;

use anyhow::Context;
use crossterm::event::KeyCode;
use okf_core::{Coord, RegistryClaim};

use crate::sessions::claude_state::{TrustStatus, trust_status};
use crate::sessions::model::{Pane, Session, parse_sessions};
use crate::sessions::tmux::{self, TmuxError};

// ── BA.26.F task 1: watch-vs-attach key routing ──────────────────────────
//
// Pure decision only — no tmux argv construction here. term-core's
// `capture_pane_args`/`attach_args` (`../engine-rs/crates/term-core/src/tmux.rs:53,69`)
// are already unit-tested element-by-element; this block calls the existing
// `tmux::capture_pane_raw`/`tmux::suspend_and_attach` wrappers rather than
// building new argv.

/// The two verbs a node-selection surface can produce for a node with a
/// live tmux session: a non-pausing `Watch` (the default affordance) and a
/// deliberate `Attach` (the real tmux attach, behind a separate key).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeTerminalVerb {
    Watch,
    Attach,
}

/// Map a key to a [`NodeTerminalVerb`]. `'w'` watches (read-only, never
/// pauses the engine); `'t'` attaches (deliberate, pauses the engine's
/// sends for the attach duration + 60s). Every other key — including `'a'`,
/// already bound to the ordinary session-attach action, and every other
/// currently-bound key in `src/sessions/app.rs`'s `on_key` `Mode::Normal`
/// match — returns `None`.
pub(crate) fn node_terminal_verb_for_key(key: KeyCode) -> Option<NodeTerminalVerb> {
    match key {
        KeyCode::Char('w') => Some(NodeTerminalVerb::Watch),
        KeyCode::Char('t') => Some(NodeTerminalVerb::Attach),
        _ => None,
    }
}

/// Entry point for `bastion sessions`.
/// Gathers data from tmux and prints a plain-text table.
pub fn run() -> anyhow::Result<()> {
    let raw = match tmux::list_sessions_raw() {
        Ok(r) => r,
        Err(e) => {
            // Graceful degradation: missing binary or no server → human message, no panic.
            // Every public term-core tmux fn wraps its error in `Context` — unwrap via
            // `root_cause()` first, or a wrapped NoServer/NotInstalled falls through
            // to the fatal branch below instead of degrading gracefully.
            match e.root_cause() {
                TmuxError::NotInstalled => {
                    println!("tmux not installed — install tmux to use `bastion sessions`");
                    return Ok(());
                }
                TmuxError::NoServer => {
                    println!("no tmux server running");
                    return Ok(());
                }
                _ => {}
            }
            return Err(e.into());
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

/// Resolve `<repo>/<lane>` to its held engine session and attach the operator's terminal
/// to it (`BA.25.E` task 2).
///
/// Resolves the real fleet lock directory (`engine_core::coord::resolve_lock_dir`) and
/// delegates the lane→session resolution and the "is this lane actually live" registry
/// check to [`attach_lane_at`], then hands the resolved session name to the existing
/// [`attach`] for the real (blocking, interactive) tmux attach. Adds no new tmux logic of
/// its own — see `attach`'s own doc comment for that half.
pub fn attach_lane(repo: &str, lane: &str) -> anyhow::Result<()> {
    let brain_root =
        engine_core::brain_root::resolve_brain_root().context("cannot resolve brain root")?;
    let lock_dir = engine_core::coord::resolve_lock_dir(&brain_root);
    attach_lane_at(&lock_dir, repo, lane, attach)
}

/// Fixture-driven sibling of [`attach_lane`]: takes the already-resolved lock directory
/// directly (the same `*_at` split `coord_cli`/`sweep_cli`/`drain_cli` already use) and the
/// final attach call as an injected function, so a unit test can assert the resolved
/// session name via the stub's captured argument without ever spawning a real tmux
/// process.
///
/// Verifies a live registry claim exists for `repo`/`lane` under
/// `<lock_dir>/lane-agents/*.json` BEFORE calling `do_attach` — an unresolved lane never
/// reaches tmux (AC-1). On no match, the returned error names both `<repo>/<lane>` and the
/// literal registry path that was checked (AC-1's "naming the registry it consulted").
///
/// Registry claims are read via `okf_core`'s own [`Coord`]/[`RegistryClaim`] types — the
/// exact shape `engine_core::coord`'s reader composes from. That reader's own
/// `read_registry` helper is private to its module, so this reuses the public record type
/// it deserializes into rather than reimplementing the record shape a second time. Unlike
/// `engine_core::coord::read_coordination_view`, this does not classify or surface
/// `Coord::Legacy`/unreadable entries as a `DegradationReason` — it only answers "is this
/// lane claimed", not the full coordination-surface health accounting that `bastion coord
/// status` (`EN.15.A`) already owns.
pub fn attach_lane_at(
    lock_dir: &Path,
    repo: &str,
    lane: &str,
    do_attach: impl FnOnce(&str) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let registry_dir = lock_dir.join("lane-agents");
    let matched = registry_claims(&registry_dir)
        .into_iter()
        .any(|claim| claim.repo == repo && claim.lane == lane);

    if !matched {
        anyhow::bail!(
            "no registry claim for lane '{repo}/{lane}' — checked {}",
            registry_dir.display()
        );
    }

    let session_name = engine_core::workflows::orchestration::graph::held_session_name(repo, lane);
    do_attach(&session_name)
}

/// Every strictly-typed [`RegistryClaim`] found under `registry_dir`
/// (`<lock_dir>/lane-agents`). A missing directory, an unreadable file, malformed JSON, or
/// a `Coord::Legacy` fallback are all silently skipped — see [`attach_lane_at`]'s doc
/// comment for why that is deliberate here.
fn registry_claims(registry_dir: &Path) -> Vec<RegistryClaim> {
    let entries = match fs::read_dir(registry_dir) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| fs::read_to_string(entry.path()).ok())
        .filter_map(|contents| serde_json::from_str::<okf_core::Registry>(&contents).ok())
        .filter_map(|record| match record {
            Coord::Typed(claim) => Some(claim),
            Coord::Legacy(_) => None,
        })
        .collect()
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
        TmuxError::Io(_) | TmuxError::Timeout { .. } | TmuxError::Context { .. } => {
            Degraded::Fatal(format!("error: {verb} '{session_name}' failed: {err}"))
        }
    }
}

/// Apply the degradation outcome for a tmux error: print the message and either
/// swallow (graceful) or propagate (fatal) the original error.
///
/// Every public term-core tmux fn wraps its error in `Context` — unwrap via
/// `root_cause()` before classifying, or a wrapped NoServer/NotInstalled
/// silently loses its graceful-degradation treatment.
fn apply_degradation(verb: &str, session_name: &str, e: TmuxError) -> anyhow::Result<()> {
    match degrade_tmux_error(verb, session_name, e.root_cause()) {
        Degraded::Graceful(msg) => {
            println!("{msg}");
            Ok(())
        }
        Degraded::Fatal(msg) => {
            println!("{msg}");
            Err(e.into())
        }
    }
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
            agent_state: crate::detect::AgentState::Unknown,
            blocked_reason: None,
            cwd: String::new(),
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
            agent_state: crate::detect::AgentState::Unknown,
            blocked_reason: None,
            cwd: String::new(),
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

    // ── node_terminal_verb_for_key (BA.26.F task 1) ─────────────────────

    #[test]
    fn node_terminal_verb_for_key_w_is_watch() {
        assert_eq!(
            node_terminal_verb_for_key(KeyCode::Char('w')),
            Some(NodeTerminalVerb::Watch)
        );
    }

    #[test]
    fn node_terminal_verb_for_key_t_is_attach() {
        assert_eq!(
            node_terminal_verb_for_key(KeyCode::Char('t')),
            Some(NodeTerminalVerb::Attach)
        );
    }

    #[test]
    fn node_terminal_verb_for_key_none_for_every_other_currently_bound_key() {
        // The taken set as of this block (src/sessions/app.rs's on_key
        // Mode::Normal match), plus 'a' explicitly per this task's AC.
        let taken = [
            KeyCode::Char('j'),
            KeyCode::Down,
            KeyCode::Up,
            KeyCode::Char('a'),
            KeyCode::Char('n'),
            KeyCode::Char('s'),
            KeyCode::Char('k'),
            KeyCode::Char('q'),
            KeyCode::Char('v'),
            KeyCode::Char('e'),
            KeyCode::Char('r'),
            KeyCode::Char('p'),
            KeyCode::Char('f'),
            KeyCode::Enter,
            KeyCode::Right,
            KeyCode::Left,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Backspace,
            KeyCode::Esc,
        ];
        for key in taken {
            assert_eq!(
                node_terminal_verb_for_key(key),
                None,
                "key {key:?} should not map to a terminal verb"
            );
        }
    }

    #[test]
    fn node_terminal_verb_for_key_none_for_unrelated_char() {
        assert_eq!(node_terminal_verb_for_key(KeyCode::Char('z')), None);
    }

    // ── attach_lane_at (BA.25.E task 2) ─────────────────────────────────────────

    fn write_claim(dir: &std::path::Path, file_name: &str, repo: &str, lane: &str) {
        std::fs::create_dir_all(dir).expect("create lane-agents dir");
        let body = format!(
            r#"{{"agent_name":"agent-1","repo":"{repo}","lane":"{lane}","roadmap":"r","started_at":"2026-09-01T00:00:00Z","heartbeat":"2026-09-01T00:00:00Z"}}"#
        );
        std::fs::write(dir.join(file_name), body).expect("write claim fixture");
    }

    #[test]
    fn attach_lane_at_no_matching_claim_names_lane_and_registry_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let lock_dir = tmp.path();
        // A claim exists, but for a different repo/lane — must not match.
        write_claim(&lock_dir.join("lane-agents"), "other.json", "bastion", "b1");

        let err = attach_lane_at(lock_dir, "engine-rs", "e1", |_| Ok(()))
            .expect_err("expected no-match error");
        let msg = err.to_string();
        assert!(msg.contains("engine-rs/e1"), "got: {msg}");
        assert!(
            msg.contains(&lock_dir.join("lane-agents").display().to_string()),
            "got: {msg}"
        );
    }

    #[test]
    fn attach_lane_at_missing_registry_dir_names_lane_and_registry_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let lock_dir = tmp.path();
        // No lane-agents/ directory at all.

        let err = attach_lane_at(lock_dir, "bastion", "b1", |_| Ok(()))
            .expect_err("expected no-match error");
        let msg = err.to_string();
        assert!(msg.contains("bastion/b1"), "got: {msg}");
        assert!(
            msg.contains(&lock_dir.join("lane-agents").display().to_string()),
            "got: {msg}"
        );
    }

    #[test]
    fn attach_lane_at_matching_claim_calls_attach_with_exact_session_name() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let lock_dir = tmp.path();
        write_claim(&lock_dir.join("lane-agents"), "mine.json", "bastion", "b1");

        let captured: std::cell::RefCell<Option<String>> = std::cell::RefCell::new(None);
        let result = attach_lane_at(lock_dir, "bastion", "b1", |session_name| {
            *captured.borrow_mut() = Some(session_name.to_string());
            Ok(())
        });

        assert!(result.is_ok(), "expected Ok, got {result:?}");
        assert_eq!(
            captured.into_inner().as_deref(),
            Some("lane-bastion-b1"),
            "session name must be exactly lane-<repo>-<lane>"
        );
    }

    #[test]
    fn attach_lane_at_legacy_claim_does_not_match() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let lock_dir = tmp.path();
        let dir = lock_dir.join("lane-agents");
        std::fs::create_dir_all(&dir).expect("create lane-agents dir");
        // Valid JSON, but not the strict RegistryClaim shape — falls back to Coord::Legacy
        // and must never be treated as a match.
        std::fs::write(dir.join("legacy.json"), r#"{"unexpected":"shape"}"#)
            .expect("write legacy fixture");

        let err = attach_lane_at(lock_dir, "bastion", "b1", |_| Ok(()))
            .expect_err("legacy record must not satisfy the registry check");
        assert!(err.to_string().contains("bastion/b1"));
    }

    /// Architectural guarantee: the sessions code path does not call Config::load()
    /// and does not open a Postgres pool. We verify this by calling the pure
    /// render/parse functions directly with DATABASE_URL intentionally absent from
    /// the environment, and confirming they do not panic or return a config error.
    #[test]
    fn sessions_render_path_requires_no_database_url() {
        // Remove DATABASE_URL from the environment for this test.
        // (In CI it may never be set; either way, these functions must not care.)
        //
        // The old "single-threaded test" safety note was wrong: under
        // `cargo test` every test in this binary is a *thread* of one process,
        // so this bare `remove_var` raced every concurrent reader of env — and
        // it never restored the value, so it silently unset DATABASE_URL for
        // the rest of the run. Both halves are fixed by the crate-wide lock +
        // an RAII guard that restores on drop. See `crate::testsupport`.
        let env_lock = crate::testsupport::lock_env();
        let _database_url = crate::testsupport::EnvVarGuard::unset(&env_lock, "DATABASE_URL");

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
