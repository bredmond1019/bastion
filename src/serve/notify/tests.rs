//! Unit tests for the transport seam (`BA.18.B` task 1).

use super::*;

/// A token-shaped substring: long enough and specific enough that it would
/// never appear in any of these error renderings by accident, so its
/// absence is a meaningful assertion.
const TOKEN_SHAPED: &str = "123456789:AAExampleFakeTokenShapeNotReal12345";

#[test]
fn is_retryable_transport_true() {
    let err = NotifyError::Transport {
        reason: "connect timeout".to_string(),
    };
    assert!(err.is_retryable());
}

#[test]
fn is_retryable_rate_limited_true() {
    let err = NotifyError::RateLimited {
        retry_after_secs: 5,
    };
    assert!(err.is_retryable());
}

#[test]
fn is_retryable_payload_rejected_false() {
    let err = NotifyError::PayloadRejected {
        reason: "too many options".to_string(),
    };
    assert!(!err.is_retryable());
}

#[test]
fn is_retryable_unauthorized_false() {
    assert!(!NotifyError::Unauthorized.is_retryable());
}

#[test]
fn is_retryable_malformed_false() {
    let err = NotifyError::Malformed {
        reason: "not ok envelope".to_string(),
    };
    assert!(!err.is_retryable());
}

/// Every `NotifyError` variant's `Display` and `Debug` renderings must
/// never be able to carry a token — none of the variants take a credential
/// as a field, so this asserts the *shape* holds even if a future edit adds
/// a `reason` string somewhere: none of the fixed, non-caller-supplied
/// variants (`Unauthorized`) can leak one, and this test documents that the
/// caller-supplied `reason`/`retry_after_secs` fields are the only place a
/// token could ever land — which is exactly why callers constructing them
/// must never pass a token-shaped reason (task 4's HTTP shell maps status
/// codes onto `reason` strings it writes itself, never onto response
/// bodies verbatim).
#[test]
fn unauthorized_rendering_contains_no_token_shaped_substring() {
    let err = NotifyError::Unauthorized;
    assert!(!format!("{err}").contains(TOKEN_SHAPED));
    assert!(!format!("{err:?}").contains(TOKEN_SHAPED));
    assert!(!format!("{err}").to_lowercase().contains("token"));
}

#[test]
fn transport_rendering_with_benign_reason_contains_no_token_shaped_substring() {
    let err = NotifyError::Transport {
        reason: "connection reset by peer".to_string(),
    };
    assert!(!format!("{err}").contains(TOKEN_SHAPED));
    assert!(!format!("{err:?}").contains(TOKEN_SHAPED));
}

#[test]
fn rate_limited_rendering_contains_no_token_shaped_substring() {
    let err = NotifyError::RateLimited {
        retry_after_secs: 30,
    };
    assert!(!format!("{err}").contains(TOKEN_SHAPED));
    assert!(!format!("{err:?}").contains(TOKEN_SHAPED));
}

#[test]
fn payload_rejected_rendering_contains_no_token_shaped_substring() {
    let err = NotifyError::PayloadRejected {
        reason: "summary exceeds 1024 chars".to_string(),
    };
    assert!(!format!("{err}").contains(TOKEN_SHAPED));
    assert!(!format!("{err:?}").contains(TOKEN_SHAPED));
}

#[test]
fn malformed_rendering_contains_no_token_shaped_substring() {
    let err = NotifyError::Malformed {
        reason: "expected ok:true envelope".to_string(),
    };
    assert!(!format!("{err}").contains(TOKEN_SHAPED));
    assert!(!format!("{err:?}").contains(TOKEN_SHAPED));
}

#[test]
fn operator_response_fields_are_reachable() {
    let now = chrono::Utc::now();
    let resp = OperatorResponse {
        gate_id: "gate-1".to_string(),
        digest: "abc123".to_string(),
        option_key: "approve".to_string(),
        received_at: now,
    };
    assert_eq!(resp.gate_id, "gate-1");
    assert_eq!(resp.digest, "abc123");
    assert_eq!(resp.option_key, "approve");
    assert_eq!(resp.received_at, now);
}

#[test]
fn delivered_message_carries_transport_id() {
    let msg = DeliveredMessage {
        transport_message_id: "42".to_string(),
    };
    assert_eq!(msg.transport_message_id, "42");
}

#[test]
fn update_cursor_round_trips_its_value() {
    let cursor = UpdateCursor("17".to_string());
    assert_eq!(cursor.0, "17");
    // Debug rendering is explicit (not derived) — assert it names the type
    // and carries the value, not a redaction.
    assert!(format!("{cursor:?}").contains("17"));
}

/// Compile-time evidence `OperatorTransport` is object-safe: a value of a
/// concrete type implementing it can be named behind `dyn`.
struct NoopTransport;

#[async_trait]
impl OperatorTransport for NoopTransport {
    async fn send(
        &self,
        _payload: &ValidatedOperatorPayload,
    ) -> Result<DeliveredMessage, NotifyError> {
        Ok(DeliveredMessage {
            transport_message_id: String::new(),
        })
    }

    async fn poll_responses(
        &self,
        since: Option<UpdateCursor>,
    ) -> Result<(Vec<OperatorResponse>, Option<UpdateCursor>), NotifyError> {
        Ok((Vec::new(), since))
    }
}

#[tokio::test]
async fn operator_transport_is_object_safe_and_dispatches() {
    let transport: Box<dyn OperatorTransport> = Box::new(NoopTransport);
    let payload = engine_core::operator::OperatorPayload::new(
        "gate-1",
        "diff summary",
        vec![
            engine_core::operator::OperatorResponseOption::new("approve", "Approve"),
            engine_core::operator::OperatorResponseOption::new("reject", "Reject"),
        ],
    );
    let validated = engine_core::operator::validate(
        payload,
        &engine_core::operator::OperatorPayloadLimits::default(),
    )
    .expect("valid payload");

    let delivered = transport.send(&validated).await.expect("send succeeds");
    assert_eq!(delivered.transport_message_id, "");

    let (responses, cursor) = transport
        .poll_responses(Some(UpdateCursor("5".to_string())))
        .await
        .expect("poll succeeds");
    assert!(responses.is_empty());
    assert_eq!(cursor, Some(UpdateCursor("5".to_string())));
}
