//! Bastion's half of `BA.21.D`: consume `mev attention-queue --notify-only`
//! payloads and deliver the admitted subset over BA.21.A's operator
//! transport, mirroring [`crate::serve::blocked_edge`]'s module shape
//! (`mod.rs` / `poller.rs` / `tests.rs`).
//!
//! **This module never applies a triage rule.** `mev attention-queue
//! --notify-only` already filters the full Attention board down to the
//! interrupt-worthy subset, reading the operator's rule from `brain.toml`'s
//! `[attention]` table — measured 2026-09-01, a 548-item board reduces to 2
//! admitted items. A second cut implemented here would desynchronise from
//! `/attention` the moment either side changed, which is the exact failure
//! class this block exists to avoid (see `planning/blocks/BA.21.D.json`'s
//! `out_of_scope`). Task 1 (this file) only parses; task 2 adds the pure
//! admission-into-the-queue decision (new/duplicate, depth, storm — never a
//! second triage rule); task 3 mounts the process-spawning poller.
//!
//! # Why this is its own type, not `engine_core::operator::OperatorPayload`
//!
//! `telegram.rs` already reuses `engine_core::operator::ValidatedOperatorPayload`
//! for the transport it drives, and this module was asked to prefer that
//! existing type over declaring a parallel one where the shape fits. It does
//! not fit here: `mev attention-queue --notify-only` emits `item_id`,
//! `effective_priority`, `lane`, `repo` and `source` alongside the
//! `gate_id`/`rendered_summary`/`options`/`digest` fields `OperatorPayload`
//! already carries — task 2's admission decision (dedup by `item_id`, depth
//! and storm suppression keyed on `effective_priority`/`lane`) needs those
//! extra fields before a payload is ever handed to `validate()` and wrapped
//! as a `ValidatedOperatorPayload` for delivery. [`AttentionQueueItem`] is
//! that superset; [`AttentionQueueItem::options`] reuses
//! `engine_core::operator::OperatorResponseOption` directly (its `{key,
//! label}` shape matches the emitted JSON exactly), so nothing about the
//! response-option shape is forked — only the extra board-context fields are
//! new here.

// Task 2 adds the pure admission decision (new/duplicate, depth, storm —
// never a second triage rule); task 3 adds the process-spawning shell on
// top and mounts a poller driving it from `src/serve/mod.rs`.
pub mod poller;
#[cfg(test)]
mod tests;

use engine_core::operator::OperatorResponseOption;
use serde::{Deserialize, Serialize};

/// One `EN.8.A`-compatible item as emitted by `mev attention-queue
/// --notify-only` — the admitted subset of the Attention board, already
/// filtered by the operator's triage rule (`brain.toml`'s `[attention]`
/// table). Bastion consumes this shape; it does not produce or re-derive it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AttentionQueueItem {
    /// Stable identity of this Attention item, used by task 2's admission
    /// decision to detect an item already delivered (dedup key).
    pub item_id: String,
    /// Identity of the gate this item renders as, e.g.
    /// `"attention:<item_id>"`.
    pub gate_id: String,
    /// The inline rendered summary — the text as it will appear in the
    /// channel.
    pub rendered_summary: String,
    /// The fixed set of named response options offered to the operator.
    /// Reuses `engine_core::operator::OperatorResponseOption` — the emitted
    /// `{key, label}` shape matches it exactly.
    pub options: Vec<OperatorResponseOption>,
    /// Digest mev computed over the rendered payload
    /// (`AttentionQueuePayload::digest_of`, a second independent
    /// implementation of `engine_core::operator::OperatorPayload::digest_of`
    /// pinned by mev's own tests). Bastion never recomputes this — see the
    /// module-level BLAST RADIUS note in `planning/blocks/BA.21.D.json`.
    pub digest: String,
    /// Effective priority after the board's own priority propagation
    /// (0 = highest).
    pub effective_priority: i64,
    /// Which Attention lane this item surfaced in (e.g. `"hot"`,
    /// `"blocking"`).
    pub lane: String,
    /// Which repo this item is scoped to.
    pub repo: String,
    /// Always `"attention-board"` for items from this source — carried
    /// through rather than assumed, so a future second producer is
    /// distinguishable without a schema change.
    pub source: String,
}

/// Failure to parse `mev attention-queue --notify-only`'s JSON output.
///
/// Distinct from "the command failed to run" (task 3's process-spawning
/// shell handles that) — this is purely "the bytes we got are not a valid
/// `Vec<AttentionQueueItem>`".
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("malformed attention-queue payload: {0}")]
    Malformed(#[source] serde_json::Error),
}

/// Parse `mev attention-queue --notify-only`'s JSON array output into
/// [`AttentionQueueItem`]s.
///
/// Pure: no process spawn, no file I/O — Standing Rule 6's
/// construction-vs-execution split. An empty array (`mev` prints `[]` and
/// exits 0 when the admitted set is empty) parses to an empty `Vec` and is
/// **not** an error; treating the healthy empty-board case as a failure
/// would alarm on exactly the case this source is supposed to stay quiet
/// for. Malformed JSON returns a typed [`ParseError`] rather than panicking.
pub fn parse_attention_queue_payload(json: &str) -> Result<Vec<AttentionQueueItem>, ParseError> {
    serde_json::from_str(json).map_err(ParseError::Malformed)
}

// ── I/O shell (task 3): spawn `mev`, mount a poller, deliver ────────────────
//
// Everything above this line (and all of `poller.rs`) is pure — Standing
// Rule 6's construction-vs-execution split. The process spawn and the
// transport `.send().await` calls live only here, in the thin shell task 3
// owns.

use std::collections::HashSet;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use engine_core::operator::queue::{OperatorQueuePolicy, QueueDigest};
use engine_core::operator::{OperatorPayload, OperatorPayloadLimits, validate};

use super::notify::{OperatorTransport, PendingPayloads};
use poller::{AttentionFetch, TickOutcome, tick_decision};

/// Run `mev attention-queue --notify-only` and classify the result into an
/// [`AttentionFetch`] — the only place in this module a process is spawned.
///
/// - The binary cannot be found/executed at all -> [`AttentionFetch::MevMissing`].
/// - It runs and exits non-zero -> [`AttentionFetch::NonZeroExit`].
/// - It exits 0 but stdout does not parse -> [`AttentionFetch::Unparseable`].
/// - It exits 0 and stdout parses -> [`AttentionFetch::Items`].
///
/// Blocking (a `std::process::Command::output()` call) — callers run this
/// via `tokio::task::spawn_blocking`, never directly on an async executor.
fn spawn_attention_queue_command() -> AttentionFetch {
    let output = match Command::new("mev")
        .args(["attention-queue", "--notify-only"])
        .output()
    {
        Ok(output) => output,
        Err(_) => return AttentionFetch::MevMissing,
    };

    if !output.status.success() {
        return AttentionFetch::NonZeroExit {
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        };
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    match parse_attention_queue_payload(&stdout) {
        Ok(items) => AttentionFetch::Items(items),
        Err(err) => AttentionFetch::Unparseable(err.to_string()),
    }
}

/// Render a [`QueueDigest`] of un-delivered attention items into an
/// [`OperatorPayload`] — a plain summary ("N more items, top: <summary>")
/// with two named options (`ack` / `view`, satisfying
/// `OperatorPayloadLimits::min_options`), never the full remainder list (the
/// whole point of a digest — see `engine_core::operator::queue::digest`'s
/// module doc).
///
/// `gate_id` is namespaced under `attention-digest:` (disjoint from every
/// individual item's `attention:<item_id>` gate id and from the other gate
/// id spaces this process mints — `approve-and-run:`, the notify test
/// route's per-request uuid, and `session_qa::headless`'s `hq-` prefix) and
/// keyed by the digest's own `generated_at` timestamp, so two digests
/// produced on different ticks never collide.
fn digest_payload(digest: &QueueDigest) -> OperatorPayload {
    let gate_id = format!("attention-digest:{}", digest.generated_at.timestamp());
    let remainder = digest.total_count.saturating_sub(1);
    let summary = format!(
        "{} more attention item(s) not shown; top: {}",
        remainder, digest.top.payload.rendered_summary
    );
    OperatorPayload::new(
        gate_id,
        summary,
        vec![
            engine_core::operator::OperatorResponseOption::new("ack", "Acknowledge"),
            engine_core::operator::OperatorResponseOption::new("view", "View board"),
        ],
    )
}

/// Bastion's half of `BA.21.D`: poll `mev attention-queue --notify-only` on
/// a cadence and deliver the admitted subset over BA.21.A's operator
/// transport, registering each delivered item in [`PendingPayloads`] (the
/// same registry `notify::stale_run_alarm` and `session_qa::headless` use)
/// so an operator's later tap resolves against it.
///
/// Never applies a triage rule (see this module's header) and never writes
/// any `state.json` — the process spawn above only ever *reads* `mev`'s
/// stdout; nothing in this file opens a file for writing.
pub struct AttentionSourcePoller {
    fetch: Arc<dyn Fn() -> AttentionFetch + Send + Sync>,
    delivered_ids: HashSet<String>,
    policy: OperatorQueuePolicy,
    limits: OperatorPayloadLimits,
    transport: Arc<dyn OperatorTransport>,
    pending: Arc<PendingPayloads>,
}

impl AttentionSourcePoller {
    /// Construct a poller against the real `mev attention-queue
    /// --notify-only` subprocess.
    pub fn new(
        transport: Arc<dyn OperatorTransport>,
        pending: Arc<PendingPayloads>,
        policy: OperatorQueuePolicy,
    ) -> Self {
        Self::with_fetch(
            transport,
            pending,
            policy,
            Arc::new(spawn_attention_queue_command),
        )
    }

    /// Construct a poller with an injected fetch function — what every test
    /// in this module uses, so no test ever spawns a real `mev` process.
    pub fn with_fetch(
        transport: Arc<dyn OperatorTransport>,
        pending: Arc<PendingPayloads>,
        policy: OperatorQueuePolicy,
        fetch: Arc<dyn Fn() -> AttentionFetch + Send + Sync>,
    ) -> Self {
        Self {
            fetch,
            delivered_ids: HashSet::new(),
            policy,
            limits: OperatorPayloadLimits::default(),
            transport,
            pending,
        }
    }

    /// Run [`tick_decision`] over an already-observed `fetch` result, then
    /// deliver the outcome: validate and send each admitted item as one
    /// individual message (bounded by `policy.operator_queue_depth` —
    /// never N unbounded sends), plus at most one digest message for
    /// whatever did not fit. A validation or send failure for one item is
    /// `warn!`-logged and skipped, matching `stale_run_alarm::deliver_once`
    /// and `session_qa::headless::deliver_once`'s own skip-on-failure
    /// contract — one bad item never blocks the rest of the tick.
    ///
    /// Returns the number of items actually delivered (individual items
    /// only; a sent digest is not counted since it does not correspond to
    /// one admitted item).
    async fn deliver(&mut self, fetch: AttentionFetch, now: DateTime<Utc>) -> usize {
        let TickOutcome {
            to_deliver,
            digest,
            warning,
        } = tick_decision(&fetch, &self.delivered_ids, self.policy, now);

        if let Some(warning) = warning {
            tracing::warn!(
                target: "bastion::serve",
                warning = %warning,
                "attention source fetch failed this tick; admitting nothing"
            );
        }

        let mut delivered = 0usize;
        for item in &to_deliver {
            let payload = OperatorPayload::new(
                item.gate_id.clone(),
                item.rendered_summary.clone(),
                item.options.clone(),
            );
            let validated = match validate(payload, &self.limits) {
                Ok(validated) => validated,
                Err(err) => {
                    tracing::warn!(
                        target: "bastion::serve",
                        error = %err,
                        item_id = %item.item_id,
                        "attention item failed re-validation; skipping"
                    );
                    continue;
                }
            };

            self.pending.insert(validated.clone());

            match self.transport.send(&validated).await {
                Ok(_) => {
                    self.delivered_ids.insert(item.item_id.clone());
                    delivered += 1;
                }
                Err(err) => {
                    tracing::warn!(
                        target: "bastion::serve",
                        error = %err,
                        item_id = %item.item_id,
                        "attention item delivery failed; skipping"
                    );
                }
            }
        }

        if let Some(digest) = &digest {
            let payload = digest_payload(digest);
            match validate(payload, &self.limits) {
                Ok(validated) => {
                    if let Err(err) = self.transport.send(&validated).await {
                        tracing::warn!(
                            target: "bastion::serve",
                            error = %err,
                            "attention digest delivery failed"
                        );
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        target: "bastion::serve",
                        error = %err,
                        "attention digest payload failed re-validation"
                    );
                }
            }
        }

        delivered
    }

    /// Drive [`Self::deliver`] forever on `poll_secs` cadence. Never returns
    /// under normal operation; intended to be spawned once at server boot
    /// (`src/serve/mod.rs::run_server`) via `actix_web::rt::spawn`, mirroring
    /// `notify::stale_run_alarm::spawn_alarm_delivery_loop` and
    /// `session_qa::headless::spawn_headless_question_loop`'s own boot-time
    /// spawn shape.
    ///
    /// Each tick's fetch (a blocking process spawn) is offloaded to
    /// `tokio::task::spawn_blocking`, matching `BlockedEdgePoller::run`'s own
    /// tmux-capture offload — the async `deliver` step (which awaits
    /// `transport.send`) then runs back on the calling task.
    pub async fn run(mut self, poll_secs: u64) {
        let mut interval = tokio::time::interval(Duration::from_secs(poll_secs.max(1)));
        loop {
            interval.tick().await;
            let now = Utc::now();
            let fetch_fn = Arc::clone(&self.fetch);
            let fetch_result = match tokio::task::spawn_blocking(move || (fetch_fn)()).await {
                Ok(result) => result,
                // The blocking task panicked — treat this tick as a failed
                // fetch (fail closed) rather than propagating the panic.
                Err(_) => AttentionFetch::MevMissing,
            };
            let _ = self.deliver(fetch_result, now).await;
        }
    }
}
