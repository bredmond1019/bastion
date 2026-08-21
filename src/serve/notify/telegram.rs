//! Pure Telegram request construction from a `ValidatedOperatorPayload`
//! (`BA.18.B` task 2).
//!
//! This module only *builds* the JSON/query shapes Telegram's Bot API
//! expects — it performs no HTTP execution (the established
//! `sessions/tmux.rs` construction-vs-execution split: `*_args`/`*_body`/
//! `*_query` functions are pure and unit-tested directly; a thin I/O shell
//! calling them is added in a later task).
//!
//! Every function here consumes `engine_core::operator::ValidatedOperatorPayload`
//! — the `EN.8.A` contract — and never invents its own option/label/summary
//! shape. [`check_whatsapp_portability`] is a second, transport-side gate:
//! it re-checks the confirmed WhatsApp reply-buttons limits regardless of
//! what (possibly looser) `OperatorPayloadLimits` the payload was validated
//! against upstream, so a payload that would not survive the WhatsApp leg
//! is rejected here rather than shipping as a Telegram-only POC.

use std::time::Duration;

use async_trait::async_trait;
use engine_core::operator::{
    AckHandle, DeliveredMessage, NotifyError, OperatorPayloadLimits, OperatorResponse,
    OperatorTransport, ResponseVerdict, UpdateCursor, ValidatedOperatorPayload,
};

use crate::config::{BotToken, TelegramConfig};
use crate::serve::notify::telegram_http;

/// Number of leading hex characters of the payload's SHA-256 digest carried
/// in a Telegram `callback_data` string. A named const (never a magic
/// number) so the 64-byte `callback_data` ceiling stays provably respected
/// as gate ids/option keys grow, and so [`resolve_response`]-style callers
/// always compare *prefix to prefix*, never a truncated prefix against a
/// full digest (which would silently under-reject).
pub const CALLBACK_DIGEST_PREFIX_LEN: usize = 12;

/// Telegram's confirmed ceiling on `callback_data`, in bytes.
pub const CALLBACK_DATA_MAX_BYTES: usize = 64;

/// The delimiter joining `gate_id` / `digest_prefix` / `option_key` inside
/// an encoded `callback_data` string. Gate ids and option keys are
/// machine-chosen identifiers (never operator-supplied free text), so they
/// are not expected to contain this character; [`decode_callback_data`]
/// splits on exactly two of them and would misparse an id that did.
const CALLBACK_DATA_DELIMITER: char = '|';

/// The decoded contents of a Telegram `callback_data` string: which gate
/// the tap answers, the truncated digest prefix the payload was rendered
/// at, and which option the operator tapped.
///
/// `digest_prefix` is deliberately not the full digest — see
/// [`CALLBACK_DIGEST_PREFIX_LEN`]. Resolving a response against an
/// expected payload must compare `digest_prefix` to the *same* truncation
/// of the expected payload's digest, never to the expected payload's full
/// digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackData {
    /// The gate this response answers.
    pub gate_id: String,
    /// The first [`CALLBACK_DIGEST_PREFIX_LEN`] characters of the payload's
    /// digest at render time.
    pub digest_prefix: String,
    /// The stable machine key of the tapped option.
    pub option_key: String,
}

impl CallbackData {
    /// Build a `CallbackData`, truncating `digest` to the fixed prefix
    /// length so the constructed value is always the round-trippable form
    /// — there is no way to hold a `CallbackData` carrying an untruncated
    /// digest.
    #[must_use]
    pub fn new(gate_id: impl Into<String>, digest: &str, option_key: impl Into<String>) -> Self {
        Self {
            gate_id: gate_id.into(),
            digest_prefix: digest.chars().take(CALLBACK_DIGEST_PREFIX_LEN).collect(),
            option_key: option_key.into(),
        }
    }
}

/// Encode `data` into a Telegram `callback_data` string. Pure and
/// round-trippable via [`decode_callback_data`].
#[must_use]
pub fn encode_callback_data(data: &CallbackData) -> String {
    format!(
        "{gate}{d}{digest}{d}{option}",
        gate = data.gate_id,
        digest = data.digest_prefix,
        option = data.option_key,
        d = CALLBACK_DATA_DELIMITER,
    )
}

/// Decode a Telegram `callback_data` string back into its parts. `None` if
/// `raw` does not have exactly three delimiter-separated fields.
#[must_use]
pub fn decode_callback_data(raw: &str) -> Option<CallbackData> {
    let mut parts = raw.splitn(3, CALLBACK_DATA_DELIMITER);
    let gate_id = parts.next()?.to_string();
    let digest_prefix = parts.next()?.to_string();
    let option_key = parts.next()?.to_string();
    if parts.next().is_some() {
        // splitn(3, ..) can never yield a 4th item, but guard explicitly
        // rather than relying on that invariant silently.
        return None;
    }
    Some(CallbackData {
        gate_id,
        digest_prefix,
        option_key,
    })
}

/// Render a `sendMessage` request body: `rendered_summary` inline in
/// `text`, and one inline-keyboard button per declared option, in a single
/// row (Telegram, unlike WhatsApp, has no 3-button ceiling of its own, but
/// this transport only ever carries payloads already bounded to ≤3 options
/// by `EN.8.A` validation plus [`check_whatsapp_portability`]).
#[must_use]
pub fn sendmessage_body(payload: &ValidatedOperatorPayload, chat_id: &str) -> serde_json::Value {
    let p = payload.payload();
    let buttons: Vec<serde_json::Value> = p
        .options
        .iter()
        .map(|opt| {
            let callback_data =
                encode_callback_data(&CallbackData::new(&p.gate_id, &p.digest, &opt.key));
            serde_json::json!({
                "text": opt.label,
                "callback_data": callback_data,
            })
        })
        .collect();

    serde_json::json!({
        "chat_id": chat_id,
        "text": p.rendered_summary,
        "reply_markup": {
            "inline_keyboard": [buttons],
        },
    })
}

/// Reject `payload` before send if it would not survive WhatsApp's
/// confirmed limits (>3 options, >20-char label, >1024-char summary),
/// checked against `limits` regardless of what (possibly looser) limits the
/// payload was validated against upstream. Every rejection is
/// [`NotifyError::PayloadRejected`] — permanent, non-retryable — with a
/// distinct reason per violated limit.
pub fn check_whatsapp_portability(
    payload: &ValidatedOperatorPayload,
    limits: &OperatorPayloadLimits,
) -> Result<(), NotifyError> {
    let p = payload.payload();

    let option_count = p.options.len();
    if option_count > limits.max_options {
        return Err(NotifyError::PayloadRejected {
            reason: format!(
                "payload offers {option_count} options, exceeds WhatsApp's confirmed maximum of {}",
                limits.max_options
            ),
        });
    }

    for opt in &p.options {
        let label_chars = opt.label.chars().count();
        if label_chars > limits.max_label_chars {
            return Err(NotifyError::PayloadRejected {
                reason: format!(
                    "option '{}' label is {label_chars} characters, exceeds WhatsApp's confirmed maximum of {}",
                    opt.key, limits.max_label_chars
                ),
            });
        }
    }

    let summary_chars = p.rendered_summary.chars().count();
    if summary_chars > limits.max_summary_chars {
        return Err(NotifyError::PayloadRejected {
            reason: format!(
                "rendered summary is {summary_chars} characters, exceeds WhatsApp's confirmed maximum of {}",
                limits.max_summary_chars
            ),
        });
    }

    Ok(())
}

/// Build the `getUpdates` long-poll query. Never emits a webhook-related
/// parameter — this transport is long-polling only (`BA.18.B` non-negotiable
/// constraint 2).
#[must_use]
pub fn getupdates_query(cursor: Option<UpdateCursor>, timeout_secs: u64) -> Vec<(String, String)> {
    let mut query = Vec::new();
    if let Some(UpdateCursor(offset)) = cursor {
        query.push(("offset".to_string(), offset));
    }
    query.push(("timeout".to_string(), timeout_secs.to_string()));
    query.push((
        "allowed_updates".to_string(),
        r#"["callback_query"]"#.to_string(),
    ));
    query
}

/// Parse a raw `getUpdates` response body into the operator responses it
/// carries and the cursor to resume from on the next poll (`BA.18.B` task 3).
///
/// - The envelope must be `{"ok": true, "result": [...]}`; anything else is
///   [`NotifyError::Malformed`] for the whole batch.
/// - Within `result`, an individual update that is missing `update_id`, has
///   no `callback_query`, or whose `callback_query.data` does not decode via
///   [`decode_callback_data`] is **skipped**, not fatal — the rest of the
///   batch is still returned.
/// - The `callback_query`'s `id`, when present, is captured into the
///   returned response's `ack` field as an opaque [`AckHandle`]. A
///   `callback_query` with no `id` still yields a response with `ack: None`
///   — it is not skipped, since losing a real decision because it cannot
///   be acknowledged is strictly worse than not acknowledging it.
/// - The `callback_query`'s `message.message_id` and `message.chat.id`,
///   when both present and well-typed, are captured into the returned
///   response's `message` field as an opaque
///   [`MessageHandle`](crate::serve::notify::MessageHandle) — the same
///   "carry it, never drop the response for lacking it" treatment as `ack`.
/// - The returned cursor is `max(update_id) + 1` across every update that
///   *did* carry a parseable `update_id`, whether or not it produced an
///   `OperatorResponse` — an update with no callback tap still consumed an
///   offset slot and must be acknowledged, or `getUpdates` redelivers it
///   forever.
/// - An empty (or entirely update-id-less) batch returns `None` for the
///   cursor. Callers must treat `None` as "unchanged" and keep polling from
///   whatever cursor they already had — never reset to the start of the
///   backlog.
pub fn parse_updates(
    raw: &serde_json::Value,
) -> Result<(Vec<OperatorResponse>, Option<UpdateCursor>), NotifyError> {
    let ok = raw.get("ok").and_then(serde_json::Value::as_bool);
    let result = raw.get("result").and_then(serde_json::Value::as_array);

    let (Some(true), Some(result)) = (ok, result) else {
        return Err(NotifyError::Malformed {
            reason: "getUpdates response is not a {ok: true, result: [...]} envelope".to_string(),
        });
    };

    let mut responses = Vec::new();
    let mut max_update_id: Option<i64> = None;

    for update in result {
        let Some(update_id) = update.get("update_id").and_then(serde_json::Value::as_i64) else {
            // No usable update_id at all: cannot contribute to the cursor
            // and cannot be resolved to a response either. Skip.
            continue;
        };
        max_update_id = Some(max_update_id.map_or(update_id, |current| current.max(update_id)));

        let Some(callback_query) = update.get("callback_query") else {
            continue;
        };

        let Some(data) = callback_query
            .get("data")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };

        let Some(decoded) = decode_callback_data(data) else {
            continue;
        };

        // A callback_query missing an `id` still yields a response (with
        // `ack: None`) rather than being dropped — see this module's docs
        // on `parse_updates` and `OperatorResponse::ack`.
        let ack = callback_query
            .get("id")
            .and_then(serde_json::Value::as_str)
            .map(|id| AckHandle(id.to_string()));

        // The message location (chat_id + message_id) is captured the same
        // opaque way as the ack handle: present when both halves are
        // present and well-typed, `None` otherwise. A response with no
        // resolvable message location simply cannot have its original
        // message edited later — it is still returned rather than dropped.
        let message = callback_query.get("message").and_then(|message| {
            let message_id = message
                .get("message_id")
                .and_then(serde_json::Value::as_i64)?;
            let chat_id = message
                .get("chat")
                .and_then(|chat| chat.get("id"))
                .and_then(serde_json::Value::as_i64)?;
            Some(crate::serve::notify::MessageHandle {
                chat_id: chat_id.to_string(),
                message_id,
            })
        });

        responses.push(OperatorResponse {
            gate_id: decoded.gate_id,
            digest: decoded.digest_prefix,
            option_key: decoded.option_key,
            received_at: chrono::Utc::now(),
            ack,
            message,
        });
    }

    let cursor = max_update_id.map(|id| UpdateCursor((id + 1).to_string()));
    Ok((responses, cursor))
}

/// Resolve `resp` against the payload it is expected to answer.
///
/// `resp.digest` is already the truncated prefix a transport observed (see
/// [`CALLBACK_DIGEST_PREFIX_LEN`]), so `expected`'s full digest is truncated
/// to the same prefix length before comparing — never compared against the
/// full digest, which would spuriously reject every real response.
#[must_use]
pub fn resolve_response(
    resp: &OperatorResponse,
    expected: &ValidatedOperatorPayload,
) -> ResponseVerdict {
    let expected_payload = expected.payload();

    if resp.gate_id != expected_payload.gate_id {
        return ResponseVerdict::UnknownGate;
    }

    let expected_prefix: String = expected_payload
        .digest
        .chars()
        .take(CALLBACK_DIGEST_PREFIX_LEN)
        .collect();

    if resp.digest != expected_prefix {
        return ResponseVerdict::StaleDigest {
            gate_id: resp.gate_id.clone(),
            option_key: resp.option_key.clone(),
            digest: resp.digest.clone(),
            decided_at: resp.received_at,
        };
    }

    ResponseVerdict::Accepted {
        gate_id: resp.gate_id.clone(),
        option_key: resp.option_key.clone(),
        digest: resp.digest.clone(),
        decided_at: resp.received_at,
    }
}

/// Telegram's confirmed ceiling on an `answerCallbackQuery` `text` field, in
/// characters (not bytes) — see the Bot API docs' 200-character limit on
/// this parameter.
pub const ANSWER_CALLBACK_TEXT_MAX_CHARS: usize = 200;

/// Truncate `text` to at most `max_chars` **characters**, never bytes — a
/// byte-length clamp could split a multi-byte character and produce
/// invalid UTF-8 or a mangled glyph. Pure.
fn clamp_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        text.chars().take(max_chars).collect()
    }
}

/// Render an `answerCallbackQuery` request body acknowledging `verdict` for
/// the callback identified by `ack`. Every verdict arm gets distinct,
/// non-empty text so the operator can tell a registered decision from a
/// stale or already-answered tap — silence is the defect this exists to
/// fix (`ticket-telegram-answer-callback`). Pure — no I/O.
///
/// Text is clamped to [`ANSWER_CALLBACK_TEXT_MAX_CHARS`] characters
/// (counted by `.chars().count()`, never bytes) to respect Telegram's
/// confirmed ceiling on this field.
#[must_use]
pub fn answercallbackquery_body(ack: &AckHandle, verdict: &ResponseVerdict) -> serde_json::Value {
    let text = match verdict {
        ResponseVerdict::Accepted { option_key, .. } => {
            format!("Recorded: {option_key}")
        }
        ResponseVerdict::StaleDigest { .. } => {
            "This question changed - no longer valid".to_string()
        }
        ResponseVerdict::UnknownGate => "Already answered or no longer pending".to_string(),
    };

    serde_json::json!({
        "callback_query_id": ack.0,
        "text": clamp_chars(&text, ANSWER_CALLBACK_TEXT_MAX_CHARS),
    })
}

/// Render an `editMessageText` request body that replaces the original
/// prompt's text with `summary` plus which option (`chosen`) was taken, and
/// **clears** `reply_markup` (an empty `inline_keyboard`, not an absent
/// field) so the live buttons are gone from the operator's view. Pure — no
/// I/O.
///
/// An absent `reply_markup` leaves Telegram's prior keyboard in place —
/// that is exactly the bug (`ticket-telegram-answer-callback`) this
/// function exists to prevent, so callers must never omit this field.
#[must_use]
pub fn editmessagetext_body(
    chat_id: &str,
    message_id: i64,
    summary: &str,
    chosen: &str,
) -> serde_json::Value {
    serde_json::json!({
        "chat_id": chat_id,
        "message_id": message_id,
        "text": format!("{summary}\n\nDecision: {chosen}"),
        "reply_markup": {
            "inline_keyboard": [],
        },
    })
}

// ── Thin HTTP shell (BA.18.B task 4) ────────────────────────────────────────
//
// Everything above this section is pure (construction/parsing). Everything
// below is the thin I/O shell that calls it — the established
// `sessions/tmux.rs` construction-vs-execution split. Per `CLAUDE.md` rule
// 6, this shell is manually smoke-tested rather than unit-tested end to
// end; the smoke-test recipe is recorded in `planning/BA.18.B/tasks.md`'s
// `## Notes` by task 7 as an operator-run gate — the live bot token is never
// obtained, read, or handled by an agent (non-negotiable constraint 3).

/// Thin `reqwest`-backed [`OperatorTransport`] over the Telegram Bot API.
///
/// Holds the bot token as a [`BotToken`] (never `Debug`-printable) and the
/// destination `chat_id`. The token is interpolated into the request URL
/// path only at the point of the actual HTTP call — this struct's own
/// `Debug` impl never renders it (derived `Debug` would recurse into
/// `BotToken`'s redacted impl, so deriving here is safe).
#[derive(Debug, Clone)]
pub struct TelegramTransport {
    bot_token: BotToken,
    chat_id: String,
    client: reqwest::Client,
}

impl TelegramTransport {
    /// Long-poll timeout Telegram is asked to hold the connection open for.
    const GETUPDATES_TIMEOUT_SECS: u64 = 30;
    /// Client-side request timeout — must exceed the long-poll timeout Telegram
    /// itself waits on, or every `getUpdates` call would time out on our side
    /// before Telegram ever responds.
    const CLIENT_TIMEOUT_SECS: u64 = Self::GETUPDATES_TIMEOUT_SECS + 10;

    #[must_use]
    pub fn new(config: TelegramConfig) -> Self {
        Self {
            bot_token: config.bot_token,
            chat_id: config.chat_id,
            client: reqwest::Client::new(),
        }
    }

    /// Build the base API URL for `method` (e.g. `"sendMessage"`,
    /// `"getUpdates"`). The token lives in this URL's path — callers must
    /// never log the returned string; log the method name alone instead
    /// (non-negotiable constraint 3).
    fn api_url(&self, method: &str) -> String {
        telegram_http::api_url(&self.bot_token, method)
    }
}

#[async_trait]
impl OperatorTransport for TelegramTransport {
    async fn send(
        &self,
        payload: &ValidatedOperatorPayload,
    ) -> Result<DeliveredMessage, NotifyError> {
        check_whatsapp_portability(payload, &OperatorPayloadLimits::default())?;

        let body = sendmessage_body(payload, &self.chat_id);
        let method = "sendMessage";
        tracing::debug!(method, "calling Telegram Bot API");

        let resp = self
            .client
            .post(self.api_url(method))
            .timeout(Duration::from_secs(Self::CLIENT_TIMEOUT_SECS))
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

        let parsed: serde_json::Value = resp.json().await.map_err(|e| NotifyError::Malformed {
            reason: format!("{method} response body was not valid JSON: {e}"),
        })?;
        let message_id = parsed
            .get("result")
            .and_then(|r| r.get("message_id"))
            .and_then(serde_json::Value::as_i64)
            .map(|id| id.to_string())
            .unwrap_or_default();

        Ok(DeliveredMessage {
            transport_message_id: message_id,
        })
    }

    async fn poll_responses(
        &self,
        since: Option<UpdateCursor>,
    ) -> Result<(Vec<OperatorResponse>, Option<UpdateCursor>), NotifyError> {
        let method = "getUpdates";
        tracing::debug!(method, "calling Telegram Bot API");

        let query = getupdates_query(since, Self::GETUPDATES_TIMEOUT_SECS);
        let url = telegram_http::append_query_string(&self.api_url(method), &query);
        let resp = self
            .client
            .get(url)
            .timeout(Duration::from_secs(Self::CLIENT_TIMEOUT_SECS))
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

        let raw: serde_json::Value = resp.json().await.map_err(|e| NotifyError::Malformed {
            reason: format!("{method} response body was not valid JSON: {e}"),
        })?;
        parse_updates(&raw)
    }

    async fn acknowledge(
        &self,
        response: &OperatorResponse,
        verdict: &ResponseVerdict,
    ) -> Result<(), NotifyError> {
        if let Some(ack) = &response.ack {
            let method = "answerCallbackQuery";
            tracing::debug!(
                method,
                verdict = verdict_label(verdict),
                "calling Telegram Bot API"
            );

            let body = answercallbackquery_body(ack, verdict);
            let resp = self
                .client
                .post(self.api_url(method))
                .timeout(Duration::from_secs(Self::CLIENT_TIMEOUT_SECS))
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
        }

        // The edit is best-effort: it drops the live buttons and shows the
        // decision, but losing it is a cosmetic regression, not a lost
        // acknowledgement — a failure here is logged and swallowed, never
        // returned. The original rendered summary is not available at this
        // seam (`OperatorResponse` carries no payload text), so the edited
        // message names the gate and the outcome rather than repeating the
        // full original prompt.
        if let Some(message) = &response.message {
            let chosen = match verdict {
                ResponseVerdict::Accepted { option_key, .. } => option_key.clone(),
                ResponseVerdict::StaleDigest { .. } => "expired (question changed)".to_string(),
                ResponseVerdict::UnknownGate => "already answered or no longer pending".to_string(),
            };
            let summary = format!("Gate {}", response.gate_id);
            let method = "editMessageText";
            tracing::debug!(
                method,
                verdict = verdict_label(verdict),
                "calling Telegram Bot API"
            );

            let body =
                editmessagetext_body(&message.chat_id, message.message_id, &summary, &chosen);
            let edit_result = self
                .client
                .post(self.api_url(method))
                .timeout(Duration::from_secs(Self::CLIENT_TIMEOUT_SECS))
                .json(&body)
                .send()
                .await;

            match edit_result {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if let Some(err) = telegram_http::classify_http_status(
                        status,
                        telegram_http::retry_after_from_response(&resp),
                    ) {
                        tracing::warn!(
                            method,
                            error = %err,
                            "operator message edit failed"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        method,
                        error = telegram_http::classify_reqwest_error(&e),
                        "operator message edit request failed"
                    );
                }
            }
        }

        Ok(())
    }
}

/// Short, non-sensitive label for a [`ResponseVerdict`], for log fields —
/// never the full verdict `Debug` rendering, which could carry an option
/// key an operator considers sensitive in some future payload.
fn verdict_label(verdict: &ResponseVerdict) -> &'static str {
    match verdict {
        ResponseVerdict::Accepted { .. } => "accepted",
        ResponseVerdict::StaleDigest { .. } => "stale_digest",
        ResponseVerdict::UnknownGate => "unknown_gate",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::operator::{OperatorPayload, OperatorResponseOption, validate};

    fn approve_reject() -> Vec<OperatorResponseOption> {
        vec![
            OperatorResponseOption::new("approve", "Approve"),
            OperatorResponseOption::new("reject", "Reject"),
        ]
    }

    fn validated(
        gate_id: &str,
        summary: &str,
        options: Vec<OperatorResponseOption>,
    ) -> ValidatedOperatorPayload {
        let payload = OperatorPayload::new(gate_id, summary, options);
        validate(payload, &OperatorPayloadLimits::default()).expect("payload should validate")
    }

    // -- callback_data round-trip -------------------------------------

    #[test]
    fn callback_data_round_trips() {
        let data = CallbackData::new("gate-1", "0123456789abcdef", "approve");
        let encoded = encode_callback_data(&data);
        assert_eq!(decode_callback_data(&encoded), Some(data));
    }

    #[test]
    fn callback_data_round_trips_for_every_option_in_a_payload() {
        let payload = validated("gate-42", "diff summary", approve_reject());
        let p = payload.payload();
        for opt in &p.options {
            let data = CallbackData::new(&p.gate_id, &p.digest, &opt.key);
            let encoded = encode_callback_data(&data);
            assert_eq!(decode_callback_data(&encoded), Some(data));
        }
    }

    #[test]
    fn callback_data_truncates_digest_to_named_prefix_length() {
        let full_digest = "a".repeat(64); // sha256 hex is 64 chars
        let data = CallbackData::new("gate-1", &full_digest, "approve");
        assert_eq!(data.digest_prefix.len(), CALLBACK_DIGEST_PREFIX_LEN);
        assert_ne!(data.digest_prefix, full_digest);
    }

    #[test]
    fn callback_data_prefix_never_compares_equal_to_a_different_full_digest_by_accident() {
        // The whole point of comparing prefix-to-prefix: two payloads whose
        // digests share the same first N characters must not be able to
        // collide through the *full* digest. Encoding never stores the full
        // digest, so a decode can only ever be compared to another prefix.
        let short = CallbackData::new("gate-1", "abcdefabcdef", "approve");
        let long = CallbackData::new("gate-1", "abcdefabcdefXXXXX", "approve");
        assert_eq!(short.digest_prefix, long.digest_prefix);
    }

    #[test]
    fn encoded_callback_data_never_exceeds_telegram_ceiling_for_typical_ids() {
        let long_gate_id = "a".repeat(20);
        let long_option_key = "b".repeat(20);
        let full_digest = "c".repeat(64);
        let data = CallbackData::new(&long_gate_id, &full_digest, &long_option_key);
        let encoded = encode_callback_data(&data);
        assert!(
            encoded.len() <= CALLBACK_DATA_MAX_BYTES,
            "encoded callback_data was {} bytes, exceeds the {}-byte ceiling: {encoded:?}",
            encoded.len(),
            CALLBACK_DATA_MAX_BYTES,
        );
    }

    #[test]
    fn decode_rejects_malformed_input() {
        assert_eq!(decode_callback_data("only-one-field"), None);
        assert_eq!(decode_callback_data(""), None);
    }

    // -- sendmessage_body -----------------------------------------------

    #[test]
    fn sendmessage_body_contains_full_summary_inline() {
        let payload = validated("gate-1", "the full diff text, verbatim", approve_reject());
        let body = sendmessage_body(&payload, "12345");
        assert_eq!(body["text"], "the full diff text, verbatim");
        assert_eq!(body["chat_id"], "12345");
    }

    #[test]
    fn sendmessage_body_has_exactly_one_button_per_option() {
        let payload = validated("gate-1", "summary", approve_reject());
        let body = sendmessage_body(&payload, "12345");
        let rows = body["reply_markup"]["inline_keyboard"]
            .as_array()
            .expect("inline_keyboard is an array");
        assert_eq!(rows.len(), 1, "expected a single row");
        let buttons = rows[0].as_array().expect("row is an array");
        assert_eq!(buttons.len(), 2);
        assert_eq!(buttons[0]["text"], "Approve");
        assert_eq!(buttons[1]["text"], "Reject");
    }

    #[test]
    fn sendmessage_body_button_callback_data_round_trips_to_correct_option() {
        let payload = validated("gate-7", "summary", approve_reject());
        let body = sendmessage_body(&payload, "12345");
        let buttons = body["reply_markup"]["inline_keyboard"][0]
            .as_array()
            .unwrap();
        let raw = buttons[0]["callback_data"].as_str().unwrap();
        let decoded = decode_callback_data(raw).expect("callback_data decodes");
        assert_eq!(decoded.gate_id, "gate-7");
        assert_eq!(decoded.option_key, "approve");
    }

    #[test]
    fn sendmessage_body_supports_three_options() {
        let options = vec![
            OperatorResponseOption::new("a", "A"),
            OperatorResponseOption::new("b", "B"),
            OperatorResponseOption::new("c", "C"),
        ];
        let payload = validated("gate-1", "summary", options);
        let body = sendmessage_body(&payload, "12345");
        let buttons = body["reply_markup"]["inline_keyboard"][0]
            .as_array()
            .unwrap();
        assert_eq!(buttons.len(), 3);
    }

    // -- check_whatsapp_portability ---------------------------------------

    #[test]
    fn portability_accepts_exactly_two_and_three_options() {
        let limits = OperatorPayloadLimits::default();
        let two = validated("gate-1", "summary", approve_reject());
        assert!(check_whatsapp_portability(&two, &limits).is_ok());

        let three_options = vec![
            OperatorResponseOption::new("a", "A"),
            OperatorResponseOption::new("b", "B"),
            OperatorResponseOption::new("c", "C"),
        ];
        let three = validated("gate-1", "summary", three_options);
        assert!(check_whatsapp_portability(&three, &limits).is_ok());
    }

    #[test]
    fn portability_rejects_more_than_three_options() {
        // Validate under a looser limit set so the payload can even be
        // constructed with 4 options, then check portability separately.
        let loose = OperatorPayloadLimits {
            max_options: 10,
            min_options: 1,
            max_label_chars: 60,
            max_summary_chars: 4096,
        };
        let options = vec![
            OperatorResponseOption::new("a", "A"),
            OperatorResponseOption::new("b", "B"),
            OperatorResponseOption::new("c", "C"),
            OperatorResponseOption::new("d", "D"),
        ];
        let payload = OperatorPayload::new("gate-1", "summary", options);
        let validated_payload = validate(payload, &loose).expect("validates under loose limits");

        let result =
            check_whatsapp_portability(&validated_payload, &OperatorPayloadLimits::default());
        assert!(matches!(result, Err(NotifyError::PayloadRejected { .. })));
        assert!(!result.unwrap_err().is_retryable());
    }

    #[test]
    fn portability_accepts_label_at_exactly_20_chars_rejects_21() {
        let loose = OperatorPayloadLimits {
            max_options: 3,
            min_options: 2,
            max_label_chars: 60,
            max_summary_chars: 4096,
        };

        let label_20 = "x".repeat(20);
        let ok_options = vec![
            OperatorResponseOption::new("a", label_20.clone()),
            OperatorResponseOption::new("b", "B"),
        ];
        let ok_payload = validate(
            OperatorPayload::new("gate-1", "summary", ok_options),
            &loose,
        )
        .expect("validates under loose limits");
        assert!(check_whatsapp_portability(&ok_payload, &OperatorPayloadLimits::default()).is_ok());

        let label_21 = "x".repeat(21);
        let bad_options = vec![
            OperatorResponseOption::new("a", label_21),
            OperatorResponseOption::new("b", "B"),
        ];
        let bad_payload = validate(
            OperatorPayload::new("gate-1", "summary", bad_options),
            &loose,
        )
        .expect("validates under loose limits");
        let result = check_whatsapp_portability(&bad_payload, &OperatorPayloadLimits::default());
        assert!(matches!(result, Err(NotifyError::PayloadRejected { .. })));
    }

    #[test]
    fn portability_accepts_summary_at_exactly_1024_chars_rejects_1025() {
        let loose = OperatorPayloadLimits {
            max_options: 3,
            min_options: 2,
            max_label_chars: 20,
            max_summary_chars: 4096,
        };

        let summary_1024 = "x".repeat(1024);
        let ok_payload = validate(
            OperatorPayload::new("gate-1", summary_1024, approve_reject()),
            &loose,
        )
        .expect("validates under loose limits");
        assert!(check_whatsapp_portability(&ok_payload, &OperatorPayloadLimits::default()).is_ok());

        let summary_1025 = "x".repeat(1025);
        let bad_payload = validate(
            OperatorPayload::new("gate-1", summary_1025, approve_reject()),
            &loose,
        )
        .expect("validates under loose limits");
        let result = check_whatsapp_portability(&bad_payload, &OperatorPayloadLimits::default());
        assert!(matches!(result, Err(NotifyError::PayloadRejected { .. })));
    }

    #[test]
    fn portability_rejections_are_distinct_reasons_per_limit() {
        let limits = OperatorPayloadLimits::default();
        let loose = OperatorPayloadLimits {
            max_options: 10,
            min_options: 1,
            max_label_chars: 200,
            max_summary_chars: 4096,
        };

        let too_many = validate(
            OperatorPayload::new(
                "gate-1",
                "summary",
                vec![
                    OperatorResponseOption::new("a", "A"),
                    OperatorResponseOption::new("b", "B"),
                    OperatorResponseOption::new("c", "C"),
                    OperatorResponseOption::new("d", "D"),
                ],
            ),
            &loose,
        )
        .unwrap();
        let too_long_label = validate(
            OperatorPayload::new(
                "gate-1",
                "summary",
                vec![
                    OperatorResponseOption::new("a", "x".repeat(21)),
                    OperatorResponseOption::new("b", "B"),
                ],
            ),
            &loose,
        )
        .unwrap();
        let too_long_summary = validate(
            OperatorPayload::new("gate-1", "x".repeat(1025), approve_reject()),
            &loose,
        )
        .unwrap();

        let r1 = check_whatsapp_portability(&too_many, &limits).unwrap_err();
        let r2 = check_whatsapp_portability(&too_long_label, &limits).unwrap_err();
        let r3 = check_whatsapp_portability(&too_long_summary, &limits).unwrap_err();

        let (
            NotifyError::PayloadRejected { reason: reason1 },
            NotifyError::PayloadRejected { reason: reason2 },
            NotifyError::PayloadRejected { reason: reason3 },
        ) = (&r1, &r2, &r3)
        else {
            panic!("expected all three to be PayloadRejected");
        };
        assert_ne!(reason1, reason2);
        assert_ne!(reason2, reason3);
        assert_ne!(reason1, reason3);
    }

    // -- getupdates_query -------------------------------------------------

    #[test]
    fn getupdates_query_without_cursor() {
        let query = getupdates_query(None, 30);
        assert_eq!(
            query,
            vec![
                ("timeout".to_string(), "30".to_string()),
                (
                    "allowed_updates".to_string(),
                    r#"["callback_query"]"#.to_string()
                ),
            ]
        );
    }

    #[test]
    fn getupdates_query_with_cursor_includes_offset() {
        let query = getupdates_query(Some(UpdateCursor("42".to_string())), 30);
        assert_eq!(
            query,
            vec![
                ("offset".to_string(), "42".to_string()),
                ("timeout".to_string(), "30".to_string()),
                (
                    "allowed_updates".to_string(),
                    r#"["callback_query"]"#.to_string()
                ),
            ]
        );
    }

    #[test]
    fn getupdates_query_never_emits_webhook_params() {
        let query = getupdates_query(Some(UpdateCursor("1".to_string())), 30);
        for (key, _) in &query {
            assert!(
                !key.to_lowercase().contains("webhook"),
                "getupdates_query must never emit a webhook-related parameter"
            );
        }
    }

    // -- parse_updates ------------------------------------------------------
    //
    // Every fixture below is hand-built JSON with no real token and no real
    // chat id anywhere — `chat_id`/`from.id` values, where present, are
    // fabricated small integers, never a live credential.

    #[test]
    fn parse_updates_empty_result_leaves_cursor_unchanged() {
        let raw = serde_json::json!({"ok": true, "result": []});
        let (responses, cursor) = parse_updates(&raw).expect("valid envelope");
        assert!(responses.is_empty());
        assert_eq!(
            cursor, None,
            "an empty batch must return None (unchanged), never a reset cursor"
        );
    }

    #[test]
    fn parse_updates_single_callback_query() {
        let raw = serde_json::json!({
            "ok": true,
            "result": [{
                "update_id": 100,
                "callback_query": {
                    "id": "cbq-1",
                    "from": {"id": 555},
                    "data": encode_callback_data(&CallbackData::new("gate-1", "abcdef012345", "approve")),
                }
            }]
        });
        let (responses, cursor) = parse_updates(&raw).expect("valid envelope");
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].gate_id, "gate-1");
        assert_eq!(responses[0].digest, "abcdef012345");
        assert_eq!(responses[0].option_key, "approve");
        assert_eq!(cursor, Some(UpdateCursor("101".to_string())));
    }

    #[test]
    fn parse_updates_callback_query_id_round_trips_into_ack_handle() {
        let raw = serde_json::json!({
            "ok": true,
            "result": [{
                "update_id": 100,
                "callback_query": {
                    "id": "cbq-42",
                    "from": {"id": 555},
                    "data": encode_callback_data(&CallbackData::new("gate-1", "abcdef012345", "approve")),
                }
            }]
        });
        let (responses, _cursor) = parse_updates(&raw).expect("valid envelope");
        assert_eq!(responses.len(), 1);
        assert_eq!(
            responses[0].ack,
            Some(crate::serve::notify::AckHandle("cbq-42".to_string())),
            "the callback_query's id must round-trip into OperatorResponse.ack"
        );
    }

    #[test]
    fn parse_updates_callback_query_missing_id_yields_ack_none_not_a_dropped_response() {
        let raw = serde_json::json!({
            "ok": true,
            "result": [{
                "update_id": 100,
                "callback_query": {
                    "from": {"id": 555},
                    "data": encode_callback_data(&CallbackData::new("gate-1", "abcdef012345", "approve")),
                }
            }]
        });
        let (responses, cursor) = parse_updates(&raw).expect("valid envelope");
        assert_eq!(
            responses.len(),
            1,
            "a callback_query with no id must still be parsed into a response, not skipped"
        );
        assert_eq!(responses[0].gate_id, "gate-1");
        assert_eq!(responses[0].ack, None);
        assert_eq!(cursor, Some(UpdateCursor("101".to_string())));
    }

    #[test]
    fn parse_updates_captures_message_location_into_message_handle() {
        let raw = serde_json::json!({
            "ok": true,
            "result": [{
                "update_id": 100,
                "callback_query": {
                    "id": "cbq-1",
                    "from": {"id": 555},
                    "data": encode_callback_data(&CallbackData::new("gate-1", "abcdef012345", "approve")),
                    "message": {
                        "message_id": 999,
                        "chat": {"id": 42},
                    },
                }
            }]
        });
        let (responses, _cursor) = parse_updates(&raw).expect("valid envelope");
        assert_eq!(responses.len(), 1);
        assert_eq!(
            responses[0].message,
            Some(crate::serve::notify::MessageHandle {
                chat_id: "42".to_string(),
                message_id: 999,
            }),
            "message.message_id and message.chat.id must round-trip into OperatorResponse.message"
        );
    }

    #[test]
    fn parse_updates_missing_message_yields_message_handle_none_not_a_dropped_response() {
        let raw = serde_json::json!({
            "ok": true,
            "result": [{
                "update_id": 100,
                "callback_query": {
                    "id": "cbq-1",
                    "from": {"id": 555},
                    "data": encode_callback_data(&CallbackData::new("gate-1", "abcdef012345", "approve")),
                }
            }]
        });
        let (responses, _cursor) = parse_updates(&raw).expect("valid envelope");
        assert_eq!(
            responses.len(),
            1,
            "a callback_query with no message must still be parsed into a response, not skipped"
        );
        assert_eq!(responses[0].message, None);
    }

    #[test]
    fn parse_updates_message_missing_message_id_yields_message_handle_none() {
        let raw = serde_json::json!({
            "ok": true,
            "result": [{
                "update_id": 100,
                "callback_query": {
                    "id": "cbq-1",
                    "data": encode_callback_data(&CallbackData::new("gate-1", "abcdef012345", "approve")),
                    "message": {
                        "chat": {"id": 42},
                    },
                }
            }]
        });
        let (responses, _cursor) = parse_updates(&raw).expect("valid envelope");
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].message, None);
    }

    #[test]
    fn parse_updates_multiple_updates_cursor_is_max_plus_one() {
        let raw = serde_json::json!({
            "ok": true,
            "result": [
                {
                    "update_id": 100,
                    "callback_query": {
                        "data": encode_callback_data(&CallbackData::new("gate-1", "abcdef012345", "approve")),
                    }
                },
                {
                    "update_id": 105,
                    "callback_query": {
                        "data": encode_callback_data(&CallbackData::new("gate-2", "fedcba543210", "reject")),
                    }
                },
                {
                    "update_id": 103,
                    "callback_query": {
                        "data": encode_callback_data(&CallbackData::new("gate-3", "111111111111", "approve")),
                    }
                },
            ]
        });
        let (responses, cursor) = parse_updates(&raw).expect("valid envelope");
        assert_eq!(responses.len(), 3);
        assert_eq!(
            cursor,
            Some(UpdateCursor("106".to_string())),
            "cursor must be max(update_id) + 1, not the last-seen update_id + 1"
        );
    }

    #[test]
    fn parse_updates_malformed_update_does_not_discard_valid_ones() {
        let raw = serde_json::json!({
            "ok": true,
            "result": [
                {
                    "update_id": 200,
                    "callback_query": {
                        "data": encode_callback_data(&CallbackData::new("gate-1", "abcdef012345", "approve")),
                    }
                },
                {
                    "update_id": 201,
                    "callback_query": {
                        "data": "not-a-valid-callback-data-string",
                    }
                },
                {
                    "update_id": 202,
                    "message": {"text": "not a callback query at all"}
                },
                {
                    "update_id": 203,
                    "callback_query": {
                        "data": encode_callback_data(&CallbackData::new("gate-4", "222222222222", "reject")),
                    }
                },
            ]
        });
        let (responses, cursor) = parse_updates(&raw).expect("valid envelope");
        assert_eq!(
            responses.len(),
            2,
            "the two malformed updates (201, 202) must be skipped, not fail the batch"
        );
        assert_eq!(responses[0].gate_id, "gate-1");
        assert_eq!(responses[1].gate_id, "gate-4");
        assert_eq!(
            cursor,
            Some(UpdateCursor("204".to_string())),
            "malformed updates still consumed an update_id and must count toward the cursor"
        );
    }

    #[test]
    fn parse_updates_non_ok_envelope_is_malformed() {
        let raw = serde_json::json!({"ok": false, "description": "Unauthorized"});
        let result = parse_updates(&raw);
        assert!(matches!(result, Err(NotifyError::Malformed { .. })));
    }

    #[test]
    fn parse_updates_missing_result_is_malformed() {
        let raw = serde_json::json!({"ok": true});
        let result = parse_updates(&raw);
        assert!(matches!(result, Err(NotifyError::Malformed { .. })));
    }

    #[test]
    fn parse_updates_malformed_error_never_contains_a_token_shaped_value() {
        let raw = serde_json::json!({"not": "an envelope at all"});
        let err = parse_updates(&raw).unwrap_err();
        let rendered = format!("{err}");
        assert!(!rendered.to_lowercase().contains("token"));
    }

    // -- resolve_response -----------------------------------------------

    fn expected_payload() -> ValidatedOperatorPayload {
        validated("gate-1", "the rendered diff", approve_reject())
    }

    fn response_for(gate_id: &str, digest: &str, option_key: &str) -> OperatorResponse {
        OperatorResponse {
            gate_id: gate_id.to_string(),
            digest: digest.to_string(),
            option_key: option_key.to_string(),
            received_at: chrono::Utc::now(),
            ack: None,
            message: None,
        }
    }

    #[test]
    fn resolve_response_accepted_when_gate_and_digest_prefix_match() {
        let payload = expected_payload();
        let p = payload.payload();
        let prefix: String = p.digest.chars().take(CALLBACK_DIGEST_PREFIX_LEN).collect();
        let resp = response_for(&p.gate_id, &prefix, "approve");

        let verdict = resolve_response(&resp, &payload);
        assert_eq!(
            verdict,
            ResponseVerdict::Accepted {
                gate_id: p.gate_id.clone(),
                option_key: "approve".to_string(),
                digest: resp.digest.clone(),
                decided_at: resp.received_at,
            }
        );
    }

    #[test]
    fn resolve_response_stale_digest_when_gate_matches_but_digest_does_not() {
        let payload = expected_payload();
        let p = payload.payload();
        let resp = response_for(&p.gate_id, "0000000000000", "approve");

        let verdict = resolve_response(&resp, &payload);
        assert_eq!(
            verdict,
            ResponseVerdict::StaleDigest {
                gate_id: p.gate_id.clone(),
                option_key: "approve".to_string(),
                digest: resp.digest.clone(),
                decided_at: resp.received_at,
            },
            "a stale digest must be rejected, never accepted"
        );
        assert_ne!(
            verdict,
            ResponseVerdict::Accepted {
                gate_id: p.gate_id.clone(),
                option_key: "approve".to_string(),
                digest: resp.digest.clone(),
                decided_at: resp.received_at,
            }
        );
    }

    #[test]
    fn resolve_response_unknown_gate_when_gate_does_not_match() {
        let payload = expected_payload();
        let p = payload.payload();
        let prefix: String = p.digest.chars().take(CALLBACK_DIGEST_PREFIX_LEN).collect();
        let resp = response_for("some-other-gate", &prefix, "approve");

        let verdict = resolve_response(&resp, &payload);
        assert_eq!(verdict, ResponseVerdict::UnknownGate);
    }

    // -- answercallbackquery_body / editmessagetext_body (task 2) -----------

    /// A fixed timestamp for verdict literals in tests that don't care what
    /// `decided_at` actually is, only that the field exists and round-trips.
    fn test_decided_at() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(1_700_000_000, 0).expect("valid timestamp")
    }

    #[test]
    fn answercallbackquery_body_accepted_names_the_chosen_option() {
        let ack = AckHandle("cbq-1".to_string());
        let verdict = ResponseVerdict::Accepted {
            gate_id: "gate-1".to_string(),
            option_key: "approve".to_string(),
            digest: "abc123".to_string(),
            decided_at: test_decided_at(),
        };
        let body = answercallbackquery_body(&ack, &verdict);
        assert_eq!(body["callback_query_id"], "cbq-1");
        assert_eq!(body["text"], "Recorded: approve");
    }

    #[test]
    fn answercallbackquery_body_stale_digest_distinguishes_from_accepted() {
        let ack = AckHandle("cbq-2".to_string());
        let verdict = ResponseVerdict::StaleDigest {
            gate_id: "gate-1".to_string(),
            option_key: "approve".to_string(),
            digest: "abc123".to_string(),
            decided_at: test_decided_at(),
        };
        let body = answercallbackquery_body(&ack, &verdict);
        let text = body["text"].as_str().unwrap();
        assert!(!text.is_empty());
        assert!(text.to_lowercase().contains("changed") || text.to_lowercase().contains("valid"));
    }

    #[test]
    fn answercallbackquery_body_unknown_gate_distinguishes_from_accepted() {
        let ack = AckHandle("cbq-3".to_string());
        let body = answercallbackquery_body(&ack, &ResponseVerdict::UnknownGate);
        let text = body["text"].as_str().unwrap();
        assert!(!text.is_empty());
        assert!(
            text.to_lowercase().contains("answered") || text.to_lowercase().contains("pending")
        );
    }

    #[test]
    fn answercallbackquery_body_three_verdict_arms_produce_distinct_text() {
        let ack = AckHandle("cbq-4".to_string());
        let accepted = answercallbackquery_body(
            &ack,
            &ResponseVerdict::Accepted {
                gate_id: "gate-1".to_string(),
                option_key: "approve".to_string(),
                digest: "abc123".to_string(),
                decided_at: test_decided_at(),
            },
        );
        let stale = answercallbackquery_body(
            &ack,
            &ResponseVerdict::StaleDigest {
                gate_id: "gate-1".to_string(),
                option_key: "approve".to_string(),
                digest: "abc123".to_string(),
                decided_at: test_decided_at(),
            },
        );
        let unknown = answercallbackquery_body(&ack, &ResponseVerdict::UnknownGate);
        assert_ne!(accepted["text"], stale["text"]);
        assert_ne!(accepted["text"], unknown["text"]);
        assert_ne!(stale["text"], unknown["text"]);
    }

    #[test]
    fn answercallbackquery_body_text_at_200_chars_is_not_clamped() {
        let ack = AckHandle("cbq-5".to_string());
        // "Recorded: " is 10 chars; pad option_key so total text is exactly 200.
        let option_key = "x".repeat(190);
        let verdict = ResponseVerdict::Accepted {
            gate_id: "gate-1".to_string(),
            option_key: option_key.clone(),
            digest: "abc123".to_string(),
            decided_at: test_decided_at(),
        };
        let body = answercallbackquery_body(&ack, &verdict);
        let text = body["text"].as_str().unwrap();
        assert_eq!(text.chars().count(), 200);
        assert_eq!(text, format!("Recorded: {option_key}"));
    }

    #[test]
    fn answercallbackquery_body_text_at_201_chars_is_clamped_to_200() {
        let ack = AckHandle("cbq-6".to_string());
        // 191 chars of option_key => 201 chars total before clamping.
        let option_key = "x".repeat(191);
        let verdict = ResponseVerdict::Accepted {
            gate_id: "gate-1".to_string(),
            option_key,
            digest: "abc123".to_string(),
            decided_at: test_decided_at(),
        };
        let body = answercallbackquery_body(&ack, &verdict);
        let text = body["text"].as_str().unwrap();
        assert_eq!(
            text.chars().count(),
            ANSWER_CALLBACK_TEXT_MAX_CHARS,
            "text must be clamped to exactly the 200-char ceiling"
        );
    }

    #[test]
    fn editmessagetext_body_targets_chat_and_message() {
        let body = editmessagetext_body("chat-42", 777, "the rendered diff", "approve");
        assert_eq!(body["chat_id"], "chat-42");
        assert_eq!(body["message_id"], 777);
        assert!(body["text"].as_str().unwrap().contains("the rendered diff"));
        assert!(body["text"].as_str().unwrap().contains("approve"));
    }

    #[test]
    fn editmessagetext_body_clears_reply_markup_rather_than_omitting_it() {
        let body = editmessagetext_body("chat-42", 777, "summary", "approve");
        let reply_markup = body
            .get("reply_markup")
            .expect("reply_markup must be present, not absent");
        let keyboard = reply_markup
            .get("inline_keyboard")
            .expect("inline_keyboard must be present");
        assert_eq!(
            keyboard
                .as_array()
                .expect("inline_keyboard is an array")
                .len(),
            0,
            "inline_keyboard must be empty so the live buttons are gone"
        );
    }

    // -- TelegramTransport::api_url (task 4) -------------------------------

    #[test]
    fn transport_new_stores_config_without_exposing_token_via_debug() {
        let transport = TelegramTransport::new(TelegramConfig {
            bot_token: crate::config::BotToken::new("123456:AAAA-super-secret-value"),
            chat_id: "42".to_string(),
        });
        let rendered = format!("{transport:?}");
        assert!(
            !rendered.contains("AAAA-super-secret-value"),
            "TelegramTransport Debug must never contain the raw token; got: {rendered}"
        );
    }

    #[test]
    fn api_url_embeds_token_in_path_for_request_construction() {
        // The token must be embedded to build a valid request URL — this
        // test asserts the URL shape, not that it's safe to log (it isn't;
        // callers must never log this string).
        let transport = TelegramTransport::new(TelegramConfig {
            bot_token: crate::config::BotToken::new("123:ABC"),
            chat_id: "42".to_string(),
        });
        let url = transport.api_url("sendMessage");
        assert_eq!(url, "https://api.telegram.org/bot123:ABC/sendMessage");
    }
}
