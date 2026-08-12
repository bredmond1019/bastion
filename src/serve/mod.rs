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
pub mod blocked_edge;
#[cfg(test)]
pub mod contract_corpus;
pub mod docs;
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
use engine_serve::workflows::{init_repo_registry_from_env, register_builtin_workflows};

/// Build the engine's `Dispatcher` with every builtin workflow (currently
/// just `SDLC_FLOW`) registered. Pulled out of `run()` so the wiring is
/// unit-testable without standing up actix/Postgres.
fn build_engine_dispatcher() -> Dispatcher {
    let mut dispatcher = Dispatcher::new();
    register_builtin_workflows(&mut dispatcher);
    dispatcher
}
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

    // ── Live run-state store (BA.11.M) ──────────────────────────────────────
    //
    // Hoisted above the engine-mount decision so it exists (and is shareable
    // via `web::Data<LiveStateStore>`) whether or not the engine mounts. When
    // the engine *is* mounted, this exact instance is cloned into
    // `EngineAppState.live` so `on_progress` records into the same store the
    // `/api/runs` read routes below observe. When the engine is skipped, the
    // store simply stays empty (`GET /api/runs` → `[]`, `GET /api/runs/{id}`
    // → 404) — the same graceful-degradation posture as the engine routes.
    //
    // Also hoisted above `Hub::new` (BA.11.N task 3) so the hub can hold its
    // own clone for the `runs`-topic poller.
    let live_store = LiveStateStore::new();

    // ── Engine embed (BA.7.C task 2) ────────────────────────────────────────
    //
    // Decide once at boot whether to mount `engine-serve`'s route table.
    // Absent-tolerant: with `DATABASE_URL` or the engine API key missing,
    // `bastion serve` still starts its existing session/status surface —
    // the engine routes are simply left unmounted, and we say so on stderr
    // plus an `observ` event rather than failing to boot or mounting a route
    // that would 500 on every request.
    //
    // Hoisted above `Hub::new` (BA.11.N task 4) — the hub is constructed with
    // the real `stream_available` verdict this decision produces (D17
    // constraint 2), rather than a placeholder wired up after the fact.
    let (engine_data, stream_available): (
        Option<web::Data<EngineAppState>>,
        (bool, Option<String>),
    ) = match decide_engine_mount(
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
                    // EN.3.K: resolve the process-global repo registry from
                    // `ENGINE_BRAIN_ROOT` before `build_engine_dispatcher()`
                    // registers `SDLC_FLOW` — its factory reads the registry
                    // once at registration time (`register_sdlc_flow` ->
                    // `repo_registry()`), so this must run first or every
                    // `repo`-bearing event 422s regardless of the env var.
                    // No-op (logs and leaves the registry unset) when
                    // `ENGINE_BRAIN_ROOT` is absent/unresolvable — absent-`repo`
                    // events are unaffected either way.
                    init_repo_registry_from_env();
                    let state = EngineAppState {
                        dispatcher: Arc::new(build_engine_dispatcher()),
                        live: live_store.clone(),
                        durable: spawn_durable_writer(Some(pool)),
                        runs: RunRegistry::new(),
                        api_key: engine_api_key,
                    };
                    (Some(web::Data::new(state)), (true, None))
                }
                Err(e) => {
                    tracing::error!(
                        target: "bastion::serve",
                        error = %e,
                        "engine routes not mounted — failed to connect to DATABASE_URL"
                    );
                    let reason = format!(
                        "engine routes not mounted — could not connect to DATABASE_URL: {e}"
                    );
                    eprintln!("bastion serve: {reason}");
                    (None, (false, Some(reason)))
                }
            }
        }
        EngineMountDecision::Skip { reason } => {
            tracing::warn!(target: "bastion::serve", %reason);
            eprintln!("bastion serve: {reason}");
            (None, (false, Some(reason)))
        }
    };

    // Start the hub actor once (process-singleton within this actix System).
    // All per-connection WsConn actors hold an Addr<Hub> clone.
    let hub = Hub::new(
        poll_secs,
        registry.clone(),
        live_store.clone(),
        stream_available,
    )
    .start();

    // ── Blocked rising-edge poller (BA.18.A task 3) ─────────────────────────
    //
    // Always-on: spawned once at boot, independent of the hub above and of
    // any WebSocket subscription, so a session blocking with zero clients
    // ever connected still produces a durable sink record (Acceptance
    // Criteria). `addr` (this process's own bind address) doubles as its
    // host/instance identity in the sink — stable for the life of this
    // `bastion serve` instance and enough to distinguish it from any other
    // instance writing to the same path, with no new dependency.
    match blocked_edge::default_sink_path(
        std::env::var("XDG_STATE_HOME").ok(),
        std::env::var("HOME").ok(),
    ) {
        Some(sink_path) => {
            let sink = blocked_edge::BlockedEdgeSink::new(sink_path);
            let poller = blocked_edge::BlockedEdgePoller::new(sink, addr.clone());
            actix_web::rt::spawn(poller.run(poll_secs));
        }
        None => {
            tracing::warn!(
                target: "bastion::serve",
                "blocked-edge poller not started — neither XDG_STATE_HOME nor HOME is set"
            );
        }
    }

    let registry = web::Data::new(registry);

    let live_data = web::Data::new(live_store);

    HttpServer::new(move || {
        let hub_data = web::Data::new(hub.clone());
        let registry_data = registry.clone();
        let engine_data = engine_data.clone();
        let live_data = live_data.clone();

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
            // /workflows — GET only (cross-repo flow-state aggregate, A2)
            .service(
                web::resource("/workflows")
                    .route(web::get().to(handlers::status::list_all_workflows)),
            )
            // ── Quick-action command route (BA.11.E) ────────────────────────
            // /actions/command — POST only
            .service(
                web::resource("/actions/command").route(web::post().to(handlers::actions::command)),
            )
            // ── Cross-brain board route (BA.11.K) ───────────────────────────
            // /board — GET only (now/next/blocked/finished rollup)
            .service(web::resource("/board").route(web::get().to(handlers::board::get_board)))
            // ── Cost read route (BA.11.J) ────────────────────────────────────
            // /costs — GET only (exact-count spend + budget-gate state)
            .service(web::resource("/costs").route(web::get().to(handlers::costs::get_costs)))
            // ── Block-graph read route (BA.17.A) ─────────────────────────────
            // /blocks/graph — GET only (mev's enriched block-graph export)
            .service(
                web::resource("/blocks/graph")
                    .route(web::get().to(handlers::block_graph::get_block_graph)),
            )
            // ── Live run-state read routes (BA.11.M) ────────────────────────
            // /runs — GET (currently-tracked run ids)
            .service(web::resource("/runs").route(web::get().to(handlers::runs::list_runs)))
            // /runs/{id} — GET (per-node run-state snapshot)
            .service(web::resource("/runs/{id}").route(web::get().to(handlers::runs::get_run)))
            // ── Attention / carryover board route (BA.11.P) ─────────────────
            // /attention — GET only (stale carryover, aging backlog, orphaned captures)
            .service(
                web::resource("/attention")
                    .route(web::get().to(handlers::attention::get_attention)),
            )
            // ── Docs read routes (BA.11.Q) ───────────────────────────────────
            // /docs/{repo}/tree — GET only (allowlisted markdown tree)
            // /docs/{repo}/file — GET only (raw markdown read)
            .service(
                web::resource("/docs/{repo}/tree")
                    .route(web::get().to(handlers::docs::get_docs_tree)),
            )
            .service(
                web::resource("/docs/{repo}/file")
                    .route(web::get().to(handlers::docs::get_docs_file)),
            )
            // ── Epic registry route (BA.11.R) ────────────────────────────────
            // /epics — GET only (HQ cross-repo initiative registry)
            .service(web::resource("/epics").route(web::get().to(handlers::epics::get_epics)))
            // ── Pipeline / opportunities routes (BW.3.A) ─────────────────────
            // /pipeline — GET only (stage vocab + opportunity summaries)
            // /pipeline/{slug} — GET only (one opportunity's full projection)
            .service(
                web::resource("/pipeline").route(web::get().to(handlers::pipeline::get_pipeline)),
            )
            .service(
                web::resource("/pipeline/{slug}")
                    .route(web::get().to(handlers::pipeline::get_pipeline_opportunity)),
            )
            .app_data(live_data);

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
    fn build_engine_dispatcher_registers_sdlc_flow() {
        // `register_builtin_workflows` is owned by the upstream `engine-serve` crate; it
        // now registers additional built-in workflow types alongside `SDLC_FLOW`. This
        // assertion only pins the contract this module relies on — `SDLC_FLOW` is
        // present — rather than the full (and upstream-owned) registry contents, so it
        // doesn't churn every time `engine-serve` adds another built-in workflow.
        let dispatcher = build_engine_dispatcher();
        assert!(dispatcher.is_registered("SDLC_FLOW"));
    }

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
    use crate::testsupport::{DotenvShadow, EnvVarGuard, lock_env};
    use actix_web::{App, test};

    const TEST_TOKEN: &str = "test-secret-token";

    /// Build the test app mirroring production routing exactly, using a fixed test token
    /// and the given workspace registry (use `FileConfig::default()` for tests that don't
    /// exercise the repo/workflow status routes).
    ///
    /// A fresh, empty [`LiveStateStore`] is used and `runs`-stream availability is reported
    /// as `(false, None)` (no engine mounted) — callers that need to seed the store before
    /// connecting, or assert `available: true`, should use
    /// [`build_app_with_live_store`] instead.
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
        build_app_with_live_store(registry, LiveStateStore::new(), (false, None)).0
    }

    /// Same routing as [`build_app`], but threading a caller-supplied [`LiveStateStore`]
    /// and `runs`-stream availability pair through to the hub (BA.11.N task 4) — mirrors
    /// how production's `run_server` hoists the engine-mount decision above `Hub::new` and
    /// gives the hub and `EngineAppState.live` the *same* store instance. Lets tests seed
    /// the store before connecting a `/ws` client (so a later mutation produces a
    /// `run_transition` frame on that already-open connection) and assert the
    /// `run_stream_status` frame's `available`/`reason` fields for both mount outcomes.
    ///
    /// Also returns the `Addr<Hub>` the built `App` is wired to — real end-to-end WS wire
    /// traffic has no test client in this crate's dependency graph (no `awc`/`actix-test`;
    /// see `src/serve/ws/session.rs`'s "live behaviour is smoke-tested" note), so tests that
    /// need to prove a *specific, App-mounted* hub instance reacts correctly message it
    /// directly (the same `Connect`/`Subscribe` pattern `ws/server.rs`'s `RecorderActor`
    /// tests use) rather than re-deriving the Hub's internal logic, which task 3 already
    /// covers exhaustively.
    fn build_app_with_live_store(
        registry: FileConfig,
        live_store: LiveStateStore,
        stream_available: (bool, Option<String>),
    ) -> (
        actix_web::App<
            impl actix_web::dev::ServiceFactory<
                actix_web::dev::ServiceRequest,
                Config = (),
                Response = actix_web::dev::ServiceResponse,
                Error = actix_web::Error,
                InitError = (),
            >,
        >,
        Addr<Hub>,
    ) {
        // Start a hub for test routing — mirrors production (Hub::start inside the actix System).
        let hub = Hub::new(2, registry.clone(), live_store.clone(), stream_available).start();
        let hub_for_return = hub.clone();
        let hub_data = web::Data::new(hub);
        let registry_data = web::Data::new(registry);
        let live_data = web::Data::new(live_store);

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
                web::resource("/workflows")
                    .route(web::get().to(handlers::status::list_all_workflows)),
            )
            .service(
                web::resource("/actions/command").route(web::post().to(handlers::actions::command)),
            )
            .service(web::resource("/board").route(web::get().to(handlers::board::get_board)))
            // ── Cost read route (BA.11.J) ────────────────────────────────────
            .service(web::resource("/costs").route(web::get().to(handlers::costs::get_costs)))
            // ── Block-graph read route (BA.17.A) ─────────────────────────────
            .service(
                web::resource("/blocks/graph")
                    .route(web::get().to(handlers::block_graph::get_block_graph)),
            )
            .service(web::resource("/runs").route(web::get().to(handlers::runs::list_runs)))
            .service(web::resource("/runs/{id}").route(web::get().to(handlers::runs::get_run)))
            .service(
                web::resource("/attention")
                    .route(web::get().to(handlers::attention::get_attention)),
            )
            .service(
                web::resource("/docs/{repo}/tree")
                    .route(web::get().to(handlers::docs::get_docs_tree)),
            )
            .service(
                web::resource("/docs/{repo}/file")
                    .route(web::get().to(handlers::docs::get_docs_file)),
            )
            .service(web::resource("/epics").route(web::get().to(handlers::epics::get_epics)))
            .service(
                web::resource("/pipeline").route(web::get().to(handlers::pipeline::get_pipeline)),
            )
            .service(
                web::resource("/pipeline/{slug}")
                    .route(web::get().to(handlers::pipeline::get_pipeline_opportunity)),
            )
            .app_data(live_data);
        let ws_scope = web::scope("/ws")
            .wrap(BearerAuthMiddleware::new(TEST_TOKEN))
            .app_data(hub_data.clone())
            .route("", web::get().to(hub_ws_handler));

        let app = App::new()
            .app_data(hub_data)
            .app_data(registry_data)
            .app_data(json_config())
            .service(web::resource("/health").route(web::get().to(health)))
            .service(protected)
            .service(ws_scope);

        (app, hub_for_return)
    }

    /// Minimal `/api/runs` + `/api/runs/{id}` test app carrying a caller-supplied
    /// [`LiveStateStore`] (A7) — `build_app` always seeds a fresh empty store, so
    /// the `?with_repo=1` join tests (which need an *active* run to enrich) build
    /// their own scope here rather than growing `build_app`'s signature, which
    /// ~90 unrelated call sites across this test module would otherwise have to
    /// thread a store through. Auth policy mirrors `build_app` exactly (bearer
    /// token required, same middleware), just scoped to the two run routes.
    fn build_runs_test_app(
        registry: FileConfig,
        live: LiveStateStore,
    ) -> actix_web::App<
        impl actix_web::dev::ServiceFactory<
            actix_web::dev::ServiceRequest,
            Config = (),
            Response = actix_web::dev::ServiceResponse,
            Error = actix_web::Error,
            InitError = (),
        >,
    > {
        let registry_data = web::Data::new(registry);
        let live_data = web::Data::new(live);

        let protected = web::scope("/api")
            .wrap(BearerAuthMiddleware::new(TEST_TOKEN))
            .service(web::resource("/runs").route(web::get().to(handlers::runs::list_runs)))
            .service(web::resource("/runs/{id}").route(web::get().to(handlers::runs::get_run)))
            .app_data(live_data);

        App::new().app_data(registry_data).service(protected)
    }

    /// A minimal `TaskContext` snapshot suitable for [`LiveStateStore::record`] —
    /// empty event/nodes/metadata/node_runs, sufficient for the `?with_repo=1`
    /// join tests, which only care about `RunSummaryDto.repo`.
    fn empty_task_context() -> engine_contract::task_context::TaskContext {
        engine_contract::task_context::TaskContext {
            event: serde_json::json!({}),
            nodes: std::collections::HashMap::new(),
            metadata: serde_json::json!({}),
            node_runs: std::collections::HashMap::new(),
        }
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

    // ── `runs` topic wired through the real App (BA.11.N task 4) ───────────
    //
    // `run_server`'s engine-mount hoist and `build_app_with_live_store`'s test mirror both
    // construct `Hub::new` with a real `LiveStateStore` clone and a real availability verdict
    // *before* the App is built. These tests exercise that exact wiring: the `App`-mounted
    // hub is messaged directly (no WS wire round-trip is available in this crate's
    // dependency graph — see `build_app_with_live_store`'s doc comment), proving the specific
    // hub instance the running App would serve `/ws` traffic through reacts as task 3's
    // Hub-level tests predict, not a freshly constructed stand-in.

    /// Records every [`ServerFrame`] it receives — a local, `mod tests`-scoped stand-in
    /// for a `WsConn` recipient (the `ws/server.rs` `RecorderActor` used for the equivalent
    /// hub-level tests is private to that module, so this is a small deliberate duplicate
    /// rather than a visibility change to another task's file).
    #[derive(Default)]
    struct RunsFrameRecorder {
        received: std::sync::Arc<std::sync::Mutex<Vec<crate::serve::dto::WsFrame>>>,
    }

    impl actix::Actor for RunsFrameRecorder {
        type Context = actix::Context<Self>;
    }

    impl actix::Handler<crate::serve::ws::server::ServerFrame> for RunsFrameRecorder {
        type Result = ();

        fn handle(
            &mut self,
            msg: crate::serve::ws::server::ServerFrame,
            _ctx: &mut actix::Context<Self>,
        ) {
            self.received.lock().unwrap().push(msg.0);
        }
    }

    /// Round-trips through the recorder's mailbox so the caller can await it and be sure
    /// every earlier `do_send`/`send` in the (single-threaded, FIFO) mailbox has already
    /// been processed — mirrors `ws/server.rs`'s `DrainReceived`.
    #[derive(actix::Message)]
    #[rtype(result = "Vec<crate::serve::dto::WsFrame>")]
    struct DrainRunsFrames;

    impl actix::Handler<DrainRunsFrames> for RunsFrameRecorder {
        type Result = Vec<crate::serve::dto::WsFrame>;

        fn handle(
            &mut self,
            _msg: DrainRunsFrames,
            _ctx: &mut actix::Context<Self>,
        ) -> Vec<crate::serve::dto::WsFrame> {
            // Genuinely destructive (unlike `ws/server.rs`'s otherwise-identical
            // `DrainReceived`, which only clones): the seeded-store test below calls this
            // twice on the same recorder and must see only *new* frames on the second call.
            std::mem::take(&mut *self.received.lock().unwrap())
        }
    }

    #[actix_web::test]
    async fn ws_upgrade_still_401s_without_token_when_engine_mounted() {
        // D17 constraint: threading a real (mounted) availability verdict through the app
        // factory must not change the unrelated bearer-auth gate on the `/ws` upgrade.
        let (app, _hub) =
            build_app_with_live_store(FileConfig::default(), LiveStateStore::new(), (true, None));
        let app = test::init_service(app).await;

        let req = test::TestRequest::get().uri("/ws").to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(
            resp.status(),
            401,
            "GET /ws without token must still return 401 with the engine mounted; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn app_mounted_hub_pushes_run_stream_status_on_runs_subscribe() {
        use crate::serve::dto::Topic;
        use crate::serve::ws::server::{ConnId, Connect, Subscribe};

        let (app, hub) =
            build_app_with_live_store(FileConfig::default(), LiveStateStore::new(), (true, None));
        // Build the app to prove the App factory wires this exact hub — the assertions
        // below then message that same `Addr<Hub>` directly.
        let _app = test::init_service(app).await;

        let recorder = RunsFrameRecorder::default().start();
        let id = ConnId::next();
        hub.send(Connect {
            id,
            addr: recorder.clone().recipient(),
        })
        .await
        .unwrap();
        hub.send(Subscribe {
            id,
            topic: Topic::Runs,
        })
        .await
        .unwrap();

        let received = recorder.send(DrainRunsFrames).await.unwrap();
        assert_eq!(
            received.len(),
            1,
            "the App-mounted hub must push exactly one run_stream_status frame on subscribe"
        );
        assert_eq!(received[0].payload["event"], "run_stream_status");
        assert_eq!(received[0].payload["available"], true);
    }

    #[actix_web::test]
    async fn app_mounted_hub_reports_unavailable_with_reason_when_engine_unmounted() {
        use crate::serve::dto::Topic;
        use crate::serve::ws::server::{ConnId, Connect, Subscribe};

        let (app, hub) = build_app_with_live_store(
            FileConfig::default(),
            LiveStateStore::new(),
            (false, Some("DATABASE_URL not set".to_owned())),
        );
        let _app = test::init_service(app).await;

        let recorder = RunsFrameRecorder::default().start();
        let id = ConnId::next();
        hub.send(Connect {
            id,
            addr: recorder.clone().recipient(),
        })
        .await
        .unwrap();
        hub.send(Subscribe {
            id,
            topic: Topic::Runs,
        })
        .await
        .unwrap();

        let received = recorder.send(DrainRunsFrames).await.unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].payload["available"], false);
        assert_eq!(received[0].payload["reason"], "DATABASE_URL not set");
    }

    #[actix_web::test]
    async fn app_mounted_hub_emits_run_transition_from_seeded_store_on_next_poll() {
        // "With a store seeded before the connection, a subsequent status change produces
        // a `run_transition` frame" — seeded here means the store already carries a run
        // whose *first* poll observation is a no-op (no previous status to diff against);
        // the transition frame requires a second poll cycle after the status changes.
        // `build_app_with_live_store`'s hub polls every 2s (matching `Hub::new`'s first
        // arg below), so the wait after seeding the terminal transition covers two ticks.
        use crate::serve::dto::Topic;
        use crate::serve::ws::server::{ConnId, Connect, Subscribe};
        use engine_contract::task_context::TaskContext;
        use uuid::Uuid;

        let live_store = LiveStateStore::new();
        let run_id = Uuid::new_v4();
        let seed_ctx = TaskContext {
            event: serde_json::json!({}),
            nodes: std::collections::HashMap::new(),
            metadata: serde_json::json!({}),
            node_runs: std::collections::HashMap::new(),
        };
        live_store.record(run_id, &seed_ctx);

        let (app, hub) =
            build_app_with_live_store(FileConfig::default(), live_store.clone(), (true, None));
        let _app = test::init_service(app).await;

        let recorder = RunsFrameRecorder::default().start();
        let id = ConnId::next();
        hub.send(Connect {
            id,
            addr: recorder.clone().recipient(),
        })
        .await
        .unwrap();
        hub.send(Subscribe {
            id,
            topic: Topic::Runs,
        })
        .await
        .unwrap();

        // Let the poller's first tick observe the run's first appearance (a no-op — no
        // previous status), then drain the immediate run_stream_status frame plus any
        // no-op poll output before seeding the terminal transition.
        tokio::time::sleep(std::time::Duration::from_millis(2_200)).await;
        let _ = recorder.send(DrainRunsFrames).await.unwrap();

        // Mark the seeded run lifecycle-terminal so it disappears from `list_active()` —
        // the poller's next tick must observe the disappearance and emit a terminal
        // `run_transition` frame.
        let now = chrono::Utc::now();
        live_store.mark_terminal(run_id, &seed_ctx, "SDLC_FLOW", now, now);

        tokio::time::sleep(std::time::Duration::from_millis(2_200)).await;

        let received = recorder.send(DrainRunsFrames).await.unwrap();
        assert!(
            !received.is_empty(),
            "the App-mounted hub's poller must emit a run_transition frame after the seeded \
             run goes lifecycle-terminal"
        );
        assert_eq!(received[0].payload["event"], "run_transition");
        assert_eq!(received[0].payload["run_id"], run_id.to_string());
        assert_eq!(received[0].payload["terminal"], true);
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
    const FLOW_JSON_WITH_RUN_ID: &str = include_str!("status/fixtures/flow_state_with_run_id.json");

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

    /// Same shape as [`registry_with_fixture_repo`], but the workspace's
    /// `sdlc-flow-state.json` carries a `run_id` (`FLOW_JSON_WITH_RUN_ID`) —
    /// used by the `?with_repo=1` join tests (A7) to exercise a repo-resolvable
    /// run id.
    fn registry_with_fixture_repo_with_run_id() -> (TempDir, FileConfig) {
        let tmp = TempDir::new();
        write_fixture(&tmp.path().join("planning/status.md"), STATUS_MD);
        write_fixture(&tmp.path().join("planning/handoff.md"), HANDOFF_MD);
        write_fixture(
            &tmp.path()
                .join("planning/phase6-blockA/sdlc/sdlc-flow-state.json"),
            FLOW_JSON_WITH_RUN_ID,
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

    /// A1: `run_id` present on-disk must surface verbatim in the raw JSON
    /// body of `GET /api/repos/{name}/workflows`.
    #[actix_web::test]
    async fn get_repo_workflows_surfaces_run_id_verbatim() {
        let tmp = TempDir::new();
        write_fixture(&tmp.path().join("planning/status.md"), STATUS_MD);
        write_fixture(
            &tmp.path()
                .join("planning/phase6-blockA/sdlc/sdlc-flow-state.json"),
            FLOW_JSON_WITH_RUN_ID,
        );
        let mut workspaces = std::collections::HashMap::new();
        workspaces.insert("repo-with-run-id".to_string(), tmp.path().to_path_buf());
        let registry = FileConfig {
            workspaces: Some(workspaces),
            ..Default::default()
        };
        let app = test::init_service(build_app(registry)).await;
        let req = test::TestRequest::get()
            .uri("/api/repos/repo-with-run-id/workflows")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(
            body[0]["run_id"], "a1b2c3d4-e5f6-4789-a012-3456789abcde",
            "run_id must surface verbatim, got: {body}"
        );
    }

    /// A1: a state file with no `run_id` key must produce an ABSENT key in
    /// the raw serialized JSON — never `null` — so a consumer can
    /// distinguish "predates the stamp" from "field not understood".
    #[actix_web::test]
    async fn get_repo_workflows_omits_run_id_key_when_unstamped() {
        let (_tmp, registry) = registry_with_fixture_repo();
        let app = test::init_service(build_app(registry)).await;
        let req = test::TestRequest::get()
            .uri("/api/repos/repo-x/workflows")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = test::read_body_json(resp).await;
        let entry = body[0]
            .as_object()
            .expect("workflow entry must be a JSON object");
        assert!(
            !entry.contains_key("run_id"),
            "run_id should be an absent key when unstamped, got: {body}"
        );
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

    // ── /api/workflows — cross-repo aggregate (A2) ──────────────────────────

    #[actix_web::test]
    async fn list_all_workflows_rejects_missing_token_with_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get().uri("/api/workflows").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            401,
            "GET /api/workflows without token must return 401; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn list_all_workflows_empty_registry_returns_200_empty_array() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get()
            .uri("/api/workflows")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body, serde_json::json!([]));
    }

    #[actix_web::test]
    async fn list_all_workflows_populated_two_repos_returns_repo_tagged_entries() {
        let tmp_a = TempDir::new();
        write_fixture(
            &tmp_a
                .path()
                .join("planning/phase6-blockA/sdlc/sdlc-flow-state.json"),
            FLOW_JSON,
        );
        let tmp_b = TempDir::new();
        write_fixture(
            &tmp_b
                .path()
                .join("planning/phase6-blockA/sdlc/sdlc-flow-state.json"),
            FLOW_JSON_WITH_RUN_ID,
        );

        let mut workspaces = std::collections::HashMap::new();
        workspaces.insert("repo-alpha".to_string(), tmp_a.path().to_path_buf());
        workspaces.insert("repo-beta".to_string(), tmp_b.path().to_path_buf());
        let registry = FileConfig {
            workspaces: Some(workspaces),
            ..Default::default()
        };

        let app = test::init_service(build_app(registry)).await;
        let req = test::TestRequest::get()
            .uri("/api/workflows")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = test::read_body_json(resp).await;
        let entries = body.as_array().expect("response must be a JSON array");
        assert_eq!(entries.len(), 2, "expected one entry per repo, got: {body}");

        // Ordered by (repo name, then spec_slug): "repo-alpha" sorts before "repo-beta".
        assert_eq!(entries[0]["repo"], "repo-alpha");
        assert_eq!(entries[0]["spec_slug"], "phase6-blockA");
        assert!(
            !entries[0]
                .as_object()
                .expect("entry must be an object")
                .contains_key("run_id"),
            "unstamped entry must omit run_id key entirely, got: {body}"
        );

        assert_eq!(entries[1]["repo"], "repo-beta");
        assert_eq!(entries[1]["spec_slug"], "phase6-blockA");
        assert_eq!(
            entries[1]["run_id"], "a1b2c3d4-e5f6-4789-a012-3456789abcde",
            "run_id must surface verbatim, got: {body}"
        );
    }

    #[actix_web::test]
    async fn list_all_workflows_agrees_with_per_repo_route() {
        let (_tmp, registry) = registry_with_fixture_repo();
        let app = test::init_service(build_app(registry)).await;

        let agg_req = test::TestRequest::get()
            .uri("/api/workflows")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let agg_resp = test::call_service(&app, agg_req).await;
        assert_eq!(agg_resp.status(), 200);
        let agg_body: serde_json::Value = test::read_body_json(agg_resp).await;

        let per_repo_req = test::TestRequest::get()
            .uri("/api/repos/repo-x/workflows")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let per_repo_resp = test::call_service(&app, per_repo_req).await;
        assert_eq!(per_repo_resp.status(), 200);
        let per_repo_body: serde_json::Value = test::read_body_json(per_repo_resp).await;

        let agg_entries = agg_body.as_array().expect("aggregate must be an array");
        assert_eq!(agg_entries.len(), 1);
        let per_repo_entries = per_repo_body
            .as_array()
            .expect("per-repo response must be an array");
        assert_eq!(per_repo_entries.len(), 1);

        assert_eq!(agg_entries[0]["repo"], "repo-x");
        assert_eq!(
            agg_entries[0]["spec_slug"],
            per_repo_entries[0]["spec_slug"]
        );
        assert_eq!(agg_entries[0]["status"], per_repo_entries[0]["status"]);
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
        let dir = crate::testsupport::unique_temp_dir("bastion-serve-board");
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
            lanes["deferred"].is_array(),
            "lanes.deferred must be an array"
        );
        assert!(
            lanes["finished"].is_array(),
            "lanes.finished must be an array"
        );
        assert!(body["repos"].is_array(), "repos must be an array");
        assert!(body["stale"].is_boolean(), "stale must be a boolean");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── /api/costs — route registration (BA.11.J) ───────────────────────────
    //
    // The route mounts unconditionally (Load-bearing decision 2 in
    // `planning/11.J-cost-read-endpoint/tasks.md`): a missing/unreachable
    // database is a typed error response from the handler, never a 404.

    // The former module-local `DATABASE_URL_ENV_LOCK` is gone. Process-global
    // env (and the repo-cwd `.env` file `DotenvShadow` swaps) is now
    // serialized by the **one** crate-wide lock in `crate::testsupport` —
    // deliberately one lock, so ordering between modules is moot and
    // deadlock between them impossible. A per-module lock is exactly what
    // failed here before: `serve::contract_corpus` had its own, and the two
    // never excluded each other.
    //
    // Discipline (full version in `crate::testsupport`'s module docs):
    // take `lock_env()` **once**, at the top of the test; mutate only through
    // `EnvVarGuard`; never hold it across an `.await`. It is a plain
    // `std::sync::Mutex` — not reentrant.

    // `DotenvShadow` itself now lives in `crate::testsupport` alongside the
    // env lock it requires as a witness — `serve::contract_corpus`'s costs
    // scenarios need the identical `.env`-shadowing guarantee, and a second
    // copy of a global-resource guard is exactly the per-module duplication
    // the crate-wide lock exists to undo. Its own unit test moved with it.

    #[actix_web::test]
    async fn get_costs_rejects_missing_token_with_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get().uri("/api/costs").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            401,
            "GET /api/costs without a token must return 401; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn get_costs_wrong_token_returns_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get()
            .uri("/api/costs")
            .insert_header(("authorization", "Bearer not-the-token"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            401,
            "GET /api/costs with an invalid token must return 401; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn get_costs_invalid_window_returns_400_c006_before_db_access() {
        // No DATABASE_URL manipulation here on purpose: `resolve_window` runs
        // before `Config::load`, so this must short-circuit with no Postgres
        // running and regardless of whatever DATABASE_URL happens to be set
        // to in the ambient environment.
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get()
            .uri("/api/costs?window=nonsense")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            400,
            "GET /api/costs?window=nonsense must return 400; got {}",
            resp.status()
        );

        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "C006");
    }

    #[actix_web::test]
    async fn get_costs_missing_database_url_returns_503_c005() {
        let env_lock = lock_env();

        // `Config::load` calls `dotenvy::dotenv()`, which repopulates
        // DATABASE_URL from a `.env` the moment we remove it from the process
        // env (dotenvy only sets vars that are absent) — so a bare `remove_var`
        // isn't enough to simulate "unset" in a dev checkout that has a working
        // local Postgres. `DotenvShadow` neutralizes that lookup, including the
        // ancestor-`.env` case that bites inside a git worktree; see its docs.
        let _dotenv_shadow = DotenvShadow::new(&env_lock, "get_costs_c005");

        // Serialized by `env_lock` above — no other test that participates in
        // the discipline reads or writes env while these guards are held. The
        // guard also restores the previous value on unwind, so a failing
        // assertion below cannot leak an unset DATABASE_URL into a sibling.
        let _database_url = EnvVarGuard::unset(&env_lock, "DATABASE_URL");

        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get()
            .uri("/api/costs")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;

        let status = resp.status();
        let body: serde_json::Value = test::read_body_json(resp).await;

        assert_eq!(
            status, 503,
            "GET /api/costs with DATABASE_URL unset must return 503; got {status}"
        );
        assert_eq!(body["code"], "C005");
    }

    #[actix_web::test]
    async fn get_costs_unreachable_database_returns_503_c009() {
        let env_lock = lock_env();

        // Same `.env` shadowing as the C005 test above — `Config::load`
        // would otherwise repopulate DATABASE_URL from the repo's dev
        // `.env` via `dotenvy::dotenv()` before we get to set our own
        // (present-but-unreachable) value.
        let _dotenv_shadow = DotenvShadow::new(&env_lock, "get_costs_c009");

        // Serialized by `env_lock` above — no other participating test reads
        // or writes env while these guards are held. A
        // syntactically-invalid connection string fails `PgPoolOptions::
        // connect` at URL-parse time, before any TCP attempt — so this
        // exercises the same `fetch_all_runs` `Err` -> `db_error_response`
        // path as a genuinely unreachable Postgres, without incurring
        // sqlx's ~30s default `acquire_timeout` retry/backoff on an
        // actually-refused connection.
        let _database_url = EnvVarGuard::set(&env_lock, "DATABASE_URL", "not-a-valid-postgres-url");

        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get()
            .uri("/api/costs")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;

        let status = resp.status();
        let body: serde_json::Value = test::read_body_json(resp).await;

        assert_eq!(
            status, 503,
            "GET /api/costs with an unreachable database must return 503; got {status}"
        );
        assert_eq!(body["code"], "C009");
    }

    // ── /api/epics + enriched /api/board — route registration (BA.11.R) ────

    /// Temp brain root for the `/api/epics` + enriched `/api/board` tests.
    ///
    /// Two state files, unlike [`make_temp_board_brain_root`]'s single file:
    /// - the root's own `<dir>/planning/state.json` — always the HQ file
    ///   (`discover_state_files` step 1 discovers it unconditionally at that
    ///   path regardless of `[[repos]]`) — `kind: "brain"`, carrying the
    ///   `epics[]` registry.
    /// - `<dir>/bastion/planning/state.json` — the "bastion" leaf project,
    ///   `kind: "project"`, registered via a single `[[repos]]` entry with
    ///   `repo_path = "bastion"` (deliberately *not* `"."`, so it doesn't
    ///   collide with the HQ file's path and so `tier_scope_for` doesn't
    ///   mistake the HQ file for a tier-container self-entry).
    ///
    /// The leaf's `tracks[]` authors four blocks exercising every enrichment +
    /// `blocked_by` case: `BA.11.R` (in-progress, fully authored
    /// epics/wave/priority/due), `BA.11.S` (open, depends on `BA.11.R` which
    /// is not `closed` — unmet), `BA.11.T` (open, depends on `BA.11.K` which
    /// is `closed` — met/ready), `BA.11.K` (closed — lands in `finished`).
    fn make_temp_epics_brain_root() -> std::path::PathBuf {
        let dir = crate::testsupport::unique_temp_dir("bastion-serve-epics");
        let planning_dir = dir.join("planning");
        std::fs::create_dir_all(&planning_dir).unwrap();
        let leaf_planning_dir = dir.join("bastion").join("planning");
        std::fs::create_dir_all(&leaf_planning_dir).unwrap();

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
repo_path = "bastion"
status_file = "planning/status.md"
cache_doc = "docs/projects/bastion.md"
heading = "bastion"
"#,
        )
        .unwrap();

        std::fs::write(
            planning_dir.join("state.json"),
            r#"{
  "repo": "hq",
  "kind": "brain",
  "updated": "2026-07-25",
  "focus": { "now": [], "next": [], "blocked": [] },
  "tracks": [],
  "epics": [
    {
      "slug": "bastion-surfaces",
      "title": "Bastion Surfaces",
      "description": "Cross-surface work",
      "status": "active",
      "weight": 85,
      "plan": "core/planning/master-plan.md",
      "repos": ["bastion"]
    },
    {
      "slug": "brain-rag",
      "title": "Brain RAG"
    }
  ]
}"#,
        )
        .unwrap();

        std::fs::write(
            leaf_planning_dir.join("state.json"),
            r#"{
  "repo": "bastion",
  "kind": "project",
  "updated": "2026-07-25",
  "focus": { "now": [], "next": [], "blocked": [] },
  "tracks": [
    {
      "title": "Phase 11",
      "blocks": [
        {
          "id": "BA.11.R",
          "title": "Epic ranking enrichment",
          "status": "in_progress",
          "epics": ["bastion-surfaces"],
          "wave": 3,
          "priority": 1,
          "due": "2026-07-15"
        },
        {
          "id": "BA.11.S",
          "title": "Downstream consumer",
          "status": "open",
          "depends_on": [{ "type": "block", "repo": "bastion", "id": "BA.11.R" }],
          "epics": ["bastion-surfaces"]
        },
        {
          "id": "BA.11.T",
          "title": "Ready block",
          "status": "open",
          "depends_on": [{ "type": "block", "repo": "bastion", "id": "BA.11.K" }],
          "epics": ["brain-rag"]
        },
        {
          "id": "BA.11.K",
          "title": "Prior closed block",
          "status": "closed",
          "epics": ["bastion-surfaces"]
        }
      ]
    }
  ]
}"#,
        )
        .unwrap();

        dir
    }

    /// Registry pointing `default_workspace` at the temp epics brain root —
    /// mirrors [`registry_with_board_fixture`].
    fn registry_with_epics_fixture(brain_root: &std::path::Path) -> FileConfig {
        let mut workspaces = std::collections::HashMap::new();
        workspaces.insert("brain-root".to_string(), brain_root.to_path_buf());
        FileConfig {
            workspaces: Some(workspaces),
            default_workspace: Some("brain-root".to_string()),
            ..Default::default()
        }
    }

    #[actix_web::test]
    async fn get_epics_rejects_missing_token_with_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get().uri("/api/epics").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            401,
            "GET /api/epics without a token must return 401; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn get_epics_returns_the_hq_registry() {
        let dir = make_temp_epics_brain_root();
        let registry = registry_with_epics_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/epics")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            200,
            "GET /api/epics with a valid token must return 200; got {}",
            resp.status()
        );

        let body: serde_json::Value = test::read_body_json(resp).await;
        let entries = body.as_array().expect("body must be an array");
        assert_eq!(
            entries.len(),
            2,
            "expected two registry entries; got {body:?}"
        );
        assert_eq!(entries[0]["slug"], "bastion-surfaces");
        assert_eq!(entries[0]["title"], "Bastion Surfaces");
        assert_eq!(entries[0]["status"], "active");
        assert_eq!(entries[0]["plan"], "core/planning/master-plan.md");
        assert_eq!(entries[0]["repos"][0], "bastion");
        assert_eq!(
            entries[0]["weight"], 85,
            "authored weight must reach the wire verbatim"
        );
        assert_eq!(entries[1]["slug"], "brain-rag");
        assert!(
            entries[1]["repos"].as_array().unwrap().is_empty(),
            "minimal registry entry must default repos to []"
        );
        assert!(
            entries[1]["weight"].is_null(),
            "unauthored weight must be null on the wire, not omitted; got {:?}",
            entries[1].get("weight")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[actix_web::test]
    async fn get_board_returns_enrichment_fields() {
        let dir = make_temp_epics_brain_root();
        let registry = registry_with_epics_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/board?scope=hq")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = test::read_body_json(resp).await;
        let lanes = &body["lanes"];
        let all_blocks = lanes["now"]
            .as_array()
            .unwrap()
            .iter()
            .chain(lanes["next"].as_array().unwrap())
            .chain(lanes["blocked"].as_array().unwrap())
            .chain(lanes["finished"].as_array().unwrap())
            .collect::<Vec<_>>();

        let entry = all_blocks
            .iter()
            .find(|b| b["id"] == "BA.11.R")
            .unwrap_or_else(|| panic!("BA.11.R must appear in some lane; got {body:?}"));
        assert_eq!(entry["epics"][0], "bastion-surfaces");
        assert_eq!(entry["wave"], 3);
        assert_eq!(entry["priority"], 1);
        assert_eq!(entry["due"], "2026-07-15");
        assert_eq!(entry["track"], "Phase 11");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── /api/board — `last_touched` route-level coverage (BA.11.S task 4) ──

    /// [`make_temp_epics_brain_root`]'s corpus, plus an on-disk SDLC spec
    /// folder for `BA.11.R` only
    /// (`bastion/planning/BA.11.R-epic-ranking-enrichment/sdlc/sdlc-flow-state.json`)
    /// carrying `updated_at`. `BA.11.K` (and every other authored block)
    /// deliberately gets no spec folder, so `derive_last_touched` — and
    /// therefore the board response — must omit `last_touched` for it
    /// entirely rather than reporting `None`-as-`null`.
    fn make_temp_epics_brain_root_with_last_touched() -> std::path::PathBuf {
        let dir = make_temp_epics_brain_root();
        let ba_11_r_sdlc_dir = dir
            .join("bastion")
            .join("planning")
            .join("BA.11.R-epic-ranking-enrichment")
            .join("sdlc");
        std::fs::create_dir_all(&ba_11_r_sdlc_dir).unwrap();
        std::fs::write(
            ba_11_r_sdlc_dir.join("sdlc-flow-state.json"),
            r#"{"updated_at": "2026-07-29T09:00:00Z"}"#,
        )
        .unwrap();
        dir
    }

    /// Collect every lane entry (all five lanes) into one flat `Vec` for
    /// lookup-by-id assertions, mirroring [`get_board_returns_enrichment_fields`]'s
    /// `all_blocks` pattern but including `deferred` too.
    fn all_board_lane_entries(body: &serde_json::Value) -> Vec<&serde_json::Value> {
        let lanes = &body["lanes"];
        lanes["now"]
            .as_array()
            .unwrap()
            .iter()
            .chain(lanes["next"].as_array().unwrap())
            .chain(lanes["blocked"].as_array().unwrap())
            .chain(lanes["deferred"].as_array().unwrap())
            .chain(lanes["finished"].as_array().unwrap())
            .collect::<Vec<_>>()
    }

    #[actix_web::test]
    async fn get_board_route_reports_last_touched_verbatim_for_matched_block() {
        let dir = make_temp_epics_brain_root_with_last_touched();
        let registry = registry_with_epics_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/board?scope=hq")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            200,
            "GET /api/board?scope=hq with a valid token must return 200; got {}",
            resp.status()
        );

        let body: serde_json::Value = test::read_body_json(resp).await;
        let all_blocks = all_board_lane_entries(&body);

        let ba_11_r = all_blocks
            .iter()
            .find(|b| b["id"] == "BA.11.R")
            .unwrap_or_else(|| panic!("BA.11.R must appear in some lane; got {body:?}"));
        assert_eq!(
            ba_11_r["last_touched"], "2026-07-29T09:00:00Z",
            "BA.11.R's spec folder carries an updated_at that must surface verbatim on the board DTO"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[actix_web::test]
    async fn get_board_route_omits_last_touched_key_for_unmatched_block() {
        let dir = make_temp_epics_brain_root_with_last_touched();
        let registry = registry_with_epics_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/board?scope=hq")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = test::read_body_json(resp).await;
        let all_blocks = all_board_lane_entries(&body);

        let ba_11_k = all_blocks
            .iter()
            .find(|b| b["id"] == "BA.11.K")
            .unwrap_or_else(|| panic!("BA.11.K must appear in some lane; got {body:?}"));
        assert!(
            ba_11_k.get("last_touched").is_none(),
            "BA.11.K has no SDLC spec folder, so last_touched must be an absent key, not null: {ba_11_k:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[actix_web::test]
    async fn get_board_route_rejects_missing_token_with_401_even_with_last_touched_fixture() {
        let dir = make_temp_epics_brain_root_with_last_touched();
        let registry = registry_with_epics_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/board?scope=hq")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            401,
            "GET /api/board without a token must still return 401 with the last_touched fixture; got {}",
            resp.status()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[actix_web::test]
    async fn get_board_populates_blocked_by_outside_the_blocked_lane() {
        let dir = make_temp_epics_brain_root();
        let registry = registry_with_epics_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/board?scope=hq")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = test::read_body_json(resp).await;
        let lanes = &body["lanes"];

        // `BA.11.K` is unambiguously in `finished` (closed status) — assert
        // `blocked_by` enrichment reaches that lane too (per 5.4's sanity
        // note: whichever lane `BA.11.S` actually lands in among
        // now/next/blocked is derivation-dependent, but `finished` is not).
        let finished = lanes["finished"].as_array().unwrap();
        let k_entry = finished
            .iter()
            .find(|b| b["id"] == "BA.11.K")
            .unwrap_or_else(|| panic!("BA.11.K must appear in finished; got {finished:?}"));
        assert!(
            k_entry["blocked_by"].is_array(),
            "finished-lane entries must carry a blocked_by array"
        );

        // `BA.11.T` depends on the already-closed `BA.11.K`, so wherever it
        // lands its dependency is met and `blocked_by` must be empty.
        let all_blocks = lanes["now"]
            .as_array()
            .unwrap()
            .iter()
            .chain(lanes["next"].as_array().unwrap())
            .chain(lanes["blocked"].as_array().unwrap())
            .chain(lanes["finished"].as_array().unwrap())
            .collect::<Vec<_>>();
        let t_entry = all_blocks
            .iter()
            .find(|b| b["id"] == "BA.11.T")
            .unwrap_or_else(|| panic!("BA.11.T must appear in some lane; got {body:?}"));
        assert_eq!(
            t_entry["blocked_by"],
            serde_json::json!([]),
            "BA.11.T's sole dependency is closed, so blocked_by must be empty"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[actix_web::test]
    async fn get_board_epic_scope_filters_to_that_epic() {
        let dir = make_temp_epics_brain_root();
        let registry = registry_with_epics_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/board?scope=epic&epic=bastion-surfaces")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = test::read_body_json(resp).await;
        let lanes = &body["lanes"];
        let allowed = ["BA.11.R", "BA.11.S", "BA.11.K"];
        for lane_name in ["now", "next", "blocked", "finished"] {
            for block in lanes[lane_name].as_array().unwrap() {
                let id = block["id"].as_str().unwrap();
                assert!(
                    allowed.contains(&id),
                    "unexpected block {id} in scope=epic&epic=bastion-surfaces lane {lane_name}"
                );
                assert_ne!(
                    id, "BA.11.T",
                    "BA.11.T is tagged brain-rag, not bastion-surfaces, and must not appear"
                );
            }
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[actix_web::test]
    async fn get_board_epic_scope_unknown_slug_returns_404_c005() {
        let dir = make_temp_epics_brain_root();
        let registry = registry_with_epics_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/board?scope=epic&epic=nope")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 404);

        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "C005");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[actix_web::test]
    async fn get_board_epic_scope_without_epic_param_returns_404_c005() {
        let dir = make_temp_epics_brain_root();
        let registry = registry_with_epics_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/board?scope=epic")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 404);

        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "C005");
        assert!(
            body["message"]
                .as_str()
                .unwrap_or_default()
                .to_lowercase()
                .contains("epic"),
            "message must name the missing 'epic' param; got {body:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[actix_web::test]
    async fn get_board_epic_scope_rejects_missing_token_with_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get()
            .uri("/api/board?scope=epic&epic=bastion-surfaces")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            401,
            "GET /api/board?scope=epic without a token must return 401; got {}",
            resp.status()
        );
    }

    // ── /api/board — block_graph enrichment route-level coverage
    //    (`plan-board-graph-enrichment` task 6, A5) ──────────────────────────

    /// `make_temp_epics_brain_root`'s four-block corpus (`BA.11.R`/`S`/`T`/`K`,
    /// see its own doc comment) exercises every enrichment case at once when
    /// requested with `?graph=true`: `BA.11.R` (now) has one corpus-wide
    /// dependent (`BA.11.S`) and is not itself ready (its own dependency graph
    /// membership aside, mev's `ready` reports it `false` here because it sits
    /// mid-DAG, not a leaf); `BA.11.T` (next) has zero dependents and IS ready
    /// (its sole dependency, `BA.11.K`, is closed); `BA.11.S` (blocked) is the
    /// only entry carrying `unmet_count` (`1`, from its unmet dependency on
    /// `BA.11.R`); `BA.11.K` (finished) has one corpus-wide dependent
    /// (`BA.11.T`). These exact values were captured from a live run of this
    /// fixture through the route and are asserted verbatim below so a
    /// regression in any of `board.rs`'s five lane branches is caught at the
    /// wire, not just at the `build_board` unit level (task 4/5 already cover
    /// that layer).
    #[actix_web::test]
    async fn get_board_route_with_graph_true_returns_dependent_count_and_ready_across_lanes() {
        let dir = make_temp_epics_brain_root();
        let registry = registry_with_epics_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/board?scope=hq&graph=true")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            200,
            "GET /api/board?scope=hq&graph=true with a valid token must return 200; got {}",
            resp.status()
        );

        let body: serde_json::Value = test::read_body_json(resp).await;
        let lanes = &body["lanes"];

        let now_r = &lanes["now"].as_array().unwrap()[0];
        assert_eq!(now_r["id"], "BA.11.R");
        assert_eq!(
            now_r["dependent_count"],
            serde_json::json!(1),
            "BA.11.R has one corpus-wide dependent (BA.11.S); got {now_r:?}"
        );
        assert_eq!(now_r["ready"], serde_json::json!(false), "{now_r:?}");

        let next_t = &lanes["next"].as_array().unwrap()[0];
        assert_eq!(next_t["id"], "BA.11.T");
        assert_eq!(
            next_t["dependent_count"],
            serde_json::json!(0),
            "BA.11.T has zero corpus-wide dependents; got {next_t:?}"
        );
        assert_eq!(
            next_t["ready"],
            serde_json::json!(true),
            "BA.11.T's sole dependency is closed, so it is ready; got {next_t:?}"
        );

        let finished_k = &lanes["finished"].as_array().unwrap()[0];
        assert_eq!(finished_k["id"], "BA.11.K");
        assert_eq!(
            finished_k["dependent_count"],
            serde_json::json!(1),
            "BA.11.K has one corpus-wide dependent (BA.11.T); got {finished_k:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[actix_web::test]
    async fn get_board_route_with_graph_true_scopes_unmet_count_to_blocked_lane_only() {
        let dir = make_temp_epics_brain_root();
        let registry = registry_with_epics_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/board?scope=hq&graph=true")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = test::read_body_json(resp).await;
        let lanes = &body["lanes"];

        let blocked_s = &lanes["blocked"].as_array().unwrap()[0];
        assert_eq!(blocked_s["id"], "BA.11.S");
        assert_eq!(
            blocked_s["unmet_count"],
            serde_json::json!(1),
            "the only blocked-lane entry must carry mev's raw unmet_count; got {blocked_s:?}"
        );

        // `now`/`next`/`finished` each have exactly one entry in this fixture
        // (`deferred` is empty) — every one of them must OMIT the
        // `unmet_count` key entirely, not carry a `null` or a fabricated `0`.
        for (lane_name, entry) in [
            ("now", &lanes["now"].as_array().unwrap()[0]),
            ("next", &lanes["next"].as_array().unwrap()[0]),
            ("finished", &lanes["finished"].as_array().unwrap()[0]),
        ] {
            let obj = entry.as_object().unwrap();
            assert!(
                !obj.contains_key("unmet_count"),
                "{lane_name} lane entry {entry:?} must not carry an unmet_count key at all"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[actix_web::test]
    async fn get_board_route_without_graph_param_omits_all_three_enrichment_keys() {
        let dir = make_temp_epics_brain_root();
        let registry = registry_with_epics_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        // No `?graph=true` — task 1's gating decision means `assemble_board`
        // never calls `mev::build_block_graph_export` at all, so every block
        // on the board has "no graph entry" and must omit all three keys.
        let req = test::TestRequest::get()
            .uri("/api/board?scope=hq")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = test::read_body_json(resp).await;
        let all_blocks = all_board_lane_entries(&body);
        assert!(
            !all_blocks.is_empty(),
            "fixture must produce at least one board entry to make this assertion meaningful"
        );

        for entry in all_blocks {
            let obj = entry.as_object().unwrap();
            for key in ["dependent_count", "ready", "unmet_count"] {
                assert!(
                    !obj.contains_key(key),
                    "without ?graph=true, entry {entry:?} must omit `{key}` entirely \
                     (an absent key, not `null` or a fabricated zero/false)"
                );
            }
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[actix_web::test]
    async fn get_board_route_rejects_missing_token_with_401_even_with_graph_true() {
        let dir = make_temp_epics_brain_root();
        let registry = registry_with_epics_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/board?scope=hq&graph=true")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            401,
            "GET /api/board?graph=true without a token must still return 401; got {}",
            resp.status()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── /api/blocks/graph — route registration (BA.17.A) ─────────────────────

    #[actix_web::test]
    async fn get_blocks_graph_rejects_missing_token_with_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get()
            .uri("/api/blocks/graph?scope=hq")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            401,
            "GET /api/blocks/graph without a token must return 401; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn get_blocks_graph_hq_scope_returns_200_with_well_formed_body() {
        let dir = make_temp_board_brain_root();
        let registry = registry_with_board_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/blocks/graph?scope=hq")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            200,
            "GET /api/blocks/graph?scope=hq with a valid token must return 200"
        );

        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["scope"], "hq");
        assert!(body["version"].is_string(), "version must be a string");
        assert!(body["root"].is_string(), "root must be a string");
        assert!(body["nodes"].is_array(), "nodes must be an array");
        assert!(body["edges"].is_array(), "edges must be an array");
        assert!(body["cycles"].is_array(), "cycles must be an array");
        assert!(
            body["total_nodes"].is_number(),
            "total_nodes must be a number"
        );
        assert!(
            body["truncated"].is_boolean(),
            "truncated must be a boolean"
        );
        assert!(body["stale"].is_boolean(), "stale must be a boolean");
        let nodes = body["nodes"].as_array().unwrap();
        assert_eq!(
            nodes.len(),
            1,
            "the single fixture block must appear as one node"
        );
        assert_eq!(nodes[0]["key"], "bastion:BA.11.K");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[actix_web::test]
    async fn get_blocks_graph_epic_scope_without_epic_param_returns_404_c005_same_shape_as_board() {
        let dir = make_temp_epics_brain_root();
        let registry = registry_with_epics_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        let graph_req = test::TestRequest::get()
            .uri("/api/blocks/graph?scope=epic")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let graph_resp = test::call_service(&app, graph_req).await;
        assert_eq!(graph_resp.status(), 404);
        let graph_body: serde_json::Value = test::read_body_json(graph_resp).await;

        let board_req = test::TestRequest::get()
            .uri("/api/board?scope=epic")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let board_resp = test::call_service(&app, board_req).await;
        assert_eq!(board_resp.status(), 404);
        let board_body: serde_json::Value = test::read_body_json(board_resp).await;

        assert_eq!(graph_body["code"], "C005");
        assert_eq!(
            graph_body["code"], board_body["code"],
            "scope=epic with a missing &epic= must return the same code shape on both routes"
        );
        assert!(
            graph_body["message"]
                .as_str()
                .unwrap_or_default()
                .to_lowercase()
                .contains("epic"),
            "message must name the missing 'epic' param; got {graph_body:?}"
        );
        assert_eq!(
            graph_body.as_object().map(|o| {
                let mut keys: Vec<&String> = o.keys().collect();
                keys.sort();
                keys
            }),
            board_body.as_object().map(|o| {
                let mut keys: Vec<&String> = o.keys().collect();
                keys.sort();
                keys
            }),
            "the 404/C005 payload shape (field set) must be byte-identical between the two routes"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[actix_web::test]
    async fn get_blocks_graph_epic_scope_unknown_slug_returns_404_c005_same_shape_as_board() {
        let dir = make_temp_epics_brain_root();
        let registry = registry_with_epics_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        let graph_req = test::TestRequest::get()
            .uri("/api/blocks/graph?scope=epic&epic=nope")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let graph_resp = test::call_service(&app, graph_req).await;
        assert_eq!(graph_resp.status(), 404);
        let graph_body: serde_json::Value = test::read_body_json(graph_resp).await;

        let board_req = test::TestRequest::get()
            .uri("/api/board?scope=epic&epic=nope")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let board_resp = test::call_service(&app, board_req).await;
        assert_eq!(board_resp.status(), 404);
        let board_body: serde_json::Value = test::read_body_json(board_resp).await;

        assert_eq!(graph_body["code"], "C005");
        assert_eq!(graph_body["code"], board_body["code"]);
        assert_eq!(
            graph_body.as_object().map(|o| {
                let mut keys: Vec<&String> = o.keys().collect();
                keys.sort();
                keys
            }),
            board_body.as_object().map(|o| {
                let mut keys: Vec<&String> = o.keys().collect();
                keys.sort();
                keys
            }),
            "the 404/C005 payload shape (field set) must be byte-identical between the two routes"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[actix_web::test]
    async fn get_blocks_graph_max_nodes_truncates_end_to_end() {
        let dir = make_temp_epics_brain_root();
        let registry = registry_with_epics_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        // Baseline: default max_nodes (400) — three non-closed blocks
        // (BA.11.R/S/T; BA.11.K is closed and excluded by default), not
        // truncated.
        let baseline_req = test::TestRequest::get()
            .uri("/api/blocks/graph?scope=hq")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let baseline_resp = test::call_service(&app, baseline_req).await;
        assert_eq!(baseline_resp.status(), 200);
        let baseline_body: serde_json::Value = test::read_body_json(baseline_resp).await;
        assert_eq!(baseline_body["total_nodes"], 3);
        assert_eq!(baseline_body["nodes"].as_array().unwrap().len(), 3);
        assert_eq!(baseline_body["truncated"], false);

        // max_nodes=1 — total_nodes still reports the pre-truncation count,
        // but the returned node list is capped.
        let truncated_req = test::TestRequest::get()
            .uri("/api/blocks/graph?scope=hq&max_nodes=1")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let truncated_resp = test::call_service(&app, truncated_req).await;
        assert_eq!(truncated_resp.status(), 200);
        let truncated_body: serde_json::Value = test::read_body_json(truncated_resp).await;
        assert_eq!(truncated_body["total_nodes"], 3);
        let returned = truncated_body["nodes"].as_array().unwrap();
        assert_eq!(returned.len(), 1);
        assert_eq!(truncated_body["truncated"], true);
        assert!(
            (returned.len() as u64) < truncated_body["total_nodes"].as_u64().unwrap(),
            "truncated response must return fewer nodes than total_nodes"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[actix_web::test]
    async fn get_blocks_graph_include_closed_and_repo_are_threaded_into_scope() {
        let dir = make_temp_epics_brain_root();
        let registry = registry_with_epics_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        // Default include_closed=false: BA.11.K (closed) is excluded.
        let default_req = test::TestRequest::get()
            .uri("/api/blocks/graph?scope=hq")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let default_resp = test::call_service(&app, default_req).await;
        let default_body: serde_json::Value = test::read_body_json(default_resp).await;
        let default_keys: Vec<&str> = default_body["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["key"].as_str().unwrap())
            .collect();
        assert!(
            !default_keys.contains(&"bastion:BA.11.K"),
            "include_closed=false (default) must exclude the closed block; got {default_keys:?}"
        );

        // include_closed=true: BA.11.K reappears.
        let included_req = test::TestRequest::get()
            .uri("/api/blocks/graph?scope=hq&include_closed=true")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let included_resp = test::call_service(&app, included_req).await;
        let included_body: serde_json::Value = test::read_body_json(included_resp).await;
        let included_keys: Vec<&str> = included_body["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["key"].as_str().unwrap())
            .collect();
        assert!(
            included_keys.contains(&"bastion:BA.11.K"),
            "include_closed=true must thread through and include the closed block; got {included_keys:?}"
        );

        // repo=nonexistent-repo: no repo matches, so the node list is empty —
        // proves `repo` narrows `BlockGraphScope.repo` rather than being
        // ignored.
        let repo_req = test::TestRequest::get()
            .uri("/api/blocks/graph?scope=hq&repo=nonexistent-repo")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let repo_resp = test::call_service(&app, repo_req).await;
        let repo_body: serde_json::Value = test::read_body_json(repo_resp).await;
        assert!(
            repo_body["nodes"].as_array().unwrap().is_empty(),
            "repo=nonexistent-repo must exclude every node; got {:?}",
            repo_body["nodes"]
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Live run-state routes (BA.11.M) ───────────────────────────────────

    #[actix_web::test]
    async fn list_runs_rejects_missing_token_with_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get().uri("/api/runs").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            401,
            "GET /api/runs without a token must return 401; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn list_runs_returns_200_empty_array_when_store_empty() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get()
            .uri("/api/runs")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            200,
            "GET /api/runs with a valid token must return 200; got {}",
            resp.status()
        );

        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(
            body,
            serde_json::json!([]),
            "GET /api/runs must return an empty array when no run is tracked; got {body}"
        );
    }

    #[actix_web::test]
    async fn get_run_returns_404_for_unknown_run_id() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let unknown_id = uuid::Uuid::new_v4();
        let req = test::TestRequest::get()
            .uri(&format!("/api/runs/{unknown_id}"))
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            404,
            "GET /api/runs/{{id}} for an unknown run must return 404; got {}",
            resp.status()
        );
    }

    // ── run_id -> repo join route tests (A7, ?with_repo=1) ──────────────────

    #[actix_web::test]
    async fn list_runs_with_repo_rejects_missing_token_with_401() {
        let app = test::init_service(build_runs_test_app(
            FileConfig::default(),
            LiveStateStore::new(),
        ))
        .await;
        let req = test::TestRequest::get()
            .uri("/api/runs?with_repo=1")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            401,
            "GET /api/runs?with_repo=1 without a token must return 401; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn list_runs_with_repo_returns_repo_for_run_matching_flow_state() {
        // Flow-state fixture carries run_id "a1b2c3d4-e5f6-4789-a012-3456789abcde"
        // (FLOW_JSON_WITH_RUN_ID) under workspace "repo-x".
        let (_tmp, registry) = registry_with_fixture_repo_with_run_id();
        let live = LiveStateStore::new();
        let run_id: uuid::Uuid = "a1b2c3d4-e5f6-4789-a012-3456789abcde".parse().unwrap();
        live.record(run_id, &empty_task_context());

        let app = test::init_service(build_runs_test_app(registry, live)).await;
        let req = test::TestRequest::get()
            .uri("/api/runs?with_repo=1")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = test::read_body_json(resp).await;
        let runs = body.as_array().expect("response must be a JSON array");
        assert_eq!(runs.len(), 1, "expected exactly one active run; got {body}");
        assert_eq!(
            runs[0]["repo"], "repo-x",
            "run whose id matches a flow state's run_id must carry that repo; got {body}"
        );
    }

    #[actix_web::test]
    async fn list_runs_with_repo_omits_repo_key_for_unmatched_run() {
        // Same fixture repo/flow-state as above, but the *active* run id is a
        // fresh UUID that appears in no flow state — the honest-degradation path.
        let (_tmp, registry) = registry_with_fixture_repo_with_run_id();
        let live = LiveStateStore::new();
        let run_id = uuid::Uuid::new_v4();
        live.record(run_id, &empty_task_context());

        let app = test::init_service(build_runs_test_app(registry, live)).await;
        let req = test::TestRequest::get()
            .uri("/api/runs?with_repo=1")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = test::read_body_json(resp).await;
        let runs = body.as_array().expect("response must be a JSON array");
        assert_eq!(runs.len(), 1, "expected exactly one active run; got {body}");
        assert!(
            !runs[0]
                .as_object()
                .expect("run entry must be an object")
                .contains_key("repo"),
            "an unmatched run must omit the repo key entirely, never null; got {body}"
        );
    }

    #[actix_web::test]
    async fn list_runs_with_repo_empty_registry_returns_200_with_repo_absent() {
        // Empty registry (no registered workspaces) must still be 200, never a
        // 404/500 — matches A2's "absent registry is 200 [], never a 404".
        let live = LiveStateStore::new();
        let run_id = uuid::Uuid::new_v4();
        live.record(run_id, &empty_task_context());

        let app = test::init_service(build_runs_test_app(FileConfig::default(), live)).await;
        let req = test::TestRequest::get()
            .uri("/api/runs?with_repo=1")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            200,
            "GET /api/runs?with_repo=1 against an empty registry must return 200; got {}",
            resp.status()
        );

        let body: serde_json::Value = test::read_body_json(resp).await;
        let runs = body.as_array().expect("response must be a JSON array");
        assert_eq!(runs.len(), 1, "expected exactly one active run; got {body}");
        assert!(
            !runs[0]
                .as_object()
                .expect("run entry must be an object")
                .contains_key("repo"),
            "empty registry must leave repo absent, never null or an error; got {body}"
        );
    }

    // ── Attention / carryover board route (BA.11.P) ────────────────────────

    /// Temp brain root for `/api/attention` tests — same overall shape as
    /// [`make_temp_board_brain_root`] (a `brain.toml` + a leaf `planning/state.json`),
    /// but the root's own `planning/state.json` is `kind: "brain"` (it's the HQ
    /// file `discover_state_files` resolves at `<root>/planning/state.json`) and
    /// carries `carryover[]` + `backlog[]` fixtures: one stale carryover item,
    /// one plain aging-backlog item, one capture-origin item (→ `orphaned_captures`),
    /// one snoozed item, and one under-threshold (recent) item — all with dates
    /// far enough in the past (or future, for the snooze) to be deterministic
    /// regardless of when the test runs.
    fn make_temp_attention_brain_root() -> std::path::PathBuf {
        let dir = crate::testsupport::unique_temp_dir("bastion-serve-attention");
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
  "kind": "brain",
  "updated": "2026-07-04",
  "focus": { "now": [], "next": [], "blocked": [] },
  "tracks": [],
  "carryover": [
    {
      "slug": "stale-known-issue",
      "scope": {},
      "kind": "known_issue",
      "text": "stale carryover fixture item",
      "created": "2026-01-01"
    }
  ],
  "backlog": [
    {
      "slug": "aging-idea",
      "title": "Aging backlog fixture item",
      "repo": "bastion",
      "type": "feature",
      "status": "idea",
      "created": "2026-01-01"
    },
    {
      "slug": "captured-idea",
      "title": "Orphaned capture fixture item",
      "repo": "bastion",
      "type": "feature",
      "status": "idea",
      "created": "2026-01-01",
      "origin": { "type": "capture", "notes": "planning/captured-idea/notes.md" }
    },
    {
      "slug": "snoozed-idea",
      "title": "Snoozed backlog fixture item",
      "repo": "bastion",
      "type": "feature",
      "status": "idea",
      "created": "2026-01-01",
      "snoozed_until": "2099-01-01"
    },
    {
      "slug": "fresh-idea",
      "title": "Under-threshold backlog fixture item",
      "repo": "bastion",
      "type": "feature",
      "status": "idea",
      "created": "2999-01-01"
    }
  ]
}"#,
        )
        .unwrap();

        dir
    }

    /// Registry pointing `default_workspace` at the temp attention brain root —
    /// mirrors [`registry_with_board_fixture`].
    fn registry_with_attention_fixture(brain_root: &std::path::Path) -> FileConfig {
        let mut workspaces = std::collections::HashMap::new();
        workspaces.insert("brain-root".to_string(), brain_root.to_path_buf());
        FileConfig {
            workspaces: Some(workspaces),
            default_workspace: Some("brain-root".to_string()),
            ..Default::default()
        }
    }

    #[actix_web::test]
    async fn get_attention_rejects_missing_token_with_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get()
            .uri("/api/attention?scope=hq")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            401,
            "GET /api/attention without a token must return 401; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn get_attention_hq_scope_returns_200_with_three_lanes() {
        let dir = make_temp_attention_brain_root();
        let registry = registry_with_attention_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/attention?scope=hq")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            200,
            "GET /api/attention?scope=hq with a valid token must return 200; got {}",
            resp.status()
        );

        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["scope"], "hq");
        assert!(body["as_of"].is_string(), "as_of must be a string");
        assert!(
            body["thresholds"].is_object(),
            "thresholds must be an object"
        );

        let lanes = &body["lanes"];
        let stale_carryover = lanes["stale_carryover"]
            .as_array()
            .expect("stale_carryover must be an array");
        let aging_backlog = lanes["aging_backlog"]
            .as_array()
            .expect("aging_backlog must be an array");
        let orphaned_captures = lanes["orphaned_captures"]
            .as_array()
            .expect("orphaned_captures must be an array");

        assert!(
            stale_carryover
                .iter()
                .any(|c| c["slug"] == "stale-known-issue"),
            "stale carryover fixture item must appear in stale_carryover; got {stale_carryover:?}"
        );
        assert!(
            aging_backlog.iter().any(|b| b["slug"] == "aging-idea"),
            "aging backlog fixture item must appear in aging_backlog; got {aging_backlog:?}"
        );
        assert!(
            orphaned_captures
                .iter()
                .any(|b| b["slug"] == "captured-idea"),
            "capture fixture item must appear in orphaned_captures; got {orphaned_captures:?}"
        );
        assert!(
            !aging_backlog.iter().any(|b| b["slug"] == "captured-idea"),
            "capture fixture item must NOT appear in aging_backlog"
        );
        assert!(
            !aging_backlog.iter().any(|b| b["slug"] == "snoozed-idea"),
            "snoozed fixture item must be absent from aging_backlog"
        );
        assert!(
            !aging_backlog.iter().any(|b| b["slug"] == "fresh-idea"),
            "under-threshold fixture item must be absent from aging_backlog"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[actix_web::test]
    async fn get_attention_unknown_tier_returns_200_with_empty_lanes() {
        let dir = make_temp_attention_brain_root();
        let registry = registry_with_attention_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/attention?scope=tier&tier=nonexistent-tier")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            200,
            "an unknown tier must still return 200; got {}",
            resp.status()
        );

        let body: serde_json::Value = test::read_body_json(resp).await;
        let lanes = &body["lanes"];
        assert_eq!(lanes["stale_carryover"], serde_json::json!([]));
        assert_eq!(lanes["aging_backlog"], serde_json::json!([]));
        assert_eq!(lanes["orphaned_captures"], serde_json::json!([]));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[actix_web::test]
    async fn get_attention_unrecognized_scope_returns_400() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get()
            .uri("/api/attention?scope=bogus")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            400,
            "an unrecognized scope must return 400; got {}",
            resp.status()
        );
    }

    // ── Docs read routes (BA.11.Q) ───────────────────────────────────────

    /// Build a fixture repo whose `planning/` is a REAL symlink pointing at a
    /// sibling "vault" directory — mirroring the real repos where
    /// `planning/` is a symlink into the company-brain vault. Exercises the
    /// canonicalize-the-allowlisted-root-not-the-repo-root rule end to end
    /// (BA.11.Q task 2's core regression case).
    ///
    /// Layout:
    /// ```text
    /// <repo>/
    ///   docs/served.md
    ///   planning -> <vault>/          (symlink)
    ///   .env                          (non-markdown, must be excluded)
    /// <vault>/
    ///   status.md
    /// ```
    ///
    /// Returns `(repo_dir, vault_dir)` — callers own teardown of both.
    fn make_temp_docs_fixture() -> (std::path::PathBuf, std::path::PathBuf) {
        let base = crate::testsupport::unique_temp_dir("bastion-serve-docs");
        let repo_dir = base.join("repo");
        let vault_dir = base.join("vault");
        std::fs::create_dir_all(repo_dir.join("docs")).unwrap();
        std::fs::create_dir_all(&vault_dir).unwrap();

        std::fs::write(
            repo_dir.join("docs/served.md"),
            "# Served\n\nDocs tree fixture content.\n",
        )
        .unwrap();
        std::fs::write(
            vault_dir.join("status.md"),
            "# Status\n\nSymlinked planning fixture content.\n",
        )
        .unwrap();
        std::fs::write(repo_dir.join(".env"), "SECRET=nope\n").unwrap();

        #[cfg(unix)]
        std::os::unix::fs::symlink(&vault_dir, repo_dir.join("planning")).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&vault_dir, repo_dir.join("planning")).unwrap();

        (repo_dir, vault_dir)
    }

    /// Registry naming the fixture repo `docs-repo`.
    fn registry_with_docs_fixture(repo_dir: &std::path::Path) -> FileConfig {
        let mut workspaces = std::collections::HashMap::new();
        workspaces.insert("docs-repo".to_string(), repo_dir.to_path_buf());
        FileConfig {
            workspaces: Some(workspaces),
            ..Default::default()
        }
    }

    fn teardown_docs_fixture(repo_dir: &std::path::Path, vault_dir: &std::path::Path) {
        let _ = std::fs::remove_dir_all(repo_dir);
        let _ = std::fs::remove_dir_all(vault_dir);
    }

    #[actix_web::test]
    async fn get_docs_tree_rejects_missing_token_with_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get()
            .uri("/api/docs/docs-repo/tree")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            401,
            "GET /api/docs/{{repo}}/tree without a token must return 401; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn get_docs_file_rejects_missing_token_with_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get()
            .uri("/api/docs/docs-repo/file?path=docs/served.md")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            401,
            "GET /api/docs/{{repo}}/file without a token must return 401; got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn get_docs_tree_returns_200_with_docs_and_symlinked_planning_excluding_env() {
        let (repo_dir, vault_dir) = make_temp_docs_fixture();
        let registry = registry_with_docs_fixture(&repo_dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/docs/docs-repo/tree")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            200,
            "GET /api/docs/docs-repo/tree with a valid token must return 200; got {}",
            resp.status()
        );

        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["repo"], "docs-repo");
        let entries = body["entries"]
            .as_array()
            .expect("entries must be an array");

        // build_doc_tree returns only the top-level listing for root: "" —
        // assert both allowlisted-root directories are present and the
        // non-markdown .env file never appears anywhere in the response.
        let body_str = body.to_string();
        assert!(
            entries.iter().any(|e| e["name"] == "docs"),
            "tree must include the docs/ directory; got {entries:?}"
        );
        assert!(
            entries.iter().any(|e| e["name"] == "planning"),
            "tree must include the symlinked planning/ directory; got {entries:?}"
        );
        assert!(
            !body_str.contains(".env"),
            "tree must never include the non-markdown .env file; got {body_str}"
        );

        teardown_docs_fixture(&repo_dir, &vault_dir);
    }

    #[actix_web::test]
    async fn get_docs_file_returns_200_raw_content_through_symlinked_planning_root() {
        let (repo_dir, vault_dir) = make_temp_docs_fixture();
        let registry = registry_with_docs_fixture(&repo_dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/docs/docs-repo/file?path=planning/status.md")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            200,
            "GET /api/docs/docs-repo/file through the symlinked planning/ root must return 200; got {}",
            resp.status()
        );

        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["repo"], "docs-repo");
        assert_eq!(body["path"], "planning/status.md");
        assert_eq!(
            body["content"], "# Status\n\nSymlinked planning fixture content.\n",
            "raw content must match the file on disk through the symlinked planning/ vault byte-for-byte"
        );

        teardown_docs_fixture(&repo_dir, &vault_dir);
    }

    #[actix_web::test]
    async fn get_docs_file_traversal_dot_dot_returns_403_c003() {
        let (repo_dir, vault_dir) = make_temp_docs_fixture();
        let registry = registry_with_docs_fixture(&repo_dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/docs/docs-repo/file?path=../../etc/passwd")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 403);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "C003");

        teardown_docs_fixture(&repo_dir, &vault_dir);
    }

    #[actix_web::test]
    async fn get_docs_file_bare_dotenv_returns_403_c003() {
        let (repo_dir, vault_dir) = make_temp_docs_fixture();
        let registry = registry_with_docs_fixture(&repo_dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/docs/docs-repo/file?path=.env")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 403);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "C003");

        teardown_docs_fixture(&repo_dir, &vault_dir);
    }

    #[actix_web::test]
    async fn get_docs_file_absolute_path_returns_403_c003() {
        let (repo_dir, vault_dir) = make_temp_docs_fixture();
        let registry = registry_with_docs_fixture(&repo_dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/docs/docs-repo/file?path=%2Fetc%2Fpasswd")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 403);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "C003");

        teardown_docs_fixture(&repo_dir, &vault_dir);
    }

    #[actix_web::test]
    async fn get_docs_tree_unknown_repo_returns_404_c005() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get()
            .uri("/api/docs/no-such-repo/tree")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 404);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "C005");
    }

    #[actix_web::test]
    async fn get_docs_file_missing_file_returns_404_c002() {
        let (repo_dir, vault_dir) = make_temp_docs_fixture();
        let registry = registry_with_docs_fixture(&repo_dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/docs/docs-repo/file?path=docs/nonexistent.md")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 404);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "C002");

        teardown_docs_fixture(&repo_dir, &vault_dir);
    }

    // ── Pipeline / opportunities routes (BW.3.A) ──────────────────────────────

    const PIPELINE_MD: &str = "# Pipeline\n\n## Stages\n\n`identified` → `researching` → `contacted` → `conversation` → `proposal-sent` → `closed-won` → `closed-lost`\n\n---\n";
    const OPP_ANTHROPIC: &str = "---\ntitle: Anthropic\nkind: company\nstage: identified\nsource: RESEARCH_AGENT\n---\n\n# Anthropic\n\n## Research Brief\n```json\n{ \"company_name\": \"Anthropic\", \"summary\": \"An AI lab.\" }\n```\n";
    const OPP_LEAD: &str =
        "---\ntitle: Warm Lead\nkind: company\nstage: contacted\n---\n\n# Warm Lead\n";

    /// Build a temp brain root (with `brain.toml` so `find_brain_root` stops
    /// there) populated with `business/docs/...`, and a `FileConfig` whose
    /// `default_workspace` points at it so `resolve_workspace_root(None, None)`
    /// resolves the start path there.
    fn make_temp_pipeline_brain_root() -> std::path::PathBuf {
        let dir = crate::testsupport::unique_temp_dir("bastion-pipeline-route-test");
        let docs = dir.join("business").join("docs");
        std::fs::create_dir_all(docs.join("opportunities")).unwrap();
        std::fs::create_dir_all(docs.join("leads")).unwrap();
        std::fs::write(dir.join("brain.toml"), "").unwrap();
        std::fs::write(docs.join("pipeline.md"), PIPELINE_MD).unwrap();
        std::fs::write(docs.join("opportunities/anthropic.md"), OPP_ANTHROPIC).unwrap();
        std::fs::write(docs.join("opportunities/index.md"), "# index\n").unwrap();
        std::fs::write(docs.join("leads/warm-lead.md"), OPP_LEAD).unwrap();
        std::fs::write(docs.join("leads/README.md"), "# readme\n").unwrap();
        dir
    }

    fn registry_with_pipeline_fixture(brain_root: &std::path::Path) -> FileConfig {
        let mut workspaces = std::collections::HashMap::new();
        workspaces.insert("hq".to_string(), brain_root.to_path_buf());
        FileConfig {
            workspaces: Some(workspaces),
            default_workspace: Some("hq".to_string()),
            ..Default::default()
        }
    }

    #[actix_web::test]
    async fn get_pipeline_rejects_missing_token_with_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get().uri("/api/pipeline").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 401);
    }

    #[actix_web::test]
    async fn get_pipeline_opportunity_rejects_missing_token_with_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get()
            .uri("/api/pipeline/anthropic")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 401);
    }

    #[actix_web::test]
    async fn get_pipeline_wrong_method_returns_405() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::post()
            .uri("/api/pipeline")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 405);
    }

    #[actix_web::test]
    async fn get_pipeline_returns_200_with_stages_and_sorted_opportunities() {
        let dir = make_temp_pipeline_brain_root();
        let registry = registry_with_pipeline_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/pipeline")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["stages"][0], "identified");
        let opps = body["opportunities"].as_array().unwrap();
        // index.md/README.md skipped -> exactly two opportunities.
        assert_eq!(opps.len(), 2);
        // identified (anthropic) sorts before contacted (warm-lead).
        assert_eq!(opps[0]["slug"], "anthropic");
        assert_eq!(opps[0]["has_findings"], true);
        assert_eq!(opps[1]["slug"], "warm-lead");
        assert_eq!(opps[1]["has_findings"], false);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[actix_web::test]
    async fn get_pipeline_opportunity_returns_200_detail() {
        let dir = make_temp_pipeline_brain_root();
        let registry = registry_with_pipeline_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/pipeline/anthropic")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["slug"], "anthropic");
        assert_eq!(body["kind"], "company");
        assert_eq!(body["findings"]["kind"], "company");
        assert_eq!(body["findings"]["company_name"], "Anthropic");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[actix_web::test]
    async fn get_pipeline_opportunity_unknown_slug_returns_404_c002() {
        let dir = make_temp_pipeline_brain_root();
        let registry = registry_with_pipeline_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/pipeline/does-not-exist")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 404);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "C002");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
