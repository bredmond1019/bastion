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
use subtle::ConstantTimeEq;

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

/// Actix-web middleware factory that enforces bearer-token authentication.
///
/// Wrap a scope or resource with `BearerAuthMiddleware::new(token)` to require
/// a valid `Authorization: Bearer <token>` on every request to that scope.
#[derive(Clone)]
pub struct BearerAuthMiddleware {
    token: Rc<String>,
}

impl BearerAuthMiddleware {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: Rc::new(token.into()),
        }
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
        })
    }
}

// ── Middleware service ─────────────────────────────────────────────────────────

pub struct BearerAuthService<S> {
    service: Rc<S>,
    token: Rc<String>,
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
        let svc = self.service.clone();

        Box::pin(async move {
            // Extract the header inside the async block — req is owned here,
            // so no intermediate String allocation is needed.
            let header_value = req
                .headers()
                .get("authorization")
                .and_then(|v| v.to_str().ok());

            let matches = token_matches(header_value, &token);
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
