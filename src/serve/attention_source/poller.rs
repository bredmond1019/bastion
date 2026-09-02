//! The admission decision — task 2 of `BA.21.D`, pure.
//!
//! **This module never applies a triage rule.** `mev attention-queue
//! --notify-only` already did that (see [`super`]'s module header) — the
//! items this file receives are already the admitted subset. What this
//! module decides is narrower: which of the *admitted* items are new (not
//! already delivered), how many may be open at once
//! (`policy.operator_queue_depth`), and that the remainder collapses into a
//! single digest count rather than one message per item — the storm-
//! suppression contract a restart burst (mev restarting and re-emitting
//! every currently-admitted item at once) needs. Per the block record's
//! `testing_strategy`, this reuses `engine_core::operator::queue`'s
//! [`OperatorQueuePolicy`], [`build_digest`]-family logic
//! ([`storm_digest`]) rather than hand-rolling depth/digest math a second
//! time — the same class of desync risk the block record's BLAST RADIUS
//! note warns about for `digest_of`.
//!
//! The three absent/failure cases — `mev` missing, a non-zero exit, or
//! unparseable output — MUST fail closed: admit nothing, never fall back to
//! delivering the (unfiltered, unbounded) raw board. [`tick_decision`]
//! encodes this by matching [`AttentionFetch`] first, before anything else
//! runs.
//!
//! Task 3 adds the process-spawning shell that produces an [`AttentionFetch`]
//! from a real `mev attention-queue --notify-only` invocation and mounts a
//! poller driving [`tick_decision`] on a cadence in `src/serve/mod.rs`. This
//! file has no spawn and no I/O — every test constructs an [`AttentionFetch`]
//! directly.

use std::collections::HashMap;
use std::collections::HashSet;

use chrono::{DateTime, Utc};
use engine_core::operator::OperatorPayload;
use engine_core::operator::queue::{
    ItemSource, OperatorQueueItem, OperatorQueuePolicy, QueueDigest, compare_items, storm_digest,
};

use super::AttentionQueueItem;

/// What attempting to run `mev attention-queue --notify-only` produced, as
/// observed by task 3's process-spawning shell.
///
/// This is the boundary between the I/O shell and this file's pure
/// decision: task 3 constructs one of these from a real subprocess; every
/// test in this module constructs one directly, no process ever spawned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttentionFetch {
    /// The command ran, exited 0, and its stdout parsed cleanly into the
    /// admitted item set (already filtered by `brain.toml`'s `[attention]`
    /// table — see [`super`]'s module header).
    Items(Vec<AttentionQueueItem>),
    /// The `mev` binary could not be found or executed at all.
    MevMissing,
    /// The command ran but exited non-zero.
    NonZeroExit { code: Option<i32>, stderr: String },
    /// The command exited 0 but its stdout did not parse as the expected
    /// JSON array (see [`super::ParseError`]).
    Unparseable(String),
}

/// The result of one admission tick: which items to deliver individually
/// this tick, an optional digest summarizing whatever did not fit under
/// `policy.operator_queue_depth`, and a warning message when the fetch
/// itself failed (never populated on a healthy empty board — see
/// [`tick_decision`]'s docs).
#[derive(Debug, Clone, PartialEq)]
pub struct TickOutcome {
    /// Items to deliver this tick, highest priority first, capped at
    /// `policy.operator_queue_depth`. Carries the original
    /// [`AttentionQueueItem`] (not the internal [`OperatorQueueItem`]
    /// wrapper) so task 3's delivery shell has the full board context
    /// (`lane`, `repo`, `source`) available for logging/registration.
    pub to_deliver: Vec<AttentionQueueItem>,
    /// A digest of whatever admitted, new items did not fit in
    /// `to_deliver` this tick. `None` when there is no remainder.
    pub digest: Option<QueueDigest>,
    /// Set only when [`AttentionFetch`] itself was a failure case (mev
    /// missing, non-zero exit, or unparseable output) — never set for a
    /// healthy, merely-empty admitted set. Task 3's shell logs this via
    /// `tracing::warn!`; this pure function only produces the message.
    pub warning: Option<String>,
}

impl TickOutcome {
    fn empty() -> Self {
        Self {
            to_deliver: Vec::new(),
            digest: None,
            warning: None,
        }
    }

    fn failed(warning: impl Into<String>) -> Self {
        Self {
            to_deliver: Vec::new(),
            digest: None,
            warning: Some(warning.into()),
        }
    }
}

/// Convert one admitted [`AttentionQueueItem`] into the
/// [`OperatorQueueItem`] shape [`compare_items`]/[`storm_digest`] operate
/// over.
///
/// Two conventions are deliberately bridged here, not left to collide:
///
/// - **Priority direction is inverted.** The Attention board's
///   `effective_priority` is "0 = highest" (lower number, more urgent —
///   see [`AttentionQueueItem::effective_priority`]'s doc), while
///   `engine_core`'s [`compare_items`] sorts *higher* `effective_priority`
///   first. Negating (`0 -> 0`, `3 -> -3`) makes priority-0 items sort
///   first under `compare_items`, which is what a fixture where the
///   priority-0 item must be delivered before the priority-3 item needs.
///   Not inverting this would silently deliver the *least* urgent item
///   first.
/// - **`ItemSource` has no "attention board" variant** —
///   `engine_core::operator::queue::ItemSource` is a closed, out-of-repo
///   enum (`BlockedEdge` / `GateApproval`) and this block is explicitly
///   forbidden from touching engine-core (`out_of_scope`). `GateApproval`
///   is reused as the nearest existing tag, matching the precedent already
///   in this codebase (`src/serve/notify/stale_run_alarm.rs` tags its own
///   non-blocked-edge items the same way).
///
/// The reconstructed [`OperatorPayload::new`] recomputes its digest from
/// `rendered_summary`/`options` — the same independent-`digest_of`
/// computation the block record's BLAST RADIUS note is about. This
/// function does not assert it matches `item.digest`; that agreement is
/// task 1's fixture round-trip assertion, not this admission decision's
/// job.
fn to_queue_item(item: &AttentionQueueItem, now: DateTime<Utc>) -> OperatorQueueItem {
    let payload = OperatorPayload::new(
        item.gate_id.clone(),
        item.rendered_summary.clone(),
        item.options.clone(),
    );
    let engine_priority = i32::try_from(item.effective_priority)
        .unwrap_or(i32::MAX)
        .saturating_neg();
    OperatorQueueItem::new(
        item.item_id.clone(),
        payload,
        engine_priority,
        now,
        ItemSource::GateApproval,
    )
}

/// The pure per-tick admission decision.
///
/// `fetch` is what task 3's shell observed this tick. `delivered_ids` is
/// every `item_id` already delivered on some earlier tick (owned by the
/// caller across ticks, mirroring `BlockedEdgePoller`'s `prev` field) — an
/// item already in this set is never re-admitted even if `mev` keeps
/// emitting it (it stays admitted until the operator answers or it clears
/// off the board). `policy.operator_queue_depth` bounds how many items
/// [`TickOutcome::to_deliver`] may carry; the rest are summarized into
/// [`TickOutcome::digest`] via [`storm_digest`] rather than delivered
/// individually, which is what collapses a burst (mev restarting and
/// re-emitting every currently-admitted item in one tick) into one digest
/// message instead of N.
///
/// The three [`AttentionFetch`] failure variants are matched first and
/// return immediately with `to_deliver` and `digest` both empty/`None` and
/// `warning` set — never falling through to "deliver everything", which is
/// the exact failure this block exists to avoid (measured 2026-09-01: 548
/// board items, 2 admitted; a fallback here would put all 548 back in
/// play). An [`AttentionFetch::Items`] carrying an empty (or
/// entirely-already-delivered) set is the healthy quiet case: empty
/// `to_deliver`/`digest`, no warning.
///
/// Pure: no spawn, no I/O, no clock reads beyond the `now` argument.
#[must_use]
pub fn tick_decision(
    fetch: &AttentionFetch,
    delivered_ids: &HashSet<String>,
    policy: OperatorQueuePolicy,
    now: DateTime<Utc>,
) -> TickOutcome {
    let items = match fetch {
        AttentionFetch::Items(items) => items,
        AttentionFetch::MevMissing => {
            return TickOutcome::failed(
                "mev attention-queue --notify-only: mev binary not found; admitting nothing (never falling back to the unfiltered board)",
            );
        }
        AttentionFetch::NonZeroExit { code, stderr } => {
            return TickOutcome::failed(format!(
                "mev attention-queue --notify-only exited non-zero (code={code:?}): {stderr}; admitting nothing (never falling back to the unfiltered board)"
            ));
        }
        AttentionFetch::Unparseable(detail) => {
            return TickOutcome::failed(format!(
                "mev attention-queue --notify-only output did not parse: {detail}; admitting nothing (never falling back to the unfiltered board)"
            ));
        }
    };

    let new_items: Vec<&AttentionQueueItem> = items
        .iter()
        .filter(|item| !delivered_ids.contains(&item.item_id))
        .collect();

    if new_items.is_empty() {
        return TickOutcome::empty();
    }

    let by_id: HashMap<&str, &AttentionQueueItem> = new_items
        .iter()
        .map(|item| (item.item_id.as_str(), *item))
        .collect();

    let mut queue_items: Vec<OperatorQueueItem> = new_items
        .iter()
        .map(|item| to_queue_item(item, now))
        .collect();
    queue_items.sort_by(compare_items);

    let depth = policy.operator_queue_depth as usize;
    let (deliverable, remainder) = if queue_items.len() <= depth {
        (queue_items.as_slice(), &[][..])
    } else {
        queue_items.split_at(depth)
    };

    let to_deliver: Vec<AttentionQueueItem> = deliverable
        .iter()
        .filter_map(|qi| by_id.get(qi.item_id.as_str()).map(|item| (*item).clone()))
        .collect();

    let digest = storm_digest(remainder, policy.suppression_window_secs, now);

    TickOutcome {
        to_deliver,
        digest,
        warning: None,
    }
}

#[cfg(test)]
mod poller_tests {
    use super::*;
    use engine_core::operator::OperatorResponseOption;

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    fn item(id: &str, priority: i64) -> AttentionQueueItem {
        AttentionQueueItem {
            item_id: id.to_string(),
            gate_id: format!("attention:{id}"),
            rendered_summary: format!("[repo] item {id}"),
            options: vec![
                OperatorResponseOption::new("promote", "Promote"),
                OperatorResponseOption::new("snooze", "Snooze"),
            ],
            digest: format!("digest-{id}"),
            effective_priority: priority,
            lane: "hot".to_string(),
            repo: "repo".to_string(),
            source: "attention-board".to_string(),
        }
    }

    fn default_policy() -> OperatorQueuePolicy {
        OperatorQueuePolicy {
            operator_queue_depth: 1,
            answer_timeout_secs: 900,
            suppression_window_secs: 60,
            digest_schedule_secs: 3600,
        }
    }

    // ── new items under the depth limit ──────────────────────────────────

    #[test]
    fn n_new_items_under_the_depth_limit_are_all_delivered() {
        let policy = OperatorQueuePolicy {
            operator_queue_depth: 3,
            ..default_policy()
        };
        let items = vec![item("a", 0), item("b", 1), item("c", 2)];
        let outcome = tick_decision(
            &AttentionFetch::Items(items),
            &HashSet::new(),
            policy,
            now(),
        );

        assert_eq!(outcome.to_deliver.len(), 3);
        assert!(outcome.digest.is_none());
        assert!(outcome.warning.is_none());
    }

    #[test]
    fn priority_zero_is_delivered_before_priority_three() {
        // Attention board convention: 0 = highest urgency. Must sort first
        // despite `compare_items` sorting *higher* numbers first internally
        // — the inversion in `to_queue_item` is what makes this true.
        let policy = OperatorQueuePolicy {
            operator_queue_depth: 2,
            ..default_policy()
        };
        let items = vec![item("low-urgency", 3), item("high-urgency", 0)];
        let outcome = tick_decision(
            &AttentionFetch::Items(items),
            &HashSet::new(),
            policy,
            now(),
        );

        assert_eq!(outcome.to_deliver[0].item_id, "high-urgency");
        assert_eq!(outcome.to_deliver[1].item_id, "low-urgency");
    }

    // ── depth honored, remainder digested ────────────────────────────────

    #[test]
    fn more_items_than_the_limit_honors_depth_and_digests_the_remainder() {
        let policy = OperatorQueuePolicy {
            operator_queue_depth: 1,
            ..default_policy()
        };
        let items: Vec<AttentionQueueItem> =
            (0..5).map(|i| item(&format!("item-{i}"), i)).collect();
        let outcome = tick_decision(
            &AttentionFetch::Items(items),
            &HashSet::new(),
            policy,
            now(),
        );

        assert_eq!(outcome.to_deliver.len(), 1);
        assert_eq!(outcome.to_deliver[0].item_id, "item-0"); // priority 0 = most urgent
        let digest = outcome.digest.expect("remainder should digest");
        assert_eq!(digest.total_count, 4);
    }

    // ── dedup: already-delivered items are not re-admitted ───────────────

    #[test]
    fn already_delivered_item_is_not_re_admitted() {
        let policy = OperatorQueuePolicy {
            operator_queue_depth: 5,
            ..default_policy()
        };
        let mut delivered = HashSet::new();
        delivered.insert("already-delivered".to_string());
        let items = vec![item("already-delivered", 0), item("fresh", 1)];

        let outcome = tick_decision(&AttentionFetch::Items(items), &delivered, policy, now());

        assert_eq!(outcome.to_deliver.len(), 1);
        assert_eq!(outcome.to_deliver[0].item_id, "fresh");
    }

    // ── restart burst is storm-suppressed ────────────────────────────────

    #[test]
    fn restart_burst_of_admitted_items_collapses_into_one_digest_not_n_messages() {
        // mev restarts and re-emits every currently-admitted item in a
        // single tick — a burst arriving all at once, none previously
        // delivered. The remainder beyond depth must collapse into a single
        // digest, never N separate messages.
        let policy = OperatorQueuePolicy {
            operator_queue_depth: 1,
            ..default_policy()
        };
        let items: Vec<AttentionQueueItem> =
            (0..10).map(|i| item(&format!("burst-{i}"), i)).collect();
        let outcome = tick_decision(
            &AttentionFetch::Items(items),
            &HashSet::new(),
            policy,
            now(),
        );

        assert_eq!(outcome.to_deliver.len(), 1);
        let digest = outcome.digest.expect("burst remainder should digest");
        assert_eq!(digest.total_count, 9);
    }

    // ── empty input admits nothing, quietly ──────────────────────────────

    #[test]
    fn empty_admitted_set_admits_nothing_without_a_warning() {
        let outcome = tick_decision(
            &AttentionFetch::Items(vec![]),
            &HashSet::new(),
            default_policy(),
            now(),
        );

        assert!(outcome.to_deliver.is_empty());
        assert!(outcome.digest.is_none());
        assert!(
            outcome.warning.is_none(),
            "an empty (healthy) board must never warn"
        );
    }

    #[test]
    fn all_items_already_delivered_admits_nothing_without_a_warning() {
        let mut delivered = HashSet::new();
        delivered.insert("a".to_string());
        let outcome = tick_decision(
            &AttentionFetch::Items(vec![item("a", 0)]),
            &delivered,
            default_policy(),
            now(),
        );

        assert!(outcome.to_deliver.is_empty());
        assert!(outcome.digest.is_none());
        assert!(outcome.warning.is_none());
    }

    // ── the three failure modes fail closed, with a warning ──────────────

    #[test]
    fn mev_missing_admits_nothing_and_warns() {
        let outcome = tick_decision(
            &AttentionFetch::MevMissing,
            &HashSet::new(),
            default_policy(),
            now(),
        );

        assert!(outcome.to_deliver.is_empty());
        assert!(outcome.digest.is_none());
        assert!(outcome.warning.is_some());
    }

    #[test]
    fn non_zero_exit_admits_nothing_and_warns() {
        let outcome = tick_decision(
            &AttentionFetch::NonZeroExit {
                code: Some(1),
                stderr: "boom".to_string(),
            },
            &HashSet::new(),
            default_policy(),
            now(),
        );

        assert!(outcome.to_deliver.is_empty());
        assert!(outcome.digest.is_none());
        let warning = outcome.warning.expect("non-zero exit must warn");
        assert!(warning.contains("boom"));
    }

    #[test]
    fn unparseable_output_admits_nothing_and_warns() {
        let outcome = tick_decision(
            &AttentionFetch::Unparseable("unexpected EOF".to_string()),
            &HashSet::new(),
            default_policy(),
            now(),
        );

        assert!(outcome.to_deliver.is_empty());
        assert!(outcome.digest.is_none());
        let warning = outcome.warning.expect("unparseable output must warn");
        assert!(warning.contains("unexpected EOF"));
    }

    #[test]
    fn failure_modes_never_fall_back_to_admitting_everything() {
        // Even when the (hypothetical) fetch carried a huge unfiltered
        // board internally, `AttentionFetch`'s failure variants carry no
        // items at all — there is no path from a failure variant to a
        // non-empty `to_deliver`. This test pins that structurally: every
        // failure variant produces an empty `to_deliver` regardless of
        // `delivered_ids` or `policy`.
        for fetch in [
            AttentionFetch::MevMissing,
            AttentionFetch::NonZeroExit {
                code: None,
                stderr: String::new(),
            },
            AttentionFetch::Unparseable(String::new()),
        ] {
            let outcome = tick_decision(&fetch, &HashSet::new(), default_policy(), now());
            assert!(outcome.to_deliver.is_empty());
        }
    }
}
