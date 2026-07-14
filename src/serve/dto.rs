//! Serde DTOs for the `bastion serve` v0 surface.
//!
//! All types here are independent serde structs/enums — they do **not** derive
//! directly from the domain types (`Session`, `SessionState`, `Pane`) which only
//! implement `Debug, Clone`.  This keeps the DTO layer free to evolve independently
//! of the domain model.
//!
//! # Types
//! - [`HealthResponse`] — JSON body for `GET /health`.
//! - [`WsFrame`] — tagged envelope for all WebSocket messages (v0 skeleton).
//! - [`WsFrameKind`] — discriminant enum extended by later blocks.
//! - [`CommandRequest`] / [`CommandResponse`] — `POST /actions/command` quick-action
//!   inject/spawn request and response (BA.11.E).

use crate::sessions::model::{Pane, Session};
use serde::{Deserialize, Serialize};

// ── Health ─────────────────────────────────────────────────────────────────────

/// JSON body returned by `GET /health`.
///
/// Matches the shape documented in `docs/serve-api.md` v0:
/// ```json
/// { "status": "ok", "service": "bastion" }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HealthResponse {
    /// Liveness status; always `"ok"` when the server is healthy.
    pub status: String,
    /// Service identifier; always `"bastion"`.
    pub service: String,
}

impl HealthResponse {
    /// Construct the canonical liveness response.
    pub fn ok() -> Self {
        Self {
            status: "ok".to_owned(),
            service: "bastion".to_owned(),
        }
    }
}

// ── WebSocket frame envelope ───────────────────────────────────────────────────

/// Tagged WebSocket frame envelope.
///
/// Every WS message sent or received by `bastion serve` is wrapped in this
/// envelope so the Flutter client can dispatch on `kind` before parsing
/// `payload`.  This is the v0 skeleton; later blocks add concrete `kind`
/// variants and payload types.
///
/// Wire format (JSON):
/// ```json
/// { "kind": "echo", "payload": <any JSON value> }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WsFrame {
    /// Frame type discriminant.  The Flutter client switches on this field.
    pub kind: WsFrameKind,
    /// Arbitrary JSON payload.  Shape is defined per-kind in the serve-api contract.
    pub payload: serde_json::Value,
}

/// Discriminant for [`WsFrame::kind`].
///
/// v0 defined `Echo` and `Error`.  v0.2 adds client→server kinds (`Subscribe`,
/// `Unsubscribe`, `Send`, `SendKey`) and server→client kinds (`Sessions`, `Pane`,
/// `Event`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WsFrameKind {
    /// Echo — the `/ws` actor reflects the received frame back unchanged (v0).
    Echo,
    /// Error — server-side error notification pushed to the client.
    Error,
    // ── client → server (v0.2) ────────────────────────────────────────────
    /// Subscribe to a topic (`sessions` or `pane:<name>`).
    Subscribe,
    /// Unsubscribe from a topic.
    Unsubscribe,
    /// Send literal keystrokes to a tmux session (followed by Enter).
    Send,
    /// Send a single named tmux key to a session (e.g. `"Escape"`, `"C-c"`).
    SendKey,
    // ── server → client (v0.2) ────────────────────────────────────────────
    /// Session list snapshot pushed to `sessions` subscribers.
    Sessions,
    /// Pane diff pushed to `pane:<name>` subscribers.
    Pane,
    /// Async event pushed to all subscribed connections (e.g. `needs_input`).
    Event,
}

// ── Error payload ──────────────────────────────────────────────────────────────

/// Payload shape for `WsFrameKind::Error` frames.
///
/// Allows the server to surface typed error information over the WS channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorPayload {
    /// Short machine-readable error code (e.g. `"C001"`).
    pub code: String,
    /// Human-readable error message.
    pub message: String,
}

// ── v0.2 WebSocket payload structs ────────────────────────────────────────────

/// Payload for client→server `subscribe` / `unsubscribe` frames.
///
/// Wire format: `{ "topic": "sessions" }` or `{ "topic": "pane:work" }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubscribePayload {
    /// Topic string: `"sessions"` or `"pane:<name>"`.
    pub topic: String,
}

/// Payload for client→server `send` frames (literal keystrokes + Enter).
///
/// Wire format: `{ "session": "main", "keys": "cargo test" }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SendPayload {
    /// Target tmux session name.
    pub session: String,
    /// Literal text to send (forwarded with `-l`), followed by Enter.
    pub keys: String,
}

/// Payload for client→server `send_key` frames (single named tmux key).
///
/// Wire format: `{ "session": "main", "key": "Escape" }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SendKeyPayload {
    /// Target tmux session name.
    pub session: String,
    /// Symbolic tmux key name (e.g. `"Escape"`, `"Up"`, `"C-c"`).
    pub key: String,
}

/// Payload for server→client `sessions` frames (session list snapshot).
///
/// Wire format: `{ "sessions": [ … ] }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionsPayload {
    /// Current snapshot of all tmux sessions.
    pub sessions: Vec<SessionDto>,
}

/// Payload for server→client `pane` frames (pane diff push).
///
/// Wire format: `{ "session": "main", "seq": 42, "lines": ["line1", "line2"] }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PanePayload {
    /// tmux session name whose pane was captured.
    pub session: String,
    /// Monotonically increasing sequence number; bumped on every diff push.
    pub seq: u64,
    /// Non-blank trailing lines from the captured pane output.
    pub lines: Vec<String>,
}

/// Payload for server→client `event` frames (e.g. `needs_input`).
///
/// Wire format: `{ "session": "main", "event": "needs_input" }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventPayload {
    /// Session that triggered the event.
    pub session: String,
    /// Event name; currently only `"needs_input"` is defined.
    pub event: String,
}

// ── Topic enum + parser ────────────────────────────────────────────────────────

/// Parsed representation of a WebSocket subscription topic string.
///
/// Valid topic strings:
/// - `"sessions"` → [`Topic::Sessions`]
/// - `"pane:<name>"` → [`Topic::Pane`] where `<name>` is non-empty
///
/// Any other string (including `"pane:"` with an empty name) is invalid and
/// causes [`parse_topic`] to return `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Topic {
    /// The global sessions list topic (`"sessions"`).
    Sessions,
    /// A named pane topic (`"pane:<name>"`).
    Pane(String),
}

/// Parse a topic string into a [`Topic`] variant.
///
/// Returns `None` for any unrecognised or malformed string.
///
/// # Examples
/// ```
/// use crate::serve::dto::{parse_topic, Topic};
/// assert_eq!(parse_topic("sessions"), Some(Topic::Sessions));
/// assert_eq!(parse_topic("pane:work"), Some(Topic::Pane("work".into())));
/// assert_eq!(parse_topic("pane:"),     None);  // empty name
/// assert_eq!(parse_topic("unknown"),   None);
/// ```
pub fn parse_topic(s: &str) -> Option<Topic> {
    if s == "sessions" {
        return Some(Topic::Sessions);
    }
    if let Some(name) = s.strip_prefix("pane:") {
        if name.is_empty() {
            return None;
        }
        return Some(Topic::Pane(name.to_owned()));
    }
    None
}

// ── Session response DTOs ─────────────────────────────────────────────────────

/// JSON response for a single tmux session (one element of `GET /api/sessions`).
///
/// Wire format:
/// ```json
/// { "name": "main", "state": "running", "last_line": "$ cargo test" }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionDto {
    /// tmux session name.
    pub name: String,
    /// Session state as a string: `"running"` or `"idle"`.
    pub state: String,
    /// Last non-blank line from the session's pane, or empty string when unavailable.
    pub last_line: String,
}

impl From<&Session> for SessionDto {
    fn from(s: &Session) -> Self {
        Self {
            name: s.name.clone(),
            state: s.state.as_str().to_owned(),
            last_line: s.last_line.clone(),
        }
    }
}

/// JSON response for `GET /api/sessions/{name}/pane`.
///
/// Wire format:
/// ```json
/// { "session_name": "main", "lines": ["line1", "line2"] }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaneDto {
    /// tmux session name this pane belongs to.
    pub session_name: String,
    /// Lines of captured pane output (trailing blank padding stripped).
    pub lines: Vec<String>,
}

impl PaneDto {
    /// Build a `PaneDto` from a [`Pane`] capture, returning at most `n` trailing lines.
    ///
    /// Pass `None` to include all non-padding lines.
    pub fn from_pane(pane: &Pane, n: Option<usize>) -> Self {
        Self {
            session_name: pane.session_name.clone(),
            lines: pane.last_lines(n),
        }
    }
}

// ── Session request-body DTOs ─────────────────────────────────────────────────

/// Request body for `POST /api/sessions/{name}/send`.
///
/// Sends a literal string of keystrokes to the session followed by `Enter`.
/// Wire format: `{ "keys": "cargo test" }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SendBody {
    /// Literal text to send to the session (forwarded with `-l`).
    pub keys: String,
}

/// Request body for `POST /api/sessions/{name}/key`.
///
/// Sends a single named tmux key (e.g. `"Escape"`, `"Up"`, `"C-c"`) without
/// the `-l` flag so tmux resolves the symbolic key name.
///
/// Wire format: `{ "key": "Escape" }`
///
/// Accepted key names include: `Escape`, `Enter`, `Up`, `Down`, `Left`,
/// `Right`, and modifier combinations such as `C-c`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyBody {
    /// Symbolic tmux key name to send (e.g. `"Escape"`, `"Up"`, `"C-c"`).
    pub key: String,
}

/// Request body for `POST /api/sessions` (create a new tmux session).
///
/// Wire format: `{ "name": "mysession", "dir": "/optional/start/dir" }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewSessionBody {
    /// Name of the new tmux session to create.
    pub name: String,
    /// Optional starting directory for the session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
}

// ── Quick-action command DTOs (BA.11.E) ────────────────────────────────────────

/// Dispatch mode for `POST /actions/command`.
///
/// Serializes/deserializes as the lowercase wire string (`"inject"` / `"spawn"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandMode {
    /// Send the command into an existing tmux session.
    Inject,
    /// Create a new session, launch `claude`, wait for readiness, then send the command.
    Spawn,
}

/// `model` values accepted for `mode:"spawn"` requests (BA.11.E).
pub const ALLOWED_COMMAND_MODELS: &[&str] = &["opus", "sonnet"];

/// Request body for `POST /actions/command`.
///
/// Wire format (inject):
/// ```json
/// { "mode": "inject", "session": "main", "command": "/status" }
/// ```
///
/// Wire format (spawn):
/// ```json
/// { "mode": "spawn", "name": "work", "dir": "/repo", "model": "sonnet", "command": "/status" }
/// ```
///
/// Field requirements are mode-dependent and enforced by [`CommandRequest::validate`],
/// not by serde: `session` is required for `inject`; `name` is required for `spawn`;
/// `model`, when present, must be one of [`ALLOWED_COMMAND_MODELS`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandRequest {
    /// Dispatch mode: `"inject"` or `"spawn"`.
    pub mode: CommandMode,
    /// Target tmux session name. Required when `mode:"inject"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// Name for the new tmux session. Required when `mode:"spawn"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Optional starting directory for a spawned session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
    /// Optional Claude model for a spawned session; one of [`ALLOWED_COMMAND_MODELS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The slash command (or literal text) to send once the target session is ready.
    pub command: String,
}

/// Validation failure for a [`CommandRequest`], returned by [`CommandRequest::validate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandValidationError {
    /// `mode:"inject"` was given without a (non-empty) `session`.
    InjectMissingSession,
    /// `mode:"spawn"` was given without a (non-empty) `name`.
    SpawnMissingName,
    /// `model` was present but not one of [`ALLOWED_COMMAND_MODELS`].
    UnknownModel(String),
}

impl std::fmt::Display for CommandValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InjectMissingSession => {
                write!(f, "mode:\"inject\" requires a non-empty \"session\" field")
            }
            Self::SpawnMissingName => {
                write!(f, "mode:\"spawn\" requires a non-empty \"name\" field")
            }
            Self::UnknownModel(m) => write!(
                f,
                "unknown model {m:?}; expected one of {ALLOWED_COMMAND_MODELS:?}"
            ),
        }
    }
}

impl std::error::Error for CommandValidationError {}

impl CommandRequest {
    /// Validate mode-dependent field requirements.
    ///
    /// Pure — performs no I/O. Checked in order: mode-specific required field first
    /// (empty string counts as missing), then `model` (if present) against the
    /// allow-list, regardless of mode.
    pub fn validate(&self) -> Result<(), CommandValidationError> {
        match self.mode {
            CommandMode::Inject => {
                if self.session.as_deref().unwrap_or("").is_empty() {
                    return Err(CommandValidationError::InjectMissingSession);
                }
            }
            CommandMode::Spawn => {
                if self.name.as_deref().unwrap_or("").is_empty() {
                    return Err(CommandValidationError::SpawnMissingName);
                }
            }
        }
        if let Some(model) = &self.model
            && !ALLOWED_COMMAND_MODELS.contains(&model.as_str())
        {
            return Err(CommandValidationError::UnknownModel(model.clone()));
        }
        Ok(())
    }
}

/// Response body for `POST /actions/command`.
///
/// Wire format: `{ "session": "work" }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandResponse {
    /// The target tmux session id (existing for inject, newly created for spawn).
    pub session: String,
}

// ── Repo / workflow status DTOs (BA.11.D) ──────────────────────────────────────

/// JSON response element for `GET /repos` (one per workspace registry entry).
///
/// Wire format: `{ "name": "bastion", "now": "BA.11.D in progress", "has_handoff": false }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoSummaryDto {
    /// Workspace registry name.
    pub name: String,
    /// Frontmatter `now:` scalar from the repo's `planning/status.md`.
    pub now: String,
    /// Whether `planning/handoff.md` exists for this workspace.
    pub has_handoff: bool,
}

/// JSON response for `GET /repos/{name}/status`.
///
/// Mirrors [`crate::serve::status::repo::RepoStatus`] field-for-field — kept
/// as an independent DTO (per this module's doc comment) rather than reusing
/// the domain type directly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoStatusDto {
    /// Workspace registry name.
    pub name: String,
    /// Frontmatter `now:` scalar.
    pub now: String,
    /// Frontmatter `next:` scalar.
    pub next: String,
    /// Frontmatter `blocked:` scalar.
    pub blocked: String,
    /// Whether `planning/handoff.md` exists.
    pub has_handoff: bool,
    /// Body `## Momentum` → `now` queue line text.
    pub momentum_now: String,
    /// Body `## Momentum` → `next` queue line text.
    pub momentum_next: String,
    /// Body `## Momentum` → `blocked` queue line text.
    pub momentum_blocked: String,
    /// Body `## Momentum` → `improve` queue line text.
    pub momentum_improve: String,
    /// Body `## Momentum` → `recurring` queue line text.
    pub momentum_recurring: String,
}

impl From<crate::serve::status::repo::RepoStatus> for RepoStatusDto {
    fn from(s: crate::serve::status::repo::RepoStatus) -> Self {
        Self {
            name: s.name,
            now: s.now,
            next: s.next,
            blocked: s.blocked,
            has_handoff: s.has_handoff,
            momentum_now: s.momentum_now,
            momentum_next: s.momentum_next,
            momentum_blocked: s.momentum_blocked,
            momentum_improve: s.momentum_improve,
            momentum_recurring: s.momentum_recurring,
        }
    }
}

/// JSON response element for `GET /repos/{name}/workflows`.
///
/// Serializable projection of [`crate::serve::status::flow::FlowState`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowStateDto {
    pub spec_slug: String,
    pub branch: String,
    /// Raw status string, e.g. `"running"`, `"done"`, `"blocked"`.
    pub status: String,
    pub current_task: u32,
    pub started_at: String,
    pub updated_at: String,
}

impl From<crate::serve::status::flow::FlowState> for WorkflowStateDto {
    fn from(f: crate::serve::status::flow::FlowState) -> Self {
        Self {
            spec_slug: f.spec_slug,
            branch: f.branch,
            status: f.status,
            current_task: f.current_task,
            started_at: f.started_at,
            updated_at: f.updated_at,
        }
    }
}

/// Payload for the server→client `event{workflow_done}` WS push.
///
/// Sent inside an [`EventPayload`]-shaped frame: the `event` field is fixed
/// to `"workflow_done"` and the extra repo/spec_slug/status fields are
/// flattened into the same JSON object by the caller (Task 4 WS wiring).
///
/// Wire format: `{ "repo": "bastion", "spec_slug": "phase11-blockD", "status": "done" }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowDonePayload {
    /// Workspace registry name the workflow belongs to.
    pub repo: String,
    /// `sdlc-flow-state.json` spec slug.
    pub spec_slug: String,
    /// The terminal status that triggered the event (`"done"` or `"blocked"`).
    pub status: String,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── HealthResponse ─────────────────────────────────────────────────────

    #[test]
    fn health_response_ok_constructor() {
        let h = HealthResponse::ok();
        assert_eq!(h.status, "ok");
        assert_eq!(h.service, "bastion");
    }

    #[test]
    fn health_response_serializes_to_expected_json() {
        let h = HealthResponse::ok();
        let v = serde_json::to_value(&h).expect("serialize HealthResponse");
        assert_eq!(v["status"], "ok");
        assert_eq!(v["service"], "bastion");
    }

    #[test]
    fn health_response_round_trip() {
        let original = HealthResponse::ok();
        let json = serde_json::to_string(&original).expect("serialize");
        let decoded: HealthResponse = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, decoded, "round-trip must preserve all fields");
    }

    #[test]
    fn health_response_deserializes_from_json() {
        let raw = r#"{"status":"ok","service":"bastion"}"#;
        let h: HealthResponse = serde_json::from_str(raw).expect("deserialize HealthResponse");
        assert_eq!(h.status, "ok");
        assert_eq!(h.service, "bastion");
    }

    #[test]
    fn health_response_rejects_missing_status_field() {
        let raw = r#"{"service":"bastion"}"#;
        let result: Result<HealthResponse, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "deserialize must fail when 'status' field is missing"
        );
    }

    // ── WsFrameKind ────────────────────────────────────────────────────────

    #[test]
    fn ws_frame_kind_echo_serializes_snake_case() {
        let kind = WsFrameKind::Echo;
        let v = serde_json::to_value(&kind).expect("serialize WsFrameKind::Echo");
        assert_eq!(v, json!("echo"), "Echo must serialize to snake_case 'echo'");
    }

    #[test]
    fn ws_frame_kind_error_serializes_snake_case() {
        let kind = WsFrameKind::Error;
        let v = serde_json::to_value(&kind).expect("serialize WsFrameKind::Error");
        assert_eq!(
            v,
            json!("error"),
            "Error must serialize to snake_case 'error'"
        );
    }

    #[test]
    fn ws_frame_kind_echo_deserializes() {
        let kind: WsFrameKind =
            serde_json::from_str(r#""echo""#).expect("deserialize 'echo' variant");
        assert_eq!(kind, WsFrameKind::Echo);
    }

    #[test]
    fn ws_frame_kind_error_deserializes() {
        let kind: WsFrameKind =
            serde_json::from_str(r#""error""#).expect("deserialize 'error' variant");
        assert_eq!(kind, WsFrameKind::Error);
    }

    #[test]
    fn ws_frame_kind_unknown_variant_fails() {
        let result: Result<WsFrameKind, _> = serde_json::from_str(r#""unknown_kind""#);
        assert!(
            result.is_err(),
            "unknown kind variant must fail to deserialize"
        );
    }

    // ── WsFrame envelope ───────────────────────────────────────────────────

    #[test]
    fn ws_frame_echo_round_trip() {
        let frame = WsFrame {
            kind: WsFrameKind::Echo,
            payload: json!({"text": "hello"}),
        };
        let json = serde_json::to_string(&frame).expect("serialize WsFrame");
        let decoded: WsFrame = serde_json::from_str(&json).expect("deserialize WsFrame");
        assert_eq!(
            frame, decoded,
            "WsFrame round-trip must preserve kind + payload"
        );
    }

    #[test]
    fn ws_frame_serializes_kind_as_snake_case_tag() {
        let frame = WsFrame {
            kind: WsFrameKind::Echo,
            payload: json!(null),
        };
        let v = serde_json::to_value(&frame).expect("serialize WsFrame");
        assert_eq!(
            v["kind"], "echo",
            "WsFrame.kind must be the snake_case discriminant string; got {v}"
        );
    }

    #[test]
    fn ws_frame_payload_preserved_unchanged() {
        let payload = json!({"session_id": "abc123", "count": 42, "active": true});
        let frame = WsFrame {
            kind: WsFrameKind::Echo,
            payload: payload.clone(),
        };
        let v = serde_json::to_value(&frame).expect("serialize WsFrame");
        assert_eq!(
            v["payload"], payload,
            "WsFrame.payload must be preserved exactly"
        );
    }

    #[test]
    fn ws_frame_deserializes_from_json_object() {
        let raw = r#"{"kind":"echo","payload":{"text":"hello world"}}"#;
        let frame: WsFrame = serde_json::from_str(raw).expect("deserialize WsFrame from JSON");
        assert_eq!(frame.kind, WsFrameKind::Echo);
        assert_eq!(frame.payload["text"], "hello world");
    }

    #[test]
    fn ws_frame_error_kind_round_trip() {
        let frame = WsFrame {
            kind: WsFrameKind::Error,
            payload: json!({"code": "C001", "message": "internal error"}),
        };
        let json = serde_json::to_string(&frame).expect("serialize error frame");
        let decoded: WsFrame = serde_json::from_str(&json).expect("deserialize error frame");
        assert_eq!(frame, decoded);
    }

    #[test]
    fn ws_frame_accepts_null_payload() {
        let frame = WsFrame {
            kind: WsFrameKind::Echo,
            payload: json!(null),
        };
        let json = serde_json::to_string(&frame).expect("serialize null payload frame");
        let decoded: WsFrame = serde_json::from_str(&json).expect("deserialize null payload frame");
        assert_eq!(frame, decoded);
    }

    // ── ErrorPayload ───────────────────────────────────────────────────────

    #[test]
    fn error_payload_round_trip() {
        let ep = ErrorPayload {
            code: "C001".to_owned(),
            message: "connection refused".to_owned(),
        };
        let json = serde_json::to_string(&ep).expect("serialize ErrorPayload");
        let decoded: ErrorPayload = serde_json::from_str(&json).expect("deserialize ErrorPayload");
        assert_eq!(ep, decoded);
    }

    #[test]
    fn error_payload_serializes_expected_fields() {
        let ep = ErrorPayload {
            code: "C014".to_owned(),
            message: "unknown error".to_owned(),
        };
        let v = serde_json::to_value(&ep).expect("serialize ErrorPayload to value");
        assert_eq!(v["code"], "C014");
        assert_eq!(v["message"], "unknown error");
    }

    #[test]
    fn error_payload_rejects_missing_code() {
        let raw = r#"{"message":"oops"}"#;
        let result: Result<ErrorPayload, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "ErrorPayload must fail to deserialize when 'code' is missing"
        );
    }

    #[test]
    fn error_payload_rejects_missing_message() {
        let raw = r#"{"code":"C001"}"#;
        let result: Result<ErrorPayload, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "ErrorPayload must fail to deserialize when 'message' is missing"
        );
    }

    // ── SessionDto ────────────────────────────────────────────────────────

    fn make_session(
        name: &str,
        state: crate::sessions::model::SessionState,
        last_line: &str,
    ) -> crate::sessions::model::Session {
        crate::sessions::model::Session {
            name: name.to_owned(),
            state,
            window_count: 1,
            foreground_cmd: "zsh".to_owned(),
            last_line: last_line.to_owned(),
            agent_state: crate::detect::AgentState::Unknown,
        }
    }

    #[test]
    fn session_dto_from_running_session() {
        use crate::sessions::model::SessionState;
        let s = make_session("main", SessionState::Running, "$ cargo test");
        let dto = SessionDto::from(&s);
        assert_eq!(dto.name, "main");
        assert_eq!(dto.state, "running");
        assert_eq!(dto.last_line, "$ cargo test");
    }

    #[test]
    fn session_dto_from_idle_session() {
        use crate::sessions::model::SessionState;
        let s = make_session("scratch", SessionState::Idle, "");
        let dto = SessionDto::from(&s);
        assert_eq!(dto.name, "scratch");
        assert_eq!(dto.state, "idle");
        assert_eq!(dto.last_line, "");
    }

    #[test]
    fn session_dto_serializes_expected_fields() {
        use crate::sessions::model::SessionState;
        let s = make_session("work", SessionState::Running, "hello");
        let dto = SessionDto::from(&s);
        let v = serde_json::to_value(&dto).expect("serialize SessionDto");
        assert_eq!(v["name"], "work");
        assert_eq!(v["state"], "running");
        assert_eq!(v["last_line"], "hello");
    }

    #[test]
    fn session_dto_round_trip() {
        use crate::sessions::model::SessionState;
        let s = make_session("loop", SessionState::Idle, "done");
        let dto = SessionDto::from(&s);
        let json = serde_json::to_string(&dto).expect("serialize");
        let decoded: SessionDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(dto, decoded);
    }

    #[test]
    fn session_dto_rejects_missing_name() {
        let raw = r#"{"state":"idle","last_line":""}"#;
        let result: Result<SessionDto, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "SessionDto must fail when 'name' is missing"
        );
    }

    #[test]
    fn session_dto_rejects_missing_state() {
        let raw = r#"{"name":"s","last_line":""}"#;
        let result: Result<SessionDto, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "SessionDto must fail when 'state' is missing"
        );
    }

    #[test]
    fn session_dto_rejects_missing_last_line() {
        let raw = r#"{"name":"s","state":"idle"}"#;
        let result: Result<SessionDto, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "SessionDto must fail when 'last_line' is missing"
        );
    }

    // ── PaneDto ───────────────────────────────────────────────────────────

    #[test]
    fn pane_dto_from_pane_with_n_lines() {
        let pane = Pane::new("work", "line1\nline2\nline3\n\n");
        let dto = PaneDto::from_pane(&pane, Some(2));
        assert_eq!(dto.session_name, "work");
        assert_eq!(dto.lines, vec!["line2", "line3"]);
    }

    #[test]
    fn pane_dto_from_pane_none_returns_all() {
        let pane = Pane::new("work", "a\nb\nc\n\n");
        let dto = PaneDto::from_pane(&pane, None);
        assert_eq!(dto.lines, vec!["a", "b", "c"]);
    }

    #[test]
    fn pane_dto_serializes_expected_fields() {
        let pane = Pane::new("main", "out1\nout2\n");
        let dto = PaneDto::from_pane(&pane, None);
        let v = serde_json::to_value(&dto).expect("serialize PaneDto");
        assert_eq!(v["session_name"], "main");
        assert_eq!(v["lines"][0], "out1");
        assert_eq!(v["lines"][1], "out2");
    }

    #[test]
    fn pane_dto_round_trip() {
        let pane = Pane::new("s", "x\ny\n");
        let dto = PaneDto::from_pane(&pane, None);
        let json = serde_json::to_string(&dto).expect("serialize");
        let decoded: PaneDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(dto, decoded);
    }

    #[test]
    fn pane_dto_rejects_missing_session_name() {
        let raw = r#"{"lines":["x"]}"#;
        let result: Result<PaneDto, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "PaneDto must fail when 'session_name' is missing"
        );
    }

    #[test]
    fn pane_dto_rejects_missing_lines() {
        let raw = r#"{"session_name":"s"}"#;
        let result: Result<PaneDto, _> = serde_json::from_str(raw);
        assert!(result.is_err(), "PaneDto must fail when 'lines' is missing");
    }

    // ── SendBody ──────────────────────────────────────────────────────────

    #[test]
    fn send_body_serializes_keys_field() {
        let b = SendBody {
            keys: "cargo test".to_owned(),
        };
        let v = serde_json::to_value(&b).expect("serialize SendBody");
        assert_eq!(v["keys"], "cargo test");
    }

    #[test]
    fn send_body_round_trip() {
        let b = SendBody {
            keys: "hello world".to_owned(),
        };
        let json = serde_json::to_string(&b).expect("serialize");
        let decoded: SendBody = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(b, decoded);
    }

    #[test]
    fn send_body_rejects_missing_keys() {
        let raw = r#"{}"#;
        let result: Result<SendBody, _> = serde_json::from_str(raw);
        assert!(result.is_err(), "SendBody must fail when 'keys' is missing");
    }

    // ── KeyBody ───────────────────────────────────────────────────────────

    #[test]
    fn key_body_serializes_key_field() {
        let b = KeyBody {
            key: "Escape".to_owned(),
        };
        let v = serde_json::to_value(&b).expect("serialize KeyBody");
        assert_eq!(v["key"], "Escape");
    }

    #[test]
    fn key_body_round_trip() {
        let b = KeyBody {
            key: "C-c".to_owned(),
        };
        let json = serde_json::to_string(&b).expect("serialize");
        let decoded: KeyBody = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(b, decoded);
    }

    #[test]
    fn key_body_rejects_missing_key() {
        let raw = r#"{}"#;
        let result: Result<KeyBody, _> = serde_json::from_str(raw);
        assert!(result.is_err(), "KeyBody must fail when 'key' is missing");
    }

    // ── NewSessionBody ────────────────────────────────────────────────────

    #[test]
    fn new_session_body_with_dir_serializes() {
        let b = NewSessionBody {
            name: "work".to_owned(),
            dir: Some("/home/user".to_owned()),
        };
        let v = serde_json::to_value(&b).expect("serialize NewSessionBody");
        assert_eq!(v["name"], "work");
        assert_eq!(v["dir"], "/home/user");
    }

    #[test]
    fn new_session_body_without_dir_omits_field() {
        let b = NewSessionBody {
            name: "scratch".to_owned(),
            dir: None,
        };
        let v = serde_json::to_value(&b).expect("serialize NewSessionBody");
        assert_eq!(v["name"], "scratch");
        // dir field must be absent when None (skip_serializing_if = "Option::is_none")
        assert!(v.get("dir").is_none(), "dir must be omitted when None");
    }

    #[test]
    fn new_session_body_round_trip_with_dir() {
        let b = NewSessionBody {
            name: "work".to_owned(),
            dir: Some("/tmp".to_owned()),
        };
        let json = serde_json::to_string(&b).expect("serialize");
        let decoded: NewSessionBody = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(b, decoded);
    }

    #[test]
    fn new_session_body_round_trip_without_dir() {
        let b = NewSessionBody {
            name: "empty".to_owned(),
            dir: None,
        };
        let json = serde_json::to_string(&b).expect("serialize");
        let decoded: NewSessionBody = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(b, decoded);
    }

    #[test]
    fn new_session_body_rejects_missing_name() {
        let raw = r#"{"dir":"/tmp"}"#;
        let result: Result<NewSessionBody, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "NewSessionBody must fail when 'name' is missing"
        );
    }

    #[test]
    fn new_session_body_accepts_missing_dir_as_none() {
        // dir is optional — missing in JSON means None
        let raw = r#"{"name":"test"}"#;
        let b: NewSessionBody = serde_json::from_str(raw).expect("deserialize");
        assert_eq!(b.name, "test");
        assert!(b.dir.is_none());
    }

    // ── v0.2 WsFrameKind variants ──────────────────────────────────────────

    #[test]
    fn ws_frame_kind_subscribe_serializes_snake_case() {
        let kind = WsFrameKind::Subscribe;
        let v = serde_json::to_value(&kind).expect("serialize WsFrameKind::Subscribe");
        assert_eq!(v, json!("subscribe"));
    }

    #[test]
    fn ws_frame_kind_unsubscribe_serializes_snake_case() {
        let kind = WsFrameKind::Unsubscribe;
        let v = serde_json::to_value(&kind).expect("serialize WsFrameKind::Unsubscribe");
        assert_eq!(v, json!("unsubscribe"));
    }

    #[test]
    fn ws_frame_kind_send_serializes_snake_case() {
        let kind = WsFrameKind::Send;
        let v = serde_json::to_value(&kind).expect("serialize WsFrameKind::Send");
        assert_eq!(v, json!("send"));
    }

    #[test]
    fn ws_frame_kind_send_key_serializes_snake_case() {
        let kind = WsFrameKind::SendKey;
        let v = serde_json::to_value(&kind).expect("serialize WsFrameKind::SendKey");
        assert_eq!(v, json!("send_key"));
    }

    #[test]
    fn ws_frame_kind_sessions_serializes_snake_case() {
        let kind = WsFrameKind::Sessions;
        let v = serde_json::to_value(&kind).expect("serialize WsFrameKind::Sessions");
        assert_eq!(v, json!("sessions"));
    }

    #[test]
    fn ws_frame_kind_pane_serializes_snake_case() {
        let kind = WsFrameKind::Pane;
        let v = serde_json::to_value(&kind).expect("serialize WsFrameKind::Pane");
        assert_eq!(v, json!("pane"));
    }

    #[test]
    fn ws_frame_kind_event_serializes_snake_case() {
        let kind = WsFrameKind::Event;
        let v = serde_json::to_value(&kind).expect("serialize WsFrameKind::Event");
        assert_eq!(v, json!("event"));
    }

    #[test]
    fn ws_frame_kind_v02_round_trips() {
        for kind in [
            WsFrameKind::Subscribe,
            WsFrameKind::Unsubscribe,
            WsFrameKind::Send,
            WsFrameKind::SendKey,
            WsFrameKind::Sessions,
            WsFrameKind::Pane,
            WsFrameKind::Event,
        ] {
            let json = serde_json::to_string(&kind).expect("serialize");
            let decoded: WsFrameKind = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(kind, decoded, "round-trip failed for {json}");
        }
    }

    // ── SubscribePayload ──────────────────────────────────────────────────

    #[test]
    fn subscribe_payload_round_trip() {
        let p = SubscribePayload {
            topic: "sessions".to_owned(),
        };
        let json = serde_json::to_string(&p).expect("serialize");
        let decoded: SubscribePayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(p, decoded);
    }

    #[test]
    fn subscribe_payload_serializes_topic_field() {
        let p = SubscribePayload {
            topic: "pane:work".to_owned(),
        };
        let v = serde_json::to_value(&p).expect("serialize");
        assert_eq!(v["topic"], "pane:work");
    }

    #[test]
    fn subscribe_payload_rejects_missing_topic() {
        let raw = r#"{}"#;
        let result: Result<SubscribePayload, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "SubscribePayload must fail when topic is missing"
        );
    }

    // ── SendPayload ───────────────────────────────────────────────────────

    #[test]
    fn send_payload_round_trip() {
        let p = SendPayload {
            session: "main".to_owned(),
            keys: "cargo test".to_owned(),
        };
        let json = serde_json::to_string(&p).expect("serialize");
        let decoded: SendPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(p, decoded);
    }

    #[test]
    fn send_payload_serializes_expected_fields() {
        let p = SendPayload {
            session: "work".to_owned(),
            keys: "ls -la".to_owned(),
        };
        let v = serde_json::to_value(&p).expect("serialize");
        assert_eq!(v["session"], "work");
        assert_eq!(v["keys"], "ls -la");
    }

    #[test]
    fn send_payload_rejects_missing_session() {
        let raw = r#"{"keys":"hello"}"#;
        let result: Result<SendPayload, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "SendPayload must fail when session is missing"
        );
    }

    #[test]
    fn send_payload_rejects_missing_keys() {
        let raw = r#"{"session":"main"}"#;
        let result: Result<SendPayload, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "SendPayload must fail when keys is missing"
        );
    }

    // ── SendKeyPayload ────────────────────────────────────────────────────

    #[test]
    fn send_key_payload_round_trip() {
        let p = SendKeyPayload {
            session: "main".to_owned(),
            key: "Escape".to_owned(),
        };
        let json = serde_json::to_string(&p).expect("serialize");
        let decoded: SendKeyPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(p, decoded);
    }

    #[test]
    fn send_key_payload_serializes_expected_fields() {
        let p = SendKeyPayload {
            session: "work".to_owned(),
            key: "C-c".to_owned(),
        };
        let v = serde_json::to_value(&p).expect("serialize");
        assert_eq!(v["session"], "work");
        assert_eq!(v["key"], "C-c");
    }

    #[test]
    fn send_key_payload_rejects_missing_session() {
        let raw = r#"{"key":"Escape"}"#;
        let result: Result<SendKeyPayload, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "SendKeyPayload must fail when session is missing"
        );
    }

    #[test]
    fn send_key_payload_rejects_missing_key() {
        let raw = r#"{"session":"main"}"#;
        let result: Result<SendKeyPayload, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "SendKeyPayload must fail when key is missing"
        );
    }

    // ── SessionsPayload ───────────────────────────────────────────────────

    #[test]
    fn sessions_payload_round_trip() {
        let p = SessionsPayload {
            sessions: vec![SessionDto {
                name: "main".to_owned(),
                state: "running".to_owned(),
                last_line: "$ cargo test".to_owned(),
            }],
        };
        let json = serde_json::to_string(&p).expect("serialize");
        let decoded: SessionsPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(p, decoded);
    }

    #[test]
    fn sessions_payload_serializes_sessions_field() {
        let p = SessionsPayload { sessions: vec![] };
        let v = serde_json::to_value(&p).expect("serialize");
        assert!(v["sessions"].is_array(), "sessions must be an array");
        assert_eq!(v["sessions"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn sessions_payload_rejects_missing_sessions() {
        let raw = r#"{}"#;
        let result: Result<SessionsPayload, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "SessionsPayload must fail when sessions is missing"
        );
    }

    // ── PanePayload ───────────────────────────────────────────────────────

    #[test]
    fn pane_payload_round_trip() {
        let p = PanePayload {
            session: "main".to_owned(),
            seq: 7,
            lines: vec!["line1".to_owned(), "line2".to_owned()],
        };
        let json = serde_json::to_string(&p).expect("serialize");
        let decoded: PanePayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(p, decoded);
    }

    #[test]
    fn pane_payload_serializes_expected_fields() {
        let p = PanePayload {
            session: "work".to_owned(),
            seq: 42,
            lines: vec!["hello".to_owned()],
        };
        let v = serde_json::to_value(&p).expect("serialize");
        assert_eq!(v["session"], "work");
        assert_eq!(v["seq"], 42);
        assert_eq!(v["lines"][0], "hello");
    }

    #[test]
    fn pane_payload_rejects_missing_session() {
        let raw = r#"{"seq":1,"lines":[]}"#;
        let result: Result<PanePayload, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "PanePayload must fail when session is missing"
        );
    }

    #[test]
    fn pane_payload_rejects_missing_seq() {
        let raw = r#"{"session":"main","lines":[]}"#;
        let result: Result<PanePayload, _> = serde_json::from_str(raw);
        assert!(result.is_err(), "PanePayload must fail when seq is missing");
    }

    #[test]
    fn pane_payload_rejects_missing_lines() {
        let raw = r#"{"session":"main","seq":1}"#;
        let result: Result<PanePayload, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "PanePayload must fail when lines is missing"
        );
    }

    // ── EventPayload ──────────────────────────────────────────────────────

    #[test]
    fn event_payload_round_trip() {
        let p = EventPayload {
            session: "main".to_owned(),
            event: "needs_input".to_owned(),
        };
        let json = serde_json::to_string(&p).expect("serialize");
        let decoded: EventPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(p, decoded);
    }

    #[test]
    fn event_payload_serializes_expected_fields() {
        let p = EventPayload {
            session: "work".to_owned(),
            event: "needs_input".to_owned(),
        };
        let v = serde_json::to_value(&p).expect("serialize");
        assert_eq!(v["session"], "work");
        assert_eq!(v["event"], "needs_input");
    }

    #[test]
    fn event_payload_rejects_missing_session() {
        let raw = r#"{"event":"needs_input"}"#;
        let result: Result<EventPayload, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "EventPayload must fail when session is missing"
        );
    }

    #[test]
    fn event_payload_rejects_missing_event() {
        let raw = r#"{"session":"main"}"#;
        let result: Result<EventPayload, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "EventPayload must fail when event is missing"
        );
    }

    // ── parse_topic ───────────────────────────────────────────────────────

    #[test]
    fn parse_topic_sessions() {
        assert_eq!(parse_topic("sessions"), Some(Topic::Sessions));
    }

    #[test]
    fn parse_topic_pane_with_name() {
        assert_eq!(
            parse_topic("pane:work"),
            Some(Topic::Pane("work".to_owned()))
        );
    }

    #[test]
    fn parse_topic_pane_empty_name_is_none() {
        assert_eq!(
            parse_topic("pane:"),
            None,
            "empty pane name must be rejected"
        );
    }

    #[test]
    fn parse_topic_unknown_is_none() {
        assert_eq!(parse_topic("unknown"), None);
        assert_eq!(parse_topic(""), None);
        assert_eq!(parse_topic("SESSIONS"), None);
        assert_eq!(parse_topic("Pane:work"), None);
    }

    #[test]
    fn parse_topic_pane_name_with_hyphens_and_underscores() {
        // names like "claude-work" or "my_session" are valid
        assert_eq!(
            parse_topic("pane:claude-work"),
            Some(Topic::Pane("claude-work".to_owned()))
        );
        assert_eq!(
            parse_topic("pane:my_session"),
            Some(Topic::Pane("my_session".to_owned()))
        );
    }

    // ── RepoSummaryDto ────────────────────────────────────────────────────

    #[test]
    fn repo_summary_dto_round_trips() {
        let dto = RepoSummaryDto {
            name: "bastion".to_owned(),
            now: "BA.11.D in progress".to_owned(),
            has_handoff: true,
        };
        let json = serde_json::to_string(&dto).expect("serialize");
        let back: RepoSummaryDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(dto, back);
    }

    #[test]
    fn repo_summary_dto_rejects_missing_fields() {
        let raw = r#"{"name":"bastion","now":"x"}"#;
        let result: Result<RepoSummaryDto, _> = serde_json::from_str(raw);
        assert!(result.is_err(), "missing has_handoff must fail to parse");
    }

    // ── RepoStatusDto ─────────────────────────────────────────────────────

    fn sample_repo_status() -> crate::serve::status::repo::RepoStatus {
        crate::serve::status::repo::parse_status(
            "---\nnow: \"focus\"\nnext: \"next thing\"\nblocked: \"[]\"\n---\n\n## Momentum\n- **now** — focus\n- **next** — next thing\n- **blocked** — nothing\n- **improve** — tighten\n- **recurring** — none\n",
        )
        .expect("fixture status content must parse")
    }

    #[test]
    fn repo_status_dto_from_repo_status() {
        let status = sample_repo_status();
        let dto: RepoStatusDto = status.clone().into();
        assert_eq!(dto.now, status.now);
        assert_eq!(dto.next, status.next);
        assert_eq!(dto.blocked, status.blocked);
        assert_eq!(dto.momentum_now, status.momentum_now);
    }

    #[test]
    fn repo_status_dto_round_trips() {
        let status = sample_repo_status();
        let dto: RepoStatusDto = status.into();
        let json = serde_json::to_string(&dto).expect("serialize");
        let back: RepoStatusDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(dto, back);
    }

    // ── WorkflowStateDto ──────────────────────────────────────────────────

    fn sample_flow_state() -> crate::serve::status::flow::FlowState {
        crate::serve::status::flow::parse_flow_state(
            r#"{"spec_slug":"phase11-blockD","branch":"phase11-blockD-flow","status":"running","current_task":3,"started_at":"2026-06-30T00:00:00Z","updated_at":"2026-06-30T01:00:00Z"}"#,
        )
        .expect("fixture flow state must parse")
    }

    #[test]
    fn workflow_state_dto_from_flow_state() {
        let flow = sample_flow_state();
        let dto: WorkflowStateDto = flow.clone().into();
        assert_eq!(dto.spec_slug, flow.spec_slug);
        assert_eq!(dto.branch, flow.branch);
        assert_eq!(dto.status, flow.status);
        assert_eq!(dto.current_task, flow.current_task);
    }

    #[test]
    fn workflow_state_dto_round_trips() {
        let flow = sample_flow_state();
        let dto: WorkflowStateDto = flow.into();
        let json = serde_json::to_string(&dto).expect("serialize");
        let back: WorkflowStateDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(dto, back);
    }

    #[test]
    fn workflow_state_dto_rejects_missing_fields() {
        let raw = r#"{"spec_slug":"x","branch":"y","status":"running"}"#;
        let result: Result<WorkflowStateDto, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "missing current_task/started_at/updated_at must fail to parse"
        );
    }

    // ── WorkflowDonePayload ───────────────────────────────────────────────

    #[test]
    fn workflow_done_payload_round_trips() {
        let payload = WorkflowDonePayload {
            repo: "bastion".to_owned(),
            spec_slug: "phase11-blockD".to_owned(),
            status: "done".to_owned(),
        };
        let json = serde_json::to_string(&payload).expect("serialize");
        let back: WorkflowDonePayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(payload, back);
    }

    #[test]
    fn workflow_done_payload_serializes_expected_shape() {
        let payload = WorkflowDonePayload {
            repo: "bastion".to_owned(),
            spec_slug: "phase11-blockD".to_owned(),
            status: "blocked".to_owned(),
        };
        let v = serde_json::to_value(&payload).expect("serialize");
        assert_eq!(v["repo"], "bastion");
        assert_eq!(v["spec_slug"], "phase11-blockD");
        assert_eq!(v["status"], "blocked");
    }

    #[test]
    fn workflow_done_payload_rejects_missing_fields() {
        let raw = r#"{"repo":"bastion","spec_slug":"phase11-blockD"}"#;
        let result: Result<WorkflowDonePayload, _> = serde_json::from_str(raw);
        assert!(result.is_err(), "missing status must fail to parse");
    }

    // ── CommandMode ───────────────────────────────────────────────────────

    #[test]
    fn command_mode_inject_serializes_snake_case() {
        let v = serde_json::to_value(CommandMode::Inject).expect("serialize");
        assert_eq!(v, json!("inject"));
    }

    #[test]
    fn command_mode_spawn_serializes_snake_case() {
        let v = serde_json::to_value(CommandMode::Spawn).expect("serialize");
        assert_eq!(v, json!("spawn"));
    }

    #[test]
    fn command_mode_round_trips() {
        for mode in [CommandMode::Inject, CommandMode::Spawn] {
            let json = serde_json::to_string(&mode).expect("serialize");
            let decoded: CommandMode = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(mode, decoded, "round-trip failed for {json}");
        }
    }

    #[test]
    fn command_mode_unknown_variant_fails() {
        let result: Result<CommandMode, _> = serde_json::from_str(r#""restart""#);
        assert!(
            result.is_err(),
            "unrecognised mode string must fail to deserialize"
        );
    }

    // ── CommandRequest deserialization ───────────────────────────────────

    #[test]
    fn command_request_deserializes_valid_inject_payload() {
        let raw = r#"{"mode":"inject","session":"main","command":"/status"}"#;
        let req: CommandRequest = serde_json::from_str(raw).expect("deserialize inject payload");
        assert_eq!(req.mode, CommandMode::Inject);
        assert_eq!(req.session.as_deref(), Some("main"));
        assert_eq!(req.command, "/status");
        assert!(req.name.is_none());
        assert!(req.dir.is_none());
        assert!(req.model.is_none());
    }

    #[test]
    fn command_request_deserializes_valid_spawn_payload() {
        let raw =
            r#"{"mode":"spawn","name":"work","dir":"/repo","model":"sonnet","command":"/status"}"#;
        let req: CommandRequest = serde_json::from_str(raw).expect("deserialize spawn payload");
        assert_eq!(req.mode, CommandMode::Spawn);
        assert_eq!(req.name.as_deref(), Some("work"));
        assert_eq!(req.dir.as_deref(), Some("/repo"));
        assert_eq!(req.model.as_deref(), Some("sonnet"));
        assert_eq!(req.command, "/status");
    }

    #[test]
    fn command_request_deserializes_spawn_payload_without_optional_fields() {
        let raw = r#"{"mode":"spawn","name":"work","command":"/status"}"#;
        let req: CommandRequest = serde_json::from_str(raw).expect("deserialize");
        assert!(req.dir.is_none());
        assert!(req.model.is_none());
    }

    #[test]
    fn command_request_rejects_unknown_mode() {
        let raw = r#"{"mode":"restart","session":"main","command":"/status"}"#;
        let result: Result<CommandRequest, _> = serde_json::from_str(raw);
        assert!(result.is_err(), "unknown mode must fail to deserialize");
    }

    #[test]
    fn command_request_rejects_missing_mode() {
        let raw = r#"{"session":"main","command":"/status"}"#;
        let result: Result<CommandRequest, _> = serde_json::from_str(raw);
        assert!(result.is_err(), "missing mode must fail to deserialize");
    }

    #[test]
    fn command_request_rejects_missing_command() {
        let raw = r#"{"mode":"inject","session":"main"}"#;
        let result: Result<CommandRequest, _> = serde_json::from_str(raw);
        assert!(result.is_err(), "missing command must fail to deserialize");
    }

    #[test]
    fn command_request_round_trips() {
        let req = CommandRequest {
            mode: CommandMode::Spawn,
            session: None,
            name: Some("work".to_owned()),
            dir: Some("/repo".to_owned()),
            model: Some("opus".to_owned()),
            command: "/status".to_owned(),
        };
        let json = serde_json::to_string(&req).expect("serialize");
        let decoded: CommandRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(req, decoded);
    }

    #[test]
    fn command_request_serializes_omits_absent_optional_fields() {
        let req = CommandRequest {
            mode: CommandMode::Inject,
            session: Some("main".to_owned()),
            name: None,
            dir: None,
            model: None,
            command: "/status".to_owned(),
        };
        let v = serde_json::to_value(&req).expect("serialize");
        assert!(v.get("name").is_none(), "name must be omitted when None");
        assert!(v.get("dir").is_none(), "dir must be omitted when None");
        assert!(v.get("model").is_none(), "model must be omitted when None");
    }

    // ── CommandRequest::validate ─────────────────────────────────────────

    fn inject_request(session: Option<&str>) -> CommandRequest {
        CommandRequest {
            mode: CommandMode::Inject,
            session: session.map(str::to_owned),
            name: None,
            dir: None,
            model: None,
            command: "/status".to_owned(),
        }
    }

    fn spawn_request(name: Option<&str>, model: Option<&str>) -> CommandRequest {
        CommandRequest {
            mode: CommandMode::Spawn,
            session: None,
            name: name.map(str::to_owned),
            dir: None,
            model: model.map(str::to_owned),
            command: "/status".to_owned(),
        }
    }

    #[test]
    fn validate_accepts_inject_with_session() {
        assert_eq!(inject_request(Some("main")).validate(), Ok(()));
    }

    #[test]
    fn validate_rejects_inject_without_session() {
        assert_eq!(
            inject_request(None).validate(),
            Err(CommandValidationError::InjectMissingSession)
        );
    }

    #[test]
    fn validate_rejects_inject_with_empty_session() {
        assert_eq!(
            inject_request(Some("")).validate(),
            Err(CommandValidationError::InjectMissingSession)
        );
    }

    #[test]
    fn validate_accepts_spawn_with_name() {
        assert_eq!(spawn_request(Some("work"), None).validate(), Ok(()));
    }

    #[test]
    fn validate_rejects_spawn_without_name() {
        assert_eq!(
            spawn_request(None, None).validate(),
            Err(CommandValidationError::SpawnMissingName)
        );
    }

    #[test]
    fn validate_rejects_spawn_with_empty_name() {
        assert_eq!(
            spawn_request(Some(""), None).validate(),
            Err(CommandValidationError::SpawnMissingName)
        );
    }

    #[test]
    fn validate_accepts_spawn_with_opus_model() {
        assert_eq!(spawn_request(Some("work"), Some("opus")).validate(), Ok(()));
    }

    #[test]
    fn validate_accepts_spawn_with_sonnet_model() {
        assert_eq!(
            spawn_request(Some("work"), Some("sonnet")).validate(),
            Ok(())
        );
    }

    #[test]
    fn validate_rejects_spawn_with_unknown_model() {
        assert_eq!(
            spawn_request(Some("work"), Some("haiku")).validate(),
            Err(CommandValidationError::UnknownModel("haiku".to_owned()))
        );
    }

    #[test]
    fn validate_rejects_inject_with_unknown_model_too() {
        // Model validation applies regardless of mode.
        let mut req = inject_request(Some("main"));
        req.model = Some("gpt-5".to_owned());
        assert_eq!(
            req.validate(),
            Err(CommandValidationError::UnknownModel("gpt-5".to_owned()))
        );
    }

    #[test]
    fn command_validation_error_display_messages() {
        assert_eq!(
            CommandValidationError::InjectMissingSession.to_string(),
            "mode:\"inject\" requires a non-empty \"session\" field"
        );
        assert_eq!(
            CommandValidationError::SpawnMissingName.to_string(),
            "mode:\"spawn\" requires a non-empty \"name\" field"
        );
        assert!(
            CommandValidationError::UnknownModel("haiku".to_owned())
                .to_string()
                .contains("haiku")
        );
    }

    // ── CommandResponse ───────────────────────────────────────────────────

    #[test]
    fn command_response_serializes_session_field() {
        let resp = CommandResponse {
            session: "work".to_owned(),
        };
        let v = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(v["session"], "work");
    }

    #[test]
    fn command_response_round_trips() {
        let resp = CommandResponse {
            session: "main".to_owned(),
        };
        let json = serde_json::to_string(&resp).expect("serialize");
        let decoded: CommandResponse = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(resp, decoded);
    }

    #[test]
    fn command_response_rejects_missing_session() {
        let raw = r#"{}"#;
        let result: Result<CommandResponse, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "CommandResponse must fail when session is missing"
        );
    }
}
