//! Unit tests for the transport seam (`BA.18.B` task 1).

use super::*;

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
) -> (
    VerdictSink,
    Arc<std::sync::Mutex<Vec<telegram::ResponseVerdict>>>,
) {
    let collected = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink_collected = collected.clone();
    let sink: VerdictSink = Box::new(move |verdict| {
        if let telegram::ResponseVerdict::Accepted { gate_id, .. } = &verdict {
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
    let transport = ScriptedTransport::new(vec![Ok((vec![resp], None))]);
    let (sink, collected) = removing_sink(Arc::clone(&registry));

    let mut poll_loop = NotifyPollLoop::new(Arc::new(transport), registry.lookup(), sink);
    poll_loop.tick().await.expect("tick succeeds");

    assert_eq!(
        collected.lock().unwrap().as_slice(),
        [telegram::ResponseVerdict::Accepted {
            gate_id: "gate-1".to_string(),
            option_key: "approve".to_string(),
        }]
    );
}

#[tokio::test]
async fn e2e_sent_then_mutated_then_tapped_resolves_stale_digest() {
    let registry = Arc::new(PendingPayloads::new());
    let shown = scripted_payload("gate-1", "original summary");
    let resp = response_for(&shown, "approve");

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
        [telegram::ResponseVerdict::StaleDigest]
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
        [telegram::ResponseVerdict::UnknownGate]
    );
}

#[tokio::test]
async fn e2e_replayed_tap_of_an_already_accepted_button_resolves_unknown_gate() {
    let registry = Arc::new(PendingPayloads::new());
    let payload = scripted_payload("gate-1", "diff summary");
    registry.insert(payload.clone());

    let resp = response_for(&payload, "approve");
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
            telegram::ResponseVerdict::Accepted {
                gate_id: "gate-1".to_string(),
                option_key: "approve".to_string(),
            },
            telegram::ResponseVerdict::UnknownGate,
        ]
    );
    assert!(
        registry.get("gate-1").is_none(),
        "accepted entry must have been removed from the registry"
    );
}
