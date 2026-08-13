//! `POST /api/notify/test` — authenticated trigger that sends one real
//! validated operator payload over the configured transport
//! (`ticket-notify-send-trigger` task 2).
//!
//! `BA.18.B` shipped the transport seam, the Telegram impl, and the inbound
//! long-poll loop — but nothing in the binary ever called
//! [`OperatorTransport::send`]. This route exists solely to make the
//! `operator-telegram-live-smoke` session runnable: it builds a small fixed
//! `OperatorPayload` (exactly 2 response options), validates it through the
//! real `engine_core::operator::validate` contract, registers it in the
//! process-local [`PendingPayloads`] registry (`task 1`) so an inbound
//! response can resolve against it, sends it, and returns the `gate_id` +
//! `digest` it sent.
//!
//! # Route
//! - `POST /api/notify/test` — mounted inside the existing
//!   `web::scope("/api")` in `src/serve/mod.rs`, so it inherits
//!   [`crate::serve::auth::BearerAuthMiddleware`] like every other route in
//!   that scope. It is never mounted at the app root — that is the exact
//!   hole `BA.ticket.engine-surface-auth` closed.
//!
//! # Error mapping
//! - Transport unconfigured (`BASTION_TELEGRAM_BOT_TOKEN` /
//!   `BASTION_TELEGRAM_CHAT_ID` unset, or only one set) → 503 + `C005`,
//!   naming the missing var **by name only** — never a value.
//! - [`OperatorTransport::send`] failure → 502, with the machine `code`
//!   distinguishing [`NotifyError::is_retryable`] variants
//!   ([`notify_error_code`]) from permanent ones, rather than a single
//!   generic 500.
//!
//! # Pure/I/O split (Rule 6)
//! [`build_test_payload`] and [`notify_error_code`] are pure — unit-tested
//! directly. [`test_send_with`] carries the whole handler body
//! parameterized over an injected `&dyn OperatorTransport` (the established
//! `handlers/costs.rs::get_costs_with` shape), so the 200/502 paths are
//! testable against a scripted transport with no real network call and no
//! `actix_web::test` harness. [`test_send`] is the thin production
//! delegation: resolve the transport from env at call time (this route's
//! chosen option — see task constraint 5 — rather than resolving it once at
//! boot), then call [`test_send_with`].

use actix_web::{HttpResponse, web};
use engine_core::operator::{
    OperatorPayload, OperatorPayloadLimits, OperatorResponseOption, ValidatedOperatorPayload,
    validate,
};

use crate::config::{ConfigError, load_telegram_config};
use crate::serve::dto::{ErrorPayload, NotifyTestResponseDto};
use crate::serve::notify::telegram::TelegramTransport;
use crate::serve::notify::{NotifyError, OperatorTransport, PendingPayloads};

/// The fixed rendered summary sent by every `/api/notify/test` call. Not
/// operator-supplied content — this route exists to prove the transport
/// path works, not to render a real gate's payload.
const TEST_SUMMARY: &str = "bastion notify test-send — operator smoke check";

/// Human-readable reason shown in the 503 body when neither
/// `BASTION_TELEGRAM_BOT_TOKEN` nor `BASTION_TELEGRAM_CHAT_ID` is set.
/// Names the vars, never a value.
const BOTH_VARS_UNSET: &str = "BASTION_TELEGRAM_BOT_TOKEN and BASTION_TELEGRAM_CHAT_ID";

// ── Pure core ────────────────────────────────────────────────────────────────

/// Build and validate the fixed 2-option test payload under `gate_id`.
///
/// Pure — no I/O. `gate_id` is supplied by the caller (production: a fresh
/// uuid per request, per task constraint "gate_id is generated per request
/// ... so repeated smoke tests do not collide"; tests: a fixed id for
/// deterministic assertions). The fixed summary + exactly 2 options can
/// never fail [`validate`] against [`OperatorPayloadLimits::default`], so
/// this returns the validated payload directly rather than a `Result` —
/// there is no reachable error path to represent.
#[must_use]
pub fn build_test_payload(gate_id: &str) -> ValidatedOperatorPayload {
    let payload = OperatorPayload::new(
        gate_id,
        TEST_SUMMARY,
        vec![
            OperatorResponseOption::new("approve", "Approve"),
            OperatorResponseOption::new("reject", "Reject"),
        ],
    );
    validate(payload, &OperatorPayloadLimits::default())
        .expect("fixed 2-option test payload always satisfies the default operator limits")
}

/// Map a [`NotifyError`] to the machine `code` carried in the 502 response
/// body. Distinct codes per variant so retryable
/// ([`NotifyError::is_retryable`]) and permanent failures are
/// distinguishable in the response without adding a new field to the
/// shared [`ErrorPayload`] contract:
///
/// | Variant | Code | Retryable |
/// |---|---|---|
/// | `Transport` | `C009` (I/O error) | yes |
/// | `RateLimited` | `C013` (rate limit exceeded) | yes |
/// | `PayloadRejected` | `C006` (invalid input) | no |
/// | `Unauthorized` | `C012` (not authenticated) | no |
/// | `Malformed` | `C008` (serialization failure) | no |
///
/// Pure — no I/O.
#[must_use]
pub fn notify_error_code(err: &NotifyError) -> &'static str {
    match err {
        NotifyError::Transport { .. } => "C009",
        NotifyError::RateLimited { .. } => "C013",
        NotifyError::PayloadRejected { .. } => "C006",
        NotifyError::Unauthorized => "C012",
        NotifyError::Malformed { .. } => "C008",
    }
}

/// Build the 503 response for an unconfigured transport, naming `reason`
/// (the missing env var name(s)) — never a value. Pure — no I/O.
#[must_use]
fn unconfigured_response(reason: &str) -> HttpResponse {
    HttpResponse::ServiceUnavailable().json(ErrorPayload {
        code: "C005".to_owned(),
        message: format!("operator notification transport not configured — set {reason}"),
    })
}

// ── Handler body (transport injected) ───────────────────────────────────────

/// The whole `/api/notify/test` handler body, parameterized over the
/// transport to send through. Registers the built payload in `registry`
/// before sending, per the task order ("registers it in `PendingPayloads`,
/// sends it via the configured `OperatorTransport`") — so a response that
/// arrives even before `send` returns can still resolve.
pub async fn test_send_with(
    registry: &PendingPayloads,
    transport: &dyn OperatorTransport,
    gate_id: String,
) -> HttpResponse {
    let validated = build_test_payload(&gate_id);
    registry.insert(validated.clone());

    match transport.send(&validated).await {
        Ok(_delivered) => HttpResponse::Ok().json(NotifyTestResponseDto {
            gate_id: validated.payload().gate_id.clone(),
            digest: validated.payload().digest.clone(),
        }),
        Err(err) => HttpResponse::BadGateway().json(ErrorPayload {
            code: notify_error_code(&err).to_owned(),
            message: err.to_string(),
        }),
    }
}

// ── Handler (production) ────────────────────────────────────────────────────

/// `POST /api/notify/test` — resolves the Telegram transport from env at
/// call time (this route's chosen option for "unconfigured server must
/// still boot unchanged" — the route mounts unconditionally and degrades to
/// 503 per-request rather than being conditionally registered), builds a
/// fresh per-request `gate_id`, and delegates to [`test_send_with`].
pub async fn test_send(registry: web::Data<PendingPayloads>) -> HttpResponse {
    match load_telegram_config() {
        Ok(Some(cfg)) => {
            let transport = TelegramTransport::new(cfg);
            let gate_id = uuid::Uuid::new_v4().to_string();
            test_send_with(&registry, &transport, gate_id).await
        }
        Ok(None) => unconfigured_response(BOTH_VARS_UNSET),
        Err(ConfigError::IncompleteTelegramConfig(var)) => unconfigured_response(var),
        Err(_) => unconfigured_response(BOTH_VARS_UNSET),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    use crate::serve::notify::DeliveredMessage;

    async fn body_string(resp: HttpResponse) -> String {
        let bytes = actix_web::body::to_bytes(resp.into_body())
            .await
            .expect("response body must be readable");
        String::from_utf8(bytes.to_vec()).expect("utf8 body")
    }

    // ── build_test_payload ───────────────────────────────────────────────────

    #[test]
    fn build_test_payload_has_exactly_two_options() {
        let validated = build_test_payload("gate-1");
        assert_eq!(validated.payload().options.len(), 2);
        assert_eq!(validated.payload().gate_id, "gate-1");
    }

    #[test]
    fn build_test_payload_options_are_approve_and_reject() {
        let validated = build_test_payload("gate-1");
        let keys: Vec<&str> = validated
            .payload()
            .options
            .iter()
            .map(|o| o.key.as_str())
            .collect();
        assert_eq!(keys, vec!["approve", "reject"]);
    }

    #[test]
    fn build_test_payload_different_gate_ids_are_independent() {
        let a = build_test_payload("gate-a");
        let b = build_test_payload("gate-b");
        assert_ne!(a.payload().gate_id, b.payload().gate_id);
        // Same rendered summary + options → same digest, regardless of gate_id
        // (digest is deliberately not gate-scoped — see `OperatorPayload` docs).
        assert_eq!(a.payload().digest, b.payload().digest);
    }

    // ── notify_error_code ────────────────────────────────────────────────────

    #[test]
    fn transport_error_code_is_retryable_c009() {
        let err = NotifyError::Transport {
            reason: "connect timeout".to_owned(),
        };
        assert_eq!(notify_error_code(&err), "C009");
        assert!(err.is_retryable());
    }

    #[test]
    fn rate_limited_error_code_is_retryable_c013() {
        let err = NotifyError::RateLimited {
            retry_after_secs: 5,
        };
        assert_eq!(notify_error_code(&err), "C013");
        assert!(err.is_retryable());
    }

    #[test]
    fn payload_rejected_error_code_is_permanent_c006() {
        let err = NotifyError::PayloadRejected {
            reason: "too many options".to_owned(),
        };
        assert_eq!(notify_error_code(&err), "C006");
        assert!(!err.is_retryable());
    }

    #[test]
    fn unauthorized_error_code_is_permanent_c012() {
        assert_eq!(notify_error_code(&NotifyError::Unauthorized), "C012");
        assert!(!NotifyError::Unauthorized.is_retryable());
    }

    #[test]
    fn malformed_error_code_is_permanent_c008() {
        let err = NotifyError::Malformed {
            reason: "not ok envelope".to_owned(),
        };
        assert_eq!(notify_error_code(&err), "C008");
        assert!(!err.is_retryable());
    }

    // ── unconfigured_response ────────────────────────────────────────────────

    #[test]
    fn unconfigured_response_is_503() {
        let resp = unconfigured_response(BOTH_VARS_UNSET);
        assert_eq!(resp.status(), 503);
    }

    #[actix_web::test]
    async fn unconfigured_response_names_the_var_never_a_value() {
        let resp = unconfigured_response("BASTION_TELEGRAM_CHAT_ID");
        let body = body_string(resp).await;
        assert!(body.contains("BASTION_TELEGRAM_CHAT_ID"));
        // No credential-shaped substring can appear — `reason` here is
        // always a fixed var name, never a value, but assert the shape
        // holds for a plausible token-looking reason too.
        assert!(!body.contains("123456789:AAExampleFake"));
    }

    // ── test_send_with (injected transport) ──────────────────────────────────

    /// A transport whose `send` outcome is fixed up front — mirrors
    /// `notify::tests::ScriptedTransport`'s shape, narrowed to just `send`
    /// since this handler never calls `poll_responses`.
    struct FixedSendTransport {
        outcome: Mutex<Option<Result<DeliveredMessage, NotifyError>>>,
    }

    impl FixedSendTransport {
        fn new(outcome: Result<DeliveredMessage, NotifyError>) -> Self {
            Self {
                outcome: Mutex::new(Some(outcome)),
            }
        }
    }

    #[async_trait]
    impl OperatorTransport for FixedSendTransport {
        async fn send(
            &self,
            _payload: &ValidatedOperatorPayload,
        ) -> Result<DeliveredMessage, NotifyError> {
            self.outcome
                .lock()
                .expect("outcome mutex is never poisoned in these tests")
                .take()
                .expect("test_send_with calls send at most once per test")
        }

        async fn poll_responses(
            &self,
            since: Option<crate::serve::notify::UpdateCursor>,
        ) -> Result<
            (
                Vec<crate::serve::notify::OperatorResponse>,
                Option<crate::serve::notify::UpdateCursor>,
            ),
            NotifyError,
        > {
            Ok((Vec::new(), since))
        }
    }

    #[actix_web::test]
    async fn test_send_with_success_returns_200_with_gate_id_and_digest() {
        let registry = PendingPayloads::new();
        let transport = FixedSendTransport::new(Ok(DeliveredMessage {
            transport_message_id: "42".to_owned(),
        }));

        let resp = test_send_with(&registry, &transport, "gate-xyz".to_owned()).await;

        assert_eq!(resp.status(), 200);
        let expected_digest = build_test_payload("gate-xyz").payload().digest.clone();
        let body = body_string(resp).await;
        assert!(body.contains("gate-xyz"));
        assert!(body.contains(&expected_digest));
    }

    #[actix_web::test]
    async fn test_send_with_success_registers_payload_in_pending_registry() {
        let registry = PendingPayloads::new();
        let transport = FixedSendTransport::new(Ok(DeliveredMessage {
            transport_message_id: String::new(),
        }));

        test_send_with(&registry, &transport, "gate-reg".to_owned()).await;

        let pending = registry.get("gate-reg").expect("payload was registered");
        assert_eq!(pending.payload().gate_id, "gate-reg");
    }

    #[actix_web::test]
    async fn test_send_with_transport_failure_returns_502_retryable_code() {
        let registry = PendingPayloads::new();
        let transport = FixedSendTransport::new(Err(NotifyError::Transport {
            reason: "connect timeout".to_owned(),
        }));

        let resp = test_send_with(&registry, &transport, "gate-fail".to_owned()).await;

        assert_eq!(resp.status(), 502);
        let body = body_string(resp).await;
        assert!(body.contains("C009"));
    }

    #[actix_web::test]
    async fn test_send_with_transport_failure_returns_502_permanent_code() {
        let registry = PendingPayloads::new();
        let transport = FixedSendTransport::new(Err(NotifyError::Unauthorized));

        let resp = test_send_with(&registry, &transport, "gate-fail-2".to_owned()).await;

        assert_eq!(resp.status(), 502);
        let body = body_string(resp).await;
        assert!(body.contains("C012"));
    }

    #[actix_web::test]
    async fn test_send_with_retryable_and_permanent_failures_carry_different_codes() {
        let registry = PendingPayloads::new();

        let retryable_transport = FixedSendTransport::new(Err(NotifyError::RateLimited {
            retry_after_secs: 3,
        }));
        let retryable_resp =
            test_send_with(&registry, &retryable_transport, "gate-a".to_owned()).await;
        let retryable_body = body_string(retryable_resp).await;

        let permanent_transport = FixedSendTransport::new(Err(NotifyError::PayloadRejected {
            reason: "rejected".to_owned(),
        }));
        let permanent_resp =
            test_send_with(&registry, &permanent_transport, "gate-b".to_owned()).await;
        let permanent_body = body_string(permanent_resp).await;

        assert_ne!(retryable_body, permanent_body);
        assert!(retryable_body.contains("C013"));
        assert!(permanent_body.contains("C006"));
    }
}
