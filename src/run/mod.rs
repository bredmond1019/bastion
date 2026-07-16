// `bastion run <workflow>` — trigger a workflow via FastAPI.
// `bastion status`         — quick stack health check (non-TUI).
// `bastion abort <run>`    — operator-facing abort switch (see `abort` module).

pub mod abort;

use anyhow::{Context, Result, anyhow};
use chrono::Utc;

use crate::api::client::{ApiClient, ApiStatus};
use crate::config::{Config, ConfigError};
use crate::costs::budget::{BreachReason, Budget, GateVerdict, Spend, evaluate};
use crate::costs::{self, Window};
use crate::db::costs as db_costs;
use crate::db::health::{self, DbStatus};
use crate::monitor;

/// Parse an optional JSON string argument into a `serde_json::Value`.
///
/// - `None` → `Ok(None)` (caller sends `data: {}` via `trigger_body`)
/// - Valid JSON object string → `Ok(Some(value))`
/// - Malformed JSON → `Err` with a clear message
/// - Valid JSON but not an object (e.g. `"5"`, `"[1,2]"`) → `Err` asking for an object
///
/// Pure function — no I/O — so it is unit-testable without network or filesystem.
pub fn parse_args(args: Option<String>) -> Result<Option<serde_json::Value>> {
    match args {
        None => Ok(None),
        Some(s) => {
            let value: serde_json::Value = serde_json::from_str(&s)
                .with_context(|| format!("--args is not valid JSON: {s}"))?;
            if !value.is_object() {
                return Err(anyhow!(
                    "--args must be a JSON object (got {}), e.g. '{{\"key\": \"value\"}}'",
                    value_type_name(&value)
                ));
            }
            Ok(Some(value))
        }
    }
}

/// Return a human-readable type label for a JSON value (used in error messages).
/// Pure function — no I/O.
fn value_type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Format the success output for a triggered workflow.
/// Pure function — no I/O — so it is unit-testable.
pub fn format_trigger_success(workflow: &str, task_id: &str) -> String {
    format!("workflow: {workflow}\ntask_id: {task_id}\n")
}

// ── Pre-dispatch budget gate (task 8) ────────────────────────────────────────
//
// The Console-side "refuse/warn" half of the block's budget gate. The
// server-side enforcement point is engine-rs `EN.2.B` (already done) and is
// out of scope here — this only decides whether `bastion run` sends the
// trigger request at all, using the same pure `costs::budget::evaluate` core
// task 6's watch loop feeds each poll tick through.

/// The three-way pre-dispatch budget-gate decision. Pure — no I/O.
///
/// - No cap configured on `budget` → [`GateOutcome::NoBudgetConfigured`]:
///   the absent-tolerant case — `trigger`'s I/O shell short-circuits *before*
///   this is even called, so no spend query is made (see `trigger` below).
/// - A cap is configured and current `spend` is within it →
///   [`GateOutcome::Within`] — trigger proceeds.
/// - A cap is configured and breached → [`GateOutcome::Refuse`], carrying
///   which cap tripped it plus the spent/limit values for the operator
///   message.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GateOutcome {
    NoBudgetConfigured,
    Within,
    Refuse(BreachReason),
}

/// Evaluate the pre-dispatch budget gate against `spend`. Pure — no I/O.
///
/// Delegates the actual threshold comparison to `costs::budget::evaluate`
/// (task 3) so the boundary semantics (breach at `>=`, token cap checked
/// before cost cap) stay identical between `run`'s gate and `costs --watch`'s
/// alerting.
pub fn evaluate_gate(budget: &Budget, spend: Spend) -> GateOutcome {
    if budget.max_total_tokens.is_none() && budget.max_cost_usd.is_none() {
        return GateOutcome::NoBudgetConfigured;
    }
    match evaluate(spend, budget) {
        GateVerdict::Within => GateOutcome::Within,
        GateVerdict::Breached(reason) => GateOutcome::Refuse(reason),
    }
}

/// Format the refusal message for a breached budget gate — names the cap,
/// the spent value, and the limit, per the block's acceptance criteria.
/// Pure function — no I/O.
pub fn format_budget_refusal(workflow: &str, reason: &BreachReason) -> String {
    format!(
        "bastion run: refusing to trigger '{workflow}' — budget cap '{}' already breached \
         (spent {:.4}, limit {:.4})\n\
         Pass --force to trigger anyway.\n",
        reason.cap, reason.spent, reason.limit
    )
}

pub async fn trigger(
    workflow: String,
    args: Option<String>,
    monitor: bool,
    force: bool,
) -> Result<()> {
    let data = parse_args(args)?;
    let config = Config::load()?;

    // Pre-dispatch budget gate: only query spend when a cap is actually
    // configured (the absent-tolerant contract — "no extra query" when no
    // budget is set) and the operator hasn't asked to bypass it with
    // --force.
    if !force && (config.max_total_tokens.is_some() || config.max_cost_usd.is_some()) {
        let budget = Budget {
            max_total_tokens: config.max_total_tokens,
            max_cost_usd: config.max_cost_usd,
        };
        match db_costs::fetch_all_runs(&config.database_url).await {
            Ok(runs) => {
                let summary = costs::aggregate(&runs, &Window::All, Utc::now());
                let spend = Spend::from(&summary.totals);
                if let GateOutcome::Refuse(reason) = evaluate_gate(&budget, spend) {
                    eprint!("{}", format_budget_refusal(&workflow, &reason));
                    return Ok(());
                }
            }
            Err(e) => {
                // Fail open: an unreachable DB means the gate can't be
                // evaluated, not that the operator is blocked from
                // triggering — surfaced as a warning, not a refusal.
                eprintln!(
                    "bastion run: could not evaluate the budget gate (database unreachable: {e}); \
                     triggering anyway"
                );
            }
        }
    }

    let client = ApiClient::new(&config.api_base_url);
    let task_id = client
        .trigger_workflow(&workflow, data)
        .await
        .with_context(|| {
            format!("failed to trigger workflow '{workflow}' — is the orchestrator running?")
        })?;
    print!("{}", format_trigger_success(&workflow, &task_id));
    if monitor {
        monitor::run(Some(task_id)).await?;
    }
    Ok(())
}

pub async fn status() -> Result<()> {
    let cfg = Config::load();

    // DATABASE_URL is optional for `status` — missing just shows DB as unreachable.
    let db = match &cfg {
        Ok(c) => health::probe(&c.database_url).await,
        Err(ConfigError::MissingVar(_)) => {
            DbStatus::Unreachable("DATABASE_URL not set".to_string())
        }
        Err(e) => return Err(anyhow!("{e}")),
    };

    let fallback = "http://localhost:8080".to_string();
    let api_url = cfg.as_ref().map(|c| &c.api_base_url).unwrap_or(&fallback);
    let api = ApiClient::new(api_url).health().await;

    print!("{}", render_status(&db, &api));
    Ok(())
}

/// Render a plain-text summary table for the given probe outcomes.
/// Pure function (no I/O) so it can be unit-tested without live services.
fn render_status(db: &DbStatus, api: &ApiStatus) -> String {
    let db_row = match db {
        DbStatus::Reachable => "DB    reachable".to_string(),
        DbStatus::Unreachable(msg) => format!("DB    unreachable ({msg})"),
    };

    let api_row = match api {
        ApiStatus::Reachable { status, version } => {
            format!("API   reachable (status={status}, version={version})")
        }
        ApiStatus::Unreachable(msg) => format!("API   unreachable ({msg})"),
    };

    format!("Stack health\n{db_row}\n{api_row}\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::costs::budget::Cap;

    // ── evaluate_gate — the three branches ─────────────────────────────────────

    #[test]
    fn gate_no_budget_configured_is_unchanged() {
        let budget = Budget::default();
        let spend = Spend {
            total_tokens: u64::MAX,
            total_cost_usd: f64::MAX,
        };
        assert_eq!(
            evaluate_gate(&budget, spend),
            GateOutcome::NoBudgetConfigured
        );
    }

    #[test]
    fn gate_within_ceiling_triggers() {
        let budget = Budget {
            max_total_tokens: Some(1_000_000),
            max_cost_usd: Some(50.0),
        };
        let spend = Spend {
            total_tokens: 1_000,
            total_cost_usd: 1.0,
        };
        assert_eq!(evaluate_gate(&budget, spend), GateOutcome::Within);
    }

    #[test]
    fn gate_breached_refuses_naming_the_cap() {
        let budget = Budget {
            max_total_tokens: Some(1_000),
            max_cost_usd: None,
        };
        let spend = Spend {
            total_tokens: 1_500,
            total_cost_usd: 0.0,
        };
        match evaluate_gate(&budget, spend) {
            GateOutcome::Refuse(reason) => {
                assert_eq!(reason.cap, Cap::MaxTotalTokens);
                assert_eq!(reason.spent, 1_500.0);
                assert_eq!(reason.limit, 1_000.0);
            }
            other => panic!("expected Refuse, got {other:?}"),
        }
    }

    #[test]
    fn gate_breached_cost_cap_refuses() {
        let budget = Budget {
            max_total_tokens: None,
            max_cost_usd: Some(5.0),
        };
        let spend = Spend {
            total_tokens: 0,
            total_cost_usd: 7.5,
        };
        match evaluate_gate(&budget, spend) {
            GateOutcome::Refuse(reason) => {
                assert_eq!(reason.cap, Cap::MaxCostUsd);
                assert_eq!(reason.spent, 7.5);
                assert_eq!(reason.limit, 5.0);
            }
            other => panic!("expected Refuse, got {other:?}"),
        }
    }

    #[test]
    fn gate_exactly_at_limit_refuses() {
        // Boundary case — mirrors costs::budget::evaluate's >= semantics.
        let budget = Budget {
            max_total_tokens: Some(1_000),
            max_cost_usd: None,
        };
        let spend = Spend {
            total_tokens: 1_000,
            total_cost_usd: 0.0,
        };
        assert!(matches!(
            evaluate_gate(&budget, spend),
            GateOutcome::Refuse(_)
        ));
    }

    #[test]
    fn gate_only_one_cap_configured_is_absent_tolerant_for_the_other() {
        let budget = Budget {
            max_total_tokens: Some(1_000_000),
            max_cost_usd: None,
        };
        let spend = Spend {
            total_tokens: 10,
            total_cost_usd: f64::MAX,
        };
        // Only tokens is configured, so the astronomical cost must not
        // spuriously refuse.
        assert_eq!(evaluate_gate(&budget, spend), GateOutcome::Within);
    }

    // ── format_budget_refusal ────────────────────────────────────────────────

    #[test]
    fn format_budget_refusal_names_cap_spent_and_limit() {
        let reason = BreachReason {
            cap: Cap::MaxTotalTokens,
            spent: 1_500.0,
            limit: 1_000.0,
        };
        let msg = format_budget_refusal("my-workflow", &reason);
        assert!(msg.contains("my-workflow"), "got: {msg}");
        assert!(msg.contains("max_total_tokens"), "got: {msg}");
        assert!(msg.contains("1500"), "got: {msg}");
        assert!(msg.contains("1000"), "got: {msg}");
        assert!(msg.contains("--force"), "got: {msg}");
    }

    #[test]
    fn format_budget_refusal_cost_cap_message() {
        let reason = BreachReason {
            cap: Cap::MaxCostUsd,
            spent: 7.5,
            limit: 5.0,
        };
        let msg = format_budget_refusal("wf", &reason);
        assert!(msg.contains("max_cost_usd"), "got: {msg}");
        assert!(msg.contains("7.5"), "got: {msg}");
        assert!(msg.contains("5.0"), "got: {msg}");
    }

    // ── parse_args ────────────────────────────────────────────────────────────

    #[test]
    fn parse_args_none_returns_none() {
        assert!(parse_args(None).unwrap().is_none());
    }

    #[test]
    fn parse_args_valid_object_returns_some() {
        let result = parse_args(Some(r#"{"k": 1, "flag": true}"#.to_string())).unwrap();
        let val = result.unwrap();
        assert!(val.is_object());
        assert_eq!(val["k"], 1);
        assert_eq!(val["flag"], true);
    }

    #[test]
    fn parse_args_malformed_json_returns_err() {
        let err = parse_args(Some("{".to_string())).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("--args is not valid JSON"), "got: {msg}");
    }

    #[test]
    fn parse_args_number_returns_err_with_type_hint() {
        let err = parse_args(Some("5".to_string())).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("--args must be a JSON object"), "got: {msg}");
        assert!(msg.contains("number"), "got: {msg}");
    }

    #[test]
    fn parse_args_array_returns_err_with_type_hint() {
        let err = parse_args(Some("[1,2,3]".to_string())).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("--args must be a JSON object"), "got: {msg}");
        assert!(msg.contains("array"), "got: {msg}");
    }

    #[test]
    fn parse_args_string_value_returns_err() {
        let err = parse_args(Some(r#""hello""#.to_string())).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("--args must be a JSON object"), "got: {msg}");
        assert!(msg.contains("string"), "got: {msg}");
    }

    // ── format_trigger_success ────────────────────────────────────────────────

    #[test]
    fn format_trigger_success_contains_workflow_and_task_id() {
        let out = format_trigger_success("research_workflow", "abc-123");
        assert!(out.contains("workflow: research_workflow"), "got: {out}");
        assert!(out.contains("task_id: abc-123"), "got: {out}");
    }

    #[test]
    fn format_trigger_success_task_id_on_own_line() {
        let out = format_trigger_success("wf", "tid-999");
        // Each key should be on its own line
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines.contains(&"workflow: wf"), "lines: {lines:?}");
        assert!(lines.contains(&"task_id: tid-999"), "lines: {lines:?}");
    }

    // ── value_type_name ───────────────────────────────────────────────────────

    #[test]
    fn value_type_name_variants() {
        use serde_json::Value;
        assert_eq!(value_type_name(&Value::Null), "null");
        assert_eq!(value_type_name(&Value::Bool(true)), "boolean");
        assert_eq!(value_type_name(&Value::Number(42.into())), "number");
        assert_eq!(value_type_name(&Value::String("s".into())), "string");
        assert_eq!(value_type_name(&Value::Array(vec![])), "array");
        assert_eq!(
            value_type_name(&Value::Object(serde_json::Map::new())),
            "object"
        );
    }

    // ── render_status ─────────────────────────────────────────────────────────

    #[test]
    fn render_both_unreachable() {
        let out = render_status(
            &DbStatus::Unreachable("connection refused".to_string()),
            &ApiStatus::Unreachable("timeout".to_string()),
        );
        assert!(out.contains("DB    unreachable (connection refused)"));
        assert!(out.contains("API   unreachable (timeout)"));
    }

    #[test]
    fn render_both_reachable() {
        let out = render_status(
            &DbStatus::Reachable,
            &ApiStatus::Reachable {
                status: "ok".to_string(),
                version: "0.1.0".to_string(),
            },
        );
        assert!(out.contains("DB    reachable"));
        assert!(out.contains("API   reachable (status=ok, version=0.1.0)"));
    }

    #[test]
    fn render_mixed_db_up_api_down() {
        let out = render_status(
            &DbStatus::Reachable,
            &ApiStatus::Unreachable("HTTP 503".to_string()),
        );
        assert!(out.contains("DB    reachable"));
        assert!(out.contains("API   unreachable (HTTP 503)"));
    }
}
