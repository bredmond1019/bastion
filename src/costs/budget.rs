//! Pure budget threshold + gate evaluation (BA.7.C task 3).
//!
//! Zero I/O — no DB, no HTTP, no process spawn. This is the pure core the
//! rest of the block builds thin shells over (Rule 6, the established
//! `tmux.rs` construction-vs-execution split): `costs::watch` (task 6) feeds
//! each poll tick's [`Spend`] through [`evaluate`] and [`detect_crossing`] to
//! decide whether to emit an alert, and `run::mod` (task 8) calls [`evaluate`]
//! once before dispatch to decide whether to refuse a run.
//!
//! The shapes here are deliberately kept close to engine-rs's own budget
//! gate (`../engine-rs/crates/engine-core/src/budget.rs`'s `Budget` /
//! `BudgetLedger::check` / `BudgetHaltReason`) — vendored, not imported
//! (D24: bastion never takes an engine-rs crate dependency for logic), but
//! chosen so the Console's breach language matches what the Engine stamps
//! into `metadata.budget.reason` (contract v1.1.0 §5): `cap` one of
//! `"max_total_tokens"` | `"max_cost_usd"`, plus `spent` and `limit`.

use super::WorkflowCost;

/// Optional per-run spend caps — the two the canonical contract v1.1.0 §5
/// names as run-configuration supplied by the caller at trigger time.
///
/// `Budget::default()` (both `None`) means "no gate" — [`evaluate`] always
/// returns [`GateVerdict::Within`], preserving the absent-tolerant contract:
/// a run with no budget configured behaves exactly as it did before v1.1.0.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Budget {
    /// Halt once accumulated `tokens_in + tokens_out` reaches this cap.
    pub max_total_tokens: Option<u64>,
    /// Halt once accumulated cost (USD) reaches this cap.
    pub max_cost_usd: Option<f64>,
}

/// A current spend reading — the two numbers [`evaluate`] and
/// [`detect_crossing`] act on. Sourced from `CostSummary.totals`
/// (`costs::aggregate`); this type itself does no I/O.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Spend {
    /// Total tokens (`tokens_in + tokens_out`) across the aggregated rows.
    pub total_tokens: u64,
    /// Total USD cost across the aggregated rows.
    pub total_cost_usd: f64,
}

impl From<&WorkflowCost> for Spend {
    /// Build a [`Spend`] reading from a `CostSummary.totals` row — the
    /// glue `costs::watch` and `run::mod`'s thin shells use to feed the
    /// pure core here.
    fn from(totals: &WorkflowCost) -> Self {
        Spend {
            total_tokens: totals.tokens_in + totals.tokens_out,
            total_cost_usd: totals.usd,
        }
    }
}

/// Which cap a [`BreachReason`] names — mirrors the contract's
/// `metadata.budget.reason.cap` string values exactly, and engine-rs's own
/// `BudgetHaltReason::cap_name()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cap {
    MaxTotalTokens,
    MaxCostUsd,
}

impl Cap {
    /// The contract-friendly lowercase snake_case string, identical to what
    /// the Engine stamps into `metadata.budget.reason.cap`.
    pub fn as_str(self) -> &'static str {
        match self {
            Cap::MaxTotalTokens => "max_total_tokens",
            Cap::MaxCostUsd => "max_cost_usd",
        }
    }
}

impl std::fmt::Display for Cap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Detail of a breached cap — mirrors the contract's `metadata.budget.reason`
/// shape (`cap`, `spent`, `limit`). `spent`/`limit` are `f64` uniformly
/// (token counts widen losslessly for any realistic run) so both caps share
/// one shape here, even though the underlying values differ in kind.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BreachReason {
    pub cap: Cap,
    pub spent: f64,
    pub limit: f64,
}

/// The result of evaluating a [`Budget`] against a [`Spend`] reading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GateVerdict {
    /// No configured cap has been reached.
    Within,
    /// A configured cap has been reached; carries which one, plus the
    /// spent/limit values that tripped it.
    Breached(BreachReason),
}

impl GateVerdict {
    /// `true` iff this verdict is [`GateVerdict::Breached`].
    pub fn is_breached(&self) -> bool {
        matches!(self, GateVerdict::Breached(_))
    }
}

/// Evaluate `spend` against `budget`.
///
/// - `None` fields on `budget` are not enforced — a `Budget::default()`
///   always yields [`GateVerdict::Within`].
/// - Checks `max_total_tokens` before `max_cost_usd`, mirroring engine-rs's
///   `BudgetLedger::check` order.
/// - A cap is breached once spend **reaches** it (`>=`), matching
///   engine-rs's boundary exactly: "at the limit" counts as breached, not
///   merely approaching it. This is the explicit, decided-on boundary case.
pub fn evaluate(spend: Spend, budget: &Budget) -> GateVerdict {
    if let Some(limit) = budget.max_total_tokens
        && spend.total_tokens >= limit
    {
        return GateVerdict::Breached(BreachReason {
            cap: Cap::MaxTotalTokens,
            spent: spend.total_tokens as f64,
            limit: limit as f64,
        });
    }

    if let Some(limit) = budget.max_cost_usd
        && spend.total_cost_usd >= limit
    {
        return GateVerdict::Breached(BreachReason {
            cap: Cap::MaxCostUsd,
            spent: spend.total_cost_usd,
            limit,
        });
    }

    GateVerdict::Within
}

/// Edge-detection result for a single watch tick — the whole reason this
/// logic is pure and separately tested from the poll loop (task 6). An alert
/// should fire on [`Crossing::FreshBreach`] only; [`Crossing::SustainedBreach`]
/// must not re-alert while a breach merely persists.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Crossing {
    /// The previous tick was within budget (or this is the first tick with
    /// no prior reading) and this tick breached a cap — fire exactly one
    /// alert, carrying the breach detail.
    FreshBreach(BreachReason),
    /// Both the previous and current tick are breached — the same ongoing
    /// crossing; do not re-alert.
    SustainedBreach,
    /// This tick is within budget — whether the previous tick was too
    /// (steady state) or was breached (recovered, which re-arms: the next
    /// breach after this is reported as fresh again).
    Within,
}

/// Detect whether `current` constitutes a fresh crossing relative to
/// `previous`.
///
/// `previous: None` — the first tick, with nothing earlier to compare
/// against — treats a [`GateVerdict::Breached`] `current` as a fresh
/// crossing (there is no sustained state yet, so it must be reported).
pub fn detect_crossing(previous: Option<GateVerdict>, current: GateVerdict) -> Crossing {
    match (previous, current) {
        (_, GateVerdict::Within) => Crossing::Within,
        (Some(GateVerdict::Breached(_)), GateVerdict::Breached(_)) => Crossing::SustainedBreach,
        (_, GateVerdict::Breached(reason)) => Crossing::FreshBreach(reason),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ─── evaluate: max_total_tokens boundary ──────────────────────────────────

    #[test]
    fn tokens_below_limit_is_within() {
        let budget = Budget {
            max_total_tokens: Some(1000),
            max_cost_usd: None,
        };
        let spend = Spend {
            total_tokens: 999,
            total_cost_usd: 0.0,
        };
        assert_eq!(evaluate(spend, &budget), GateVerdict::Within);
    }

    #[test]
    fn tokens_exactly_at_limit_is_breached() {
        let budget = Budget {
            max_total_tokens: Some(1000),
            max_cost_usd: None,
        };
        let spend = Spend {
            total_tokens: 1000,
            total_cost_usd: 0.0,
        };
        let verdict = evaluate(spend, &budget);
        assert_eq!(
            verdict,
            GateVerdict::Breached(BreachReason {
                cap: Cap::MaxTotalTokens,
                spent: 1000.0,
                limit: 1000.0,
            })
        );
    }

    #[test]
    fn tokens_above_limit_is_breached() {
        let budget = Budget {
            max_total_tokens: Some(1000),
            max_cost_usd: None,
        };
        let spend = Spend {
            total_tokens: 1500,
            total_cost_usd: 0.0,
        };
        let verdict = evaluate(spend, &budget);
        assert_eq!(
            verdict,
            GateVerdict::Breached(BreachReason {
                cap: Cap::MaxTotalTokens,
                spent: 1500.0,
                limit: 1000.0,
            })
        );
    }

    // ─── evaluate: max_cost_usd boundary ───────────────────────────────────────

    #[test]
    fn cost_below_limit_is_within() {
        let budget = Budget {
            max_total_tokens: None,
            max_cost_usd: Some(10.0),
        };
        let spend = Spend {
            total_tokens: 0,
            total_cost_usd: 9.99,
        };
        assert_eq!(evaluate(spend, &budget), GateVerdict::Within);
    }

    #[test]
    fn cost_exactly_at_limit_is_breached() {
        let budget = Budget {
            max_total_tokens: None,
            max_cost_usd: Some(10.0),
        };
        let spend = Spend {
            total_tokens: 0,
            total_cost_usd: 10.0,
        };
        let verdict = evaluate(spend, &budget);
        assert_eq!(
            verdict,
            GateVerdict::Breached(BreachReason {
                cap: Cap::MaxCostUsd,
                spent: 10.0,
                limit: 10.0,
            })
        );
    }

    #[test]
    fn cost_above_limit_is_breached() {
        let budget = Budget {
            max_total_tokens: None,
            max_cost_usd: Some(10.0),
        };
        let spend = Spend {
            total_tokens: 0,
            total_cost_usd: 25.5,
        };
        let verdict = evaluate(spend, &budget);
        assert_eq!(
            verdict,
            GateVerdict::Breached(BreachReason {
                cap: Cap::MaxCostUsd,
                spent: 25.5,
                limit: 10.0,
            })
        );
    }

    // ─── evaluate: no cap / single cap / both caps ─────────────────────────────

    #[test]
    fn no_cap_configured_is_always_within() {
        let budget = Budget::default();
        let spend = Spend {
            total_tokens: u64::MAX,
            total_cost_usd: f64::MAX,
        };
        assert_eq!(evaluate(spend, &budget), GateVerdict::Within);
    }

    #[test]
    fn only_tokens_cap_configured_cost_never_evaluated() {
        // Cost is astronomically high but no max_cost_usd cap is configured —
        // must not spuriously breach.
        let budget = Budget {
            max_total_tokens: Some(1_000_000),
            max_cost_usd: None,
        };
        let spend = Spend {
            total_tokens: 10,
            total_cost_usd: 1_000_000.0,
        };
        assert_eq!(evaluate(spend, &budget), GateVerdict::Within);
    }

    #[test]
    fn only_cost_cap_configured_tokens_never_evaluated() {
        let budget = Budget {
            max_total_tokens: None,
            max_cost_usd: Some(1.0),
        };
        let spend = Spend {
            total_tokens: u64::MAX,
            total_cost_usd: 0.5,
        };
        assert_eq!(evaluate(spend, &budget), GateVerdict::Within);
    }

    #[test]
    fn both_caps_configured_only_tokens_breached() {
        let budget = Budget {
            max_total_tokens: Some(100),
            max_cost_usd: Some(50.0),
        };
        let spend = Spend {
            total_tokens: 200,
            total_cost_usd: 10.0,
        };
        let verdict = evaluate(spend, &budget);
        assert_eq!(
            verdict,
            GateVerdict::Breached(BreachReason {
                cap: Cap::MaxTotalTokens,
                spent: 200.0,
                limit: 100.0,
            })
        );
    }

    #[test]
    fn both_caps_configured_only_cost_breached() {
        let budget = Budget {
            max_total_tokens: Some(1_000_000),
            max_cost_usd: Some(5.0),
        };
        let spend = Spend {
            total_tokens: 10,
            total_cost_usd: 7.5,
        };
        let verdict = evaluate(spend, &budget);
        assert_eq!(
            verdict,
            GateVerdict::Breached(BreachReason {
                cap: Cap::MaxCostUsd,
                spent: 7.5,
                limit: 5.0,
            })
        );
    }

    #[test]
    fn both_caps_configured_neither_breached() {
        let budget = Budget {
            max_total_tokens: Some(1_000_000),
            max_cost_usd: Some(50.0),
        };
        let spend = Spend {
            total_tokens: 10,
            total_cost_usd: 1.0,
        };
        assert_eq!(evaluate(spend, &budget), GateVerdict::Within);
    }

    #[test]
    fn both_caps_breached_tokens_reported_first() {
        // Check order mirrors engine-rs: max_total_tokens checked before
        // max_cost_usd, so when both are breached simultaneously the token
        // cap is the one named.
        let budget = Budget {
            max_total_tokens: Some(100),
            max_cost_usd: Some(5.0),
        };
        let spend = Spend {
            total_tokens: 200,
            total_cost_usd: 10.0,
        };
        let verdict = evaluate(spend, &budget);
        assert_eq!(
            verdict,
            GateVerdict::Breached(BreachReason {
                cap: Cap::MaxTotalTokens,
                spent: 200.0,
                limit: 100.0,
            })
        );
    }

    // ─── Cap::as_str / Display ──────────────────────────────────────────────

    #[test]
    fn cap_as_str_matches_contract_strings() {
        assert_eq!(Cap::MaxTotalTokens.as_str(), "max_total_tokens");
        assert_eq!(Cap::MaxCostUsd.as_str(), "max_cost_usd");
    }

    #[test]
    fn cap_display_matches_as_str() {
        assert_eq!(Cap::MaxTotalTokens.to_string(), "max_total_tokens");
        assert_eq!(Cap::MaxCostUsd.to_string(), "max_cost_usd");
    }

    // ─── GateVerdict::is_breached ────────────────────────────────────────────

    #[test]
    fn is_breached_true_for_breached_variant() {
        let verdict = GateVerdict::Breached(BreachReason {
            cap: Cap::MaxTotalTokens,
            spent: 10.0,
            limit: 5.0,
        });
        assert!(verdict.is_breached());
    }

    #[test]
    fn is_breached_false_for_within_variant() {
        assert!(!GateVerdict::Within.is_breached());
    }

    // ─── Spend::from(&WorkflowCost) ──────────────────────────────────────────

    #[test]
    fn spend_from_workflow_cost_sums_tokens_in_and_out() {
        let totals = WorkflowCost {
            workflow_name: "total".to_string(),
            runs: 3,
            tokens_in: 100,
            tokens_out: 50,
            usd: 1.23,
        };
        let spend = Spend::from(&totals);
        assert_eq!(spend.total_tokens, 150);
        assert_eq!(spend.total_cost_usd, 1.23);
    }

    // ─── detect_crossing: the four steady/edge transitions ───────────────────

    fn within() -> GateVerdict {
        GateVerdict::Within
    }

    fn breached() -> GateVerdict {
        GateVerdict::Breached(BreachReason {
            cap: Cap::MaxTotalTokens,
            spent: 200.0,
            limit: 100.0,
        })
    }

    #[test]
    fn crossing_below_to_below_is_within_no_alert() {
        let result = detect_crossing(Some(within()), within());
        assert_eq!(result, Crossing::Within);
    }

    #[test]
    fn crossing_below_to_above_is_fresh_breach() {
        let result = detect_crossing(Some(within()), breached());
        assert!(matches!(result, Crossing::FreshBreach(_)));
    }

    #[test]
    fn crossing_above_to_above_is_sustained_no_alert() {
        let result = detect_crossing(Some(breached()), breached());
        assert_eq!(result, Crossing::SustainedBreach);
    }

    #[test]
    fn crossing_above_to_below_is_within_re_arms() {
        let result = detect_crossing(Some(breached()), within());
        assert_eq!(result, Crossing::Within);
    }

    #[test]
    fn crossing_first_tick_within_is_within() {
        let result = detect_crossing(None, within());
        assert_eq!(result, Crossing::Within);
    }

    #[test]
    fn crossing_first_tick_breached_is_fresh_breach() {
        // No prior reading to compare against — a breach on the very first
        // tick must still be reported, not silently swallowed.
        let result = detect_crossing(None, breached());
        assert!(matches!(result, Crossing::FreshBreach(_)));
    }

    #[test]
    fn crossing_fresh_breach_carries_the_breach_reason() {
        let result = detect_crossing(Some(within()), breached());
        match result {
            Crossing::FreshBreach(reason) => {
                assert_eq!(reason.cap, Cap::MaxTotalTokens);
                assert_eq!(reason.spent, 200.0);
                assert_eq!(reason.limit, 100.0);
            }
            other => panic!("expected FreshBreach, got {other:?}"),
        }
    }

    #[test]
    fn crossing_re_arms_after_recovery_then_re_breaches() {
        // Simulates a full watch sequence: within -> breach (fresh) ->
        // recovered (within) -> breach again (fresh, not sustained).
        let t1 = detect_crossing(None, within());
        assert_eq!(t1, Crossing::Within);

        let t2 = detect_crossing(Some(within()), breached());
        assert!(matches!(t2, Crossing::FreshBreach(_)));

        let t3 = detect_crossing(Some(breached()), within());
        assert_eq!(t3, Crossing::Within);

        let t4 = detect_crossing(Some(within()), breached());
        assert!(matches!(t4, Crossing::FreshBreach(_)));
    }
}
