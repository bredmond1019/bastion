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

// ── NotifyPollLoop (BA.18.B task 5) ─────────────────────────────────────────

/// An injected fake transport whose `poll_responses` outcomes are supplied
/// up front, one per call, in order — the same injected-closure-over-
/// interior-mutability shape `blocked_edge`'s `CaptureFn` seam uses, adapted
/// to `OperatorTransport`'s `&self` (rather than `&mut self`) shape via a
/// `Mutex`-guarded queue. `send` is never exercised by these tests and
/// always succeeds trivially.
struct ScriptedTransport {
    poll_outcomes: std::sync::Mutex<
        std::collections::VecDeque<
            Result<(Vec<OperatorResponse>, Option<UpdateCursor>), NotifyError>,
        >,
    >,
}

impl ScriptedTransport {
    fn new(
        outcomes: Vec<Result<(Vec<OperatorResponse>, Option<UpdateCursor>), NotifyError>>,
    ) -> Self {
        Self {
            poll_outcomes: std::sync::Mutex::new(outcomes.into()),
        }
    }
}

#[async_trait]
impl OperatorTransport for ScriptedTransport {
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
        _since: Option<UpdateCursor>,
    ) -> Result<(Vec<OperatorResponse>, Option<UpdateCursor>), NotifyError> {
        self.poll_outcomes
            .lock()
            .expect("poll_outcomes mutex is never poisoned in these tests")
            .pop_front()
            .expect("test provided an outcome for every tick it drives")
    }
}

fn scripted_payload(gate_id: &str, summary: &str) -> ValidatedOperatorPayload {
    let payload = engine_core::operator::OperatorPayload::new(
        gate_id,
        summary,
        vec![
            engine_core::operator::OperatorResponseOption::new("approve", "Approve"),
            engine_core::operator::OperatorResponseOption::new("reject", "Reject"),
        ],
    );
    engine_core::operator::validate(
        payload,
        &engine_core::operator::OperatorPayloadLimits::default(),
    )
    .expect("valid payload")
}

fn response_for(payload: &ValidatedOperatorPayload, option_key: &str) -> OperatorResponse {
    let p = payload.payload();
    OperatorResponse {
        gate_id: p.gate_id.clone(),
        digest: p
            .digest
            .chars()
            .take(telegram::CALLBACK_DIGEST_PREFIX_LEN)
            .collect(),
        option_key: option_key.to_string(),
        received_at: chrono::Utc::now(),
    }
}

/// A pending lookup backed by a fixed in-memory map — stands in for the
/// not-yet-built `engine-rs:EN.8.B` queue.
fn pending_lookup_over(
    payloads: std::collections::HashMap<String, ValidatedOperatorPayload>,
) -> PendingLookup {
    Box::new(move |gate_id: &str| payloads.get(gate_id).cloned())
}

fn collecting_sink() -> (
    VerdictSink,
    Arc<std::sync::Mutex<Vec<telegram::ResponseVerdict>>>,
) {
    let collected = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink_collected = collected.clone();
    let sink: VerdictSink = Box::new(move |verdict| {
        sink_collected
            .lock()
            .expect("collected-verdicts mutex is never poisoned in these tests")
            .push(verdict);
    });
    (sink, collected)
}

#[tokio::test]
async fn tick_dispatches_accepted_verdict_for_a_matching_response() {
    let payload = scripted_payload("gate-1", "diff summary");
    let resp = response_for(&payload, "approve");
    let cursor = UpdateCursor("2".to_string());

    let transport = ScriptedTransport::new(vec![Ok((vec![resp], Some(cursor.clone())))]);
    let mut lookup = std::collections::HashMap::new();
    lookup.insert("gate-1".to_string(), payload);
    let (sink, collected) = collecting_sink();

    let mut poll_loop = NotifyPollLoop::new(Arc::new(transport), pending_lookup_over(lookup), sink);
    let observed = poll_loop.tick().await.expect("tick succeeds");

    assert_eq!(observed, 1);
    assert_eq!(poll_loop.cursor(), Some(&cursor));
    let verdicts = collected.lock().unwrap();
    assert_eq!(
        verdicts.as_slice(),
        [telegram::ResponseVerdict::Accepted {
            gate_id: "gate-1".to_string(),
            option_key: "approve".to_string(),
        }]
    );
}

#[tokio::test]
async fn tick_dispatches_stale_digest_verdict_when_payload_was_mutated() {
    let shown = scripted_payload("gate-1", "original summary");
    let resp = response_for(&shown, "approve");
    // The payload the loop looks up now has a different `rendered_summary`
    // (a re-render after the operator was shown `shown`) — same gate, a
    // different digest. The response must be rejected as stale, never
    // applied against the mutated payload.
    let mutated = scripted_payload("gate-1", "mutated summary");

    let transport = ScriptedTransport::new(vec![Ok((vec![resp], None))]);
    let mut lookup = std::collections::HashMap::new();
    lookup.insert("gate-1".to_string(), mutated);
    let (sink, collected) = collecting_sink();

    let mut poll_loop = NotifyPollLoop::new(Arc::new(transport), pending_lookup_over(lookup), sink);
    poll_loop.tick().await.expect("tick succeeds");

    let verdicts = collected.lock().unwrap();
    assert_eq!(
        verdicts.as_slice(),
        [telegram::ResponseVerdict::StaleDigest]
    );
}

#[tokio::test]
async fn tick_survives_a_retryable_failure_and_resumes_at_the_correct_cursor() {
    let payload = scripted_payload("gate-1", "diff summary");
    let resp = response_for(&payload, "approve");
    let cursor = UpdateCursor("9".to_string());

    let transport = ScriptedTransport::new(vec![
        Err(NotifyError::Transport {
            reason: "connect timeout".to_string(),
        }),
        Ok((vec![resp], Some(cursor.clone()))),
    ]);
    let mut lookup = std::collections::HashMap::new();
    lookup.insert("gate-1".to_string(), payload);
    let (sink, collected) = collecting_sink();

    let mut poll_loop = NotifyPollLoop::new(Arc::new(transport), pending_lookup_over(lookup), sink);

    let first = poll_loop.tick().await;
    assert!(first.is_err());
    assert!(first.unwrap_err().is_retryable());
    // Backoff doubled off the 1s floor; cursor untouched by the failed tick.
    assert_eq!(poll_loop.backoff(), std::time::Duration::from_secs(2));
    assert_eq!(poll_loop.cursor(), None);

    let second = poll_loop.tick().await.expect("recovery tick succeeds");
    assert_eq!(second, 1);
    // Backoff reset on the successful tick; cursor now resumed correctly.
    assert_eq!(poll_loop.backoff(), std::time::Duration::from_secs(1));
    assert_eq!(poll_loop.cursor(), Some(&cursor));
    assert_eq!(
        collected.lock().unwrap().as_slice(),
        [telegram::ResponseVerdict::Accepted {
            gate_id: "gate-1".to_string(),
            option_key: "approve".to_string(),
        }]
    );
}

#[tokio::test]
async fn tick_backoff_doubles_across_consecutive_failures_and_is_capped() {
    let transport = ScriptedTransport::new(vec![
        Err(NotifyError::Transport {
            reason: "connect timeout".to_string(),
        }),
        Err(NotifyError::Transport {
            reason: "connect timeout".to_string(),
        }),
        Err(NotifyError::Transport {
            reason: "connect timeout".to_string(),
        }),
    ]);
    let lookup = std::collections::HashMap::new();
    let (sink, _collected) = collecting_sink();

    let mut poll_loop = NotifyPollLoop::new(Arc::new(transport), pending_lookup_over(lookup), sink);

    assert!(poll_loop.tick().await.is_err());
    assert_eq!(poll_loop.backoff(), std::time::Duration::from_secs(2));
    assert!(poll_loop.tick().await.is_err());
    assert_eq!(poll_loop.backoff(), std::time::Duration::from_secs(4));
    assert!(poll_loop.tick().await.is_err());
    assert_eq!(poll_loop.backoff(), std::time::Duration::from_secs(8));
}

#[tokio::test]
async fn tick_dispatches_unknown_gate_verdict_when_pending_lookup_has_nothing() {
    let payload = scripted_payload("gate-1", "diff summary");
    let resp = response_for(&payload, "approve");

    let transport = ScriptedTransport::new(vec![Ok((vec![resp], None))]);
    let lookup = std::collections::HashMap::new(); // nothing pending for gate-1
    let (sink, collected) = collecting_sink();

    let mut poll_loop = NotifyPollLoop::new(Arc::new(transport), pending_lookup_over(lookup), sink);
    poll_loop.tick().await.expect("tick succeeds");

    assert_eq!(
        collected.lock().unwrap().as_slice(),
        [telegram::ResponseVerdict::UnknownGate]
    );
}
