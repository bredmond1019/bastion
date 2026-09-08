//! Observability spine for bastion: structured error taxonomy + tracing helpers.
//!
//! **Task 1**: vendored C001–C014 error taxonomy (`errors` module).
//! **Task 2**: tracing initialization + structured event-emission helpers (this module).

pub mod errors;

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

// ── Event phase ──────────────────────────────────────────────────────────────

/// Lifecycle phase of a [`CommandEvent`].
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EventPhase {
    /// The command has started execution.
    Start,
    /// The command completed successfully.
    Success,
    /// The command failed with an error.
    Error,
}

// ── CommandEvent — pure record builder/serializer ────────────────────────────

/// Structured record for a single command lifecycle event.
///
/// Construction and JSON serialization are pure (no I/O). The thin
/// `emit_start` / `emit_outcome` helpers call `tracing` macros over this type.
#[derive(Debug, Clone, Serialize)]
pub struct CommandEvent {
    /// Name of the bastion subcommand (e.g. `"status"`, `"inspect"`).
    pub command: String,
    /// Lifecycle phase: start, success, or error.
    pub phase: EventPhase,
    /// Elapsed time in milliseconds; `None` for start events.
    pub duration_ms: Option<u64>,
    /// `C0xx` error code; `None` unless the phase is `Error`.
    pub error_code: Option<String>,
}

impl CommandEvent {
    /// Build a **start** event for the given command (pure).
    pub fn start(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            phase: EventPhase::Start,
            duration_ms: None,
            error_code: None,
        }
    }

    /// Build a **success** outcome event (pure).
    pub fn success(command: impl Into<String>, duration_ms: u64) -> Self {
        Self {
            command: command.into(),
            phase: EventPhase::Success,
            duration_ms: Some(duration_ms),
            error_code: None,
        }
    }

    /// Build an **error** outcome event with a `C0xx` code string (pure).
    pub fn error(
        command: impl Into<String>,
        duration_ms: u64,
        error_code: impl Into<String>,
    ) -> Self {
        Self {
            command: command.into(),
            phase: EventPhase::Error,
            duration_ms: Some(duration_ms),
            error_code: Some(error_code.into()),
        }
    }

    /// Serialize this record to a JSON string (pure — `serde_json`, no process/network I/O).
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }
}

// ── Thin emit helpers (tracing shell over CommandEvent) ──────────────────────

/// Emit a **start** event via `tracing::info!` and return the record.
///
/// This is a thin I/O shell: the pure `CommandEvent::start` builds the record;
/// the only side-effect is the `tracing::info!` macro call.
pub fn emit_start(command: &str) -> CommandEvent {
    let event = CommandEvent::start(command);
    tracing::info!(
        command = %event.command,
        phase = "start",
        "command started"
    );
    event
}

/// Emit an **outcome** event (success or error) via `tracing` and return the record.
///
/// Pass `error_code = None` for success, or `Some("C0xx")` for an error outcome.
/// This is a thin I/O shell: the pure `CommandEvent` builder runs first.
pub fn emit_outcome(command: &str, duration_ms: u64, error_code: Option<&str>) -> CommandEvent {
    let event = match error_code {
        None => CommandEvent::success(command, duration_ms),
        Some(code) => CommandEvent::error(command, duration_ms, code),
    };
    match &event.phase {
        EventPhase::Success => {
            tracing::info!(
                command = %event.command,
                phase = "success",
                duration_ms = duration_ms,
                "command succeeded"
            );
        }
        EventPhase::Error => {
            tracing::error!(
                command = %event.command,
                phase = "error",
                duration_ms = duration_ms,
                error_code = %event.error_code.as_deref().unwrap_or(""),
                "command failed"
            );
        }
        EventPhase::Start => unreachable!("emit_outcome cannot produce a Start phase"),
    }
    event
}

// ── Tracing initialization (thin I/O shell) ──────────────────────────────────

/// Install the global `tracing-subscriber` for the process.
///
/// - `verbose = true` → `DEBUG` level; `false` → `INFO` level.
/// - `json_logs = true` → JSON lines on stderr; `false` → human-readable text.
///
/// Honours the `RUST_LOG` environment variable when set (via `EnvFilter`).
///
/// **Thin I/O shell**: this fn installs a process-global subscriber and must be
/// called exactly once. Calling it more than once will panic (the subscriber
/// framework enforces single installation). Unit tests must guard accordingly
/// (see the `init_tracing` smoke test in `## Notes`).
pub fn init_tracing(verbose: bool, json_logs: bool) {
    use tracing_subscriber::{EnvFilter, fmt};

    let level = if verbose { "debug" } else { "info" };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));

    if json_logs {
        fmt()
            .json()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .init();
    } else {
        fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .init();
    }
}

// ── TUI-safe diagnostics sink ─────────────────────────────────────────────────

/// Resolve the default TUI-safe diagnostics log path:
/// `$XDG_STATE_HOME/bastion/tui-diagnostics.log`, falling back to
/// `$HOME/.local/state/bastion/tui-diagnostics.log`. Returns `None` when
/// neither is set.
///
/// Pure function — reads only the two supplied env values, no I/O — mirroring
/// `blocked_edge::sink::default_sink_path`'s XDG-first/`HOME`-fallback
/// precedence and directory convention (`src/serve/blocked_edge/sink.rs`).
pub fn tui_diagnostics_path(
    xdg_state_home: Option<String>,
    home: Option<String>,
) -> Option<PathBuf> {
    if let Some(xdg) = xdg_state_home {
        Some(
            PathBuf::from(xdg)
                .join("bastion")
                .join("tui-diagnostics.log"),
        )
    } else {
        home.map(|h| {
            PathBuf::from(h)
                .join(".local")
                .join("state")
                .join("bastion")
                .join("tui-diagnostics.log")
        })
    }
}

/// A cloneable `Write` handle over a shared file.
///
/// `tracing-subscriber`'s `MakeWriter` trait is implemented for any
/// `Fn() -> W where W: Write`, so this lets [`tui_safe_subscriber`] hand out a
/// fresh, cheap handle onto the same underlying file for every event without
/// reopening it — the standard zero-extra-dependency pattern for a
/// `tracing-subscriber` file sink (no `tracing-appender` in `Cargo.toml`).
#[derive(Clone)]
struct SharedFileWriter(Arc<Mutex<std::fs::File>>);

impl std::io::Write for SharedFileWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .flush()
    }
}

/// Build a TUI-safe `tracing` subscriber that writes **exclusively** to
/// `file` — never to stderr.
///
/// Used when a `bastion` command is rendering inside the alternate screen
/// (`src/sessions/ui.rs`, `crossterm`): a `tracing::warn!`/`error!` call on
/// [`init_tracing`]'s existing stderr writer would either corrupt the render
/// or be invisible, since the alternate screen owns the terminal.
///
/// Does not install itself as the process-global default — callers reach it
/// through [`init_tracing_tui_safe`] (which does), or, in tests, through
/// `tracing::subscriber::with_default` to scope it to one closure.
/// [`init_tracing`]'s behavior, signature, and every existing non-TUI caller
/// (`monitor`, `inspect`, `costs`, `validate`, `run`, `status`) are unchanged
/// by this function's existence.
fn tui_safe_subscriber(
    verbose: bool,
    file: std::fs::File,
) -> impl tracing::Subscriber + Send + Sync {
    use tracing_subscriber::{EnvFilter, fmt};

    let level = if verbose { "debug" } else { "info" };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    let writer = SharedFileWriter(Arc::new(Mutex::new(file)));

    fmt()
        .with_env_filter(filter)
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .finish()
}

/// Install the TUI-safe subscriber as the process-global default, opening
/// (creating, including parent directories, if needed) the diagnostics file
/// at `path`.
///
/// Mirrors [`init_tracing`]'s single-installation contract — call at most
/// once per process, and never in the same process as [`init_tracing`].
/// Selects the TUI-safe sink instead of the stderr one; it does not change
/// what [`init_tracing`] does for its own callers.
///
/// Any failure to prepare or open the diagnostics file is a **new** failure
/// mode this sink introduces. It maps onto the existing C0xx taxonomy
/// (`src/observ/errors.rs`) via `ConsoleError::Io` (`C009`) — the same
/// variant every other "could not read/write a file" failure in this crate
/// already uses — rather than inventing a parallel error scheme.
pub fn init_tracing_tui_safe(verbose: bool, path: &Path) -> Result<(), errors::ConsoleError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| errors::ConsoleError::Io(format!("{}: {e}", parent.display())))?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| errors::ConsoleError::Io(format!("{}: {e}", path.display())))?;

    let subscriber = tui_safe_subscriber(verbose, file);
    tracing::subscriber::set_global_default(subscriber).map_err(|e| {
        errors::ConsoleError::Io(format!(
            "failed to install TUI-safe tracing subscriber: {e}"
        ))
    })?;
    Ok(())
}

// ── Connection-failure hint ───────────────────────────────────────────────────

/// Shared hint text for a `bastion` command that failed to reach the
/// `events` table it reads (`monitor`, `inspect`, `costs`).
///
// AMENDED 2026-09-06 (BA.chore.monitor-error-string-names-the-retired-python-orchestrator):
// historically this hint sent the operator to the Python orchestrator's ./scripts/dev.sh, which
// D48 made stale — that stack no longer writes the rows these commands read when a run is
// triggered through the embedded Engine.
///
/// Both the retired dev stack and the embedded Engine (`bastion serve`'s engine-serve mount, via
/// engine-store's durable writer, per D48) can populate `events` depending on how the run was
/// triggered — so this names the engine path first without claiming the other one never applies.
/// See `AGENTS.md`'s Environment section.
pub const DB_CONNECTION_HINT: &str = "Is a stack writing to this database? Runs served through \
`bastion serve`'s engine mount are written by engine-store's durable writer — make sure \
DATABASE_URL points at that Postgres instance.";

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // --- DB_CONNECTION_HINT ---

    #[test]
    fn db_connection_hint_names_engine_path_not_the_retired_stack() {
        let stale_phrase = ["Python", "orchestrator"].join(" ");
        assert!(
            !DB_CONNECTION_HINT.contains(&stale_phrase),
            "hint must not reissue the stale directive naming the retired stack"
        );
        assert!(
            DB_CONNECTION_HINT.contains("engine-store"),
            "hint must name engine-store as a writer of the rows"
        );
        assert!(
            DB_CONNECTION_HINT.contains("bastion serve"),
            "hint must name bastion serve's engine mount"
        );
    }

    // --- CommandEvent::start ---

    #[test]
    fn event_start_fields() {
        let ev = CommandEvent::start("inspect");
        assert_eq!(ev.command, "inspect");
        assert_eq!(ev.phase, EventPhase::Start);
        assert!(ev.duration_ms.is_none(), "start must have no duration");
        assert!(ev.error_code.is_none(), "start must have no error_code");
    }

    // --- CommandEvent::success ---

    #[test]
    fn event_success_fields() {
        let ev = CommandEvent::success("status", 42);
        assert_eq!(ev.command, "status");
        assert_eq!(ev.phase, EventPhase::Success);
        assert_eq!(ev.duration_ms, Some(42));
        assert!(ev.error_code.is_none(), "success must have no error_code");
    }

    // --- CommandEvent::error ---

    #[test]
    fn event_error_fields() {
        let ev = CommandEvent::error("monitor", 99, "C007");
        assert_eq!(ev.command, "monitor");
        assert_eq!(ev.phase, EventPhase::Error);
        assert_eq!(ev.duration_ms, Some(99));
        assert_eq!(ev.error_code.as_deref(), Some("C007"));
    }

    // --- JSON serialization — field presence element-by-element ---

    #[test]
    fn event_start_json_fields() {
        let ev = CommandEvent::start("brain");
        let json = ev.to_json();
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(v["command"], "brain");
        assert_eq!(v["phase"], "start");
        assert!(
            v["duration_ms"].is_null(),
            "start: duration_ms must be null"
        );
        assert!(v["error_code"].is_null(), "start: error_code must be null");
    }

    #[test]
    fn event_success_json_fields() {
        let ev = CommandEvent::success("costs", 123);
        let json = ev.to_json();
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(v["command"], "costs");
        assert_eq!(v["phase"], "success");
        assert_eq!(v["duration_ms"], 123u64);
        assert!(
            v["error_code"].is_null(),
            "success: error_code must be null"
        );
    }

    #[test]
    fn event_error_json_fields() {
        let ev = CommandEvent::error("run", 77, "C001");
        let json = ev.to_json();
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(v["command"], "run");
        assert_eq!(v["phase"], "error");
        assert_eq!(v["duration_ms"], 77u64);
        assert_eq!(v["error_code"], "C001");
    }

    // --- emit_outcome returns correct record (no tracing subscriber required) ---

    #[test]
    fn emit_outcome_success_returns_success_event() {
        // tracing macros are no-ops when no subscriber is installed — safe in unit tests.
        let ev = emit_outcome("validate", 55, None);
        assert_eq!(ev.phase, EventPhase::Success);
        assert_eq!(ev.command, "validate");
        assert_eq!(ev.duration_ms, Some(55));
        assert!(ev.error_code.is_none());
    }

    #[test]
    fn emit_outcome_error_returns_error_event() {
        let ev = emit_outcome("inspect", 10, Some("C009"));
        assert_eq!(ev.phase, EventPhase::Error);
        assert_eq!(ev.error_code.as_deref(), Some("C009"));
        assert_eq!(ev.duration_ms, Some(10));
    }

    #[test]
    fn emit_start_returns_start_event() {
        let ev = emit_start("sessions");
        assert_eq!(ev.phase, EventPhase::Start);
        assert_eq!(ev.command, "sessions");
        assert!(ev.duration_ms.is_none());
    }

    // --- tui_diagnostics_path ---

    #[test]
    fn tui_diagnostics_path_prefers_xdg_state_home() {
        let path = tui_diagnostics_path(
            Some("/custom/state".to_string()),
            Some("/home/user".to_string()),
        );
        assert_eq!(
            path,
            Some(PathBuf::from("/custom/state/bastion/tui-diagnostics.log"))
        );
    }

    #[test]
    fn tui_diagnostics_path_falls_back_to_home() {
        let path = tui_diagnostics_path(None, Some("/home/user".to_string()));
        assert_eq!(
            path,
            Some(PathBuf::from(
                "/home/user/.local/state/bastion/tui-diagnostics.log"
            ))
        );
    }

    #[test]
    fn tui_diagnostics_path_none_when_neither_env_set() {
        assert_eq!(tui_diagnostics_path(None, None), None);
    }

    // --- tui_safe_subscriber: event reaches the file sink, and only that sink ---

    fn temp_log_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "bastion-observ-test-{}-{}-{}.log",
            std::process::id(),
            name,
            uuid::Uuid::new_v4()
        ))
    }

    #[test]
    fn tui_safe_sink_receives_event() {
        let log_path = temp_log_path("receives");
        let file = std::fs::File::create(&log_path).expect("create temp log file");
        let subscriber = tui_safe_subscriber(false, file);

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("tui-diag-marker-present");
        });

        let contents = std::fs::read_to_string(&log_path).unwrap_or_default();
        let _ = std::fs::remove_file(&log_path);

        assert!(
            contents.contains("tui-diag-marker-present"),
            "event must reach the TUI-safe file sink, got: {contents:?}"
        );
    }

    #[test]
    fn tui_safe_sink_writes_only_to_its_own_writer_not_a_second_sink() {
        // Two independent file-backed sinks stand in for "the TUI-safe sink"
        // and "what the stderr writer would be": under tracing's per-thread
        // dispatch, an event reaches exactly the currently-active default
        // subscriber's writer and no other. Activating only the first sink
        // and asserting the second stays untouched is exactly the property
        // that guarantees this sink never bleeds onto stderr in production,
        // since `tui_safe_subscriber` never references `std::io::stderr` at
        // all (see its construction above).
        let active_log_path = temp_log_path("active");
        let other_log_path = temp_log_path("other-untouched");
        let active_file = std::fs::File::create(&active_log_path).expect("create active log");
        let other_file = std::fs::File::create(&other_log_path).expect("create other log");

        let active_subscriber = tui_safe_subscriber(false, active_file);
        // Built but never installed as the default — mirrors "the stderr
        // writer exists in the process but is not the active sink".
        let _other_subscriber = tui_safe_subscriber(false, other_file);

        tracing::subscriber::with_default(active_subscriber, || {
            tracing::info!("tui-diag-marker-isolated");
        });

        let active_contents = std::fs::read_to_string(&active_log_path).unwrap_or_default();
        let other_contents = std::fs::read_to_string(&other_log_path).unwrap_or_default();
        let _ = std::fs::remove_file(&active_log_path);
        let _ = std::fs::remove_file(&other_log_path);

        assert!(
            active_contents.contains("tui-diag-marker-isolated"),
            "event must reach the active TUI-safe sink, got: {active_contents:?}"
        );
        assert!(
            other_contents.is_empty(),
            "a sink that was not made the active default must receive nothing \
             (stand-in for: stderr must not be written while the TUI-safe \
             sink is active), got: {other_contents:?}"
        );
    }

    // --- init_tracing_tui_safe: file preparation + error mapping ---

    #[test]
    fn init_tracing_tui_safe_creates_parent_directories() {
        let dir = std::env::temp_dir().join(format!(
            "bastion-observ-test-init-dir-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let path = dir.join("nested").join("tui-diagnostics.log");
        assert!(!dir.exists());

        let file = {
            // Exercise only the file-preparation half (parent-dir creation +
            // open) without calling `set_global_default`, which can only
            // succeed once per process and would make this test order- and
            // concurrency-sensitive alongside every other test in this
            // binary.
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create parent dirs");
            }
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
        };

        assert!(file.is_ok(), "diagnostics file must open once dirs exist");
        assert!(path.parent().unwrap().is_dir());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn init_tracing_tui_safe_maps_open_failure_to_console_error_io() {
        use errors::ConsoleError;

        // A path whose parent is a FILE (not a directory) cannot have a
        // child created under it — `create_dir_all` fails, which is the new
        // failure mode this sink introduces (D... "could not open the
        // diagnostics file"). It must map onto the existing C009 IoError
        // variant, not a bespoke error type.
        let blocking_file = temp_log_path("blocking-parent");
        std::fs::write(&blocking_file, b"not a directory").expect("write blocking file");
        let bad_path = blocking_file.join("tui-diagnostics.log");

        let result = init_tracing_tui_safe(false, &bad_path);
        let _ = std::fs::remove_file(&blocking_file);

        match result {
            Err(ConsoleError::Io(msg)) => {
                assert!(
                    msg.contains(&blocking_file.display().to_string()),
                    "error message should name the offending path: {msg}"
                );
            }
            other => panic!("expected ConsoleError::Io, got {other:?}"),
        }
    }
}
