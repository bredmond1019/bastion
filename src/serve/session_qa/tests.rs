//! Tests for the session-QA bridge (`BA.20.C`).
//!
//! Two halves:
//! - The pure-core tests (task 4) below, unmodified in substance from the
//!   inline `mod tests` this file was extracted from — registry dedup,
//!   callback encode/decode, verdict resolution, body builders, and the
//!   escape-hatch follow-up state machine. No I/O anywhere in this half.
//! - The end-to-end hermetic tests (task 7), further down, that drive the
//!   whole bridge — inbound crossing-to-message and outbound tap-to-injection
//!   — through an injected fake Telegram client and injected capture/inject
//!   closures. No network call, no tmux process, anywhere in this file.
use super::*;
use crate::sessions::ask_question::{OptionKind, QuestionOption};

fn prompt(question: &str, options: &[(usize, &str, OptionKind)]) -> AskQuestionPrompt {
    AskQuestionPrompt {
        header: None,
        question: question.to_string(),
        options: options
            .iter()
            .map(|(number, label, kind)| QuestionOption {
                number: *number,
                label: (*label).to_string(),
                description: None,
                kind: *kind,
            })
            .collect(),
    }
}

fn sample_prompt() -> AskQuestionPrompt {
    prompt(
        "Which database?",
        &[
            (1, "Postgres", OptionKind::Choice),
            (2, "MySQL", OptionKind::Choice),
            (3, "Chat about this", OptionKind::ChatAbout),
        ],
    )
}

// ── PendingQuestions ─────────────────────────────────────────────────

#[test]
fn register_new_question_creates_entry() {
    let registry = PendingQuestions::new();
    let outcome = registry.register("session-a", sample_prompt(), Utc::now());
    assert!(matches!(outcome, RegisterOutcome::Created { .. }));
    assert_eq!(registry.len(), 1);
}

#[test]
fn register_identical_unanswered_question_dedups_to_existing_id() {
    let registry = PendingQuestions::new();
    let first = registry.register("session-a", sample_prompt(), Utc::now());
    let second = registry.register("session-a", sample_prompt(), Utc::now());

    assert_eq!(first.question_id(), second.question_id());
    assert!(matches!(second, RegisterOutcome::AlreadyPending { .. }));
    assert_eq!(registry.len(), 1, "dedup must not create a second entry");
}

#[test]
fn register_different_prompt_for_same_session_creates_second_entry() {
    let registry = PendingQuestions::new();
    let first = registry.register("session-a", sample_prompt(), Utc::now());
    let other = prompt("Which cache?", &[(1, "Redis", OptionKind::Choice)]);
    let second = registry.register("session-a", other, Utc::now());

    assert_ne!(first.question_id(), second.question_id());
    assert_eq!(registry.len(), 2);
}

#[test]
fn register_after_answering_creates_new_entry_not_dedup() {
    let registry = PendingQuestions::new();
    let first = registry.register("session-a", sample_prompt(), Utc::now());
    registry.mark_answered(first.question_id());

    let second = registry.register("session-a", sample_prompt(), Utc::now());
    assert_ne!(
        first.question_id(),
        second.question_id(),
        "an answered question must not dedup a fresh identical crossing"
    );
}

#[test]
fn register_different_session_same_prompt_creates_second_entry() {
    let registry = PendingQuestions::new();
    let first = registry.register("session-a", sample_prompt(), Utc::now());
    let second = registry.register("session-b", sample_prompt(), Utc::now());
    assert_ne!(first.question_id(), second.question_id());
}

#[test]
fn set_message_id_updates_the_entry() {
    let registry = PendingQuestions::new();
    let outcome = registry.register("session-a", sample_prompt(), Utc::now());
    registry.set_message_id(outcome.question_id(), 42);
    let q = registry.get(outcome.question_id()).expect("entry exists");
    assert_eq!(q.message_id, Some(42));
}

#[test]
fn registry_is_fifo_bounded_and_evicts_oldest() {
    let registry = PendingQuestions::new();
    for i in 0..PendingQuestions::CAPACITY {
        registry.register(format!("session-{i}"), sample_prompt(), Utc::now());
    }
    assert_eq!(registry.len(), PendingQuestions::CAPACITY);

    let first_id = format!("q{}", 0);
    assert!(registry.get(&first_id).is_some());

    // One more insert past capacity evicts the oldest rather than growing.
    registry.register("session-overflow", sample_prompt(), Utc::now());
    assert_eq!(
        registry.len(),
        PendingQuestions::CAPACITY,
        "bound-exceeding insert must evict, not grow"
    );
    assert!(
        registry.get(&first_id).is_none(),
        "the oldest entry must have been evicted"
    );
}

#[test]
fn get_unknown_id_returns_none() {
    let registry = PendingQuestions::new();
    assert_eq!(registry.get("does-not-exist"), None);
}

// ── callback encode/decode ───────────────────────────────────────────

#[test]
fn encode_decode_round_trips() {
    let encoded = encode_question_callback("q42", 3);
    let decoded = decode_question_callback(&encoded).expect("should decode");
    assert_eq!(decoded.question_id, "q42");
    assert_eq!(decoded.option_number, 3);
}

#[test]
fn encoded_callback_data_realistic_worst_case_under_limit() {
    // Worst case: registry has run long enough for a large sequence
    // number, and the option number itself is still small (options are
    // never more than a handful in practice, but use a generous value).
    let worst_case_id = format!("q{}", u64::MAX);
    let encoded = encode_question_callback(&worst_case_id, 99);
    assert!(
        encoded.len() <= QA_CALLBACK_DATA_MAX_BYTES,
        "encoded callback data ({encoded}, {} bytes) exceeds Telegram's {QA_CALLBACK_DATA_MAX_BYTES}-byte ceiling",
        encoded.len()
    );
}

#[test]
fn decode_rejects_missing_delimiter() {
    assert_eq!(decode_question_callback("q42"), None);
}

#[test]
fn decode_rejects_non_numeric_option() {
    assert_eq!(decode_question_callback("q42|abc"), None);
}

#[test]
fn decode_rejects_empty_question_id() {
    assert_eq!(decode_question_callback("|3"), None);
}

#[test]
fn decode_rejects_empty_string() {
    assert_eq!(decode_question_callback(""), None);
}

// ── resolve_question_response ────────────────────────────────────────

#[test]
fn resolve_accepted_for_valid_option_tap() {
    let registry = PendingQuestions::new();
    let outcome = registry.register("session-a", sample_prompt(), Utc::now());
    let callback = QuestionCallback {
        question_id: outcome.question_id().to_string(),
        option_number: 2,
    };
    let verdict = resolve_question_response(&callback, &registry);
    assert_eq!(
        verdict,
        QuestionVerdict::Accepted {
            session: "session-a".to_string(),
            option_number: 2,
            digit: "2".to_string(),
            kind: OptionKind::Choice,
        }
    );
}

#[test]
fn resolve_accepted_flags_chat_about_option() {
    let registry = PendingQuestions::new();
    let outcome = registry.register("session-a", sample_prompt(), Utc::now());
    let callback = QuestionCallback {
        question_id: outcome.question_id().to_string(),
        option_number: 3,
    };
    let verdict = resolve_question_response(&callback, &registry);
    match verdict {
        QuestionVerdict::Accepted { kind, .. } => assert_eq!(kind, OptionKind::ChatAbout),
        other => panic!("expected Accepted, got {other:?}"),
    }
}

#[test]
fn resolve_already_answered_after_mark() {
    let registry = PendingQuestions::new();
    let outcome = registry.register("session-a", sample_prompt(), Utc::now());
    registry.mark_answered(outcome.question_id());

    let callback = QuestionCallback {
        question_id: outcome.question_id().to_string(),
        option_number: 1,
    };
    let verdict = resolve_question_response(&callback, &registry);
    assert_eq!(verdict, QuestionVerdict::AlreadyAnswered);
}

#[test]
fn resolve_unknown_question_for_unregistered_id() {
    let registry = PendingQuestions::new();
    let callback = QuestionCallback {
        question_id: "q-not-registered".to_string(),
        option_number: 1,
    };
    let verdict = resolve_question_response(&callback, &registry);
    assert_eq!(verdict, QuestionVerdict::UnknownQuestion);
}

#[test]
fn resolve_unknown_question_for_out_of_range_option_number() {
    let registry = PendingQuestions::new();
    let outcome = registry.register("session-a", sample_prompt(), Utc::now());
    let callback = QuestionCallback {
        question_id: outcome.question_id().to_string(),
        option_number: 99,
    };
    let verdict = resolve_question_response(&callback, &registry);
    assert_eq!(verdict, QuestionVerdict::UnknownQuestion);
}

// ── body builders ─────────────────────────────────────────────────────

#[test]
fn sendmessage_body_has_one_button_per_option() {
    let body = sendmessage_body("q1", &sample_prompt(), "12345");
    let buttons = body["reply_markup"]["inline_keyboard"][0]
        .as_array()
        .expect("buttons array");
    assert_eq!(buttons.len(), 3);
}

#[test]
fn sendmessage_body_carries_question_text_and_chat_id() {
    let body = sendmessage_body("q1", &sample_prompt(), "12345");
    assert_eq!(body["chat_id"], "12345");
    assert_eq!(body["text"], "Which database?");
}

#[test]
fn sendmessage_body_escape_hatch_button_is_visually_distinguished() {
    let body = sendmessage_body("q1", &sample_prompt(), "12345");
    let buttons = body["reply_markup"]["inline_keyboard"][0]
        .as_array()
        .expect("buttons array");
    let escape_text = buttons[2]["text"].as_str().expect("text");
    let normal_text = buttons[0]["text"].as_str().expect("text");
    assert_ne!(
        escape_text.chars().next(),
        normal_text.chars().next(),
        "escape hatch button must read differently from a normal option button"
    );
}

#[test]
fn sendmessage_body_button_callback_data_round_trips_to_correct_option() {
    let body = sendmessage_body("q1", &sample_prompt(), "12345");
    let buttons = body["reply_markup"]["inline_keyboard"][0]
        .as_array()
        .expect("buttons array");
    let data = buttons[1]["callback_data"].as_str().expect("callback_data");
    let decoded = decode_question_callback(data).expect("should decode");
    assert_eq!(decoded.question_id, "q1");
    assert_eq!(decoded.option_number, 2);
}

#[test]
fn answercallbackquery_body_distinct_text_per_verdict() {
    let accepted = answercallbackquery_body(
        "cb1",
        &QuestionVerdict::Accepted {
            session: "s".to_string(),
            option_number: 1,
            digit: "1".to_string(),
            kind: OptionKind::Choice,
        },
    );
    let already = answercallbackquery_body("cb1", &QuestionVerdict::AlreadyAnswered);
    let unknown = answercallbackquery_body("cb1", &QuestionVerdict::UnknownQuestion);

    let a = accepted["text"].as_str().unwrap();
    let b = already["text"].as_str().unwrap();
    let c = unknown["text"].as_str().unwrap();
    assert_ne!(a, b);
    assert_ne!(a, c);
    assert_ne!(b, c);
}

#[test]
fn answercallbackquery_body_carries_callback_query_id() {
    let body = answercallbackquery_body("cb-xyz", &QuestionVerdict::AlreadyAnswered);
    assert_eq!(body["callback_query_id"], "cb-xyz");
}

#[test]
fn editmessagetext_body_clears_keyboard_and_shows_choice() {
    let body = editmessagetext_body("chat1", 99, "Which database?", "Postgres");
    assert_eq!(body["chat_id"], "chat1");
    assert_eq!(body["message_id"], 99);
    assert_eq!(
        body["reply_markup"]["inline_keyboard"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let text = body["text"].as_str().unwrap();
    assert!(text.contains("Postgres"));
}

// ── ChatFollowUpState ─────────────────────────────────────────────────

#[test]
fn idle_escape_hatch_tap_transitions_to_awaiting() {
    let state = ChatFollowUpState::Idle;
    let (new_state, outcome) = state.on_escape_hatch_tap("q1", OptionKind::FreeText);
    assert_eq!(
        new_state,
        ChatFollowUpState::AwaitingFreeText {
            question_id: "q1".to_string(),
            kind: OptionKind::FreeText,
        }
    );
    assert_eq!(
        outcome,
        FollowUpOutcome::NowAwaiting {
            question_id: "q1".to_string(),
            kind: OptionKind::FreeText,
        }
    );
}

#[test]
fn second_escape_hatch_tap_while_awaiting_replaces_with_new_question() {
    let state = ChatFollowUpState::AwaitingFreeText {
        question_id: "q1".to_string(),
        kind: OptionKind::FreeText,
    };
    let (new_state, outcome) = state.on_escape_hatch_tap("q2", OptionKind::ChatAbout);
    assert_eq!(
        new_state,
        ChatFollowUpState::AwaitingFreeText {
            question_id: "q2".to_string(),
            kind: OptionKind::ChatAbout,
        }
    );
    assert_eq!(
        outcome,
        FollowUpOutcome::ReplacedAwaiting {
            old_question_id: "q1".to_string(),
            new_question_id: "q2".to_string(),
            new_kind: OptionKind::ChatAbout,
        }
    );
}

#[test]
fn plain_text_while_awaiting_relays_and_returns_to_idle() {
    let state = ChatFollowUpState::AwaitingFreeText {
        question_id: "q1".to_string(),
        kind: OptionKind::FreeText,
    };
    let (new_state, outcome) = state.on_plain_text("use postgres please");
    assert_eq!(new_state, ChatFollowUpState::Idle);
    assert_eq!(
        outcome,
        FollowUpOutcome::RelayText {
            question_id: "q1".to_string(),
            text: "use postgres please".to_string(),
            kind: OptionKind::FreeText,
        }
    );
}

#[test]
fn plain_text_while_idle_is_ignored_and_stays_idle() {
    let state = ChatFollowUpState::Idle;
    let (new_state, outcome) = state.on_plain_text("unrelated message");
    assert_eq!(new_state, ChatFollowUpState::Idle);
    assert_eq!(outcome, FollowUpOutcome::Ignored);
}

#[test]
fn default_state_is_idle() {
    assert_eq!(ChatFollowUpState::default(), ChatFollowUpState::Idle);
}

// ── End-to-end hermetic coverage (BA.20.C task 7) ───────────────────────────
//
// Everything below drives the whole bridge — inbound crossing-to-message and
// outbound tap-to-injection — through an injected fake `QaTelegramClient` and
// injected `CapturePaneFn`/`InjectFn` closures, exactly the seams task 5
// built for this purpose. No test in this section makes a real network call
// or spawns a real tmux process: `FakeQaTelegramClient` never touches
// `reqwest`, and every capture/inject closure is a plain in-memory closure.
//
// Live delivery (a real Telegram message actually arriving, a real tap
// reaching tmux) is **not** verified here — see this spec's `## Notes` for
// the explicit declaration of what this hermetic suite does and does not
// prove.

mod e2e {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex as StdMutex};

    use async_trait::async_trait;
    use chrono::Utc;
    use serde_json::json;

    use super::super::*;
    use crate::config::BotToken;
    use crate::detect::AgentState;
    use crate::serve::blocked_edge::sink::BlockedEdgeRecord;

    /// A pane that parses to a genuine `AskUserQuestion` prompt: a question, two plain
    /// choices, and the same rule-bounded `FreeText`/`ChatAbout` pair as the real
    /// captured widget — options 3 and 4 sit around the separating horizontal rule.
    const SAMPLE_PANE: &str = "Which database?\n1. Postgres\n2. MySQL\n3. Type something.\n────────────\n4. Chat about this\nEnter to select\n";

    /// A pane with no `AskUserQuestion` footer marker at all — e.g. a plain
    /// permission dialog. Must parse to `None`.
    const NON_QUESTION_PANE: &str = "Allow this action? (y/n)\n";

    // ── Fake Telegram client ────────────────────────────────────────────

    /// One recorded call to [`FakeQaTelegramClient`], in call order.
    #[derive(Debug, Clone)]
    enum Call {
        SendMessage(serde_json::Value),
        AnswerCallbackQuery(serde_json::Value),
        EditMessageText(serde_json::Value),
        GetUpdates,
    }

    /// In-memory [`QaTelegramClient`]: records every call, and answers from a
    /// per-method queue of canned results (falling back to a benign default
    /// once a queue is exhausted). No network anywhere in this type.
    #[derive(Default)]
    struct FakeQaTelegramClient {
        calls: StdMutex<Vec<Call>>,
        send_message_queue: StdMutex<VecDeque<Result<serde_json::Value, NotifyError>>>,
        answer_queue: StdMutex<VecDeque<Result<(), NotifyError>>>,
        edit_queue: StdMutex<VecDeque<Result<(), NotifyError>>>,
        get_updates_queue: StdMutex<VecDeque<Result<serde_json::Value, NotifyError>>>,
    }

    impl FakeQaTelegramClient {
        fn new() -> Self {
            Self::default()
        }

        fn calls(&self) -> Vec<Call> {
            self.calls.lock().expect("calls mutex poisoned").clone()
        }

        fn calls_matching<F: Fn(&Call) -> bool>(&self, pred: F) -> usize {
            self.calls().iter().filter(|c| pred(c)).count()
        }

        fn push_get_updates(&self, result: Result<serde_json::Value, NotifyError>) {
            self.get_updates_queue
                .lock()
                .expect("queue mutex poisoned")
                .push_back(result);
        }
    }

    #[async_trait]
    impl QaTelegramClient for FakeQaTelegramClient {
        async fn send_message(
            &self,
            body: serde_json::Value,
        ) -> Result<serde_json::Value, NotifyError> {
            self.calls
                .lock()
                .expect("calls mutex poisoned")
                .push(Call::SendMessage(body));
            self.send_message_queue
                .lock()
                .expect("queue mutex poisoned")
                .pop_front()
                .unwrap_or_else(|| Ok(json!({"ok": true, "result": {"message_id": 555}})))
        }

        async fn answer_callback_query(&self, body: serde_json::Value) -> Result<(), NotifyError> {
            self.calls
                .lock()
                .expect("calls mutex poisoned")
                .push(Call::AnswerCallbackQuery(body));
            self.answer_queue
                .lock()
                .expect("queue mutex poisoned")
                .pop_front()
                .unwrap_or(Ok(()))
        }

        async fn edit_message_text(&self, body: serde_json::Value) -> Result<(), NotifyError> {
            self.calls
                .lock()
                .expect("calls mutex poisoned")
                .push(Call::EditMessageText(body));
            self.edit_queue
                .lock()
                .expect("queue mutex poisoned")
                .pop_front()
                .unwrap_or(Ok(()))
        }

        async fn get_updates(
            &self,
            _query: &[(String, String)],
        ) -> Result<serde_json::Value, NotifyError> {
            // A real `HttpQaTelegramClient::get_updates` always crosses a
            // real await point (the network call), which cooperatively
            // yields back to the runtime. This fake has no such await
            // point once its canned-result queue is exhausted, so an
            // unbroken success stream would let `run_outbound`'s loop spin
            // without ever yielding — starving any `tokio::time::timeout`
            // wrapped around it. `yield_now` restores that same
            // cooperative-scheduling property for tests that bound
            // `run_outbound` with a timeout.
            tokio::task::yield_now().await;
            self.calls
                .lock()
                .expect("calls mutex poisoned")
                .push(Call::GetUpdates);
            self.get_updates_queue
                .lock()
                .expect("queue mutex poisoned")
                .pop_front()
                .unwrap_or_else(|| Ok(json!({"ok": true, "result": []})))
        }
    }

    // ── Capture / inject seam helpers ───────────────────────────────────

    fn capture_returning(pane: &'static str) -> CapturePaneFn {
        Box::new(move |_session| Ok(pane.to_string()))
    }

    /// Captures `fail_for` with an error; every other session gets `pane`.
    fn capture_failing_for(fail_for: &'static str, pane: &'static str) -> CapturePaneFn {
        Box::new(move |session| {
            if session == fail_for {
                Err(anyhow::anyhow!("tmux capture-pane failed: no server"))
            } else {
                Ok(pane.to_string())
            }
        })
    }

    fn inject_ok() -> InjectFn {
        Box::new(|_session, _text, _with_enter| Ok(()))
    }

    fn inject_always_err() -> InjectFn {
        Box::new(|_session, _text, _with_enter| {
            Err(anyhow::anyhow!("tmux send-keys failed: no server"))
        })
    }

    /// An [`InjectFn`] that records every `(session, text)` it was called
    /// with, for tests that assert on exactly what got injected.
    ///
    /// The `with_enter` flag is accepted (the seam now carries it) but not
    /// logged here — this fake preserves the pre-task-1 two-tuple log shape
    /// so every existing assertion in this file keeps passing unchanged.
    /// Task 3 adds an ordered `(session, text, with_enter)` fake for the
    /// sequence-level assertions.
    fn inject_recording() -> (InjectFn, Arc<StdMutex<Vec<(String, String)>>>) {
        let log = Arc::new(StdMutex::new(Vec::new()));
        let log_for_closure = log.clone();
        let f: InjectFn = Box::new(move |session, text, _with_enter| {
            log_for_closure
                .lock()
                .expect("inject log mutex poisoned")
                .push((session.to_string(), text.to_string()));
            Ok(())
        });
        (f, log)
    }

    /// An [`InjectFn`] that records an ORDERED log of `(session, text,
    /// with_enter)` tuples — the full keystroke-sequence fake `ticket-
    /// session-qa-freetext-injection`'s task 3 requires. Every scenario in
    /// that ticket asserts on this full ordered sequence, never on call
    /// counts or a two-tuple log alone: a call-count assertion is exactly
    /// what let the shipped `BA.20.C` defect (option 1 silently submitted
    /// under a `FreeText`/`ChatAbout` tap) through review.
    fn inject_ordered_recording() -> (InjectFn, Arc<StdMutex<Vec<(String, String, bool)>>>) {
        let log = Arc::new(StdMutex::new(Vec::new()));
        let log_for_closure = log.clone();
        let f: InjectFn = Box::new(move |session, text, with_enter| {
            log_for_closure
                .lock()
                .expect("ordered inject log mutex poisoned")
                .push((session.to_string(), text.to_string(), with_enter));
            Ok(())
        });
        (f, log)
    }

    fn edge_record(session: &str) -> BlockedEdgeRecord {
        BlockedEdgeRecord::new(session, "host-a", None, AgentState::Blocked, Utc::now())
    }

    /// Pull the `question_id` a `sendMessage` body's first button's
    /// `callback_data` encodes — the id the bridge generated for that
    /// crossing, needed to build a realistic callback-query update.
    fn question_id_from_send_message(body: &serde_json::Value) -> String {
        let data = body["reply_markup"]["inline_keyboard"][0][0]["callback_data"]
            .as_str()
            .expect("first button must carry callback_data");
        decode_question_callback(data)
            .expect("callback_data must decode")
            .question_id
    }

    fn callback_query_update(
        update_id: i64,
        callback_query_id: &str,
        question_id: &str,
        option_number: usize,
        chat_id: i64,
        message_id: i64,
    ) -> serde_json::Value {
        json!({
            "update_id": update_id,
            "callback_query": {
                "id": callback_query_id,
                "data": encode_question_callback(question_id, option_number),
                "message": {
                    "message_id": message_id,
                    "chat": {"id": chat_id},
                },
            },
        })
    }

    fn text_message_update(update_id: i64, chat_id: i64, text: &str) -> serde_json::Value {
        json!({
            "update_id": update_id,
            "message": {
                "chat": {"id": chat_id},
                "text": text,
            },
        })
    }

    // ── Inbound: crossing → message ─────────────────────────────────────

    #[tokio::test]
    async fn crossing_with_question_sends_exactly_one_message_with_right_button_count() {
        let client = Arc::new(FakeQaTelegramClient::new());
        let bridge = SessionQaBridge::with_seams(
            "chat-1".to_string(),
            client.clone(),
            capture_returning(SAMPLE_PANE),
            inject_ok(),
        );

        bridge.handle_edge_record(edge_record("sess-1")).await;

        let calls = client.calls();
        let sends: Vec<_> = calls
            .iter()
            .filter_map(|c| match c {
                Call::SendMessage(body) => Some(body),
                _ => None,
            })
            .collect();
        assert_eq!(sends.len(), 1, "exactly one sendMessage for one crossing");
        let buttons = sends[0]["reply_markup"]["inline_keyboard"][0]
            .as_array()
            .expect("buttons array");
        assert_eq!(buttons.len(), 4, "one button per parsed option");
    }

    #[tokio::test]
    async fn crossing_with_non_question_pane_sends_no_message() {
        let client = Arc::new(FakeQaTelegramClient::new());
        let bridge = SessionQaBridge::with_seams(
            "chat-1".to_string(),
            client.clone(),
            capture_returning(NON_QUESTION_PANE),
            inject_ok(),
        );

        bridge.handle_edge_record(edge_record("sess-1")).await;

        assert!(
            client.calls().is_empty(),
            "a permission-dialog pane must send nothing"
        );
    }

    #[tokio::test]
    async fn duplicate_crossing_for_still_pending_question_sends_no_second_message() {
        let client = Arc::new(FakeQaTelegramClient::new());
        let bridge = SessionQaBridge::with_seams(
            "chat-1".to_string(),
            client.clone(),
            capture_returning(SAMPLE_PANE),
            inject_ok(),
        );

        bridge.handle_edge_record(edge_record("sess-1")).await;
        bridge.handle_edge_record(edge_record("sess-1")).await;

        assert_eq!(
            client.calls_matching(|c| matches!(c, Call::SendMessage(_))),
            1,
            "a repeat crossing for an identical still-pending question must not re-send"
        );
    }

    // ── Outbound: tap → injection ────────────────────────────────────────

    #[tokio::test]
    async fn tap_option_acks_then_injects_digit_then_edits_message() {
        let client = Arc::new(FakeQaTelegramClient::new());
        let (inject, inject_log) = inject_recording();
        let bridge = SessionQaBridge::with_seams(
            "chat-1".to_string(),
            client.clone(),
            capture_returning(SAMPLE_PANE),
            inject,
        );
        bridge.handle_edge_record(edge_record("sess-1")).await;

        let calls = client.calls();
        let Call::SendMessage(body) = &calls[0] else {
            panic!("expected sendMessage first");
        };
        let question_id = question_id_from_send_message(body);

        let update = callback_query_update(1, "cbq-1", &question_id, 2, 999, 555);
        bridge.handle_update(&update).await;

        let calls = client.calls();
        let ack_idx = calls
            .iter()
            .position(|c| matches!(c, Call::AnswerCallbackQuery(_)))
            .expect("answerCallbackQuery must have been called");
        let edit_idx = calls
            .iter()
            .position(|c| matches!(c, Call::EditMessageText(_)))
            .expect("editMessageText must have been called");
        assert!(
            ack_idx < edit_idx,
            "answerCallbackQuery must be issued before editMessageText"
        );

        let injected = inject_log.lock().expect("inject log mutex poisoned");
        assert_eq!(injected.len(), 1);
        assert_eq!(injected[0], ("sess-1".to_string(), "2".to_string()));

        let Call::EditMessageText(edit_body) = &calls[edit_idx] else {
            unreachable!()
        };
        assert_eq!(edit_body["message_id"], 555);
        assert_eq!(edit_body["chat_id"], "chat-1");
    }

    #[tokio::test]
    async fn tap_targets_only_the_session_recorded_on_the_question() {
        // The update JSON carries no session name at all — this test exists
        // to document (and pin) that injection can only ever be addressed by
        // the pending question's own recorded session, never by anything in
        // the Telegram update.
        let client = Arc::new(FakeQaTelegramClient::new());
        let (inject, inject_log) = inject_recording();
        let bridge = SessionQaBridge::with_seams(
            "chat-1".to_string(),
            client.clone(),
            capture_returning(SAMPLE_PANE),
            inject,
        );
        bridge
            .handle_edge_record(edge_record("the-real-session"))
            .await;

        let calls = client.calls();
        let Call::SendMessage(body) = &calls[0] else {
            panic!("expected sendMessage first");
        };
        let question_id = question_id_from_send_message(body);

        let update = callback_query_update(1, "cbq-1", &question_id, 1, 999, 555);
        bridge.handle_update(&update).await;

        let injected = inject_log.lock().expect("inject log mutex poisoned");
        assert_eq!(injected[0].0, "the-real-session");
    }

    #[tokio::test]
    async fn tap_escape_hatch_then_plain_text_relays_verbatim() {
        let client = Arc::new(FakeQaTelegramClient::new());
        let (inject, inject_log) = inject_recording();
        let bridge = SessionQaBridge::with_seams(
            "chat-1".to_string(),
            client.clone(),
            capture_returning(SAMPLE_PANE),
            inject,
        );
        bridge.handle_edge_record(edge_record("sess-1")).await;

        let calls = client.calls();
        let Call::SendMessage(body) = &calls[0] else {
            panic!("expected sendMessage first");
        };
        let question_id = question_id_from_send_message(body);

        // Option 4 is the ChatAbout option in SAMPLE_PANE ("Chat about this").
        // The tap injects digit-with-Enter immediately (closing the widget);
        // it does not wait for the follow-up text before injecting.
        let tap = callback_query_update(1, "cbq-1", &question_id, 4, 999, 555);
        bridge.handle_update(&tap).await;

        assert_eq!(
            inject_log.lock().expect("inject log mutex poisoned").len(),
            1,
            "a ChatAbout tap must inject the digit at tap time"
        );

        let follow_up = text_message_update(2, 999, "use postgres please");
        bridge.handle_update(&follow_up).await;

        let injected = inject_log.lock().expect("inject log mutex poisoned");
        assert_eq!(injected.len(), 2);
        assert_eq!(
            injected[1],
            ("sess-1".to_string(), "use postgres please".to_string())
        );
    }

    #[tokio::test]
    async fn tap_on_already_answered_question_acks_distinctly_with_no_second_injection() {
        let client = Arc::new(FakeQaTelegramClient::new());
        let (inject, inject_log) = inject_recording();
        let bridge = SessionQaBridge::with_seams(
            "chat-1".to_string(),
            client.clone(),
            capture_returning(SAMPLE_PANE),
            inject,
        );
        bridge.handle_edge_record(edge_record("sess-1")).await;
        let calls = client.calls();
        let Call::SendMessage(body) = &calls[0] else {
            panic!("expected sendMessage first");
        };
        let question_id = question_id_from_send_message(body);

        let first_tap = callback_query_update(1, "cbq-1", &question_id, 1, 999, 555);
        bridge.handle_update(&first_tap).await;
        assert_eq!(inject_log.lock().expect("mutex poisoned").len(), 1);

        let second_tap = callback_query_update(2, "cbq-2", &question_id, 2, 999, 555);
        bridge.handle_update(&second_tap).await;

        assert_eq!(
            inject_log.lock().expect("mutex poisoned").len(),
            1,
            "a tap on an already-answered question must not inject a second time"
        );

        let acks: Vec<_> = client
            .calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::AnswerCallbackQuery(body) => Some(body),
                _ => None,
            })
            .collect();
        assert_eq!(acks.len(), 2);
        let second_ack_text = acks[1]["text"].as_str().expect("text field");
        assert!(
            second_ack_text.to_lowercase().contains("already"),
            "expected a distinguishing 'already answered' ack, got: {second_ack_text}"
        );
    }

    #[tokio::test]
    async fn tap_on_unknown_question_id_gets_unknown_question_ack() {
        let client = Arc::new(FakeQaTelegramClient::new());
        let (inject, inject_log) = inject_recording();
        let bridge = SessionQaBridge::with_seams(
            "chat-1".to_string(),
            client.clone(),
            capture_returning(SAMPLE_PANE),
            inject,
        );

        let tap = callback_query_update(1, "cbq-1", "no-such-question", 1, 999, 555);
        bridge.handle_update(&tap).await;

        assert!(inject_log.lock().expect("mutex poisoned").is_empty());
        let acks: Vec<_> = client
            .calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::AnswerCallbackQuery(body) => Some(body),
                _ => None,
            })
            .collect();
        assert_eq!(acks.len(), 1);
        let text = acks[0]["text"].as_str().expect("text field");
        assert!(
            text.to_lowercase().contains("unknown") || text.to_lowercase().contains("expired"),
            "expected an unknown/expired-question ack, got: {text}"
        );
    }

    // ── Failure-path degradation ─────────────────────────────────────────

    #[tokio::test]
    async fn get_updates_429_retry_after_is_honored_and_the_loop_survives() {
        let client = Arc::new(FakeQaTelegramClient::new());
        client.push_get_updates(Err(NotifyError::RateLimited {
            retry_after_secs: 0,
        }));
        let bridge = SessionQaBridge::with_seams(
            "chat-1".to_string(),
            client.clone(),
            capture_returning(SAMPLE_PANE),
            inject_ok(),
        );

        // `run_outbound` loops forever by design — bound the test with a
        // short timeout and treat "still running when the timeout fires" as
        // proof the 429 did not terminate the loop.
        let outcome =
            tokio::time::timeout(std::time::Duration::from_millis(200), bridge.run_outbound())
                .await;
        assert!(
            outcome.is_err(),
            "run_outbound must still be running after honoring a 429 (timeout expected)"
        );
        assert!(
            client.calls_matching(|c| matches!(c, Call::GetUpdates)) >= 2,
            "the loop must have called getUpdates again after the rate limit"
        );
    }

    #[tokio::test]
    async fn capture_failure_is_logged_and_the_next_crossing_still_succeeds() {
        let client = Arc::new(FakeQaTelegramClient::new());
        let bridge = SessionQaBridge::with_seams(
            "chat-1".to_string(),
            client.clone(),
            capture_failing_for("sess-broken", SAMPLE_PANE),
            inject_ok(),
        );

        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let bridge = Arc::new(bridge);
        let inbound_bridge = bridge.clone();
        let handle = tokio::spawn(async move { inbound_bridge.run_inbound(rx).await });

        tx.send(edge_record("sess-broken"))
            .await
            .expect("send must succeed");
        tx.send(edge_record("sess-ok"))
            .await
            .expect("send must succeed");
        drop(tx);
        handle.await.expect("run_inbound task must not panic");

        assert_eq!(
            client.calls_matching(|c| matches!(c, Call::SendMessage(_))),
            1,
            "the capture failure must drop only its own crossing, not the next one"
        );
    }

    #[tokio::test]
    async fn injection_failure_is_logged_and_the_bridge_keeps_working() {
        let client = Arc::new(FakeQaTelegramClient::new());
        let bridge = SessionQaBridge::with_seams(
            "chat-1".to_string(),
            client.clone(),
            capture_returning(SAMPLE_PANE),
            inject_always_err(),
        );

        bridge.handle_edge_record(edge_record("sess-1")).await;
        let calls = client.calls();
        let Call::SendMessage(body) = &calls[0] else {
            panic!("expected sendMessage first");
        };
        let question_id = question_id_from_send_message(body);

        let tap = callback_query_update(1, "cbq-1", &question_id, 1, 999, 555);
        // Must not panic even though injection fails.
        bridge.handle_update(&tap).await;

        assert_eq!(
            client.calls_matching(|c| matches!(c, Call::AnswerCallbackQuery(_))),
            1,
            "the tap must still be acknowledged even though injection failed"
        );

        // A second, unrelated crossing must still work — one injection
        // failure must not have poisoned the bridge.
        bridge.handle_edge_record(edge_record("sess-2")).await;
        assert_eq!(
            client.calls_matching(|c| matches!(c, Call::SendMessage(_))),
            2,
            "the bridge must keep working after an injection failure"
        );
    }

    // ── Redaction ─────────────────────────────────────────────────────────

    /// A distinctive sentinel value standing in for a real bot token. Not
    /// shaped like a real Telegram token (`[0-9]{6,12}:[A-Za-z0-9_-]{30,}`)
    /// so it can't trip the repo's own secret scan, but distinctive enough
    /// that its absence from captured log output is a meaningful assertion.
    const SENTINEL_TOKEN: &str = "SESSION-QA-SENTINEL-VALUE-MUST-NEVER-LOG-9f8e7d6c";

    /// `tracing_subscriber::fmt::MakeWriter` over a shared in-memory buffer,
    /// mirroring `serve::mod`'s `SharedLogBuf` test helper — lets a test
    /// assert on captured log lines with no real log file or stdout race
    /// against sibling tests.
    #[derive(Clone)]
    struct SharedLogBuf(Arc<StdMutex<Vec<u8>>>);

    impl std::io::Write for SharedLogBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SharedLogBuf {
        type Writer = SharedLogBuf;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[tokio::test]
    async fn failing_path_never_logs_the_bot_token() {
        // Real client, holding a sentinel token — but the capture failure
        // below means the client is never actually called, so this never
        // makes a real network call.
        let client: Arc<dyn QaTelegramClient> =
            Arc::new(HttpQaTelegramClient::new(BotToken::new(SENTINEL_TOKEN)));
        let bridge = SessionQaBridge::with_seams(
            "chat-1".to_string(),
            client,
            capture_failing_for("sess-1", SAMPLE_PANE),
            inject_always_err(),
        );

        let buf = Arc::new(StdMutex::new(Vec::new()));
        let writer = SharedLogBuf(buf.clone());
        let subscriber = tracing_subscriber::fmt()
            .with_writer(writer)
            .with_ansi(false)
            .without_time()
            .with_target(false)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        // Failing path 1: pane capture fails.
        bridge.handle_edge_record(edge_record("sess-1")).await;
        // Failing path 2: an update decodes but names an unknown question,
        // and (separately) injection is wired to always fail.
        let tap = callback_query_update(1, "cbq-1", "no-such-question", 1, 999, 555);
        bridge.handle_update(&tap).await;

        drop(_guard);
        let logs =
            String::from_utf8_lossy(&buf.lock().unwrap_or_else(|e| e.into_inner())).to_string();

        assert!(
            !logs.contains(SENTINEL_TOKEN),
            "captured log output must never contain the bot token; got:\n{logs}"
        );
    }

    // ── Ordered keystroke-sequence coverage (ticket-session-qa-freetext-injection) ──
    //
    // Each test below asserts the FULL ORDERED sequence of injected
    // `(session, text, with_enter)` tuples, never a call count alone — a
    // call-count assertion is exactly what let the shipped `BA.20.C` defect
    // (a `FreeText`/`ChatAbout` tap silently submitting option 1) pass
    // review. `SAMPLE_PANE` parses to: 1/2 `Choice`, 3 `FreeText` ("Type
    // something."), 4 `ChatAbout` ("Chat about this").

    /// Sends a crossing through `bridge`, returning the question id the
    /// bridge generated for it.
    async fn send_sample_crossing(
        bridge: &SessionQaBridge,
        client: &FakeQaTelegramClient,
        session: &str,
    ) -> String {
        bridge.handle_edge_record(edge_record(session)).await;
        let calls = client.calls();
        let Some(Call::SendMessage(body)) = calls
            .iter()
            .rev()
            .find(|c| matches!(c, Call::SendMessage(_)))
        else {
            panic!("expected at least one sendMessage call");
        };
        question_id_from_send_message(body)
    }

    #[tokio::test]
    async fn choice_tap_injects_exactly_digit_with_enter() {
        let client = Arc::new(FakeQaTelegramClient::new());
        let (inject, log) = inject_ordered_recording();
        let bridge = SessionQaBridge::with_seams(
            "chat-1".to_string(),
            client.clone(),
            capture_returning(SAMPLE_PANE),
            inject,
        );
        let question_id = send_sample_crossing(&bridge, &client, "sess-1").await;

        // Option 2 is "MySQL", a plain `Choice`.
        let tap = callback_query_update(1, "cbq-1", &question_id, 2, 999, 555);
        bridge.handle_update(&tap).await;

        let injected = log
            .lock()
            .expect("ordered inject log mutex poisoned")
            .clone();
        assert_eq!(
            injected,
            vec![("sess-1".to_string(), "2".to_string(), true)]
        );
    }

    #[tokio::test]
    async fn freetext_tap_then_relay_injects_digit_no_enter_then_text_with_enter() {
        let client = Arc::new(FakeQaTelegramClient::new());
        let (inject, log) = inject_ordered_recording();
        let bridge = SessionQaBridge::with_seams(
            "chat-1".to_string(),
            client.clone(),
            capture_returning(SAMPLE_PANE),
            inject,
        );
        let question_id = send_sample_crossing(&bridge, &client, "sess-1").await;

        // Option 3 is "Type something.", the `FreeText` option.
        let tap = callback_query_update(1, "cbq-1", &question_id, 3, 999, 555);
        bridge.handle_update(&tap).await;

        assert_eq!(
            log.lock().expect("mutex poisoned").clone(),
            vec![("sess-1".to_string(), "3".to_string(), false)],
            "a FreeText tap must inject only the digit, with no trailing Enter"
        );

        let follow_up = text_message_update(2, 999, "teal, actually");
        bridge.handle_update(&follow_up).await;

        assert_eq!(
            log.lock().expect("mutex poisoned").clone(),
            vec![
                ("sess-1".to_string(), "3".to_string(), false),
                ("sess-1".to_string(), "teal, actually".to_string(), true),
            ]
        );
    }

    #[tokio::test]
    async fn chatabout_tap_then_relay_injects_digit_with_enter_then_text_with_enter() {
        let client = Arc::new(FakeQaTelegramClient::new());
        let (inject, log) = inject_ordered_recording();
        let bridge = SessionQaBridge::with_seams(
            "chat-1".to_string(),
            client.clone(),
            capture_returning(SAMPLE_PANE),
            inject,
        );
        let question_id = send_sample_crossing(&bridge, &client, "sess-1").await;

        // Option 4 is "Chat about this", the `ChatAbout` option.
        let tap = callback_query_update(1, "cbq-1", &question_id, 4, 999, 555);
        bridge.handle_update(&tap).await;

        assert_eq!(
            log.lock().expect("mutex poisoned").clone(),
            vec![("sess-1".to_string(), "4".to_string(), true)],
            "a ChatAbout tap must inject the digit WITH Enter at tap time, closing the widget"
        );

        let follow_up = text_message_update(2, 999, "tell me more");
        bridge.handle_update(&follow_up).await;

        assert_eq!(
            log.lock().expect("mutex poisoned").clone(),
            vec![
                ("sess-1".to_string(), "4".to_string(), true),
                ("sess-1".to_string(), "tell me more".to_string(), true),
            ]
        );
    }

    #[tokio::test]
    async fn plain_text_with_no_armed_follow_up_injects_nothing() {
        let client = Arc::new(FakeQaTelegramClient::new());
        let (inject, log) = inject_ordered_recording();
        let bridge = SessionQaBridge::with_seams(
            "chat-1".to_string(),
            client.clone(),
            capture_returning(SAMPLE_PANE),
            inject,
        );

        // No crossing was ever sent, so no follow-up state is armed for
        // this chat — a stray message must relay nowhere.
        let stray = text_message_update(1, 999, "hello?");
        bridge.handle_update(&stray).await;

        assert!(
            log.lock().expect("mutex poisoned").is_empty(),
            "a relay with no armed follow-up state must inject nothing"
        );
    }

    #[tokio::test]
    async fn second_freetext_tap_replaces_armed_state_and_sends_second_digit_no_enter() {
        let client = Arc::new(FakeQaTelegramClient::new());
        let (inject, log) = inject_ordered_recording();
        let bridge = SessionQaBridge::with_seams(
            "chat-1".to_string(),
            client.clone(),
            capture_returning(SAMPLE_PANE),
            inject,
        );

        // Two separate crossings on the same chat, same session: tapping
        // FreeText on the second while the first's follow-up state is
        // still armed must replace it (`ReplacedAwaiting`), and the second
        // tap's digit must still go out with no trailing Enter.
        let first_question_id = send_sample_crossing(&bridge, &client, "sess-1").await;
        let first_tap = callback_query_update(1, "cbq-1", &first_question_id, 3, 999, 555);
        bridge.handle_update(&first_tap).await;

        let second_question_id = send_sample_crossing(&bridge, &client, "sess-1").await;
        let second_tap = callback_query_update(2, "cbq-2", &second_question_id, 3, 999, 555);
        bridge.handle_update(&second_tap).await;

        assert_eq!(
            log.lock().expect("mutex poisoned").clone(),
            vec![
                ("sess-1".to_string(), "3".to_string(), false),
                ("sess-1".to_string(), "3".to_string(), false),
            ],
            "both FreeText taps must inject their digit with no trailing Enter"
        );

        // The relay now resolves against the SECOND question (replaced
        // state), and still submits the text with Enter.
        let follow_up = text_message_update(3, 999, "final answer");
        bridge.handle_update(&follow_up).await;

        let injected = log.lock().expect("mutex poisoned").clone();
        assert_eq!(injected.len(), 3);
        assert_eq!(
            injected[2],
            ("sess-1".to_string(), "final answer".to_string(), true)
        );
    }

    /// Regression pin for the shipped `BA.20.C` defect: a `FreeText` tap
    /// followed by a relay must NEVER produce a log whose first entry
    /// carries `with_enter == true` — that sequence is exactly the one
    /// that silently submitted option 1 and discarded the operator's text.
    #[tokio::test]
    async fn regression_freetext_tap_first_entry_never_carries_enter() {
        let client = Arc::new(FakeQaTelegramClient::new());
        let (inject, log) = inject_ordered_recording();
        let bridge = SessionQaBridge::with_seams(
            "chat-1".to_string(),
            client.clone(),
            capture_returning(SAMPLE_PANE),
            inject,
        );
        let question_id = send_sample_crossing(&bridge, &client, "sess-1").await;

        let tap = callback_query_update(1, "cbq-1", &question_id, 3, 999, 555);
        bridge.handle_update(&tap).await;
        let follow_up = text_message_update(2, 999, "teal, actually");
        bridge.handle_update(&follow_up).await;

        let injected = log.lock().expect("mutex poisoned").clone();
        assert!(
            !injected.is_empty(),
            "a FreeText tap must inject at least the digit"
        );
        assert!(
            !injected[0].2,
            "REGRESSION: the digit for a FreeText tap must never be sent with a trailing \
             Enter — that is the shipped BA.20.C sequence that silently submitted option 1 \
             and discarded the operator's text; got: {injected:?}"
        );
    }
}
