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

use actix_web::{App, HttpResponse, HttpServer, web};
use anyhow::Result;
use serde_json::json;

// ── Handlers ──────────────────────────────────────────────────────────────────

/// `GET /health` — returns a small JSON liveness body.
///
/// Auth policy: public (no bearer token required). This matches the
/// [`docs/serve-api.md`](../../docs/serve-api.md) v0 contract (Task 6).
async fn health() -> HttpResponse {
    HttpResponse::Ok().json(json!({
        "status": "ok",
        "service": "bastion"
    }))
}

// ── Server boot ───────────────────────────────────────────────────────────────

/// Boot the actix-web HTTP server and block until it shuts down.
///
/// The `token` parameter is threaded through the signature now (stable API
/// surface) and will be consumed by the bearer-auth middleware added in Task 3.
/// It is intentionally unused in Task 1.
///
/// **Blocking** — run on a dedicated OS thread or via
/// `tokio::task::spawn_blocking` to avoid stalling the tokio executor.
pub fn run(addr: String, _token: String) -> Result<()> {
    // Spin up the actix System on the current thread; block_on drives the
    // async server future inside the System's Arbiter-aware runtime.
    actix_web::rt::System::new().block_on(run_server(addr))
}

/// Inner async server setup — separated from `run` so it is independently
/// testable via `actix_web::test` utilities.
///
/// Uses `web::resource` (not `web::route`) so that unregistered HTTP methods
/// on `/health` return `405 Method Not Allowed` rather than `404 Not Found`.
async fn run_server(addr: String) -> Result<()> {
    HttpServer::new(|| App::new().service(web::resource("/health").route(web::get().to(health))))
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

    /// Build the test app with the same resource-based routing as production.
    /// Using `web::resource` ensures POST→405 rather than 404.
    fn build_app() -> actix_web::App<
        impl actix_web::dev::ServiceFactory<
            actix_web::dev::ServiceRequest,
            Config = (),
            Response = actix_web::dev::ServiceResponse,
            Error = actix_web::Error,
            InitError = (),
        >,
    > {
        App::new().service(web::resource("/health").route(web::get().to(health)))
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
}
