//! Live spend watch + threshold alerts (BA.7.C task 6) — the loop behind
//! `bastion costs --watch`.
//!
//! Keeps the thin-shell/pure-core split (Rule 6, the established `tmux.rs`
//! construction-vs-execution pattern): tick sequencing, the alert-vs-no-alert
//! decision, and rendering are pure functions asserted directly here; only
//! the poll/sleep/DB-fetch loop in [`run`] is I/O.
//!
//! Reuses the existing `costs::aggregate` / `costs::render_table` pipeline —
//! it does not fork the aggregation — and feeds each tick's
//! [`super::CostSummary::totals`] through [`super::budget::evaluate`] and
//! [`super::budget::detect_crossing`] (task 3) to decide whether this tick's
//! crossing warrants a structured `observ` alert.

use std::time::Duration;

use anyhow::Result;
use chrono::Utc;

use super::budget::{Budget, Crossing, GateVerdict, Spend, detect_crossing, evaluate};
use super::{Window, aggregate, render_table};
use crate::config::Config;
use crate::db::costs as db_costs;
use crate::observ;

// ── Pure: per-tick decision ─────────────────────────────────────────────────

/// The pure decision for a single watch tick: what to render, and whether
/// this tick constitutes a fresh crossing that warrants an alert.
///
/// Built by [`tick`] from the aggregated summary and the previous tick's
/// verdict; consumed by [`run`]'s thin I/O shell, which does the actual
/// printing / `observ` emission.
#[derive(Debug, Clone, PartialEq)]
pub struct TickOutcome {
    /// The rendered spend table for this tick (always re-rendered).
    pub rendered: String,
    /// This tick's gate verdict — carried forward as `previous` on the next
    /// tick so [`detect_crossing`] can distinguish a fresh crossing from a
    /// sustained breach.
    pub verdict: GateVerdict,
    /// `Some` iff this tick is a fresh crossing that should alert; mirrors
    /// [`Crossing::FreshBreach`] only — [`Crossing::SustainedBreach`] and
    /// [`Crossing::Within`] both yield `None` here.
    pub alert: Option<super::budget::BreachReason>,
}

/// Evaluate one watch tick against `budget` and the `previous` tick's
/// verdict, using `runs` (the raw DB rows, already fetched) and `now`.
///
/// Pure function — no I/O, no sleep, no fetch. Reuses `costs::aggregate` /
/// `costs::render_table` (the existing one-shot pipeline, not forked) and
/// `costs::budget::evaluate` / `detect_crossing` (task 3) for the gate
/// decision.
pub fn tick(
    runs: &[crate::db::workflows::WorkflowRun],
    window: &Window,
    now: chrono::DateTime<Utc>,
    budget: &Budget,
    previous: Option<GateVerdict>,
) -> TickOutcome {
    let summary = aggregate(runs, window, now);
    let rendered = render_table(&summary);
    let spend = Spend::from(&summary.totals);
    let verdict = evaluate(spend, budget);

    let alert = match detect_crossing(previous, verdict) {
        Crossing::FreshBreach(reason) => Some(reason),
        Crossing::SustainedBreach | Crossing::Within => None,
    };

    TickOutcome {
        rendered,
        verdict,
        alert,
    }
}

/// Build the structured alert message for a fresh crossing — carries the
/// cap name, spent value, and limit, per the acceptance criteria. Pure
/// function — no I/O; [`run`]'s thin shell hands this to `observ`.
pub fn alert_message(reason: &super::budget::BreachReason) -> String {
    format!(
        "budget alert: cap '{}' breached — spent {:.4}, limit {:.4}",
        reason.cap, reason.spent, reason.limit
    )
}

// ── Thin I/O shell ───────────────────────────────────────────────────────────

/// Run `bastion costs --watch <window>`.
///
/// Polls on `Config::poll_interval_secs`, re-running the existing
/// `db_costs::fetch_all_runs` + `aggregate` pipeline and re-rendering
/// `render_table` each tick until interrupted (Ctrl-C / process signal —
/// no in-process stop condition; this loop runs until the process exits).
///
/// Degrades rather than panics on a DB failure: prints a `C0xx`-coded
/// message and keeps polling — a transient failure mid-watch does not kill
/// the loop. Uses the same graceful posture as `costs::run`, plus the
/// `C0xx` code and loop resilience `costs::run` doesn't need (it runs once).
pub async fn run(window: String) -> Result<()> {
    let window = match super::parse_window(&window) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("bastion costs --watch: {e}");
            return Ok(());
        }
    };

    let config = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "bastion costs --watch: {e}\n\
                 Set DATABASE_URL in your .env file or environment:\n\
                 DATABASE_URL=postgres://user:pass@localhost:5432/dbname"
            );
            return Ok(());
        }
    };

    let budget = Budget {
        max_total_tokens: config.max_total_tokens,
        max_cost_usd: config.max_cost_usd,
    };

    let mut previous: Option<GateVerdict> = None;
    let poll_interval = Duration::from_secs(config.poll_interval_secs);

    loop {
        match db_costs::fetch_all_runs(&config.database_url).await {
            Ok(runs) => {
                let now = Utc::now();
                let outcome = tick(&runs, &window, now, &budget, previous);

                // Clear-ish redraw: a leading blank line separates each tick
                // rather than mixing with terminal escape sequences, keeping
                // this shell trivially testable-in-spirit and dependency-free.
                print!("{}", outcome.rendered);

                if let Some(reason) = &outcome.alert {
                    let msg = alert_message(reason);
                    eprintln!("{msg}");
                    // Structured `observ` alert event — carries the cap name,
                    // spent value, and limit. Not a `C0xx`-coded error: a
                    // budget crossing is an operator-facing signal, not a
                    // command failure, so it does not go through
                    // `emit_outcome` (which reserves its `error_code` slot
                    // for the C001-C014 taxonomy).
                    tracing::warn!(
                        event = "budget_alert",
                        cap = %reason.cap,
                        spent = reason.spent,
                        limit = reason.limit,
                        "budget threshold crossed"
                    );
                }

                previous = Some(outcome.verdict);
            }
            Err(e) => {
                eprintln!(
                    "bastion costs --watch: could not connect to database: {e} [{}]",
                    observ::errors::ErrorCode::IoError
                );
                // Transient failure — keep the loop alive; do not clobber
                // `previous`, so a crossing detected on the next successful
                // tick is still evaluated against the last known verdict.
            }
        }

        tokio::time::sleep(poll_interval).await;
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::workflows::{NodeState, RunStatus, WorkflowRun};

    fn fixed_now() -> chrono::DateTime<Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-07-16T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn run_with_tokens(tokens_in: u64, tokens_out: u64) -> WorkflowRun {
        WorkflowRun {
            id: "run-1".to_string(),
            workflow_name: "pipeline".to_string(),
            status: RunStatus::Success,
            budget_halt: None,
            nodes: vec![NodeState {
                id: "n".to_string(),
                name: "n".to_string(),
                status: RunStatus::Success,
                depends_on: vec![],
                input: None,
                output: None,
                error: None,
                tokens_in: Some(tokens_in),
                tokens_out: Some(tokens_out),
                model: None,
                started_at: None,
                elapsed_secs: None,
            }],
            started_at: Some("2026-07-16T00:00:00Z".to_string()),
            elapsed_secs: None,
        }
    }

    // ── tick: rendering always happens ──────────────────────────────────────

    #[test]
    fn tick_always_renders_the_summary() {
        let runs = vec![run_with_tokens(10, 5)];
        let outcome = tick(&runs, &Window::All, fixed_now(), &Budget::default(), None);
        assert!(outcome.rendered.contains("pipeline"));
        assert!(outcome.rendered.contains("TOTAL"));
    }

    // ── tick: no cap configured never alerts ────────────────────────────────

    #[test]
    fn tick_no_budget_never_alerts() {
        let runs = vec![run_with_tokens(1_000_000, 1_000_000)];
        let outcome = tick(&runs, &Window::All, fixed_now(), &Budget::default(), None);
        assert_eq!(outcome.alert, None);
        assert_eq!(outcome.verdict, GateVerdict::Within);
    }

    // ── tick: fresh crossing alerts exactly once ────────────────────────────

    #[test]
    fn tick_fresh_crossing_alerts() {
        let runs = vec![run_with_tokens(200, 0)];
        let budget = Budget {
            max_total_tokens: Some(100),
            max_cost_usd: None,
        };
        // previous = Within (or None) -> this tick breaches -> fresh alert.
        let outcome = tick(
            &runs,
            &Window::All,
            fixed_now(),
            &budget,
            Some(GateVerdict::Within),
        );
        assert!(outcome.alert.is_some());
        let reason = outcome.alert.unwrap();
        assert_eq!(reason.cap.as_str(), "max_total_tokens");
        assert_eq!(reason.spent, 200.0);
        assert_eq!(reason.limit, 100.0);
    }

    #[test]
    fn tick_first_tick_breach_is_fresh_alert() {
        let runs = vec![run_with_tokens(200, 0)];
        let budget = Budget {
            max_total_tokens: Some(100),
            max_cost_usd: None,
        };
        let outcome = tick(&runs, &Window::All, fixed_now(), &budget, None);
        assert!(outcome.alert.is_some());
    }

    // ── tick: sustained breach does not re-alert ────────────────────────────

    #[test]
    fn tick_sustained_breach_does_not_alert() {
        let runs = vec![run_with_tokens(200, 0)];
        let budget = Budget {
            max_total_tokens: Some(100),
            max_cost_usd: None,
        };
        let previous_breach = GateVerdict::Breached(super::super::budget::BreachReason {
            cap: super::super::budget::Cap::MaxTotalTokens,
            spent: 150.0,
            limit: 100.0,
        });
        let outcome = tick(
            &runs,
            &Window::All,
            fixed_now(),
            &budget,
            Some(previous_breach),
        );
        assert_eq!(outcome.alert, None);
        assert!(outcome.verdict.is_breached());
    }

    // ── tick: re-arm after dropping back below ──────────────────────────────

    #[test]
    fn tick_rearms_after_recovery_then_realerts() {
        let budget = Budget {
            max_total_tokens: Some(100),
            max_cost_usd: None,
        };

        // Tick 1: breach (fresh alert).
        let breach_runs = vec![run_with_tokens(200, 0)];
        let t1 = tick(&breach_runs, &Window::All, fixed_now(), &budget, None);
        assert!(t1.alert.is_some());

        // Tick 2: recovered (within budget) — no alert.
        let recovered_runs = vec![run_with_tokens(10, 0)];
        let t2 = tick(
            &recovered_runs,
            &Window::All,
            fixed_now(),
            &budget,
            Some(t1.verdict),
        );
        assert_eq!(t2.alert, None);
        assert_eq!(t2.verdict, GateVerdict::Within);

        // Tick 3: breaches again — must be reported fresh, not swallowed.
        let breach_again_runs = vec![run_with_tokens(300, 0)];
        let t3 = tick(
            &breach_again_runs,
            &Window::All,
            fixed_now(),
            &budget,
            Some(t2.verdict),
        );
        assert!(t3.alert.is_some());
    }

    // ── alert_message ────────────────────────────────────────────────────────

    #[test]
    fn alert_message_carries_cap_spent_limit() {
        let reason = super::super::budget::BreachReason {
            cap: super::super::budget::Cap::MaxCostUsd,
            spent: 12.5,
            limit: 10.0,
        };
        let msg = alert_message(&reason);
        assert!(msg.contains("max_cost_usd"), "got: {msg}");
        assert!(msg.contains("12.5"), "got: {msg}");
        assert!(
            msg.contains("10.0") || msg.contains("10.0000"),
            "got: {msg}"
        );
    }

    // ── tick: cost cap breach also works end-to-end ─────────────────────────

    #[test]
    fn tick_cost_cap_breach_via_pricing() {
        // Use a priced model so aggregate() computes a nonzero usd, then
        // configure a cap low enough to be breached.
        let run = WorkflowRun {
            id: "run-2".to_string(),
            workflow_name: "pipeline".to_string(),
            status: RunStatus::Success,
            budget_halt: None,
            nodes: vec![NodeState {
                id: "n".to_string(),
                name: "n".to_string(),
                status: RunStatus::Success,
                depends_on: vec![],
                input: None,
                output: None,
                error: None,
                tokens_in: Some(1_000_000),
                tokens_out: Some(1_000_000),
                model: Some("claude-opus-4-8".to_string()),
                started_at: None,
                elapsed_secs: None,
            }],
            started_at: Some("2026-07-16T00:00:00Z".to_string()),
            elapsed_secs: None,
        };
        let budget = Budget {
            max_total_tokens: None,
            max_cost_usd: Some(0.01),
        };
        let outcome = tick(&[run], &Window::All, fixed_now(), &budget, None);
        assert!(outcome.alert.is_some());
        assert_eq!(outcome.alert.unwrap().cap.as_str(), "max_cost_usd");
    }
}
