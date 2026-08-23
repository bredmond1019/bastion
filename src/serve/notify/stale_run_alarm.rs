//! Stale-run alarm delivery: drain `engine-serve`'s alarmed-run operator
//! queue outward over the injected `OperatorTransport` (`BA.21.B`).
//!
//! `engine_serve::orphan::spawn_stale_run_sweep` (`EN.12.A`) already does
//! the staleness detection, the once-per-run dedup (via
//! `LiveStateStore::mark_alarmed`), and the payload render/validate/enqueue
//! into `live.operator_queue()`. This module does **not** re-derive
//! staleness or re-implement dedup — it is the DRAIN half: pop items the
//! engine side already decided are deliverable, re-validate them against
//! this process's `OperatorPayloadLimits`, and hand them to the transport.
//!
//! This file ships only the PURE decision core (task 1 of `BA.21.B`): no
//! tokio task, no transport call, no boot wiring. The clock is threaded in
//! as a parameter throughout, matching the pattern
//! `src/serve/blocked_edge/poller.rs` and `src/serve/handlers/attention.rs`
//! already use, so every function here is testable without a real sleep.
//! The async delivery shell (task 2) and the `run_server` boot wiring
//! (task 3) build on top of these functions without duplicating their
//! logic.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};

use engine_core::operator::orphan::OrphanPolicy;
use engine_core::operator::queue::{OperatorQueue, OperatorQueueItem};
use engine_core::operator::{
    OperatorPayloadLimits, OperatorTransport, OperatorValidationError, ValidatedOperatorPayload,
};
use engine_serve::live_state::LiveStateStore;

use super::PendingPayloads;

/// Lower bound on the derived sweep interval, so a pathologically small
/// `stale_run_alarm_secs` (e.g. from a test fixture or a misconfigured
/// profile) cannot spin this loop hot.
const MIN_SWEEP_INTERVAL_SECS: u64 = 5;

/// Upper bound on the derived sweep interval, so a very large
/// `stale_run_alarm_secs` (e.g. `thorough`'s widened grace period) still
/// checks often enough that delivery lag stays bounded.
const MAX_SWEEP_INTERVAL_SECS: u64 = 300;

/// The fraction of `stale_run_alarm_secs` the sweep interval is derived as
/// — one quarter of the alarm threshold, so a run has on average several
/// sweep ticks' worth of margin between crossing the threshold and being
/// observed, without polling anywhere near as often as the threshold
/// itself.
const SWEEP_INTERVAL_DIVISOR: u64 = 4;

/// Derive how often the stale-run sweep should tick from
/// `policy.stale_run_alarm_secs` — never a literal. `threshold / 4`,
/// clamped to `[`[`MIN_SWEEP_INTERVAL_SECS`]`, `[`MAX_SWEEP_INTERVAL_SECS`]`]`,
/// so a non-default `stale_run_alarm_secs` demonstrably moves the interval
/// (within the clamp) rather than being shadowed by a constant.
#[must_use]
pub fn sweep_interval(policy: &OrphanPolicy) -> Duration {
    let derived = policy.stale_run_alarm_secs / SWEEP_INTERVAL_DIVISOR;
    let clamped = derived.clamp(MIN_SWEEP_INTERVAL_SECS, MAX_SWEEP_INTERVAL_SECS);
    Duration::from_secs(clamped)
}

/// Re-validate a queue item's payload against this process's
/// `OperatorPayloadLimits` before it may reach a transport.
///
/// The queue stores an unvalidated `OperatorPayload` (`engine-core`'s queue
/// item shape does not carry `ValidatedOperatorPayload` — see
/// `engine_core::operator::queue::item`); `OperatorTransport::send` only
/// accepts the validated wrapper. This re-runs `engine_core::operator::validate`
/// rather than trusting the engine side's own validation pass, so a limits
/// mismatch between the two processes fails closed here instead of at the
/// transport.
pub fn validated_from_item(
    item: &OperatorQueueItem,
    limits: &OperatorPayloadLimits,
) -> Result<ValidatedOperatorPayload, OperatorValidationError> {
    engine_core::operator::validate(item.payload.clone(), limits)
}

/// Pop at most `max` deliverable items from `queue` as of `now`, via the
/// queue's own `next_deliverable`, preserving its priority ordering
/// (highest `effective_priority` first, oldest first on a tie). `max`
/// bounds one delivery tick so a backlog cannot monopolise the loop.
///
/// Each popped item transitions to "open" in `queue` per
/// `OperatorQueue::next_deliverable`'s own contract — this function does
/// not itself do any dedup; that already happened on the engine side via
/// `LiveStateStore::mark_alarmed` before the item was ever enqueued.
pub fn take_deliverable_batch(
    queue: &mut OperatorQueue,
    now: DateTime<Utc>,
    max: usize,
) -> Vec<OperatorQueueItem> {
    let mut batch = Vec::with_capacity(max.min(queue.pending_count()));
    while batch.len() < max {
        match queue.next_deliverable(now) {
            Some(item) => batch.push(item),
            None => break,
        }
    }
    batch
}

// ── Delivery loop (task 2): drain the queue over the injected transport ──

/// A handle to the background alarm-delivery loop
/// [`spawn_alarm_delivery_loop`] spawned. Mirrors
/// `engine_serve::orphan::StaleRunSweepHandle` and
/// `engine_serve::schedule::ScheduleLoopHandle`'s "hold or drop" shape
/// exactly — the same boot-wiring convention `run_server` already knows how
/// to call: the caller may hold this to [`abort`](Self::abort) the loop
/// (e.g. on shutdown) or drop it — dropping does **not** stop the loop (a
/// `tokio::task::JoinHandle` detaches on drop), matching how both of those
/// handles behave.
pub struct AlarmDeliveryHandle {
    task: tokio::task::JoinHandle<()>,
}

impl AlarmDeliveryHandle {
    /// Stop the background delivery loop.
    pub fn abort(&self) {
        self.task.abort();
    }
}

/// One delivery tick: drain at most `max` deliverable items from `live`'s
/// operator queue as of `now`, re-validate each against `limits`, register
/// it in `pending` (so a later operator tap resolves through the existing
/// [`super::PendingLookup`] fallback registry), and hand it to `transport`.
///
/// The queue's write lock is a `std::sync::RwLock` — released BEFORE any
/// `await` (a lock held across an await is a deadlock risk on a
/// single-threaded actix worker), so the lock is taken, the batch is
/// popped, and the guard is dropped before any item is validated or sent.
///
/// A validation error or a `send` error for one item is logged at `warn!`
/// and SKIPPED, never propagated — the same skip-on-render-failure contract
/// [`engine_serve::orphan::alarm_stale_runs`] itself follows: one bad item
/// never blocks delivery of the rest of the batch.
///
/// Returns the number of items actually delivered (a successful `send`),
/// which may be less than the number popped from the queue.
pub async fn deliver_once(
    live: &LiveStateStore,
    transport: &Arc<dyn OperatorTransport>,
    pending: &Arc<PendingPayloads>,
    limits: &OperatorPayloadLimits,
    now: DateTime<Utc>,
    max: usize,
) -> usize {
    let batch = {
        let mut queue = live
            .operator_queue()
            .write()
            .expect("operator queue lock poisoned on write");
        take_deliverable_batch(&mut queue, now, max)
        // `queue` (the write guard) is dropped here, at the end of this
        // block — before any `await` below.
    };

    let mut delivered = 0;
    for item in batch {
        let item_id = item.item_id.clone();
        let validated = match validated_from_item(&item, limits) {
            Ok(validated) => validated,
            Err(err) => {
                tracing::warn!(
                    target: "bastion::serve",
                    error = %err,
                    item_id = %item_id,
                    "stale-run alarm payload failed re-validation; skipping"
                );
                continue;
            }
        };

        pending.insert(validated.clone());

        match transport.send(&validated).await {
            Ok(_) => delivered += 1,
            Err(err) => {
                tracing::warn!(
                    target: "bastion::serve",
                    error = %err,
                    item_id = %item_id,
                    "stale-run alarm delivery failed; skipping"
                );
            }
        }
    }

    delivered
}

/// Upper bound on items drained from the operator queue in a single
/// [`spawn_alarm_delivery_loop`] tick. Generous relative to any plausible
/// number of runs alarmed between two ticks of a several-second interval —
/// this exists only so a pathological backlog cannot monopolise a single
/// tick, matching [`take_deliverable_batch`]'s own `max`-bounds-one-tick
/// contract.
const DELIVERY_BATCH_MAX: usize = 32;

/// Spawn the background delivery loop: a `tokio::spawn`ed
/// `tokio::time::interval(interval)` loop whose body is one
/// [`deliver_once`] call per tick, evaluated at `chrono::Utc::now()`.
///
/// `live` is cheap to clone (an `Arc` around each guarded map, per
/// `LiveStateStore`'s own doc comment), so it and `transport`/`pending` are
/// captured by value/clone into the spawned task with no extra
/// synchronization needed by the caller.
#[must_use]
pub fn spawn_alarm_delivery_loop(
    live: LiveStateStore,
    transport: Arc<dyn OperatorTransport>,
    pending: Arc<PendingPayloads>,
    interval: Duration,
) -> AlarmDeliveryHandle {
    let task = tokio::spawn(async move {
        let limits = OperatorPayloadLimits::default();
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            let now = Utc::now();
            let _ = deliver_once(
                &live,
                &transport,
                &pending,
                &limits,
                now,
                DELIVERY_BATCH_MAX,
            )
            .await;
        }
    });

    AlarmDeliveryHandle { task }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use engine_core::operator::queue::{ItemSource, OperatorQueuePolicy};
    use engine_core::operator::{OperatorPayload, OperatorResponseOption};

    fn now() -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000, 0).unwrap()
    }

    fn payload(gate_id: &str) -> OperatorPayload {
        OperatorPayload::new(
            gate_id,
            "Run stuck: running / Run: r-1 / No progress for 5280s",
            vec![
                OperatorResponseOption::new("ack", "Acknowledge"),
                OperatorResponseOption::new("view", "View run"),
            ],
        )
    }

    fn queue_item(id: &str, priority: i32, secs: i64) -> OperatorQueueItem {
        let ts = Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap();
        OperatorQueueItem::new(id, payload(id), priority, ts, ItemSource::GateApproval)
    }

    // ── sweep_interval ──────────────────────────────────────────────────

    #[test]
    fn sweep_interval_derives_from_default_policy_threshold() {
        let policy = OrphanPolicy::default();
        // default stale_run_alarm_secs is 5280 -> 5280/4 = 1320, above the
        // 300s ceiling, so the derived interval clamps to the ceiling.
        assert_eq!(policy.stale_run_alarm_secs, 5_280);
        assert_eq!(
            sweep_interval(&policy),
            Duration::from_secs(MAX_SWEEP_INTERVAL_SECS)
        );
    }

    #[test]
    fn sweep_interval_moves_with_a_non_default_threshold() {
        let default_policy = OrphanPolicy::default();
        let mut faster_policy = default_policy;
        faster_policy.stale_run_alarm_secs = 400; // 400/4 = 100, within clamp
        assert_ne!(
            sweep_interval(&default_policy),
            sweep_interval(&faster_policy)
        );
        assert_eq!(sweep_interval(&faster_policy), Duration::from_secs(100));
    }

    #[test]
    fn sweep_interval_is_floored_for_a_tiny_threshold() {
        let mut policy = OrphanPolicy::default();
        policy.stale_run_alarm_secs = 8; // 8/4 = 2, below the floor of 5
        assert_eq!(
            sweep_interval(&policy),
            Duration::from_secs(MIN_SWEEP_INTERVAL_SECS)
        );
    }

    #[test]
    fn sweep_interval_is_ceilinged_for_a_huge_threshold() {
        let mut policy = OrphanPolicy::default();
        policy.stale_run_alarm_secs = 100_000; // /4 = 25000, above the ceiling of 300
        assert_eq!(
            sweep_interval(&policy),
            Duration::from_secs(MAX_SWEEP_INTERVAL_SECS)
        );
    }

    // ── validated_from_item ─────────────────────────────────────────────

    #[test]
    fn validated_from_item_round_trips_a_well_formed_payload() {
        let item = queue_item("a", 0, 0);
        let limits = OperatorPayloadLimits::default();
        let validated =
            validated_from_item(&item, &limits).expect("well-formed payload should validate");
        assert_eq!(validated.payload().gate_id, "a");
    }

    #[test]
    fn validated_from_item_returns_err_never_panics_for_an_over_limit_payload() {
        let mut item = queue_item("a", 0, 0);
        item.payload.rendered_summary = "x".repeat(5_000);
        item.payload.digest = engine_core::operator::OperatorPayload::digest_of(
            &item.payload.rendered_summary,
            &item.payload.options,
        );
        let limits = OperatorPayloadLimits::default();
        let err = validated_from_item(&item, &limits)
            .expect_err("oversized summary must fail validation, not panic");
        assert!(matches!(
            err,
            OperatorValidationError::RenderedSummaryTooLong { .. }
        ));
    }

    #[test]
    fn validated_from_item_returns_err_for_too_few_options() {
        let mut item = queue_item("a", 0, 0);
        item.payload.options.clear();
        item.payload.digest = engine_core::operator::OperatorPayload::digest_of(
            &item.payload.rendered_summary,
            &item.payload.options,
        );
        let limits = OperatorPayloadLimits::default();
        let err = validated_from_item(&item, &limits)
            .expect_err("too few options must fail validation, not panic");
        assert!(matches!(err, OperatorValidationError::TooFewOptions { .. }));
    }

    // ── take_deliverable_batch ──────────────────────────────────────────

    #[test]
    fn take_deliverable_batch_returns_empty_on_an_empty_queue() {
        let mut queue = OperatorQueue::new(OperatorQueuePolicy::default());
        let batch = take_deliverable_batch(&mut queue, now(), 5);
        assert!(batch.is_empty());
    }

    #[test]
    fn take_deliverable_batch_respects_max() {
        let policy = OperatorQueuePolicy {
            operator_queue_depth: 10,
            ..OperatorQueuePolicy::default()
        };
        let mut queue = OperatorQueue::new(policy);
        for i in 0..5 {
            queue.enqueue(queue_item(&format!("item-{i}"), i, i as i64));
        }
        let batch = take_deliverable_batch(&mut queue, now(), 2);
        assert_eq!(batch.len(), 2);
    }

    #[test]
    fn take_deliverable_batch_returns_highest_priority_item_first() {
        let policy = OperatorQueuePolicy {
            operator_queue_depth: 10,
            ..OperatorQueuePolicy::default()
        };
        let mut queue = OperatorQueue::new(policy);
        queue.enqueue(queue_item("low", 1, 0));
        queue.enqueue(queue_item("high", 9, 0));
        queue.enqueue(queue_item("mid", 5, 0));
        let batch = take_deliverable_batch(&mut queue, now(), 3);
        let ids: Vec<&str> = batch.iter().map(|i| i.item_id.as_str()).collect();
        assert_eq!(ids, vec!["high", "mid", "low"]);
    }

    #[test]
    fn take_deliverable_batch_bounded_by_queue_depth_not_only_max() {
        // operator_queue_depth defaults to 1, so even with max=10 only one
        // item can be open at once per next_deliverable's own contract.
        let mut queue = OperatorQueue::new(OperatorQueuePolicy::default());
        queue.enqueue(queue_item("a", 1, 0));
        queue.enqueue(queue_item("b", 2, 0));
        let batch = take_deliverable_batch(&mut queue, now(), 10);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].item_id, "b");
    }
}
