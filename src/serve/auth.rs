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
    provided == expected_token
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
