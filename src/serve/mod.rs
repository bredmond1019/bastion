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
pub mod notify;
pub mod poll;
pub mod session_qa;
pub mod status;
pub mod ws;

use crate::config::{FileConfig, load_workspace_registry};
use actix::{Actor, Addr};
use actix_web::{App, HttpRequest, HttpResponse, HttpServer, web};
use actix_web_actors::ws as actix_ws;
use anyhow::Result;
use auth::{ApiKeyAuthMiddleware, BearerAuthMiddleware};
use dto::ErrorPayload;
use engine_serve::abort::RunRegistry;
use engine_serve::dispatch::Dispatcher;
use engine_serve::durable::spawn_durable_writer;
use engine_serve::http::AppState as EngineAppState;
use engine_serve::live_state::LiveStateStore;
use engine_serve::orphan::ReconcileSummary;
use engine_serve::workflows::{init_repo_registry_from_env, register_builtin_workflows};

/// Build the engine's `Dispatcher` with every builtin workflow (currently
/// just `SDLC_FLOW`) registered. Pulled out of `run()` so the wiring is
/// unit-testable without standing up actix/Postgres.
fn build_engine_dispatcher() -> Dispatcher {
    let mut dispatcher = Dispatcher::new();
    register_builtin_workflows(&mut dispatcher);
    dispatcher
}

/// The boot-time orphan sweep's outcome (`ticket-orphan-reconcile-wiring`
/// task 1), classified once so the caller logs exactly one deliberate line
/// instead of the "comes back up healthy in silence" failure mode this
/// ticket exists to end. `Skipped` covers the DB-free `:8080` console path
/// (never produced by [`classify_orphan_sweep`] itself, which only ever
/// sees a `Result` from an actual sweep call — callers reach for `Skipped`
/// directly when the pool is absent).
#[derive(Debug, Clone, PartialEq, Eq)]
enum OrphanSweepOutcome {
    Skipped,
    Swept {
        scanned: usize,
        reconciled: Vec<uuid::Uuid>,
    },
    SweptNothing {
        scanned: usize,
    },
    Failed(String),
}

/// Pure classifier: `Ok(summary)` with a non-empty `reconciled` is `Swept`;
/// `Ok(summary)` with an empty `reconciled` is `SweptNothing` (its `scanned`
/// may still be non-zero — a candidate found and reconciled onto a
/// completion marker by an earlier sweep is scanned again but not
/// re-reconciled); `Err` is `Failed`. No I/O, no logging — the caller
/// ([`log_orphan_sweep`]) emits the decision this returns.
fn classify_orphan_sweep(result: Result<ReconcileSummary, String>) -> OrphanSweepOutcome {
    match result {
        Ok(summary) if !summary.reconciled.is_empty() => OrphanSweepOutcome::Swept {
            scanned: summary.scanned,
            reconciled: summary.reconciled,
        },
        Ok(summary) => OrphanSweepOutcome::SweptNothing {
            scanned: summary.scanned,
        },
        Err(msg) => OrphanSweepOutcome::Failed(msg),
    }
}

/// Emit exactly one `tracing` line for a classified orphan-sweep outcome,
/// `target: "bastion::serve"` matching the surrounding boot-log style.
/// `Failed` logs at `error` (loudly, per the ticket's acceptance criteria)
/// but never propagates — a transient database hiccup at boot must not
/// prevent `bastion serve` from starting.
fn log_orphan_sweep(outcome: &OrphanSweepOutcome) {
    match outcome {
        OrphanSweepOutcome::Skipped => {
            tracing::info!(
                target: "bastion::serve",
                "orphan sweep skipped — engine not mounted (no DATABASE_URL)"
            );
        }
        OrphanSweepOutcome::Swept {
            scanned,
            reconciled,
        } => {
            tracing::info!(
                target: "bastion::serve",
                scanned,
                reconciled = ?reconciled,
                "orphan sweep reconciled {} crash-stranded run(s) of {} scanned",
                reconciled.len(),
                scanned
            );
        }
        OrphanSweepOutcome::SweptNothing { scanned } => {
            tracing::info!(
                target: "bastion::serve",
                scanned,
                "orphan sweep found nothing to reconcile ({scanned} candidate(s) scanned)"
            );
        }
        OrphanSweepOutcome::Failed(msg) => {
            tracing::error!(
                target: "bastion::serve",
                error = %msg,
                "orphan sweep failed at boot — server is still starting"
            );
        }
    }
}

/// Default path for the `ApproveAndRunSeams` approval ledger
/// (`ticket-approve-and-run-seams` task 2), mirroring
/// `blocked_edge::default_sink_path`'s exact XDG-first/`HOME`-fallback
/// precedence and directory convention (`src/serve/blocked_edge/sink.rs`) —
/// same env vars, same `~/.local/state/bastion/` (or `$XDG_STATE_HOME/
/// bastion/`) directory, just a different filename. Pulled out of `run()`
/// so the resolution is unit-testable without standing up actix.
///
/// Unlike its sibling, this never returns `None`: `FileApprovalLedger`
/// construction never touches the filesystem (the file and its parent
/// directory are created lazily on the first write, per its own doc
/// comment), so there is no boot-time decision to skip constructing the
/// ledger the way the blocked-edge poller skips spawning without a sink
/// path — a relative fallback filename in the process's current directory
/// is a safe, always-constructible default for the case neither var is set.
/// Resolve `gate_id` against the composed `PendingLookup` source
/// (`ticket-approve-and-run-seams` task 2): the real engine queue
/// (`seams.lookup_pending`) tried first, falling back to the process-local
/// `/api/notify/test` registry (`test_registry.get`). Pulled out of the
/// closure `run()` builds so the composition itself — precedence, and that
/// neither source is ever skipped — is unit-testable without standing up
/// actix or a Telegram config.
#[must_use]
fn resolve_pending_lookup(
    seams: &engine_core::workflows::approve_and_run::ApproveAndRunSeams,
    test_registry: &notify::PendingPayloads,
    gate_id: &str,
) -> Option<engine_core::operator::ValidatedOperatorPayload> {
    seams
        .lookup_pending(gate_id)
        .or_else(|| test_registry.get(gate_id))
}

/// Build the engine-side [`ApproveAndRunVerdict`][engine_core::workflows::approve_and_run::ApproveAndRunVerdict]
/// `ApproveAndRunSeams::resolve_verdict` needs from a resolved
/// [`notify::telegram::ResponseVerdict`] (`ticket-approve-and-run-seams` task
/// 3). `Accepted` and `StaleDigest` both now carry every field a verdict
/// needs — `gate_id`, `option_key`, `digest`, `decided_at` — so both convert
/// the same way; this function does **not** decide whether the digest
/// matches. That is deliberate: `engine_core`'s own `decide()` (reached
/// through `resolve_verdict`) already enforces mismatch -> a `Requeued`
/// ledger row with no execution authorized, and re-checking here would be
/// the exact re-implementation the ticket's Notes warn against. `UnknownGate`
/// carries no gate id to resolve against and converts to `None` — there is
/// nothing for `resolve_verdict` to act on.
#[must_use]
fn approve_and_run_verdict_for(
    verdict: &notify::telegram::ResponseVerdict,
    who: &str,
) -> Option<engine_core::workflows::approve_and_run::ApproveAndRunVerdict> {
    use notify::telegram::ResponseVerdict;
    match verdict {
        ResponseVerdict::Accepted {
            gate_id,
            option_key,
            digest,
            decided_at,
        }
        | ResponseVerdict::StaleDigest {
            gate_id,
            option_key,
            digest,
            decided_at,
        } => Some(
            engine_core::workflows::approve_and_run::ApproveAndRunVerdict {
                gate_id: gate_id.clone(),
                presented_digest: digest.clone(),
                option_key: option_key.clone(),
                who: who.to_string(),
                decided_at: *decided_at,
            },
        ),
        ResponseVerdict::UnknownGate => None,
    }
}

fn approval_ledger_default_path(
    xdg_state_home: Option<String>,
    home: Option<String>,
) -> std::path::PathBuf {
    const FILENAME: &str = "approval-ledger.jsonl";
    if let Some(xdg) = xdg_state_home {
        std::path::PathBuf::from(xdg).join("bastion").join(FILENAME)
    } else if let Some(home) = home {
        std::path::PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("bastion")
            .join(FILENAME)
    } else {
        std::path::PathBuf::from(FILENAME)
    }
}

/// Resolve the `harness.json` path `engine_serve::schedule::spawn_schedule_loop`
/// reads its `schedule` block from (`ticket-spawn-schedule-loop` task 1).
///
/// Reads `BASTION_ENGINE_HARNESS_PATH` only — there is **no** fallback and no
/// derivation from `ENGINE_BRAIN_ROOT`. Both were considered and rejected
/// (operator decision, 2026-08-18; see this spec's `tasks.md` `## Notes`):
/// deriving the path from `ENGINE_BRAIN_ROOT` would bake engine-rs's repo
/// layout into bastion, and that var is unset on the deployed Mac Mini today
/// (engine-rs carryover `en7d-brain-root-not-set-in-deployment`) — so tying
/// resolution to it would silently keep the loop unspawned in production for
/// a reason invisible from this repo.
///
/// `None` on: the var unset, empty, or pointing at a path that does not
/// exist / is not readable. Callers must treat `None` as the **ordinary**
/// case (today's real state, `entries: []`) and log at info, not error —
/// only a caller that reads the file and hits a parse error (a distinct,
/// later outcome inside `spawn_schedule_loop` itself) should log loudly.
/// This function does no file *parsing* — only a variable read and a
/// filesystem stat — so it stays exhaustively unit-testable without a real
/// schedule config on disk (CLAUDE.md rule 6).
fn resolve_engine_harness_path() -> Option<std::path::PathBuf> {
    let raw = std::env::var("BASTION_ENGINE_HARNESS_PATH").ok()?;
    if raw.is_empty() {
        return None;
    }
    let path = std::path::PathBuf::from(raw);
    if path.is_file() { Some(path) } else { None }
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
    // BA.18.A review fix: shared holder for the always-on BlockedEdgePoller's
    // tmux sweep, read by the hub's `sessions` topic poll instead of it
    // running an independent sweep — see `serve::poll::SharedSessionsSweep`.
    let shared_sessions: crate::serve::poll::SharedSessionsSweep =
        std::sync::Arc::new(std::sync::Mutex::new(None));

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

                    // ticket-orphan-reconcile-wiring task 1: sweep
                    // crash-stranded runs once at boot, before the HTTP
                    // listener binds. `pool.clone()` is required because
                    // `spawn_durable_writer(Some(pool))` below consumes the
                    // pool by value; `PgPool` is a cheap `Arc` clone.
                    let sweep = engine_serve::orphan::reconcile_orphans(
                        engine_serve::orphan::orphan_lister_live(pool.clone()).as_ref(),
                        &engine_core::operator::orphan::OrphanPolicy::default(),
                        chrono::Utc::now(),
                    )
                    .await;
                    log_orphan_sweep(&classify_orphan_sweep(sweep));

                    let state = EngineAppState {
                        dispatcher: Arc::new(build_engine_dispatcher()),
                        live: live_store.clone(),
                        durable: spawn_durable_writer(Some(pool)),
                        runs: RunRegistry::new(),
                        api_key: engine_api_key,
                    };
                    let engine_data = web::Data::new(state);

                    // ticket-spawn-schedule-loop task 2: spawn the schedule
                    // tick loop from the SAME `AppState` instance actix
                    // serves from — `web::Data<T>` is an `Arc<T>` newtype,
                    // so `.clone().into_inner()` yields the shared `Arc`
                    // rather than constructing a second `AppState`. A
                    // second instance would make a scheduled fire dispatch
                    // through a different `Dispatcher`/`RunRegistry` and
                    // record into a different `LiveStateStore`, producing
                    // runs invisible to `/api/runs`. `resolve_engine_harness_path()`
                    // returns `None` (ordinary, not an error) whenever
                    // `BASTION_ENGINE_HARNESS_PATH` is unset/unreadable —
                    // today's deployed Mac Mini state — in which case no
                    // loop is spawned, mirroring `Ok(None)` below.
                    let schedule_handle = match resolve_engine_harness_path() {
                        Some(harness_path) => engine_serve::schedule::spawn_schedule_loop(
                            &harness_path,
                            engine_data.clone().into_inner(),
                        ),
                        None => {
                            tracing::info!(
                                target: "bastion::serve",
                                "schedule loop not spawned — BASTION_ENGINE_HARNESS_PATH unset or unreadable"
                            );
                            Ok(None)
                        }
                    };
                    // Bound to this async fn's stack frame, which lives for
                    // the lifetime of the server future below (`.run().await`)
                    // — never dropped early. `ScheduleLoopHandle` wraps a
                    // `tokio::JoinHandle`, which *detaches* rather than
                    // aborts on drop, so an early drop would still run the
                    // loop but leave it unabortable and untestable.
                    let _schedule_handle = match schedule_handle {
                        Ok(Some(handle)) => {
                            // `ScheduleLoopHandle`'s public API (schedule.rs,
                            // read-only/other repo) exposes only `abort()` —
                            // no entry-count accessor — so the count named
                            // here is left to the loop's own internal
                            // `println!("schedule loop: starting with {n}
                            // entries")` rather than duplicated registry-load
                            // logic at this call site.
                            tracing::info!(
                                target: "bastion::serve",
                                "schedule loop spawned"
                            );
                            Some(handle)
                        }
                        Ok(None) => {
                            tracing::info!(
                                target: "bastion::serve",
                                "schedule loop not spawned — no entries configured"
                            );
                            None
                        }
                        Err(e) => {
                            tracing::error!(
                                target: "bastion::serve",
                                error = %e,
                                "schedule loop not spawned — failed to load schedule config"
                            );
                            eprintln!("bastion serve: schedule loop not spawned — {e}");
                            None
                        }
                    };

                    (Some(engine_data), (true, None))
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

    // Resolve the blocked-edge poller's sink path *before* constructing the
    // hub: whether the poller can start at all determines whether the hub
    // should be wired onto `shared_sessions`. Wiring it unconditionally (the
    // pre-fix behavior) left the hub holding `Some(shared_sessions)` even
    // when the poller below never started (no `XDG_STATE_HOME`/`HOME`) — the
    // Mutex it points at then never gets populated, so the `sessions` WS
    // topic's "no sweep yet: skip this cycle" branch fires on every tick,
    // forever, and the topic never pushes a frame. The fallback sweep that
    // exists for exactly this case (`server.rs`'s `Topic::Sessions` handler)
    // only runs when `shared_sessions` is `None` on the hub, which requires
    // not calling `.with_shared_sessions` at all rather than calling it with
    // an empty holder.
    let sink_path = blocked_edge::default_sink_path(
        std::env::var("XDG_STATE_HOME").ok(),
        std::env::var("HOME").ok(),
    );

    // Start the hub actor once (process-singleton within this actix System).
    // All per-connection WsConn actors hold an Addr<Hub> clone.
    let mut hub_builder = Hub::new(
        poll_secs,
        registry.clone(),
        live_store.clone(),
        stream_available,
    );
    if sink_path.is_some() {
        hub_builder = hub_builder.with_shared_sessions(shared_sessions.clone());
    }
    let hub = hub_builder.start();

    // ── Blocked rising-edge poller (BA.18.A task 3) ─────────────────────────
    //
    // Always-on: spawned once at boot, independent of the hub above and of
    // any WebSocket subscription, so a session blocking with zero clients
    // ever connected still produces a durable sink record (Acceptance
    // Criteria). `addr` (this process's own bind address) doubles as its
    // host/instance identity in the sink — stable for the life of this
    // `bastion serve` instance and enough to distinguish it from any other
    // instance writing to the same path, with no new dependency.
    // ── Session-QA bridge (BA.20.C task 6) ───────────────────────────────────
    //
    // Gated on CodeSessionsBot config (task 2) being present. Absent is the
    // expected state today — the bot does not exist yet — and must boot
    // byte-identically to before this block: the poller below is wired
    // exactly as it was pre-BA.20.C and no bridge task is spawned. Computed
    // *before* the poller so its `mpsc::Sender` (if any) can be wired onto
    // `BlockedEdgePoller::with_edge_tx` at construction time, alongside the
    // existing always-on poller — not replacing it. The startup log line
    // states only whether the bridge is enabled, never the token or chat id.
    let edge_tx = match crate::config::load_code_sessions_bot_config() {
        Ok(Some(qa_config)) => {
            tracing::info!(
                target: "bastion::serve",
                "session-QA bridge enabled (CodeSessionsBot configured)"
            );
            let (tx, rx) = tokio::sync::mpsc::channel(32);
            let bridge = std::sync::Arc::new(session_qa::SessionQaBridge::new(qa_config));
            let inbound_bridge = std::sync::Arc::clone(&bridge);
            actix_web::rt::spawn(async move {
                inbound_bridge.run_inbound(rx).await;
            });
            let outbound_bridge = std::sync::Arc::clone(&bridge);
            actix_web::rt::spawn(async move {
                outbound_bridge.run_outbound().await;
            });
            Some(tx)
        }
        Ok(None) => {
            tracing::info!(
                target: "bastion::serve",
                "session-QA bridge disabled (BASTION_CODESESSIONS_BOT_TOKEN / BASTION_CODESESSIONS_CHAT_ID unset)"
            );
            None
        }
        Err(e) => {
            tracing::warn!(
                target: "bastion::serve",
                error = %e,
                "session-QA bridge disabled — invalid CodeSessionsBot config"
            );
            None
        }
    };

    match sink_path {
        Some(sink_path) => {
            let sink = blocked_edge::BlockedEdgeSink::new(sink_path);
            // `.with_hub(hub.clone())` (task 4) makes the hub a *consumer* of
            // this poller's edge decision — the poller remains the sole
            // owner and keeps writing the durable sink record regardless of
            // whether anyone is subscribed; the hub just also gets told so
            // it can fan `event{needs_input}` out to current subscribers.
            let mut poller = blocked_edge::BlockedEdgePoller::new(sink, addr.clone())
                .with_hub(hub.clone())
                .with_shared_sessions(shared_sessions.clone());
            // `.with_edge_tx(...)` (task 6) makes the session-QA bridge a
            // *second* consumer of this poller's edge decision, mirroring
            // `with_hub`'s additive shape exactly — `None` when the bridge
            // is disabled, which is byte-identical to pre-BA.20.C wiring.
            if let Some(tx) = edge_tx {
                poller = poller.with_edge_tx(tx);
            }
            actix_web::rt::spawn(poller.run(poll_secs));
        }
        None => {
            tracing::warn!(
                target: "bastion::serve",
                "blocked-edge poller not started — neither XDG_STATE_HOME nor HOME is set"
            );
        }
    }

    // ── Pending-payload registry (ticket-notify-send-trigger tasks 1-3) ─────
    //
    // Constructed once and shared (`Arc`) between the poll loop's
    // `PendingLookup` below and the `/api/notify/test` route's app data, so
    // a response can be resolved against a payload this same process sent.
    let pending_payloads = std::sync::Arc::new(notify::PendingPayloads::new());

    // ── ApproveAndRunSeams (ticket-approve-and-run-seams task 2) ────────────
    //
    // `engine_core::workflows::approve_and_run::ApproveAndRunSeams` is
    // `engine-rs:EN.8.D`'s pair of seams this process is meant to drive:
    // `lookup_pending` (this process's `PendingLookup` source for a real
    // engine-queued `APPROVE_AND_RUN` gate) and `resolve_verdict` (records
    // the ledger row and, on a matched-digest `Approved` verdict, executes).
    // Nothing in `engine-rs` constructs one in production yet — the only
    // existing construction sites are its own crate's tests — so this is the
    // first production instance. It owns its own queue/records state (a
    // fresh, empty `OperatorQueue` at boot); nothing yet drains an engine
    // run's pending-harvest records into it (that plumbing is a separate,
    // not-yet-written block), so `lookup_pending` legitimately resolves
    // `None` for every gate_id until a drain call exists somewhere in this
    // process. The live `FileApprovalLedger` and the live `HttpPost`
    // (`engine_core::nodes::http_post::http_post_live`) are wired here — no
    // new/invented transport, per the ticket's Notes.
    //
    // Cross-process caveat (ticket Notes): the Mini runs two `bastion`
    // processes — console on `:8080` and engine on `:8090`, and only the
    // engine process's plist sets `DATABASE_URL` / `BASTION_ENGINE_API_KEY`
    // / the Telegram token. `run_server` is the single entry point for both,
    // so the `Arc` shared below between these seams and the poll loop is
    // always in-process by construction: whichever process this `run()` call
    // is executing in, the seams instance and the poll loop that resolves
    // taps against it live in that same process. There is nothing further
    // to confirm here — the two only diverge if a future change moves the
    // poll loop or the seams construction into a different binary/process
    // than this function.
    let approval_ledger_path = approval_ledger_default_path(
        std::env::var("XDG_STATE_HOME").ok(),
        std::env::var("HOME").ok(),
    );
    let approve_and_run_seams = std::sync::Arc::new(
        engine_core::workflows::approve_and_run::ApproveAndRunSeams::new(
            std::sync::Arc::new(std::sync::Mutex::new(
                engine_core::operator::queue::OperatorQueue::new(
                    engine_core::operator::queue::OperatorQueuePolicy::default(),
                ),
            )),
            std::sync::Arc::new(engine_core::operator::ledger::FileApprovalLedger::new(
                approval_ledger_path,
            )),
            engine_core::nodes::http_post::http_post_live(),
            engine_core::operator::OperatorPayloadLimits::default(),
            engine_core::workflows::approve_and_run::ApproveAndRunPolicy::default(),
        ),
    );

    // ── Operator-notification transport (BA.18.B task 5) ────────────────────
    //
    // Constructed only when both `BASTION_TELEGRAM_BOT_TOKEN` and
    // `BASTION_TELEGRAM_CHAT_ID` resolve. Unconfigured is the default and is
    // an `info!` line at boot, never a warning and never a failure — a fresh
    // `bastion serve` with neither var set boots byte-identically to before
    // this task. No route is registered for this anywhere (inbound is
    // `getUpdates` long-polling only, spawned as a background task — see
    // `NotifyPollLoop::run` — never a listening socket).
    //
    // The pending-gate lookup composes two sources (`ticket-approve-and-run-
    // seams` task 2): `approve_and_run_seams.lookup_pending` — the real
    // source for a gate an engine run queued via `ApproveAndRunSeams::drain`
    // — tried first, falling back to `pending_payloads` — payloads this
    // process has sent via `POST /api/notify/test`
    // (`ticket-notify-send-trigger` tasks 1-3). Composed rather than
    // replaced: the test route and its registry are how the
    // `operator-telegram-live-smoke` session is run, and silently breaking
    // that route is a failed task, so both must keep resolving. The two id
    // spaces cannot collide by construction — `gate_id_for` (engine side)
    // and the test route's per-request uuid (`build_test_payload`) draw
    // from disjoint generators — so trying the engine source first can never
    // shadow a real test-route gate. On `Accepted` the `pending_payloads`
    // entry is removed so a replayed tap of the same test-route button
    // resolves to `UnknownGate`, not a second `Accepted`; the engine-side
    // queue's own equivalent eviction is `ApproveAndRunSeams::resolve_verdict`'s
    // job (task 3), not this lookup's.
    match crate::config::load_telegram_config() {
        Ok(Some(telegram_config)) => {
            tracing::info!(
                target: "bastion::serve",
                "operator notification transport configured (Telegram)"
            );
            // `who` for the ledger row the sink below writes (task 3): the
            // Telegram bot is configured against a single chat id, not an
            // operator identity, so the ledger attributes decisions to that
            // chat id rather than inventing one. An honest "which channel
            // approved this" beats a fabricated "who" in an audit ledger —
            // see the ticket's Notes.
            let who = telegram_config.chat_id.clone();
            let transport: std::sync::Arc<dyn notify::OperatorTransport> =
                std::sync::Arc::new(notify::telegram::TelegramTransport::new(telegram_config));
            let verdict_registry = std::sync::Arc::clone(&pending_payloads);
            let verdict_seams = std::sync::Arc::clone(&approve_and_run_seams);
            let lookup_seams = std::sync::Arc::clone(&approve_and_run_seams);
            let lookup_test_registry = std::sync::Arc::clone(&pending_payloads);
            let pending_lookup: notify::PendingLookup = Box::new(move |gate_id: &str| {
                resolve_pending_lookup(&lookup_seams, &lookup_test_registry, gate_id)
            });
            let poll_loop = notify::NotifyPollLoop::new(
                transport,
                pending_lookup,
                Box::new(move |verdict| {
                    // Log the verdict arm and gate_id only — never the
                    // payload body, since a rendered summary may quote
                    // arbitrary operator-supplied content.
                    match &verdict {
                        notify::telegram::ResponseVerdict::Accepted { gate_id, .. } => {
                            verdict_registry.remove(gate_id);
                            tracing::info!(
                                target: "bastion::serve",
                                verdict = "accepted",
                                gate_id = %gate_id,
                                "operator response resolved"
                            );
                        }
                        notify::telegram::ResponseVerdict::StaleDigest { gate_id, .. } => {
                            tracing::info!(
                                target: "bastion::serve",
                                verdict = "stale_digest",
                                gate_id = %gate_id,
                                "operator response resolved"
                            );
                        }
                        notify::telegram::ResponseVerdict::UnknownGate => {
                            tracing::info!(
                                target: "bastion::serve",
                                verdict = "unknown_gate",
                                "operator response resolved"
                            );
                        }
                    }

                    // ── Act on the decision (ticket-approve-and-run-seams task 3) ──
                    //
                    // `ApproveAndRunSeams::resolve_verdict` is `async` (a
                    // matched `Approved` verdict performs a `POST`), but this
                    // sink is a sync `Box<dyn Fn(..) + Send + Sync>` invoked
                    // inline from `NotifyPollLoop::tick`, which itself runs
                    // under `actix_web::rt::spawn` — i.e. `spawn_local` on
                    // the single-threaded `LocalSet` that also drives this
                    // process's HTTP and WS surface. Blocking here (e.g. a
                    // naive `block_on`) would stall that worker, and every
                    // co-resident session's requests with it, for the
                    // duration of the ledger write and any POST. Instead the
                    // resolution itself is spawned onto the same local set —
                    // `actix_web::rt::spawn` again, not a new thread or
                    // executor — so this sink returns immediately and the
                    // poll loop's next `tick` is never blocked behind it.
                    if let Some(engine_verdict) = approve_and_run_verdict_for(&verdict, &who) {
                        let seams = std::sync::Arc::clone(&verdict_seams);
                        actix_web::rt::spawn(async move {
                            match seams.resolve_verdict(engine_verdict).await {
                                Ok(resolution) => {
                                    tracing::info!(
                                        target: "bastion::serve",
                                        executed = resolution.executed.is_some(),
                                        "approve-and-run verdict resolved"
                                    );
                                }
                                Err(
                                    engine_core::workflows::approve_and_run::ApproveAndRunSeamError::UnknownGate(gate_id),
                                ) => {
                                    // Expected whenever the resolved gate came
                                    // from `POST /api/notify/test` rather than
                                    // a real engine-drained item — that gate
                                    // never existed on the engine's queue, so
                                    // this is not a regression. Logged at the
                                    // same level as `NotifyPollLoop`'s own
                                    // `unknown_gate` arm above.
                                    tracing::info!(
                                        target: "bastion::serve",
                                        gate_id = %gate_id,
                                        "approve-and-run verdict: unknown gate"
                                    );
                                }
                                Err(
                                    engine_core::workflows::approve_and_run::ApproveAndRunSeamError::UnknownOption(err),
                                ) => {
                                    tracing::warn!(
                                        target: "bastion::serve",
                                        error = %err,
                                        "approve-and-run verdict: unrecognized option key"
                                    );
                                }
                                Err(
                                    engine_core::workflows::approve_and_run::ApproveAndRunSeamError::Execution(err),
                                ) => {
                                    // Must stay visible: an authorized
                                    // execution that failed to POST is the one
                                    // failure mode this sink must never
                                    // swallow.
                                    tracing::error!(
                                        target: "bastion::serve",
                                        error = %err,
                                        "approve-and-run execution failed"
                                    );
                                }
                            }
                        });
                    }
                }),
            );
            actix_web::rt::spawn(poll_loop.run());
        }
        Ok(None) => {
            tracing::info!(
                target: "bastion::serve",
                "operator notification transport not configured (BASTION_TELEGRAM_BOT_TOKEN / BASTION_TELEGRAM_CHAT_ID unset)"
            );
        }
        Err(e) => {
            tracing::warn!(
                target: "bastion::serve",
                error = %e,
                "operator notification transport not configured — invalid config"
            );
        }
    }

    let registry = web::Data::new(registry);

    let live_data = web::Data::new(live_store);

    // Convert to `web::Data` now that the poll loop (if spawned above) holds
    // its own `Arc` clone — this is the same registry instance either way.
    let pending_payloads = web::Data::from(pending_payloads);

    HttpServer::new(move || {
        let hub_data = web::Data::new(hub.clone());
        let registry_data = registry.clone();
        let engine_data = engine_data.clone();
        let live_data = live_data.clone();
        let pending_payloads = pending_payloads.clone();

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
            // ── Fleet-scoped lane-segment availability route (BA.19.C) ───────
            // /lanes — GET only (one aggregate per lane segment, pass-through
            // over mev::lanes_brain)
            .service(web::resource("/lanes").route(web::get().to(handlers::lanes::get_lanes)))
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
            // ── Operator-notification test-send route (ticket-notify-send-trigger) ──
            // /notify/test — POST only. Resolves the Telegram transport from env at
            // call time (503 if unconfigured); see `handlers::notify` module docs.
            .service(
                web::resource("/notify/test").route(web::post().to(handlers::notify::test_send)),
            )
            .app_data(pending_payloads.clone())
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
        // `Bearer` middleware — they carry their own `X-API-Key` gate. Task 1
        // (`BA.ticket.engine-surface-auth`) found that gate was only wired
        // inline on 9 of the 11 registered routes (`engine_serve::http::
        // check_api_key`, called as the first line of each handler);
        // `list_workflows` and `workflow_graph` took no `HttpRequest` at all
        // and skipped it entirely, answering 200 to a bogus/absent key. Task
        // 3 closes that by wrapping the *whole* mount in one
        // `ApiKeyAuthMiddleware` — mirroring `BearerAuthMiddleware`'s shape
        // (`auth.rs`) — so every route in the table is gated the same way
        // regardless of whether its handler also calls `check_api_key`
        // itself (redundant on the 9, now load-bearing on the 2).
        //
        // `GET /health` registered by `engine_serve::http::configure` stays
        // shadowed by bastion's own public `/health` above
        // (first-registration-wins, see the collision note there) — wrapping
        // the engine mount here does not touch that liveness contract.
        if let Some(engine_data) = engine_data {
            let engine_api_key = engine_data.api_key.clone();
            app = app.app_data(engine_data).service(
                web::scope("")
                    .wrap(ApiKeyAuthMiddleware::new(engine_api_key))
                    .configure(engine_serve::http::configure),
            );
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

    // ── classify_orphan_sweep tests (ticket-orphan-reconcile-wiring task 2) ──
    //
    // Pure, sync, no I/O — kept in this module (not `mod tests` below) for
    // the same shadowing reason every other test here is: `mod tests` imports
    // `actix_web::test`, which shadows the built-in `#[test]` attribute.

    #[test]
    fn classify_orphan_sweep_ok_with_reconciled_ids_is_swept() {
        let id1 = uuid::Uuid::new_v4();
        let id2 = uuid::Uuid::new_v4();
        let outcome = classify_orphan_sweep(Ok(ReconcileSummary {
            scanned: 3,
            reconciled: vec![id1, id2],
        }));
        assert_eq!(
            outcome,
            OrphanSweepOutcome::Swept {
                scanned: 3,
                reconciled: vec![id1, id2],
            }
        );
    }

    #[test]
    fn classify_orphan_sweep_ok_with_nonzero_scanned_but_empty_reconciled_is_swept_nothing() {
        // Pins the boundary the log lines differ on: a non-zero `scanned`
        // with nothing reconciled is `SweptNothing`, NOT `Swept` with an
        // empty vec — those two states must never collapse into each other.
        let outcome = classify_orphan_sweep(Ok(ReconcileSummary {
            scanned: 5,
            reconciled: vec![],
        }));
        assert_eq!(outcome, OrphanSweepOutcome::SweptNothing { scanned: 5 });
    }

    #[test]
    fn classify_orphan_sweep_default_summary_is_swept_nothing_zero() {
        let outcome = classify_orphan_sweep(Ok(ReconcileSummary::default()));
        assert_eq!(outcome, OrphanSweepOutcome::SweptNothing { scanned: 0 });
    }

    #[test]
    fn classify_orphan_sweep_err_is_failed_preserving_message_verbatim() {
        let outcome = classify_orphan_sweep(Err("connection refused".to_string()));
        assert_eq!(
            outcome,
            OrphanSweepOutcome::Failed("connection refused".to_string())
        );
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
            // ── Fleet-scoped lane-segment availability route (BA.19.C) ───────
            // /lanes — GET only (one aggregate per lane segment, pass-through
            // over mev::lanes_brain)
            .service(web::resource("/lanes").route(web::get().to(handlers::lanes::get_lanes)))
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
            // ── Operator-notification test-send route (ticket-notify-send-trigger) ──
            .service(
                web::resource("/notify/test").route(web::post().to(handlers::notify::test_send)),
            )
            .app_data(web::Data::new(notify::PendingPayloads::new()))
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

    /// Structural regression guard for `BA.18.B` non-negotiable constraint
    /// 2: inbound Telegram delivery is `getUpdates` long-polling only —
    /// there must never be a webhook route anywhere in this app, since a
    /// webhook needs a public listener into the Mini
    /// (`brain:HQ.ticket.tailscale-bind-and-token-rotation` closes exactly
    /// that). This app currently registers no webhook route at all, so
    /// every plausible webhook path below must 404 — a future refactor
    /// that registers one (under any of these guesses, protected or not)
    /// fails this test rather than shipping silently.
    #[actix_web::test]
    async fn no_telegram_webhook_route_is_ever_registered() {
        let app = test::init_service(build_app(FileConfig::default())).await;

        for path in [
            "/telegram/webhook",
            "/webhook",
            "/api/telegram/webhook",
            "/api/webhook",
            "/notify/telegram/webhook",
            "/bot/webhook",
            "/hooks/telegram",
        ] {
            for req in [
                test::TestRequest::post()
                    .uri(path)
                    .insert_header(("Authorization", format!("Bearer {TEST_TOKEN}")))
                    .to_request(),
                test::TestRequest::get()
                    .uri(path)
                    .insert_header(("Authorization", format!("Bearer {TEST_TOKEN}")))
                    .to_request(),
            ] {
                // Authenticated on every candidate path (including the /api-scoped
                // ones) so a 401 from `BearerAuthMiddleware` can never be mistaken
                // for "no such route" — an unauthenticated probe can't distinguish
                // "route doesn't exist" from "route exists but requires auth",
                // which would make this test pass even if a webhook route were
                // added under the protected scope.
                let resp = test::call_service(&app, req).await;
                assert_eq!(
                    resp.status(),
                    actix_web::http::StatusCode::NOT_FOUND,
                    "path {path} responded {} — no webhook route may ever be registered \
                     (BA.18.B constraint 2: inbound is getUpdates long-polling only)",
                    resp.status()
                );
            }
        }
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

    // ── /api/notify/test — route registration (ticket-notify-send-trigger) ──

    #[actix_web::test]
    async fn notify_test_send_rejects_missing_token_with_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::post()
            .uri("/api/notify/test")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 401);
    }

    #[actix_web::test]
    async fn notify_test_send_is_absent_at_the_app_root() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::post()
            .uri("/notify/test")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            404,
            "POST /notify/test must not exist at the app root — only /api/notify/test"
        );
    }

    #[actix_web::test]
    async fn notify_test_send_unconfigured_transport_returns_503_c005() {
        let env_lock = lock_env();
        // `load_telegram_config` calls `dotenvy::dotenv()`, which would
        // repopulate the vars from a checked-out `.env` the moment they are
        // removed from the process env — see `get_costs_missing_database_url_
        // returns_503_c005`'s comment for the same trap with `DATABASE_URL`.
        let _dotenv_shadow = DotenvShadow::new(&env_lock, "notify_test_send_c005");
        let _bot_token = EnvVarGuard::unset(&env_lock, "BASTION_TELEGRAM_BOT_TOKEN");
        let _chat_id = EnvVarGuard::unset(&env_lock, "BASTION_TELEGRAM_CHAT_ID");

        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::post()
            .uri("/api/notify/test")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;

        let status = resp.status();
        let body: serde_json::Value = test::read_body_json(resp).await;

        assert_eq!(
            status, 503,
            "POST /api/notify/test with no Telegram config must return 503; got {status}"
        );
        assert_eq!(body["code"], "C005");
        let message = body["message"].as_str().expect("message is a string");
        assert!(message.contains("BASTION_TELEGRAM_BOT_TOKEN"));
        assert!(message.contains("BASTION_TELEGRAM_CHAT_ID"));
    }

    #[actix_web::test]
    async fn notify_test_send_incomplete_config_names_only_the_missing_var() {
        let env_lock = lock_env();
        let _dotenv_shadow = DotenvShadow::new(&env_lock, "notify_test_send_incomplete");
        let _bot_token =
            EnvVarGuard::set(&env_lock, "BASTION_TELEGRAM_BOT_TOKEN", "fake-token-value");
        let _chat_id = EnvVarGuard::unset(&env_lock, "BASTION_TELEGRAM_CHAT_ID");

        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::post()
            .uri("/api/notify/test")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), 503);
        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(body["code"], "C005");
        let message = body["message"].as_str().expect("message is a string");
        assert!(message.contains("BASTION_TELEGRAM_CHAT_ID"));
        // The var that *was* set must never appear in the error body — its
        // value never should either way, but confirm the message names only
        // the missing side.
        assert!(!message.contains("fake-token-value"));
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

    // ── Engine mount X-API-Key gate (task 3, BA.ticket.engine-surface-auth) ────
    //
    // Mirrors production's mount exactly: bastion's own `/health` registered
    // first (so it stays reachable regardless of the engine gate — the
    // `/health` collision, see the comment above `run`'s `App::new()`), then
    // the whole `engine_serve::http::configure` table wrapped in one
    // `ApiKeyAuthMiddleware` scope. Covers every route task 1 enumerated,
    // including the two (`GET /workflows`, `GET /workflows/{type}/graph`)
    // that were the actual gap — the other nine already carried an inline
    // `check_api_key` call, so this is a regression guard for those plus the
    // fix for these two.

    const ENGINE_TEST_KEY: &str = "engine-test-key-abc";

    /// Build a standalone test service mounting bastion's own `/health` plus
    /// the engine route table, gated by [`ApiKeyAuthMiddleware`] exactly as
    /// `run()` wires it. DB-free: `spawn_durable_writer(None)` self-skips
    /// Postgres writes, matching how the engine already degrades gracefully
    /// without `DATABASE_URL`.
    async fn engine_test_service(
        api_key: &str,
    ) -> impl actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    > {
        let state = EngineAppState {
            dispatcher: Arc::new(build_engine_dispatcher()),
            live: LiveStateStore::new(),
            durable: spawn_durable_writer(None),
            runs: RunRegistry::new(),
            api_key: api_key.to_string(),
        };
        let engine_data = web::Data::new(state);

        let app = App::new()
            .service(web::resource("/health").route(web::get().to(health)))
            .app_data(engine_data)
            .service(
                web::scope("")
                    .wrap(ApiKeyAuthMiddleware::new(api_key))
                    .configure(engine_serve::http::configure),
            );

        test::init_service(app).await
    }

    /// One (method, path) pair per route registered by
    /// `engine_serve::http::configure`, mirroring task 1's enumeration
    /// exactly. `POST /events/{run_id}/pause` and `.../resume` and `GET
    /// /events/{event_id}` etc. use a fresh random uuid — the handler must
    /// still reach its own not-found branch (proving the request got past
    /// auth) for the "correct key" case, and must never get that far for the
    /// "no/bogus key" cases.
    fn engine_route_cases() -> Vec<(&'static str, String)> {
        let run_id = uuid::Uuid::new_v4();
        vec![
            ("GET", "/workflows".to_string()),
            ("GET", "/workflows/SDLC_FLOW/graph".to_string()),
            ("POST", "/events/".to_string()),
            ("GET", "/events/suspended".to_string()),
            ("GET", format!("/events/{run_id}")),
            ("POST", format!("/events/{run_id}/abort")),
            ("POST", format!("/events/{run_id}/pause")),
            ("POST", format!("/events/{run_id}/resume")),
            ("GET", format!("/events/{run_id}/stream")),
            ("POST", "/webhooks/email/inbound".to_string()),
            ("POST", "/webhooks/email/events".to_string()),
        ]
    }

    /// `POST /events/` needs a JSON body to clear the `web::Json<TriggerBody>`
    /// extractor and actually reach the handler's own auth check — every
    /// other route in the table takes no body. Mirrors task 1's amendment
    /// log note about the malformed-probe artifact.
    fn body_for(path: &str) -> Option<serde_json::Value> {
        if path == "/events/" {
            Some(serde_json::json!({ "workflow_type": "UNKNOWN_TYPE", "data": {} }))
        } else {
            None
        }
    }

    fn request_for(method: &str, path: &str) -> test::TestRequest {
        let req = match method {
            "GET" => test::TestRequest::get(),
            "POST" => test::TestRequest::post(),
            other => panic!("unsupported method in engine_route_cases: {other}"),
        };
        let req = req.uri(path);
        match body_for(path) {
            Some(body) => req.set_json(body),
            None => req,
        }
    }

    #[actix_web::test]
    async fn engine_routes_reject_missing_api_key() {
        let service = engine_test_service(ENGINE_TEST_KEY).await;
        for (method, path) in engine_route_cases() {
            let resp = test::call_service(&service, request_for(method, &path).to_request()).await;
            assert_eq!(
                resp.status(),
                401,
                "{method} {path} without X-API-Key must return 401; got {}",
                resp.status()
            );
        }
    }

    #[actix_web::test]
    async fn engine_routes_reject_bogus_api_key() {
        let service = engine_test_service(ENGINE_TEST_KEY).await;
        for (method, path) in engine_route_cases() {
            let req = request_for(method, &path)
                .insert_header(("X-API-Key", "totally-bogus-value"))
                .to_request();
            let resp = test::call_service(&service, req).await;
            assert_eq!(
                resp.status(),
                401,
                "{method} {path} with a bogus X-API-Key must return 401; got {}",
                resp.status()
            );
        }
    }

    #[actix_web::test]
    async fn engine_routes_reject_empty_api_key() {
        let service = engine_test_service(ENGINE_TEST_KEY).await;
        for (method, path) in engine_route_cases() {
            let req = request_for(method, &path)
                .insert_header(("X-API-Key", ""))
                .to_request();
            let resp = test::call_service(&service, req).await;
            assert_eq!(
                resp.status(),
                401,
                "{method} {path} with an empty X-API-Key must return 401; got {}",
                resp.status()
            );
        }
    }

    #[actix_web::test]
    async fn engine_routes_accept_correct_api_key() {
        let service = engine_test_service(ENGINE_TEST_KEY).await;
        for (method, path) in engine_route_cases() {
            let req = request_for(method, &path)
                .insert_header(("X-API-Key", ENGINE_TEST_KEY))
                .to_request();
            let resp = test::call_service(&service, req).await;
            assert_ne!(
                resp.status(),
                401,
                "{method} {path} with the correct X-API-Key must not return 401; got {}",
                resp.status()
            );
        }
    }

    /// `POST /events/` is the mutating route the acceptance criteria calls
    /// out explicitly — asserted on its own (not just folded into the table
    /// loops above) with all three key states.
    #[actix_web::test]
    async fn post_events_is_gated_explicitly() {
        let service = engine_test_service(ENGINE_TEST_KEY).await;
        let body = serde_json::json!({ "workflow_type": "UNKNOWN_TYPE", "data": {} });

        let no_header = test::TestRequest::post()
            .uri("/events/")
            .set_json(&body)
            .to_request();
        assert_eq!(test::call_service(&service, no_header).await.status(), 401);

        let bogus = test::TestRequest::post()
            .uri("/events/")
            .insert_header(("X-API-Key", "totally-bogus-value"))
            .set_json(&body)
            .to_request();
        assert_eq!(test::call_service(&service, bogus).await.status(), 401);

        let correct = test::TestRequest::post()
            .uri("/events/")
            .insert_header(("X-API-Key", ENGINE_TEST_KEY))
            .set_json(&body)
            .to_request();
        // Reached past auth: dispatch rejects the unknown workflow_type with
        // 422, never 401.
        assert_eq!(test::call_service(&service, correct).await.status(), 422);
    }

    /// `GET /health` (bastion's own, registered before the engine mount)
    /// stays unauthenticated even when the engine is mounted and gated —
    /// the collision ordering documented at `run`'s `App::new()` call must
    /// hold.
    #[actix_web::test]
    async fn health_stays_unauthenticated_with_engine_mounted() {
        let service = engine_test_service(ENGINE_TEST_KEY).await;

        let no_header = test::TestRequest::get().uri("/health").to_request();
        let resp = test::call_service(&service, no_header).await;
        assert_eq!(resp.status(), 200);

        let bogus = test::TestRequest::get()
            .uri("/health")
            .insert_header(("X-API-Key", "totally-bogus-value"))
            .to_request();
        let resp = test::call_service(&service, bogus).await;
        assert_eq!(resp.status(), 200);
    }

    /// Existing `/api/*` bearer routes are unaffected by the engine-mount
    /// change — same `build_app` harness used throughout this module, which
    /// never mounts the engine, still enforces bearer auth exactly as
    /// before.
    #[actix_web::test]
    async fn api_bearer_routes_unchanged_by_engine_gate() {
        let app = test::init_service(build_app(FileConfig::default())).await;

        let no_token = test::TestRequest::get().uri("/api/board").to_request();
        assert_eq!(test::call_service(&app, no_token).await.status(), 401);

        let with_token = test::TestRequest::get()
            .uri("/api/board")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, with_token).await;
        assert_ne!(resp.status(), 401);
    }

    // ── session-QA bridge boot wiring (BA.20.C task 6) ──────────────────────

    /// This block adds **no** new HTTP route — the bridge polls `getUpdates`
    /// itself. Guards against a future edit accidentally wiring a webhook or
    /// a `session-qa`-named endpoint into the route table by asserting a
    /// handful of plausible such paths are all unregistered (404, not 405 —
    /// a matched-but-wrong-method resource would 405, per this module's
    /// `web::resource` convention documented at the top of `run_server`).
    #[actix_web::test]
    async fn session_qa_bridge_adds_no_new_route() {
        let app = test::init_service(build_app(FileConfig::default())).await;

        for path in [
            "/api/session-qa",
            "/api/session-qa/webhook",
            "/api/notify/session-qa",
            "/api/telegram/webhook",
            "/telegram/webhook",
            "/webhook",
        ] {
            let req = test::TestRequest::post()
                .uri(path)
                .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
                .to_request();
            let status = test::call_service(&app, req).await.status();
            assert_eq!(
                status, 404,
                "expected {path} to be unregistered (404), got {status} — this block must \
                 add no new HTTP route"
            );
        }

        // The existing routes this block's spec calls out by name stay
        // exactly as they were (`/sessions/{name}/send`, present in both
        // app factories per the spec's Context Pointers).
        let send_req = test::TestRequest::post()
            .uri("/api/sessions/nonexistent-session/send")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .set_json(serde_json::json!({"text": "hello"}))
            .to_request();
        let send_status = test::call_service(&app, send_req).await.status();
        assert_ne!(
            send_status, 404,
            "/api/sessions/{{name}}/send must remain registered"
        );
    }

    /// A minimal [`tracing_subscriber::fmt::MakeWriter`] that appends every
    /// formatted log line into a shared in-memory buffer, so a test can
    /// assert on `run_server`'s startup log lines without a real log file or
    /// stdout capture race against sibling tests.
    #[derive(Clone)]
    struct SharedLogBuf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for SharedLogBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SharedLogBuf {
        type Writer = SharedLogBuf;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// `run_server`'s setup path, with CodeSessionsBot config absent, spawns
    /// no session-QA bridge and leaves the always-on `BlockedEdgePoller`
    /// wired exactly as it was before this block (task 3's `no_edge_tx_wired_
    /// behaves_identically_to_today` already pins the poller side of that
    /// claim in isolation; this test pins the boot-time *decision* that
    /// reaches it — a real `run_server` call, never given a bridge to spawn).
    ///
    /// Binds `127.0.0.1:0` (OS-assigned ephemeral port, never a fixed one a
    /// sibling test could collide on) and aborts the task shortly after
    /// boot — `run_server` otherwise serves forever, so there is no natural
    /// completion to await.
    #[actix_web::test]
    async fn run_server_with_no_codesessions_config_spawns_no_bridge() {
        let env_lock = lock_env();
        let _dotenv_shadow = DotenvShadow::new(&env_lock, "run_server_no_qa_bridge");
        let _t1 = EnvVarGuard::unset(&env_lock, "BASTION_CODESESSIONS_BOT_TOKEN");
        let _t2 = EnvVarGuard::unset(&env_lock, "BASTION_CODESESSIONS_CHAT_ID");
        // Keep the boot path fast and deterministic — no real DB connect
        // attempt, no real BastionBot poll loop, regardless of this dev
        // machine's actual `.env` contents (already shadowed above, but the
        // process env itself could still carry these from the caller's shell).
        let _db = EnvVarGuard::unset(&env_lock, "DATABASE_URL");
        let _engine_key = EnvVarGuard::unset(&env_lock, "BASTION_ENGINE_API_KEY");
        let _tg_token = EnvVarGuard::unset(&env_lock, "BASTION_TELEGRAM_BOT_TOKEN");
        let _tg_chat = EnvVarGuard::unset(&env_lock, "BASTION_TELEGRAM_CHAT_ID");

        let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let writer = SharedLogBuf(buf.clone());
        let subscriber = tracing_subscriber::fmt()
            .with_writer(writer)
            .with_ansi(false)
            .without_time()
            .with_target(false)
            .finish();
        let _tracing_guard = tracing::subscriber::set_default(subscriber);

        let handle = actix_web::rt::spawn(run_server(
            "127.0.0.1:0".to_string(),
            "boot-test-token".to_string(),
            2,
        ));
        // `run_server` serves forever once bound — give its setup path (which
        // runs before `.bind(..).run().await`) time to reach and log the
        // session-QA gate decision, then cancel the still-running server.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        handle.abort();
        let _ = handle.await;

        drop(_tracing_guard);
        let logs =
            String::from_utf8_lossy(&buf.lock().unwrap_or_else(|e| e.into_inner())).to_string();

        assert!(
            logs.contains("session-QA bridge disabled"),
            "expected the disabled-bridge log line; got logs:\n{logs}"
        );
        assert!(
            !logs.contains("session-QA bridge enabled"),
            "no bridge should have been spawned with CodeSessionsBot config absent; got logs:\n{logs}"
        );
    }

    // ── boot orphan sweep, end to end against a stub OrphanLister
    //    (ticket-orphan-reconcile-wiring task 2) ───────────────────────────
    //
    // Drives `engine_serve::orphan::reconcile_orphans` through
    // `engine_serve::orphan::RecordingOrphanLister` — the no-database test
    // seam the module docs name for exactly this caller — then classifies
    // the result the same way `run_server`'s boot path does. No
    // `DATABASE_URL`, no live Postgres, matching how engine-rs tests this
    // same code (orphan.rs's own `mod tests`). `#[actix_web::test]` because
    // this module already imports `actix_web::test` for the HTTP-handler
    // tests above (see `engine_mount_tests` for the pure, sync cases
    // instead).

    fn orphan_row(
        id: uuid::Uuid,
        updated_at: chrono::DateTime<chrono::Utc>,
    ) -> engine_contract::EventsRow {
        engine_contract::EventsRow {
            id,
            workflow_type: "CONTENT_PIPELINE".to_string(),
            data: serde_json::json!({}),
            task_context: empty_task_context(),
            created_at: updated_at,
            updated_at,
        }
    }

    #[actix_web::test]
    async fn boot_sweep_two_candidates_classifies_as_swept_with_both_ids() {
        let id1 = uuid::Uuid::new_v4();
        let id2 = uuid::Uuid::new_v4();
        let old = chrono::Utc::now() - chrono::Duration::hours(2);
        let lister = engine_serve::orphan::RecordingOrphanLister::new(vec![
            orphan_row(id1, old),
            orphan_row(id2, old),
        ]);

        let result = engine_serve::orphan::reconcile_orphans(
            &lister,
            &engine_core::operator::orphan::OrphanPolicy::default(),
            chrono::Utc::now(),
        )
        .await;
        let outcome = classify_orphan_sweep(result);

        match outcome {
            OrphanSweepOutcome::Swept {
                scanned,
                reconciled,
            } => {
                assert_eq!(scanned, 2);
                assert_eq!(reconciled.len(), 2);
                assert!(reconciled.contains(&id1));
                assert!(reconciled.contains(&id2));
            }
            other => panic!("expected Swept, got {other:?}"),
        }
    }

    #[actix_web::test]
    async fn boot_sweep_zero_candidates_classifies_as_swept_nothing() {
        let lister = engine_serve::orphan::RecordingOrphanLister::new(vec![]);

        let result = engine_serve::orphan::reconcile_orphans(
            &lister,
            &engine_core::operator::orphan::OrphanPolicy::default(),
            chrono::Utc::now(),
        )
        .await;
        let outcome = classify_orphan_sweep(result);

        assert_eq!(outcome, OrphanSweepOutcome::SweptNothing { scanned: 0 });
    }

    #[actix_web::test]
    async fn boot_sweep_lister_error_classifies_as_failed_and_returns_rather_than_panicking() {
        let lister =
            engine_serve::orphan::RecordingOrphanLister::failing_list("connection refused");

        let result = engine_serve::orphan::reconcile_orphans(
            &lister,
            &engine_core::operator::orphan::OrphanPolicy::default(),
            chrono::Utc::now(),
        )
        .await;
        // The call above returning at all (rather than panicking/unwinding)
        // is the criterion that the server still starts; classification
        // below confirms it lands on the right variant too.
        let outcome = classify_orphan_sweep(result);

        match outcome {
            OrphanSweepOutcome::Failed(msg) => assert!(msg.contains("connection refused")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[actix_web::test]
    async fn boot_sweep_reconcile_on_boot_false_yields_swept_nothing_with_no_lister_call() {
        // A lister that errors if it is ever called — `reconcile_on_boot:
        // false` must short-circuit in `reconcile_orphans` before touching
        // the lister at all, so a `list_orphan_candidates` call landing here
        // would flip this test from `SweptNothing` to `Failed`.
        let lister = engine_serve::orphan::RecordingOrphanLister::failing_list(
            "lister must not be called when reconcile_on_boot is false",
        );
        let policy = engine_core::operator::orphan::OrphanPolicy {
            reconcile_on_boot: false,
            ..engine_core::operator::orphan::OrphanPolicy::default()
        };

        let result =
            engine_serve::orphan::reconcile_orphans(&lister, &policy, chrono::Utc::now()).await;
        let outcome = classify_orphan_sweep(result);

        assert_eq!(outcome, OrphanSweepOutcome::SweptNothing { scanned: 0 });
    }

    // ── /api/lanes — route registration (BA.19.C task 4) ────────────────────
    //
    // Reuses `make_temp_board_brain_root` / `registry_with_board_fixture`:
    // both handlers walk the same `find_brain_root` -> `discover_state_files`
    // -> `load_state` shape, and this fixture carries no lane files, so
    // `mev::lanes_brain` resolves successfully to an empty `segments` Vec —
    // the route test only needs to confirm the endpoint answers at all, not
    // exercise the availability computation itself (that's MV.13.C's suite).

    #[actix_web::test]
    async fn get_lanes_rejects_missing_token_with_401() {
        let app = test::init_service(build_app(FileConfig::default())).await;
        let req = test::TestRequest::get().uri("/api/lanes").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            401,
            "GET /api/lanes without a token must return 401 (inherited BearerAuthMiddleware); got {}",
            resp.status()
        );
    }

    #[actix_web::test]
    async fn get_lanes_returns_200_with_valid_token() {
        let dir = make_temp_board_brain_root();
        let registry = registry_with_board_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/lanes")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            200,
            "GET /api/lanes with a valid token must return 200; got {}",
            resp.status()
        );

        let body: serde_json::Value = test::read_body_json(resp).await;
        assert!(
            body["derived_at"].is_string(),
            "derived_at must be present: {body}"
        );
        assert!(
            body["degraded"].is_boolean(),
            "degraded must be present: {body}"
        );
        assert!(
            body["segments"].is_array(),
            "segments must be an array: {body}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[actix_web::test]
    async fn get_lanes_unknown_epic_returns_same_error_shape_as_board() {
        let dir = make_temp_board_brain_root();
        let registry = registry_with_board_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/lanes?epic=no-such-epic-slug")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            404,
            "an unknown ?epic=<slug> on /api/lanes must be a 404, mirroring board::epic_error_response; got {}",
            resp.status()
        );

        let body: serde_json::Value = test::read_body_json(resp).await;
        assert_eq!(
            body["code"], "C005",
            "unknown-epic error must carry the same C005 code /api/board uses: {body}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[actix_web::test]
    async fn get_lanes_blank_epic_param_matches_board_epic_param_missing_behavior() {
        let dir = make_temp_board_brain_root();
        let registry = registry_with_board_fixture(&dir);
        let app = test::init_service(build_app(registry)).await;

        let req = test::TestRequest::get()
            .uri("/api/lanes?epic=")
            .insert_header(("authorization", format!("Bearer {TEST_TOKEN}")))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            404,
            "?epic= present but blank must behave like board.rs::epic_param_missing (an error, not silently ignored); got {}",
            resp.status()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[actix_web::test]
    async fn get_lanes_route_registered_in_build_app_with_live_store_too() {
        // Catches the trap called out in the task: the route must be added
        // to BOTH `App` builders (production `run_server`'s inline scope and
        // this test module's `build_app_with_live_store`, which `build_app`
        // itself wraps), or a route test against only `build_app` could pass
        // against an app whose live-store variant never registered the
        // route. Drives `build_app_with_live_store` directly rather than
        // through `build_app`, so a divergence between the two functions
        // would be caught here.
        let (app, _hub) =
            build_app_with_live_store(FileConfig::default(), LiveStateStore::new(), (false, None));
        let app = test::init_service(app).await;
        let req = test::TestRequest::get().uri("/api/lanes").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            401,
            "GET /api/lanes must be registered in build_app_with_live_store too (401 without a token, not 404); got {}",
            resp.status()
        );
    }
}

// ── approve_and_run_seams_wiring tests (ticket-approve-and-run-seams task 2) ─
//
// Kept in a dedicated module (mirroring `engine_mount_tests` above) rather
// than inside `mod tests`, for the exact reason documented on that module:
// `mod tests` does `use actix_web::{App, test};`, which shadows the built-in
// `#[test]` attribute with actix's async-only test macro — a plain sync
// `#[test] fn` there fails to compile.
#[cfg(test)]
mod approve_and_run_seams_wiring_tests {
    use super::*;
    use engine_core::nodes::harvest_gate::pending_harvest_record;
    use engine_core::nodes::http_post::StubHttpPost;
    use engine_core::operator::ledger::FileApprovalLedger;
    use engine_core::operator::queue::{OperatorQueue, OperatorQueuePolicy};
    use engine_core::operator::{
        OperatorPayload, OperatorPayloadLimits, OperatorResponseOption, validate,
    };
    use engine_core::workflows::approve_and_run::{
        ApproveAndRunPolicy, ApproveAndRunSeams, PendingHarvestRecord,
    };
    use std::sync::{Arc, Mutex};

    // ── approval_ledger_default_path ─────────────────────────────────────

    #[test]
    fn approval_ledger_default_path_prefers_xdg_state_home() {
        let path = approval_ledger_default_path(
            Some("/xdg/state".to_string()),
            Some("/home/u".to_string()),
        );
        assert_eq!(
            path,
            std::path::PathBuf::from("/xdg/state/bastion/approval-ledger.jsonl")
        );
    }

    #[test]
    fn approval_ledger_default_path_falls_back_to_home() {
        let path = approval_ledger_default_path(None, Some("/home/u".to_string()));
        assert_eq!(
            path,
            std::path::PathBuf::from("/home/u/.local/state/bastion/approval-ledger.jsonl")
        );
    }

    #[test]
    fn approval_ledger_default_path_falls_back_to_relative_filename_when_neither_var_set() {
        let path = approval_ledger_default_path(None, None);
        assert_eq!(path, std::path::PathBuf::from("approval-ledger.jsonl"));
    }

    // ── resolve_pending_lookup ────────────────────────────────────────────
    //
    // Exercises the composed `PendingLookup` source in isolation, with no
    // actix and no Telegram config — mirrors `engine_core`'s own
    // `ApproveAndRunSeams` seams_tests fixtures.

    /// A fresh `ApproveAndRunSeams` over an empty queue and a
    /// `FileApprovalLedger` pointed at a throwaway tempdir path — the
    /// ledger is never exercised by `lookup_pending`, but a real
    /// `FileApprovalLedger` is used anyway (rather than an
    /// engine-core-internal `#[cfg(test)]`-only stub, which is not visible
    /// to bastion as a normal downstream dependency) to mirror exactly what
    /// `run()` constructs.
    fn seams() -> ApproveAndRunSeams {
        let dir = tempfile::tempdir().expect("tempdir");
        ApproveAndRunSeams::new(
            Arc::new(Mutex::new(OperatorQueue::new(
                OperatorQueuePolicy::default(),
            ))),
            Arc::new(FileApprovalLedger::new(dir.path().join("ledger.jsonl"))),
            Arc::new(StubHttpPost::succeeding(serde_json::json!({"ok": true}))),
            OperatorPayloadLimits::default(),
            ApproveAndRunPolicy::default(),
        )
    }

    fn harvest_record(artifact_id: &str) -> PendingHarvestRecord {
        let value = pending_harvest_record(
            artifact_id,
            "https://synapse.example/ingest/learning-artifact",
            serde_json::json!({"title": "some artifact"}),
            vec!["docs/foo.md".to_string()],
        );
        PendingHarvestRecord::from_value(&value).expect("record parses")
    }

    #[test]
    fn resolves_a_gate_the_engine_seams_drained() {
        let seams = seams();
        let test_registry = notify::PendingPayloads::new();

        let report = seams.drain(&[harvest_record("artifact-1")], chrono::Utc::now());
        let delivered = report.delivered.expect("one item delivered");

        let found = resolve_pending_lookup(&seams, &test_registry, &delivered.item_id)
            .expect("gate drained into the engine seams should resolve");
        assert_eq!(found.payload().gate_id, delivered.item_id);
    }

    #[test]
    fn a_gate_never_drained_resolves_to_none() {
        let seams = seams();
        let test_registry = notify::PendingPayloads::new();

        assert!(resolve_pending_lookup(&seams, &test_registry, "never-drained").is_none());
    }

    #[test]
    fn a_payload_sent_via_notify_test_still_resolves() {
        let seams = seams();
        let test_registry = notify::PendingPayloads::new();

        let payload = OperatorPayload::new(
            "test-gate-1",
            "a test summary",
            vec![
                OperatorResponseOption::new("approve", "Approve"),
                OperatorResponseOption::new("reject", "Reject"),
            ],
        );
        let validated = validate(payload, &OperatorPayloadLimits::default())
            .expect("fixed 2-option test payload always validates");
        test_registry.insert(validated);

        let found = resolve_pending_lookup(&seams, &test_registry, "test-gate-1")
            .expect("a payload sent via /api/notify/test should still resolve");
        assert_eq!(found.payload().gate_id, "test-gate-1");
    }

    // ── approve_and_run_verdict_for ──────────────────────────────────────
    //
    // Pure conversion, no actix, no engine seams — the sink's async
    // dispatch (`resolve_verdict`) is covered by the hermetic
    // resolve-and-execute tests in `notify/tests.rs` (task 4); this module
    // only proves the widened `ResponseVerdict` arms convert correctly and
    // that `UnknownGate` yields no verdict at all.

    fn ts(secs: i64) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    #[test]
    fn approve_and_run_verdict_for_converts_accepted() {
        let verdict = notify::telegram::ResponseVerdict::Accepted {
            gate_id: "gate-1".to_string(),
            option_key: "approve".to_string(),
            digest: "ab12".to_string(),
            decided_at: ts(1),
        };

        let built = approve_and_run_verdict_for(&verdict, "chat-42")
            .expect("Accepted converts to a verdict");
        assert_eq!(built.gate_id, "gate-1");
        assert_eq!(built.option_key, "approve");
        assert_eq!(built.presented_digest, "ab12");
        assert_eq!(built.who, "chat-42");
        assert_eq!(built.decided_at, ts(1));
    }

    #[test]
    fn approve_and_run_verdict_for_converts_stale_digest_and_keeps_its_gate_id() {
        let verdict = notify::telegram::ResponseVerdict::StaleDigest {
            gate_id: "gate-2".to_string(),
            option_key: "approve".to_string(),
            digest: "stale-digest".to_string(),
            decided_at: ts(2),
        };

        let built = approve_and_run_verdict_for(&verdict, "chat-42")
            .expect("StaleDigest converts to a verdict, letting decide() re-queue it");
        assert_eq!(built.gate_id, "gate-2");
        assert_eq!(built.presented_digest, "stale-digest");
        assert_eq!(built.decided_at, ts(2));
    }

    #[test]
    fn approve_and_run_verdict_for_unknown_gate_yields_none() {
        assert!(
            approve_and_run_verdict_for(&notify::telegram::ResponseVerdict::UnknownGate, "chat-42")
                .is_none()
        );
    }
}

// ── ticket-spawn-schedule-loop task 3: resolver + three-outcome seam tests ─
//
// Tests THE SEAM ONLY — does bastion resolve a path, call
// `engine_serve::schedule::spawn_schedule_loop`, and handle all three
// returns? The engine's own entry-fires-through-dispatch behavior is
// already covered by `crates/engine-serve/tests/schedule.rs` and is
// deliberately not re-tested here (see the spec's Testing Strategy).
//
// Kept in a dedicated module (mirroring `engine_mount_tests` /
// `approve_and_run_seams_wiring_tests` above) for the same reason those
// are: `mod tests` above does `use actix_web::{App, test}`, which shadows
// the built-in `#[test]` attribute with actix's async-only test macro — a
// plain sync `#[test] fn` there fails to compile. This module never
// imports `actix_web::test`, so both plain `#[test]` (for the pure
// resolver) and `#[actix_web::test]` (for the outcome tests, which need a
// tokio runtime because `spawn_schedule_loop` calls `tokio::spawn`
// synchronously) coexist without conflict.
#[cfg(test)]
mod schedule_loop_wiring_tests {
    use super::*;
    use crate::testsupport::{EnvVarGuard, lock_env};

    // ── resolve_engine_harness_path (task 1) ────────────────────────────────

    #[test]
    fn resolve_engine_harness_path_returns_the_path_when_set_and_readable() {
        let _env = lock_env();
        let dir = tempfile::tempdir().expect("tempdir");
        let harness = dir.path().join("harness.json");
        std::fs::write(&harness, "{}").expect("write fixture");
        let _guard = EnvVarGuard::set(
            &_env,
            "BASTION_ENGINE_HARNESS_PATH",
            harness.to_str().expect("utf8 fixture path"),
        );

        assert_eq!(resolve_engine_harness_path(), Some(harness));
    }

    #[test]
    fn resolve_engine_harness_path_is_none_when_unset() {
        let _env = lock_env();
        let _guard = EnvVarGuard::unset(&_env, "BASTION_ENGINE_HARNESS_PATH");

        assert_eq!(resolve_engine_harness_path(), None);
    }

    #[test]
    fn resolve_engine_harness_path_is_none_when_path_does_not_exist() {
        let _env = lock_env();
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("does-not-exist-harness.json");
        let _guard = EnvVarGuard::set(
            &_env,
            "BASTION_ENGINE_HARNESS_PATH",
            missing.to_str().expect("utf8 fixture path"),
        );

        assert_eq!(
            resolve_engine_harness_path(),
            None,
            "a configured-but-missing path must not panic and must resolve to None"
        );
    }

    #[test]
    fn resolve_engine_harness_path_is_none_when_set_to_empty_string() {
        let _env = lock_env();
        let _guard = EnvVarGuard::set(&_env, "BASTION_ENGINE_HARNESS_PATH", "");

        assert_eq!(resolve_engine_harness_path(), None);
    }

    // ── the three spawn_schedule_loop outcomes (task 2) ─────────────────────

    /// A throwaway `Arc<AppState>` mirroring exactly what the production
    /// call site builds — `web::Data::new(state).into_inner()` — but
    /// DB-free (`spawn_durable_writer(None)`) since these tests never touch
    /// Postgres.
    fn schedule_test_state() -> Arc<EngineAppState> {
        web::Data::new(EngineAppState {
            dispatcher: Arc::new(build_engine_dispatcher()),
            live: LiveStateStore::new(),
            durable: spawn_durable_writer(None),
            runs: RunRegistry::new(),
            api_key: "schedule-loop-test-key".to_string(),
        })
        .into_inner()
    }

    #[actix_web::test]
    async fn empty_entries_yields_ok_none_the_clean_no_op() {
        let dir = tempfile::tempdir().expect("tempdir");
        let harness_path = dir.path().join("harness.json");
        std::fs::write(&harness_path, r#"{"schedule": {"entries": []}}"#).expect("write fixture");

        let result =
            engine_serve::schedule::spawn_schedule_loop(&harness_path, schedule_test_state());

        assert!(
            matches!(result, Ok(None)),
            "entries: [] must yield Ok(None) — the clean no-op this ticket's \
             Ok(None) branch handles; got Ok(Some(_)) or Err(_) instead"
        );
    }

    #[actix_web::test]
    async fn malformed_schedule_block_yields_err_and_the_result_stays_absorbable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let harness_path = dir.path().join("harness.json");
        std::fs::write(
            &harness_path,
            r#"{"schedule": {"poll_interval_ms": "fast", "entries": []}}"#,
        )
        .expect("write fixture");

        let result =
            engine_serve::schedule::spawn_schedule_loop(&harness_path, schedule_test_state());

        let got = match &result {
            Ok(Some(_)) => "Ok(Some(_))",
            Ok(None) => "Ok(None)",
            Err(engine_serve::schedule::LoadScheduleError::Io(_)) => "Err(Io(_))",
            Err(engine_serve::schedule::LoadScheduleError::Parse(_)) => "Err(Parse(_))",
            Err(engine_serve::schedule::LoadScheduleError::InvalidSchedule { .. }) => {
                "Err(InvalidSchedule { .. })"
            }
        };
        assert!(
            matches!(
                result,
                Err(engine_serve::schedule::LoadScheduleError::Parse(_))
            ),
            "a non-numeric poll_interval_ms must produce LoadScheduleError::Parse, got {got}"
        );
        // The 'serve still starts' criterion: `spawn_schedule_loop` returning
        // a plain `Result` here (no panic, no early-return via `?` inside
        // this test) is exactly the shape `run_server`'s own `match` relies
        // on to keep booting on this arm — reaching this assertion is the
        // proof the error is absorbable rather than fatal.
    }

    #[actix_web::test]
    async fn absent_config_resolver_returns_none_so_nothing_is_spawned() {
        // Mirrors the production call site's `None` arm exactly: when
        // `resolve_engine_harness_path()` returns `None` (today's deployed
        // Mac Mini state — `BASTION_ENGINE_HARNESS_PATH` unset), the call
        // site never calls `spawn_schedule_loop` at all, so nothing is
        // spawned and nothing is logged at error level.
        let _env = lock_env();
        let _guard = EnvVarGuard::unset(&_env, "BASTION_ENGINE_HARNESS_PATH");

        assert_eq!(
            resolve_engine_harness_path(),
            None,
            "unset BASTION_ENGINE_HARNESS_PATH must resolve to None, matching \
             the deployed Mini's state until an operator sets it"
        );
        // No `spawn_schedule_loop` call happens in this branch — the
        // production `match resolve_engine_harness_path() { Some(p) => ..., \
        // None => Ok(None) }` short-circuits before touching the filesystem,
        // which this test mirrors by never constructing a harness fixture.
    }
}
