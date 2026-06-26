//! Observability spine for bastion: structured error taxonomy + tracing helpers.
//!
//! **Task 1**: vendored C001–C014 error taxonomy (`errors` module).
//! **Task 2**: tracing initialization + structured event-emission helpers (this module).

pub mod errors;

use serde::Serialize;

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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
}
