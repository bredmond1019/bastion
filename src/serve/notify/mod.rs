//! Operator-notification transport seam (`BA.18.B` task 1).
//!
//! This module consumes `engine_core::operator::ValidatedOperatorPayload` —
//! the `EN.8.A` payload contract — and never redefines it. Nothing here
//! invents an option/label/summary shape of its own; a transport either
//! ships a validated payload or fails closed.
//!
//! [`OperatorTransport`] is the seam: one async trait with a `send` half
//! (deliver a validated payload) and a `poll_responses` half (long-poll for
//! the operator's tap). Telegram (`BA.18.B` tasks 2-5) is the first
//! implementation; WhatsApp is meant to be a second `impl`, not a rewrite —
//! the trait itself carries no channel-specific shape.
//!
//! Inbound is long-polling only (no webhook route, no listening socket) —
//! see `planning/BA.18.B/tasks.md`'s non-negotiable constraint 2. The bot
//! token never appears in a tracked file, a log line, or an error message —
//! constraint 3; [`NotifyError`]'s `Display`/`Debug` renderings are asserted
//! by this module's tests to never contain a token-shaped substring.

use std::fmt;

use async_trait::async_trait;
use engine_core::operator::ValidatedOperatorPayload;
use thiserror::Error;

#[cfg(test)]
mod tests;

/// A response the operator gave, resolved back to the gate and digest it
/// answers. `option_key` is the stable machine key of the tapped option
/// (`OperatorResponseOption::key`), never the operator-visible label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorResponse {
    /// The gate this response answers.
    pub gate_id: String,
    /// The digest of the payload the operator was shown when they
    /// responded — used to reject a response against a payload that has
    /// since been mutated (stale-digest rejection, task 3).
    pub digest: String,
    /// The stable machine key of the option the operator tapped.
    pub option_key: String,
    /// When this transport observed the response.
    pub received_at: chrono::DateTime<chrono::Utc>,
}

/// Confirmation that [`OperatorTransport::send`] delivered a payload.
/// Transport-agnostic: a channel-specific message id, if any, belongs to
/// that transport's own impl module, not this shared shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveredMessage {
    /// Opaque transport-assigned identifier for the delivered message
    /// (e.g. a Telegram `message_id` rendered as a string). Transports that
    /// have no such id may leave this empty.
    pub transport_message_id: String,
}

/// Opaque position in the inbound update stream, threaded back into the
/// next [`OperatorTransport::poll_responses`] call so a restart resumes
/// instead of replaying (or dropping) the backlog. The concrete encoding is
/// transport-specific (Telegram: the next `offset`); callers must not parse
/// it, only round-trip it.
#[derive(Clone, PartialEq, Eq)]
pub struct UpdateCursor(pub String);

/// Why an [`OperatorTransport`] operation failed. Variants split along one
/// axis: whether the caller should retry.
///
/// - `Transport` / `RateLimited` are **retryable** — a transient send/poll
///   failure (connect error, timeout, HTTP 429).
/// - `PayloadRejected` / `Unauthorized` / `Malformed` are **permanent** — a
///   retry with the same inputs cannot succeed.
///
/// No variant's `Display` may interpolate a token or other credential; see
/// this module's `tests` for the assertion. Constructing an `Unauthorized`
/// or `Transport` variant never takes the credential as a field for exactly
/// this reason.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum NotifyError {
    /// A transient transport-level failure (connect error, timeout, DNS
    /// failure). Retryable.
    #[error("operator transport failure: {reason}")]
    Transport {
        /// Human-readable failure reason. Must never contain a credential.
        reason: String,
    },
    /// The transport reported a rate limit; retry after the given delay.
    /// Retryable.
    #[error("operator transport rate limited, retry after {retry_after_secs}s")]
    RateLimited {
        /// Seconds to wait before retrying, per the transport's own hint.
        retry_after_secs: u64,
    },
    /// The payload cannot be sent over this transport (e.g. it exceeds the
    /// transport's confirmed limits). Permanent — resending the same
    /// payload cannot succeed; the caller must re-render it.
    #[error("operator payload rejected by transport: {reason}")]
    PayloadRejected {
        /// Why the payload was rejected. Must never contain a credential.
        reason: String,
    },
    /// The transport rejected the credentials (401/403). Permanent from
    /// this call's perspective — deliberately carries no credential value,
    /// only the fact of the rejection.
    #[error("operator transport unauthorized")]
    Unauthorized,
    /// The transport returned a response this code could not parse (e.g.
    /// not the expected envelope shape). Permanent for this response;
    /// does not imply the whole batch is unusable (task 3's skip-and-continue
    /// policy for individual malformed updates).
    #[error("operator transport returned a malformed response: {reason}")]
    Malformed {
        /// What was malformed. Must never contain a credential.
        reason: String,
    },
}

impl NotifyError {
    /// Whether the caller should retry the operation that produced this
    /// error. `true` for transient transport-level failures; `false` for
    /// anything permanent (bad payload, bad credentials, unparseable
    /// response).
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            NotifyError::Transport { .. } | NotifyError::RateLimited { .. }
        )
    }
}

/// The transport seam: deliver a validated operator payload, and long-poll
/// for the operator's response. Implemented once per channel (Telegram
/// first; WhatsApp is meant to be a second `impl` sharing this trait
/// unchanged).
///
/// Object-safe: both methods take `&self`, return `Result<_, NotifyError>`
/// futures with no generic parameters, and the trait has no associated
/// types or `Self: Sized` bounds — `Box<dyn OperatorTransport>` /
/// `Arc<dyn OperatorTransport>` are both nameable. Marked `#[async_trait]`
/// to keep it object-safe under stable Rust (a native `async fn` in a trait
/// is not, without boxing the returned future by hand).
#[async_trait]
pub trait OperatorTransport: Send + Sync {
    /// Deliver `payload` over this transport. Must reject (via
    /// `NotifyError::PayloadRejected`) anything that would not survive the
    /// narrowest target channel's limits, rather than sending a truncated
    /// or partial rendering.
    async fn send(
        &self,
        payload: &ValidatedOperatorPayload,
    ) -> Result<DeliveredMessage, NotifyError>;

    /// Long-poll for operator responses since `since` (or from the start of
    /// the backlog if `None`). Returns the observed responses and the
    /// cursor to pass on the next call.
    async fn poll_responses(
        &self,
        since: Option<UpdateCursor>,
    ) -> Result<(Vec<OperatorResponse>, Option<UpdateCursor>), NotifyError>;
}

impl fmt::Debug for UpdateCursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The cursor is opaque and transport-specific (Telegram: a plain
        // integer offset), not a credential — but formatted explicitly
        // (rather than derived) so a future transport that encodes
        // something sensitive into the cursor does not get free `Debug`
        // access without a deliberate decision here.
        f.debug_tuple("UpdateCursor").field(&self.0).finish()
    }
}
