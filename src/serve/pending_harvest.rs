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

use engine_core::workflows::approve_and_run::PendingHarvestRecord;
use serde_json::Value;

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
}
