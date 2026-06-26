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
}
