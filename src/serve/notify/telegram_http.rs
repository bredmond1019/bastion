//! Gate-agnostic Telegram Bot API HTTP primitives (`BA.20.C` task 1).
//!
//! Extracted from `telegram.rs` — these functions carry no dependency on
//! `ValidatedOperatorPayload`, `AckHandle`, `ResponseVerdict`, `CallbackData`,
//! or any other gate/digest concept. They are pure URL/query/status-code
//! shapes any Telegram Bot API caller needs, regardless of what it is
//! polling or sending — the shared substrate the gate-approval transport
//! (`telegram.rs`) and the session-QA bridge (`BA.20.C`) both build on.

use crate::config::BotToken;
use crate::serve::notify::NotifyError;

/// Percent-encode `value` for use in a URL query string (RFC 3986
/// "unreserved" characters pass through unescaped; everything else becomes
/// `%XX`). Pure — no dependency on `reqwest`'s optional `query` feature,
/// which this crate does not enable.
pub(crate) fn percent_encode_query_value(value: &str) -> String {
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
/// empty key), as callers such as `getupdates_query` guarantee.
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

/// Pull Telegram's `Retry-After` header (seconds) off a response, if present.
/// Pure w.r.t. the header map it's handed.
pub(crate) fn retry_after_from_response(resp: &reqwest::Response) -> Option<u64> {
    resp.headers()
        .get("Retry-After")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
}

/// Render a `reqwest::Error` without ever including request/connection
/// details that could carry the token (the token is embedded in the URL a
/// `reqwest::Error` may otherwise stringify). Reports only the failure
/// class, never `e.to_string()` verbatim.
pub(crate) fn classify_reqwest_error(e: &reqwest::Error) -> &'static str {
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

/// Build the base Telegram Bot API URL for `method` (e.g. `"sendMessage"`,
/// `"getUpdates"`) against `token`. The token lives in this URL's path —
/// callers must never log the returned string; log the method name alone
/// instead (non-negotiable constraint 3).
#[must_use]
pub fn api_url(token: &BotToken, method: &str) -> String {
    format!("https://api.telegram.org/bot{}/{method}", token.expose())
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- append_query_string ------------------------------------------------

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

    // -- classify_http_status ------------------------------------------------

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

    // -- api_url --------------------------------------------------------------

    #[test]
    fn api_url_embeds_token_in_path_for_request_construction() {
        // The token must be embedded to build a valid request URL — this
        // test asserts the URL shape, not that it's safe to log (it isn't;
        // callers must never log this string).
        let token = crate::config::BotToken::new("123:ABC");
        let url = api_url(&token, "sendMessage");
        assert_eq!(url, "https://api.telegram.org/bot123:ABC/sendMessage");
    }
}
