//! Bearer-token auth middleware for `bastion serve`.
//!
//! Protected routes require an `Authorization: Bearer <token>` header that
//! matches the configured `BASTION_SERVE_TOKEN`.  A missing or invalid token
//! returns **401 Unauthorized**.
//!
//! # Design
//! The comparison lives in the pure [`token_matches`] function so it can be
//! exhaustively unit-tested without spinning up a server.  The actix
//! [`BearerAuth`] extractor wraps it in a thin I/O shell.

use actix_web::body::BoxBody;
use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform, forward_ready};
use actix_web::{Error, HttpResponse};
use futures::future::{LocalBoxFuture, Ready, ok};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};
use subtle::ConstantTimeEq;

use crate::serve::source_auth;

// ── Pure helper (unit-tested) ──────────────────────────────────────────────────

/// Return `true` when `header_value` is a valid `Authorization: Bearer <token>`
/// header matching `expected_token`.
///
/// Accepts:
/// - `header_value` — the raw value of the `Authorization` header (may be absent).
/// - `expected_token` — the server-side secret to compare against.
///
/// Rejects:
/// - Missing header (`None`).
/// - Wrong scheme (anything other than `Bearer `).
/// - Wrong token value.
/// - Empty token in either position.
///
/// # Why constant-time
/// The final comparison uses [`subtle::ConstantTimeEq`] rather than `==`. A
/// plain byte-by-byte `PartialEq` short-circuits on the first differing
/// byte, which leaks the position of that byte through response timing —
/// the classic timing-oracle attack against secret comparison. The early
/// returns above (missing header, wrong scheme, empty token on either side)
/// are structural, not secret-dependent, and are deliberately left
/// short-circuiting; only the final value comparison needs to run in time
/// independent of where the mismatch occurs.
pub fn token_matches(header_value: Option<&str>, expected_token: &str) -> bool {
    let Some(value) = header_value else {
        return false;
    };
    let Some(provided) = value.strip_prefix("Bearer ") else {
        return false;
    };
    // Reject empty expected token (server misconfiguration guard).
    if expected_token.is_empty() || provided.is_empty() {
        return false;
    }
    bool::from(provided.as_bytes().ct_eq(expected_token.as_bytes()))
}

// ── Middleware factory ─────────────────────────────────────────────────────────

/// Actix-web middleware factory that enforces bearer-token authentication,
/// optionally alongside the machine-caller HMAC signature tier
/// ([`crate::serve::source_auth`]).
///
/// Wrap a scope or resource with `BearerAuthMiddleware::new(token)` to require
/// a valid `Authorization: Bearer <token>` on every request to that scope.
/// Chain [`BearerAuthMiddleware::with_signing`] to additionally admit a valid
/// `x-timestamp`/`x-signature` pair as an alternative to the bearer — a
/// request need only satisfy ONE of the two tiers.
#[derive(Clone)]
pub struct BearerAuthMiddleware {
    token: Rc<String>,
    signing_key: Option<Rc<String>>,
    skew_secs: u64,
}

impl BearerAuthMiddleware {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: Rc::new(token.into()),
            signing_key: None,
            skew_secs: 0,
        }
    }

    /// Opt this middleware into also admitting the machine-caller signature
    /// tier ([`crate::serve::source_auth::signature_matches`]) as an
    /// alternative to the bearer token.
    ///
    /// `signing_key: None` (the default — see [`BearerAuthMiddleware::new`])
    /// keeps the middleware bearer-only: a signed-but-unbearer'd request is
    /// rejected exactly as it is today. This is a builder method rather than
    /// a change to `new`'s signature so every pre-existing
    /// `BearerAuthMiddleware::new(TOKEN)` call site keeps compiling with no
    /// edit.
    pub fn with_signing(mut self, key: Option<String>, skew_secs: u64) -> Self {
        self.signing_key = key.map(Rc::new);
        self.skew_secs = skew_secs;
        self
    }
}

impl<S, B> Transform<S, ServiceRequest> for BearerAuthMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: actix_web::body::MessageBody + 'static,
{
    type Response = ServiceResponse<BoxBody>;
    type Error = Error;
    type InitError = ();
    type Transform = BearerAuthService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(BearerAuthService {
            service: Rc::new(service),
            token: self.token.clone(),
            signing_key: self.signing_key.clone(),
            skew_secs: self.skew_secs,
        })
    }
}

// ── Middleware service ─────────────────────────────────────────────────────────

pub struct BearerAuthService<S> {
    service: Rc<S>,
    token: Rc<String>,
    signing_key: Option<Rc<String>>,
    skew_secs: u64,
}

impl<S, B> Service<ServiceRequest> for BearerAuthService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: actix_web::body::MessageBody + 'static,
{
    type Response = ServiceResponse<BoxBody>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let token = self.token.clone();
        let signing_key = self.signing_key.clone();
        let skew_secs = self.skew_secs;
        let svc = self.service.clone();

        Box::pin(async move {
            // Extract the header inside the async block — req is owned here,
            // so no intermediate String allocation is needed.
            let header_value = req
                .headers()
                .get("authorization")
                .and_then(|v| v.to_str().ok());

            let mut matches = token_matches(header_value, &token);

            // Only fall back to the signature tier when the bearer check
            // failed — the bearer check stays first and unchanged. The
            // request body is canonicalised as an EMPTY slice on BOTH the
            // signer and verifier side: actix's `ServiceRequest` does not
            // expose a buffered body at middleware level without a
            // payload-buffering wrapper, which is out of scope for this
            // block. Documenting a scheme as binding the body while
            // verifying an empty one would be worse than no scheme at all —
            // the method and path bindings are real and are what
            // `source_auth`'s canonical-component tests protect.
            if !matches && let Some(key) = signing_key.as_deref() {
                let timestamp_header = req
                    .headers()
                    .get("x-timestamp")
                    .and_then(|v| v.to_str().ok());
                let signature_header = req
                    .headers()
                    .get("x-signature")
                    .and_then(|v| v.to_str().ok());
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);

                matches = source_auth::signature_matches(
                    Some(key),
                    timestamp_header,
                    signature_header,
                    req.method().as_str(),
                    req.path(),
                    &[],
                    now,
                    skew_secs,
                );
            }

            if matches {
                // Map the inner body type to BoxBody so both branches unify.
                svc.call(req).await.map(|r| r.map_into_boxed_body())
            } else {
                let (http_req, _payload) = req.into_parts();
                let resp = HttpResponse::Unauthorized()
                    .json(serde_json::json!({"error": "unauthorized", "code": "unauthorized"}));
                Ok(ServiceResponse::new(http_req, resp).map_into_boxed_body())
            }
        })
    }
}

// ── X-API-Key middleware (engine surface) ───────────────────────────────────────

/// Return `true` when `header_value` is a valid `X-API-Key` header matching
/// `expected_key`.
///
/// Mirrors [`token_matches`]'s structure for the engine surface, which uses
/// an `X-API-Key` header rather than `Authorization: Bearer`. Rejects an
/// empty `expected_key` rather than treating it as "no auth required" — an
/// empty configured key that accepted everything is exactly the failure mode
/// `decide_engine_mount`'s own doc comment warns about.
///
/// Rejects:
/// - Missing header (`None`).
/// - Empty header value.
/// - Empty expected key (server misconfiguration guard).
/// - Wrong key value.
///
/// # Why constant-time
/// Same reasoning as [`token_matches`]: the final comparison uses
/// [`subtle::ConstantTimeEq`] rather than `==` so response timing never
/// leaks the position of the first differing byte. Leaving this variable-time
/// while `token_matches` is constant-time would be a silent asymmetry
/// between the two schemes.
pub fn api_key_matches(header_value: Option<&str>, expected_key: &str) -> bool {
    let Some(provided) = header_value else {
        return false;
    };
    if expected_key.is_empty() || provided.is_empty() {
        return false;
    }
    bool::from(provided.as_bytes().ct_eq(expected_key.as_bytes()))
}

/// Actix-web middleware factory that enforces `X-API-Key` authentication on
/// the embedded engine route surface.
///
/// Wrap a scope or resource with `ApiKeyAuthMiddleware::new(key)` to require
/// a matching `X-API-Key` header on every request to that scope.
#[derive(Clone)]
pub struct ApiKeyAuthMiddleware {
    key: Rc<String>,
}

impl ApiKeyAuthMiddleware {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: Rc::new(key.into()),
        }
    }
}

impl<S, B> Transform<S, ServiceRequest> for ApiKeyAuthMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: actix_web::body::MessageBody + 'static,
{
    type Response = ServiceResponse<BoxBody>;
    type Error = Error;
    type InitError = ();
    type Transform = ApiKeyAuthService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(ApiKeyAuthService {
            service: Rc::new(service),
            key: self.key.clone(),
        })
    }
}

pub struct ApiKeyAuthService<S> {
    service: Rc<S>,
    key: Rc<String>,
}

impl<S, B> Service<ServiceRequest> for ApiKeyAuthService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: actix_web::body::MessageBody + 'static,
{
    type Response = ServiceResponse<BoxBody>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let key = self.key.clone();
        let svc = self.service.clone();

        Box::pin(async move {
            let header_value = req.headers().get("X-API-Key").and_then(|v| v.to_str().ok());

            let matches = api_key_matches(header_value, &key);
            if matches {
                svc.call(req).await.map(|r| r.map_into_boxed_body())
            } else {
                let (http_req, _payload) = req.into_parts();
                let resp = HttpResponse::Unauthorized()
                    .json(serde_json::json!({"error": "unauthorized", "code": "unauthorized"}));
                Ok(ServiceResponse::new(http_req, resp).map_into_boxed_body())
            }
        })
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── token_matches — present header, correct token ──────────────────────

    #[test]
    fn correct_bearer_token_matches() {
        assert!(
            token_matches(Some("Bearer secret123"), "secret123"),
            "exact correct token must match"
        );
    }

    // ── token_matches — absent header ──────────────────────────────────────

    #[test]
    fn missing_header_does_not_match() {
        assert!(
            !token_matches(None, "secret123"),
            "absent header must not match"
        );
    }

    // ── token_matches — wrong scheme ───────────────────────────────────────

    #[test]
    fn basic_scheme_does_not_match() {
        assert!(
            !token_matches(Some("Basic secret123"), "secret123"),
            "Basic scheme must not match"
        );
    }

    #[test]
    fn no_scheme_does_not_match() {
        assert!(
            !token_matches(Some("secret123"), "secret123"),
            "raw token without scheme must not match"
        );
    }

    #[test]
    fn bearer_lowercase_does_not_match() {
        // The scheme prefix is case-sensitive per the implementation.
        assert!(
            !token_matches(Some("bearer secret123"), "secret123"),
            "lowercase 'bearer' scheme must not match (case-sensitive)"
        );
    }

    // ── token_matches — wrong token value ──────────────────────────────────

    #[test]
    fn wrong_token_does_not_match() {
        assert!(
            !token_matches(Some("Bearer wrongtoken"), "secret123"),
            "wrong token value must not match"
        );
    }

    #[test]
    fn partial_token_does_not_match() {
        assert!(
            !token_matches(Some("Bearer secre"), "secret123"),
            "truncated token must not match"
        );
    }

    #[test]
    fn token_with_trailing_space_does_not_match() {
        assert!(
            !token_matches(Some("Bearer secret123 "), "secret123"),
            "token with trailing whitespace must not match"
        );
    }

    // ── token_matches — empty token guards ────────────────────────────────

    #[test]
    fn empty_expected_token_never_matches() {
        assert!(
            !token_matches(Some("Bearer "), ""),
            "empty expected token (server misconfiguration) must not match"
        );
    }

    #[test]
    fn empty_provided_token_does_not_match() {
        assert!(
            !token_matches(Some("Bearer "), "secret123"),
            "empty provided token must not match"
        );
    }

    // ── token_matches — constant-time compare, same/different length ───────

    #[test]
    fn two_same_length_wrong_tokens_are_both_rejected() {
        assert!(
            !token_matches(Some("Bearer aaaaaaaa"), "secret123"),
            "wrong token of the same length as expected must not match"
        );
        assert!(
            !token_matches(Some("Bearer zzzzzzzz"), "secret123"),
            "a different wrong token of the same length as expected must not match"
        );
    }

    #[test]
    fn wrong_token_of_different_length_does_not_match() {
        // ConstantTimeEq on unequal-length slices must still reject —
        // exercised explicitly since a length mismatch is handled before
        // the byte-wise constant-time comparison runs.
        assert!(
            !token_matches(Some("Bearer short"), "a-much-longer-secret-token"),
            "shorter provided token must not match a longer expected token"
        );
        assert!(
            !token_matches(Some("Bearer a-much-longer-provided-token"), "short-secret"),
            "longer provided token must not match a shorter expected token"
        );
    }

    // ── Middleware integration — request-level tests ───────────────────────

    #[actix_web::test]
    async fn middleware_allows_valid_token() {
        use actix_web::{App, HttpResponse, test, web};

        let token = "test-token-abc";
        let app = test::init_service(
            App::new().service(
                web::scope("/protected")
                    .wrap(BearerAuthMiddleware::new(token))
                    .route(
                        "/ping",
                        web::get().to(|| async { HttpResponse::Ok().finish() }),
                    ),
            ),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/protected/ping")
            .insert_header(("authorization", format!("Bearer {token}")))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(
            resp.status(),
            200,
            "valid bearer token must receive 200; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn middleware_rejects_missing_token_with_401() {
        use actix_web::{App, HttpResponse, test, web};

        let app = test::init_service(
            App::new().service(
                web::scope("/protected")
                    .wrap(BearerAuthMiddleware::new("test-token-abc"))
                    .route(
                        "/ping",
                        web::get().to(|| async { HttpResponse::Ok().finish() }),
                    ),
            ),
        )
        .await;

        let req = test::TestRequest::get().uri("/protected/ping").to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(
            resp.status(),
            401,
            "missing token must receive 401; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn middleware_rejects_wrong_token_with_401() {
        use actix_web::{App, HttpResponse, test, web};

        let app = test::init_service(
            App::new().service(
                web::scope("/protected")
                    .wrap(BearerAuthMiddleware::new("correct-token"))
                    .route(
                        "/ping",
                        web::get().to(|| async { HttpResponse::Ok().finish() }),
                    ),
            ),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/protected/ping")
            .insert_header(("authorization", "Bearer wrong-token"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(
            resp.status(),
            401,
            "wrong token must receive 401; got {}",
            resp.status()
        );
    }

    // ── api_key_matches — present header, correct key ──────────────────────

    #[test]
    fn correct_api_key_matches() {
        assert!(
            api_key_matches(Some("secret123"), "secret123"),
            "exact correct key must match"
        );
    }

    // ── api_key_matches — absent header ─────────────────────────────────────

    #[test]
    fn missing_api_key_header_does_not_match() {
        assert!(
            !api_key_matches(None, "secret123"),
            "absent header must not match"
        );
    }

    // ── api_key_matches — empty header ──────────────────────────────────────

    #[test]
    fn empty_api_key_header_does_not_match() {
        assert!(
            !api_key_matches(Some(""), "secret123"),
            "empty header value must not match"
        );
    }

    // ── api_key_matches — wrong key value ───────────────────────────────────

    #[test]
    fn wrong_api_key_does_not_match() {
        assert!(
            !api_key_matches(Some("wrong-key"), "secret123"),
            "wrong key value must not match"
        );
    }

    // ── api_key_matches — empty configured key guard ────────────────────────

    #[test]
    fn empty_expected_api_key_never_matches() {
        // This is the case decide_engine_mount's doc comment warns about: an
        // empty configured key must never be treated as "no auth required".
        assert!(
            !api_key_matches(Some("anything"), ""),
            "empty expected key (server misconfiguration) must not match"
        );
    }

    #[test]
    fn empty_expected_api_key_and_empty_header_does_not_match() {
        assert!(
            !api_key_matches(Some(""), ""),
            "empty expected key with empty header must not match"
        );
    }

    // ── ApiKeyAuthMiddleware — request-level tests ──────────────────────────

    #[actix_web::test]
    async fn api_key_middleware_allows_valid_key() {
        use actix_web::{App, HttpResponse, test, web};

        let key = "engine-key-abc";
        let app = test::init_service(
            App::new().service(
                web::scope("/engine")
                    .wrap(ApiKeyAuthMiddleware::new(key))
                    .route(
                        "/ping",
                        web::get().to(|| async { HttpResponse::Ok().finish() }),
                    ),
            ),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/engine/ping")
            .insert_header(("X-API-Key", key))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(
            resp.status(),
            200,
            "valid api key must receive 200; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn api_key_middleware_rejects_missing_key_with_401() {
        use actix_web::{App, HttpResponse, test, web};

        let app = test::init_service(
            App::new().service(
                web::scope("/engine")
                    .wrap(ApiKeyAuthMiddleware::new("engine-key-abc"))
                    .route(
                        "/ping",
                        web::get().to(|| async { HttpResponse::Ok().finish() }),
                    ),
            ),
        )
        .await;

        let req = test::TestRequest::get().uri("/engine/ping").to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(
            resp.status(),
            401,
            "missing X-API-Key must receive 401; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn api_key_middleware_rejects_wrong_key_with_401() {
        use actix_web::{App, HttpResponse, test, web};

        let app = test::init_service(
            App::new().service(
                web::scope("/engine")
                    .wrap(ApiKeyAuthMiddleware::new("correct-key"))
                    .route(
                        "/ping",
                        web::get().to(|| async { HttpResponse::Ok().finish() }),
                    ),
            ),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/engine/ping")
            .insert_header(("X-API-Key", "wrong-key"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(
            resp.status(),
            401,
            "wrong api key must receive 401; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn api_key_middleware_rejects_empty_configured_key_with_401() {
        use actix_web::{App, HttpResponse, test, web};

        // Even if a caller sends the (empty) key back, an empty configured
        // key must still reject — this is the misconfiguration guard.
        let app = test::init_service(
            App::new().service(
                web::scope("/engine")
                    .wrap(ApiKeyAuthMiddleware::new(""))
                    .route(
                        "/ping",
                        web::get().to(|| async { HttpResponse::Ok().finish() }),
                    ),
            ),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/engine/ping")
            .insert_header(("X-API-Key", ""))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(
            resp.status(),
            401,
            "empty configured key must never accept; got {}",
            resp.status()
        );
    }

    // ── Signature-tier admission (BA.ticket.serve-auth-boundary-freeze task 4) ─

    /// Build the canonical string + HMAC hex digest for `method`/`path` under
    /// `key` at `timestamp`, matching what a real machine caller would send.
    fn sign(key: &str, timestamp: i64, method: &str, path: &str) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        let canonical = source_auth::canonical_string(method, path, &[]);
        let payload = source_auth::signing_payload(timestamp, &canonical);
        let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).unwrap();
        mac.update(payload.as_bytes());
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    #[actix_web::test]
    async fn valid_signature_with_no_authorization_header_is_admitted() {
        use actix_web::{App, HttpResponse, test, web};

        let signing_key = "sign-key-abc";
        let app = test::init_service(
            App::new().service(
                web::scope("/protected")
                    .wrap(
                        BearerAuthMiddleware::new("bearer-token")
                            .with_signing(Some(signing_key.to_owned()), 300),
                    )
                    .route(
                        "/ping",
                        web::get().to(|| async { HttpResponse::Ok().finish() }),
                    ),
            ),
        )
        .await;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let sig = sign(signing_key, now, "GET", "/protected/ping");
        let req = test::TestRequest::get()
            .uri("/protected/ping")
            .insert_header(("x-timestamp", now.to_string()))
            .insert_header(("x-signature", sig))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(
            resp.status(),
            200,
            "valid signature with no Authorization header must reach the route; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn valid_bearer_with_no_signature_still_admitted() {
        use actix_web::{App, HttpResponse, test, web};

        let app = test::init_service(
            App::new().service(
                web::scope("/protected")
                    .wrap(
                        BearerAuthMiddleware::new("bearer-token")
                            .with_signing(Some("sign-key-abc".to_owned()), 300),
                    )
                    .route(
                        "/ping",
                        web::get().to(|| async { HttpResponse::Ok().finish() }),
                    ),
            ),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/protected/ping")
            .insert_header(("authorization", "Bearer bearer-token"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(
            resp.status(),
            200,
            "valid bearer with no signature must reach the route; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn neither_bearer_nor_signature_returns_exact_401_body() {
        use actix_web::{App, HttpResponse, test, web};

        let app = test::init_service(
            App::new().service(
                web::scope("/protected")
                    .wrap(
                        BearerAuthMiddleware::new("bearer-token")
                            .with_signing(Some("sign-key-abc".to_owned()), 300),
                    )
                    .route(
                        "/ping",
                        web::get().to(|| async { HttpResponse::Ok().finish() }),
                    ),
            ),
        )
        .await;

        let req = test::TestRequest::get().uri("/protected/ping").to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 401, "neither credential must 401");
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(
            body,
            serde_json::json!({"error": "unauthorized", "code": "unauthorized"}),
            "401 body must match the documented contract exactly, parsed as JSON"
        );
    }

    #[actix_web::test]
    async fn signature_outside_skew_window_returns_401() {
        use actix_web::{App, HttpResponse, test, web};

        let signing_key = "sign-key-abc";
        let skew_secs = 60u64;
        let app = test::init_service(
            App::new().service(
                web::scope("/protected")
                    .wrap(
                        BearerAuthMiddleware::new("bearer-token")
                            .with_signing(Some(signing_key.to_owned()), skew_secs),
                    )
                    .route(
                        "/ping",
                        web::get().to(|| async { HttpResponse::Ok().finish() }),
                    ),
            ),
        )
        .await;

        // Sign at t=0 but the verifier below effectively sees "now" far in
        // the future because we hand the *stale* timestamp as the header
        // and only need it more than skew_secs away from the true current
        // time — simulate this by signing a timestamp far in the past.
        let stale_ts = 1_000_000_000i64;
        let sig = sign(signing_key, stale_ts, "GET", "/protected/ping");
        let req = test::TestRequest::get()
            .uri("/protected/ping")
            .insert_header(("x-timestamp", stale_ts.to_string()))
            .insert_header(("x-signature", sig))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(
            resp.status(),
            401,
            "a signature timestamped far outside the skew window (relative to real now) must 401; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn app_without_signing_key_boots_and_rejects_signed_requests() {
        use actix_web::{App, HttpResponse, test, web};

        // Bearer-only boot (criterion 6): with_signing is never called, so
        // the app must boot without panicking, serve bearer requests, and
        // 401 a signed-only request.
        let app = test::init_service(
            App::new().service(
                web::scope("/protected")
                    .wrap(BearerAuthMiddleware::new("bearer-token"))
                    .route(
                        "/ping",
                        web::get().to(|| async { HttpResponse::Ok().finish() }),
                    ),
            ),
        )
        .await;

        let bearer_req = test::TestRequest::get()
            .uri("/protected/ping")
            .insert_header(("authorization", "Bearer bearer-token"))
            .to_request();
        let bearer_resp = test::call_service(&app, bearer_req).await;
        assert_eq!(
            bearer_resp.status(),
            200,
            "bearer-only boot must still serve bearer requests"
        );

        let sig = sign(
            "some-key-nobody-configured",
            1_700_000_000,
            "GET",
            "/protected/ping",
        );
        let signed_req = test::TestRequest::get()
            .uri("/protected/ping")
            .insert_header(("x-timestamp", "1700000000"))
            .insert_header(("x-signature", sig))
            .to_request();
        let signed_resp = test::call_service(&app, signed_req).await;
        assert_eq!(
            signed_resp.status(),
            401,
            "a signed request against a bearer-only (no signing key) app must 401"
        );
    }

    #[actix_web::test]
    async fn health_is_reachable_with_no_credentials_alongside_signing() {
        use actix_web::{App, HttpResponse, test, web};

        let app =
            test::init_service(
                App::new()
                    .service(web::resource("/health").route(web::get().to(|| async {
                        HttpResponse::Ok().json(serde_json::json!({"status": "ok"}))
                    })))
                    .service(
                        web::scope("/protected")
                            .wrap(
                                BearerAuthMiddleware::new("bearer-token")
                                    .with_signing(Some("sign-key-abc".to_owned()), 300),
                            )
                            .route(
                                "/ping",
                                web::get().to(|| async { HttpResponse::Ok().finish() }),
                            ),
                    ),
            )
            .await;

        let req = test::TestRequest::get().uri("/health").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            200,
            "GET /health must stay public even when the protected scope carries signing"
        );
    }

    #[actix_web::test]
    async fn api_key_scheme_and_bearer_signature_scheme_do_not_leak_into_each_other() {
        use actix_web::{App, HttpResponse, test, web};

        let bearer_token = "bearer-token";
        let signing_key = "sign-key-abc";
        let engine_key = "engine-key-abc";

        let app = test::init_service(
            App::new()
                .service(
                    web::scope("/api")
                        .wrap(
                            BearerAuthMiddleware::new(bearer_token)
                                .with_signing(Some(signing_key.to_owned()), 300),
                        )
                        .route(
                            "/ping",
                            web::get().to(|| async { HttpResponse::Ok().finish() }),
                        ),
                )
                .service(
                    web::scope("/engine")
                        .wrap(ApiKeyAuthMiddleware::new(engine_key))
                        .route(
                            "/ping",
                            web::get().to(|| async { HttpResponse::Ok().finish() }),
                        ),
                ),
        )
        .await;

        // A valid bastion bearer must not satisfy the engine's X-API-Key scope.
        let engine_req_with_bearer = test::TestRequest::get()
            .uri("/engine/ping")
            .insert_header(("authorization", format!("Bearer {bearer_token}")))
            .to_request();
        let resp1 = test::call_service(&app, engine_req_with_bearer).await;
        assert_eq!(
            resp1.status(),
            401,
            "a valid bastion bearer must not satisfy the engine's X-API-Key scope"
        );

        // A valid bastion signature must not satisfy the engine's X-API-Key scope.
        let sig = sign(signing_key, 1_700_000_000, "GET", "/engine/ping");
        let engine_req_with_sig = test::TestRequest::get()
            .uri("/engine/ping")
            .insert_header(("x-timestamp", "1700000000"))
            .insert_header(("x-signature", sig))
            .to_request();
        let resp2 = test::call_service(&app, engine_req_with_sig).await;
        assert_eq!(
            resp2.status(),
            401,
            "a valid bastion signature must not satisfy the engine's X-API-Key scope"
        );

        // A valid X-API-Key must not satisfy the /api protected scope.
        let api_req_with_key = test::TestRequest::get()
            .uri("/api/ping")
            .insert_header(("X-API-Key", engine_key))
            .to_request();
        let resp3 = test::call_service(&app, api_req_with_key).await;
        assert_eq!(
            resp3.status(),
            401,
            "a valid engine X-API-Key must not satisfy the bastion /api protected scope"
        );
    }

    #[actix_web::test]
    async fn middleware_rejects_bad_scheme_with_401() {
        use actix_web::{App, HttpResponse, test, web};

        let app = test::init_service(
            App::new().service(
                web::scope("/protected")
                    .wrap(BearerAuthMiddleware::new("correct-token"))
                    .route(
                        "/ping",
                        web::get().to(|| async { HttpResponse::Ok().finish() }),
                    ),
            ),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/protected/ping")
            .insert_header(("authorization", "Basic correct-token"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(
            resp.status(),
            401,
            "Basic scheme must receive 401; got {}",
            resp.status()
        );
    }
}
