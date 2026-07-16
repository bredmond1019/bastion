// `bastion costs --last <window>` — LLM spend summary from PostgreSQL.

mod budget;
mod pricing;
mod tokens;
mod watch;

use anyhow::{Result, bail};
use chrono::{DateTime, Duration, Utc};

use crate::config::Config;
use crate::db::costs as db_costs;
use crate::db::workflows::{NodeState, WorkflowRun};

// ── Window ────────────────────────────────────────────────────────────────────

/// The time window for the cost summary.
#[derive(Debug, Clone, PartialEq)]
pub enum Window {
    /// Include runs whose `started_at` is within the last N days.
    Days(i64),
    /// Include all runs regardless of age.
    All,
}

/// Parse a window string. Accepts `"7d"`, `"30d"`, `"all"` (case-insensitive).
/// Returns an error for any other value.
pub fn parse_window(s: &str) -> Result<Window> {
    match s.to_ascii_lowercase().as_str() {
        "7d" => Ok(Window::Days(7)),
        "30d" => Ok(Window::Days(30)),
        "all" => Ok(Window::All),
        other => bail!(
            "unknown window '{}': expected one of '7d', '30d', 'all'",
            other
        ),
    }
}

/// Return `true` iff the run's `started_at` falls within `window` relative to `now`.
///
/// - `Window::All`  → always `true` (even for missing / unparseable `started_at`).
/// - `Window::Days(n)` → `true` iff `started_at` parses as RFC3339 **and** is
///   `>= now - n days`.  A missing or unparseable `started_at` is excluded.
///
/// `now` is injected as a parameter so the function stays pure and testable.
pub fn within_window(window: &Window, now: DateTime<Utc>, started_at: Option<&str>) -> bool {
    match window {
        Window::All => true,
        Window::Days(n) => {
            let cutoff = now - Duration::days(*n);
            match started_at {
                None => false,
                Some(s) => match DateTime::parse_from_rfc3339(s) {
                    Ok(dt) => dt.with_timezone(&Utc) >= cutoff,
                    Err(_) => false,
                },
            }
        }
    }
}

// ── Aggregation ───────────────────────────────────────────────────────────────

/// Per-workflow aggregated cost row.
#[derive(Debug, Clone)]
pub struct WorkflowCost {
    pub workflow_name: String,
    pub runs: u64,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub usd: f64,
}

/// Full cost summary including per-workflow rows, totals, and any unpriced models.
#[derive(Debug)]
pub struct CostSummary {
    /// One row per distinct `workflow_name`, sorted by `usd` descending.
    pub rows: Vec<WorkflowCost>,
    /// Totals across all rows.
    pub totals: WorkflowCost,
    /// Model IDs that appeared in the data but have no price entry.
    pub unpriced_models: Vec<String>,
}

/// Extract countable text from a node's `input`/`output` JSON value. String
/// values are used directly; other JSON shapes are serialized to their text
/// form. `None` means no countable text is present.
fn extract_text(value: &Option<serde_json::Value>) -> Option<String> {
    match value {
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(other) => Some(other.to_string()),
        None => None,
    }
}

/// Compute `(tokens_in, tokens_out)` for a node: exact tiktoken counts when
/// countable text and a model are present, falling back to the
/// orchestrator-reported `tokens_in`/`tokens_out` otherwise.
fn exact_or_reported_tokens(node: &NodeState) -> (u64, u64) {
    let tokens_in = match (&node.model, extract_text(&node.input)) {
        (Some(model), Some(text)) => tokens::count(&text, model) as u64,
        _ => node.tokens_in.unwrap_or(0),
    };
    let tokens_out = match (&node.model, extract_text(&node.output)) {
        (Some(model), Some(text)) => tokens::count(&text, model) as u64,
        _ => node.tokens_out.unwrap_or(0),
    };
    (tokens_in, tokens_out)
}

/// Aggregate `runs` into a `CostSummary`, filtered by `window` relative to `now`.
pub fn aggregate(runs: &[WorkflowRun], window: &Window, now: DateTime<Utc>) -> CostSummary {
    use std::collections::BTreeMap;

    let mut by_workflow: BTreeMap<String, WorkflowCost> = BTreeMap::new();
    let mut unpriced: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    for run in runs {
        if !within_window(window, now, run.started_at.as_deref()) {
            continue;
        }

        let entry = by_workflow
            .entry(run.workflow_name.clone())
            .or_insert_with(|| WorkflowCost {
                workflow_name: run.workflow_name.clone(),
                runs: 0,
                tokens_in: 0,
                tokens_out: 0,
                usd: 0.0,
            });

        entry.runs += 1;

        for node in &run.nodes {
            let (ti, to) = exact_or_reported_tokens(node);
            entry.tokens_in += ti;
            entry.tokens_out += to;

            if let Some(model) = &node.model {
                if pricing::price_for(model).is_none() {
                    unpriced.insert(model.clone());
                }
                entry.usd += pricing::cost_usd(model, ti, to);
            }
        }
    }

    // Sort rows by usd descending.
    let mut rows: Vec<WorkflowCost> = by_workflow.into_values().collect();
    rows.sort_by(|a, b| {
        b.usd
            .partial_cmp(&a.usd)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Compute totals.
    let totals = WorkflowCost {
        workflow_name: "TOTAL".to_string(),
        runs: rows.iter().map(|r| r.runs).sum(),
        tokens_in: rows.iter().map(|r| r.tokens_in).sum(),
        tokens_out: rows.iter().map(|r| r.tokens_out).sum(),
        usd: rows.iter().map(|r| r.usd).sum(),
    };

    CostSummary {
        rows,
        totals,
        unpriced_models: unpriced.into_iter().collect(),
    }
}

// ── Table rendering ───────────────────────────────────────────────────────────

const COL_WORKFLOW: usize = 30;
const COL_RUNS: usize = 6;
const COL_TOK_IN: usize = 12;
const COL_TOK_OUT: usize = 12;
const COL_USD: usize = 10;

/// Render the cost summary as a fixed-width table string.
/// Returns a `String` (caller is responsible for printing) so it is unit-testable.
pub fn render_table(summary: &CostSummary) -> String {
    let mut out = String::new();

    // Header
    out.push_str(&format!(
        "{:<COL_WORKFLOW$}  {:>COL_RUNS$}  {:>COL_TOK_IN$}  {:>COL_TOK_OUT$}  {:>COL_USD$}\n",
        "Workflow",
        "Runs",
        "Tokens In",
        "Tokens Out",
        "USD",
        COL_WORKFLOW = COL_WORKFLOW,
        COL_RUNS = COL_RUNS,
        COL_TOK_IN = COL_TOK_IN,
        COL_TOK_OUT = COL_TOK_OUT,
        COL_USD = COL_USD,
    ));

    // Separator
    let sep_width = COL_WORKFLOW + 2 + COL_RUNS + 2 + COL_TOK_IN + 2 + COL_TOK_OUT + 2 + COL_USD;
    out.push_str(&format!("{}\n", "-".repeat(sep_width)));

    // Data rows
    for row in &summary.rows {
        out.push_str(&render_row(row));
    }

    // Separator before totals
    out.push_str(&format!("{}\n", "=".repeat(sep_width)));

    // Totals
    out.push_str(&render_row(&summary.totals));

    // Unpriced notice
    if !summary.unpriced_models.is_empty() {
        out.push('\n');
        out.push_str("Note: the following models have no price entry (counted as $0.00):\n");
        for m in &summary.unpriced_models {
            out.push_str(&format!("  - {m}\n"));
        }
    }

    out
}

fn render_row(row: &WorkflowCost) -> String {
    // Truncate workflow name if it exceeds the column width.
    let name = if row.workflow_name.len() > COL_WORKFLOW {
        format!("{}…", &row.workflow_name[..COL_WORKFLOW - 1])
    } else {
        row.workflow_name.clone()
    };

    format!(
        "{:<COL_WORKFLOW$}  {:>COL_RUNS$}  {:>COL_TOK_IN$}  {:>COL_TOK_OUT$}  {:>COL_USD$}\n",
        name,
        row.runs,
        row.tokens_in,
        row.tokens_out,
        format!("${:.4}", row.usd),
        COL_WORKFLOW = COL_WORKFLOW,
        COL_RUNS = COL_RUNS,
        COL_TOK_IN = COL_TOK_IN,
        COL_TOK_OUT = COL_TOK_OUT,
        COL_USD = COL_USD,
    )
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Run `bastion costs --last <window>`.
///
/// Gracefully degrades when `DATABASE_URL` is missing or Postgres is unreachable —
/// prints an actionable message and returns `Ok(())` (no panic).
pub async fn run(window: String) -> Result<()> {
    let window = match parse_window(&window) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("bastion costs: {e}");
            return Ok(());
        }
    };

    let db_url = match Config::load() {
        Ok(cfg) => cfg.database_url,
        Err(_) => {
            eprintln!(
                "bastion costs: DATABASE_URL is not set.\n\
                 Set it in your .env file or environment:\n\
                 DATABASE_URL=postgres://user:pass@localhost:5432/dbname"
            );
            return Ok(());
        }
    };

    let runs = match db_costs::fetch_all_runs(&db_url).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("bastion costs: could not connect to database: {e}");
            eprintln!(
                "Make sure the Python orchestrator stack is running:\n\
                 cd ../python-orchestration-system && ./scripts/dev.sh"
            );
            return Ok(());
        }
    };

    let now = Utc::now();
    let summary = aggregate(&runs, &window, now);
    print!("{}", render_table(&summary));

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::workflows::{NodeState, RunStatus, WorkflowRun};

    // ── parse_window ──────────────────────────────────────────────────────────

    #[test]
    fn parse_window_7d() {
        assert_eq!(parse_window("7d").unwrap(), Window::Days(7));
    }

    #[test]
    fn parse_window_30d() {
        assert_eq!(parse_window("30d").unwrap(), Window::Days(30));
    }

    #[test]
    fn parse_window_all() {
        assert_eq!(parse_window("all").unwrap(), Window::All);
    }

    #[test]
    fn parse_window_case_insensitive() {
        assert_eq!(parse_window("ALL").unwrap(), Window::All);
        assert_eq!(parse_window("7D").unwrap(), Window::Days(7));
        assert_eq!(parse_window("30D").unwrap(), Window::Days(30));
    }

    #[test]
    fn parse_window_rejects_garbage() {
        assert!(parse_window("1y").is_err());
        assert!(parse_window("").is_err());
        assert!(parse_window("90d").is_err());
        assert!(parse_window("yesterday").is_err());
    }

    // ── within_window ─────────────────────────────────────────────────────────

    fn fixed_now() -> DateTime<Utc> {
        // 2026-06-22T00:00:00Z — use as the fixed "now" for boundary tests.
        DateTime::parse_from_rfc3339("2026-06-22T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn within_window_all_always_true() {
        assert!(within_window(&Window::All, fixed_now(), None));
        assert!(within_window(&Window::All, fixed_now(), Some("not-a-date")));
        assert!(within_window(
            &Window::All,
            fixed_now(),
            Some("2020-01-01T00:00:00Z")
        ));
    }

    #[test]
    fn within_window_days_in_window() {
        // 2026-06-20 is 2 days before now=2026-06-22 → inside 7d window
        assert!(within_window(
            &Window::Days(7),
            fixed_now(),
            Some("2026-06-20T10:00:00Z")
        ));
    }

    #[test]
    fn within_window_days_out_of_window() {
        // 2026-06-01 is 21 days before now=2026-06-22 → outside 7d window
        assert!(!within_window(
            &Window::Days(7),
            fixed_now(),
            Some("2026-06-01T00:00:00Z")
        ));
    }

    #[test]
    fn within_window_days_at_exact_boundary() {
        // exactly N days ago is inside the window (>= cutoff)
        let cutoff_str = "2026-06-15T00:00:00Z"; // 7 days before now
        assert!(within_window(
            &Window::Days(7),
            fixed_now(),
            Some(cutoff_str)
        ));
    }

    #[test]
    fn within_window_none_started_at_excluded_under_days() {
        assert!(!within_window(&Window::Days(7), fixed_now(), None));
    }

    #[test]
    fn within_window_unparseable_started_at_excluded_under_days() {
        assert!(!within_window(
            &Window::Days(7),
            fixed_now(),
            Some("not-a-date")
        ));
    }

    #[test]
    fn within_window_none_started_at_included_under_all() {
        assert!(within_window(&Window::All, fixed_now(), None));
    }

    // ── aggregate ─────────────────────────────────────────────────────────────

    fn make_node_with_usage(
        name: &str,
        model: Option<&str>,
        tokens_in: Option<u64>,
        tokens_out: Option<u64>,
    ) -> NodeState {
        NodeState {
            id: name.to_string(),
            name: name.to_string(),
            status: RunStatus::Success,
            depends_on: vec![],
            input: None,
            output: None,
            error: None,
            tokens_in,
            tokens_out,
            model: model.map(str::to_string),
            started_at: None,
            elapsed_secs: None,
        }
    }

    fn make_node_with_text(
        name: &str,
        model: Option<&str>,
        input: Option<&str>,
        output: Option<&str>,
        tokens_in: Option<u64>,
        tokens_out: Option<u64>,
    ) -> NodeState {
        NodeState {
            id: name.to_string(),
            name: name.to_string(),
            status: RunStatus::Success,
            depends_on: vec![],
            input: input.map(|s| serde_json::Value::String(s.to_string())),
            output: output.map(|s| serde_json::Value::String(s.to_string())),
            error: None,
            tokens_in,
            tokens_out,
            model: model.map(str::to_string),
            started_at: None,
            elapsed_secs: None,
        }
    }

    fn make_run(
        workflow_name: &str,
        started_at: Option<&str>,
        nodes: Vec<NodeState>,
    ) -> WorkflowRun {
        WorkflowRun {
            id: format!("run-{}", workflow_name),
            workflow_name: workflow_name.to_string(),
            status: RunStatus::Success,
            budget_halt: None,
            nodes,
            started_at: started_at.map(str::to_string),
            elapsed_secs: None,
        }
    }

    #[test]
    fn aggregate_counts_runs_and_sums_tokens() {
        let run1 = make_run(
            "rag_pipeline",
            Some("2026-06-20T10:00:00Z"),
            vec![
                make_node_with_usage(
                    "EmbeddingNode",
                    Some("text-embedding-3-small"),
                    Some(512),
                    Some(0),
                ),
                make_node_with_usage(
                    "LLMNode",
                    Some("claude-3-5-haiku-20241022"),
                    Some(2048),
                    Some(256),
                ),
            ],
        );
        let run2 = make_run(
            "rag_pipeline",
            Some("2026-06-21T08:00:00Z"),
            vec![make_node_with_usage(
                "EmbeddingNode",
                Some("text-embedding-3-small"),
                Some(1024),
                Some(0),
            )],
        );

        let now = fixed_now();
        let summary = aggregate(&[run1, run2], &Window::Days(7), now);

        assert_eq!(summary.rows.len(), 1, "one workflow type");
        let row = &summary.rows[0];
        assert_eq!(row.workflow_name, "rag_pipeline");
        assert_eq!(row.runs, 2);
        assert_eq!(row.tokens_in, 512 + 2048 + 1024, "sum of all input tokens");
        assert_eq!(row.tokens_out, 256, "sum of all output tokens");
    }

    #[test]
    fn aggregate_window_drops_out_of_range_run() {
        let old_run = make_run(
            "rag_pipeline",
            Some("2026-05-01T00:00:00Z"), // > 7 days ago
            vec![make_node_with_usage(
                "LLMNode",
                Some("claude-3-5-haiku-20241022"),
                Some(1000),
                Some(100),
            )],
        );
        let recent_run = make_run(
            "rag_pipeline",
            Some("2026-06-21T00:00:00Z"),
            vec![make_node_with_usage(
                "LLMNode",
                Some("claude-3-5-haiku-20241022"),
                Some(500),
                Some(50),
            )],
        );

        let now = fixed_now();
        let summary = aggregate(&[old_run, recent_run], &Window::Days(7), now);

        assert_eq!(summary.rows.len(), 1);
        assert_eq!(
            summary.rows[0].runs, 1,
            "only the recent run should be counted"
        );
        assert_eq!(summary.rows[0].tokens_in, 500);
    }

    #[test]
    fn aggregate_null_usage_nodes_counted_as_zero() {
        let run = make_run(
            "pipeline",
            Some("2026-06-20T00:00:00Z"),
            vec![
                make_node_with_usage("NoUsageNode", None, None, None),
                make_node_with_usage("LLMNode", Some("claude-haiku-4-5"), Some(1000), Some(100)),
            ],
        );

        let now = fixed_now();
        let summary = aggregate(&[run], &Window::All, now);

        assert_eq!(summary.rows[0].tokens_in, 1000, "None treated as 0");
        assert_eq!(summary.rows[0].tokens_out, 100, "None treated as 0");
    }

    #[test]
    fn aggregate_unknown_model_recorded_as_unpriced() {
        let run = make_run(
            "pipeline",
            Some("2026-06-20T00:00:00Z"),
            vec![make_node_with_usage(
                "UnknownNode",
                Some("gpt-4o-unknown"),
                Some(1000),
                Some(100),
            )],
        );

        let now = fixed_now();
        let summary = aggregate(&[run], &Window::All, now);

        assert!(
            summary
                .unpriced_models
                .contains(&"gpt-4o-unknown".to_string()),
            "unknown model must appear in unpriced_models"
        );
        // usd should be 0 for the unknown model
        assert_eq!(summary.rows[0].usd, 0.0);
    }

    #[test]
    fn aggregate_two_workflow_types_sorted_by_usd() {
        // cheap_pipeline uses haiku (cheap); expensive_pipeline uses opus (expensive).
        let cheap = make_run(
            "cheap_pipeline",
            Some("2026-06-20T00:00:00Z"),
            vec![make_node_with_usage(
                "node",
                Some("claude-haiku-4-5"),
                Some(1000),
                Some(100),
            )],
        );
        let expensive = make_run(
            "expensive_pipeline",
            Some("2026-06-20T00:00:00Z"),
            vec![make_node_with_usage(
                "node",
                Some("claude-opus-4-8"),
                Some(1_000_000),
                Some(1_000_000),
            )],
        );

        let now = fixed_now();
        let summary = aggregate(&[cheap, expensive], &Window::All, now);

        assert_eq!(summary.rows.len(), 2);
        // expensive_pipeline should be first (higher usd)
        assert_eq!(summary.rows[0].workflow_name, "expensive_pipeline");
        assert_eq!(summary.rows[1].workflow_name, "cheap_pipeline");
    }

    #[test]
    fn aggregate_totals_row_sums_all_rows() {
        let run1 = make_run(
            "pipe_a",
            Some("2026-06-20T00:00:00Z"),
            vec![make_node_with_usage(
                "n",
                Some("claude-haiku-4-5"),
                Some(1_000_000),
                Some(0),
            )],
        );
        let run2 = make_run(
            "pipe_b",
            Some("2026-06-20T00:00:00Z"),
            vec![make_node_with_usage(
                "n",
                Some("claude-haiku-4-5"),
                Some(0),
                Some(1_000_000),
            )],
        );

        let now = fixed_now();
        let summary = aggregate(&[run1, run2], &Window::All, now);

        assert_eq!(summary.totals.runs, 2);
        assert_eq!(summary.totals.tokens_in, 1_000_000);
        assert_eq!(summary.totals.tokens_out, 1_000_000);
        // pipe_a: 1.00 (input), pipe_b: 5.00 (output) → total $6.00
        assert!((summary.totals.usd - 6.00).abs() < 1e-6);
    }

    // ── exact token counts (extract_text / exact_or_reported_tokens) ───────────

    #[test]
    fn extract_text_string_value_used_directly() {
        let v = Some(serde_json::Value::String("hello".to_string()));
        assert_eq!(extract_text(&v), Some("hello".to_string()));
    }

    #[test]
    fn extract_text_non_string_serialized_to_text() {
        let v = Some(serde_json::json!({"a": 1}));
        assert_eq!(extract_text(&v), Some(r#"{"a":1}"#.to_string()));
    }

    #[test]
    fn extract_text_none_is_no_countable_text() {
        assert_eq!(extract_text(&None), None);
    }

    #[test]
    fn exact_or_reported_tokens_uses_exact_count_when_text_and_model_present() {
        let node = make_node_with_text(
            "n",
            Some("claude-haiku-4-5"),
            Some("Hello, world!"),
            Some("Hello, world!"),
            Some(999), // reported counts must be ignored in favor of the exact count
            Some(999),
        );
        let (ti, to) = exact_or_reported_tokens(&node);
        let expected = tokens::count("Hello, world!", "claude-haiku-4-5") as u64;
        assert_eq!(ti, expected);
        assert_eq!(to, expected);
        assert_ne!(ti, 999, "exact count must win over the reported fallback");
    }

    #[test]
    fn exact_or_reported_tokens_falls_back_when_no_text() {
        let node =
            make_node_with_text("n", Some("claude-haiku-4-5"), None, None, Some(42), Some(7));
        let (ti, to) = exact_or_reported_tokens(&node);
        assert_eq!(ti, 42, "no input text -> reported tokens_in fallback");
        assert_eq!(to, 7, "no output text -> reported tokens_out fallback");
    }

    #[test]
    fn exact_or_reported_tokens_falls_back_when_no_model() {
        let node = make_node_with_text("n", None, Some("Hello, world!"), None, Some(42), Some(7));
        let (ti, to) = exact_or_reported_tokens(&node);
        assert_eq!(
            ti, 42,
            "no model -> reported tokens_in fallback even with text"
        );
        assert_eq!(to, 7);
    }

    #[test]
    fn aggregate_node_with_text_matches_exact_tiktoken_count() {
        let run = make_run(
            "pipeline",
            Some("2026-06-20T00:00:00Z"),
            vec![make_node_with_text(
                "n",
                Some("claude-haiku-4-5"),
                Some("Hello, world!"),
                Some("Hello, world!"),
                Some(0), // deliberately wrong reported counts — must be ignored
                Some(0),
            )],
        );
        let now = fixed_now();
        let summary = aggregate(&[run], &Window::All, now);

        let expected = tokens::count("Hello, world!", "claude-haiku-4-5") as u64;
        assert_eq!(summary.rows[0].tokens_in, expected);
        assert_eq!(summary.rows[0].tokens_out, expected);
    }

    #[test]
    fn aggregate_node_without_text_uses_reported_counts() {
        let run = make_run(
            "pipeline",
            Some("2026-06-20T00:00:00Z"),
            vec![make_node_with_text(
                "n",
                Some("claude-haiku-4-5"),
                None,
                None,
                Some(500),
                Some(50),
            )],
        );
        let now = fixed_now();
        let summary = aggregate(&[run], &Window::All, now);

        assert_eq!(summary.rows[0].tokens_in, 500);
        assert_eq!(summary.rows[0].tokens_out, 50);
    }

    #[test]
    fn aggregate_mixed_text_and_no_text_nodes_sum_correctly() {
        let run = make_run(
            "pipeline",
            Some("2026-06-20T00:00:00Z"),
            vec![
                make_node_with_text(
                    "with_text",
                    Some("claude-haiku-4-5"),
                    Some("Hello, world!"),
                    Some("Hello, world!"),
                    Some(0),
                    Some(0),
                ),
                make_node_with_text(
                    "no_text",
                    Some("claude-haiku-4-5"),
                    None,
                    None,
                    Some(500),
                    Some(50),
                ),
            ],
        );
        let now = fixed_now();
        let summary = aggregate(&[run], &Window::All, now);

        let exact = tokens::count("Hello, world!", "claude-haiku-4-5") as u64;
        assert_eq!(summary.rows[0].tokens_in, exact + 500);
        assert_eq!(summary.rows[0].tokens_out, exact + 50);
    }

    #[test]
    fn aggregate_usd_computed_from_exact_counts() {
        let run = make_run(
            "pipeline",
            Some("2026-06-20T00:00:00Z"),
            vec![make_node_with_text(
                "n",
                Some("claude-haiku-4-5"),
                Some("Hello, world!"),
                Some("Hello, world!"),
                Some(0), // deliberately wrong reported counts — must be ignored
                Some(0),
            )],
        );
        let now = fixed_now();
        let summary = aggregate(&[run], &Window::All, now);

        let exact = tokens::count("Hello, world!", "claude-haiku-4-5") as u64;
        let expected_usd = pricing::cost_usd("claude-haiku-4-5", exact, exact);
        assert!(
            (summary.rows[0].usd - expected_usd).abs() < 1e-12,
            "expected {expected_usd}, got {}",
            summary.rows[0].usd
        );
    }

    // ── render_table ──────────────────────────────────────────────────────────

    #[test]
    fn render_table_contains_header() {
        let summary = aggregate(&[], &Window::All, fixed_now());
        let output = render_table(&summary);
        assert!(
            output.contains("Workflow"),
            "header must contain 'Workflow'"
        );
        assert!(output.contains("Runs"), "header must contain 'Runs'");
        assert!(
            output.contains("Tokens In"),
            "header must contain 'Tokens In'"
        );
        assert!(output.contains("USD"), "header must contain 'USD'");
        assert!(
            !output.contains("Est. USD"),
            "header must no longer label USD as an estimate"
        );
    }

    #[test]
    fn render_table_contains_totals_row() {
        let run = make_run(
            "rag_pipeline",
            Some("2026-06-20T00:00:00Z"),
            vec![make_node_with_usage(
                "n",
                Some("claude-haiku-4-5"),
                Some(1_000),
                Some(100),
            )],
        );
        let summary = aggregate(&[run], &Window::All, fixed_now());
        let output = render_table(&summary);
        assert!(output.contains("TOTAL"), "must have a TOTAL row");
    }

    #[test]
    fn render_table_contains_workflow_name() {
        let run = make_run(
            "rag_pipeline",
            Some("2026-06-20T00:00:00Z"),
            vec![make_node_with_usage(
                "n",
                Some("claude-haiku-4-5"),
                Some(1_000),
                Some(100),
            )],
        );
        let summary = aggregate(&[run], &Window::All, fixed_now());
        let output = render_table(&summary);
        assert!(
            output.contains("rag_pipeline"),
            "must contain the workflow name"
        );
    }

    #[test]
    fn render_table_notes_unpriced_models() {
        let run = make_run(
            "pipeline",
            Some("2026-06-20T00:00:00Z"),
            vec![make_node_with_usage(
                "n",
                Some("gpt-4-turbo"),
                Some(1000),
                Some(100),
            )],
        );
        let summary = aggregate(&[run], &Window::All, fixed_now());
        let output = render_table(&summary);
        assert!(
            output.contains("gpt-4-turbo"),
            "unpriced model must appear in the note"
        );
        assert!(output.contains("$0.00"), "unpriced note must mention $0.00");
    }

    // ── degrade branches ──────────────────────────────────────────────────────

    #[test]
    fn parse_window_bad_input_surfaces_clear_error() {
        let err = parse_window("invalid").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("unknown window"),
            "error should mention 'unknown window', got: {msg}"
        );
        assert!(
            msg.contains("invalid"),
            "error should echo the bad input, got: {msg}"
        );
    }
}
