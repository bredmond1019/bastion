//! `GET /api/costs` read-only cost + budget-state REST handler for `bastion
//! serve` (BA.11.J).
//!
//! Read-only (D25) — this route never mutates the configured budget caps.
//! Mutation stays CLI/D48. It exposes the existing `src/costs/` aggregation
//! (BA.7.B exact counts) and `costs::budget` gate evaluation (BA.7.C) over
//! HTTP, unchanged.
//!
//! # Route
//! - `GET /api/costs?window=7d|30d|all` — case-insensitive; omitting `window`
//!   applies [`DEFAULT_WINDOW`]. **No `?repo=` filter** — the `events`
//!   contract carries no repo dimension (`WorkflowRun` has no repo field;
//!   `costs::aggregate` groups by `workflow_name` only). This is a deliberate
//!   deviation from the block definition's optional `?repo=`, decided with
//!   the owner 2026-07-29 (see `tasks.md`'s Amendment Log).
//!
//! # Pure core vs I/O shell (Rule 6)
//! [`resolve_window`], [`budget_state_dto`], [`cost_summary_dto`], and
//! [`budget_from_config`] are pure — unit-tested directly, no filesystem/DB
//! access. [`get_costs_with`] carries the entire handler body — resolve
//! window, load [`Config`], fetch every run, then hand the pure functions
//! the resulting `CostSummary` — parameterized over the fetch function
//! (a generic `F: FnOnce(String) -> Fut` parameter) so the contract corpus
//! can freeze the populated `200`, budget-breached, empty, windowed, and
//! `C009` shapes without a live Postgres. [`get_costs`] is a
//! **compile-time-only** delegation to
//! [`get_costs_with`] passing `db::costs::fetch_all_runs` — there is no env
//! var, config key, or `app_data` slot by which fixture rows could reach a
//! running `bastion serve`; the only way to supply fake rows is test code
//! linking against `get_costs_with` directly.
//!
//! # Error mapping
//! - Unparseable `window` → 400 + `C006` (invalid input).
//! - `DATABASE_URL` unset (`Config::load` → `ConfigError::MissingVar`) → 503 +
//!   `C005` (configuration error) — the route mounts unconditionally (unlike
//!   the engine routes) and degrades here rather than being absent.
//! - Postgres unreachable or the query failing (`db::costs::fetch_all_runs`
//!   error) → 503 + `C009` (I/O error).
//! - `web::block` thread-pool failure → 500 + `C010`, mirroring
//!   `handlers/board.rs::blocking_error_response`.

use std::future::Future;

use actix_web::{HttpResponse, web};
use serde::Deserialize;

use crate::config::Config;
use crate::costs::budget::{Budget, GateVerdict, Spend, evaluate};
use crate::costs::{CostSummary, Window, aggregate, parse_window};
use crate::db::costs as db_costs;
use crate::db::workflows::WorkflowRun;
use crate::serve::dto::{BudgetBreachDto, BudgetStateDto, CostSummaryDto, ErrorPayload};

/// Window applied when `?window=` is absent or empty: the last 7 days,
/// matching `bastion costs`' own default (`Commands::Costs`'s `--last`
/// default in `cli.rs`).
const DEFAULT_WINDOW: &str = "7d";

// ── Query params ─────────────────────────────────────────────────────────────

/// `GET /api/costs` query params.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CostsQuery {
    /// `window=7d|30d|all` (case-insensitive); missing/empty → [`DEFAULT_WINDOW`].
    #[serde(default)]
    pub window: Option<String>,
}

// ── Pure core ────────────────────────────────────────────────────────────────

/// Resolve the `?window=` query param into a [`Window`], applying
/// [`DEFAULT_WINDOW`] when `param` is `None` or empty/whitespace-only.
///
/// Delegates parsing to `costs::parse_window` — this function does not
/// reimplement the `"7d"`/`"30d"`/`"all"` matching, only the default-
/// substitution `costs::parse_window` itself does not do. Returns the
/// error message on an unparseable value (mapped to 400/`C006` by the
/// caller).
pub fn resolve_window(param: Option<&str>) -> Result<Window, String> {
    let raw = param
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_WINDOW);
    parse_window(raw).map_err(|e| e.to_string())
}

/// Build a [`Budget`] from the loaded [`Config`], mirroring
/// `costs::watch::run`'s construction (`src/costs/watch.rs:123`) rather than
/// inventing a second one.
pub fn budget_from_config(config: &Config) -> Budget {
    Budget {
        max_total_tokens: config.max_total_tokens,
        max_cost_usd: config.max_cost_usd,
    }
}

/// Project `summary.totals` + `budget` into the wire [`BudgetStateDto`]:
/// builds a [`Spend`] reading via the existing `From<&WorkflowCost>` impl,
/// evaluates it against `budget`, and maps the [`GateVerdict`] onto
/// `breached` + an optional [`BudgetBreachDto`] using [`Cap::as_str`].
///
/// A [`Budget::default()`] (both caps `None`) always reports `breached:
/// false` — "no gate configured" behaves exactly as `evaluate` documents.
pub fn budget_state_dto(summary: &CostSummary, budget: &Budget) -> BudgetStateDto {
    let spend = Spend::from(&summary.totals);
    let verdict = evaluate(spend, budget);

    let (breached, breach) = match verdict {
        GateVerdict::Within => (false, None),
        GateVerdict::Breached(reason) => (
            true,
            Some(BudgetBreachDto {
                cap: reason.cap.as_str().to_owned(),
                spent: reason.spent,
                limit: reason.limit,
            }),
        ),
    };

    BudgetStateDto {
        max_total_tokens: budget.max_total_tokens,
        max_cost_usd: budget.max_cost_usd,
        total_tokens: spend.total_tokens,
        total_cost_usd: spend.total_cost_usd,
        breached,
        breach,
    }
}

/// Echo `window` back as its wire string — the inverse of `resolve_window`'s
/// input, not `costs::parse_window`'s (there is no `Window -> &str`
/// round-trip in `costs::mod`, so this mirrors it locally rather than adding
/// one there for a single caller).
fn window_label(window: &Window) -> String {
    match window {
        Window::Days(7) => "7d".to_owned(),
        Window::Days(30) => "30d".to_owned(),
        Window::Days(n) => format!("{n}d"),
        Window::All => "all".to_owned(),
    }
}

/// Project a [`CostSummary`] + the resolved [`Window`] + [`Budget`] into the
/// full [`CostSummaryDto`] wire shape: rows/totals/`unpriced_models` carried
/// through verbatim (order preserved — `costs::aggregate` already sorts by
/// `usd` descending), plus the echoed window string and the budget state.
pub fn cost_summary_dto(summary: &CostSummary, window: &Window, budget: &Budget) -> CostSummaryDto {
    CostSummaryDto {
        window: window_label(window),
        rows: summary
            .rows
            .iter()
            .map(|r| crate::serve::dto::WorkflowCostDto {
                workflow_name: r.workflow_name.clone(),
                runs: r.runs,
                tokens_in: r.tokens_in,
                tokens_out: r.tokens_out,
                usd: r.usd,
            })
            .collect(),
        totals: crate::serve::dto::WorkflowCostDto {
            workflow_name: summary.totals.workflow_name.clone(),
            runs: summary.totals.runs,
            tokens_in: summary.totals.tokens_in,
            tokens_out: summary.totals.tokens_out,
            usd: summary.totals.usd,
        },
        unpriced_models: summary.unpriced_models.clone(),
        budget: budget_state_dto(summary, budget),
    }
}

// ── I/O shell ────────────────────────────────────────────────────────────────

/// Build a 400 response for an unparseable `?window=` value.
fn invalid_window_response(message: impl std::fmt::Display) -> HttpResponse {
    HttpResponse::BadRequest().json(ErrorPayload {
        code: "C006".to_owned(),
        message: message.to_string(),
    })
}

/// Build a 503 response for a configuration error (`DATABASE_URL` unset).
fn config_error_response(message: impl std::fmt::Display) -> HttpResponse {
    HttpResponse::ServiceUnavailable().json(ErrorPayload {
        code: "C005".to_owned(),
        message: message.to_string(),
    })
}

/// Build a 503 response for a database I/O failure (unreachable Postgres,
/// or the `events` query itself failing).
fn db_error_response(message: impl std::fmt::Display) -> HttpResponse {
    HttpResponse::ServiceUnavailable().json(ErrorPayload {
        code: "C009".to_owned(),
        message: message.to_string(),
    })
}

/// Build a 500 response from a `BlockingError` (thread panic / runtime
/// shutdown), mirroring `handlers/board.rs::blocking_error_response`.
fn blocking_error_response(err: actix_web::error::BlockingError) -> HttpResponse {
    HttpResponse::InternalServerError().json(ErrorPayload {
        code: "C010".to_owned(),
        message: format!("blocking thread error: {err}"),
    })
}

/// `GET /api/costs` — read-only cost + budget-state summary for the
/// resolved window.
///
/// Bearer auth is inherited from the `/api` scope's `BearerAuthMiddleware` —
/// a request without a valid token never reaches this handler (401
/// upstream). Mounts unconditionally: a missing/unreachable database is
/// answered as a typed error response here, not an absent route.
///
/// A thin, **compile-time-only** delegation to [`get_costs_with`], passing
/// `db::costs::fetch_all_runs` as the fetch function. There is deliberately
/// no env var, config key, `app_data` entry, or CLI flag that could swap
/// this for a different fetch function at runtime — only test code linking
/// against `get_costs_with` directly can supply fixture rows.
pub async fn get_costs(query: web::Query<CostsQuery>) -> HttpResponse {
    get_costs_with(query, |db_url| async move {
        db_costs::fetch_all_runs(&db_url).await
    })
    .await
}

/// The entire `GET /api/costs` handler body, parameterized over the
/// runs-fetching function `fetch_runs`.
///
/// `fetch_runs` is resolved at **compile time** as a generic type parameter
/// (`F: FnOnce(String) -> Fut`), not injected via any runtime mechanism.
/// Production calls this through [`get_costs`] with `db::costs::fetch_all_runs`;
/// the contract corpus (`serve::contract_corpus::costs_scenarios`) calls it
/// directly with a closure over fixture rows so the populated `200`,
/// budget-breached, empty, windowed, and `C009` shapes can be frozen without
/// a live Postgres.
///
/// Resolves the window first — an unparseable `?window=` returns 400/`C006`
/// **before `fetch_runs` is ever invoked** (see the corpus's `bad-window`
/// golden, and the `unparseable_window_never_invokes_fetch` test below).
pub async fn get_costs_with<F, Fut>(query: web::Query<CostsQuery>, fetch_runs: F) -> HttpResponse
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = anyhow::Result<Vec<WorkflowRun>>>,
{
    let window = match resolve_window(query.window.as_deref()) {
        Ok(w) => w,
        Err(e) => return invalid_window_response(e),
    };

    let config = match Config::load() {
        Ok(c) => c,
        Err(e) => return config_error_response(e),
    };

    let runs = match fetch_runs(config.database_url.clone()).await {
        Ok(runs) => runs,
        Err(e) => return db_error_response(e),
    };

    let budget = budget_from_config(&config);

    match web::block(move || {
        let summary = aggregate(&runs, &window, chrono::Utc::now());
        cost_summary_dto(&summary, &window, &budget)
    })
    .await
    {
        Ok(dto) => HttpResponse::Ok().json(dto),
        Err(err) => blocking_error_response(err),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::costs::budget::{BreachReason, Cap};
    use crate::db::workflows::{EventRow, NodeState, RunStatus, WorkflowRun, parse_event_row};

    // ─── resolve_window ─────────────────────────────────────────────────────

    #[test]
    fn resolve_window_accepts_7d() {
        assert_eq!(resolve_window(Some("7d")).unwrap(), Window::Days(7));
    }

    #[test]
    fn resolve_window_accepts_30d() {
        assert_eq!(resolve_window(Some("30d")).unwrap(), Window::Days(30));
    }

    #[test]
    fn resolve_window_accepts_all() {
        assert_eq!(resolve_window(Some("all")).unwrap(), Window::All);
    }

    #[test]
    fn resolve_window_is_case_insensitive() {
        assert_eq!(resolve_window(Some("7D")).unwrap(), Window::Days(7));
        assert_eq!(resolve_window(Some("ALL")).unwrap(), Window::All);
        assert_eq!(resolve_window(Some("30D")).unwrap(), Window::Days(30));
    }

    #[test]
    fn resolve_window_applies_default_when_absent() {
        assert_eq!(resolve_window(None).unwrap(), Window::Days(7));
    }

    #[test]
    fn resolve_window_applies_default_when_empty() {
        assert_eq!(resolve_window(Some("")).unwrap(), Window::Days(7));
        assert_eq!(resolve_window(Some("   ")).unwrap(), Window::Days(7));
    }

    #[test]
    fn resolve_window_errors_on_garbage() {
        let err = resolve_window(Some("nonsense")).unwrap_err();
        assert!(
            err.contains("nonsense"),
            "error should name the bad value: {err}"
        );
    }

    // ─── budget_state_dto ───────────────────────────────────────────────────

    fn workflow_cost(usd: f64, tokens_in: u64, tokens_out: u64) -> crate::costs::WorkflowCost {
        crate::costs::WorkflowCost {
            workflow_name: "TOTAL".to_owned(),
            runs: 1,
            tokens_in,
            tokens_out,
            usd,
        }
    }

    fn summary_with_totals(totals: crate::costs::WorkflowCost) -> CostSummary {
        CostSummary {
            rows: vec![],
            totals,
            unpriced_models: vec![],
        }
    }

    #[test]
    fn budget_state_dto_no_caps_is_within() {
        let summary = summary_with_totals(workflow_cost(1.0, 100, 50));
        let budget = Budget::default();

        let dto = budget_state_dto(&summary, &budget);

        assert!(!dto.breached);
        assert!(dto.breach.is_none());
        assert_eq!(dto.max_total_tokens, None);
        assert_eq!(dto.max_cost_usd, None);
        assert_eq!(dto.total_tokens, 150);
        assert_eq!(dto.total_cost_usd, 1.0);
    }

    #[test]
    fn budget_state_dto_tokens_cap_breached() {
        let summary = summary_with_totals(workflow_cost(1.0, 900, 200));
        let budget = Budget {
            max_total_tokens: Some(1000),
            max_cost_usd: None,
        };

        let dto = budget_state_dto(&summary, &budget);

        assert!(dto.breached);
        let breach = dto.breach.expect("breach present");
        assert_eq!(breach.cap, "max_total_tokens");
        assert_eq!(breach.spent, 1100.0);
        assert_eq!(breach.limit, 1000.0);
    }

    #[test]
    fn budget_state_dto_cost_cap_breached() {
        let summary = summary_with_totals(workflow_cost(2.5, 10, 10));
        let budget = Budget {
            max_total_tokens: None,
            max_cost_usd: Some(2.0),
        };

        let dto = budget_state_dto(&summary, &budget);

        assert!(dto.breached);
        let breach = dto.breach.expect("breach present");
        assert_eq!(breach.cap, "max_cost_usd");
        assert_eq!(breach.spent, 2.5);
        assert_eq!(breach.limit, 2.0);
    }

    #[test]
    fn budget_state_dto_boundary_at_limit_is_breached() {
        // `evaluate`'s documented boundary: spend exactly at the limit counts
        // as breached (`>=`), not merely approaching it.
        let summary = summary_with_totals(workflow_cost(0.0, 1000, 0));
        let budget = Budget {
            max_total_tokens: Some(1000),
            max_cost_usd: None,
        };

        let dto = budget_state_dto(&summary, &budget);

        assert!(dto.breached, "spend exactly at the cap must be breached");
        assert_eq!(dto.breach.unwrap().cap, "max_total_tokens");
    }

    #[test]
    fn budget_state_dto_cap_as_str_matches_reason() {
        // Sanity: BudgetBreachDto.cap uses the exact Cap::as_str() strings.
        assert_eq!(Cap::MaxTotalTokens.as_str(), "max_total_tokens");
        assert_eq!(Cap::MaxCostUsd.as_str(), "max_cost_usd");
        let reason = BreachReason {
            cap: Cap::MaxCostUsd,
            spent: 5.0,
            limit: 4.0,
        };
        assert_eq!(reason.cap.as_str(), "max_cost_usd");
    }

    // ─── cost_summary_dto ───────────────────────────────────────────────────

    fn make_node(model: &str, tokens_in: u64, tokens_out: u64) -> NodeState {
        NodeState {
            id: "n1".to_owned(),
            name: "n1".to_owned(),
            status: RunStatus::Success,
            depends_on: vec![],
            input: None,
            output: None,
            error: None,
            tokens_in: Some(tokens_in),
            tokens_out: Some(tokens_out),
            model: Some(model.to_owned()),
            started_at: Some("2026-06-20T09:00:00Z".to_owned()),
            elapsed_secs: None,
        }
    }

    fn make_run(workflow_name: &str, usd_nodes: Vec<NodeState>) -> WorkflowRun {
        WorkflowRun {
            id: format!("run-{workflow_name}"),
            workflow_name: workflow_name.to_owned(),
            status: RunStatus::Success,
            budget_halt: None,
            nodes: usd_nodes,
            started_at: Some("2026-06-20T09:00:00Z".to_owned()),
            elapsed_secs: None,
        }
    }

    #[test]
    fn cost_summary_dto_preserves_row_order_totals_and_unpriced() {
        let cheap = make_run(
            "cheap-pipeline",
            vec![make_node("claude-3-5-haiku-20241022", 100, 50)],
        );
        let expensive = make_run(
            "expensive-pipeline",
            vec![make_node("some-unpriced-model", 1000, 500)],
        );
        let now = chrono::DateTime::parse_from_rfc3339("2026-06-25T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        let summary = aggregate(&[cheap, expensive], &Window::All, now);
        let budget = Budget::default();
        let dto = cost_summary_dto(&summary, &Window::All, &budget);

        assert_eq!(dto.window, "all");
        assert_eq!(dto.rows.len(), summary.rows.len());
        for (dto_row, summary_row) in dto.rows.iter().zip(summary.rows.iter()) {
            assert_eq!(dto_row.workflow_name, summary_row.workflow_name);
            assert_eq!(dto_row.usd, summary_row.usd);
        }
        assert_eq!(dto.totals.usd, summary.totals.usd);
        assert_eq!(dto.totals.runs, summary.totals.runs);
        assert_eq!(dto.unpriced_models, summary.unpriced_models);
        assert!(
            dto.unpriced_models
                .contains(&"some-unpriced-model".to_owned()),
            "unpriced model must be carried through"
        );
    }

    #[test]
    fn cost_summary_dto_window_label_days() {
        let summary = summary_with_totals(workflow_cost(0.0, 0, 0));
        let dto7 = cost_summary_dto(&summary, &Window::Days(7), &Budget::default());
        let dto30 = cost_summary_dto(&summary, &Window::Days(30), &Budget::default());
        let dto_all = cost_summary_dto(&summary, &Window::All, &Budget::default());
        assert_eq!(dto7.window, "7d");
        assert_eq!(dto30.window, "30d");
        assert_eq!(dto_all.window, "all");
    }

    // ─── CLI-parity: DTO must not drift from `costs::aggregate` ────────────

    const COMPLETED_FIXTURE: &str = include_str!("../../db/fixtures/completed_run.json");
    const IN_PROGRESS_FIXTURE: &str = include_str!("../../db/fixtures/in_progress_run.json");
    const CANCELLED_FIXTURE: &str = include_str!("../../db/fixtures/cancelled_run.json");

    fn run_from_fixture(id: &str, workflow_type: &str, fixture: &str) -> WorkflowRun {
        let task_context: serde_json::Value = serde_json::from_str(fixture).unwrap();
        let row = EventRow {
            id: id.to_owned(),
            workflow_type: workflow_type.to_owned(),
            task_context: Some(task_context),
        };
        parse_event_row(row).expect("fixture parses into a WorkflowRun")
    }

    #[test]
    fn cost_summary_dto_matches_aggregate_over_fixture_runs() {
        let runs = vec![
            run_from_fixture("run-1", "content-pipeline", COMPLETED_FIXTURE),
            run_from_fixture("run-2", "content-pipeline", IN_PROGRESS_FIXTURE),
            run_from_fixture("run-3", "research-pipeline", CANCELLED_FIXTURE),
        ];

        let now = chrono::Utc::now();
        let window = Window::All;
        let summary = aggregate(&runs, &window, now);
        let budget = Budget::default();
        let dto = cost_summary_dto(&summary, &window, &budget);

        // Rows and totals must equal `CostSummary`'s fields verbatim — the
        // endpoint and `bastion costs` cannot drift.
        assert_eq!(dto.rows.len(), summary.rows.len());
        for (dto_row, summary_row) in dto.rows.iter().zip(summary.rows.iter()) {
            assert_eq!(dto_row.workflow_name, summary_row.workflow_name);
            assert_eq!(dto_row.runs, summary_row.runs);
            assert_eq!(dto_row.tokens_in, summary_row.tokens_in);
            assert_eq!(dto_row.tokens_out, summary_row.tokens_out);
            assert_eq!(dto_row.usd, summary_row.usd);
        }
        assert_eq!(dto.totals.runs, summary.totals.runs);
        assert_eq!(dto.totals.tokens_in, summary.totals.tokens_in);
        assert_eq!(dto.totals.tokens_out, summary.totals.tokens_out);
        assert_eq!(dto.totals.usd, summary.totals.usd);
        assert_eq!(dto.unpriced_models, summary.unpriced_models);
    }

    // ─── get_costs_with (compile-time fetch seam) ──────────────────────────

    /// Env discipline mirrors `contract_corpus::costs_scenarios`: one
    /// [`crate::testsupport::lock_env`] per test held for the whole test,
    /// every mutation through `EnvVarGuard`, `DotenvShadow` to neutralize
    /// `dotenvy`'s upward `.env` walk, `XDG_CONFIG_HOME` pointed at an empty
    /// temp dir so no developer config file can supply a `database_url` or a
    /// budget cap, and `DATABASE_URL` set to an unroutable dummy that is
    /// never actually connected to (the fetch function is faked).
    struct PinnedEnv {
        _lock: crate::testsupport::EnvLock,
        _dotenv: crate::testsupport::DotenvShadow,
        _database_url: crate::testsupport::EnvVarGuard,
        _max_total_tokens: crate::testsupport::EnvVarGuard,
        _max_cost_usd: crate::testsupport::EnvVarGuard,
        _xdg: crate::testsupport::EnvVarGuard,
        empty_config_home: std::path::PathBuf,
    }

    impl PinnedEnv {
        fn new(label: &str) -> Self {
            let lock = crate::testsupport::lock_env();
            let dotenv = crate::testsupport::DotenvShadow::new(&lock, label);
            let database_url = crate::testsupport::EnvVarGuard::set(
                &lock,
                "DATABASE_URL",
                "postgres://corpus@127.0.0.1:1/corpus",
            );
            let max_total_tokens =
                crate::testsupport::EnvVarGuard::unset(&lock, "BASTION_MAX_TOTAL_TOKENS");
            let max_cost_usd =
                crate::testsupport::EnvVarGuard::unset(&lock, "BASTION_MAX_COST_USD");

            let empty_config_home =
                crate::testsupport::unique_temp_dir(&format!("bastion-costs-handler-{label}"));
            std::fs::create_dir_all(&empty_config_home).unwrap();
            let xdg = crate::testsupport::EnvVarGuard::set(
                &lock,
                "XDG_CONFIG_HOME",
                empty_config_home.to_str().expect("temp path must be UTF-8"),
            );

            Self {
                _lock: lock,
                _dotenv: dotenv,
                _database_url: database_url,
                _max_total_tokens: max_total_tokens,
                _max_cost_usd: max_cost_usd,
                _xdg: xdg,
                empty_config_home,
            }
        }
    }

    impl Drop for PinnedEnv {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.empty_config_home);
        }
    }

    async fn body_json<T: serde::de::DeserializeOwned>(resp: HttpResponse) -> T {
        let bytes = actix_web::body::to_bytes(resp.into_body())
            .await
            .expect("response body must be readable");
        serde_json::from_slice(&bytes).expect("response body must be valid JSON for T")
    }

    #[actix_web::test]
    async fn get_costs_with_fixture_rows_matches_aggregate_and_dto() {
        let _env = PinnedEnv::new("populated");

        let cheap = make_run(
            "cheap-pipeline",
            vec![make_node("claude-3-5-haiku-20241022", 100, 50)],
        );
        let expensive = make_run(
            "expensive-pipeline",
            vec![make_node("some-unpriced-model", 1000, 500)],
        );
        let runs = vec![cheap, expensive];
        let expected_summary = aggregate(&runs, &Window::All, chrono::Utc::now());
        let expected_dto = cost_summary_dto(&expected_summary, &Window::All, &Budget::default());

        let query = web::Query::<CostsQuery>(CostsQuery {
            window: Some("all".to_owned()),
        });
        let resp = get_costs_with(query, move |_db_url| async move { Ok(runs) }).await;

        assert_eq!(resp.status(), actix_web::http::StatusCode::OK);
        let dto: CostSummaryDto = body_json(resp).await;
        assert_eq!(dto.window, expected_dto.window);
        assert_eq!(dto.rows.len(), expected_dto.rows.len());
        for (got, want) in dto.rows.iter().zip(expected_dto.rows.iter()) {
            assert_eq!(got.workflow_name, want.workflow_name);
            assert_eq!(got.usd, want.usd);
        }
        assert_eq!(dto.totals.usd, expected_dto.totals.usd);
        assert_eq!(dto.unpriced_models, expected_dto.unpriced_models);
        assert!(!dto.budget.breached);
    }

    #[actix_web::test]
    async fn get_costs_with_fetch_err_returns_503_c009() {
        let _env = PinnedEnv::new("db-error");

        let query = web::Query::<CostsQuery>(CostsQuery { window: None });
        let resp = get_costs_with(query, |_db_url| async move {
            Err::<Vec<WorkflowRun>, _>(anyhow::anyhow!("simulated fetch failure"))
        })
        .await;

        assert_eq!(
            resp.status(),
            actix_web::http::StatusCode::SERVICE_UNAVAILABLE
        );
        let payload: ErrorPayload = body_json(resp).await;
        assert_eq!(payload.code, "C009");
    }

    #[actix_web::test]
    async fn get_costs_with_unparseable_window_never_invokes_fetch() {
        // No env pinning needed at all — this is itself part of the
        // contract: `resolve_window` must run before any `Config::load` or
        // fetch call, so this response is produced with no env/database
        // dependency whatsoever.
        let invoked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let invoked_for_closure = std::sync::Arc::clone(&invoked);

        let query = web::Query::<CostsQuery>(CostsQuery {
            window: Some("nonsense".to_owned()),
        });
        let resp = get_costs_with(query, move |_db_url| {
            invoked_for_closure.store(true, std::sync::atomic::Ordering::SeqCst);
            async move { Ok(Vec::<WorkflowRun>::new()) }
        })
        .await;

        assert_eq!(resp.status(), actix_web::http::StatusCode::BAD_REQUEST);
        let payload: ErrorPayload = body_json(resp).await;
        assert_eq!(payload.code, "C006");
        assert!(
            !invoked.load(std::sync::atomic::Ordering::SeqCst),
            "fetch_runs must not be invoked when the window fails to parse"
        );
    }
}
