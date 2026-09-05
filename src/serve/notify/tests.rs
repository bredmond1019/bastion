//! Unit tests for the transport seam (`BA.18.B` task 1).

use async_trait::async_trait;

use super::*;
// NOTE: deliberately does NOT `use actix_web::test` at module scope — that
// shadows the built-in `#[test]` attribute every other test in this file
// relies on (see `src/serve/mod.rs`'s own comment on this same trap). The
// one test below that needs `actix_web::test` utilities refers to them via
// the fully-qualified `actix_web::test::` path instead.
use actix_web::{App, HttpResponse, web};

/// A token-shaped substring: long enough and specific enough that it would
/// never appear in any of these error renderings by accident, so its
/// absence is a meaningful assertion. A `.` breaks the run so this literal
/// does not itself trip the repo's Telegram-bot-token secret scan
/// (`[0-9]{6,12}:[A-Za-z0-9_-]{30,}`, see `planning/BA.18.B/tasks.md` task 7)
/// while still reading as a fake credential.
const TOKEN_SHAPED: &str = "123456789:AAExampleFake.TokenShapeNotReal.12345";

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
        ack: None,
        message: None,
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
/// One scripted `poll` outcome: the responses and cursor a call returns, or
/// the error it fails with. Named rather than spelled inline at both use
/// sites — clippy::type_complexity, and the two spellings could drift apart.
type ScriptedPollOutcome = Result<(Vec<OperatorResponse>, Option<UpdateCursor>), NotifyError>;

struct ScriptedTransport {
    poll_outcomes: std::sync::Mutex<std::collections::VecDeque<ScriptedPollOutcome>>,
    /// Scripted outcomes for `acknowledge`, one per call, in order. Empty
    /// (the default) means every call succeeds — most tests never script
    /// this and don't care about acknowledgement at all.
    ack_outcomes: std::sync::Mutex<std::collections::VecDeque<Result<(), NotifyError>>>,
    /// Every `acknowledge` call this transport observed, in order —
    /// `(gate_id, verdict)` — so task-4 tests can assert exactly which
    /// responses were acknowledged and with which verdict.
    ack_calls: std::sync::Mutex<Vec<(String, ResponseVerdict)>>,
    /// Optional shared event log an `acknowledge` call appends `"ack"` to,
    /// so a test can interleave it with the sink's own `"verdict"` pushes
    /// and assert ack-before-dispatch ordering.
    order_log: std::sync::Mutex<Option<Arc<std::sync::Mutex<Vec<String>>>>>,
    /// Every payload a `send` call was given, in order — task 4 (BA.21.A)
    /// uses this to prove an engine-side caller resolving this transport
    /// out of `app_data` actually reaches `send` with a
    /// `ValidatedOperatorPayload`, rather than merely compiling.
    send_calls: std::sync::Mutex<Vec<ValidatedOperatorPayload>>,
}

impl ScriptedTransport {
    fn new(outcomes: Vec<ScriptedPollOutcome>) -> Self {
        Self {
            poll_outcomes: std::sync::Mutex::new(outcomes.into()),
            ack_outcomes: std::sync::Mutex::new(std::collections::VecDeque::new()),
            ack_calls: std::sync::Mutex::new(Vec::new()),
            order_log: std::sync::Mutex::new(None),
            send_calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Script the outcomes `acknowledge` returns, one per call, in order.
    fn with_ack_outcomes(self, outcomes: Vec<Result<(), NotifyError>>) -> Self {
        *self
            .ack_outcomes
            .lock()
            .expect("ack_outcomes mutex is never poisoned in these tests") = outcomes.into();
        self
    }

    /// Wire a shared order-event log so `acknowledge` calls record `"ack"`
    /// into it, interleaved with a sink pushing `"verdict"`.
    fn with_order_log(self, log: Arc<std::sync::Mutex<Vec<String>>>) -> Self {
        *self
            .order_log
            .lock()
            .expect("order_log mutex is never poisoned in these tests") = Some(log);
        self
    }

    fn ack_calls(&self) -> Vec<(String, ResponseVerdict)> {
        self.ack_calls
            .lock()
            .expect("ack_calls mutex is never poisoned in these tests")
            .clone()
    }

    /// Every payload a `send` call was given, in order.
    fn send_calls(&self) -> Vec<ValidatedOperatorPayload> {
        self.send_calls
            .lock()
            .expect("send_calls mutex is never poisoned in these tests")
            .clone()
    }
}

#[async_trait]
impl OperatorTransport for ScriptedTransport {
    async fn send(
        &self,
        payload: &ValidatedOperatorPayload,
    ) -> Result<DeliveredMessage, NotifyError> {
        self.send_calls
            .lock()
            .expect("send_calls mutex is never poisoned in these tests")
            .push(payload.clone());
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

    async fn acknowledge(
        &self,
        response: &OperatorResponse,
        verdict: &ResponseVerdict,
    ) -> Result<(), NotifyError> {
        self.ack_calls
            .lock()
            .expect("ack_calls mutex is never poisoned in these tests")
            .push((response.gate_id.clone(), verdict.clone()));
        if let Some(log) = self
            .order_log
            .lock()
            .expect("order_log mutex is never poisoned in these tests")
            .as_ref()
        {
            log.lock()
                .expect("shared order log mutex is never poisoned in these tests")
                .push("ack".to_string());
        }
        self.ack_outcomes
            .lock()
            .expect("ack_outcomes mutex is never poisoned in these tests")
            .pop_front()
            .unwrap_or(Ok(()))
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
        ack: None,
        message: None,
    }
}

/// A pending lookup backed by a fixed in-memory map — stands in for the
/// not-yet-built `engine-rs:EN.8.B` queue.
fn pending_lookup_over(
    payloads: std::collections::HashMap<String, ValidatedOperatorPayload>,
) -> PendingLookup {
    Box::new(move |gate_id: &str| payloads.get(gate_id).cloned())
}

fn collecting_sink() -> (VerdictSink, Arc<std::sync::Mutex<Vec<ResponseVerdict>>>) {
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

/// Like [`collecting_sink`], but also appends `"verdict"` to a shared order
/// log — paired with [`ScriptedTransport::with_order_log`] to assert
/// ack-before-dispatch ordering (task 4).
fn collecting_sink_with_log(
    log: Arc<std::sync::Mutex<Vec<String>>>,
) -> (VerdictSink, Arc<std::sync::Mutex<Vec<ResponseVerdict>>>) {
    let collected = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink_collected = collected.clone();
    let sink: VerdictSink = Box::new(move |verdict| {
        log.lock()
            .expect("shared order log mutex is never poisoned in these tests")
            .push("verdict".to_string());
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
    let (digest, decided_at) = (resp.digest.clone(), resp.received_at);
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
        [ResponseVerdict::Accepted {
            gate_id: "gate-1".to_string(),
            option_key: "approve".to_string(),
            digest,
            decided_at,
        }]
    );
}

#[tokio::test]
async fn tick_dispatches_stale_digest_verdict_when_payload_was_mutated() {
    let shown = scripted_payload("gate-1", "original summary");
    let resp = response_for(&shown, "approve");
    let (digest, decided_at) = (resp.digest.clone(), resp.received_at);
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
        [ResponseVerdict::StaleDigest {
            gate_id: "gate-1".to_string(),
            option_key: "approve".to_string(),
            digest,
            decided_at,
        }]
    );
}

#[tokio::test]
async fn tick_survives_a_retryable_failure_and_resumes_at_the_correct_cursor() {
    let payload = scripted_payload("gate-1", "diff summary");
    let resp = response_for(&payload, "approve");
    let (digest, decided_at) = (resp.digest.clone(), resp.received_at);
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
        [ResponseVerdict::Accepted {
            gate_id: "gate-1".to_string(),
            option_key: "approve".to_string(),
            digest,
            decided_at,
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
        [ResponseVerdict::UnknownGate]
    );
}

// ── Acknowledge every verdict, ack-before-dispatch (ticket-telegram-answer-
//    callback task 4) ───────────────────────────────────────────────────────

#[tokio::test]
async fn tick_acknowledges_every_verdict_arm_exactly_once() {
    // gate-1 resolves Accepted, gate-2 resolves StaleDigest (mutated after
    // being shown), gate-3 resolves UnknownGate (never registered).
    let accepted_payload = scripted_payload("gate-1", "diff summary");
    let accepted_resp = response_for(&accepted_payload, "approve");
    let (accepted_digest, accepted_decided_at) =
        (accepted_resp.digest.clone(), accepted_resp.received_at);

    let shown = scripted_payload("gate-2", "original summary");
    let stale_resp = response_for(&shown, "approve");
    let (stale_digest, stale_decided_at) = (stale_resp.digest.clone(), stale_resp.received_at);
    let mutated = scripted_payload("gate-2", "mutated summary");

    let unknown_payload = scripted_payload("gate-3", "diff summary");
    let unknown_resp = response_for(&unknown_payload, "approve");

    let transport = ScriptedTransport::new(vec![Ok((
        vec![accepted_resp, stale_resp, unknown_resp],
        None,
    ))]);
    let mut lookup = std::collections::HashMap::new();
    lookup.insert("gate-1".to_string(), accepted_payload);
    lookup.insert("gate-2".to_string(), mutated);
    let (sink, collected) = collecting_sink();

    let transport = Arc::new(transport);
    let dyn_transport: Arc<dyn OperatorTransport> = Arc::clone(&transport) as _;
    let mut poll_loop = NotifyPollLoop::new(dyn_transport, pending_lookup_over(lookup), sink);
    poll_loop.tick().await.expect("tick succeeds");

    let verdicts = collected.lock().unwrap();
    assert_eq!(
        verdicts.as_slice(),
        [
            ResponseVerdict::Accepted {
                gate_id: "gate-1".to_string(),
                option_key: "approve".to_string(),
                digest: accepted_digest.clone(),
                decided_at: accepted_decided_at,
            },
            ResponseVerdict::StaleDigest {
                gate_id: "gate-2".to_string(),
                option_key: "approve".to_string(),
                digest: stale_digest.clone(),
                decided_at: stale_decided_at,
            },
            ResponseVerdict::UnknownGate,
        ]
    );

    let acks = transport.ack_calls();
    assert_eq!(acks.len(), 3, "exactly one acknowledge call per response");
    assert_eq!(
        acks,
        vec![
            (
                "gate-1".to_string(),
                ResponseVerdict::Accepted {
                    gate_id: "gate-1".to_string(),
                    option_key: "approve".to_string(),
                    digest: accepted_digest,
                    decided_at: accepted_decided_at,
                }
            ),
            (
                "gate-2".to_string(),
                ResponseVerdict::StaleDigest {
                    gate_id: "gate-2".to_string(),
                    option_key: "approve".to_string(),
                    digest: stale_digest,
                    decided_at: stale_decided_at,
                }
            ),
            ("gate-3".to_string(), ResponseVerdict::UnknownGate),
        ]
    );
}

#[tokio::test]
async fn tick_acknowledges_before_dispatching_to_the_sink() {
    let payload = scripted_payload("gate-1", "diff summary");
    let resp = response_for(&payload, "approve");
    let order_log = Arc::new(std::sync::Mutex::new(Vec::new()));

    let transport =
        ScriptedTransport::new(vec![Ok((vec![resp], None))]).with_order_log(Arc::clone(&order_log));
    let mut lookup = std::collections::HashMap::new();
    lookup.insert("gate-1".to_string(), payload);
    let (sink, _collected) = collecting_sink_with_log(Arc::clone(&order_log));

    let mut poll_loop = NotifyPollLoop::new(Arc::new(transport), pending_lookup_over(lookup), sink);
    poll_loop.tick().await.expect("tick succeeds");

    assert_eq!(
        order_log.lock().unwrap().as_slice(),
        ["ack".to_string(), "verdict".to_string()],
        "acknowledge must happen before the verdict reaches the sink"
    );
}

#[tokio::test]
async fn tick_still_dispatches_verdict_exactly_once_when_acknowledge_keeps_failing() {
    let payload = scripted_payload("gate-1", "diff summary");
    let resp = response_for(&payload, "approve");
    let (digest, decided_at) = (resp.digest.clone(), resp.received_at);
    let cursor = UpdateCursor("5".to_string());

    // Both the initial attempt and the single retry fail — acknowledge must
    // give up after that, per the "retried at most once" constraint, and
    // still dispatch the resolved verdict.
    let transport = ScriptedTransport::new(vec![
        Ok((vec![resp], Some(cursor.clone()))),
        Ok((Vec::new(), None)), // next tick: nothing new to reprocess
    ])
    .with_ack_outcomes(vec![
        Err(NotifyError::Transport {
            reason: "connect timeout".to_string(),
        }),
        Err(NotifyError::Transport {
            reason: "connect timeout".to_string(),
        }),
    ]);
    let mut lookup = std::collections::HashMap::new();
    lookup.insert("gate-1".to_string(), payload);
    let (sink, collected) = collecting_sink();

    let transport = Arc::new(transport);
    let dyn_transport: Arc<dyn OperatorTransport> = Arc::clone(&transport) as _;
    let mut poll_loop = NotifyPollLoop::new(dyn_transport, pending_lookup_over(lookup), sink);
    poll_loop
        .tick()
        .await
        .expect("tick succeeds despite ack failure");

    assert_eq!(
        collected.lock().unwrap().as_slice(),
        [ResponseVerdict::Accepted {
            gate_id: "gate-1".to_string(),
            option_key: "approve".to_string(),
            digest,
            decided_at,
        }],
        "the verdict must still be dispatched exactly once"
    );
    assert_eq!(
        transport.ack_calls().len(),
        2,
        "acknowledge is attempted once, then retried at most once"
    );

    // Next tick observes no new responses (per the script) — the failed-ack
    // response is never reprocessed.
    let observed = poll_loop.tick().await.expect("second tick succeeds");
    assert_eq!(observed, 0);
    assert_eq!(
        collected.lock().unwrap().len(),
        1,
        "the response must not be dispatched a second time on the next tick"
    );
}

// ── PendingPayloads (ticket-notify-send-trigger task 1) ─────────────────────

#[test]
fn pending_payloads_round_trips_by_gate_id() {
    let registry = PendingPayloads::new();
    let payload = scripted_payload("gate-1", "diff summary");
    registry.insert(payload.clone());

    let found = registry.get("gate-1").expect("payload was inserted");
    assert_eq!(found, payload);
}

#[test]
fn pending_payloads_get_returns_none_for_unknown_gate() {
    let registry = PendingPayloads::new();
    assert_eq!(registry.get("never-sent"), None);
}

#[test]
fn pending_payloads_remove_returns_and_evicts_the_entry() {
    let registry = PendingPayloads::new();
    let payload = scripted_payload("gate-1", "diff summary");
    registry.insert(payload.clone());

    let removed = registry.remove("gate-1").expect("payload was inserted");
    assert_eq!(removed, payload);
    assert_eq!(registry.get("gate-1"), None);
}

#[test]
fn pending_payloads_remove_on_unknown_gate_is_a_no_op() {
    let registry = PendingPayloads::new();
    assert_eq!(registry.remove("never-sent"), None);
}

#[test]
fn pending_payloads_lookup_returns_the_existing_pending_lookup_type() {
    let registry = Arc::new(PendingPayloads::new());
    let payload = scripted_payload("gate-1", "diff summary");
    registry.insert(payload.clone());

    let lookup: PendingLookup = registry.lookup();
    assert_eq!(lookup("gate-1"), Some(payload));
    assert_eq!(lookup("never-sent"), None);
}

#[test]
fn pending_payloads_at_capacity_accepts_without_eviction() {
    let registry = PendingPayloads::new();
    for i in 0..PendingPayloads::CAPACITY {
        registry.insert(scripted_payload(&format!("gate-{i}"), "diff summary"));
    }
    // Every one of the CAPACITY entries is still present — nothing evicted
    // yet at exactly the cap.
    for i in 0..PendingPayloads::CAPACITY {
        assert!(
            registry.get(&format!("gate-{i}")).is_some(),
            "gate-{i} should still be pending at exactly capacity"
        );
    }
}

#[test]
fn pending_payloads_one_over_capacity_evicts_oldest_first() {
    let registry = PendingPayloads::new();
    for i in 0..PendingPayloads::CAPACITY {
        registry.insert(scripted_payload(&format!("gate-{i}"), "diff summary"));
    }
    // One more insert past the cap must evict gate-0 (the oldest by
    // insertion order), never a newer entry.
    registry.insert(scripted_payload("gate-overflow", "diff summary"));

    assert_eq!(
        registry.get("gate-0"),
        None,
        "oldest entry must be evicted once the registry is one over capacity"
    );
    assert!(registry.get("gate-overflow").is_some());
    assert!(
        registry.get("gate-1").is_some(),
        "the second-oldest entry must survive a single one-over-capacity eviction"
    );
}

#[test]
fn pending_payloads_reinsert_of_existing_gate_does_not_trigger_eviction() {
    let registry = PendingPayloads::new();
    for i in 0..PendingPayloads::CAPACITY {
        registry.insert(scripted_payload(&format!("gate-{i}"), "diff summary"));
    }
    // Re-inserting an already-present gate_id must not evict anything —
    // the registry is still exactly at capacity, not over it.
    registry.insert(scripted_payload("gate-0", "updated summary"));

    for i in 0..PendingPayloads::CAPACITY {
        assert!(
            registry.get(&format!("gate-{i}")).is_some(),
            "gate-{i} must still be pending after a same-key reinsert"
        );
    }
}

#[tokio::test]
async fn pending_payloads_concurrent_insert_and_get_from_two_tasks() {
    let registry = Arc::new(PendingPayloads::new());

    let writer = {
        let registry = Arc::clone(&registry);
        tokio::spawn(async move {
            for i in 0..64 {
                registry.insert(scripted_payload(&format!("gate-{i}"), "diff summary"));
            }
        })
    };
    let reader = {
        let registry = Arc::clone(&registry);
        tokio::spawn(async move {
            // Reads racing the writer must never panic or deadlock, and
            // any hit must be the exact payload for that gate_id.
            for i in 0..64 {
                if let Some(found) = registry.get(&format!("gate-{i}")) {
                    assert_eq!(found.payload().gate_id, format!("gate-{i}"));
                }
            }
        })
    };

    writer.await.expect("writer task completes");
    reader.await.expect("reader task completes");

    for i in 0..64 {
        assert!(registry.get(&format!("gate-{i}")).is_some());
    }
}

// ── End-to-end: PendingPayloads wired as NotifyPollLoop's PendingLookup
//    (ticket-notify-send-trigger task 3) ────────────────────────────────────
//
// These drive the real loop — `PendingPayloads::lookup()` as `PendingLookup`,
// the `ScriptedTransport` fake from BA.18.B — the same shape
// `run_server` wires in production, minus the network. They exercise
// `Accepted` and `StaleDigest` end-to-end (not just the pure resolver), the
// never-sent `UnknownGate` path, and the replay-after-accept rule.

/// A verdict sink that also removes an `Accepted` response's entry from the
/// registry, mirroring `run_server`'s wiring exactly (task 3): the loop
/// itself never mutates the registry, so the removal-on-accept behaviour is
/// this closure's job, both in production and here.
fn removing_sink(
    registry: Arc<PendingPayloads>,
) -> (VerdictSink, Arc<std::sync::Mutex<Vec<ResponseVerdict>>>) {
    let collected = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink_collected = collected.clone();
    let sink: VerdictSink = Box::new(move |verdict| {
        if let ResponseVerdict::Accepted { gate_id, .. } = &verdict {
            registry.remove(gate_id);
        }
        sink_collected
            .lock()
            .expect("collected-verdicts mutex is never poisoned in these tests")
            .push(verdict);
    });
    (sink, collected)
}

#[tokio::test]
async fn e2e_sent_then_tapped_resolves_accepted() {
    let registry = Arc::new(PendingPayloads::new());
    let payload = scripted_payload("gate-1", "diff summary");
    registry.insert(payload.clone());

    let resp = response_for(&payload, "approve");
    let (digest, decided_at) = (resp.digest.clone(), resp.received_at);
    let transport = ScriptedTransport::new(vec![Ok((vec![resp], None))]);
    let (sink, collected) = removing_sink(Arc::clone(&registry));

    let mut poll_loop = NotifyPollLoop::new(Arc::new(transport), registry.lookup(), sink);
    poll_loop.tick().await.expect("tick succeeds");

    assert_eq!(
        collected.lock().unwrap().as_slice(),
        [ResponseVerdict::Accepted {
            gate_id: "gate-1".to_string(),
            option_key: "approve".to_string(),
            digest,
            decided_at,
        }]
    );
}

#[tokio::test]
async fn e2e_sent_then_mutated_then_tapped_resolves_stale_digest() {
    let registry = Arc::new(PendingPayloads::new());
    let shown = scripted_payload("gate-1", "original summary");
    let resp = response_for(&shown, "approve");
    let (digest, decided_at) = (resp.digest.clone(), resp.received_at);

    // Re-render after the operator was shown `shown` — same gate, a
    // different digest, replacing the registered entry.
    let mutated = scripted_payload("gate-1", "mutated summary");
    registry.insert(mutated);

    let transport = ScriptedTransport::new(vec![Ok((vec![resp], None))]);
    let (sink, collected) = removing_sink(Arc::clone(&registry));

    let mut poll_loop = NotifyPollLoop::new(Arc::new(transport), registry.lookup(), sink);
    poll_loop.tick().await.expect("tick succeeds");

    assert_eq!(
        collected.lock().unwrap().as_slice(),
        [ResponseVerdict::StaleDigest {
            gate_id: "gate-1".to_string(),
            option_key: "approve".to_string(),
            digest,
            decided_at,
        }]
    );
    // A stale-digest verdict must not be applied — the entry stays pending
    // (unlike `Accepted`, which removes it) so a corrected re-tap could
    // still resolve it.
    assert!(registry.get("gate-1").is_some());
}

#[tokio::test]
async fn e2e_never_sent_gate_resolves_unknown_gate() {
    let registry = Arc::new(PendingPayloads::new());
    // Nothing registered for "gate-1" — the registry never saw this gate.
    let payload = scripted_payload("gate-1", "diff summary");
    let resp = response_for(&payload, "approve");

    let transport = ScriptedTransport::new(vec![Ok((vec![resp], None))]);
    let (sink, collected) = removing_sink(Arc::clone(&registry));

    let mut poll_loop = NotifyPollLoop::new(Arc::new(transport), registry.lookup(), sink);
    poll_loop.tick().await.expect("tick succeeds");

    assert_eq!(
        collected.lock().unwrap().as_slice(),
        [ResponseVerdict::UnknownGate]
    );
}

#[tokio::test]
async fn e2e_replayed_tap_of_an_already_accepted_button_resolves_unknown_gate() {
    let registry = Arc::new(PendingPayloads::new());
    let payload = scripted_payload("gate-1", "diff summary");
    registry.insert(payload.clone());

    let resp = response_for(&payload, "approve");
    let (digest, decided_at) = (resp.digest.clone(), resp.received_at);
    // Same response observed twice — e.g. a replayed webhook/poll delivery
    // of the same tap.
    let transport =
        ScriptedTransport::new(vec![Ok((vec![resp.clone()], None)), Ok((vec![resp], None))]);
    let (sink, collected) = removing_sink(Arc::clone(&registry));

    let mut poll_loop = NotifyPollLoop::new(Arc::new(transport), registry.lookup(), sink);
    poll_loop.tick().await.expect("first tick succeeds");
    poll_loop.tick().await.expect("second tick succeeds");

    let verdicts = collected.lock().unwrap();
    assert_eq!(
        verdicts.as_slice(),
        [
            ResponseVerdict::Accepted {
                gate_id: "gate-1".to_string(),
                option_key: "approve".to_string(),
                digest,
                decided_at,
            },
            ResponseVerdict::UnknownGate,
        ]
    );
    assert!(
        registry.get("gate-1").is_none(),
        "accepted entry must have been removed from the registry"
    );
}

// ── ApproveAndRunSeams resolve-and-execute coverage
//    (ticket-approve-and-run-seams task 4) ────────────────────────────────
//
// Exercises the sink's real action (task 3): a resolved `ResponseVerdict`
// converted via `crate::serve::approve_and_run_verdict_for` and driven
// through `ApproveAndRunSeams::resolve_verdict`, spawned non-blocking
// exactly as `run_server`'s wiring does. Hermetic per the ticket's Testing
// Strategy — no Telegram, no network, no database:
// `ApproveAndRunSeams::new` takes injected `ApprovalLedger`/`HttpPost`
// seams precisely so this is possible.
//
// The verdicts here are built directly (not via `telegram::resolve_response`
// against a real Telegram `CallbackData`) because `resolve_response`
// truncates the presented digest to `CALLBACK_DIGEST_PREFIX_LEN` for
// Telegram's callback-data size limit, while `ApproveAndRunSeams::
// resolve_verdict` (via `engine_core`'s `record_decision`) compares against
// the item's full stored digest — the two digest lengths are a Telegram
// transport-layer concern that is orthogonal to what this suite covers:
// whether a resolved verdict, once built, drives the seams correctly and
// without blocking. `approve_and_run_verdict_for`'s own pure-conversion
// tests (`src/serve/mod.rs::approve_and_run_seams_wiring_tests`) already
// prove the field-by-field mapping off `ResponseVerdict`; these tests start
// one step later, from an already-built verdict, exactly like
// `engine_core::workflows::approve_and_run::seams_tests` does at the
// engine-core layer.
//
// The `/api/notify/test` regression this ticket calls out is covered
// separately: `resolve_pending_lookup`'s composition (engine queue first,
// falling back to the test registry) is asserted by
// `crate::serve::approve_and_run_seams_wiring_tests::
// a_payload_sent_via_notify_test_still_resolves`, and this file's `e2e_*`
// tests above already prove the loop still resolves taps against
// `PendingPayloads` end to end.
mod resolve_and_execute_tests {
    use super::super::*;
    use crate::serve::approve_and_run_verdict_for;
    use engine_core::nodes::harvest_gate::pending_harvest_record;
    use engine_core::nodes::http_post::{HttpPost, HttpPostResponse, StubHttpPost};
    use engine_core::operator::OperatorPayloadLimits;
    use engine_core::operator::ledger::{ApprovalLedger, FileApprovalLedger, LedgerDecision};
    use engine_core::operator::queue::{OperatorQueue, OperatorQueuePolicy};
    use engine_core::workflows::approve_and_run::{
        ApproveAndRunPolicy, ApproveAndRunSeams, OPTION_APPROVE, OPTION_SKIP, PendingHarvestRecord,
    };
    use std::sync::Mutex as StdMutex;
    use std::time::{Duration, Instant};

    fn harvest_record(artifact_id: &str) -> PendingHarvestRecord {
        let value = pending_harvest_record(
            artifact_id,
            "https://synapse.example/ingest/learning-artifact",
            serde_json::json!({"title": "some artifact"}),
            vec!["docs/foo.md".to_string()],
        );
        PendingHarvestRecord::from_value(&value).expect("record parses")
    }

    /// A fresh `ApproveAndRunSeams` over an empty queue and a
    /// `FileApprovalLedger` pointed at a throwaway tempdir path, plus a
    /// shared handle to that same ledger so tests can `read_all()` it —
    /// mirrors `src/serve/mod.rs::approve_and_run_seams_wiring_tests::
    /// seams()` exactly, extended to hand back the ledger (that module
    /// only needed `lookup_pending`, never a ledger read).
    fn seams_with(http_post: Arc<dyn HttpPost>) -> (ApproveAndRunSeams, Arc<FileApprovalLedger>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let ledger = Arc::new(FileApprovalLedger::new(dir.path().join("ledger.jsonl")));
        let seams = ApproveAndRunSeams::new(
            Arc::new(StdMutex::new(OperatorQueue::new(
                OperatorQueuePolicy::default(),
            ))),
            ledger.clone(),
            http_post,
            OperatorPayloadLimits::default(),
            ApproveAndRunPolicy::default(),
        );
        (seams, ledger)
    }

    #[tokio::test]
    async fn resolve_and_execute_writes_one_ledger_row_and_posts_the_stored_payload_byte_for_byte()
    {
        let stub = StubHttpPost::succeeding(serde_json::json!({"ok": true}));
        let stub_dyn: Arc<dyn HttpPost> = Arc::new(stub.clone());
        let (seams, ledger) = seams_with(stub_dyn);

        let report = seams.drain(&[harvest_record("artifact-1")], chrono::Utc::now());
        let delivered = report.delivered.expect("one item delivered");

        let verdict = ResponseVerdict::Accepted {
            gate_id: delivered.item_id.clone(),
            option_key: OPTION_APPROVE.to_string(),
            digest: delivered.payload.digest.clone(),
            decided_at: chrono::Utc::now(),
        };
        let engine_verdict = approve_and_run_verdict_for(&verdict, "operator")
            .expect("Accepted always converts to a verdict");

        let resolution = seams
            .resolve_verdict(engine_verdict)
            .await
            .expect("a matched-digest approve should resolve");

        assert!(resolution.outcome.ledger_outcome.should_execute);
        let rows = ledger.read_all();
        assert_eq!(rows.len(), 1, "exactly one ledger row");
        assert_eq!(rows[0].decision, LedgerDecision::Approved);

        assert!(resolution.executed.is_some(), "execution result present");
        let (url, body) = stub.last_call().expect("exactly one POST should occur");
        assert_eq!(url, "https://synapse.example/ingest/learning-artifact");
        assert_eq!(body, serde_json::json!({"title": "some artifact"}));
    }

    #[tokio::test]
    async fn digest_mismatch_requeues_with_zero_posts_and_stays_resolvable() {
        let stub = StubHttpPost::succeeding(serde_json::json!({"ok": true}));
        let stub_dyn: Arc<dyn HttpPost> = Arc::new(stub.clone());
        let (seams, ledger) = seams_with(stub_dyn);

        let report = seams.drain(&[harvest_record("artifact-1")], chrono::Utc::now());
        let delivered = report.delivered.expect("one item delivered");

        let verdict = ResponseVerdict::Accepted {
            gate_id: delivered.item_id.clone(),
            option_key: OPTION_APPROVE.to_string(),
            digest: "a-different-digest-than-was-delivered".to_string(),
            decided_at: chrono::Utc::now(),
        };
        let engine_verdict = approve_and_run_verdict_for(&verdict, "operator")
            .expect("Accepted always converts to a verdict");

        let resolution = seams
            .resolve_verdict(engine_verdict)
            .await
            .expect("a mismatched digest should still resolve, as a requeue");

        assert!(!resolution.outcome.ledger_outcome.should_execute);
        assert!(resolution.outcome.requeued);
        assert!(resolution.executed.is_none());
        let rows = ledger.read_all();
        assert_eq!(rows.len(), 1, "exactly one ledger row");
        assert_eq!(rows[0].decision, LedgerDecision::Requeued);
        assert!(
            stub.last_call().is_none(),
            "a digest mismatch must never POST"
        );

        // The item was re-queued, never dropped — a fresh delivery pass
        // (an empty drain, since nothing new arrived) redelivers it under
        // the same gate_id, proving it is still resolvable from the queue.
        let redelivered = seams.drain(&[], chrono::Utc::now());
        assert_eq!(
            redelivered.delivered.map(|item| item.item_id),
            Some(delivered.item_id.clone()),
            "the requeued item should be the next one delivered"
        );
        assert!(seams.lookup_pending(&delivered.item_id).is_some());
    }

    #[tokio::test]
    async fn skip_writes_its_row_with_zero_posts() {
        let stub = StubHttpPost::succeeding(serde_json::json!({"ok": true}));
        let stub_dyn: Arc<dyn HttpPost> = Arc::new(stub.clone());
        let (seams, ledger) = seams_with(stub_dyn);

        let report = seams.drain(&[harvest_record("artifact-1")], chrono::Utc::now());
        let delivered = report.delivered.expect("one item delivered");

        let verdict = ResponseVerdict::Accepted {
            gate_id: delivered.item_id.clone(),
            option_key: OPTION_SKIP.to_string(),
            digest: delivered.payload.digest.clone(),
            decided_at: chrono::Utc::now(),
        };
        let engine_verdict = approve_and_run_verdict_for(&verdict, "operator")
            .expect("Accepted always converts to a verdict");

        let resolution = seams
            .resolve_verdict(engine_verdict)
            .await
            .expect("a matched-digest skip should resolve");

        assert!(!resolution.outcome.ledger_outcome.should_execute);
        assert!(resolution.executed.is_none());
        let rows = ledger.read_all();
        assert_eq!(rows.len(), 1, "exactly one ledger row");
        assert_eq!(rows[0].decision, LedgerDecision::Skipped);
        assert!(stub.last_call().is_none(), "a skip must never POST");
    }

    #[tokio::test]
    async fn unknown_gate_still_resolves_to_unknown_gate_and_writes_no_row() {
        let stub: Arc<dyn HttpPost> =
            Arc::new(StubHttpPost::succeeding(serde_json::json!({"ok": true})));
        let (seams, ledger) = seams_with(stub);

        let verdict = ResponseVerdict::Accepted {
            gate_id: "never-drained".to_string(),
            option_key: OPTION_APPROVE.to_string(),
            digest: "whatever".to_string(),
            decided_at: chrono::Utc::now(),
        };
        let engine_verdict = approve_and_run_verdict_for(&verdict, "operator")
            .expect("Accepted always converts to a verdict");

        let err = seams
            .resolve_verdict(engine_verdict)
            .await
            .expect_err("a gate nothing ever drained should error, not silently no-op");

        assert_eq!(
            err,
            engine_core::workflows::approve_and_run::ApproveAndRunSeamError::UnknownGate(
                "never-drained".to_string()
            )
        );
        assert!(
            ledger.read_all().is_empty(),
            "an unknown gate must not write a ledger row"
        );
    }

    /// An `HttpPost` that sleeps before delegating to an inner
    /// [`StubHttpPost`] — lets a test observe whether a caller waited for
    /// the POST to finish or moved on without it.
    struct DelayedHttpPost {
        delay: Duration,
        inner: StubHttpPost,
    }

    #[async_trait::async_trait]
    impl HttpPost for DelayedHttpPost {
        async fn post(
            &self,
            url: &str,
            json_body: serde_json::Value,
        ) -> Result<HttpPostResponse, String> {
            tokio::time::sleep(self.delay).await;
            self.inner.post(url, json_body).await
        }
    }

    /// Mirrors `run_server`'s sink wiring (`ticket-approve-and-run-seams`
    /// task 3): a resolved verdict is converted, then the actual
    /// `resolve_verdict` call is spawned onto the executor rather than
    /// awaited inline, so the sink itself returns immediately regardless of
    /// how long resolution takes. This test spawns onto the ambient tokio
    /// runtime (`tokio::spawn`) rather than `actix_web::rt::spawn` — the
    /// property under test (the sink call does not await the spawned
    /// future) is the same either way; only production's choice of
    /// executor differs, for the reason documented at the `run_server`
    /// call site.
    #[tokio::test]
    async fn sink_returns_without_awaiting_a_slow_resolution() {
        let delayed = DelayedHttpPost {
            delay: Duration::from_millis(200),
            inner: StubHttpPost::succeeding(serde_json::json!({"ok": true})),
        };
        let (seams, ledger) = seams_with(Arc::new(delayed));
        let seams = Arc::new(seams);

        let report = seams.drain(&[harvest_record("artifact-1")], chrono::Utc::now());
        let delivered = report.delivered.expect("one item delivered");

        let verdict = ResponseVerdict::Accepted {
            gate_id: delivered.item_id.clone(),
            option_key: OPTION_APPROVE.to_string(),
            digest: delivered.payload.digest.clone(),
            decided_at: chrono::Utc::now(),
        };

        let seams_for_sink = Arc::clone(&seams);
        let sink = move |verdict: ResponseVerdict| {
            if let Some(engine_verdict) = approve_and_run_verdict_for(&verdict, "operator") {
                let seams = Arc::clone(&seams_for_sink);
                tokio::spawn(async move {
                    let _ = seams.resolve_verdict(engine_verdict).await;
                });
            }
        };

        let started = Instant::now();
        sink(verdict);
        let sink_call_took = started.elapsed();

        assert!(
            sink_call_took < Duration::from_millis(50),
            "sink call took {sink_call_took:?}, which suggests it awaited the \
             200ms-delayed resolution inline instead of spawning it"
        );
        // The spawned resolution has not necessarily run yet — nothing
        // ledger-side is asserted here. Only after actually waiting past
        // the delay does the resolution's effect become observable.
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(
            ledger.read_all().len(),
            1,
            "the spawned resolution should have completed by now"
        );
    }
}

// ── BA.21.A task 4: engine-side wiring evidence (D64) ───────────────────────
//
// The block record carries one acceptance criterion marked `gateable: false`:
// "An engine node's message arrives on Telegram from the Mini's `bastion
// serve`". No in-repo check can observe a real Telegram delivery — that is
// verified by hand on the Mini under `operator-mac-mini-visit` (the operator
// gate in `planning/blocks/BA.21.A.json`'s `depends_on`).
//
// What CAN be proven here, and is: the WIRING. `src/serve/mod.rs` task 2
// registers the same `Arc<dyn OperatorTransport>` that `NotifyPollLoop` holds
// as `app_data` on the engine-mount branch, exactly as `ledger_data` is
// registered one line away (the D15 additive-seam pattern) — so that when
// `engine-serve` eventually grows an extractor for it (`EN.12.J`'s
// `out_of_scope`: it shipped the abstraction only, no caller), resolving that
// `app_data` and calling `send` through the `dyn` trait object is exactly
// what will happen. This test stands in for that not-yet-built extractor: it
// builds a minimal actix app registering the transport as `app_data` the
// same way production does, resolves it inside a handler exactly as an
// engine-side caller would, and calls `send` through the `dyn` trait object.
//
// IMPORTANT — this proves WIRING, not DELIVERY. A green result here means an
// engine-side holder of the registered `app_data` can reach a real
// transport's `send` with a `ValidatedOperatorPayload`; it does NOT mean a
// Telegram message was ever sent over the network (`ScriptedTransport`
// never touches the network — see `telegram_http.rs`, which isolates the
// real HTTP surface). A reader must not mistake this test passing for a
// message having been delivered; that is the Mini hand-check's job.
#[actix_web::test]
async fn engine_side_app_data_resolves_transport_and_reaches_scripted_send() {
    let transport = Arc::new(ScriptedTransport::new(Vec::new()));
    // Register the SAME shape production uses in `src/serve/mod.rs`'s
    // engine-mount branch: `app.app_data(web::Data::new(transport))` where
    // `transport: Arc<dyn OperatorTransport>`.
    let dyn_transport: Arc<dyn OperatorTransport> = transport.clone();

    async fn engine_side_send_handler(
        transport: web::Data<Arc<dyn OperatorTransport>>,
    ) -> HttpResponse {
        let payload = scripted_payload("gate-wiring-1", "engine-side wiring probe");
        match transport.send(&payload).await {
            Ok(_delivered) => HttpResponse::Ok().finish(),
            Err(_) => HttpResponse::InternalServerError().finish(),
        }
    }

    let app = actix_web::test::init_service(
        App::new()
            .app_data(web::Data::new(dyn_transport))
            .route("/probe", web::post().to(engine_side_send_handler)),
    )
    .await;

    let req = actix_web::test::TestRequest::post()
        .uri("/probe")
        .to_request();
    let resp = actix_web::test::call_service(&app, req).await;
    assert!(
        resp.status().is_success(),
        "engine-side handler resolving the app_data transport should reach send successfully"
    );

    let recorded = transport.send_calls();
    assert_eq!(
        recorded.len(),
        1,
        "ScriptedTransport should have recorded exactly one send call from the engine-side handler"
    );
    assert_eq!(
        recorded[0].payload().gate_id,
        "gate-wiring-1",
        "the ValidatedOperatorPayload reaching send must be the one the engine-side caller built"
    );
}

// ── Stale-run alarm delivery loop (`BA.21.B` task 2) ────────────────────

/// A transport double whose `send` outcomes are scripted one per call, in
/// order — the same shape `ScriptedTransport::poll_outcomes` uses for
/// `poll_responses`, adapted to `send` — so a test can prove `deliver_once`
/// skips a failed item without panicking or aborting delivery of the rest
/// of the batch. `poll_responses`/`acknowledge` are never exercised by
/// these tests and always succeed trivially.
struct FailingSendTransport {
    send_outcomes:
        std::sync::Mutex<std::collections::VecDeque<Result<DeliveredMessage, NotifyError>>>,
    send_calls: std::sync::Mutex<Vec<ValidatedOperatorPayload>>,
}

impl FailingSendTransport {
    fn new(outcomes: Vec<Result<DeliveredMessage, NotifyError>>) -> Self {
        Self {
            send_outcomes: std::sync::Mutex::new(outcomes.into()),
            send_calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn send_calls(&self) -> Vec<ValidatedOperatorPayload> {
        self.send_calls
            .lock()
            .expect("send_calls mutex is never poisoned in these tests")
            .clone()
    }
}

#[async_trait]
impl OperatorTransport for FailingSendTransport {
    async fn send(
        &self,
        payload: &ValidatedOperatorPayload,
    ) -> Result<DeliveredMessage, NotifyError> {
        self.send_calls
            .lock()
            .expect("send_calls mutex is never poisoned in these tests")
            .push(payload.clone());
        self.send_outcomes
            .lock()
            .expect("send_outcomes mutex is never poisoned in these tests")
            .pop_front()
            .expect("test provided an outcome for every send call it drives")
    }

    async fn poll_responses(
        &self,
        since: Option<UpdateCursor>,
    ) -> Result<(Vec<OperatorResponse>, Option<UpdateCursor>), NotifyError> {
        Ok((Vec::new(), since))
    }
}

/// A minimal `TaskContext` with one node in `Running` status — the same
/// shape `engine_serve::orphan`'s own `running_context()` test helper
/// builds — sufficient for `alarm_stale_runs`/`sweep_stale_runs_once` to
/// treat a record as a live, non-terminal run.
fn running_task_context() -> engine_contract::task_context::TaskContext {
    let mut node_runs = std::collections::HashMap::new();
    node_runs.insert(
        "SomeNode".to_string(),
        engine_contract::task_context::NodeRun {
            status: engine_contract::task_context::NodeRunStatus::Running,
            started_at: Some(chrono::Utc::now()),
            completed_at: None,
            error: None,
            input: None,
            usage: None,
        },
    );
    engine_contract::task_context::TaskContext {
        event: serde_json::json!({}),
        nodes: std::collections::HashMap::new(),
        metadata: serde_json::json!({}),
        node_runs,
    }
}

fn default_limits() -> engine_core::operator::OperatorPayloadLimits {
    engine_core::operator::OperatorPayloadLimits::default()
}

#[tokio::test]
async fn deliver_once_sends_exactly_one_payload_naming_the_stalled_run_for_a_real_stale_record() {
    let live = engine_serve::live_state::LiveStateStore::new();
    let run_id = uuid::Uuid::new_v4();
    let now = chrono::Utc::now();
    live.record(run_id, &running_task_context());

    let far_future = now + chrono::Duration::hours(2);
    let policy = engine_core::operator::orphan::OrphanPolicy::default();
    let enqueued = engine_serve::orphan::sweep_stale_runs_once(&live, &policy, far_future);
    assert_eq!(enqueued, 1, "the run must have been alarmed and enqueued");

    let scripted = Arc::new(ScriptedTransport::new(Vec::new()));
    let transport: Arc<dyn OperatorTransport> = scripted.clone();
    let pending = Arc::new(PendingPayloads::new());
    let limits = default_limits();

    let delivered =
        stale_run_alarm::deliver_once(&live, &transport, &pending, &limits, far_future, 10).await;
    assert_eq!(delivered, 1, "exactly one item must be delivered");

    let sent = scripted.send_calls();
    assert_eq!(sent.len(), 1, "exactly one send call");
    assert!(
        sent[0]
            .payload()
            .rendered_summary
            .contains(&run_id.to_string()),
        "the delivered payload must name the stalled run id"
    );
}

#[tokio::test]
async fn a_second_delivery_tick_over_the_same_still_stale_run_sends_nothing_further() {
    let live = engine_serve::live_state::LiveStateStore::new();
    let run_id = uuid::Uuid::new_v4();
    let now = chrono::Utc::now();
    live.record(run_id, &running_task_context());

    let far_future = now + chrono::Duration::hours(2);
    let policy = engine_core::operator::orphan::OrphanPolicy::default();
    let first_enqueued = engine_serve::orphan::sweep_stale_runs_once(&live, &policy, far_future);
    assert_eq!(first_enqueued, 1);

    let scripted = Arc::new(ScriptedTransport::new(Vec::new()));
    let transport: Arc<dyn OperatorTransport> = scripted.clone();
    let pending = Arc::new(PendingPayloads::new());
    let limits = default_limits();

    let first =
        stale_run_alarm::deliver_once(&live, &transport, &pending, &limits, far_future, 10).await;
    assert_eq!(first, 1);

    // A later sweep tick over the same still-stale run enqueues nothing
    // further (`LiveStateStore::mark_alarmed` dedups on the engine side),
    // so a second delivery tick — well inside the default
    // `answer_timeout_secs` (900s) so the already-delivered item is not
    // released back to `pending` — has nothing new to drain either.
    let second_tick = far_future + chrono::Duration::seconds(5);
    let second_enqueued = engine_serve::orphan::sweep_stale_runs_once(&live, &policy, second_tick);
    assert_eq!(second_enqueued, 0, "the once-per-run dedup must hold");
    let second =
        stale_run_alarm::deliver_once(&live, &transport, &pending, &limits, second_tick, 10).await;
    assert_eq!(
        second, 0,
        "one stall must produce one message, not one per tick"
    );

    let sent = scripted.send_calls();
    assert_eq!(sent.len(), 1, "exactly one send call across both ticks");
    assert!(
        sent[0]
            .payload()
            .rendered_summary
            .contains(&run_id.to_string()),
        "the delivered payload must name the stalled run id"
    );
}

#[tokio::test]
async fn a_run_below_the_threshold_produces_no_delivery() {
    let live = engine_serve::live_state::LiveStateStore::new();
    let run_id = uuid::Uuid::new_v4();
    let now = chrono::Utc::now();
    live.record(run_id, &running_task_context());

    // Fresh run: `now` is the same instant as the record, far below any
    // policy threshold, so the engine-side sweep enqueues nothing.
    let policy = engine_core::operator::orphan::OrphanPolicy::default();
    let enqueued = engine_serve::orphan::sweep_stale_runs_once(&live, &policy, now);
    assert_eq!(enqueued, 0, "a fresh run must not be alarmed");

    let scripted = Arc::new(ScriptedTransport::new(Vec::new()));
    let transport: Arc<dyn OperatorTransport> = scripted.clone();
    let pending = Arc::new(PendingPayloads::new());
    let limits = default_limits();

    let delivered =
        stale_run_alarm::deliver_once(&live, &transport, &pending, &limits, now, 10).await;
    assert_eq!(delivered, 0);
    assert!(scripted.send_calls().is_empty());
}

#[tokio::test]
async fn a_failing_send_is_skipped_without_panicking_and_the_queue_keeps_working() {
    // The default `OperatorQueuePolicy` (`LiveStateStore::new()` always
    // constructs its queue under it — see `take_deliverable_batch_bounded_
    // by_queue_depth_not_only_max` in `stale_run_alarm.rs`) caps
    // `operator_queue_depth` at 1: only one item can be OPEN at a time, so
    // a single `deliver_once` batch here pops at most one item regardless
    // of `max`. This test proves the two halves of the criterion the depth
    // cap still lets it prove: (a) a failing `send` is skipped rather than
    // panicking or propagating, and (b) the queue is not left wedged by
    // that failure — once the failed item is answered, a later item is
    // still deliverable.
    let live = engine_serve::live_state::LiveStateStore::new();
    let run_a = uuid::Uuid::new_v4();
    let now = chrono::Utc::now();
    live.record(run_a, &running_task_context());

    let far_future = now + chrono::Duration::hours(2);
    let policy = engine_core::operator::orphan::OrphanPolicy::default();
    let enqueued = engine_serve::orphan::sweep_stale_runs_once(&live, &policy, far_future);
    assert_eq!(enqueued, 1, "run_a must have been alarmed and enqueued");

    let transport: Arc<dyn OperatorTransport> = Arc::new(FailingSendTransport::new(vec![
        Err(NotifyError::Transport {
            reason: "connect timeout".to_string(),
        }),
        Ok(DeliveredMessage {
            transport_message_id: "42".to_string(),
        }),
    ]));
    let pending = Arc::new(PendingPayloads::new());
    let limits = default_limits();

    let delivered =
        stale_run_alarm::deliver_once(&live, &transport, &pending, &limits, far_future, 10).await;
    assert_eq!(
        delivered, 0,
        "a failing send must not count as delivered, and must not panic"
    );

    // The failed item is still OPEN (never answered) — release it, the way
    // an eventual answer/timeout would, so the queue is not wedged.
    {
        let mut queue = live
            .operator_queue()
            .write()
            .expect("operator queue lock poisoned");
        assert_eq!(queue.open_count(), 1, "the failed item stays open");
        assert!(queue.answer(&format!("stale-run:{run_a}")));
    }

    // A second stale run alarms and enqueues normally, and delivers via the
    // scripted `Ok` outcome — proving the earlier failure did not abort or
    // permanently break delivery.
    let run_b = uuid::Uuid::new_v4();
    live.record(run_b, &running_task_context());
    let enqueued_b = engine_serve::orphan::sweep_stale_runs_once(
        &live,
        &policy,
        far_future + chrono::Duration::hours(1),
    );
    assert_eq!(enqueued_b, 1);

    let delivered_b = stale_run_alarm::deliver_once(
        &live,
        &transport,
        &pending,
        &limits,
        far_future + chrono::Duration::hours(1),
        10,
    )
    .await;
    assert_eq!(delivered_b, 1, "delivery keeps working after the failure");
}
