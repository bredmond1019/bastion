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
/// Only `Echo` is defined at v0; later blocks extend this enum.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WsFrameKind {
    /// Echo — the `/ws` actor reflects the received frame back unchanged (Task 5).
    Echo,
    /// Error — server-side error notification pushed to the client.
    Error,
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
}
