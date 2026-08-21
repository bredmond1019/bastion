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

use crate::sessions::ask_question::{AskQuestionPrompt, OptionKind};

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
/// `engine_core::operator::ResponseVerdict`, reimplemented rather than
/// shared (see this module's top-level docs).
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
        kind: OptionKind,
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
        kind: option.kind,
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
            let text = match opt.kind {
                OptionKind::ChatAbout => format!("💬 {}", opt.label),
                OptionKind::FreeText | OptionKind::Choice => {
                    format!("{}. {}", opt.number, opt.label)
                }
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
    /// question's session, and the state returns to `Idle`. `kind` records
    /// which `OptionKind` armed this state (`FreeText` or `ChatAbout`) so
    /// the relay site (`handle_message`) — and, transitively, whichever
    /// caller reads the resulting `FollowUpOutcome` — can tell the two
    /// cases apart. `Choice` never arms this state.
    AwaitingFreeText {
        question_id: String,
        kind: OptionKind,
    },
}

/// What happened as a result of feeding an event into the follow-up state
/// machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FollowUpOutcome {
    /// The state transitioned to (or stayed in) `AwaitingFreeText` for
    /// `question_id` — a first escape-hatch tap.
    NowAwaiting {
        question_id: String,
        kind: OptionKind,
    },
    /// A second escape-hatch tap arrived while already awaiting free text
    /// for a (possibly different) question. The **new** tap wins: the state
    /// is reset to await free text for the new `question_id`, and the old
    /// one is abandoned — a chat can only ever be relaying for one question
    /// at a time, and the most recent tap best reflects operator intent.
    ReplacedAwaiting {
        old_question_id: String,
        new_question_id: String,
        new_kind: OptionKind,
    },
    /// A plain-text message arrived while awaiting free text: the message
    /// is the relay target. State returns to `Idle`.
    RelayText {
        question_id: String,
        text: String,
        kind: OptionKind,
    },
    /// A plain-text message arrived while `Idle` — nothing to relay it to.
    /// State stays `Idle`.
    Ignored,
}

impl ChatFollowUpState {
    /// Feed an escape-hatch tap for `question_id` (armed by `kind`, either
    /// `FreeText` or `ChatAbout`) into the state machine, returning the new
    /// state and what happened.
    #[must_use]
    pub fn on_escape_hatch_tap(
        self,
        question_id: impl Into<String>,
        kind: OptionKind,
    ) -> (Self, FollowUpOutcome) {
        let question_id = question_id.into();
        match self {
            ChatFollowUpState::Idle => {
                let outcome = FollowUpOutcome::NowAwaiting {
                    question_id: question_id.clone(),
                    kind,
                };
                (
                    ChatFollowUpState::AwaitingFreeText { question_id, kind },
                    outcome,
                )
            }
            ChatFollowUpState::AwaitingFreeText {
                question_id: old_question_id,
                ..
            } => {
                let outcome = FollowUpOutcome::ReplacedAwaiting {
                    old_question_id,
                    new_question_id: question_id.clone(),
                    new_kind: kind,
                };
                (
                    ChatFollowUpState::AwaitingFreeText { question_id, kind },
                    outcome,
                )
            }
        }
    }

    /// Feed a plain-text message into the state machine.
    #[must_use]
    pub fn on_plain_text(self, text: impl Into<String>) -> (Self, FollowUpOutcome) {
        match self {
            ChatFollowUpState::Idle => (ChatFollowUpState::Idle, FollowUpOutcome::Ignored),
            ChatFollowUpState::AwaitingFreeText { question_id, kind } => (
                ChatFollowUpState::Idle,
                FollowUpOutcome::RelayText {
                    question_id,
                    text: text.into(),
                    kind,
                },
            ),
        }
    }
}

// ── I/O shell — bridge runtime (`BA.20.C` task 5) ───────────────────────────
//
// Everything below is a thin shell over the pure core above, per `CLAUDE.md`
// rule 6: every decision (dedup, verdict resolution, follow-up state
// transition, body construction) is delegated to the pure functions/types
// already defined in this module. This section only performs HTTP calls,
// tmux capture/injection, and channel plumbing — and every one of those
// seams is injectable so task 7 can drive the whole flow with no network and
// no tmux, exactly as `BlockedEdgePoller::with_capture` does.

use std::collections::HashMap as StdHashMap;
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::config::{BotToken, CodeSessionsBotConfig};
use crate::serve::blocked_edge::sink::BlockedEdgeRecord;
use crate::serve::notify::{NotifyError, telegram_http};
use crate::sessions::ask_question::parse_ask_question;

/// The bridge's view of the Telegram Bot API, boxed as a trait object so
/// task 7 can inject a fake with no real network call — mirrors
/// `BlockedEdgePoller::with_capture`'s seam.
///
/// Every method takes/returns pre-built `serde_json::Value` bodies (from
/// this module's pure body builders) rather than typed request structs, so
/// the trait stays a thin transport seam and never duplicates the pure
/// core's shapes.
#[async_trait]
pub trait QaTelegramClient: Send + Sync {
    async fn send_message(&self, body: serde_json::Value)
    -> Result<serde_json::Value, NotifyError>;
    async fn answer_callback_query(&self, body: serde_json::Value) -> Result<(), NotifyError>;
    async fn edit_message_text(&self, body: serde_json::Value) -> Result<(), NotifyError>;
    async fn get_updates(
        &self,
        query: &[(String, String)],
    ) -> Result<serde_json::Value, NotifyError>;
}

/// Real `reqwest`-backed [`QaTelegramClient`] against CodeSessionsBot.
///
/// **Never logs the constructed API URL** — every log line below names only
/// the bare Bot API method, per `telegram_http::api_url`'s doc comment (the
/// token lives in that URL's path).
pub struct HttpQaTelegramClient {
    bot_token: BotToken,
    client: reqwest::Client,
}

impl HttpQaTelegramClient {
    /// Client-side request timeout for non-long-poll calls.
    const REQUEST_TIMEOUT_SECS: u64 = 15;
    /// Long-poll timeout asked of Telegram for `getUpdates`.
    const GETUPDATES_TIMEOUT_SECS: u64 = 30;
    /// Client-side timeout for `getUpdates` — must exceed the long-poll
    /// timeout above or every call would time out on our side first.
    const GETUPDATES_CLIENT_TIMEOUT_SECS: u64 = Self::GETUPDATES_TIMEOUT_SECS + 10;

    #[must_use]
    pub fn new(bot_token: BotToken) -> Self {
        Self {
            bot_token,
            client: reqwest::Client::new(),
        }
    }

    fn api_url(&self, method: &str) -> String {
        telegram_http::api_url(&self.bot_token, method)
    }

    async fn post(
        &self,
        method: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, NotifyError> {
        tracing::debug!(method, "calling Telegram Bot API (session-qa bridge)");
        let resp = self
            .client
            .post(self.api_url(method))
            .timeout(std::time::Duration::from_secs(Self::REQUEST_TIMEOUT_SECS))
            .json(&body)
            .send()
            .await
            .map_err(|e| NotifyError::Transport {
                reason: format!(
                    "{method} request failed: {}",
                    telegram_http::classify_reqwest_error(&e)
                ),
            })?;

        let status = resp.status().as_u16();
        if let Some(err) = telegram_http::classify_http_status(
            status,
            telegram_http::retry_after_from_response(&resp),
        ) {
            return Err(err);
        }

        resp.json().await.map_err(|e| NotifyError::Malformed {
            reason: format!("{method} response body was not valid JSON: {e}"),
        })
    }
}

#[async_trait]
impl QaTelegramClient for HttpQaTelegramClient {
    async fn send_message(
        &self,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, NotifyError> {
        self.post("sendMessage", body).await
    }

    async fn answer_callback_query(&self, body: serde_json::Value) -> Result<(), NotifyError> {
        self.post("answerCallbackQuery", body).await.map(|_| ())
    }

    async fn edit_message_text(&self, body: serde_json::Value) -> Result<(), NotifyError> {
        self.post("editMessageText", body).await.map(|_| ())
    }

    async fn get_updates(
        &self,
        query: &[(String, String)],
    ) -> Result<serde_json::Value, NotifyError> {
        let method = "getUpdates";
        tracing::debug!(method, "calling Telegram Bot API (session-qa bridge)");
        let url = telegram_http::append_query_string(&self.api_url(method), query);
        let resp = self
            .client
            .get(url)
            .timeout(std::time::Duration::from_secs(
                Self::GETUPDATES_CLIENT_TIMEOUT_SECS,
            ))
            .send()
            .await
            .map_err(|e| NotifyError::Transport {
                reason: format!(
                    "{method} request failed: {}",
                    telegram_http::classify_reqwest_error(&e)
                ),
            })?;

        let status = resp.status().as_u16();
        if let Some(err) = telegram_http::classify_http_status(
            status,
            telegram_http::retry_after_from_response(&resp),
        ) {
            return Err(err);
        }

        resp.json().await.map_err(|e| NotifyError::Malformed {
            reason: format!("{method} response body was not valid JSON: {e}"),
        })
    }
}

/// Capture one session's pane, boxed so tests can inject a fake — mirrors
/// `BlockedEdgePoller`'s `CaptureFn` seam, scoped to a single named session
/// instead of a full sweep.
pub type CapturePaneFn = Box<dyn Fn(&str) -> anyhow::Result<String> + Send + Sync>;

/// Inject `text` (a digit, or relayed free text) into the named session,
/// followed by an Enter keypress only when `with_enter` is `true`. Boxed so
/// tests can inject a fake with no live tmux server.
///
/// The `with_enter` flag exists because the `AskUserQuestion` widget's
/// free-text option must be selected (digit sent) WITHOUT a trailing Enter —
/// Enter on that option with nothing typed yet closes the widget and submits
/// nothing. `ChatAbout` and `Choice` taps, and every relayed answer, still
/// want the trailing Enter.
pub type InjectFn = Box<dyn Fn(&str, &str, bool) -> anyhow::Result<()> + Send + Sync>;

/// Real [`CapturePaneFn`] over `sessions::tmux::capture_pane_raw`.
fn real_capture_pane(session: &str) -> anyhow::Result<String> {
    Ok(crate::sessions::tmux::capture_pane_raw(session)?)
}

/// Real [`InjectFn`] over `sessions::tmux::send_keys` — the same function
/// `POST /api/sessions/{name}/send`'s handler calls
/// (`handlers::sessions::send`). That handler is not factored to be called
/// directly from here (it is bound to `actix_web::web::Path`/`web::Json`
/// extractors and returns an `HttpResponse`), so this calls
/// `sessions::tmux::send_keys` directly instead — the underlying tmux call is
/// identical either way; only the HTTP-specific wrapping differs. Recorded
/// as a deviation in this spec's Amendment Log.
fn real_inject(session: &str, text: &str, with_enter: bool) -> anyhow::Result<()> {
    if with_enter {
        Ok(crate::sessions::tmux::send_keys(session, text)?)
    } else {
        Ok(crate::sessions::tmux::send_keys_no_enter(session, text)?)
    }
}

/// The Telegram-visible chat identity a follow-up-state entry is keyed by.
/// Telegram's own `chat.id`, rendered as a string (this bridge only ever
/// talks to one configured chat, but keying by the update's own chat id
/// keeps the state machine correct even if that ever changes, and makes the
/// per-chat semantics the spec calls for explicit rather than implicit).
type ChatKey = String;

/// The runtime bridge: consumes `BlockedEdgeRecord`s from task 3's channel
/// (inbound path) and runs one `getUpdates` long-poll loop against
/// CodeSessionsBot (outbound path). Every I/O seam — the Telegram client,
/// pane capture, and tmux injection — is injected, so the whole flow is
/// unit-testable with no network and no live tmux session (task 7).
pub struct SessionQaBridge {
    chat_id: String,
    client: Arc<dyn QaTelegramClient>,
    registry: Arc<PendingQuestions>,
    capture: CapturePaneFn,
    inject: InjectFn,
    follow_up: StdMutex<StdHashMap<ChatKey, ChatFollowUpState>>,
}

impl SessionQaBridge {
    /// Construct a bridge against the real Telegram API, real tmux capture,
    /// and real tmux injection.
    #[must_use]
    pub fn new(config: CodeSessionsBotConfig) -> Self {
        Self::with_seams(
            config.chat_id,
            Arc::new(HttpQaTelegramClient::new(config.bot_token)),
            Box::new(real_capture_pane),
            Box::new(real_inject),
        )
    }

    /// Construct a bridge with every I/O seam injected — the constructor
    /// task 7's hermetic tests use.
    #[must_use]
    pub fn with_seams(
        chat_id: String,
        client: Arc<dyn QaTelegramClient>,
        capture: CapturePaneFn,
        inject: InjectFn,
    ) -> Self {
        Self {
            chat_id,
            client,
            registry: Arc::new(PendingQuestions::new()),
            capture,
            inject,
            follow_up: StdMutex::new(StdHashMap::new()),
        }
    }

    /// Read-only access to the pending-questions registry, for tests that
    /// need to assert on it directly.
    #[must_use]
    pub fn registry(&self) -> &Arc<PendingQuestions> {
        &self.registry
    }

    // ── Inbound path: crossing → message ────────────────────────────────

    /// Handle one `BlockedEdgeRecord` from task 3's channel: capture that
    /// session's pane, parse it, and — on a genuine question — send exactly
    /// one Telegram message (never a second one for an already-pending
    /// identical question).
    pub async fn handle_edge_record(&self, record: BlockedEdgeRecord) {
        let pane = match (self.capture)(&record.session) {
            Ok(pane) => pane,
            Err(err) => {
                tracing::debug!(
                    session = %record.session,
                    error = %err,
                    "session-qa: pane capture failed for a Blocked crossing; dropping"
                );
                return;
            }
        };

        let Some(prompt) = parse_ask_question(&pane) else {
            // Not an AskUserQuestion pane (e.g. a permission dialog) — send
            // nothing, per the design note.
            return;
        };

        let outcome = self
            .registry
            .register(record.session.clone(), prompt.clone(), Utc::now());

        let RegisterOutcome::Created { question_id } = outcome else {
            // Already pending an identical question for this session — send
            // nothing, which is what makes "sent once" true.
            return;
        };

        let body = sendmessage_body(&question_id, &prompt, &self.chat_id);
        match self.client.send_message(body).await {
            Ok(resp) => {
                if let Some(message_id) = resp
                    .get("result")
                    .and_then(|r| r.get("message_id"))
                    .and_then(serde_json::Value::as_i64)
                {
                    self.registry.set_message_id(&question_id, message_id);
                } else {
                    tracing::debug!(
                        question_id,
                        "session-qa: sendMessage response carried no message_id"
                    );
                }
            }
            Err(err) => {
                tracing::warn!(
                    question_id,
                    session = %record.session,
                    error = %err,
                    "session-qa: sendMessage failed"
                );
            }
        }
    }

    /// Drive [`Self::handle_edge_record`] over every record `rx` yields,
    /// forever. Intended to be spawned once alongside the `getUpdates` loop.
    pub async fn run_inbound(&self, mut rx: mpsc::Receiver<BlockedEdgeRecord>) {
        while let Some(record) = rx.recv().await {
            self.handle_edge_record(record).await;
        }
    }

    // ── Outbound path: tap → injection ──────────────────────────────────

    /// Run one `getUpdates` long-poll loop, handling every update as it
    /// arrives, forever. Every failure path (HTTP error, rate limit,
    /// malformed update) is logged and the loop continues — no failure ever
    /// terminates it.
    pub async fn run_outbound(&self) {
        let mut cursor: Option<String> = None;
        loop {
            let mut query = Vec::new();
            if let Some(offset) = &cursor {
                query.push(("offset".to_string(), offset.clone()));
            }
            query.push((
                "timeout".to_string(),
                HttpQaTelegramClient::GETUPDATES_TIMEOUT_SECS.to_string(),
            ));

            let raw = match self.client.get_updates(&query).await {
                Ok(raw) => raw,
                Err(NotifyError::RateLimited { retry_after_secs }) => {
                    tracing::debug!(
                        retry_after_secs,
                        "session-qa: getUpdates rate limited; backing off"
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(retry_after_secs)).await;
                    continue;
                }
                Err(err) => {
                    tracing::warn!(error = %err, "session-qa: getUpdates failed; retrying");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
            };

            let Some(updates) = raw.get("result").and_then(serde_json::Value::as_array) else {
                tracing::warn!("session-qa: getUpdates response missing result array");
                continue;
            };

            for update in updates {
                if let Some(update_id) = update.get("update_id").and_then(serde_json::Value::as_i64)
                {
                    cursor = Some((update_id + 1).to_string());
                }
                self.handle_update(update).await;
            }
        }
    }

    /// Dispatch one raw Telegram update: a callback-query tap, or a
    /// plain-text message. Anything else (unrelated update types) is
    /// silently ignored.
    async fn handle_update(&self, update: &serde_json::Value) {
        if let Some(callback_query) = update.get("callback_query") {
            self.handle_callback_query(callback_query).await;
            return;
        }
        if let Some(message) = update.get("message") {
            self.handle_message(message).await;
        }
    }

    async fn handle_callback_query(&self, callback_query: &serde_json::Value) {
        let Some(callback_query_id) = callback_query.get("id").and_then(serde_json::Value::as_str)
        else {
            tracing::debug!("session-qa: callback_query missing id; cannot acknowledge, dropping");
            return;
        };

        let Some(data) = callback_query
            .get("data")
            .and_then(serde_json::Value::as_str)
        else {
            return;
        };
        let Some(decoded) = decode_question_callback(data) else {
            tracing::debug!("session-qa: malformed callback_data; dropping");
            return;
        };

        let chat_id = callback_query
            .get("message")
            .and_then(|m| m.get("chat"))
            .and_then(|c| c.get("id"))
            .map(|id| id.to_string());
        let message_id = callback_query
            .get("message")
            .and_then(|m| m.get("message_id"))
            .and_then(serde_json::Value::as_i64);

        let verdict = resolve_question_response(&decoded, &self.registry);

        // `answerCallbackQuery` FIRST, before any injection — matching the
        // ordering `ticket-telegram-answer-callback` established, for every
        // verdict including rejections.
        let ack_body = answercallbackquery_body(callback_query_id, &verdict);
        if let Err(err) = self.client.answer_callback_query(ack_body).await {
            tracing::warn!(error = %err, "session-qa: answerCallbackQuery failed");
        }

        match verdict {
            QuestionVerdict::Accepted {
                session,
                digit,
                kind,
                ..
            } => {
                // The keystroke sequence differs per `OptionKind`, verified
                // against a real AskUserQuestion widget — see
                // `ticket-session-qa-freetext-injection`'s spec:
                match kind {
                    OptionKind::Choice => {
                        // Digit moves the highlight, Enter selects. Unchanged.
                        if let Err(err) = (self.inject)(&session, &digit, true) {
                            tracing::warn!(
                                session = %session,
                                error = %err,
                                "session-qa: injection failed"
                            );
                        }
                    }
                    OptionKind::FreeText => {
                        // Digit WITHOUT Enter: selecting the option opens an
                        // inline editor without submitting. The follow-up
                        // relay (handle_message) later injects the typed
                        // text WITH Enter to submit it as the answer.
                        if let Err(err) = (self.inject)(&session, &digit, false) {
                            tracing::warn!(
                                session = %session,
                                error = %err,
                                "session-qa: injection failed"
                            );
                        }
                        if let Some(chat_id) = chat_id {
                            let state = self.take_follow_up_state(&chat_id).unwrap_or_default();
                            let (new_state, _outcome) =
                                state.on_escape_hatch_tap(decoded.question_id.clone(), kind);
                            self.set_follow_up_state(chat_id, new_state);
                        }
                    }
                    OptionKind::ChatAbout => {
                        // Digit WITH Enter: this closes the widget and
                        // returns to the normal session prompt. The
                        // follow-up relay (handle_message) later injects
                        // the operator's text as an ordinary turn.
                        if let Err(err) = (self.inject)(&session, &digit, true) {
                            tracing::warn!(
                                session = %session,
                                error = %err,
                                "session-qa: injection failed"
                            );
                        }
                        if let Some(chat_id) = chat_id {
                            let state = self.take_follow_up_state(&chat_id).unwrap_or_default();
                            let (new_state, _outcome) =
                                state.on_escape_hatch_tap(decoded.question_id.clone(), kind);
                            self.set_follow_up_state(chat_id, new_state);
                        }
                    }
                }

                self.registry.mark_answered(&decoded.question_id);

                if let Some(question) = self.registry.get(&decoded.question_id)
                    && let Some(message_id) = message_id
                {
                    let chosen_label = question
                        .prompt
                        .options
                        .iter()
                        .find(|opt| opt.number == decoded.option_number)
                        .map_or_else(|| digit.clone(), |opt| opt.label.clone());
                    let edit_body = editmessagetext_body(
                        &self.chat_id,
                        message_id,
                        &question.prompt.question,
                        &chosen_label,
                    );
                    if let Err(err) = self.client.edit_message_text(edit_body).await {
                        tracing::warn!(error = %err, "session-qa: editMessageText failed");
                    }
                }
            }
            QuestionVerdict::AlreadyAnswered | QuestionVerdict::UnknownQuestion => {
                // Acknowledged above with a distinguishing message; no
                // injection, no edit.
            }
        }
    }

    async fn handle_message(&self, message: &serde_json::Value) {
        let Some(text) = message.get("text").and_then(serde_json::Value::as_str) else {
            return;
        };
        let Some(chat_id) = message
            .get("chat")
            .and_then(|c| c.get("id"))
            .map(|id| id.to_string())
        else {
            return;
        };

        let Some(state) = self.take_follow_up_state(&chat_id) else {
            return;
        };
        let (new_state, outcome) = state.on_plain_text(text.to_string());
        self.set_follow_up_state(chat_id, new_state);

        if let FollowUpOutcome::RelayText {
            question_id, text, ..
        } = outcome
        {
            // Both `FreeText` and `ChatAbout` relay their text WITH Enter
            // here — they differ in what was injected at TAP time, not at
            // relay time. For `FreeText` this submits the inline editor
            // bound to the question; for `ChatAbout` it is an ordinary
            // prompt turn. Do not collapse this back into the tap-time
            // branch above; the two arms diverge there, not here.
            let Some(question) = self.registry.get(&question_id) else {
                tracing::debug!(
                    question_id,
                    "session-qa: relay for unknown question; dropping"
                );
                return;
            };
            if let Err(err) = (self.inject)(&question.session, &text, true) {
                tracing::warn!(
                    session = %question.session,
                    error = %err,
                    "session-qa: escape-hatch relay injection failed"
                );
                return;
            }
            self.registry.mark_answered(&question_id);
        }
    }

    fn take_follow_up_state(&self, chat_id: &str) -> Option<ChatFollowUpState> {
        self.follow_up
            .lock()
            .expect("session-qa follow_up mutex poisoned")
            .remove(chat_id)
    }

    fn set_follow_up_state(&self, chat_id: String, state: ChatFollowUpState) {
        if matches!(state, ChatFollowUpState::Idle) {
            self.follow_up
                .lock()
                .expect("session-qa follow_up mutex poisoned")
                .remove(&chat_id);
        } else {
            self.follow_up
                .lock()
                .expect("session-qa follow_up mutex poisoned")
                .insert(chat_id, state);
        }
    }
}

#[cfg(test)]
mod tests;
