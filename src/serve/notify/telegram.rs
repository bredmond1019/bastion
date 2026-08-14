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
use engine_core::operator::{OperatorPayloadLimits, ValidatedOperatorPayload};

use crate::config::{BotToken, TelegramConfig};
use crate::serve::notify::{
    AckHandle, DeliveredMessage, NotifyError, OperatorResponse, OperatorTransport, UpdateCursor,
};

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

/// Percent-encode `value` for use in a URL query string (RFC 3986
/// "unreserved" characters pass through unescaped; everything else becomes
/// `%XX`). Pure — no dependency on `reqwest`'s optional `query` feature,
/// which this crate does not enable.
fn percent_encode_query_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char);
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Append `query` pairs onto `base_url` as a percent-encoded query string.
/// Pure. `query` is expected to already be empty-free (no pair with an
/// empty key), as [`getupdates_query`] guarantees.
#[must_use]
pub fn append_query_string(base_url: &str, query: &[(String, String)]) -> String {
    if query.is_empty() {
        return base_url.to_string();
    }
    let pairs: Vec<String> = query
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                percent_encode_query_value(k),
                percent_encode_query_value(v)
            )
        })
        .collect();
    format!("{base_url}?{}", pairs.join("&"))
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

        responses.push(OperatorResponse {
            gate_id: decoded.gate_id,
            digest: decoded.digest_prefix,
            option_key: decoded.option_key,
            received_at: chrono::Utc::now(),
            ack,
        });
    }

    let cursor = max_update_id.map(|id| UpdateCursor((id + 1).to_string()));
    Ok((responses, cursor))
}

/// The outcome of resolving an [`OperatorResponse`] against the payload the
/// caller expects it to answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseVerdict {
    /// Both the gate id and the digest prefix match: this response applies.
    Accepted {
        /// The gate this response answers.
        gate_id: String,
        /// The stable machine key of the option the operator tapped.
        option_key: String,
    },
    /// The gate matches but the digest does not — the payload was mutated
    /// (re-rendered) after this response was shown, so it must re-queue
    /// rather than execute. Never conflated with `Accepted`.
    StaleDigest,
    /// The response answers a different gate than `expected` — not this
    /// payload's response at all.
    UnknownGate,
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
        return ResponseVerdict::StaleDigest;
    }

    ResponseVerdict::Accepted {
        gate_id: resp.gate_id.clone(),
        option_key: resp.option_key.clone(),
    }
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

/// Map an observed Telegram HTTP status code onto a [`NotifyError`]. Pure —
/// no I/O — so the status-code → error-class mapping is unit-testable
/// without a live call.
///
/// - `429` → `RateLimited`, using `retry_after_secs` from Telegram's
///   `parameters.retry_after` field if present, else a conservative default.
/// - `401` / `403` → `Unauthorized` (bad/revoked bot token). Deliberately
///   carries no credential value.
/// - Any other non-2xx status → retryable `Transport` (Telegram outages,
///   5xx, etc. are transient from this caller's perspective).
#[must_use]
pub fn classify_http_status(status: u16, retry_after_secs: Option<u64>) -> Option<NotifyError> {
    match status {
        200..=299 => None,
        429 => Some(NotifyError::RateLimited {
            retry_after_secs: retry_after_secs.unwrap_or(30),
        }),
        401 | 403 => Some(NotifyError::Unauthorized),
        other => Some(NotifyError::Transport {
            reason: format!("unexpected Telegram API status {other}"),
        }),
    }
}

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
        format!(
            "https://api.telegram.org/bot{}/{method}",
            self.bot_token.expose()
        )
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
                reason: format!("{method} request failed: {}", classify_reqwest_error(&e)),
            })?;

        let status = resp.status().as_u16();
        if let Some(err) = classify_http_status(status, retry_after_from_response(&resp)) {
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
        let url = append_query_string(&self.api_url(method), &query);
        let resp = self
            .client
            .get(url)
            .timeout(Duration::from_secs(Self::CLIENT_TIMEOUT_SECS))
            .send()
            .await
            .map_err(|e| NotifyError::Transport {
                reason: format!("{method} request failed: {}", classify_reqwest_error(&e)),
            })?;

        let status = resp.status().as_u16();
        if let Some(err) = classify_http_status(status, retry_after_from_response(&resp)) {
            return Err(err);
        }

        let raw: serde_json::Value = resp.json().await.map_err(|e| NotifyError::Malformed {
            reason: format!("{method} response body was not valid JSON: {e}"),
        })?;
        parse_updates(&raw)
    }
}

/// Pull Telegram's `Retry-After` header (seconds) off a response, if present.
/// Pure w.r.t. the header map it's handed.
fn retry_after_from_response(resp: &reqwest::Response) -> Option<u64> {
    resp.headers()
        .get("Retry-After")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
}

/// Render a `reqwest::Error` without ever including request/connection
/// details that could carry the token (the token is embedded in the URL a
/// `reqwest::Error` may otherwise stringify). Reports only the failure
/// class, never `e.to_string()` verbatim.
fn classify_reqwest_error(e: &reqwest::Error) -> &'static str {
    if e.is_timeout() {
        "timed out"
    } else if e.is_connect() {
        "connection failed"
    } else if e.is_decode() {
        "response decode failed"
    } else {
        "request failed"
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
            ResponseVerdict::StaleDigest,
            "a stale digest must be rejected, never accepted"
        );
        assert_ne!(
            verdict,
            ResponseVerdict::Accepted {
                gate_id: p.gate_id.clone(),
                option_key: "approve".to_string(),
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

    // -- append_query_string (task 4) --------------------------------------

    #[test]
    fn append_query_string_empty_query_returns_base_unchanged() {
        assert_eq!(
            append_query_string("https://example.com/x", &[]),
            "https://example.com/x"
        );
    }

    #[test]
    fn append_query_string_encodes_and_joins_pairs() {
        let query = vec![
            ("timeout".to_string(), "30".to_string()),
            (
                "allowed_updates".to_string(),
                r#"["callback_query"]"#.to_string(),
            ),
        ];
        let url = append_query_string("https://example.com/getUpdates", &query);
        assert_eq!(
            url,
            "https://example.com/getUpdates?timeout=30&allowed_updates=%5B%22callback_query%22%5D"
        );
    }

    #[test]
    fn append_query_string_matches_getupdates_query_output() {
        let query = getupdates_query(Some(UpdateCursor("42".to_string())), 30);
        let url = append_query_string("https://example.com/getUpdates", &query);
        assert!(url.starts_with("https://example.com/getUpdates?offset=42&timeout=30"));
    }

    // -- classify_http_status (task 4) -----------------------------------

    #[test]
    fn classify_http_status_2xx_is_success() {
        assert_eq!(classify_http_status(200, None), None);
        assert_eq!(classify_http_status(201, None), None);
        assert_eq!(classify_http_status(299, None), None);
    }

    #[test]
    fn classify_http_status_429_is_rate_limited_with_hint() {
        let err = classify_http_status(429, Some(5)).expect("429 is an error");
        assert_eq!(
            err,
            NotifyError::RateLimited {
                retry_after_secs: 5
            }
        );
        assert!(err.is_retryable());
    }

    #[test]
    fn classify_http_status_429_without_hint_uses_default() {
        let err = classify_http_status(429, None).expect("429 is an error");
        assert_eq!(
            err,
            NotifyError::RateLimited {
                retry_after_secs: 30
            }
        );
    }

    #[test]
    fn classify_http_status_401_and_403_are_unauthorized_and_permanent() {
        let err_401 = classify_http_status(401, None).expect("401 is an error");
        let err_403 = classify_http_status(403, None).expect("403 is an error");
        assert_eq!(err_401, NotifyError::Unauthorized);
        assert_eq!(err_403, NotifyError::Unauthorized);
        assert!(!err_401.is_retryable());
        assert!(!err_403.is_retryable());
    }

    #[test]
    fn classify_http_status_other_non_2xx_is_retryable_transport() {
        let err = classify_http_status(500, None).expect("500 is an error");
        assert!(matches!(err, NotifyError::Transport { .. }));
        assert!(err.is_retryable());

        let err = classify_http_status(400, None).expect("400 is an error");
        assert!(matches!(err, NotifyError::Transport { .. }));
    }

    #[test]
    fn classify_http_status_error_never_contains_a_token_shaped_value() {
        for status in [401, 403, 429, 500] {
            if let Some(err) = classify_http_status(status, Some(3)) {
                let rendered = format!("{err}");
                assert!(!rendered.to_lowercase().contains("token"));
            }
        }
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
