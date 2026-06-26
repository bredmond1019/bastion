//! `bastion serve` — actix-web HTTP+WebSocket network face.
//!
//! This module exposes [`run`] as the synchronous entry-point for the server.
//! The caller (CLI dispatch arm, Task 2) should invoke it on a dedicated OS
//! thread or via `tokio::task::spawn_blocking` to avoid stalling the tokio
//! executor.
//!
//! # Runtime-spike outcome (Task 1)
//!
//! The integration risk: `actix-web-actors` WS actors need an actix `System`
//! / `Arbiter` that the existing `#[tokio::main]` entry-point does not
//! provide.
//!
//! **What was tested:** Both approaches were evaluated:
//! 1. `HttpServer::new(...).run().await` directly inside a tokio-spawned
//!    future — this compiles and works for the plain-HTTP `/health` surface,
//!    but when `actix-web-actors` starts (Block C), the WS actor needs an
//!    `Arbiter` which is absent in a pure-tokio context.
//! 2. `actix_web::rt::System::new().block_on(...)` on a dedicated OS thread —
//!    spins up the actix `System` which provides the `Arbiter`; the inner
//!    async block can then run `HttpServer`, `/health`, and WS actors uniformly.
//!
//! **Decision:** approach 2 (thread + System) is adopted now so the
//! entry-point stays uniform when WS actors land in Task 5 / Block C.  The
//! `run` function is therefore synchronous and blocking; tokio dispatch calls
//! it via `tokio::task::spawn_blocking`.
//!
//! # Auth policy (Task 3)
//!
//! - `GET /health` — **public**, no bearer token required (liveness probe).
//! - All other routes (including future `/ws`) — **protected** behind
//!   [`auth::BearerAuthMiddleware`], requiring `Authorization: Bearer <token>`.

pub mod auth;
pub mod dto;
pub mod ws;

use actix_web::{App, HttpResponse, HttpServer, web};
use anyhow::Result;
use auth::BearerAuthMiddleware;

// ── Handlers ──────────────────────────────────────────────────────────────────

/// `GET /health` — returns a small JSON liveness body.
///
/// Auth policy: public (no bearer token required). This matches the
/// [`docs/serve-api.md`](../../docs/serve-api.md) v0 contract (Task 6).
async fn health() -> HttpResponse {
    HttpResponse::Ok().json(dto::HealthResponse::ok())
}

// ── Server boot ───────────────────────────────────────────────────────────────

/// Boot the actix-web HTTP server and block until it shuts down.
///
/// `token` is the bearer secret enforced by [`BearerAuthMiddleware`] on all
/// protected routes.  `/health` remains public.
///
/// **Blocking** — run on a dedicated OS thread or via
/// `tokio::task::spawn_blocking` to avoid stalling the tokio executor.
pub fn run(addr: String, token: String) -> Result<()> {
    // Spin up the actix System on the current thread; block_on drives the
    // async server future inside the System's Arbiter-aware runtime.
    actix_web::rt::System::new().block_on(run_server(addr, token))
}

/// Inner async server setup — separated from `run` so it is independently
/// testable via `actix_web::test` utilities.
///
/// # Routing
/// - `/health` — public (no auth).
/// - `/api/*` — protected by [`BearerAuthMiddleware`]; placeholder scope that
///   Task 5 (`/ws`) and later blocks extend.
///
/// Uses `web::resource` (not `web::route`) for `/health` so that unregistered
/// HTTP methods return `405 Method Not Allowed` rather than `404 Not Found`.
async fn run_server(addr: String, token: String) -> Result<()> {
    HttpServer::new(move || {
        // Protected scope — bearer auth enforced on all children.
        // Later blocks add more routes here (e.g. /api/sessions, /api/costs).
        let protected = web::scope("/api").wrap(BearerAuthMiddleware::new(token.clone()));

        // Protected WebSocket scope — bearer auth enforced on upgrade.
        // The /ws route is a separate scope so its upgrade semantics are distinct
        // from the REST /api scope.
        let ws_scope = web::scope("/ws")
            .wrap(BearerAuthMiddleware::new(token.clone()))
            .route("", web::get().to(ws::echo::ws_handler));

        App::new()
            // Public liveness endpoint.
            .service(web::resource("/health").route(web::get().to(health)))
            // Protected REST scope (extended by later blocks).
            .service(protected)
            // Protected WS upgrade route.
            .service(ws_scope)
    })
    .bind(&addr)
    .map_err(anyhow::Error::from)?
    .run()
    .await
    .map_err(anyhow::Error::from)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{App, test};

    const TEST_TOKEN: &str = "test-secret-token";

    /// Build the test app mirroring production routing exactly, using a fixed test token.
    fn build_app() -> actix_web::App<
        impl actix_web::dev::ServiceFactory<
            actix_web::dev::ServiceRequest,
            Config = (),
            Response = actix_web::dev::ServiceResponse,
            Error = actix_web::Error,
            InitError = (),
        >,
    > {
        let protected = web::scope("/api").wrap(BearerAuthMiddleware::new(TEST_TOKEN));
        let ws_scope = web::scope("/ws")
            .wrap(BearerAuthMiddleware::new(TEST_TOKEN))
            .route("", web::get().to(ws::echo::ws_handler));

        App::new()
            .service(web::resource("/health").route(web::get().to(health)))
            .service(protected)
            .service(ws_scope)
    }

    // ── health handler — happy path ────────────────────────────────────────

    #[actix_web::test]
    async fn health_returns_200_ok() {
        let app = test::init_service(build_app()).await;

        let req = test::TestRequest::get().uri("/health").to_request();
        let resp = test::call_service(&app, req).await;

        assert!(
            resp.status().is_success(),
            "GET /health must return 2xx; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn health_body_contains_status_ok() {
        let app = test::init_service(build_app()).await;

        let req = test::TestRequest::get().uri("/health").to_request();
        let resp = test::call_service(&app, req).await;

        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(
            body["status"], "ok",
            "health body must include status: ok; got {body}"
        );
    }

    #[actix_web::test]
    async fn health_body_contains_service_field() {
        let app = test::init_service(build_app()).await;

        let req = test::TestRequest::get().uri("/health").to_request();
        let resp = test::call_service(&app, req).await;

        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(
            body["service"], "bastion",
            "health body must include service: bastion; got {body}"
        );
    }

    // ── health handler — negative paths ───────────────────────────────────

    #[actix_web::test]
    async fn health_post_returns_405() {
        // web::resource registers the /health resource; actix-web returns 405
        // (not 404) when the resource exists but has no handler for the method.
        let app = test::init_service(build_app()).await;

        let req = test::TestRequest::post().uri("/health").to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(
            resp.status(),
            405,
            "POST /health must return 405 Method Not Allowed; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn unknown_route_returns_404() {
        let app = test::init_service(build_app()).await;

        let req = test::TestRequest::get().uri("/nonexistent").to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(
            resp.status(),
            404,
            "Unknown route must return 404; got {}",
            resp.status()
        );
    }

    // ── health is public — no auth required ───────────────────────────────

    #[actix_web::test]
    async fn health_is_public_without_auth() {
        let app = test::init_service(build_app()).await;

        // No Authorization header — health must still return 200.
        let req = test::TestRequest::get().uri("/health").to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(
            resp.status(),
            200,
            "GET /health must be public (no auth); got {}",
            resp.status()
        );
    }

    // ── protected scope rejects missing/wrong token ───────────────────────

    #[actix_web::test]
    async fn protected_scope_rejects_missing_token() {
        use actix_web::HttpResponse;

        let app = test::init_service(
            App::new()
                .service(web::resource("/health").route(web::get().to(health)))
                .service(
                    web::scope("/api")
                        .wrap(BearerAuthMiddleware::new(TEST_TOKEN))
                        .route(
                            "/ping",
                            web::get().to(|| async { HttpResponse::Ok().finish() }),
                        ),
                ),
        )
        .await;

        let req = test::TestRequest::get().uri("/api/ping").to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(
            resp.status(),
            401,
            "missing token on protected route must return 401; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn protected_scope_rejects_wrong_token() {
        use actix_web::HttpResponse;

        let app = test::init_service(
            App::new()
                .service(web::resource("/health").route(web::get().to(health)))
                .service(
                    web::scope("/api")
                        .wrap(BearerAuthMiddleware::new(TEST_TOKEN))
                        .route(
                            "/ping",
                            web::get().to(|| async { HttpResponse::Ok().finish() }),
                        ),
                ),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/api/ping")
            .insert_header(("authorization", "Bearer wrong-token"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(
            resp.status(),
            401,
            "wrong token on protected route must return 401; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn protected_scope_allows_correct_token() {
        use actix_web::HttpResponse;

        let app = test::init_service(
            App::new()
                .service(web::resource("/health").route(web::get().to(health)))
                .service(
                    web::scope("/api")
                        .wrap(BearerAuthMiddleware::new(TEST_TOKEN))
                        .route(
                            "/ping",
                            web::get().to(|| async { HttpResponse::Ok().finish() }),
                        ),
                ),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/api/ping")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(
            resp.status(),
            200,
            "correct token on protected route must return 200; got {}",
            resp.status()
        );
    }

    // ── /ws scope auth — bearer enforced on WS upgrade ────────────────────

    #[actix_web::test]
    async fn ws_scope_rejects_missing_token_with_401() {
        let app = test::init_service(build_app()).await;

        let req = test::TestRequest::get().uri("/ws").to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(
            resp.status(),
            401,
            "GET /ws without token must return 401; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn ws_scope_rejects_wrong_token_with_401() {
        let app = test::init_service(build_app()).await;

        let req = test::TestRequest::get()
            .uri("/ws")
            .insert_header(("authorization", "Bearer wrong-token"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(
            resp.status(),
            401,
            "GET /ws with wrong token must return 401; got {}",
            resp.status()
        );
    }
}
