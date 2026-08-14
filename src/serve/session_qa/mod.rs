//! Pure core of the Telegram session-QA bridge (`BA.20.C` task 4).
//!
//! Everything in this module is pure logic: no HTTP call, no tmux call, no
//! task spawn. It exists so the bridge's decision-making — "have we already
//! asked this?", "does this callback tap resolve to a valid, still-pending
//! question?", "what does a plain-text follow-up mean for this chat right
//! now?" — is exhaustively unit-tested without a network or a live tmux
//! session anywhere in reach. The I/O shell that calls into this module
//! (capturing panes, calling Telegram, injecting into tmux) is added in a
//! later task (`BA.20.C` task 5), kept thin over this core per `CLAUDE.md`
//! rule 6.
//!
//! This module deliberately does **not** reuse `notify::telegram`'s
//! `CallbackData` / `ResponseVerdict` / `resolve_response` — those are
//! hard-typed to `ValidatedOperatorPayload` / `AckHandle` / gate digests
//! (approve/reject decisions). A session-QA question has no gate, no
//! digest, and its options are plain 1-indexed numbers, not machine option
//! keys — so this module writes its own question-shaped equivalents, per
//! the plan's Architecture decision 2.

use std::collections::{HashMap, VecDeque};

use chrono::{DateTime, Utc};

use crate::sessions::ask_question::AskQuestionPrompt;

// ── PendingQuestion / PendingQuestions registry ─────────────────────────────

/// One question this process has sent (or is about to send) to Telegram,
/// awaiting a tap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingQuestion {
    /// The tmux session name this question was captured from — injection
    /// (task 5) targets only this session, never anything named in an
    /// inbound Telegram update.
    pub session: String,
    /// The parsed prompt this question was built from.
    pub prompt: AskQuestionPrompt,
    /// The Telegram message id the question was sent as, once known. `None`
    /// until the `sendMessage` call (task 5) resolves — registration
    /// happens before the HTTP call completes so a duplicate crossing that
    /// races the send still dedups against this entry.
    pub message_id: Option<i64>,
    /// When this question was registered.
    pub asked_at: DateTime<Utc>,
    /// Whether a tap has already been accepted for this question. Set by
    /// [`PendingQuestions::mark_answered`]; a second tap after this is
    /// `AlreadyAnswered`, never a second injection.
    pub answered: bool,
}

impl PendingQuestion {
    #[must_use]
    pub fn new(
        session: impl Into<String>,
        prompt: AskQuestionPrompt,
        asked_at: DateTime<Utc>,
    ) -> Self {
        Self {
            session: session.into(),
            prompt,
            message_id: None,
            asked_at,
            answered: false,
        }
    }
}

/// The outcome of [`PendingQuestions::register`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterOutcome {
    /// A new entry was created under `question_id`; the caller should send
    /// a Telegram message for it.
    Created { question_id: String },
    /// An unanswered entry for this exact `(session, prompt)` already
    /// existed; its id is returned and the caller must send nothing — this
    /// is what makes "sent once, not re-sent every tick" true.
    AlreadyPending { question_id: String },
}

impl RegisterOutcome {
    #[must_use]
    pub fn question_id(&self) -> &str {
        match self {
            RegisterOutcome::Created { question_id }
            | RegisterOutcome::AlreadyPending { question_id } => question_id,
        }
    }
}

struct PendingQuestionsInner {
    by_id: HashMap<String, PendingQuestion>,
    order: VecDeque<String>,
    next_seq: u64,
}

/// Bounded, FIFO-eviction registry of [`PendingQuestion`]s, keyed by a
/// bridge-generated opaque id. Mirrors
/// [`crate::serve::notify::PendingPayloads`]'s shape and eviction policy —
/// a long-running server must not be able to grow this registry without
/// limit.
pub struct PendingQuestions {
    inner: std::sync::Mutex<PendingQuestionsInner>,
}

impl PendingQuestions {
    /// Maximum number of pending questions held at once, mirroring
    /// [`crate::serve::notify::PendingPayloads::CAPACITY`]'s generous
    /// headroom above any plausible in-flight volume.
    pub const CAPACITY: usize = 256;

    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: std::sync::Mutex::new(PendingQuestionsInner {
                by_id: HashMap::new(),
                order: VecDeque::new(),
                next_seq: 0,
            }),
        }
    }

    fn mutex_lock(&self) -> std::sync::MutexGuard<'_, PendingQuestionsInner> {
        self.inner.lock().expect("PendingQuestions mutex poisoned")
    }

    /// Register a question for `session`, deduping against any existing
    /// unanswered entry for the same session whose `prompt` is `==` to this
    /// one. Bound-exceeding insertion evicts the single oldest entry first
    /// (FIFO by insertion order), exactly like `PendingPayloads::insert`.
    pub fn register(
        &self,
        session: impl Into<String>,
        prompt: AskQuestionPrompt,
        asked_at: DateTime<Utc>,
    ) -> RegisterOutcome {
        let session = session.into();
        let mut inner = self.mutex_lock();

        if let Some(existing_id) = inner.by_id.iter().find_map(|(id, q)| {
            (!q.answered && q.session == session && q.prompt == prompt).then(|| id.clone())
        }) {
            return RegisterOutcome::AlreadyPending {
                question_id: existing_id,
            };
        }

        if inner.by_id.len() >= Self::CAPACITY
            && let Some(oldest) = inner.order.pop_front()
        {
            inner.by_id.remove(&oldest);
        }

        let question_id = format!("q{}", inner.next_seq);
        inner.next_seq += 1;
        inner.order.push_back(question_id.clone());
        inner.by_id.insert(
            question_id.clone(),
            PendingQuestion::new(session, prompt, asked_at),
        );

        RegisterOutcome::Created { question_id }
    }

    /// Record the Telegram `message_id` a question was sent as.
    pub fn set_message_id(&self, question_id: &str, message_id: i64) {
        let mut inner = self.mutex_lock();
        if let Some(q) = inner.by_id.get_mut(question_id) {
            q.message_id = Some(message_id);
        }
    }

    /// Look up a question by id.
    #[must_use]
    pub fn get(&self, question_id: &str) -> Option<PendingQuestion> {
        self.mutex_lock().by_id.get(question_id).cloned()
    }

    /// Mark a question answered. No-op if the id is unknown.
    pub fn mark_answered(&self, question_id: &str) {
        let mut inner = self.mutex_lock();
        if let Some(q) = inner.by_id.get_mut(question_id) {
            q.answered = true;
        }
    }

    /// Current number of entries held (for tests).
    #[must_use]
    pub fn len(&self) -> usize {
        self.mutex_lock().by_id.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for PendingQuestions {
    fn default() -> Self {
        Self::new()
    }
}

// ── Callback payload encode/decode ──────────────────────────────────────────

/// Telegram's confirmed ceiling on `callback_data`, in bytes. Same value as
/// `notify::telegram::CALLBACK_DATA_MAX_BYTES`, restated here rather than
/// imported so this module's encoding stays independently provable against
/// the limit without depending on that module's constant meaning the same
/// thing forever.
pub const QA_CALLBACK_DATA_MAX_BYTES: usize = 64;

/// The delimiter joining `question_id` / `option_number` inside an encoded
/// callback string.
const QA_CALLBACK_DELIMITER: char = '|';

/// The decoded contents of a question-tap `callback_data` string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionCallback {
    pub question_id: String,
    pub option_number: usize,
}

/// Encode a question tap into a Telegram `callback_data` string. Pure and
/// round-trippable via [`decode_question_callback`].
#[must_use]
pub fn encode_question_callback(question_id: &str, option_number: usize) -> String {
    format!("{question_id}{QA_CALLBACK_DELIMITER}{option_number}")
}

/// Decode a question-tap `callback_data` string. `None` if `raw` does not
/// have exactly two delimiter-separated fields, or the second field is not
/// a valid `usize`.
#[must_use]
pub fn decode_question_callback(raw: &str) -> Option<QuestionCallback> {
    let mut parts = raw.splitn(2, QA_CALLBACK_DELIMITER);
    let question_id = parts.next()?.to_string();
    let option_raw = parts.next()?;
    if question_id.is_empty() || option_raw.is_empty() {
        return None;
    }
    let option_number: usize = option_raw.parse().ok()?;
    Some(QuestionCallback {
        question_id,
        option_number,
    })
}

// ── QuestionVerdict / resolve_question_response ─────────────────────────────

/// The outcome of resolving a decoded [`QuestionCallback`] against the
/// [`PendingQuestions`] registry — the question-shaped analogue of
/// `notify::telegram::ResponseVerdict`, reimplemented rather than shared
/// (see this module's top-level docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestionVerdict {
    /// The question exists, is unanswered, and `option_number` names one of
    /// its parsed options.
    Accepted {
        session: String,
        option_number: usize,
        /// The digit/label to inject — the tapped option's 1-indexed number
        /// as text, ready to send followed by Enter.
        digit: String,
        is_escape_hatch: bool,
    },
    /// The question exists but was already answered by an earlier tap.
    AlreadyAnswered,
    /// No pending question matches this id (never registered, evicted, or
    /// stale).
    UnknownQuestion,
}

/// Resolve a decoded callback against `registry`. Pure with respect to the
/// registry snapshot passed in the sense that it performs exactly one
/// lookup and, on `Accepted`, does **not** itself mark the question
/// answered — callers (task 5) call [`PendingQuestions::mark_answered`]
/// only after every other step of handling the tap has succeeded, mirroring
/// `answerCallbackQuery`-before-injection ordering at the call site.
#[must_use]
pub fn resolve_question_response(
    callback: &QuestionCallback,
    registry: &PendingQuestions,
) -> QuestionVerdict {
    let Some(question) = registry.get(&callback.question_id) else {
        return QuestionVerdict::UnknownQuestion;
    };

    if question.answered {
        return QuestionVerdict::AlreadyAnswered;
    }

    let Some(option) = question
        .prompt
        .options
        .iter()
        .find(|opt| opt.number == callback.option_number)
    else {
        return QuestionVerdict::UnknownQuestion;
    };

    QuestionVerdict::Accepted {
        session: question.session,
        option_number: option.number,
        digit: option.number.to_string(),
        is_escape_hatch: option.is_escape_hatch,
    }
}

// ── Pure body builders ──────────────────────────────────────────────────────

/// Render a `sendMessage` body for `prompt`: the question text, and one
/// inline-keyboard button per option (escape-hatch button visually
/// distinguished with a leading glyph), each button's `callback_data`
/// encoding `question_id` + that option's number. Mirrors
/// `notify::telegram::sendmessage_body`'s shape, typed to this block's
/// data. Pure — no I/O.
#[must_use]
pub fn sendmessage_body(
    question_id: &str,
    prompt: &AskQuestionPrompt,
    chat_id: &str,
) -> serde_json::Value {
    let buttons: Vec<serde_json::Value> = prompt
        .options
        .iter()
        .map(|opt| {
            let text = if opt.is_escape_hatch {
                format!("💬 {}", opt.label)
            } else {
                format!("{}. {}", opt.number, opt.label)
            };
            serde_json::json!({
                "text": text,
                "callback_data": encode_question_callback(question_id, opt.number),
            })
        })
        .collect();

    serde_json::json!({
        "chat_id": chat_id,
        "text": prompt.question,
        "reply_markup": {
            "inline_keyboard": [buttons],
        },
    })
}

/// Telegram's confirmed ceiling on an `answerCallbackQuery` `text` field, in
/// characters — same limit `notify::telegram` observes, restated here for
/// the same independent-provability reason as [`QA_CALLBACK_DATA_MAX_BYTES`].
pub const QA_ANSWER_CALLBACK_TEXT_MAX_CHARS: usize = 200;

fn clamp_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        text.chars().take(max_chars).collect()
    }
}

/// Render an `answerCallbackQuery` body for `verdict`, acknowledging the tap
/// identified by `callback_query_id`. Every verdict arm gets distinct,
/// non-empty text, mirroring `notify::telegram::answercallbackquery_body`.
/// Pure — no I/O.
#[must_use]
pub fn answercallbackquery_body(
    callback_query_id: &str,
    verdict: &QuestionVerdict,
) -> serde_json::Value {
    let text = match verdict {
        QuestionVerdict::Accepted { option_number, .. } => {
            format!("Recorded: option {option_number}")
        }
        QuestionVerdict::AlreadyAnswered => "Already answered".to_string(),
        QuestionVerdict::UnknownQuestion => "Unknown or expired question".to_string(),
    };

    serde_json::json!({
        "callback_query_id": callback_query_id,
        "text": clamp_chars(&text, QA_ANSWER_CALLBACK_TEXT_MAX_CHARS),
    })
}

/// Render an `editMessageText` body that removes the inline keyboard and
/// shows which option was chosen. Mirrors
/// `notify::telegram::editmessagetext_body`. Pure — no I/O.
#[must_use]
pub fn editmessagetext_body(
    chat_id: &str,
    message_id: i64,
    question: &str,
    chosen_label: &str,
) -> serde_json::Value {
    serde_json::json!({
        "chat_id": chat_id,
        "message_id": message_id,
        "text": format!("{question}\n\nAnswered: {chosen_label}"),
        "reply_markup": {
            "inline_keyboard": [],
        },
    })
}

// ── Per-chat escape-hatch follow-up state machine ───────────────────────────

/// What a chat is currently doing, with respect to the escape-hatch
/// free-text follow-up flow.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ChatFollowUpState {
    /// Not awaiting anything from this chat.
    #[default]
    Idle,
    /// The escape hatch was tapped for `question_id`; the next plain-text
    /// message from this chat should be relayed verbatim to that
    /// question's session, and the state returns to `Idle`.
    AwaitingFreeText { question_id: String },
}

/// What happened as a result of feeding an event into the follow-up state
/// machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FollowUpOutcome {
    /// The state transitioned to (or stayed in) `AwaitingFreeText` for
    /// `question_id` — a first escape-hatch tap.
    NowAwaiting { question_id: String },
    /// A second escape-hatch tap arrived while already awaiting free text
    /// for a (possibly different) question. The **new** tap wins: the state
    /// is reset to await free text for the new `question_id`, and the old
    /// one is abandoned — a chat can only ever be relaying for one question
    /// at a time, and the most recent tap best reflects operator intent.
    ReplacedAwaiting {
        old_question_id: String,
        new_question_id: String,
    },
    /// A plain-text message arrived while awaiting free text: the message
    /// is the relay target. State returns to `Idle`.
    RelayText { question_id: String, text: String },
    /// A plain-text message arrived while `Idle` — nothing to relay it to.
    /// State stays `Idle`.
    Ignored,
}

impl ChatFollowUpState {
    /// Feed an escape-hatch tap for `question_id` into the state machine,
    /// returning the new state and what happened.
    #[must_use]
    pub fn on_escape_hatch_tap(self, question_id: impl Into<String>) -> (Self, FollowUpOutcome) {
        let question_id = question_id.into();
        match self {
            ChatFollowUpState::Idle => {
                let outcome = FollowUpOutcome::NowAwaiting {
                    question_id: question_id.clone(),
                };
                (ChatFollowUpState::AwaitingFreeText { question_id }, outcome)
            }
            ChatFollowUpState::AwaitingFreeText {
                question_id: old_question_id,
            } => {
                let outcome = FollowUpOutcome::ReplacedAwaiting {
                    old_question_id,
                    new_question_id: question_id.clone(),
                };
                (ChatFollowUpState::AwaitingFreeText { question_id }, outcome)
            }
        }
    }

    /// Feed a plain-text message into the state machine.
    #[must_use]
    pub fn on_plain_text(self, text: impl Into<String>) -> (Self, FollowUpOutcome) {
        match self {
            ChatFollowUpState::Idle => (ChatFollowUpState::Idle, FollowUpOutcome::Ignored),
            ChatFollowUpState::AwaitingFreeText { question_id } => (
                ChatFollowUpState::Idle,
                FollowUpOutcome::RelayText {
                    question_id,
                    text: text.into(),
                },
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::ask_question::QuestionOption;

    fn prompt(question: &str, options: &[(usize, &str, bool)]) -> AskQuestionPrompt {
        AskQuestionPrompt {
            question: question.to_string(),
            options: options
                .iter()
                .map(|(number, label, is_escape_hatch)| QuestionOption {
                    number: *number,
                    label: (*label).to_string(),
                    description: None,
                    is_escape_hatch: *is_escape_hatch,
                })
                .collect(),
        }
    }

    fn sample_prompt() -> AskQuestionPrompt {
        prompt(
            "Which database?",
            &[
                (1, "Postgres", false),
                (2, "MySQL", false),
                (3, "Chat about this", true),
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
        let other = prompt("Which cache?", &[(1, "Redis", false)]);
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
                is_escape_hatch: false,
            }
        );
    }

    #[test]
    fn resolve_accepted_flags_escape_hatch_option() {
        let registry = PendingQuestions::new();
        let outcome = registry.register("session-a", sample_prompt(), Utc::now());
        let callback = QuestionCallback {
            question_id: outcome.question_id().to_string(),
            option_number: 3,
        };
        let verdict = resolve_question_response(&callback, &registry);
        match verdict {
            QuestionVerdict::Accepted {
                is_escape_hatch, ..
            } => assert!(is_escape_hatch),
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
                is_escape_hatch: false,
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
        let (new_state, outcome) = state.on_escape_hatch_tap("q1");
        assert_eq!(
            new_state,
            ChatFollowUpState::AwaitingFreeText {
                question_id: "q1".to_string()
            }
        );
        assert_eq!(
            outcome,
            FollowUpOutcome::NowAwaiting {
                question_id: "q1".to_string()
            }
        );
    }

    #[test]
    fn second_escape_hatch_tap_while_awaiting_replaces_with_new_question() {
        let state = ChatFollowUpState::AwaitingFreeText {
            question_id: "q1".to_string(),
        };
        let (new_state, outcome) = state.on_escape_hatch_tap("q2");
        assert_eq!(
            new_state,
            ChatFollowUpState::AwaitingFreeText {
                question_id: "q2".to_string()
            }
        );
        assert_eq!(
            outcome,
            FollowUpOutcome::ReplacedAwaiting {
                old_question_id: "q1".to_string(),
                new_question_id: "q2".to_string(),
            }
        );
    }

    #[test]
    fn plain_text_while_awaiting_relays_and_returns_to_idle() {
        let state = ChatFollowUpState::AwaitingFreeText {
            question_id: "q1".to_string(),
        };
        let (new_state, outcome) = state.on_plain_text("use postgres please");
        assert_eq!(new_state, ChatFollowUpState::Idle);
        assert_eq!(
            outcome,
            FollowUpOutcome::RelayText {
                question_id: "q1".to_string(),
                text: "use postgres please".to_string(),
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
}
