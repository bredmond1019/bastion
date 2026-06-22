// PostgreSQL queries for LLM cost data (Phase 2+).
// Observer only (D2): read-only access to the Python orchestrator's DB.

use anyhow::{Context, Result};
use sqlx::postgres::PgPoolOptions;

use super::workflows::{EventRow, WorkflowRun, parse_event_row};

/// Fetch all workflow runs from the `events` table.
///
/// This is the same underlying table as `db::workflows`, but fetches **all**
/// rows (active + completed) for cost aggregation.  Read-only; never writes (D2).
pub async fn fetch_all_runs(db_url: &str) -> Result<Vec<WorkflowRun>> {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(db_url)
        .await
        .context("failed to connect to PostgreSQL")?;

    let rows = sqlx::query_as::<_, EventRow>("SELECT id, workflow_type, task_context FROM events")
        .fetch_all(&pool)
        .await
        .context("failed to query events table")?;

    let mut runs = Vec::with_capacity(rows.len());
    for row in rows {
        let run = parse_event_row(row)?;
        runs.push(run);
    }
    Ok(runs)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── integration stub (requires a live DB; skipped in CI) ─────────────────
    //
    // Run with:
    //   BASTION_INTEGRATION_TEST=1 cargo test -- --ignored
    //
    // Verifies that fetch_all_runs returns a Vec<WorkflowRun> against the live
    // Python orchestrator's PostgreSQL instance.

    #[tokio::test]
    #[ignore]
    async fn integration_fetch_all_runs_returns_vec() {
        if std::env::var("BASTION_INTEGRATION_TEST").is_err() {
            return;
        }
        let db_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
        let runs = fetch_all_runs(&db_url)
            .await
            .expect("fetch_all_runs should not error against a live DB");
        // Just confirm the return type; count may be 0 in a clean DB.
        let _: Vec<WorkflowRun> = runs;
    }
}
