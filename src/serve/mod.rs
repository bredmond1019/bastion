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
pub mod handlers;
pub mod poll;
pub mod status;
pub mod ws;

use crate::config::{FileConfig, load_workspace_registry};
use actix::{Actor, Addr};
use actix_web::{App, HttpRequest, HttpResponse, HttpServer, web};
use actix_web_actors::ws as actix_ws;
use anyhow::Result;
use auth::BearerAuthMiddleware;
use dto::ErrorPayload;
use engine_serve::abort::RunRegistry;
use engine_serve::dispatch::Dispatcher;
use engine_serve::durable::spawn_durable_writer;
use engine_serve::http::AppState as EngineAppState;
use engine_serve::live_state::LiveStateStore;
use std::sync::Arc;
use ws::server::Hub;

// ── Engine embed (BA.7.C task 2) ────────────────────────────────────────────
//
// `bastion serve` embeds `engine-serve`'s route table (D48: the abort endpoint
// and the rest of the engine surface are served through `bastion serve`, not
// the Python orchestrator). See the block's *Scope growth* section in
// `planning/7.C-cost-budget-alerts-abort/tasks.md`.

/// Whether — and why — the embedded engine's route table should be mounted
/// this boot, given the two config values it needs.
///
/// Pure function — no I/O, no env access — so the decision itself is directly
/// unit-testable; only the env-var reads and the `PgPool`/`HttpServer` setup
/// around it are the thin I/O shell (Rule 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineMountDecision {
    /// Both `DATABASE_URL` and the engine API key are present (non-empty) —
    /// mount the engine's route table using these values.
    Mount {
        database_url: String,
        engine_api_key: String,
    },
    /// At least one required value is absent (or empty) — leave the engine
    /// routes unmounted this boot; `reason` names what was missing.
    Skip { reason: String },
}

/// Decide whether to mount the embedded engine's route table, given the two
/// values it needs: `DATABASE_URL` (for the durable writer's `PgPool`) and
/// the engine's `X-API-Key` secret. Both are absent-tolerant: `bastion serve`
/// must still boot its existing session/status surface with the engine
/// routes unmounted (and say so) rather than fail to boot or mount a route
/// that would 500 on every request.
///
/// A present-but-empty-string value is treated the same as absent — an
/// empty `X-API-Key` would accept every request (see `check_api_key`'s
/// exact-match semantics), which is never the intended configuration.
pub fn decide_engine_mount(
    database_url: Option<&str>,
    engine_api_key: Option<&str>,
) -> EngineMountDecision {
    let database_url = database_url.filter(|s| !s.is_empty());
    let engine_api_key = engine_api_key.filter(|s| !s.is_empty());

    match (database_url, engine_api_key) {
        (Some(database_url), Some(engine_api_key)) => EngineMountDecision::Mount {
            database_url: database_url.to_string(),
            engine_api_key: engine_api_key.to_string(),
        },
        (database_url, engine_api_key) => {
            let mut missing = Vec::new();
            if database_url.is_none() {
                missing.push("DATABASE_URL");
            }
            if engine_api_key.is_none() {
                missing.push("BASTION_ENGINE_API_KEY (engine_api_key)");
            }
            EngineMountDecision::Skip {
                reason: format!(
                    "engine routes not mounted (POST /events/, GET /workflows, \
                     POST /events/{{run_id}}/abort, etc. are unavailable this boot) — \
                     missing: {}",
                    missing.join(", ")
                ),
            }
        }
    }
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// `GET /health` — returns a small JSON liveness body.
///
/// Auth policy: public (no bearer token required). This matches the
/// [`docs/serve-api.md`](../../docs/serve-api.md) v0 contract (Task 6).
async fn health() -> HttpResponse {
    HttpResponse::Ok().json(dto::HealthResponse::ok())
}

/// `GET /ws` — WebSocket upgrade handler (v0.2, hub-backed).
///
/// Upgrades the HTTP connection to a WebSocket and starts a [`ws::session::WsConn`]
/// actor linked to the shared [`Hub`].  The bearer middleware wrapping the `/ws`
/// scope enforces auth before this handler is reached.
async fn hub_ws_handler(
    req: HttpRequest,
    stream: web::Payload,
    hub: web::Data<Addr<Hub>>,
) -> Result<HttpResponse, actix_web::Error> {
    actix_ws::start(
        ws::session::WsConn::new(hub.get_ref().clone()),
        &req,
        stream,
    )
}

// ── Malformed-body contract (Gap 5) ─────────────────────────────────────────

/// Build the `web::JsonConfig` that maps a failed `web::Json<T>` deserialize
/// (unknown enum variant, wrong-typed field, non-JSON body) to the project's
/// `400` + `ErrorPayload { code: "C006", .. }` contract instead of actix's
/// default plain-text 400.
///
/// Shared by both [`run_server`]'s production `App` and the test `build_app`
/// so the two exercise identical behaviour (Rule 6: the closure itself is a
/// thin I/O shell around the pure [`ErrorPayload`] shape).
fn json_config() -> web::JsonConfig {
    web::JsonConfig::default().error_handler(|err, _req| {
        let message = err.to_string();
        actix_web::error::InternalError::from_response(
            err,
            HttpResponse::BadRequest().json(ErrorPayload {
                code: "C006".to_owned(),
                message,
            }),
        )
        .into()
    })
}

// ── Server boot ───────────────────────────────────────────────────────────────

/// Boot the actix-web HTTP server and block until it shuts down.
///
/// `token` is the bearer secret enforced by [`BearerAuthMiddleware`] on all
/// protected routes.  `/health` remains public.
///
/// `poll_secs` sets the hub's poll cadence for sessions-list and pane pushes
/// (sourced from `BASTION_POLL_INTERVAL`, defaulting to 2).
///
/// **Blocking** — run on a dedicated OS thread or via
/// `tokio::task::spawn_blocking` to avoid stalling the tokio executor.
pub fn run(addr: String, token: String) -> Result<()> {
    // Read poll cadence from env (BASTION_POLL_INTERVAL), defaulting to 2s.
    let poll_secs: u64 = std::env::var("BASTION_POLL_INTERVAL")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);

    // Spin up the actix System on the current thread; block_on drives the
    // async server future inside the System's Arbiter-aware runtime.
    actix_web::rt::System::new().block_on(run_server(addr, token, poll_secs))
}

/// Inner async server setup — separated from `run` so it is independently
/// testable via `actix_web::test` utilities.
///
/// # Routing
/// - `/health` — public (no auth).
/// - `/api/*` — protected by [`BearerAuthMiddleware`]; session REST surface.
/// - `/ws` — protected WebSocket upgrade; hub-backed since v0.2.
///
/// Uses `web::resource` (not `web::route`) for `/health` so that unregistered
/// HTTP methods return `405 Method Not Allowed` rather than `404 Not Found`.
///
/// `poll_secs` is passed to the [`Hub`] to set its poll cadence.
async fn run_server(addr: String, token: String, poll_secs: u64) -> Result<()> {
    // Load the workspace registry once at startup (BA.11.D) — malformed or
    // absent config degrades to an empty registry rather than failing boot,
    // matching `load_workspace_registry`'s own degradation contract.
    let registry: FileConfig = load_workspace_registry(
        std::env::var("XDG_CONFIG_HOME").ok(),
        std::env::var("HOME").ok(),
    )
    .unwrap_or_default();

    // Start the hub actor once (process-singleton within this actix System).
    // All per-connection WsConn actors hold an Addr<Hub> clone.
    let hub = Hub::new(poll_secs, registry.clone()).start();

    let registry = web::Data::new(registry);

    // ── Engine embed (BA.7.C task 2) ────────────────────────────────────────
    //
    // Decide once at boot whether to mount `engine-serve`'s route table.
    // Absent-tolerant: with `DATABASE_URL` or the engine API key missing,
    // `bastion serve` still starts its existing session/status surface —
    // the engine routes are simply left unmounted, and we say so on stderr
    // plus an `observ` event rather than failing to boot or mounting a route
    // that would 500 on every request.
    let engine_data: Option<web::Data<EngineAppState>> = match decide_engine_mount(
        std::env::var("DATABASE_URL").ok().as_deref(),
        std::env::var("BASTION_ENGINE_API_KEY").ok().as_deref(),
    ) {
        EngineMountDecision::Mount {
            database_url,
            engine_api_key,
        } => {
            // One shared sqlx/PgPool — engine-rs is aligned on sqlx 0.9 with
            // bastion (see the spec's *Dependency alignment* section), so no
            // two-pool `engine_store::connect` workaround is needed here.
            match sqlx::postgres::PgPoolOptions::new()
                .max_connections(1)
                .connect(&database_url)
                .await
            {
                Ok(pool) => {
                    tracing::info!(
                        target: "bastion::serve",
                        "engine routes mounted (DATABASE_URL + engine_api_key present)"
                    );
                    let state = EngineAppState {
                        dispatcher: Arc::new(Dispatcher::new()),
                        live: LiveStateStore::new(),
                        durable: spawn_durable_writer(Some(pool)),
                        runs: RunRegistry::new(),
                        api_key: engine_api_key,
                    };
                    Some(web::Data::new(state))
                }
                Err(e) => {
                    tracing::error!(
                        target: "bastion::serve",
                        error = %e,
                        "engine routes not mounted — failed to connect to DATABASE_URL"
                    );
                    eprintln!(
                        "bastion serve: engine routes not mounted — could not connect to \
                         DATABASE_URL: {e}"
                    );
                    None
                }
            }
        }
        EngineMountDecision::Skip { reason } => {
            tracing::warn!(target: "bastion::serve", %reason);
            eprintln!("bastion serve: {reason}");
            None
        }
    };

    HttpServer::new(move || {
        let hub_data = web::Data::new(hub.clone());
        let registry_data = registry.clone();
        let engine_data = engine_data.clone();

        // Protected scope — bearer auth enforced on all children.
        //
        // Session routes use `web::resource()` (not bare `.route()`) so that
        // actix-web returns 405 Method Not Allowed when the path matches but
        // the HTTP method is not registered — bare `.route()` would silently
        // return 404 in that case.
        let protected = web::scope("/api")
            .wrap(BearerAuthMiddleware::new(token.clone()))
            // ── Session routes ──────────────────────────────────────────────
            // /sessions — GET (list) + POST (create)
            .service(
                web::resource("/sessions")
                    .route(web::get().to(handlers::sessions::list_sessions))
                    .route(web::post().to(handlers::sessions::create_session)),
            )
            // /sessions/{name}/pane — GET only
            .service(
                web::resource("/sessions/{name}/pane")
                    .route(web::get().to(handlers::sessions::get_pane)),
            )
            // /sessions/{name}/send — POST only
            .service(
                web::resource("/sessions/{name}/send")
                    .route(web::post().to(handlers::sessions::send)),
            )
            // /sessions/{name}/key — POST only
            .service(
                web::resource("/sessions/{name}/key")
                    .route(web::post().to(handlers::sessions::send_key)),
            )
            // /sessions/{name} — DELETE only
            .service(
                web::resource("/sessions/{name}")
                    .route(web::delete().to(handlers::sessions::delete_session)),
            )
            // ── Repo / workflow status routes (BA.11.D) ─────────────────────
            // /repos — GET (list workspace registry entries)
            .service(web::resource("/repos").route(web::get().to(handlers::status::list_repos)))
            // /repos/{name}/status — GET only
            .service(
                web::resource("/repos/{name}/status")
                    .route(web::get().to(handlers::status::get_repo_status)),
            )
            // /repos/{name}/handoff — GET only
            .service(
                web::resource("/repos/{name}/handoff")
                    .route(web::get().to(handlers::status::get_repo_handoff)),
            )
            // /repos/{name}/workflows — GET only
            .service(
                web::resource("/repos/{name}/workflows")
                    .route(web::get().to(handlers::status::get_repo_workflows)),
            )
            // ── Quick-action command route (BA.11.E) ────────────────────────
            // /actions/command — POST only
            .service(
                web::resource("/actions/command").route(web::post().to(handlers::actions::command)),
            )
            // ── Cross-brain board route (BA.11.K) ───────────────────────────
            // /board — GET only (now/next/blocked/finished rollup)
            .service(web::resource("/board").route(web::get().to(handlers::board::get_board)));

        // Protected WebSocket scope — bearer auth enforced on upgrade.
        // v0.2: route backed by hub + WsConn (replaces echo actor).
        let ws_scope = web::scope("/ws")
            .wrap(BearerAuthMiddleware::new(token.clone()))
            .app_data(hub_data.clone())
            .route("", web::get().to(hub_ws_handler));

        let mut app = App::new()
            // Shared hub data — accessible to hub_ws_handler via web::Data<Addr<Hub>>.
            .app_data(hub_data)
            // Shared workspace registry — accessible to status handlers via
            // web::Data<FileConfig> (BA.11.D).
            .app_data(registry_data)
            // Malformed request bodies (unknown enum variant, wrong-typed
            // field, non-JSON) get the C0xx ErrorPayload contract instead of
            // actix's default plain-text 400 (Gap 5).
            .app_data(json_config())
            // Public liveness endpoint.
            //
            // `/health` collision (BA.7.C task 2): `engine_serve::http::configure`
            // (mounted below when the engine is present) registers its own
            // `GET /health`. actix-web resolves duplicate exact-path resources by
            // first-registration-wins (verified empirically — the second
            // registration is simply unreachable, not a panic), so registering
            // bastion's own `/health` *before* `.configure(engine_serve::http::configure)`
            // deliberately keeps bastion's own liveness contract
            // (`docs/serve-api.md`) unchanged for existing consumers: the whole
            // process's `/health` always answers, engine-mounted or not.
            .service(web::resource("/health").route(web::get().to(health)))
            // Protected REST scope (extended by later blocks).
            .service(protected)
            // Protected WS upgrade route.
            .service(ws_scope);

        // Mount the embedded engine's route table when config allows it
        // (BA.7.C task 2). These routes are NOT wrapped in bastion's own
        // `Bearer` middleware — they carry their own `X-API-Key` gate
        // (`engine_serve::http::check_api_key`), and double-gating them would
        // break the pinned contract's 401 semantics (a caller supplying only
        // `X-API-Key` would otherwise be rejected by bastion's Bearer layer
        // before ever reaching the engine handler).
        if let Some(engine_data) = engine_data {
            app = app
                .app_data(engine_data)
                .configure(engine_serve::http::configure);
        }

        app
    })
    .bind(&addr)
    .map_err(anyhow::Error::from)?
    .run()
    .await
    .map_err(anyhow::Error::from)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

// ── decide_engine_mount tests (BA.7.C task 2) ───────────────────────────────
//
// Kept in a dedicated module (rather than inside `mod tests` below) because
// that module does `use actix_web::{App, test};`, which brings `actix_web`'s
// `#[test]` attribute macro into scope under the bare name `test` and shadows
// the built-in `#[test]` attribute — a plain `#[test] fn ...` (sync) in that
// module resolves to actix's async-only test macro and fails to compile.
#[cfg(test)]
mod engine_mount_tests {
    use super::*;

    #[test]
    fn decide_engine_mount_mounts_when_both_present() {
        let decision =
            decide_engine_mount(Some("postgres://localhost/db"), Some("engine-secret-key"));
        assert_eq!(
            decision,
            EngineMountDecision::Mount {
                database_url: "postgres://localhost/db".to_string(),
                engine_api_key: "engine-secret-key".to_string(),
            }
        );
    }

    #[test]
    fn decide_engine_mount_skips_when_database_url_absent() {
        let decision = decide_engine_mount(None, Some("engine-secret-key"));
        match decision {
            EngineMountDecision::Skip { reason } => {
                assert!(reason.contains("DATABASE_URL"));
                assert!(!reason.contains("BASTION_ENGINE_API_KEY"));
            }
            other => panic!("expected Skip, got {other:?}"),
        }
    }

    #[test]
    fn decide_engine_mount_skips_when_engine_api_key_absent() {
        let decision = decide_engine_mount(Some("postgres://localhost/db"), None);
        match decision {
            EngineMountDecision::Skip { reason } => {
                assert!(reason.contains("BASTION_ENGINE_API_KEY"));
                assert!(!reason.contains("missing: DATABASE_URL"));
            }
            other => panic!("expected Skip, got {other:?}"),
        }
    }

    #[test]
    fn decide_engine_mount_skips_when_both_absent() {
        let decision = decide_engine_mount(None, None);
        match decision {
            EngineMountDecision::Skip { reason } => {
                assert!(reason.contains("DATABASE_URL"));
                assert!(reason.contains("BASTION_ENGINE_API_KEY"));
            }
            other => panic!("expected Skip, got {other:?}"),
        }
    }

    #[test]
    fn decide_engine_mount_treats_empty_database_url_as_absent() {
        let decision = decide_engine_mount(Some(""), Some("engine-secret-key"));
        match decision {
            EngineMountDecision::Skip { reason } => {
                assert!(reason.contains("DATABASE_URL"));
            }
            other => panic!("expected Skip, got {other:?}"),
        }
    }

    #[test]
    fn decide_engine_mount_treats_empty_engine_api_key_as_absent() {
        let decision = decide_engine_mount(Some("postgres://localhost/db"), Some(""));
        match decision {
            EngineMountDecision::Skip { reason } => {
                assert!(reason.contains("BASTION_ENGINE_API_KEY"));
            }
            other => panic!("expected Skip, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{App, test};

    const TEST_TOKEN: &str = "test-secret-token";

    /// Build the test app mirroring production routing exactly, using a fixed test token
    /// and the given workspace registry (use `FileConfig::default()` for tests that don't
    /// exercise the repo/workflow status routes).
    ///
    /// Must be called from within an actix test context (`#[actix_web::test]`) so that
    /// `Hub::start()` can register with the current actix System arbiter.
    fn build_app(
        registry: FileConfig,
    ) -> actix_web::App<
        impl actix_web::dev::ServiceFactory<
            actix_web::dev::ServiceRequest,
            Config = (),
            Response = actix_web::dev::ServiceResponse,
            Error = actix_web::Error,
            InitError = (),
        >,
    > {
        // Start a hub for test routing — mirrors production (Hub::start inside the actix System).
        let hub = Hub::new(2, registry.clone()).start();
        let hub_data = web::Data::new(hub);
        let registry_data = web::Data::new(registry);

        // Mirror production routing exactly (same web::resource groupings for
        // correct 405 behaviour on wrong methods).
        let protected = web::scope("/api")
            .wrap(BearerAuthMiddleware::new(TEST_TOKEN))
            .service(
                web::resource("/sessions")
                    .route(web::get().to(handlers::sessions::list_sessions))
                    .route(web::post().to(handlers::sessions::create_session)),
            )
            .service(
                web::resource("/sessions/{name}/pane")
                    .route(web::get().to(handlers::sessions::get_pane)),
            )
            .service(
                web::resource("/sessions/{name}/send")
                    .route(web::post().to(handlers::sessions::send)),
            )
            .service(
                web::resource("/sessions/{name}/key")
                    .route(web::post().to(handlers::sessions::send_key)),
            )
            .service(
                web::resource("/sessions/{name}")
                    .route(web::delete().to(handlers::sessions::delete_session)),
            )
            .service(web::resource("/repos").route(web::get().to(handlers::status::list_repos)))
            .service(
                web::resource("/repos/{name}/status")
                    .route(web::get().to(handlers::status::get_repo_status)),
            )
            .service(
                web::resource("/repos/{name}/handoff")
                    .route(web::get().to(handlers::status::get_repo_handoff)),
            )
            .service(
                web::resource("/repos/{name}/workflows")
                    .route(web::get().to(handlers::status::get_repo_workflows)),
            )
            .service(
                web::resource("/actions/command").route(web::post().to(handlers::actions::command)),
            )
            .service(web::resource("/board").route(web::get().to(handlers::board::get_board)));
        let ws_scope = web::scope("/ws")
            .wrap(BearerAuthMiddleware::new(TEST_TOKEN))
            .app_data(hub_data.clone())
            .route("", web::get().to(hub_ws_handler));

        App::new()
            .app_data(hub_data)
            .app_data(registry_data)
            .app_data(json_config())
            .service(web::resource("/health").route(web::get().to(health)))
            .service(protected)
            .service(ws_scope)
    }

    // ── health handler — happy path ────────────────────────────────────────

    #[actix_web::test]
    async fn health_returns_200_ok() {
        let app = test::init_service(build_app(FileConfig::default())).await;

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
        let app = test::init_service(build_app(FileConfig::default())).await;

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
        let app = test::init_service(build_app(FileConfig::default())).await;

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
        let app = test::init_service(build_app(FileConfig::default())).await;

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
        let app = test::init_service(build_app(FileConfig::default())).await;

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
        let app = test::init_service(build_app(FileConfig::default())).await;

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

    // ── session routes — bearer auth enforced ─────────────────────────────

    #[actix_web::test]
    async fn get_sessions_rejects_missing_token_with_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get().uri("/api/sessions").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            401,
            "GET /api/sessions without token must return 401; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn get_sessions_rejects_wrong_token_with_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get()
            .uri("/api/sessions")
            .insert_header(("authorization", "Bearer wrong-token"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            401,
            "GET /api/sessions with wrong token must return 401; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn get_sessions_with_valid_token_returns_200_json_array() {
        // Live tmux behaviour is smoke-tested, not asserted in-process (Rule 6).
        // This test only verifies that the route is wired and produces a JSON
        // array (empty when tmux is not running in CI — list_sessions_raw
        // returns an error that the handler maps to 503, OR no sessions exist
        // and we get 200 []).  We accept either: 200 with array OR 503.
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get()
            .uri("/api/sessions")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        let status = resp.status().as_u16();
        assert!(
            status == 200 || status == 503,
            "GET /api/sessions must return 200 or 503; got {status}"
        );
        if status == 200 {
            let body: serde_json::Value = test::read_body_json(resp).await;
            assert!(
                body.is_array(),
                "GET /api/sessions 200 body must be a JSON array; got {body}"
            );
        }
    }

    #[actix_web::test]
    async fn post_sessions_send_rejects_missing_token_with_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::post()
            .uri("/api/sessions/work/send")
            .set_json(serde_json::json!({"keys": "hello"}))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            401,
            "POST /api/sessions/work/send without token must return 401; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn post_sessions_key_rejects_missing_token_with_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::post()
            .uri("/api/sessions/work/key")
            .set_json(serde_json::json!({"key": "Escape"}))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            401,
            "POST /api/sessions/work/key without token must return 401; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn delete_session_rejects_missing_token_with_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::delete()
            .uri("/api/sessions/work")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            401,
            "DELETE /api/sessions/work without token must return 401; got {}",
            resp.status()
        );
    }

    // ── session routes — method/path mapping ──────────────────────────────

    #[actix_web::test]
    async fn put_sessions_returns_405_method_not_allowed() {
        // actix-web returns 405 when a path is registered (GET + POST on
        // /api/sessions) but the requested method (PUT) is not.
        // This verifies route wiring: correct paths registered, wrong method
        // → 405 not 404.
        // Auth check happens after method dispatch, so we include the token to
        // ensure the 405 is from method matching, not auth rejection.
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::put()
            .uri("/api/sessions")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            405,
            "PUT /api/sessions (unregistered method) must return 405; got {}",
            resp.status()
        );
    }

    // ── /ws scope auth — bearer enforced on WS upgrade ────────────────────

    #[actix_web::test]
    async fn ws_scope_rejects_missing_token_with_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;

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
        let app = test::init_service(build_app(FileConfig::default())).await;

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

    #[actix_web::test]
    async fn ws_scope_upgrade_succeeds_with_valid_token() {
        // With a valid bearer token and proper WebSocket upgrade headers the
        // handler calls actix_ws::start(WsConn::new(hub), ...) which returns
        // 101 Switching Protocols.  This asserts auth passes and the hub-backed
        // handler is correctly wired (not the old echo actor).
        let app = test::init_service(build_app(FileConfig::default())).await;

        let req = test::TestRequest::get()
            .uri("/ws")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .insert_header(("connection", "Upgrade"))
            .insert_header(("upgrade", "websocket"))
            .insert_header(("sec-websocket-version", "13"))
            // A valid base64-encoded 16-byte nonce (per RFC 6455).
            .insert_header(("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ=="))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(
            resp.status(),
            101,
            "GET /ws with valid token and WS upgrade headers must return 101; got {}",
            resp.status()
        );
    }

    // ── repo/workflow status routes (BA.11.D) ──────────────────────────────

    /// Minimal temp-dir helper that cleans up on drop (avoids adding `tempfile` dep
    /// — mirrors `src/validate/mod.rs` / `src/serve/handlers/status.rs` test helpers).
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            let dir = std::env::temp_dir().join(format!("bastion_serve_mod_test_{pid}_{id}"));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const STATUS_MD: &str = include_str!("status/fixtures/status_well_formed.md");
    const HANDOFF_MD: &str = include_str!("status/fixtures/handoff_minimal.md");
    const FLOW_JSON: &str = include_str!("status/fixtures/flow_state_valid.json");

    fn write_fixture(path: &std::path::Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    /// Build a [`FileConfig`] registering a single workspace named `repo-x`
    /// rooted at a freshly populated temp dir containing `status.md`,
    /// `handoff.md`, and one `sdlc-flow-state.json` fixture.
    fn registry_with_fixture_repo() -> (TempDir, FileConfig) {
        let tmp = TempDir::new();
        write_fixture(&tmp.path().join("planning/status.md"), STATUS_MD);
        write_fixture(&tmp.path().join("planning/handoff.md"), HANDOFF_MD);
        write_fixture(
            &tmp.path()
                .join("planning/phase6-blockA/sdlc/sdlc-flow-state.json"),
            FLOW_JSON,
        );

        let mut workspaces = std::collections::HashMap::new();
        workspaces.insert("repo-x".to_string(), tmp.path().to_path_buf());
        let registry = FileConfig {
            workspaces: Some(workspaces),
            ..Default::default()
        };
        (tmp, registry)
    }

    #[actix_web::test]
    async fn get_repos_rejects_missing_token_with_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get().uri("/api/repos").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 401);
    }

    #[actix_web::test]
    async fn get_repo_status_rejects_missing_token_with_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get()
            .uri("/api/repos/repo-x/status")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 401);
    }

    #[actix_web::test]
    async fn get_repo_handoff_rejects_missing_token_with_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get()
            .uri("/api/repos/repo-x/handoff")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 401);
    }

    #[actix_web::test]
    async fn get_repo_workflows_rejects_missing_token_with_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get()
            .uri("/api/repos/repo-x/workflows")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 401);
    }

    #[actix_web::test]
    async fn get_repo_status_unknown_repo_returns_404() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get()
            .uri("/api/repos/no-such-repo/status")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 404);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "C005");
    }

    #[actix_web::test]
    async fn get_repo_handoff_unknown_repo_returns_404() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get()
            .uri("/api/repos/no-such-repo/handoff")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 404);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "C005");
    }

    #[actix_web::test]
    async fn get_repo_workflows_unknown_repo_returns_404() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get()
            .uri("/api/repos/no-such-repo/workflows")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 404);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "C005");
    }

    /// Gap 4: the two handoff 404 paths must be distinguishable by error
    /// code — an unregistered workspace name (`C005`, config/registry miss)
    /// vs a registered repo whose `handoff.md` is simply absent (`C002`).
    #[actix_web::test]
    async fn get_repo_handoff_unknown_repo_vs_missing_handoff_have_distinct_codes() {
        // Unknown repo (not in registry at all) -> 404 + C005.
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get()
            .uri("/api/repos/no-such-repo/handoff")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 404);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "C005");

        // Registered repo with no handoff.md fixture written -> 404 + C002.
        let tmp = TempDir::new();
        write_fixture(&tmp.path().join("planning/status.md"), STATUS_MD);
        let mut workspaces = std::collections::HashMap::new();
        workspaces.insert("repo-no-handoff".to_string(), tmp.path().to_path_buf());
        let registry = FileConfig {
            workspaces: Some(workspaces),
            ..Default::default()
        };
        let app = test::init_service(build_app(registry)).await;
        let req = test::TestRequest::get()
            .uri("/api/repos/repo-no-handoff/handoff")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 404);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "C002");
    }

    #[actix_web::test]
    async fn get_repos_with_valid_token_returns_200_json_array() {
        let (_tmp, registry) = registry_with_fixture_repo();
        let app = test::init_service(build_app(registry)).await;
        let req = test::TestRequest::get()
            .uri("/api/repos")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = test::read_body_json(resp).await;
        assert!(
            body.is_array(),
            "GET /api/repos body must be an array; got {body}"
        );
        assert_eq!(body[0]["name"], "repo-x");
        assert_eq!(body[0]["has_handoff"], true);
    }

    #[actix_web::test]
    async fn get_repo_status_with_valid_token_returns_200() {
        let (_tmp, registry) = registry_with_fixture_repo();
        let app = test::init_service(build_app(registry)).await;
        let req = test::TestRequest::get()
            .uri("/api/repos/repo-x/status")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["name"], "repo-x");
        assert_eq!(body["has_handoff"], true);
        assert_eq!(body["momentum_next"], "Wire WS event push");
    }

    #[actix_web::test]
    async fn get_repo_handoff_with_valid_token_returns_200() {
        let (_tmp, registry) = registry_with_fixture_repo();
        let app = test::init_service(build_app(registry)).await;
        let req = test::TestRequest::get()
            .uri("/api/repos/repo-x/handoff")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["title"], "Handoff — minimal fixture");
        assert!(body["body"].as_str().unwrap().contains("read_handoff"));
    }

    #[actix_web::test]
    async fn get_repo_workflows_with_valid_token_returns_200_array() {
        let (_tmp, registry) = registry_with_fixture_repo();
        let app = test::init_service(build_app(registry)).await;
        let req = test::TestRequest::get()
            .uri("/api/repos/repo-x/workflows")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = test::read_body_json(resp).await;
        assert!(body.is_array());
        assert_eq!(body[0]["spec_slug"], "phase6-blockA");
        assert_eq!(body[0]["status"], "done");
    }

    #[actix_web::test]
    async fn get_repo_workflows_empty_planning_dir_returns_200_empty_array() {
        let tmp = TempDir::new();
        let mut workspaces = std::collections::HashMap::new();
        workspaces.insert("repo-empty".to_string(), tmp.path().to_path_buf());
        let registry = FileConfig {
            workspaces: Some(workspaces),
            ..Default::default()
        };
        let app = test::init_service(build_app(registry)).await;
        let req = test::TestRequest::get()
            .uri("/api/repos/repo-empty/workflows")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body, serde_json::json!([]));
    }

    // ── /api/actions/command — route registration (BA.11.E) ────────────────

    #[actix_web::test]
    async fn actions_command_rejects_missing_token_with_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::post()
            .uri("/api/actions/command")
            .set_json(serde_json::json!({
                "mode": "inject",
                "session": "main",
                "command": "/status"
            }))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            401,
            "POST /api/actions/command without token must return 401; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn actions_command_rejects_wrong_token_with_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::post()
            .uri("/api/actions/command")
            .insert_header(("authorization", "Bearer wrong-token"))
            .set_json(serde_json::json!({
                "mode": "inject",
                "session": "main",
                "command": "/status"
            }))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            401,
            "POST /api/actions/command with wrong token must return 401; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn actions_command_wrong_method_returns_405() {
        // web::resource registers /actions/command with only POST — GET must
        // return 405 (not 404), matching the surface's existing route style.
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get()
            .uri("/api/actions/command")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            405,
            "GET /api/actions/command must return 405 Method Not Allowed; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn actions_command_bad_mode_returns_400_with_valid_token() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::post()
            .uri("/api/actions/command")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .set_json(serde_json::json!({
                "mode": "restart",
                "session": "main",
                "command": "/status"
            }))
            .to_request();
        let resp = test::call_service(&app, req).await;
        // Unknown "mode" fails JSON deserialization -> the C0xx ErrorPayload
        // contract (Gap 5), not actix's default plain-text 400.
        assert_eq!(resp.status(), 400);

        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "C006");
    }

    #[actix_web::test]
    async fn actions_command_non_json_body_returns_400_c006() {
        // A malformed body that never even parses as JSON (wrong content and
        // no valid JSON syntax) must still hit the JsonConfig error handler,
        // not actix's default plain-text 400 (Gap 5).
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::post()
            .uri("/api/actions/command")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .insert_header(("content-type", "application/json"))
            .set_payload("this is not json")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 400);

        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "C006");
    }

    #[actix_web::test]
    async fn actions_command_wrong_typed_field_returns_400_c006() {
        // "session" typed as a number instead of a string fails deserialize
        // of CommandRequest -> the JsonConfig error handler, not the
        // handler-level validation_error_response path.
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::post()
            .uri("/api/actions/command")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .set_json(serde_json::json!({
                "mode": "inject",
                "session": 12345,
                "command": "/status"
            }))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 400);

        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "C006");
    }

    #[actix_web::test]
    async fn actions_command_inject_without_session_returns_400_c006() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::post()
            .uri("/api/actions/command")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .set_json(serde_json::json!({
                "mode": "inject",
                "command": "/status"
            }))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 400);

        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "C006");
    }

    // ── /api/board — route registration (BA.11.K) ───────────────────────────

    /// Build a temp brain root containing a minimal valid `brain.toml` plus a
    /// minimal leaf-shaped `planning/state.json`, so the board handler's brain
    /// walk (`find_brain_root` → `discover_state_files` → `load_state`) resolves
    /// successfully. Mirrors `brainval::tests::make_temp_brain_root`. Returns the
    /// directory — callers are responsible for `remove_dir_all` teardown.
    fn make_temp_board_brain_root() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bastion-serve-board-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let planning_dir = dir.join("planning");
        std::fs::create_dir_all(&planning_dir).unwrap();

        std::fs::write(
            dir.join("brain.toml"),
            r#"[vocab]
layer = ["console"]
status = ["active"]

[crawl]
skip_dirs = ["target", ".git"]

[[repos]]
slug = "bastion"
tier = "core"
repo_path = "."
status_file = "planning/status.md"
cache_doc = "docs/projects/bastion.md"
heading = "bastion"
"#,
        )
        .unwrap();

        std::fs::write(
            planning_dir.join("state.json"),
            r#"{
  "repo": "bastion",
  "kind": "project",
  "updated": "2026-07-04",
  "focus": {
    "now": [{ "id": "BA.11.K", "title": "Cross-brain board read endpoint", "status": "in_progress" }],
    "next": [],
    "blocked": []
  },
  "tracks": [
    {
      "title": "Phase 11",
      "blocks": [
        { "id": "BA.11.K", "title": "Cross-brain board read endpoint", "status": "in_progress" }
      ]
    }
  ]
}"#,
        )
        .unwrap();

        dir
    }

    /// Registry whose (unnamed) `default_workspace` resolves to the temp brain
    /// root — `get_board`'s `resolve_workspace_root(None, None, &registry)` call
    /// takes the same "no explicit root, no workspace name" path `bastion serve`
    /// uses in production, so routing it through `default_workspace` mirrors how
    /// a real deployment's registry would point at its own brain root.
    fn registry_with_board_fixture(brain_root: &std::path::Path) -> FileConfig {
        let mut workspaces = std::collections::HashMap::new();
        workspaces.insert("brain-root".to_string(), brain_root.to_path_buf());
        FileConfig {
            workspaces: Some(workspaces),
            default_workspace: Some("brain-root".to_string()),
            ..Default::default()
        }
    }

    #[actix_web::test]
    async fn get_board_rejects_missing_token_with_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get()
            .uri("/api/board?scope=hq")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            401,
            "GET /api/board without a token must return 401; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn get_board_hq_scope_returns_200_with_four_lanes() {
        let dir = make_temp_board_brain_root();
        let registry = registry_with_board_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/board?scope=hq")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            200,
            "GET /api/board?scope=hq with a valid token must return 200"
        );

        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["scope"], "hq");
        let lanes = &body["lanes"];
        assert!(lanes["now"].is_array(), "lanes.now must be an array");
        assert!(lanes["next"].is_array(), "lanes.next must be an array");
        assert!(
            lanes["blocked"].is_array(),
            "lanes.blocked must be an array"
        );
        assert!(
            lanes["finished"].is_array(),
            "lanes.finished must be an array"
        );
        assert!(body["repos"].is_array(), "repos must be an array");
        assert!(body["stale"].is_boolean(), "stale must be a boolean");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
