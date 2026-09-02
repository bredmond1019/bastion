//! Task 1: pure parsing tests for `mev attention-queue --notify-only`
//! payloads, against the checked-in fixture captured 2026-09-01
//! (`planning/BA.21.D/attention-queue-notify-only.fixture.json`, 2 items
//! from a live 548-item board).

use super::*;

const FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/planning/BA.21.D/attention-queue-notify-only.fixture.json"
));

#[test]
fn fixture_parses_to_exactly_two_items() {
    let items = parse_attention_queue_payload(FIXTURE).expect("fixture should parse");
    assert_eq!(items.len(), 2);
}

#[test]
fn fixture_first_item_fields_match_the_captured_payload() {
    let items = parse_attention_queue_payload(FIXTURE).expect("fixture should parse");
    let first = &items[0];

    assert_eq!(
        first.item_id,
        "14e25af6d375d891b8ae1cfd0faa53571b8fa5e9bb9e38494c7274cab15965b3"
    );
    assert_eq!(
        first.gate_id,
        "attention:14e25af6d375d891b8ae1cfd0faa53571b8fa5e9bb9e38494c7274cab15965b3"
    );
    assert!(first.rendered_summary.starts_with(
        "[engine-rs] HOT kind=defect slug=budget-max-cost-usd-cannot-fire-on-any-sdlc-run"
    ));
    assert_eq!(
        first.digest,
        "d298430f49717b35c37979c3c8e4db78d113f7c2f0b0d342d11e451ba214655b"
    );
    assert_eq!(first.effective_priority, 0);
    assert_eq!(first.lane, "hot");
    assert_eq!(first.repo, "engine-rs");
    assert_eq!(first.source, "attention-board");

    assert_eq!(first.options.len(), 3);
    assert_eq!(first.options[0].key, "promote");
    assert_eq!(first.options[0].label, "Promote");
    assert_eq!(first.options[1].key, "snooze");
    assert_eq!(first.options[1].label, "Snooze");
    assert_eq!(first.options[2].key, "session");
    assert_eq!(first.options[2].label, "Open session");
}

#[test]
fn fixture_second_item_fields_match_the_captured_payload() {
    let items = parse_attention_queue_payload(FIXTURE).expect("fixture should parse");
    let second = &items[1];

    assert_eq!(
        second.item_id,
        "ec7b3433bbc43d24d4c4cbcdeda73a3b2e2321b003422d0de1b71d1174cb2358"
    );
    assert_eq!(
        second.gate_id,
        "attention:ec7b3433bbc43d24d4c4cbcdeda73a3b2e2321b003422d0de1b71d1174cb2358"
    );
    assert!(
        second
            .rendered_summary
            .starts_with("[bastion-web] BLOCKING kind=deferred")
    );
    assert_eq!(
        second.digest,
        "8af47ccea144b5352a8b434dc4aaff3de5be68922efc1a7cf2f69afb5184df38"
    );
    assert_eq!(second.effective_priority, 3);
    assert_eq!(second.lane, "blocking");
    assert_eq!(second.repo, "bastion-web");
    assert_eq!(second.source, "attention-board");
    assert_eq!(second.options.len(), 3);
}

#[test]
fn round_trip_serializes_back_to_an_equivalent_value() {
    let items = parse_attention_queue_payload(FIXTURE).expect("fixture should parse");
    let json = serde_json::to_string(&items).expect("serialize should succeed");
    let reparsed = parse_attention_queue_payload(&json).expect("re-parse should succeed");
    assert_eq!(items, reparsed);
}

#[test]
fn empty_array_parses_to_an_empty_vec_and_is_not_an_error() {
    // `mev attention-queue --notify-only` prints `[]` and exits 0 when the
    // admitted set is empty — the healthy, common case. Treating it as a
    // failure would alarm on exactly the case this source should stay quiet
    // for.
    let items = parse_attention_queue_payload("[]").expect("empty array is not an error");
    assert!(items.is_empty());
}

#[test]
fn whitespace_only_empty_array_also_parses_cleanly() {
    let items = parse_attention_queue_payload("  [ ]\n").expect("should parse");
    assert!(items.is_empty());
}

#[test]
fn malformed_json_returns_a_typed_error_not_a_panic() {
    let result = parse_attention_queue_payload("not json at all");
    match result {
        Err(ParseError::Malformed(_)) => {}
        Ok(items) => panic!("expected a parse error, got Ok({items:?})"),
    }
}

#[test]
fn missing_required_field_returns_a_typed_error() {
    let json = r#"[{"item_id": "a", "gate_id": "g"}]"#;
    let result = parse_attention_queue_payload(json);
    assert!(matches!(result, Err(ParseError::Malformed(_))));
}

#[test]
fn wrong_shape_top_level_value_returns_a_typed_error() {
    // A single object, not wrapped in an array, is not the shape `mev`
    // emits — must be a typed error, never a panic or a silent single-item
    // vec.
    let json = r#"{"item_id": "a"}"#;
    let result = parse_attention_queue_payload(json);
    assert!(matches!(result, Err(ParseError::Malformed(_))));
}

#[test]
fn parse_error_display_names_the_payload_as_malformed() {
    let err = parse_attention_queue_payload("{{{").unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("malformed attention-queue payload"),
        "unexpected error message: {message}"
    );
}

// ── Task 3: the I/O shell — deliver(), digest rendering, run() ─────────────

mod shell {
    use super::*;
    use async_trait::async_trait;
    use engine_core::operator::{
        DeliveredMessage, NotifyError, OperatorResponse, OperatorResponseOption, UpdateCursor,
        ValidatedOperatorPayload,
    };
    use poller::AttentionFetch;
    use std::sync::Mutex;

    fn item(id: &str, priority: i64) -> AttentionQueueItem {
        AttentionQueueItem {
            item_id: id.to_string(),
            gate_id: format!("attention:{id}"),
            rendered_summary: format!("[repo] item {id}"),
            options: vec![
                OperatorResponseOption::new("promote", "Promote"),
                OperatorResponseOption::new("snooze", "Snooze"),
            ],
            digest: format!("digest-{id}"),
            effective_priority: priority,
            lane: "hot".to_string(),
            repo: "repo".to_string(),
            source: "attention-board".to_string(),
        }
    }

    /// A transport double recording every `send` call, always succeeding.
    /// `poll_responses` is never exercised by these tests.
    #[derive(Default)]
    struct RecordingTransport {
        sent: Mutex<Vec<ValidatedOperatorPayload>>,
    }

    impl RecordingTransport {
        fn sent_gate_ids(&self) -> Vec<String> {
            self.sent
                .lock()
                .expect("sent mutex is never poisoned in these tests")
                .iter()
                .map(|p| p.payload().gate_id.clone())
                .collect()
        }
    }

    #[async_trait]
    impl OperatorTransport for RecordingTransport {
        async fn send(
            &self,
            payload: &ValidatedOperatorPayload,
        ) -> Result<DeliveredMessage, NotifyError> {
            self.sent
                .lock()
                .expect("sent mutex is never poisoned in these tests")
                .push(payload.clone());
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

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    #[tokio::test]
    async fn deliver_sends_admitted_items_and_registers_them_pending() {
        let transport = Arc::new(RecordingTransport::default());
        let pending = Arc::new(PendingPayloads::new());
        let mut poller = AttentionSourcePoller::with_fetch(
            transport.clone(),
            pending.clone(),
            OperatorQueuePolicy {
                operator_queue_depth: 5,
                ..OperatorQueuePolicy::default()
            },
            Arc::new(|| AttentionFetch::MevMissing),
        );

        let fetch = AttentionFetch::Items(vec![item("a", 0)]);
        let delivered = poller.deliver(fetch, now()).await;

        assert_eq!(delivered, 1);
        assert_eq!(transport.sent_gate_ids(), vec!["attention:a".to_string()]);
        assert!(pending.get("attention:a").is_some());
    }

    #[tokio::test]
    async fn deliver_never_re_sends_an_item_already_delivered() {
        let transport = Arc::new(RecordingTransport::default());
        let pending = Arc::new(PendingPayloads::new());
        let mut poller = AttentionSourcePoller::with_fetch(
            transport.clone(),
            pending,
            OperatorQueuePolicy::default(),
            Arc::new(|| AttentionFetch::MevMissing),
        );

        let fetch = AttentionFetch::Items(vec![item("a", 0)]);
        poller.deliver(fetch.clone(), now()).await;
        poller.deliver(fetch, now()).await;

        assert_eq!(
            transport.sent_gate_ids().len(),
            1,
            "second tick must not re-send an already-delivered item"
        );
    }

    #[tokio::test]
    async fn deliver_sends_a_digest_for_the_remainder_beyond_depth() {
        let transport = Arc::new(RecordingTransport::default());
        let pending = Arc::new(PendingPayloads::new());
        let mut poller = AttentionSourcePoller::with_fetch(
            transport.clone(),
            pending,
            OperatorQueuePolicy {
                operator_queue_depth: 1,
                ..OperatorQueuePolicy::default()
            },
            Arc::new(|| AttentionFetch::MevMissing),
        );

        let fetch = AttentionFetch::Items(vec![item("a", 0), item("b", 1), item("c", 2)]);
        let delivered = poller.deliver(fetch, now()).await;

        // One individual item delivered (depth == 1), plus one digest
        // message for the remainder — never N independent sends.
        assert_eq!(delivered, 1);
        assert_eq!(transport.sent_gate_ids().len(), 2);
        assert!(
            transport
                .sent_gate_ids()
                .iter()
                .any(|g| g.starts_with("attention-digest:")),
            "expected one digest-gated message among the sends: {:?}",
            transport.sent_gate_ids()
        );
    }

    #[tokio::test]
    async fn deliver_sends_nothing_on_a_failed_fetch() {
        let transport = Arc::new(RecordingTransport::default());
        let pending = Arc::new(PendingPayloads::new());
        let mut poller = AttentionSourcePoller::with_fetch(
            transport.clone(),
            pending,
            OperatorQueuePolicy::default(),
            Arc::new(|| AttentionFetch::MevMissing),
        );

        let delivered = poller.deliver(AttentionFetch::MevMissing, now()).await;

        assert_eq!(delivered, 0);
        assert!(transport.sent_gate_ids().is_empty());
    }

    #[tokio::test]
    async fn run_delivers_on_its_first_tick_from_the_injected_fetch() {
        let transport = Arc::new(RecordingTransport::default());
        let pending = Arc::new(PendingPayloads::new());
        let poller = AttentionSourcePoller::with_fetch(
            transport.clone(),
            pending,
            OperatorQueuePolicy::default(),
            Arc::new(|| AttentionFetch::Items(vec![item("a", 0)])),
        );

        // `run` never returns under normal operation — race it against a
        // short timeout and assert the first tick already delivered.
        let _ = tokio::time::timeout(Duration::from_millis(50), poller.run(1)).await;

        assert_eq!(transport.sent_gate_ids(), vec!["attention:a".to_string()]);
    }
}

// ── Read-only guarantee (D25): no write anywhere in this source ───────────

#[test]
fn attention_source_module_never_opens_a_file_for_writing() {
    // Static source-text assertion, mirroring `src/serve/mod.rs`'s own
    // `include_str!`-based needle tests: `mev attention-queue --notify-only`
    // is spawned to READ its stdout only. Every write-shaped std::fs/
    // OpenOptions call is a needle this module's source must never contain.
    let mod_source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/serve/attention_source/mod.rs"
    ));
    let poller_source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/serve/attention_source/poller.rs"
    ));

    for needle in [
        "std::fs::write",
        "std::fs::File::create",
        "OpenOptions",
        ".write(true)",
    ] {
        assert!(
            !mod_source.contains(needle),
            "attention_source/mod.rs must never write a file (D25 read-only guarantee) \
             — found forbidden needle {needle:?}"
        );
        assert!(
            !poller_source.contains(needle),
            "attention_source/poller.rs must never write a file (D25 read-only guarantee) \
             — found forbidden needle {needle:?}"
        );
    }
}
