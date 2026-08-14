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
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use engine_core::operator::ValidatedOperatorPayload;
use thiserror::Error;

pub mod telegram;
pub mod telegram_http;

#[cfg(test)]
mod tests;

/// An opaque handle a transport mints when it observes an inbound response,
/// and later consumes to acknowledge that same response
/// (`OperatorTransport::acknowledge`). The encoding is entirely
/// transport-specific (Telegram: the callback query's `id`); callers must
/// not parse it, only round-trip it. A transport with no acknowledgement
/// concept (e.g. WhatsApp) never mints one — its responses carry `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AckHandle(pub String);

/// An opaque handle identifying which message an inbound response's
/// original prompt was delivered as, so a later edit (dropping its buttons
/// and showing the chosen option) can target the right message. The
/// encoding is entirely transport-specific (Telegram: `chat_id` +
/// `message_id`); callers must not parse it, only round-trip it. A
/// transport with no such concept, or a response the transport could not
/// resolve a message for, carries `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageHandle {
    /// Transport-specific chat identifier the original message lives in.
    pub chat_id: String,
    /// Transport-specific identifier of the original message itself.
    pub message_id: i64,
}

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
    /// Opaque handle to acknowledge this response back to the transport it
    /// arrived on, if that transport has an acknowledgement concept. `None`
    /// for a transport with no such concept, or when the id could not be
    /// captured — losing a real decision because it cannot be acknowledged
    /// is strictly worse than not acknowledging it.
    pub ack: Option<AckHandle>,
    /// Opaque handle to the message the operator responded to, if the
    /// transport can resolve one — used to edit that message and drop its
    /// live buttons once a decision is taken. `None` for a transport with
    /// no such concept, or when the location could not be captured.
    pub message: Option<MessageHandle>,
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

    /// Acknowledge `response`, whose verdict has already been resolved to
    /// `verdict`, back to the transport it arrived on — so the operator's
    /// tap stops spinning and (where the transport supports it) the
    /// original message is edited to show the decision and drop its live
    /// buttons (`ticket-telegram-answer-callback` task 3).
    ///
    /// Default-implemented as a no-op `Ok(())`: a transport with no
    /// acknowledgement concept (WhatsApp, and every existing test fake)
    /// needs no change to keep compiling and behaving correctly. Telegram
    /// overrides this to actually call `answerCallbackQuery` (and, best
    /// effort, `editMessageText`).
    async fn acknowledge(
        &self,
        _response: &OperatorResponse,
        _verdict: &telegram::ResponseVerdict,
    ) -> Result<(), NotifyError> {
        Ok(())
    }
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

// ── Long-poll loop (BA.18.B task 5) ─────────────────────────────────────────

/// Look up the payload a gate is currently waiting on a response for, so a
/// raw [`OperatorResponse`] can be resolved against it (gate + digest match,
/// stale-digest, or unknown-gate — see [`telegram::resolve_response`]).
///
/// Production wiring (`src/serve/mod.rs::run_server`) currently backs this
/// with [`PendingPayloads`], a process-local registry populated only by
/// `POST /api/notify/test` (`ticket-notify-send-trigger`) — so a gate an
/// engine run actually queued is never in it and resolves to `UnknownGate`.
/// The real backing store for those (`engine_core::workflows::approve_and_run::ApproveAndRunSeams::lookup_pending`,
/// `engine-rs:EN.8.D`) is what `ticket-approve-and-run-seams` points this
/// seam at; the type itself does not change either way, and this seam still
/// lets tests inject a fixed in-memory map without depending on either
/// source.
pub type PendingLookup = Box<dyn Fn(&str) -> Option<ValidatedOperatorPayload> + Send + Sync>;

/// What the loop does with a resolved verdict. A callback rather than a
/// fixed action so this loop never needs to know how to *act* on a
/// verdict (ledger write, execution) — that consumption is
/// `ApproveAndRunSeams::resolve_verdict`'s side (`engine-rs:EN.8.D`,
/// wired in by `ticket-approve-and-run-seams`); this loop's job stops at
/// resolving and reporting.
pub type VerdictSink = Box<dyn Fn(telegram::ResponseVerdict) + Send + Sync>;

/// Drives [`OperatorTransport::poll_responses`] on a cadence set by the
/// transport's own long-poll timeout, resolving each observed response
/// against [`PendingLookup`] and reporting the verdict to [`VerdictSink`].
///
/// Backoff: a **retryable** [`NotifyError`] (transport failure, rate limit)
/// doubles the backoff (capped at [`Self::MAX_BACKOFF_SECS`]) before the
/// next attempt, so a persistent outage never spins the loop hot; a
/// successful tick resets the backoff to the floor. A **permanent**
/// `NotifyError` (bad credentials, malformed response) also backs off —
/// the loop has no way to distinguish "will never succeed" from "the
/// operator hasn't fixed the token yet", so it keeps trying at the same
/// backed-off cadence rather than giving up (there is nothing to give up
/// *to* — this is a background task, not a request with a caller to fail).
pub struct NotifyPollLoop {
    transport: Arc<dyn OperatorTransport>,
    cursor: Option<UpdateCursor>,
    pending: PendingLookup,
    on_verdict: VerdictSink,
    backoff: Duration,
}

impl NotifyPollLoop {
    /// Floor of the backoff range — the delay after any single failed tick.
    const MIN_BACKOFF_SECS: u64 = 1;
    /// Ceiling of the backoff range — a persistent outage never waits
    /// longer than this between attempts.
    const MAX_BACKOFF_SECS: u64 = 60;

    #[must_use]
    pub fn new(
        transport: Arc<dyn OperatorTransport>,
        pending: PendingLookup,
        on_verdict: VerdictSink,
    ) -> Self {
        Self {
            transport,
            cursor: None,
            pending,
            on_verdict,
            backoff: Duration::from_secs(Self::MIN_BACKOFF_SECS),
        }
    }

    /// The cursor this loop will pass to the next `poll_responses` call.
    /// Exposed for tests asserting cursor arithmetic survives a failed tick
    /// and resumes at the correct position on recovery.
    #[must_use]
    pub fn cursor(&self) -> Option<&UpdateCursor> {
        self.cursor.as_ref()
    }

    /// The backoff duration the next failed tick would apply. Exposed for
    /// tests asserting the doubling/reset/cap behaviour.
    #[must_use]
    pub fn backoff(&self) -> Duration {
        self.backoff
    }

    /// Run one poll-resolve-dispatch cycle. On success, returns the number
    /// of responses observed (0 is a normal empty long-poll), advances the
    /// cursor (an unchanged cursor from the transport, per its own
    /// no-reset-on-empty contract, leaves this loop's cursor unchanged
    /// too), and resets the backoff. On failure, applies backoff and
    /// returns the error — the cursor is left untouched either way, so a
    /// retry resumes from exactly where it left off rather than replaying
    /// or skipping the backlog.
    pub async fn tick(&mut self) -> Result<usize, NotifyError> {
        let outcome = self.transport.poll_responses(self.cursor.clone()).await;
        match outcome {
            Ok((responses, new_cursor)) => {
                self.backoff = Duration::from_secs(Self::MIN_BACKOFF_SECS);
                if let Some(cursor) = new_cursor {
                    self.cursor = Some(cursor);
                }
                let observed = responses.len();
                for resp in responses {
                    let verdict = match (self.pending)(&resp.gate_id) {
                        Some(expected) => telegram::resolve_response(&resp, &expected),
                        None => telegram::ResponseVerdict::UnknownGate,
                    };
                    // Acknowledge first, before the (caller-supplied, possibly
                    // blocking) sink dispatch — Telegram expires callback
                    // queries, so the ack must not wait behind verdict
                    // consumption. A failed ack is retried at most once; it
                    // is logged either way but never discards the response
                    // and never skips the dispatch below, since the verdict
                    // is already resolved by this point regardless of
                    // whether the operator's tap could be visibly confirmed.
                    if let Err(err) = self.transport.acknowledge(&resp, &verdict).await {
                        tracing::warn!(
                            target: "bastion::serve",
                            error = %err,
                            gate_id = %resp.gate_id,
                            "operator response acknowledgement failed; retrying once"
                        );
                        if let Err(retry_err) = self.transport.acknowledge(&resp, &verdict).await {
                            tracing::warn!(
                                target: "bastion::serve",
                                error = %retry_err,
                                gate_id = %resp.gate_id,
                                "operator response acknowledgement retry failed; dispatching verdict anyway"
                            );
                        }
                    }
                    (self.on_verdict)(verdict);
                }
                Ok(observed)
            }
            Err(err) => {
                if err.is_retryable() {
                    self.backoff =
                        (self.backoff * 2).min(Duration::from_secs(Self::MAX_BACKOFF_SECS));
                }
                Err(err)
            }
        }
    }

    /// Drive [`Self::tick`] forever. Never returns under normal operation;
    /// intended to be spawned once at server boot
    /// (`src/serve/mod.rs::run_server`) via `actix_web::rt::spawn`, only
    /// when Telegram config resolved (constraint: unconfigured is simply
    /// not spawned, not a warning and not a failure).
    ///
    /// Each transport call already carries its own long-poll timeout
    /// (Telegram: `TelegramTransport::GETUPDATES_TIMEOUT_SECS`), so a
    /// successful tick's own blocking wait *is* this loop's pacing — only a
    /// failed tick additionally sleeps, for [`Self::backoff`].
    pub async fn run(mut self) {
        loop {
            if let Err(err) = self.tick().await {
                tracing::warn!(
                    target: "bastion::serve",
                    error = %err,
                    retryable = err.is_retryable(),
                    backoff_secs = self.backoff.as_secs(),
                    "operator response poll failed"
                );
                tokio::time::sleep(self.backoff).await;
            }
        }
    }
}

// ── Pending-payload registry (ticket-notify-send-trigger task 1) ───────────

/// Bounded in-memory holding area for [`ValidatedOperatorPayload`]s this
/// process has sent, keyed by `gate_id`, so an inbound
/// [`OperatorResponse`][crate::serve::notify::OperatorResponse] can be
/// resolved against the payload the operator was actually shown.
///
/// This is deliberately not `engine-rs:EN.8.B`'s queue — it is a
/// process-local stand-in that only knows about payloads sent by *this*
/// process, via [`Self::lookup`]'s [`PendingLookup`]. When `EN.8.B` lands,
/// its queue becomes the `PendingLookup` source instead; the seam type
/// itself does not change, so `NotifyPollLoop` needs no edit either way.
///
/// Bounded so a registry fed by a test/smoke route cannot grow without
/// limit: insertion past [`Self::CAPACITY`] evicts the single oldest
/// still-pending entry first (FIFO by insertion order, not by any notion
/// of "processed"), tracked via an insertion-order `VecDeque` of gate ids
/// alongside the map.
pub struct PendingPayloads {
    inner: std::sync::Mutex<PendingPayloadsInner>,
}

struct PendingPayloadsInner {
    by_gate: std::collections::HashMap<String, ValidatedOperatorPayload>,
    order: std::collections::VecDeque<String>,
}

impl PendingPayloads {
    /// Maximum number of pending payloads held at once. Chosen generously
    /// above any plausible in-flight smoke-test or real operator-gate
    /// volume — this bounds runaway growth, not normal operation.
    pub const CAPACITY: usize = 256;

    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: std::sync::Mutex::new(PendingPayloadsInner {
                by_gate: std::collections::HashMap::new(),
                order: std::collections::VecDeque::new(),
            }),
        }
    }

    /// Register `payload` under its own `gate_id`. If the registry is
    /// already at [`Self::CAPACITY`], the single oldest entry (by
    /// insertion order) is evicted first, so this call never grows the
    /// registry past the cap. Re-inserting an existing `gate_id` replaces
    /// its payload without changing its position in eviction order.
    pub fn insert(&self, payload: ValidatedOperatorPayload) {
        let gate_id = payload.payload().gate_id.clone();
        let mut inner = self.inner.lock().expect("PendingPayloads mutex poisoned");
        let is_new = !inner.by_gate.contains_key(&gate_id);
        if is_new
            && inner.by_gate.len() >= Self::CAPACITY
            && let Some(oldest) = inner.order.pop_front()
        {
            inner.by_gate.remove(&oldest);
        }
        if is_new {
            inner.order.push_back(gate_id.clone());
        }
        inner.by_gate.insert(gate_id, payload);
    }

    /// Look up the payload currently pending for `gate_id`, if any.
    #[must_use]
    pub fn get(&self, gate_id: &str) -> Option<ValidatedOperatorPayload> {
        let inner = self.inner.lock().expect("PendingPayloads mutex poisoned");
        inner.by_gate.get(gate_id).cloned()
    }

    /// Remove and return the pending payload for `gate_id`, if any — used
    /// on `Accepted` so a replayed tap of the same button cannot resolve
    /// twice (task 3).
    pub fn remove(&self, gate_id: &str) -> Option<ValidatedOperatorPayload> {
        let mut inner = self.inner.lock().expect("PendingPayloads mutex poisoned");
        let removed = inner.by_gate.remove(gate_id);
        if removed.is_some() {
            inner.order.retain(|g| g != gate_id);
        }
        removed
    }

    /// Produce a [`PendingLookup`] closure backed by this registry, so
    /// [`NotifyPollLoop`] can resolve responses against payloads this
    /// process sent without knowing anything about the registry's
    /// existence. Callers wanting removal-on-accept (task 3) do that at
    /// the verdict-handling layer via [`Self::remove`], not here — this
    /// closure only reads.
    #[must_use]
    pub fn lookup(self: &Arc<Self>) -> PendingLookup {
        let registry = Arc::clone(self);
        Box::new(move |gate_id: &str| registry.get(gate_id))
    }
}

impl Default for PendingPayloads {
    fn default() -> Self {
        Self::new()
    }
}
