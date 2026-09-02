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
