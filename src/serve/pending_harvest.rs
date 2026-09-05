//! Reader for the operator-approval queue's production source:
//! `PersistToBrainNode` results carrying a deferred `pending` harvest
//! record, stored inside a completed `CONTENT_PIPELINE` run's node result
//! (`BA.ticket.wire-approve-and-run-drain-trigger`, task 1).
//!
//! **Design decision (settled by the block record, not to be revisited
//! here):** the reader lives in bastion (option A) as a periodic sweep
//! against the `events` table bastion already reads read-only per D2 —
//! not as a new engine-rs lister (option B). See
//! `planning/blocks/BA.ticket.wire-approve-and-run-drain-trigger.json`.
//!
//! # Shape provenance
//!
//! **No live row was observed.** The 2026-09-04 measurement against the
//! live `orchestration_dev` Postgres (`events` table, 30,734 rows, node
//! results carried under `task_context->'nodes'`, not `data`) found `0`
//! rows with a `PersistToBrainNode` key anywhere in `task_context` — the
//! producer has never run against that database. Both control queries
//! (an unqualified `PersistToBrainNode` lookup, and the 87 near-miss rows
//! containing the string `"pending"` that turned out to belong to
//! unrelated SDLC bookkeeping nodes) ruled out a broken instrument, so
//! this is a real negative, not a missed row.
//!
//! With no live row available, the envelope this module parses is derived
//! from the producer's own source instead (source 2 of the two allowed by
//! the task spec):
//!
//! - `engine-core/src/nodes/harvest_gate.rs`'s `pending_harvest_record`
//!   constructs exactly the four keys `{artifact_id, url, payload,
//!   doc_paths}` (pinned by that file's own
//!   `pending_harvest_record_has_exactly_the_four_documented_keys` test).
//! - `engine-core/src/workflows/content_pipeline/persist_to_brain.rs`'s
//!   `HarvestDecision::Defer` arm stores that record under a `pending` key
//!   inside a `PersistToBrainNode` node result shaped
//!   `{"posted": false, "skipped": true, "harvest_mode": .., "status":
//!   null, "artifact_id": .., "response": null, "pending": <record>}`.
//!
//! `tests/fixtures/pending_harvest_event_row.json` records that envelope
//! verbatim (one `PersistToBrainNode` node-result value, as it would be
//! found under `task_context->'nodes'->'PersistToBrainNode'` in an
//! `events` row) so this module's tests and the recorded shape can never
//! drift apart.

use chrono::{DateTime, Utc};
use engine_core::workflows::approve_and_run::{
    ApproveAndRunSeams, PendingHarvestRecord, gate_id_for,
};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

/// Parse the raw `PersistToBrainNode` node-result values a query over the
/// `events` table returns into deduped [`PendingHarvestRecord`]s.
///
/// Each element of `rows` is one node-result JSON object as stored by
/// `persist_to_brain.rs`'s `Defer` arm — see the module doc comment for the
/// exact shape. A row with no `pending` key (or a `pending` that is
/// JSON `null`) contributes nothing; a `pending` object present but
/// malformed (wrong types, a missing `artifact_id`, etc.) is skipped
/// rather than panicking or aborting the rest of the batch — one bad row
/// must never lose the good rows beside it. Rows sharing the same
/// `artifact_id` are deduped, keeping the first occurrence.
pub(crate) fn parse_pending_records(rows: &[Value]) -> Vec<PendingHarvestRecord> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();

    for row in rows {
        let pending = match row.get("pending") {
            Some(value) if !value.is_null() => value,
            _ => continue,
        };
        let record = match PendingHarvestRecord::from_value(pending) {
            Ok(record) => record,
            Err(_) => continue,
        };
        if seen.insert(record.artifact_id.clone()) {
            out.push(record);
        }
    }

    out
}

// ─────────────────────────────────────────────────────────────────────────
// SQL shell + sweep tick (task 2)
// ─────────────────────────────────────────────────────────────────────────
//
// The pure core above (`parse_pending_records`) never touches SQL. Everything
// below it is the thin I/O shell over that core, per standing rule 6: a
// narrow read-only query, and a periodic tick that degrades (never panics,
// never aborts the loop) exactly as the sibling
// `engine_serve::orphan::spawn_stale_run_sweep` does.

/// Query the `events` table (bastion reads it read-only per D2 — this is a
/// `SELECT` and nothing else, no DDL, no migration, no dedicated
/// pending-harvest table per the block record's `out_of_scope`) for every
/// `PersistToBrainNode` node result that carries a non-null `pending` key.
///
/// Node results live under `task_context -> 'nodes' -> '<NodeName>'`, not
/// under `data` — see the module doc comment's shape-provenance note, which
/// records the 2026-09-04 measurement that pinned this column. Each
/// returned [`Value`] is one such node-result object, in exactly the shape
/// [`parse_pending_records`] expects and `tests/fixtures/pending_harvest_event_row.json`
/// records.
pub(crate) async fn query_pending_rows(pool: &sqlx::PgPool) -> Result<Vec<Value>, sqlx::Error> {
    sqlx::query_scalar::<_, Value>(
        "SELECT task_context -> 'nodes' -> 'PersistToBrainNode' \
         FROM events \
         WHERE task_context -> 'nodes' -> 'PersistToBrainNode' -> 'pending' IS NOT NULL",
    )
    .fetch_all(pool)
    .await
}

/// The tick's parse-and-drain step, kept separate from [`sweep_once`]'s
/// query call so it can be driven directly with fixture `rows` in tests
/// rather than a live query.
///
/// Parses `rows` (task 1's [`parse_pending_records`]), then — **idempotence
/// is the contract, asserted explicitly by this module's tests** —
/// filters out any record already open on `seams`'s queue under its
/// `gate_id` before draining. `ApproveAndRunSeams::drain` keys its
/// *pending-record store* by `gate_id_for(artifact_id)` deterministically,
/// but the underlying `OperatorQueue::enqueue` does not itself dedupe by
/// item id — draining the same record twice would push two
/// `OperatorQueueItem`s with the same `gate_id` onto the queue. Checking
/// `seams.lookup_pending` first is what makes a second sweep tick over the
/// same rows a no-op instead of re-paging the operator for one artifact.
///
/// Returns the number of records freshly drained this tick (`0` when
/// nothing was new).
///
/// `pub(crate)` (not module-private) so `src/serve/mod.rs`'s
/// `approve_and_run_seams_wiring_tests` can seed records through this same
/// production entry point — the sweep's parse-and-drain step, one layer
/// below the SQL query — rather than calling `ApproveAndRunSeams::drain`
/// directly, which would prove nothing this block did not already have
/// (`BA.ticket.wire-approve-and-run-drain-trigger` task 3).
pub(crate) fn drain_fresh_records(
    rows: &[Value],
    seams: &ApproveAndRunSeams,
    now: DateTime<Utc>,
) -> usize {
    let records = parse_pending_records(rows);
    let fresh: Vec<PendingHarvestRecord> = records
        .into_iter()
        .filter(|record| {
            seams
                .lookup_pending(&gate_id_for(&record.artifact_id))
                .is_none()
        })
        .collect();

    if fresh.is_empty() {
        return 0;
    }

    let count = fresh.len();
    seams.drain(&fresh, now);
    count
}

/// One sweep tick: query `pool` for pending rows and drain whatever is new
/// into `seams`. `now` is a parameter, never read from the clock inside
/// this function's own logic, so a test can drive a tick deterministically
/// with no sleeping.
///
/// Degrades rather than fails, mirroring the sibling stale-run sweep's
/// contract exactly: a query error is logged at `warn` and this tick is
/// skipped — never a panic, never a loop abort. `bastion serve` must still
/// boot and keep ticking with the database unreachable.
pub(crate) async fn sweep_once(
    pool: &sqlx::PgPool,
    seams: &ApproveAndRunSeams,
    now: DateTime<Utc>,
) {
    let rows = match query_pending_rows(pool).await {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(
                target: "bastion::serve",
                error = %err,
                "pending-harvest sweep: query failed, skipping this tick"
            );
            return;
        }
    };

    drain_fresh_records(&rows, seams, now);
}

/// A handle to the background pending-harvest sweep loop
/// [`spawn_pending_harvest_sweep`] spawned. Mirrors
/// `engine_serve::orphan::StaleRunSweepHandle`'s "hold or drop" shape: the
/// caller may [`abort`](Self::abort) the loop (e.g. on shutdown), or drop
/// it — dropping does **not** stop a spawned loop (a `tokio::task::JoinHandle`
/// detaches on drop). `pool: None` never spawns a loop at all (see
/// [`spawn_pending_harvest_sweep`]'s doc comment), so `task` is `None` in
/// that case and [`abort`](Self::abort) is a no-op.
pub(crate) struct PendingHarvestSweepHandle {
    task: Option<tokio::task::JoinHandle<()>>,
}

impl PendingHarvestSweepHandle {
    /// Stop the background sweep loop, if one was spawned.
    pub(crate) fn abort(&self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

/// The spawnable pending-harvest sweep bootstrap, modeled directly on
/// `engine_serve::orphan::spawn_stale_run_sweep`: `tokio::spawn` a
/// background loop that calls [`sweep_once`] on every tick of a
/// `tokio::time::interval(interval)`.
///
/// `pool: None` — the same DB-free boot `spawn_durable_writer(None)`
/// already establishes elsewhere in this file's tests — self-skips: this
/// logs once at `info` and never spawns a querying loop at all, so
/// `bastion serve` still boots with the database down. `seams` is expected
/// to be the SAME `Arc<ApproveAndRunSeams>` the notify poll loop and
/// `lookup_pending` composition already share (wiring this at the correct
/// call site is task 3's job, not this one).
pub(crate) fn spawn_pending_harvest_sweep(
    pool: Option<sqlx::PgPool>,
    seams: Arc<ApproveAndRunSeams>,
    interval: Duration,
) -> PendingHarvestSweepHandle {
    let Some(pool) = pool else {
        tracing::info!(
            target: "bastion::serve",
            "pending-harvest sweep: no DATABASE_URL configured, self-skipping"
        );
        return PendingHarvestSweepHandle { task: None };
    };

    let task = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            sweep_once(&pool, &seams, Utc::now()).await;
        }
    });

    PendingHarvestSweepHandle { task: Some(task) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The confirmed envelope, loaded from the tracked fixture so the test
    /// and the recorded shape can never drift apart.
    fn well_formed_row() -> Value {
        serde_json::from_str(include_str!(
            "../../tests/fixtures/pending_harvest_event_row.json"
        ))
        .expect("fixture is valid JSON")
    }

    /// A fresh `ApproveAndRunSeams` over an empty queue and a
    /// `FileApprovalLedger` pointed at a throwaway tempdir path — mirrors
    /// `src/serve/mod.rs`'s `approve_and_run_seams_wiring_tests::seams()`
    /// exactly, so this module's tests construct seams the same way the
    /// production wiring (task 3) and the existing wiring tests do.
    fn seams() -> ApproveAndRunSeams {
        use engine_core::nodes::http_post::StubHttpPost;
        use engine_core::operator::OperatorPayloadLimits;
        use engine_core::operator::ledger::FileApprovalLedger;
        use engine_core::operator::queue::{OperatorQueue, OperatorQueuePolicy};
        use engine_core::workflows::approve_and_run::ApproveAndRunPolicy;
        use std::sync::Mutex;

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

    #[test]
    fn parses_a_well_formed_record() {
        let rows = vec![well_formed_row()];
        let records = parse_pending_records(&rows);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].artifact_id, "artifact-123");
        assert_eq!(
            records[0].url,
            "https://synapse.example/ingest/learning-artifact"
        );
        assert_eq!(records[0].doc_paths, vec!["docs/foo.md".to_string()]);
    }

    #[test]
    fn a_row_with_no_pending_key_yields_nothing_and_does_not_error() {
        let row = serde_json::json!({
            "posted": true,
            "skipped": false,
            "harvest_mode": "in_process",
        });
        let records = parse_pending_records(&[row]);
        assert!(records.is_empty());
    }

    #[test]
    fn a_row_with_a_null_pending_yields_nothing() {
        let mut row = well_formed_row();
        row.as_object_mut()
            .expect("fixture is an object")
            .insert("pending".to_string(), Value::Null);
        let records = parse_pending_records(&[row]);
        assert!(records.is_empty());
    }

    #[test]
    fn a_malformed_pending_is_skipped_but_surrounding_well_formed_rows_still_parse() {
        let mut malformed = well_formed_row();
        // Drop the required `artifact_id` key from the nested `pending`
        // object so it fails to deserialize as a `PendingHarvestRecord`.
        malformed
            .get_mut("pending")
            .and_then(Value::as_object_mut)
            .expect("pending is an object")
            .remove("artifact_id");

        let mut second = well_formed_row();
        second["pending"]["artifact_id"] = serde_json::json!("artifact-456");

        let rows = vec![well_formed_row(), malformed, second];
        let records = parse_pending_records(&rows);

        assert_eq!(records.len(), 2);
        let ids: Vec<&str> = records.iter().map(|r| r.artifact_id.as_str()).collect();
        assert!(ids.contains(&"artifact-123"));
        assert!(ids.contains(&"artifact-456"));
    }

    #[test]
    fn two_rows_with_the_same_artifact_id_collapse_to_one() {
        let rows = vec![well_formed_row(), well_formed_row()];
        let records = parse_pending_records(&rows);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].artifact_id, "artifact-123");
    }

    #[test]
    fn an_empty_input_yields_an_empty_vec() {
        let records = parse_pending_records(&[]);
        assert!(records.is_empty());
    }

    // ── task 2: SQL shell + sweep tick ──────────────────────────────────

    #[test]
    fn drain_fresh_records_drains_a_well_formed_row_into_the_seams_queue() {
        let seams = seams();
        let rows = vec![well_formed_row()];

        let drained = drain_fresh_records(&rows, &seams, Utc::now());

        assert_eq!(drained, 1);
        assert!(seams.lookup_pending(&gate_id_for("artifact-123")).is_some());
    }

    #[test]
    fn drain_fresh_records_holds_one_item_per_distinct_artifact_id() {
        // `ApproveAndRunSeams::drain` makes exactly ONE `next_deliverable`
        // call per invocation (its own doc comment), so a single batch of
        // two records only ever opens one of them regardless of queue
        // depth. Two SEPARATE ticks — one per record, as a real sweep would
        // actually see them arrive — is what exercises "one item per
        // distinct artifact_id" landing on the queue; `operator_queue_depth`
        // is raised to 2 (`OperatorQueuePolicy::default()`'s is `1`, §7.5
        // Invariant 3) so the second tick's item can open alongside the
        // first rather than being displaced by it.
        use engine_core::nodes::http_post::StubHttpPost;
        use engine_core::operator::OperatorPayloadLimits;
        use engine_core::operator::ledger::FileApprovalLedger;
        use engine_core::operator::queue::{OperatorQueue, OperatorQueuePolicy};
        use engine_core::workflows::approve_and_run::ApproveAndRunPolicy;
        use std::sync::Mutex;

        let dir = tempfile::tempdir().expect("tempdir");
        let seams = ApproveAndRunSeams::new(
            Arc::new(Mutex::new(OperatorQueue::new(OperatorQueuePolicy {
                operator_queue_depth: 2,
                ..OperatorQueuePolicy::default()
            }))),
            Arc::new(FileApprovalLedger::new(dir.path().join("ledger.jsonl"))),
            Arc::new(StubHttpPost::succeeding(serde_json::json!({"ok": true}))),
            OperatorPayloadLimits::default(),
            ApproveAndRunPolicy::default(),
        );

        let mut second_row = well_formed_row();
        second_row["pending"]["artifact_id"] = serde_json::json!("artifact-456");

        let first_tick = drain_fresh_records(&[well_formed_row()], &seams, Utc::now());
        let second_tick = drain_fresh_records(&[second_row], &seams, Utc::now());

        assert_eq!(first_tick, 1);
        assert_eq!(second_tick, 1);
        assert!(seams.lookup_pending(&gate_id_for("artifact-123")).is_some());
        assert!(seams.lookup_pending(&gate_id_for("artifact-456")).is_some());
    }

    /// Idempotence is the difference between a working queue and an
    /// operator being paged repeatedly for one artifact — asserted
    /// explicitly per the block record's acceptance criteria. Running the
    /// parse-and-drain step twice over the SAME rows must leave exactly
    /// one queue item, not two: the second call drains 0 fresh records
    /// because `seams.lookup_pending` already resolves the gate from the
    /// first call.
    #[test]
    fn drain_fresh_records_run_twice_over_the_same_rows_drains_nothing_the_second_time() {
        let seams = seams();
        let rows = vec![well_formed_row()];
        let now = Utc::now();

        let first = drain_fresh_records(&rows, &seams, now);
        let second = drain_fresh_records(&rows, &seams, now);

        assert_eq!(first, 1);
        assert_eq!(
            second, 0,
            "a repeat tick over the same rows must not re-enqueue"
        );
        assert!(seams.lookup_pending(&gate_id_for("artifact-123")).is_some());
    }

    #[test]
    fn drain_fresh_records_with_no_rows_drains_nothing() {
        let seams = seams();
        let drained = drain_fresh_records(&[], &seams, Utc::now());
        assert_eq!(drained, 0);
    }

    /// `connect_lazy` never touches the network at construction time — the
    /// pool object is built eagerly but the actual TCP connect is deferred
    /// to first use — so this test needs no live Postgres and no
    /// `DATABASE_URL`; nothing listens on `127.0.0.1:1`, so the sweep's
    /// only query fails with a connection error, exercising the
    /// degrade-on-query-error contract without a database dependency. A
    /// short `acquire_timeout` keeps this test's own failure fast rather
    /// than waiting out sqlx's 30s pool default.
    #[tokio::test]
    async fn sweep_once_on_a_query_error_skips_the_tick_without_panicking() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(200))
            .connect_lazy("postgres://nobody:nobody@127.0.0.1:1/nonexistent")
            .expect("connect_lazy never touches the network");
        let seams = seams();

        // Must not panic.
        sweep_once(&pool, &seams, Utc::now()).await;

        assert!(seams.lookup_pending(&gate_id_for("artifact-123")).is_none());
    }

    /// `pool: None` (the same DB-free boot `spawn_durable_writer(None)`
    /// already establishes elsewhere in this file) must self-skip: no
    /// querying loop is spawned at all, so `bastion serve` still boots
    /// with the database down.
    #[tokio::test]
    async fn spawn_pending_harvest_sweep_with_no_pool_self_skips() {
        let seams = Arc::new(seams());

        let handle = spawn_pending_harvest_sweep(None, Arc::clone(&seams), Duration::from_secs(60));

        assert!(
            handle.task.is_none(),
            "no querying loop should be spawned when pool is None"
        );
        assert!(seams.lookup_pending(&gate_id_for("artifact-123")).is_none());
        // A no-op abort on a self-skipped handle must not panic.
        handle.abort();
    }
}
