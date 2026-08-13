//! Cross-brain now/next/blocked/finished board REST handler for `bastion serve` (BA.11.K).
//!
//! Read-only (D25) — this route never mutates any brain/tier/repo `state.json`. It
//! projects the same rollup the mev/okf-core brain walk already computes for
//! `bastion emit-state` / `bastion validate-brain --state` over HTTP.
//!
//! # Route
//! - `GET /api/board?scope=hq|tier|project|business|epic[&tier=<name>][&epic=<slug>]`
//!   — `scope=epic` requires `&epic=<slug>` and projects every block tagged
//!   with that epic across every repo (BA.11.R); the other scopes are
//!   unchanged (BA.11.K).
//!
//! # Pure core vs I/O shell (Rule 6)
//! [`resolve_scope`], [`build_board`], [`filter_board_to_epic`] (plus
//! [`is_stale_for_scope`]) are pure — unit-tested directly, no filesystem
//! access. [`get_board`] is the thin async handler: it resolves a starting
//! path from the shared [`FileConfig`] registry, walks up to the brain root
//! (`mev::brain::config::find_brain_root`), then runs the same discover →
//! load → build-graph → derive-rollup pipeline `mev::validate_brain_state` /
//! `bastion emit-state` already use — see `src/brainval/mod.rs` — under
//! `web::block`, and hands the pure functions the resulting rollups/files.
//! `scope=epic` additionally consults [`crate::serve::handlers::epics::hq_epic_registry`]
//! to validate `&epic=<slug>` before filtering.
//!
//! # Error mapping
//! - Brain root unresolvable (no `brain.toml` walking up from the workspace root)
//!   → 500 + `C010` (mirrors the `web::block` failure code used by
//!   `handlers/status.rs`; there is no dedicated "brain not found" C-code and this
//!   is an operator-configuration problem, not a per-request one).
//! - `scope=epic` with a missing `&epic=` param, or a value absent from the HQ
//!   `epics[]` registry → 404 + `C005` (one uniform "no such epic board"
//!   response for both, per §11's registry-miss convention).
//! - `web::block` thread-pool failure → 500 + `C010`.
//! - Malformed `scope`/`tier` query parsing is handled by actix's `web::Query`
//!   extractor before the handler runs (surfaced as 400).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use actix_web::{HttpResponse, web};
use serde::Deserialize;

use crate::config::{FileConfig, resolve_workspace_root};
use crate::serve::dto::{
    BoardBlockDto, BoardDto, BoardLaneDto, BoardScope, ErrorPayload, RepoBoardDto,
};
use crate::serve::handlers::epics::hq_epic_registry;

use mev::Diagnostic;
use mev::brain::block_graph::{BlockGraphScope, build_block_graph_export};
use mev::brain::config::{BrainConfig, find_brain_root, load_brain_config};
use mev::brain::last_touched::derive_last_touched;
use mev::brain::state::{
    RepoRollup, StateGraph, StateSource, TierScope, build_state_graph, derive_rollup,
    discover_state_files, load_state,
};
use mev::brain::sync::check_sync;
use okf_core::{BlockedBy, StateFile, TrackBlock};

/// Default tier name used when `scope=tier`/`scope=project` omits `&tier=`.
const DEFAULT_TIER: &str = "core";
/// Tier name `scope=business` shortcuts to.
const BUSINESS_TIER: &str = "business";
/// Lifecycle status value that puts a `tracks[].blocks[]` entry in the `finished` lane.
const CLOSED_STATUS: &str = "closed";

// ── Query params ────────────────────────────────────────────────────────────────

/// `GET /api/board` query params.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BoardQuery {
    /// `scope=hq|tier|project|business|epic`; missing defaults to [`BoardScope::Hq`].
    #[serde(default)]
    pub scope: BoardScope,
    /// `tier=<name>`; only consulted for `scope=tier`/`scope=project` (default `"core"`).
    #[serde(default)]
    pub tier: Option<String>,
    /// `epic=<slug>`; required (and only consulted) for `scope=epic`. Missing or
    /// unknown → 404/`C005` (see [`epic_error_response`]).
    #[serde(default)]
    pub epic: Option<String>,
    /// `graph=true`; gates the A5 `dependent_count`/`ready`/`unmet_count`
    /// enrichment (`plan-board-graph-enrichment`) — defaults to `false` since
    /// task 1 measured the added `mev::build_block_graph_export` call as
    /// roughly doubling per-request board-assembly time on the live HQ
    /// corpus. Absent/`false` yields the three fields omitted (not nulled)
    /// from every `BoardBlockDto` on the wire.
    #[serde(default)]
    pub graph: bool,
}

// ── Pure core ────────────────────────────────────────────────────────────────────

/// Resolve a [`BoardScope`] + optional `tier` query param into the
/// [`TierScope`] the brain-walk rollup should use, plus the tier name (if any)
/// the response DTO should echo back in `BoardDto.tier`.
///
/// Mapping (decided with owner 2026-07-23, see `tasks.md`):
/// - `Hq` → (`TierScope::All`, `None`).
/// - `Tier` / `Project` → (`TierScope::Tier(tier_param or "core")`, `Some(<resolved tier>)`).
/// - `Business` → (`TierScope::Tier("business")`, `Some("business")`) — `tier_param` ignored.
pub fn resolve_scope(scope: BoardScope, tier_param: Option<&str>) -> (TierScope, Option<String>) {
    match scope {
        BoardScope::Hq => (TierScope::All, None),
        BoardScope::Tier | BoardScope::Project => {
            let tier = tier_param
                .filter(|t| !t.trim().is_empty())
                .unwrap_or(DEFAULT_TIER)
                .to_owned();
            (TierScope::Tier(tier.clone()), Some(tier))
        }
        BoardScope::Business => (
            TierScope::Tier(BUSINESS_TIER.to_owned()),
            Some(BUSINESS_TIER.to_owned()),
        ),
        // `Epic` is a cross-repo projection (`TierScope::All`, same as `Hq`), then
        // further filtered by `&epic=<slug>` in `get_board` via
        // `filter_board_to_epic` — an epic spans every repo, so `tier` is `None`.
        BoardScope::Epic => (TierScope::All, None),
    }
}

/// Convert one rollup lane entry (`okf_core::Block`) into a [`BoardBlockDto`],
/// tagging it with the owning `repo` slug (the rollup entries themselves don't carry
/// their own repo — they're already scoped to one repo by [`RepoRollup`]).
///
/// `epics`/`wave`/`priority`/`due`/`track` start at their defaults —
/// `derive_rollup` hard-codes these empty on the `okf_core::Block` it builds for
/// now/next/blocked lane entries. Callers in [`build_board`] fill them in via
/// [`enrich_block`] against a [`track_block_index`] built from `tracks[].blocks[]`.
///
/// `last_touched` is looked up by `"{repo}:{id}"` in `last_touched` (mev's
/// `derive_last_touched` map, threaded through from [`BoardAssembly`]) — a
/// missing key yields `None` ("never worked", never a sentinel). This is the
/// *only* place `board_block_from`'s callers populate the field; `serve`
/// derives nothing itself.
///
/// `dependent_count`/`ready` are looked up the same way in `block_graph`
/// (mev's corpus-wide `BlockEnrichment` map, threaded through from
/// [`BoardAssembly::block_graph`]) — a missing key yields `None` for both,
/// never a fabricated `0`/`false`. `unmet_count` is **never** populated here:
/// mev defines it as `0` for every non-`Blocked` lane, so surfacing it
/// unqualified on every lane would read as falsely-ready; only the `blocked`
/// lane branch in [`build_board`] sets it, from the same `block_graph` entry.
fn board_block_from(
    block: &okf_core::Block,
    repo: &str,
    last_touched: &HashMap<String, String>,
    block_graph: &HashMap<String, BlockEnrichment>,
) -> BoardBlockDto {
    let key = format!("{repo}:{}", block.id);
    let enrichment = block_graph.get(&key);
    BoardBlockDto {
        id: block.id.clone(),
        title: block.title.clone(),
        repo: repo.to_owned(),
        status: block.status.clone(),
        blocked_by: block.blocked_by.clone(),
        epics: Vec::new(),
        wave: None,
        priority: None,
        due: None,
        track: None,
        last_touched: last_touched.get(&key).cloned(),
        dependent_count: enrichment.map(|e| e.dependent_count),
        ready: enrichment.map(|e| e.ready),
        unmet_count: None,
    }
}

/// Build a per-repo `id -> (authoring TrackBlock, enclosing Track.title)` index
/// from the loaded `(StateSource, StateFile)` pairs, for one `repo` slug.
///
/// Looks up the `StateFile` whose `StateSource::repo_slug` matches `repo`; a repo
/// with no loaded file contributes an empty index. If the same block id appears
/// in more than one `tracks[]` entry (which shouldn't happen in well-formed
/// state, but isn't rejected upstream either), the later entry wins — callers
/// walk `file.tracks` in order and simply overwrite the map entry.
fn track_block_index<'a>(
    repo: &str,
    files: &'a [(StateSource, StateFile)],
) -> HashMap<&'a str, (&'a TrackBlock, &'a str)> {
    let mut index = HashMap::new();
    let Some((_, file)) = files.iter().find(|(src, _)| src.repo_slug == repo) else {
        return index;
    };

    for track in &file.tracks {
        for block in &track.blocks {
            index.insert(block.id.as_str(), (block, track.title.as_str()));
        }
    }

    index
}

/// Fill `epics`/`wave`/`priority`/`due`/`track`/`status` on `dto` from the
/// authoring `TrackBlock` + enclosing track title, when `entry` matches. An
/// unmatched id (no `tracks[]` entry with this block's id) leaves the DTO's
/// existing defaults (`epics: []`, four `None`s, and the rollup-fabricated
/// `status`) untouched.
fn enrich_block(dto: &mut BoardBlockDto, entry: Option<(&TrackBlock, &str)>) {
    let Some((track_block, track_title)) = entry else {
        return;
    };

    dto.epics = track_block.epics.clone();
    dto.wave = track_block.wave;
    dto.priority = track_block.priority;
    dto.due = track_block.due.clone();
    dto.track = Some(track_title.to_owned());
    dto.status = track_block.status.clone();
}

/// Build a `"<repo>:<id>" -> Option<status>` map from the loaded `files`, the
/// same key shape `mev::brain::state::derive_focus`'s private unmet-dependency
/// filter uses, for [`unmet_deps`] to consult.
fn block_status_map(files: &[(StateSource, StateFile)]) -> HashMap<String, Option<String>> {
    let mut map = HashMap::new();
    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                let key = format!("{}:{}", src.repo_slug, block.id);
                map.insert(key, block.status.clone());
            }
        }
    }
    map
}

/// Filter `deps` down to the unmet subset, reimplementing the private closure
/// `mev::brain::state::derive_focus` uses internally (not exported — see
/// `../mev/src/brain/state.rs:1795-1810`). An edge is unmet when it is
/// **targetless** — [`BlockedBy::External`], [`BlockedBy::Operator`], or
/// [`BlockedBy::Approval`] — or a [`BlockedBy::Block`] whose target's mapped
/// authored status is not `Some("closed")` — including a target absent from
/// `status_map` entirely (an unresolvable/missing dependency is unmet, not
/// vacuously satisfied).
///
/// The three targetless variants have no dependency to resolve and are unmet for
/// as long as they are present: an operator session clears when its exit artifact
/// exists and an approval clears when a human answers, neither of which is visible
/// in `status_map`. This mirrors `mev::brain::state`'s own startability filter
/// (`../mev/src/brain/state.rs:2234-2241`), which groups all three the same way.
fn unmet_deps(deps: &[BlockedBy], status_map: &HashMap<String, Option<String>>) -> Vec<BlockedBy> {
    deps.iter()
        .filter(|d| match d {
            BlockedBy::External { .. }
            | BlockedBy::Operator { .. }
            | BlockedBy::Approval { .. } => true,
            BlockedBy::Block { repo, id, .. } => {
                let key = format!("{repo}:{id}");
                status_map.get(&key).and_then(|s| s.as_deref()) != Some(CLOSED_STATUS)
            }
        })
        .cloned()
        .collect()
}

/// Derive the `finished` lane (blocks with `status == "closed"`) for one repo
/// slug from `index` (already built by [`track_block_index`] for this repo) and
/// `status_map` (for [`unmet_deps`]).
///
/// Entries are sorted by block id for deterministic ordering (a `HashMap`'s
/// iteration order is otherwise unspecified). A repo with no loaded file (e.g.
/// its `state.json` is missing/malformed) has an empty `index` and contributes
/// an empty finished lane rather than erroring, matching [`derive_rollup`]'s own
/// degrade-gracefully behavior for `now`/`next`/`blocked`.
fn finished_blocks_for_repo(
    repo: &str,
    index: &HashMap<&str, (&TrackBlock, &str)>,
    status_map: &HashMap<String, Option<String>>,
    last_touched: &HashMap<String, String>,
    block_graph: &HashMap<String, BlockEnrichment>,
) -> Vec<BoardBlockDto> {
    let mut entries: Vec<&(&TrackBlock, &str)> = index.values().collect();
    entries.sort_by_key(|(block, _)| block.id.as_str());

    entries
        .into_iter()
        .filter(|(block, _)| block.status.as_deref() == Some(CLOSED_STATUS))
        .map(|&(block, track_title)| {
            let key = format!("{repo}:{}", block.id);
            let enrichment = block_graph.get(&key);
            let mut dto = BoardBlockDto {
                id: block.id.clone(),
                title: block.title.clone(),
                repo: repo.to_owned(),
                status: block.status.clone(),
                blocked_by: Vec::new(),
                epics: Vec::new(),
                wave: None,
                priority: None,
                due: None,
                track: None,
                last_touched: last_touched.get(&key).cloned(),
                dependent_count: enrichment.map(|e| e.dependent_count),
                ready: enrichment.map(|e| e.ready),
                // `finished` is never the blocked lane — `unmet_count` stays
                // `None` here (mev defines it as `0` for this lane, which
                // would read as falsely-ready if surfaced unqualified).
                unmet_count: None,
            };
            enrich_block(&mut dto, Some((block, track_title)));
            dto.blocked_by = unmet_deps(&block.depends_on, status_map);
            dto
        })
        .collect()
}

/// Project the in-scope [`RepoRollup`]s + loaded `files` into a [`BoardDto`]:
/// per-repo `now`/`next`/`blocked` lanes straight from `rollups`, `finished`
/// derived from `files`' `tracks[].blocks[]`, an aggregate `lanes` across every
/// in-scope repo, and the caller-computed `stale` freshness flag threaded through
/// unchanged.
///
/// `block_graph` (mev's corpus-wide `"{repo}:{id}" -> BlockEnrichment` map,
/// threaded through from [`BoardAssembly::block_graph`]) populates
/// `dependent_count`/`ready` on **every** lane — `now`, `next`, `blocked`,
/// `deferred`, and `finished` — for every mapped block; a block absent from
/// the map (e.g. `?graph=1` not requested, or `max_nodes`-truncated) gets
/// `None` for both, never a fabricated `0`/`false`. `unmet_count` is the one
/// exception: it is populated (`Some`) **only** on the `blocked` lane, and
/// left `None` on the other four — mev defines `unmet_count` as `0` for every
/// non-`Blocked` lane, so surfacing it unqualified there would read as
/// falsely-ready (see `BoardBlockDto::unmet_count`'s doc comment).
pub fn build_board(
    scope: BoardScope,
    resolved_tier: Option<String>,
    rollups: &[RepoRollup],
    files: &[(StateSource, StateFile)],
    stale: bool,
    last_touched: &HashMap<String, String>,
    block_graph: &HashMap<String, BlockEnrichment>,
) -> BoardDto {
    let mut repos: Vec<RepoBoardDto> = Vec::new();
    let mut agg_now = Vec::new();
    let mut agg_next = Vec::new();
    let mut agg_blocked = Vec::new();
    let mut agg_deferred = Vec::new();
    let mut agg_finished = Vec::new();

    let status_map = block_status_map(files);

    for rollup in rollups {
        let index = track_block_index(&rollup.repo, files);

        // `now`/`next`: enrich fields from tracks[] and recompute blocked_by via
        // `unmet_deps` when the id has a tracks[] match; an unmatched id keeps
        // the rollup's existing (empty) blocked_by.
        let now: Vec<BoardBlockDto> = rollup
            .now
            .iter()
            .map(|b| {
                let mut dto = board_block_from(b, &rollup.repo, last_touched, block_graph);
                let entry = index.get(b.id.as_str()).copied();
                enrich_block(&mut dto, entry);
                if let Some((track_block, _)) = entry {
                    dto.blocked_by = unmet_deps(&track_block.depends_on, &status_map);
                }
                dto
            })
            .collect();
        let next: Vec<BoardBlockDto> = rollup
            .next
            .iter()
            .map(|b| {
                let mut dto = board_block_from(b, &rollup.repo, last_touched, block_graph);
                let entry = index.get(b.id.as_str()).copied();
                enrich_block(&mut dto, entry);
                if let Some((track_block, _)) = entry {
                    dto.blocked_by = unmet_deps(&track_block.depends_on, &status_map);
                }
                dto
            })
            .collect();
        // `blocked` lane: enrich the other five fields only — `blocked_by`
        // stays the rollup's already-computed `unmet` list so the two
        // derivations (rollup's and this handler's) cannot drift apart.
        // This is the ONLY lane where `unmet_count` is surfaced (`Some`) —
        // mev defines `unmet_count` as `0` for every other lane, which would
        // read as falsely-ready if projected unqualified (see task 2's doc
        // comment on `BoardBlockDto::unmet_count`).
        let blocked: Vec<BoardBlockDto> = rollup
            .blocked
            .iter()
            .map(|b| {
                let mut dto = board_block_from(b, &rollup.repo, last_touched, block_graph);
                let entry = index.get(b.id.as_str()).copied();
                enrich_block(&mut dto, entry);
                let key = format!("{}:{}", rollup.repo, b.id);
                dto.unmet_count = block_graph.get(&key).map(|e| e.unmet_count);
                dto
            })
            .collect();
        // `deferred` lane: same enrich + recompute path as `now`/`next`. A
        // deferred block CAN carry real unmet deps worth showing in the detail
        // drawer — deferral suppresses attention, it does not erase the DAG.
        let deferred: Vec<BoardBlockDto> = rollup
            .deferred
            .iter()
            .map(|b| {
                let mut dto = board_block_from(b, &rollup.repo, last_touched, block_graph);
                let entry = index.get(b.id.as_str()).copied();
                enrich_block(&mut dto, entry);
                if let Some((track_block, _)) = entry {
                    dto.blocked_by = unmet_deps(&track_block.depends_on, &status_map);
                }
                dto
            })
            .collect();
        let finished =
            finished_blocks_for_repo(&rollup.repo, &index, &status_map, last_touched, block_graph);

        agg_now.extend(now.iter().cloned());
        agg_next.extend(next.iter().cloned());
        agg_blocked.extend(blocked.iter().cloned());
        agg_deferred.extend(deferred.iter().cloned());
        agg_finished.extend(finished.iter().cloned());

        repos.push(RepoBoardDto {
            repo: rollup.repo.clone(),
            tier: rollup.tier.clone(),
            lanes: BoardLaneDto {
                now,
                next,
                blocked,
                deferred,
                finished,
            },
        });
    }

    BoardDto {
        scope,
        tier: resolved_tier,
        lanes: BoardLaneDto {
            now: agg_now,
            next: agg_next,
            blocked: agg_blocked,
            deferred: agg_deferred,
            finished: agg_finished,
        },
        repos,
        stale,
    }
}

/// Filter an already-built [`BoardDto`] down to the blocks tagged with `slug`
/// in their `epics[]` membership, on all four lanes — both the aggregate
/// `lanes` and each [`RepoBoardDto`]'s own lanes. A [`RepoBoardDto`] left with
/// all four lanes empty after filtering is dropped from `repos[]` entirely (a
/// repo that contributes no member block has nothing to say about this
/// epic). A block tagged with more than one epic slug naturally survives the
/// filter for each of its epics — callers invoke this once per `&epic=`
/// value, so a block in `["a", "b"]` appears on both `a`'s and `b`'s boards.
///
/// `scope`/`tier` on the returned [`BoardDto`] are left as `build_board`
/// already set them (`Epic`, `tier: None`) — this fn only prunes lane
/// membership, it doesn't touch the envelope fields.
pub fn filter_board_to_epic(mut board: BoardDto, slug: &str) -> BoardDto {
    let keep = |dto: &BoardBlockDto| dto.epics.iter().any(|e| e == slug);

    board.lanes.now.retain(keep);
    board.lanes.next.retain(keep);
    board.lanes.blocked.retain(keep);
    board.lanes.deferred.retain(keep);
    board.lanes.finished.retain(keep);

    board.repos.retain_mut(|repo| {
        repo.lanes.now.retain(keep);
        repo.lanes.next.retain(keep);
        repo.lanes.blocked.retain(keep);
        repo.lanes.deferred.retain(keep);
        repo.lanes.finished.retain(keep);
        // Every lane must be listed here: a repo whose only surviving blocks in
        // this epic are deferred still belongs on the board.
        !(repo.lanes.now.is_empty()
            && repo.lanes.next.is_empty()
            && repo.lanes.blocked.is_empty()
            && repo.lanes.deferred.is_empty()
            && repo.lanes.finished.is_empty())
    });

    board
}

/// Is `&epic=` absent or blank on a `scope=epic` request? Pulled out of
/// [`get_board`] as its own pure fn so the missing-param error branch is
/// unit-testable without spinning up actix. `pub(crate)` so sibling handler
/// modules (e.g. `block_graph`) can reuse the same registry-miss convention.
pub(crate) fn epic_param_missing(epic: Option<&str>) -> bool {
    epic.map(str::trim).unwrap_or("").is_empty()
}

/// Is `slug` present in the HQ `epics[]` registry? Pulled out of [`get_board`]
/// as its own pure fn so the unknown-slug error branch is unit-testable
/// without spinning up actix. `pub(crate)` so sibling handler modules can
/// reuse the same registry-miss convention.
pub(crate) fn epic_known(slug: &str, registry: &[okf_core::Epic]) -> bool {
    registry.iter().any(|e| e.slug == slug)
}

/// Is any in-scope repo's `status.md` cache stale relative to its `state.json`?
///
/// `check_sync` runs over every `[[repos]]` entry in `brain.toml` regardless of
/// scope; this narrows that to the repos actually in scope for this board response
/// by matching each diagnostic's message against `"repo '<slug>'"` — the stable
/// substring every `check_sync` diagnostic carries (see `mev::brain::sync`).
pub fn is_stale_for_scope(diagnostics: &[Diagnostic], in_scope_repos: &[String]) -> bool {
    in_scope_repos.iter().any(|slug| {
        let needle = format!("repo '{slug}'");
        diagnostics.iter().any(|d| d.message.contains(&needle))
    })
}

// ── I/O shell ──────────────────────────────────────────────────────────────────

/// Build a 500 response from a `BlockingError` (thread panic / runtime shutdown),
/// mirroring `handlers/status.rs::blocking_error_response`. `pub(crate)` so
/// sibling handler modules can reuse the same error shape.
pub(crate) fn blocking_error_response(err: actix_web::error::BlockingError) -> HttpResponse {
    HttpResponse::InternalServerError().json(ErrorPayload {
        code: "C010".to_owned(),
        message: format!("blocking thread error: {err}"),
    })
}

/// Build a 404 response for `scope=epic`'s two registry-miss cases: a missing
/// `&epic=` param, or a value not present in the HQ `epics[]` registry — one
/// uniform "no such epic board" shape (matching §11's registry-miss
/// convention), `message` naming which of the two happened. `pub(crate)` so
/// sibling handler modules can reuse the same registry-miss response shape.
pub(crate) fn epic_error_response(message: impl std::fmt::Display) -> HttpResponse {
    HttpResponse::NotFound().json(ErrorPayload {
        code: "C005".to_owned(),
        message: message.to_string(),
    })
}

/// Build a 500 response for a brain-root resolution failure (no `brain.toml`
/// found walking up from the resolved workspace root, or the file failed to
/// parse). This is an operator-configuration problem, not a per-request one —
/// mirrored on the same `C010` code used for other I/O-shell failures since
/// there is no dedicated brain-root C-code. `pub(crate)` so sibling handler
/// modules can reuse the same 500/`C010` response shape.
pub(crate) fn brain_root_error_response(message: impl std::fmt::Display) -> HttpResponse {
    HttpResponse::InternalServerError().json(ErrorPayload {
        code: "C010".to_owned(),
        message: message.to_string(),
    })
}

/// The loaded [`BrainConfig`], the in-scope `RepoRollup`s for a resolved
/// [`TierScope`], the loaded `(StateSource, StateFile)` pairs, the built
/// [`StateGraph`], and the `stale` freshness flag — the inputs [`build_board`]
/// (plus, for `scope=epic`, [`hq_epic_registry`]) needs, assembled by
/// [`assemble_board`]. `pub(crate)` (and each field `pub(crate)`) so sibling
/// handler modules (e.g. `block_graph`) can reuse the same brain-walk
/// assembly instead of re-plumbing discover → load → build-graph →
/// derive-rollup themselves — the graph and the board must read one corpus in
/// one request shape.
pub(crate) struct BoardAssembly {
    pub(crate) config: BrainConfig,
    pub(crate) rollups: Vec<RepoRollup>,
    pub(crate) files: Vec<(StateSource, StateFile)>,
    pub(crate) graph: StateGraph,
    pub(crate) stale: bool,
    /// `"{repo}:{id}" -> updated_at` — mev's derived per-block SDLC recency
    /// (`MV.10.D`, `mev::brain::last_touched::derive_last_touched`), computed
    /// exactly once per request here and threaded into [`build_board`]. A
    /// block absent from this map has no resolvable SDLC run — never a
    /// sentinel, never backfilled from `StateFile.updated`.
    pub(crate) last_touched: HashMap<String, String>,
    /// `"{repo}:{id}" -> BlockEnrichment` — mev's corpus-wide
    /// `dependent_count`/`ready`/`unmet_count` (`plan-board-graph-enrichment`
    /// A5), computed **at most once** per request by a single UNSCOPED
    /// `mev::build_block_graph_export` call and threaded into [`build_board`].
    /// Empty (not an error) when the graph enrichment wasn't requested
    /// (`?graph=1` gate, per task 1's measurement) or the corpus has no
    /// blocks. A block absent from this map has no graph entry (e.g.
    /// `max_nodes`-truncated) — never a fabricated zero.
    pub(crate) block_graph: HashMap<String, BlockEnrichment>,
}

/// One block's corpus-wide graph enrichment, carried verbatim from
/// `mev::brain::block_graph::BlockGraphNode` — bastion derives nothing here.
/// `unmet_count` is populated by mev as `0` for every non-blocked lane (see
/// `BlockGraphNode::unmet_count`'s doc comment); [`build_board`] (task 4) is
/// responsible for only surfacing it as `Some` on the blocked lane and `None`
/// elsewhere on the wire — this struct just mirrors mev's raw values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BlockEnrichment {
    /// Corpus-wide fan-in count — identical for a node key across a scoped
    /// and an unscoped export (see `BlockGraphNode::dependent_count`).
    pub(crate) dependent_count: u32,
    /// Membership in mev's full-corpus `ready_order`.
    pub(crate) ready: bool,
    /// Unmet-dependency count; `0` for every non-`Blocked` lane per mev's own
    /// contract — do not treat this raw `0` as "ready" on a non-blocked lane.
    pub(crate) unmet_count: u32,
}

/// Assemble the brain-walk inputs `build_board` needs: the loaded
/// [`BrainConfig`], the loaded `(StateSource, StateFile)` pairs, the built
/// [`StateGraph`], the in-scope `RepoRollup`s for `tier_scope`, and the
/// `stale` flag. Reuses the exact discover → load → build-graph →
/// derive-rollup pipeline `mev::validate_brain_state` runs (see
/// `src/brainval/mod.rs`) instead of re-plumbing it. Malformed/unreadable
/// individual `state.json` files are skipped (degrade gracefully) rather than
/// failing the whole request — only an unresolvable brain root is a hard
/// error. `pub(crate)` so sibling handler modules can call it directly rather
/// than duplicating this pipeline.
///
/// `include_graph` gates the A5 enrichment (`plan-board-graph-enrichment`
/// task 1's measurement: an unconditional `build_block_graph_export` call
/// roughly doubles per-request board-assembly time on the live HQ corpus) —
/// when `false`, [`BoardAssembly::block_graph`] is left empty and no export
/// is built at all; callers that don't need `dependent_count`/`ready` (e.g.
/// `handlers/block_graph.rs`, which builds its own scoped export) should pass
/// `false`.
pub(crate) fn assemble_board(
    root: &Path,
    tier_scope: &TierScope,
    include_graph: bool,
) -> Result<BoardAssembly, String> {
    let config = load_brain_config(&root.join("brain.toml"))
        .map_err(|e| format!("could not load brain.toml at {}: {e}", root.display()))?;

    let (sources, _discovery_diags) = discover_state_files(root, &config);

    let mut loaded: Vec<(StateSource, StateFile)> = Vec::new();
    for src in &sources {
        if let Ok(file) = load_state(&src.abs_path) {
            loaded.push((src.clone(), file));
        }
    }

    let graph = build_state_graph(&loaded);
    let rollups = derive_rollup(tier_scope, &config, &[], &graph, &loaded);

    let in_scope_repos: Vec<String> = rollups.iter().map(|r| r.repo.clone()).collect();
    let sync_diags = check_sync(root, &config);
    let stale = is_stale_for_scope(&sync_diags, &in_scope_repos);

    let last_touched = flatten_last_touched(derive_last_touched(root, &config, &loaded));

    let block_graph = if include_graph {
        build_block_graph_enrichment(root, &config, &graph, &loaded)
    } else {
        HashMap::new()
    };

    Ok(BoardAssembly {
        config,
        rollups,
        files: loaded,
        graph,
        stale,
        last_touched,
        block_graph,
    })
}

/// Reduce mev's `"{repo}:{id}" -> LastTouched` map to the plain
/// `"{repo}:{id}" -> updated_at` map the board consumes.
///
/// mev's `derive_last_touched` returns `LastTouched { updated_at, status }` as of its
/// `ticket-reconcile-failed-consumer` work. The board only ever renders the timestamp
/// (`BoardBlockDto.last_touched` is an `Option<String>`), so the `status` half is
/// dropped **here, at the boundary**, rather than threaded through every
/// `board_block_from` signature. That keeps the wire contract byte-identical.
///
/// Dropping `status` is deliberate and is not a decision about whether the board
/// should eventually show it: `status` is what mev derives `BlockGraphNode.reconcile_failed`
/// from, and surfacing it here would be a behaviour change owned by its own block, not
/// by a build fix.
pub(crate) fn flatten_last_touched(
    map: HashMap<String, mev::brain::last_touched::LastTouched>,
) -> HashMap<String, String> {
    map.into_iter()
        .map(|(key, touched)| (key, touched.updated_at))
        .collect()
}

/// Build the corpus-wide `"{repo}:{id}" -> BlockEnrichment` map by calling
/// `mev::build_block_graph_export` **exactly once**, with an UNSCOPED
/// [`BlockGraphScope`] (`TierScope::All`, no epic/repo restriction,
/// `include_closed: true`, `include_boundary: false`, `max_nodes: usize::MAX`).
///
/// Scoping this call to the board's own `tier_scope` would silently
/// reintroduce the exact scope-dependence this block exists to eliminate:
/// `dependent_count`/`ready` are corpus-invariant by construction in mev
/// (`BlockGraphNode::dependent_count`'s doc comment), but only when the export
/// itself is built over the full corpus before any scope filtering — a scoped
/// export's fan-in counts only its own in-scope neighbours. `include_closed:
/// true` and no `max_nodes` truncation are likewise required so a block
/// dropped from *this* export for a reason unrelated to the board's own scope
/// doesn't fabricate a `None` that should have been a real value.
fn build_block_graph_enrichment(
    root: &Path,
    config: &BrainConfig,
    graph: &StateGraph,
    files: &[(StateSource, StateFile)],
) -> HashMap<String, BlockEnrichment> {
    let scope = BlockGraphScope {
        tier: TierScope::All,
        epic: None,
        repo: None,
        include_closed: true,
        include_boundary: false,
        max_nodes: usize::MAX,
    };

    let export = build_block_graph_export(root, config, graph, files, &scope);

    export
        .nodes
        .into_iter()
        .map(|node| {
            (
                node.key,
                BlockEnrichment {
                    dependent_count: node.dependent_count,
                    ready: node.ready,
                    unmet_count: node.unmet_count,
                },
            )
        })
        .collect()
}

/// The two error shapes [`get_board`]'s `web::block` closure can fail with:
/// an operator-configuration brain-root problem (500/`C010`), or — `scope=epic`
/// only — a slug absent from the HQ registry (404/`C005`). The missing-`epic`-
/// param case is checked synchronously before the closure ever runs, so it
/// isn't a variant here.
enum BoardError {
    BrainRoot(String),
    UnknownEpic(String),
}

/// `GET /api/board?scope=hq|tier|project|business|epic[&tier=<name>][&epic=<slug>]`
/// — cross-brain now/next/blocked/finished board (BA.11.K), plus the
/// cross-repo `scope=epic` initiative projection (BA.11.R).
///
/// Bearer auth is inherited from the `/api` scope's `BearerAuthMiddleware` — a
/// request without a valid token never reaches this handler (401 upstream).
pub async fn get_board(
    query: web::Query<BoardQuery>,
    registry: web::Data<FileConfig>,
) -> HttpResponse {
    let BoardQuery {
        scope,
        tier,
        epic,
        graph: include_graph,
    } = query.into_inner();

    if scope == BoardScope::Epic && epic_param_missing(epic.as_deref()) {
        return epic_error_response("scope=epic requires a non-empty &epic=<slug> query param");
    }

    let (tier_scope, resolved_tier) = resolve_scope(scope, tier.as_deref());

    let start: PathBuf =
        resolve_workspace_root(None, None, &registry).unwrap_or_else(|_| PathBuf::from("."));

    match web::block(move || -> Result<BoardDto, BoardError> {
        let root = find_brain_root(&start).map_err(|e| {
            BoardError::BrainRoot(format!(
                "could not resolve brain root from {}: {e}",
                start.display()
            ))
        })?;
        let BoardAssembly {
            config,
            rollups,
            files,
            graph: _graph,
            stale,
            last_touched,
            block_graph,
        } = assemble_board(&root, &tier_scope, include_graph).map_err(BoardError::BrainRoot)?;
        let board = build_board(
            scope,
            resolved_tier,
            &rollups,
            &files,
            stale,
            &last_touched,
            &block_graph,
        );

        if scope != BoardScope::Epic {
            return Ok(board);
        }

        // Checked non-empty above.
        let slug = epic.expect("scope=epic requires &epic=, checked before web::block");
        if !epic_known(&slug, hq_epic_registry(&config, &files)) {
            return Err(BoardError::UnknownEpic(format!("unknown epic: {slug}")));
        }
        Ok(filter_board_to_epic(board, &slug))
    })
    .await
    {
        Ok(Ok(dto)) => HttpResponse::Ok().json(dto),
        Ok(Err(BoardError::BrainRoot(msg))) => brain_root_error_response(msg),
        Ok(Err(BoardError::UnknownEpic(msg))) => epic_error_response(msg),
        Err(err) => blocking_error_response(err),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use okf_core::BlockedBy;
    use okf_core::{Block, Track, TrackBlock};

    // ── resolve_scope ────────────────────────────────────────────────────────

    #[test]
    fn resolve_scope_hq_is_all_with_no_tier() {
        assert_eq!(resolve_scope(BoardScope::Hq, None), (TierScope::All, None));
    }

    #[test]
    fn resolve_scope_hq_ignores_tier_param() {
        // scope=hq is a whole-brain aggregate; a stray &tier= is ignored.
        assert_eq!(
            resolve_scope(BoardScope::Hq, Some("core")),
            (TierScope::All, None)
        );
    }

    #[test]
    fn resolve_scope_tier_defaults_to_core_when_absent() {
        assert_eq!(
            resolve_scope(BoardScope::Tier, None),
            (TierScope::Tier("core".to_owned()), Some("core".to_owned()))
        );
    }

    #[test]
    fn resolve_scope_tier_defaults_to_core_when_empty_string() {
        assert_eq!(
            resolve_scope(BoardScope::Tier, Some("")),
            (TierScope::Tier("core".to_owned()), Some("core".to_owned()))
        );
    }

    #[test]
    fn resolve_scope_tier_uses_given_tier() {
        assert_eq!(
            resolve_scope(BoardScope::Tier, Some("side")),
            (TierScope::Tier("side".to_owned()), Some("side".to_owned()))
        );
    }

    #[test]
    fn resolve_scope_project_mirrors_tier_default() {
        assert_eq!(
            resolve_scope(BoardScope::Project, None),
            (TierScope::Tier("core".to_owned()), Some("core".to_owned()))
        );
    }

    #[test]
    fn resolve_scope_project_uses_given_tier() {
        assert_eq!(
            resolve_scope(BoardScope::Project, Some("client")),
            (
                TierScope::Tier("client".to_owned()),
                Some("client".to_owned())
            )
        );
    }

    #[test]
    fn resolve_scope_business_is_shortcut_ignoring_tier_param() {
        assert_eq!(
            resolve_scope(BoardScope::Business, Some("core")),
            (
                TierScope::Tier("business".to_owned()),
                Some("business".to_owned())
            )
        );
    }

    #[test]
    fn resolve_scope_epic_is_all_with_no_tier() {
        // An epic spans every repo — same TierScope as Hq — and tier is
        // always None since epics aren't tier-scoped.
        assert_eq!(
            resolve_scope(BoardScope::Epic, None),
            (TierScope::All, None)
        );
    }

    #[test]
    fn resolve_scope_epic_ignores_tier_param() {
        assert_eq!(
            resolve_scope(BoardScope::Epic, Some("core")),
            (TierScope::All, None)
        );
    }

    // ── finished_blocks_for_repo ─────────────────────────────────────────────

    fn sample_state_file(blocks: Vec<TrackBlock>) -> StateFile {
        StateFile {
            epics: Vec::new(),
            repo: "bastion".to_owned(),
            kind: "project".to_owned(),
            updated: "2026-07-23".to_owned(),
            focus: Default::default(),
            tracks: vec![Track {
                title: "Phase 11".to_owned(),
                blocks,
                ..Default::default()
            }],
            repos: Vec::new(),
            cross_repo: Vec::new(),
            tiers: Vec::new(),
            note: None,
            backlog: Vec::new(),
            carryover: Vec::new(),
            ..Default::default()
        }
    }

    fn sample_track_block(id: &str, status: Option<&str>) -> TrackBlock {
        sample_track_block_full(id, status, Vec::new(), None, None, None, Vec::new())
    }

    #[allow(clippy::too_many_arguments)]
    fn sample_track_block_full(
        id: &str,
        status: Option<&str>,
        depends_on: Vec<BlockedBy>,
        wave: Option<i64>,
        priority: Option<u8>,
        due: Option<&str>,
        epics: Vec<String>,
    ) -> TrackBlock {
        TrackBlock {
            epics,
            id: id.to_owned(),
            title: format!("{id} title"),
            status: status.map(|s| s.to_owned()),
            depends_on,
            wave,
            origin: None,
            priority,
            due: due.map(|d| d.to_owned()),
            sdlc_workflow: None,
            model: None,
            note: None,
            description: None,
            ..Default::default()
        }
    }

    fn sample_source(repo: &str) -> StateSource {
        StateSource {
            repo_slug: repo.to_owned(),
            abs_path: PathBuf::from(format!("/tmp/{repo}/planning/state.json")),
            expected_kind: "project",
        }
    }

    #[test]
    fn finished_blocks_filters_to_closed_status() {
        let file = sample_state_file(vec![
            sample_track_block("BA.1.A", Some("closed")),
            sample_track_block("BA.1.B", Some("open")),
            sample_track_block("BA.1.C", Some("in_progress")),
            sample_track_block("BA.1.D", None),
        ]);
        let files = vec![(sample_source("bastion"), file)];
        let index = track_block_index("bastion", &files);
        let status_map = block_status_map(&files);

        let finished = finished_blocks_for_repo(
            "bastion",
            &index,
            &status_map,
            &HashMap::new(),
            &HashMap::new(),
        );
        assert_eq!(finished.len(), 1);
        assert_eq!(finished[0].id, "BA.1.A");
        assert_eq!(finished[0].repo, "bastion");
        assert_eq!(finished[0].status.as_deref(), Some("closed"));
    }

    #[test]
    fn finished_blocks_empty_when_repo_not_loaded() {
        let files: Vec<(StateSource, StateFile)> = Vec::new();
        let index = track_block_index("bastion", &files);
        let status_map = block_status_map(&files);
        assert!(
            finished_blocks_for_repo(
                "bastion",
                &index,
                &status_map,
                &HashMap::new(),
                &HashMap::new()
            )
            .is_empty()
        );
    }

    #[test]
    fn finished_blocks_empty_when_no_closed_blocks() {
        let file = sample_state_file(vec![sample_track_block("BA.1.A", Some("open"))]);
        let files = vec![(sample_source("bastion"), file)];
        let index = track_block_index("bastion", &files);
        let status_map = block_status_map(&files);
        assert!(
            finished_blocks_for_repo(
                "bastion",
                &index,
                &status_map,
                &HashMap::new(),
                &HashMap::new()
            )
            .is_empty()
        );
    }

    // ── track_block_index ─────────────────────────────────────────────────

    #[test]
    fn track_block_index_maps_id_to_block_and_track_title() {
        let file = StateFile {
            tracks: vec![
                Track {
                    title: "Phase 1".to_owned(),
                    blocks: vec![sample_track_block("BA.1.A", Some("open"))],
                    ..Default::default()
                },
                Track {
                    title: "Phase 2".to_owned(),
                    blocks: vec![sample_track_block("BA.2.A", Some("open"))],
                    ..Default::default()
                },
            ],
            ..sample_state_file(Vec::new())
        };
        let files = vec![(sample_source("bastion"), file)];

        let index = track_block_index("bastion", &files);
        assert_eq!(index.len(), 2);
        let (block, track_title) = index["BA.1.A"];
        assert_eq!(block.id, "BA.1.A");
        assert_eq!(track_title, "Phase 1");
        let (_, track_title2) = index["BA.2.A"];
        assert_eq!(track_title2, "Phase 2");
    }

    #[test]
    fn track_block_index_empty_when_repo_not_loaded() {
        let files: Vec<(StateSource, StateFile)> = Vec::new();
        assert!(track_block_index("bastion", &files).is_empty());
    }

    #[test]
    fn track_block_index_last_wins_on_duplicate_id() {
        let file = StateFile {
            tracks: vec![
                Track {
                    title: "Phase 1".to_owned(),
                    blocks: vec![sample_track_block("BA.1.A", Some("open"))],
                    ..Default::default()
                },
                Track {
                    title: "Phase 2".to_owned(),
                    blocks: vec![sample_track_block("BA.1.A", Some("closed"))],
                    ..Default::default()
                },
            ],
            ..sample_state_file(Vec::new())
        };
        let files = vec![(sample_source("bastion"), file)];

        let index = track_block_index("bastion", &files);
        assert_eq!(index.len(), 1);
        let (_, track_title) = index["BA.1.A"];
        assert_eq!(track_title, "Phase 2");
    }

    // ── enrich_block ──────────────────────────────────────────────────────

    #[test]
    fn enrich_block_fills_all_six_fields_when_matched() {
        let track_block = sample_track_block_full(
            "BA.1.A",
            Some("open"),
            Vec::new(),
            Some(3),
            Some(1),
            Some("2026-08-01"),
            vec!["epic-alpha".to_owned()],
        );
        let mut dto = board_block_from(
            &sample_block("BA.1.A", Some("open"), Vec::new()),
            "bastion",
            &HashMap::new(),
            &HashMap::new(),
        );

        enrich_block(&mut dto, Some((&track_block, "Phase 11")));

        assert_eq!(dto.epics, vec!["epic-alpha".to_owned()]);
        assert_eq!(dto.wave, Some(3));
        assert_eq!(dto.priority, Some(1));
        assert_eq!(dto.due, Some("2026-08-01".to_owned()));
        assert_eq!(dto.track, Some("Phase 11".to_owned()));
        assert_eq!(dto.status, Some("open".to_owned()));
    }

    #[test]
    fn enrich_block_leaves_defaults_when_no_fields_authored() {
        let track_block = sample_track_block("BA.1.A", None);
        let mut dto = board_block_from(
            &sample_block("BA.1.A", Some("open"), Vec::new()),
            "bastion",
            &HashMap::new(),
            &HashMap::new(),
        );

        enrich_block(&mut dto, Some((&track_block, "Phase 11")));

        assert!(dto.epics.is_empty());
        assert_eq!(dto.wave, None);
        assert_eq!(dto.priority, None);
        assert_eq!(dto.due, None);
        assert_eq!(dto.track, Some("Phase 11".to_owned()));
        assert_eq!(dto.status, None);
    }

    #[test]
    fn enrich_block_untouched_when_id_unmatched() {
        let mut dto = board_block_from(
            &sample_block("BA.1.A", Some("open"), Vec::new()),
            "bastion",
            &HashMap::new(),
            &HashMap::new(),
        );

        enrich_block(&mut dto, None);

        assert!(dto.epics.is_empty());
        assert_eq!(dto.wave, None);
        assert_eq!(dto.priority, None);
        assert_eq!(dto.due, None);
        assert_eq!(dto.track, None);
        assert_eq!(dto.status, Some("open".to_owned()));
    }

    // ── unmet_deps ────────────────────────────────────────────────────────

    #[test]
    fn unmet_deps_empty_when_no_deps() {
        let status_map = HashMap::new();
        assert!(unmet_deps(&[], &status_map).is_empty());
    }

    #[test]
    fn unmet_deps_closed_target_is_met() {
        let mut status_map = HashMap::new();
        status_map.insert("bastion:BA.1.A".to_owned(), Some("closed".to_owned()));
        let deps = vec![BlockedBy::Block {
            repo: "bastion".to_owned(),
            id: "BA.1.A".to_owned(),
            what: None,
        }];
        assert!(unmet_deps(&deps, &status_map).is_empty());
    }

    #[test]
    fn unmet_deps_non_closed_target_is_unmet() {
        let mut status_map = HashMap::new();
        status_map.insert("bastion:BA.1.A".to_owned(), Some("open".to_owned()));
        let deps = vec![BlockedBy::Block {
            repo: "bastion".to_owned(),
            id: "BA.1.A".to_owned(),
            what: None,
        }];
        assert_eq!(unmet_deps(&deps, &status_map), deps);
    }

    #[test]
    fn unmet_deps_absent_target_is_unmet() {
        let status_map: HashMap<String, Option<String>> = HashMap::new();
        let deps = vec![BlockedBy::Block {
            repo: "bastion".to_owned(),
            id: "BA.1.A".to_owned(),
            what: None,
        }];
        assert_eq!(unmet_deps(&deps, &status_map), deps);
    }

    #[test]
    fn unmet_deps_external_always_unmet() {
        let status_map = HashMap::new();
        let deps = vec![BlockedBy::External {
            what: "reviewer availability".to_owned(),
        }];
        assert_eq!(unmet_deps(&deps, &status_map), deps);
    }

    #[test]
    fn unmet_deps_mixed_filters_to_unmet_subset() {
        let mut status_map = HashMap::new();
        status_map.insert("bastion:BA.1.A".to_owned(), Some("closed".to_owned()));
        status_map.insert("bastion:BA.1.B".to_owned(), Some("open".to_owned()));
        let closed_dep = BlockedBy::Block {
            repo: "bastion".to_owned(),
            id: "BA.1.A".to_owned(),
            what: None,
        };
        let open_dep = BlockedBy::Block {
            repo: "bastion".to_owned(),
            id: "BA.1.B".to_owned(),
            what: None,
        };
        let deps = vec![closed_dep, open_dep.clone()];
        assert_eq!(unmet_deps(&deps, &status_map), vec![open_dep]);
    }

    #[test]
    fn unmet_deps_operator_always_unmet() {
        let status_map = HashMap::new();
        let deps = vec![BlockedBy::Operator {
            slug: "session-mac-mini".to_owned(),
            exit: "planning/handoff.md".to_owned(),
            start: "/begin-session mac-mini".to_owned(),
            what: None,
        }];
        assert_eq!(unmet_deps(&deps, &status_map), deps);
    }

    #[test]
    fn unmet_deps_approval_always_unmet() {
        let status_map = HashMap::new();
        let deps = vec![BlockedBy::Approval {
            slug: "approve-devto-sweep".to_owned(),
            what: "approve four one-line diffs".to_owned(),
            digest: "sha256:abc123".to_owned(),
        }];
        assert_eq!(unmet_deps(&deps, &status_map), deps);
    }

    /// The targetless variants are unmet on their own terms — a `status_map`
    /// entry that happens to share their slug must not satisfy them, because
    /// they carry no `repo:id` target to look up in the first place.
    #[test]
    fn unmet_deps_targetless_ignores_status_map() {
        let mut status_map = HashMap::new();
        status_map.insert(
            "bastion:session-mac-mini".to_owned(),
            Some("closed".to_owned()),
        );
        status_map.insert(
            "bastion:approve-devto-sweep".to_owned(),
            Some("closed".to_owned()),
        );
        let deps = vec![
            BlockedBy::Operator {
                slug: "session-mac-mini".to_owned(),
                exit: "planning/handoff.md".to_owned(),
                start: "/begin-session mac-mini".to_owned(),
                what: None,
            },
            BlockedBy::Approval {
                slug: "approve-devto-sweep".to_owned(),
                what: "approve four one-line diffs".to_owned(),
                digest: "sha256:abc123".to_owned(),
            },
        ];
        assert_eq!(unmet_deps(&deps, &status_map), deps);
    }

    /// A closed block dep alongside the two targetless variants filters down to
    /// exactly the targetless pair — the mixed case the board actually renders.
    #[test]
    fn unmet_deps_mixed_keeps_targetless_drops_closed_block() {
        let mut status_map = HashMap::new();
        status_map.insert("bastion:BA.1.A".to_owned(), Some("closed".to_owned()));
        let closed_dep = BlockedBy::Block {
            repo: "bastion".to_owned(),
            id: "BA.1.A".to_owned(),
            what: None,
        };
        let operator_dep = BlockedBy::Operator {
            slug: "session-telegram-bot".to_owned(),
            exit: "token in the Mini's plist".to_owned(),
            start: "/begin-session telegram-bot".to_owned(),
            what: None,
        };
        let approval_dep = BlockedBy::Approval {
            slug: "approve-payload".to_owned(),
            what: "approve the operator payload contract".to_owned(),
            digest: "sha256:def456".to_owned(),
        };
        let deps = vec![closed_dep, operator_dep.clone(), approval_dep.clone()];
        assert_eq!(
            unmet_deps(&deps, &status_map),
            vec![operator_dep, approval_dep]
        );
    }

    // ── block_status_map ──────────────────────────────────────────────────

    #[test]
    fn block_status_map_keys_by_repo_and_id() {
        let file = sample_state_file(vec![sample_track_block("BA.1.A", Some("closed"))]);
        let files = vec![(sample_source("bastion"), file)];
        let map = block_status_map(&files);
        assert_eq!(
            map.get("bastion:BA.1.A").cloned().flatten(),
            Some("closed".to_owned())
        );
    }

    // ── build_board ────────────────────────────────────────────────────────

    fn sample_block(id: &str, status: Option<&str>, blocked_by: Vec<BlockedBy>) -> Block {
        Block {
            epics: Vec::new(),
            id: id.to_owned(),
            title: format!("{id} title"),
            status: status.map(|s| s.to_owned()),
            note: None,
            repo: None,
            blocked_by,
            priority: None,
            due: None,
        }
    }

    fn sample_rollup(repo: &str, tier: &str) -> RepoRollup {
        RepoRollup {
            repo: repo.to_owned(),
            tier: Some(tier.to_owned()),
            now: vec![sample_block("BA.1.A", Some("in_progress"), Vec::new())],
            next: vec![sample_block("BA.1.B", None, Vec::new())],
            blocked: vec![sample_block(
                "BA.1.C",
                None,
                vec![BlockedBy::External {
                    what: "reviewer availability".to_owned(),
                }],
            )],
            deferred: Vec::new(),
        }
    }

    #[test]
    fn build_board_maps_lanes_and_tags_repo() {
        let rollups = vec![sample_rollup("bastion", "core")];
        let files: Vec<(StateSource, StateFile)> = Vec::new();

        let dto = build_board(
            BoardScope::Tier,
            Some("core".to_owned()),
            &rollups,
            &files,
            false,
            &HashMap::new(),
            &HashMap::new(),
        );

        assert_eq!(dto.scope, BoardScope::Tier);
        assert_eq!(dto.tier, Some("core".to_owned()));
        assert!(!dto.stale);
        assert_eq!(dto.lanes.now.len(), 1);
        assert_eq!(dto.lanes.now[0].id, "BA.1.A");
        assert_eq!(dto.lanes.now[0].repo, "bastion");
        assert_eq!(dto.lanes.next[0].id, "BA.1.B");
        assert_eq!(dto.lanes.blocked[0].id, "BA.1.C");
        assert!(
            dto.lanes.deferred.is_empty(),
            "no deferred blocks in the fixture"
        );
        assert_eq!(dto.repos.len(), 1);
        assert_eq!(dto.repos[0].repo, "bastion");
        assert_eq!(dto.repos[0].tier, Some("core".to_owned()));
    }

    #[test]
    fn build_board_maps_the_deferred_lane_and_keeps_it_out_of_next() {
        let mut rollup = sample_rollup("bastion", "core");
        rollup.deferred = vec![sample_block("BA.9.A", Some("deferred"), Vec::new())];
        let files: Vec<(StateSource, StateFile)> = Vec::new();

        let dto = build_board(
            BoardScope::Tier,
            Some("core".to_owned()),
            &[rollup],
            &files,
            false,
            &HashMap::new(),
            &HashMap::new(),
        );

        assert_eq!(dto.lanes.deferred.len(), 1);
        assert_eq!(dto.lanes.deferred[0].id, "BA.9.A");
        assert_eq!(dto.lanes.deferred[0].repo, "bastion");
        assert_eq!(dto.lanes.deferred[0].status.as_deref(), Some("deferred"));
        assert!(
            !dto.lanes.next.iter().any(|b| b.id == "BA.9.A"),
            "deferred work must never appear in the next lane"
        );
        assert_eq!(dto.repos[0].lanes.deferred.len(), 1);
    }

    #[test]
    fn build_board_preserves_blocked_by() {
        let rollups = vec![sample_rollup("bastion", "core")];
        let files: Vec<(StateSource, StateFile)> = Vec::new();

        let dto = build_board(
            BoardScope::Tier,
            Some("core".to_owned()),
            &rollups,
            &files,
            false,
            &HashMap::new(),
            &HashMap::new(),
        );

        assert_eq!(dto.lanes.blocked[0].blocked_by.len(), 1);
        assert_eq!(
            dto.lanes.blocked[0].blocked_by[0],
            BlockedBy::External {
                what: "reviewer availability".to_owned()
            }
        );
    }

    #[test]
    fn build_board_derives_finished_lane_from_files() {
        let rollups = vec![sample_rollup("bastion", "core")];
        let file = sample_state_file(vec![sample_track_block("BA.1.Z", Some("closed"))]);
        let files = vec![(sample_source("bastion"), file)];

        let dto = build_board(
            BoardScope::Hq,
            None,
            &rollups,
            &files,
            false,
            &HashMap::new(),
            &HashMap::new(),
        );

        assert_eq!(dto.lanes.finished.len(), 1);
        assert_eq!(dto.lanes.finished[0].id, "BA.1.Z");
        assert_eq!(dto.repos[0].lanes.finished.len(), 1);
    }

    #[test]
    fn build_board_enriches_now_next_finished_lanes_from_tracks() {
        let rollups = vec![sample_rollup("bastion", "core")];
        let file = StateFile {
            tracks: vec![Track {
                title: "Phase 11".to_owned(),
                blocks: vec![
                    sample_track_block_full(
                        "BA.1.A",
                        Some("in_progress"),
                        Vec::new(),
                        Some(2),
                        Some(1),
                        Some("2026-08-01"),
                        vec!["epic-alpha".to_owned()],
                    ),
                    sample_track_block_full(
                        "BA.1.B",
                        None,
                        Vec::new(),
                        Some(3),
                        Some(2),
                        Some("2026-08-15"),
                        vec!["epic-beta".to_owned()],
                    ),
                ],
                ..Default::default()
            }],
            ..sample_state_file(Vec::new())
        };
        let files = vec![(sample_source("bastion"), file)];

        let dto = build_board(
            BoardScope::Hq,
            None,
            &rollups,
            &files,
            false,
            &HashMap::new(),
            &HashMap::new(),
        );

        let now = &dto.lanes.now[0];
        assert_eq!(now.epics, vec!["epic-alpha".to_owned()]);
        assert_eq!(now.wave, Some(2));
        assert_eq!(now.priority, Some(1));
        assert_eq!(now.due, Some("2026-08-01".to_owned()));
        assert_eq!(now.track, Some("Phase 11".to_owned()));

        let next = &dto.lanes.next[0];
        assert_eq!(next.epics, vec!["epic-beta".to_owned()]);
        assert_eq!(next.wave, Some(3));
        assert_eq!(next.track, Some("Phase 11".to_owned()));
    }

    #[test]
    fn build_board_next_lane_reports_authored_status_not_fabricated_null() {
        // Regression test for BA.ticket.enrich-block-authored-status: derive_rollup
        // fabricates `None` for every `next`-lane block regardless of its authored
        // status. `enrich_block` must overwrite that with the real tracks[] value.
        let rollups = vec![sample_rollup("bastion", "core")];
        let file = sample_state_file(vec![sample_track_block("BA.1.B", Some("open"))]);
        let files = vec![(sample_source("bastion"), file)];

        let dto = build_board(
            BoardScope::Hq,
            None,
            &rollups,
            &files,
            false,
            &HashMap::new(),
            &HashMap::new(),
        );

        assert_eq!(dto.lanes.next[0].status, Some("open".to_owned()));
    }

    #[test]
    fn build_board_now_lane_reports_authored_status_not_fabricated_in_progress() {
        // Regression test for BA.ticket.enrich-block-authored-status: derive_rollup
        // fabricates `Some("in_progress")` for every `now`-lane block regardless of
        // its authored status. `enrich_block` must overwrite that with the real
        // tracks[] value, even when it disagrees with the fabricated placeholder.
        let rollups = vec![sample_rollup("bastion", "core")];
        let file = sample_state_file(vec![sample_track_block("BA.1.A", Some("open"))]);
        let files = vec![(sample_source("bastion"), file)];

        let dto = build_board(
            BoardScope::Hq,
            None,
            &rollups,
            &files,
            false,
            &HashMap::new(),
            &HashMap::new(),
        );

        assert_eq!(dto.lanes.now[0].status, Some("open".to_owned()));
    }

    #[test]
    fn build_board_unmatched_block_yields_empty_epics_and_none_fields() {
        let rollups = vec![sample_rollup("bastion", "core")];
        let files: Vec<(StateSource, StateFile)> = Vec::new();

        let dto = build_board(
            BoardScope::Hq,
            None,
            &rollups,
            &files,
            false,
            &HashMap::new(),
            &HashMap::new(),
        );

        let now = &dto.lanes.now[0];
        assert!(now.epics.is_empty());
        assert_eq!(now.wave, None);
        assert_eq!(now.priority, None);
        assert_eq!(now.due, None);
        assert_eq!(now.track, None);
    }

    #[test]
    fn build_board_next_lane_blocked_by_reflects_unmet_deps() {
        let mut rollup = sample_rollup("bastion", "core");
        rollup.next = vec![
            sample_block("BA.1.B", None, Vec::new()),
            sample_block("BA.1.E", None, Vec::new()),
        ];
        let rollups = vec![rollup];

        let file = StateFile {
            tracks: vec![Track {
                title: "Phase 11".to_owned(),
                blocks: vec![
                    // BA.1.B depends on BA.1.X, which is not closed -> unmet.
                    sample_track_block_full(
                        "BA.1.B",
                        None,
                        vec![BlockedBy::Block {
                            repo: "bastion".to_owned(),
                            id: "BA.1.X".to_owned(),
                            what: None,
                        }],
                        None,
                        None,
                        None,
                        Vec::new(),
                    ),
                    sample_track_block("BA.1.X", Some("open")),
                    // BA.1.E depends on BA.1.X too, but it's closed here in a
                    // second scenario block — instead give it a closed dep.
                    sample_track_block_full(
                        "BA.1.E",
                        None,
                        vec![BlockedBy::Block {
                            repo: "bastion".to_owned(),
                            id: "BA.1.Y".to_owned(),
                            what: None,
                        }],
                        None,
                        None,
                        None,
                        Vec::new(),
                    ),
                    sample_track_block("BA.1.Y", Some("closed")),
                ],
                ..Default::default()
            }],
            ..sample_state_file(Vec::new())
        };
        let files = vec![(sample_source("bastion"), file)];

        let dto = build_board(
            BoardScope::Hq,
            None,
            &rollups,
            &files,
            false,
            &HashMap::new(),
            &HashMap::new(),
        );

        let blocked_entry = dto.lanes.next.iter().find(|b| b.id == "BA.1.B").unwrap();
        assert_eq!(blocked_entry.blocked_by.len(), 1);
        let ready_entry = dto.lanes.next.iter().find(|b| b.id == "BA.1.E").unwrap();
        assert!(ready_entry.blocked_by.is_empty());
    }

    #[test]
    fn build_board_blocked_lane_blocked_by_unchanged_by_enrichment() {
        let rollups = vec![sample_rollup("bastion", "core")];
        // Even if tracks[] authors a different depends_on for the same id, the
        // blocked lane must keep the rollup's own `unmet` list untouched.
        let file = StateFile {
            tracks: vec![Track {
                title: "Phase 11".to_owned(),
                blocks: vec![sample_track_block_full(
                    "BA.1.C",
                    None,
                    Vec::new(),
                    None,
                    None,
                    None,
                    vec!["epic-gamma".to_owned()],
                )],
                ..Default::default()
            }],
            ..sample_state_file(Vec::new())
        };
        let files = vec![(sample_source("bastion"), file)];

        let dto = build_board(
            BoardScope::Hq,
            None,
            &rollups,
            &files,
            false,
            &HashMap::new(),
            &HashMap::new(),
        );

        let blocked = &dto.lanes.blocked[0];
        assert_eq!(blocked.epics, vec!["epic-gamma".to_owned()]);
        assert_eq!(blocked.blocked_by.len(), 1);
        assert_eq!(
            blocked.blocked_by[0],
            BlockedBy::External {
                what: "reviewer availability".to_owned()
            }
        );
    }

    // ── last_touched (BA.11.S task 2) ────────────────────────────────────────

    /// A rollup with the same block id (`BA.1.A`) surfaced on all five lanes
    /// (`now`/`next`/`blocked`/`deferred`/`finished`), so one `last_touched`
    /// test can assert the map is consulted on every lane, not just one.
    fn all_lanes_rollup(repo: &str) -> RepoRollup {
        RepoRollup {
            repo: repo.to_owned(),
            tier: Some("core".to_owned()),
            now: vec![sample_block("BA.1.A", Some("in_progress"), Vec::new())],
            next: vec![sample_block("BA.1.A", Some("open"), Vec::new())],
            blocked: vec![sample_block("BA.1.A", Some("open"), Vec::new())],
            deferred: vec![sample_block("BA.1.A", Some("deferred"), Vec::new())],
        }
    }

    /// `finished` isn't sourced from the rollup — it comes from `files`'
    /// `tracks[].blocks[]` via `finished_blocks_for_repo`, so a closed
    /// `BA.1.A` in the fixture file drives that lane.
    fn all_lanes_files(repo: &str) -> Vec<(StateSource, StateFile)> {
        vec![(
            sample_source(repo),
            sample_state_file(vec![sample_track_block("BA.1.A", Some("closed"))]),
        )]
    }

    /// Pins the mev boundary conversion: the board keeps `updated_at` and drops
    /// `status`. Guards the E0308 that mev's `ticket-reconcile-failed-consumer`
    /// caused here — if mev reshapes `LastTouched` again, this fails with a clear
    /// reason instead of only the call site failing to compile.
    #[test]
    fn flatten_last_touched_keeps_updated_at_and_drops_status() {
        use mev::brain::last_touched::LastTouched;

        let mut input = HashMap::new();
        input.insert(
            "bastion:BA.1.A".to_owned(),
            LastTouched {
                updated_at: "2026-07-28T12:00:00Z".to_owned(),
                status: Some("reconcile_failed".to_owned()),
            },
        );
        input.insert(
            "bastion:BA.1.B".to_owned(),
            LastTouched {
                updated_at: "2026-07-29T09:30:00Z".to_owned(),
                status: None,
            },
        );

        let flattened = flatten_last_touched(input);

        assert_eq!(flattened.len(), 2);
        assert_eq!(
            flattened.get("bastion:BA.1.A").map(String::as_str),
            Some("2026-07-28T12:00:00Z"),
            "a non-None status must not change which value survives"
        );
        assert_eq!(
            flattened.get("bastion:BA.1.B").map(String::as_str),
            Some("2026-07-29T09:30:00Z")
        );
    }

    #[test]
    fn flatten_last_touched_on_an_empty_map_yields_an_empty_map() {
        assert!(flatten_last_touched(HashMap::new()).is_empty());
    }

    #[test]
    fn build_board_populates_last_touched_on_every_lane_when_key_matches() {
        let rollups = vec![all_lanes_rollup("bastion")];
        let files = all_lanes_files("bastion");
        let mut last_touched = HashMap::new();
        last_touched.insert(
            "bastion:BA.1.A".to_owned(),
            "2026-07-28T12:00:00Z".to_owned(),
        );

        let dto = build_board(
            BoardScope::Hq,
            None,
            &rollups,
            &files,
            false,
            &last_touched,
            &HashMap::new(),
        );

        assert_eq!(
            dto.lanes.now[0].last_touched.as_deref(),
            Some("2026-07-28T12:00:00Z"),
            "now lane"
        );
        assert_eq!(
            dto.lanes.next[0].last_touched.as_deref(),
            Some("2026-07-28T12:00:00Z"),
            "next lane"
        );
        assert_eq!(
            dto.lanes.blocked[0].last_touched.as_deref(),
            Some("2026-07-28T12:00:00Z"),
            "blocked lane"
        );
        assert_eq!(
            dto.lanes.deferred[0].last_touched.as_deref(),
            Some("2026-07-28T12:00:00Z"),
            "deferred lane"
        );
        assert_eq!(
            dto.lanes.finished[0].last_touched.as_deref(),
            Some("2026-07-28T12:00:00Z"),
            "finished lane"
        );
    }

    #[test]
    fn build_board_leaves_last_touched_none_when_key_absent() {
        let rollups = vec![all_lanes_rollup("bastion")];
        let files = all_lanes_files("bastion");
        // A populated map that simply has no entry for this block/repo.
        let mut last_touched = HashMap::new();
        last_touched.insert(
            "bastion:BA.9.Z".to_owned(),
            "2026-07-28T12:00:00Z".to_owned(),
        );

        let dto = build_board(
            BoardScope::Hq,
            None,
            &rollups,
            &files,
            false,
            &last_touched,
            &HashMap::new(),
        );

        assert_eq!(dto.lanes.now[0].last_touched, None);
        assert_eq!(dto.lanes.next[0].last_touched, None);
        assert_eq!(dto.lanes.blocked[0].last_touched, None);
        assert_eq!(dto.lanes.deferred[0].last_touched, None);
        assert_eq!(dto.lanes.finished[0].last_touched, None);
    }

    #[test]
    fn build_board_last_touched_does_not_leak_across_repos_with_same_block_id() {
        let rollups = vec![all_lanes_rollup("bastion"), all_lanes_rollup("bella")];
        let mut files = all_lanes_files("bastion");
        files.extend(all_lanes_files("bella"));
        // Only `bella:BA.1.A` has a recorded timestamp; `bastion:BA.1.A` (same
        // block id, different repo) must not pick it up.
        let mut last_touched = HashMap::new();
        last_touched.insert("bella:BA.1.A".to_owned(), "2026-07-28T12:00:00Z".to_owned());

        let dto = build_board(
            BoardScope::Hq,
            None,
            &rollups,
            &files,
            false,
            &last_touched,
            &HashMap::new(),
        );

        let bastion_now = dto.lanes.now.iter().find(|b| b.repo == "bastion").unwrap();
        assert_eq!(
            bastion_now.last_touched, None,
            "bastion must not leak bella's value"
        );

        let bella_now = dto.lanes.now.iter().find(|b| b.repo == "bella").unwrap();
        assert_eq!(
            bella_now.last_touched.as_deref(),
            Some("2026-07-28T12:00:00Z")
        );
    }

    #[test]
    fn build_board_empty_last_touched_map_leaves_every_lane_none() {
        let rollups = vec![all_lanes_rollup("bastion")];
        let files = all_lanes_files("bastion");

        let dto = build_board(
            BoardScope::Hq,
            None,
            &rollups,
            &files,
            false,
            &HashMap::new(),
            &HashMap::new(),
        );

        assert_eq!(dto.lanes.now[0].last_touched, None);
        assert_eq!(dto.lanes.next[0].last_touched, None);
        assert_eq!(dto.lanes.blocked[0].last_touched, None);
        assert_eq!(dto.lanes.deferred[0].last_touched, None);
        assert_eq!(dto.lanes.finished[0].last_touched, None);
    }

    // ── block_graph enrichment (dependent_count/ready/unmet_count) ──────────

    /// A populated `block_graph` map with one entry for `bastion:BA.1.A` —
    /// the id every `all_lanes_rollup`/`all_lanes_files` block shares — with
    /// a distinguishable `dependent_count`/`ready`/`unmet_count` so lane
    /// assertions can't pass by accident on a default value.
    fn sample_block_graph_map() -> HashMap<String, BlockEnrichment> {
        let mut map = HashMap::new();
        map.insert(
            "bastion:BA.1.A".to_owned(),
            BlockEnrichment {
                dependent_count: 7,
                ready: true,
                unmet_count: 2,
            },
        );
        map
    }

    #[test]
    fn build_board_populates_dependent_count_and_ready_on_every_lane_when_key_matches() {
        let rollups = vec![all_lanes_rollup("bastion")];
        let files = all_lanes_files("bastion");
        let block_graph = sample_block_graph_map();

        let dto = build_board(
            BoardScope::Hq,
            None,
            &rollups,
            &files,
            false,
            &HashMap::new(),
            &block_graph,
        );

        for (lane_name, entry) in [
            ("now", &dto.lanes.now[0]),
            ("next", &dto.lanes.next[0]),
            ("blocked", &dto.lanes.blocked[0]),
            ("deferred", &dto.lanes.deferred[0]),
            ("finished", &dto.lanes.finished[0]),
        ] {
            assert_eq!(
                entry.dependent_count,
                Some(7),
                "{lane_name} lane dependent_count"
            );
            assert_eq!(entry.ready, Some(true), "{lane_name} lane ready");
        }
    }

    #[test]
    fn build_board_populates_unmet_count_only_on_blocked_lane() {
        let rollups = vec![all_lanes_rollup("bastion")];
        let files = all_lanes_files("bastion");
        let block_graph = sample_block_graph_map();

        let dto = build_board(
            BoardScope::Hq,
            None,
            &rollups,
            &files,
            false,
            &HashMap::new(),
            &block_graph,
        );

        assert_eq!(
            dto.lanes.blocked[0].unmet_count,
            Some(2),
            "blocked lane surfaces mev's raw unmet_count"
        );
        assert_eq!(dto.lanes.now[0].unmet_count, None, "now lane");
        assert_eq!(dto.lanes.next[0].unmet_count, None, "next lane");
        assert_eq!(dto.lanes.deferred[0].unmet_count, None, "deferred lane");
        assert_eq!(dto.lanes.finished[0].unmet_count, None, "finished lane");
    }

    #[test]
    fn build_board_block_absent_from_graph_map_yields_none_for_all_three_fields() {
        let rollups = vec![all_lanes_rollup("bastion")];
        let files = all_lanes_files("bastion");
        // Populated map, but with no entry for this fixture's block id —
        // must yield `None`, never a fabricated `0`/`false`.
        let mut block_graph = HashMap::new();
        block_graph.insert(
            "bastion:BA.9.Z".to_owned(),
            BlockEnrichment {
                dependent_count: 9,
                ready: true,
                unmet_count: 1,
            },
        );

        let dto = build_board(
            BoardScope::Hq,
            None,
            &rollups,
            &files,
            false,
            &HashMap::new(),
            &block_graph,
        );

        for (lane_name, entry) in [
            ("now", &dto.lanes.now[0]),
            ("next", &dto.lanes.next[0]),
            ("blocked", &dto.lanes.blocked[0]),
            ("deferred", &dto.lanes.deferred[0]),
            ("finished", &dto.lanes.finished[0]),
        ] {
            assert_eq!(
                entry.dependent_count, None,
                "{lane_name} lane dependent_count"
            );
            assert_eq!(entry.ready, None, "{lane_name} lane ready");
            assert_eq!(entry.unmet_count, None, "{lane_name} lane unmet_count");
        }
    }

    #[test]
    fn build_board_empty_block_graph_map_leaves_every_lane_none() {
        let rollups = vec![all_lanes_rollup("bastion")];
        let files = all_lanes_files("bastion");

        let dto = build_board(
            BoardScope::Hq,
            None,
            &rollups,
            &files,
            false,
            &HashMap::new(),
            &HashMap::new(),
        );

        for entry in [
            &dto.lanes.now[0],
            &dto.lanes.next[0],
            &dto.lanes.blocked[0],
            &dto.lanes.deferred[0],
            &dto.lanes.finished[0],
        ] {
            assert_eq!(entry.dependent_count, None);
            assert_eq!(entry.ready, None);
            assert_eq!(entry.unmet_count, None);
        }
    }

    #[test]
    fn build_board_empty_rollups_yields_empty_board() {
        let dto = build_board(
            BoardScope::Hq,
            None,
            &[],
            &[],
            false,
            &HashMap::new(),
            &HashMap::new(),
        );
        assert!(dto.lanes.now.is_empty());
        assert!(dto.lanes.next.is_empty());
        assert!(dto.lanes.blocked.is_empty());
        assert!(dto.lanes.finished.is_empty());
        assert!(dto.repos.is_empty());
    }

    #[test]
    fn build_board_threads_stale_flag() {
        let dto = build_board(
            BoardScope::Hq,
            None,
            &[],
            &[],
            true,
            &HashMap::new(),
            &HashMap::new(),
        );
        assert!(dto.stale);
    }

    // ── filter_board_to_epic ─────────────────────────────────────────────────

    /// A two-repo board: `bastion` has a `now`-lane block tagged with both
    /// `epic-alpha` and `epic-beta`, `bella` has a `now`-lane block tagged
    /// only with `epic-alpha`, and `mev` has a `now`-lane block tagged with
    /// neither (so `mev` should be dropped entirely once filtered).
    fn multi_repo_epic_board() -> BoardDto {
        let rollups = vec![
            sample_rollup("bastion", "core"),
            sample_rollup("bella", "core"),
            sample_rollup("mev", "core"),
        ];

        let bastion_file = StateFile {
            tracks: vec![Track {
                title: "Phase 11".to_owned(),
                blocks: vec![sample_track_block_full(
                    "BA.1.A",
                    Some("in_progress"),
                    Vec::new(),
                    None,
                    None,
                    None,
                    vec!["epic-alpha".to_owned(), "epic-beta".to_owned()],
                )],
                ..Default::default()
            }],
            ..sample_state_file(Vec::new())
        };
        let bella_file = StateFile {
            repo: "bella".to_owned(),
            tracks: vec![Track {
                title: "Phase 1".to_owned(),
                blocks: vec![sample_track_block_full(
                    "BA.1.A",
                    Some("in_progress"),
                    Vec::new(),
                    None,
                    None,
                    None,
                    vec!["epic-alpha".to_owned()],
                )],
                ..Default::default()
            }],
            ..sample_state_file(Vec::new())
        };
        let mev_file = StateFile {
            repo: "mev".to_owned(),
            tracks: vec![Track {
                title: "Phase 1".to_owned(),
                blocks: vec![sample_track_block("BA.1.A", Some("in_progress"))],
                ..Default::default()
            }],
            ..sample_state_file(Vec::new())
        };

        let files = vec![
            (sample_source("bastion"), bastion_file),
            (sample_source("bella"), bella_file),
            (sample_source("mev"), mev_file),
        ];

        build_board(
            BoardScope::Epic,
            None,
            &rollups,
            &files,
            false,
            &HashMap::new(),
            &HashMap::new(),
        )
    }

    #[test]
    fn filter_board_to_epic_keeps_only_tagged_blocks_across_repos() {
        let board = multi_repo_epic_board();
        let filtered = filter_board_to_epic(board, "epic-alpha");

        // Aggregate `now` lane: bastion's + bella's blocks, mev's dropped.
        assert_eq!(filtered.lanes.now.len(), 2);
        assert!(filtered.lanes.now.iter().any(|b| b.repo == "bastion"));
        assert!(filtered.lanes.now.iter().any(|b| b.repo == "bella"));
        assert!(!filtered.lanes.now.iter().any(|b| b.repo == "mev"));
    }

    #[test]
    fn filter_board_to_epic_keeps_a_repo_whose_only_member_is_deferred() {
        // Trap guard: the drop predicate is an all-lanes-empty conjunction. If
        // `deferred` is filtered but left out of that check, a repo whose only
        // blocks in this epic are deferred silently vanishes from repos[].
        let mut rollup = sample_rollup("mev", "core");
        rollup.now.clear();
        rollup.next.clear();
        rollup.blocked.clear();
        rollup.deferred = vec![sample_block("MV.9.A", Some("deferred"), Vec::new())];

        let file = StateFile {
            repo: "mev".to_owned(),
            tracks: vec![Track {
                title: "Phase 9".to_owned(),
                blocks: vec![sample_track_block_full(
                    "MV.9.A",
                    Some("deferred"),
                    Vec::new(),
                    None,
                    None,
                    None,
                    vec!["epic-alpha".to_owned()],
                )],
                ..Default::default()
            }],
            ..sample_state_file(Vec::new())
        };

        let board = build_board(
            BoardScope::Epic,
            None,
            &[rollup],
            &[(sample_source("mev"), file)],
            false,
            &HashMap::new(),
            &HashMap::new(),
        );
        let filtered = filter_board_to_epic(board, "epic-alpha");

        assert_eq!(
            filtered.repos.len(),
            1,
            "a deferred-only repo must stay on the epic board"
        );
        assert_eq!(filtered.repos[0].repo, "mev");
        assert_eq!(filtered.repos[0].lanes.deferred.len(), 1);
        assert_eq!(filtered.lanes.deferred.len(), 1);
        assert_eq!(filtered.lanes.deferred[0].id, "MV.9.A");
    }

    #[test]
    fn filter_board_to_epic_drops_repos_with_no_member_block() {
        let board = multi_repo_epic_board();
        let filtered = filter_board_to_epic(board, "epic-alpha");

        // `mev` contributed no block tagged epic-alpha -> dropped from repos[].
        assert_eq!(filtered.repos.len(), 2);
        assert!(filtered.repos.iter().any(|r| r.repo == "bastion"));
        assert!(filtered.repos.iter().any(|r| r.repo == "bella"));
        assert!(!filtered.repos.iter().any(|r| r.repo == "mev"));
    }

    #[test]
    fn filter_board_to_epic_block_with_two_epics_appears_on_both() {
        let board = multi_repo_epic_board();

        let alpha = filter_board_to_epic(board.clone(), "epic-alpha");
        assert!(
            alpha
                .lanes
                .now
                .iter()
                .any(|b| b.id == "BA.1.A" && b.repo == "bastion")
        );

        let beta = filter_board_to_epic(board, "epic-beta");
        assert_eq!(beta.lanes.now.len(), 1);
        assert_eq!(beta.lanes.now[0].repo, "bastion");
    }

    #[test]
    fn filter_board_to_epic_unknown_slug_yields_empty_lanes_and_no_repos() {
        let board = multi_repo_epic_board();
        let filtered = filter_board_to_epic(board, "no-such-epic");

        assert!(filtered.lanes.now.is_empty());
        assert!(filtered.lanes.next.is_empty());
        assert!(filtered.lanes.blocked.is_empty());
        assert!(filtered.lanes.finished.is_empty());
        assert!(filtered.repos.is_empty());
    }

    // ── epic_param_missing / epic_known / epic_error_response ───────────────
    // (BA.11.R error branches for `scope=epic`: missing `&epic=` param, and a
    // slug absent from the HQ registry.)

    #[test]
    fn epic_param_missing_true_when_absent() {
        assert!(epic_param_missing(None));
    }

    #[test]
    fn epic_param_missing_true_when_blank() {
        assert!(epic_param_missing(Some("   ")));
        assert!(epic_param_missing(Some("")));
    }

    #[test]
    fn epic_param_missing_false_when_present() {
        assert!(!epic_param_missing(Some("bastion-surfaces")));
    }

    #[test]
    fn epic_known_true_when_slug_in_registry() {
        let registry = vec![sample_epic("bastion-surfaces")];
        assert!(epic_known("bastion-surfaces", &registry));
    }

    #[test]
    fn epic_known_false_when_slug_absent() {
        let registry = vec![sample_epic("bastion-surfaces")];
        assert!(!epic_known("no-such-epic", &registry));
    }

    #[test]
    fn epic_known_false_when_registry_empty() {
        assert!(!epic_known("bastion-surfaces", &[]));
    }

    #[test]
    fn epic_error_response_is_404_with_c005() {
        let resp = epic_error_response("unknown epic: no-such-epic");
        assert_eq!(resp.status(), actix_web::http::StatusCode::NOT_FOUND);
    }

    fn sample_epic(slug: &str) -> okf_core::Epic {
        okf_core::Epic {
            slug: slug.to_owned(),
            title: format!("{slug} title"),
            description: None,
            status: None,
            weight: None,
            plan: None,
            repos: Vec::new(),
            ..Default::default()
        }
    }

    // ── is_stale_for_scope ────────────────────────────────────────────────────

    #[test]
    fn is_stale_true_when_in_scope_repo_has_diagnostic() {
        let diags = vec![Diagnostic::error(
            "docs/x.md",
            "E_SYNC_DRIFT",
            "repo 'bastion': watermark mismatch",
        )];
        assert!(is_stale_for_scope(&diags, &["bastion".to_owned()]));
    }

    #[test]
    fn is_stale_false_when_diagnostic_is_for_out_of_scope_repo() {
        let diags = vec![Diagnostic::error(
            "docs/x.md",
            "E_SYNC_DRIFT",
            "repo 'other-repo': watermark mismatch",
        )];
        assert!(!is_stale_for_scope(&diags, &["bastion".to_owned()]));
    }

    #[test]
    fn is_stale_false_when_no_diagnostics() {
        assert!(!is_stale_for_scope(&[], &["bastion".to_owned()]));
    }

    #[test]
    fn is_stale_false_when_no_in_scope_repos() {
        let diags = vec![Diagnostic::error(
            "docs/x.md",
            "E_SYNC_DRIFT",
            "repo 'bastion': watermark mismatch",
        )];
        assert!(!is_stale_for_scope(&diags, &[]));
    }

    // ── assemble_board — I/O shell, unresolvable brain root degrades cleanly ──

    #[test]
    fn assemble_board_on_missing_brain_toml_errors_cleanly() {
        let dir = crate::testsupport::unique_temp_dir("bastion-board-assemble-test");
        std::fs::create_dir_all(&dir).unwrap();
        let result = assemble_board(&dir, &TierScope::All, false);
        assert!(
            result.is_err(),
            "expected an error with no brain.toml present"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn assemble_board_returns_graph_matching_loaded_fixture_corpus() {
        let dir = crate::testsupport::unique_temp_dir("bastion-board-assemble-graph-test");
        let repo_planning_dir = dir.join("bastion").join("planning");
        std::fs::create_dir_all(&repo_planning_dir).unwrap();

        std::fs::write(
            dir.join("brain.toml"),
            r#"
[[repos]]
slug = "bastion"
tier = "primary"
repo_path = "bastion"
status_file = "docs/status.md"
cache_doc = "docs/projects/bastion.md"
heading = "Bastion"
"#,
        )
        .unwrap();

        std::fs::write(
            repo_planning_dir.join("state.json"),
            r#"{
  "repo": "bastion",
  "kind": "project",
  "updated": "2026-07-28",
  "tracks": [
    {
      "title": "Phase 1",
      "blocks": [
        {"id": "BA.1.A", "title": "BA.1.A title", "status": "open"},
        {"id": "BA.1.B", "title": "BA.1.B title", "status": "closed"}
      ]
    }
  ]
}"#,
        )
        .unwrap();

        let assembly = assemble_board(&dir, &TierScope::All, false)
            .expect("fixture corpus should assemble cleanly");

        let mut node_keys: Vec<String> =
            assembly.graph.nodes.iter().map(|n| n.key.clone()).collect();
        node_keys.sort();
        assert_eq!(node_keys, vec!["bastion:BA.1.A", "bastion:BA.1.B"]);
        assert_eq!(assembly.rollups.len(), 1);
        assert_eq!(assembly.files.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── task 3: block_graph enrichment map on BoardAssembly ─────────────────

    /// Build a small on-disk brain root with two repos and a `BlockedBy`
    /// fan-in (`beta:B1` depends on `alpha:A1`, so `alpha:A1` has
    /// `dependent_count == 1`) — enough to exercise the enrichment map's key
    /// shape and a non-zero `dependent_count` without depending on the larger
    /// timing fixtures below.
    fn make_block_graph_enrichment_fixture_brain_root() -> std::path::PathBuf {
        let dir =
            crate::testsupport::unique_temp_dir("bastion-board-block-graph-enrichment-fixture");
        let alpha_planning = dir.join("alpha").join("planning");
        let beta_planning = dir.join("beta").join("planning");
        std::fs::create_dir_all(&alpha_planning).unwrap();
        std::fs::create_dir_all(&beta_planning).unwrap();

        std::fs::write(
            dir.join("brain.toml"),
            r#"
[[repos]]
slug = "alpha"
tier = "core"
repo_path = "alpha"
status_file = "docs/status.md"
cache_doc = "docs/projects/alpha.md"
heading = "Alpha"

[[repos]]
slug = "beta"
tier = "core"
repo_path = "beta"
status_file = "docs/status.md"
cache_doc = "docs/projects/beta.md"
heading = "Beta"
"#,
        )
        .unwrap();

        std::fs::write(
            alpha_planning.join("state.json"),
            r#"{
  "repo": "alpha",
  "kind": "project",
  "updated": "2026-08-01",
  "tracks": [
    {
      "title": "Phase 1",
      "blocks": [
        {"id": "A1", "title": "A1 title", "status": "open"}
      ]
    }
  ]
}"#,
        )
        .unwrap();

        std::fs::write(
            beta_planning.join("state.json"),
            r#"{
  "repo": "beta",
  "kind": "project",
  "updated": "2026-08-01",
  "tracks": [
    {
      "title": "Phase 1",
      "blocks": [
        {"id": "B1", "title": "B1 title", "status": "open", "depends_on": [{"type": "block", "repo": "alpha", "id": "A1"}]}
      ]
    }
  ]
}"#,
        )
        .unwrap();

        dir
    }

    #[test]
    fn assemble_board_leaves_block_graph_empty_when_include_graph_is_false() {
        let dir = make_block_graph_enrichment_fixture_brain_root();

        let assembly = assemble_board(&dir, &TierScope::All, false)
            .expect("fixture corpus should assemble cleanly");

        assert!(
            assembly.block_graph.is_empty(),
            "block_graph must stay empty (no export built at all) when include_graph=false"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn assemble_board_populates_block_graph_keyed_repo_colon_id() {
        let dir = make_block_graph_enrichment_fixture_brain_root();

        let assembly = assemble_board(&dir, &TierScope::All, true)
            .expect("fixture corpus should assemble cleanly");

        let mut keys: Vec<&String> = assembly.block_graph.keys().collect();
        keys.sort();
        assert_eq!(keys, vec!["alpha:A1", "beta:B1"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn assemble_board_block_graph_dependent_count_matches_build_block_graph_export_directly() {
        let dir = make_block_graph_enrichment_fixture_brain_root();

        let assembly = assemble_board(&dir, &TierScope::All, true)
            .expect("fixture corpus should assemble cleanly");

        // Call build_block_graph_export directly, unscoped, over the same
        // inputs, and assert the map's values for "alpha:A1" match exactly —
        // bastion must carry mev's values verbatim, never re-derive them.
        let scope = BlockGraphScope {
            tier: TierScope::All,
            epic: None,
            repo: None,
            include_closed: true,
            include_boundary: false,
            max_nodes: usize::MAX,
        };
        let export = build_block_graph_export(
            &dir,
            &assembly.config,
            &assembly.graph,
            &assembly.files,
            &scope,
        );
        let direct_a1 = export
            .nodes
            .iter()
            .find(|n| n.key == "alpha:A1")
            .expect("alpha:A1 must be present in the direct export");

        let mapped_a1 = assembly
            .block_graph
            .get("alpha:A1")
            .expect("alpha:A1 must be present in assemble_board's enrichment map");

        assert_eq!(mapped_a1.dependent_count, direct_a1.dependent_count);
        assert_eq!(mapped_a1.dependent_count, 1, "beta:B1 depends on alpha:A1");
        assert_eq!(mapped_a1.ready, direct_a1.ready);
        assert_eq!(mapped_a1.unmet_count, direct_a1.unmet_count);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn assemble_board_block_graph_is_empty_not_panicking_for_corpus_with_no_blocks() {
        let dir = crate::testsupport::unique_temp_dir("bastion-board-block-graph-no-blocks");
        let planning_dir = dir.join("bastion").join("planning");
        std::fs::create_dir_all(&planning_dir).unwrap();

        std::fs::write(
            dir.join("brain.toml"),
            r#"
[[repos]]
slug = "bastion"
tier = "core"
repo_path = "bastion"
status_file = "docs/status.md"
cache_doc = "docs/projects/bastion.md"
heading = "Bastion"
"#,
        )
        .unwrap();

        std::fs::write(
            planning_dir.join("state.json"),
            r#"{
  "repo": "bastion",
  "kind": "project",
  "updated": "2026-08-01",
  "tracks": []
}"#,
        )
        .unwrap();

        let assembly = assemble_board(&dir, &TierScope::All, true)
            .expect("fixture corpus with no blocks should still assemble cleanly");

        assert!(
            assembly.block_graph.is_empty(),
            "a corpus with no blocks must yield an empty (not panicking) enrichment map"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── task 5: corpus-invariance headline test ─────────────────────────────
    //
    // This block's headline acceptance test: `dependent_count` must be
    // IDENTICAL for a given block whether the board is fetched at `scope=hq`
    // (`TierScope::All`) or at a narrower tier scope — the property
    // bastion-web's in-scope reverse-dep count (`lib/board-view.ts:669-676`)
    // structurally cannot have, because it counts fan-in only from whatever
    // subset of the corpus that request happened to load.

    /// Two repos in *different* tiers — `alpha` (`core`) and `beta` (`other`)
    /// — where `beta:B1` depends on `alpha:A1`. A narrower `TierScope::Tier
    /// ("core")` rollup excludes `beta` entirely, so if `alpha:A1`'s
    /// `dependent_count` were derived from the *in-scope* rollup rather than
    /// mev's corpus-wide, unscoped export, the narrower board would report `0`
    /// dependents instead of `1`. `alpha` stays in scope at both tiers so the
    /// same block can be compared across both boards.
    fn make_corpus_invariance_fixture_brain_root() -> std::path::PathBuf {
        let dir = crate::testsupport::unique_temp_dir("bastion-board-corpus-invariance-fixture");
        let alpha_planning = dir.join("alpha").join("planning");
        let beta_planning = dir.join("beta").join("planning");
        std::fs::create_dir_all(&alpha_planning).unwrap();
        std::fs::create_dir_all(&beta_planning).unwrap();

        std::fs::write(
            dir.join("brain.toml"),
            r#"
[[repos]]
slug = "alpha"
tier = "core"
repo_path = "alpha"
status_file = "docs/status.md"
cache_doc = "docs/projects/alpha.md"
heading = "Alpha"

[[repos]]
slug = "beta"
tier = "other"
repo_path = "beta"
status_file = "docs/status.md"
cache_doc = "docs/projects/beta.md"
heading = "Beta"
"#,
        )
        .unwrap();

        std::fs::write(
            alpha_planning.join("state.json"),
            r#"{
  "repo": "alpha",
  "kind": "project",
  "updated": "2026-08-01",
  "tracks": [
    {
      "title": "Phase 1",
      "blocks": [
        {"id": "A1", "title": "A1 title", "status": "open"}
      ]
    }
  ]
}"#,
        )
        .unwrap();

        std::fs::write(
            beta_planning.join("state.json"),
            r#"{
  "repo": "beta",
  "kind": "project",
  "updated": "2026-08-01",
  "tracks": [
    {
      "title": "Phase 1",
      "blocks": [
        {"id": "B1", "title": "B1 title", "status": "open", "depends_on": [{"type": "block", "repo": "alpha", "id": "A1"}]}
      ]
    }
  ]
}"#,
        )
        .unwrap();

        dir
    }

    /// Look up a `(repo, id)` block's `dependent_count` in a built [`BoardDto`]
    /// by scanning every lane — the block may land in any of the five
    /// depending on its authored status, and this helper doesn't care which.
    fn dependent_count_in_board(dto: &BoardDto, repo: &str, id: &str) -> Option<u32> {
        dto.lanes
            .now
            .iter()
            .chain(dto.lanes.next.iter())
            .chain(dto.lanes.blocked.iter())
            .chain(dto.lanes.deferred.iter())
            .chain(dto.lanes.finished.iter())
            .find(|b| b.repo == repo && b.id == id)
            .unwrap_or_else(|| panic!("{repo}:{id} missing from every board lane"))
            .dependent_count
    }

    #[test]
    fn dependent_count_is_identical_at_hq_scope_and_at_a_narrower_tier_scope() {
        let dir = make_corpus_invariance_fixture_brain_root();

        // `scope=hq` — `TierScope::All` — includes both `alpha` and `beta`.
        let hq_assembly = assemble_board(&dir, &TierScope::All, true)
            .expect("fixture corpus should assemble cleanly at hq scope");
        let hq_dto = build_board(
            BoardScope::Hq,
            None,
            &hq_assembly.rollups,
            &hq_assembly.files,
            hq_assembly.stale,
            &hq_assembly.last_touched,
            &hq_assembly.block_graph,
        );

        // `scope=tier&tier=core` — `TierScope::Tier("core")` — excludes `beta`
        // (tier `"other"`) from the rollup entirely; `alpha` stays in scope.
        let narrow_scope = TierScope::Tier("core".to_owned());
        let narrow_assembly = assemble_board(&dir, &narrow_scope, true)
            .expect("fixture corpus should assemble cleanly at the narrower tier scope");
        let narrow_dto = build_board(
            BoardScope::Tier,
            Some("core".to_owned()),
            &narrow_assembly.rollups,
            &narrow_assembly.files,
            narrow_assembly.stale,
            &narrow_assembly.last_touched,
            &narrow_assembly.block_graph,
        );

        // The narrower board's rollup must actually have excluded `beta` —
        // otherwise this test wouldn't be exercising the property it claims to.
        assert!(
            !narrow_assembly.rollups.iter().any(|r| r.repo == "beta"),
            "the narrower tier scope must exclude beta from its rollup"
        );

        let hq_count = dependent_count_in_board(&hq_dto, "alpha", "A1");
        let narrow_count = dependent_count_in_board(&narrow_dto, "alpha", "A1");

        assert_eq!(
            hq_count,
            Some(1),
            "alpha:A1 has one corpus-wide dependent (beta:B1) at hq scope"
        );
        assert_eq!(
            hq_count, narrow_count,
            "dependent_count for alpha:A1 must be IDENTICAL at hq scope and at the narrower \
             tier scope that excludes beta — this is the property bastion-web's in-scope \
             reverse-dep count (lib/board-view.ts:669-676) structurally cannot have, because \
             beta:B1 (the block that makes alpha:A1's count 1, not 0) is out of scope at the \
             narrower tier and would be invisible to any in-scope-only derivation"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── task 1 measurement: cost of adding `build_block_graph_export` to the
    // board path (`plan-board-graph-enrichment`, task 1) ─────────────────────
    //
    // Decision task, no production code change. Times `build_board` alone
    // against `build_board` plus a single unscoped `mev::build_block_graph_export`
    // call over the *same* `BoardAssembly` inputs, using the `Instant`/`elapsed`
    // precedent already established at `src/main.rs:315` and
    // `src/run/abort.rs:121`. Run with `--nocapture` to see the printed
    // absolute-ms figures; the measured numbers are transcribed by hand into
    // `tasks.md`'s Notes section (this test only proves the harness works and
    // that the two calls stay ordered sanely — CI machines vary too much in
    // absolute speed for a hardcoded ms budget to be meaningful).

    /// Build a synthetic on-disk brain root with a single repo containing `n`
    /// blocks, each depending on the previous one (a long `BlockedBy` chain),
    /// approximating a larger-than-fixture-default corpus for the timing
    /// harness without depending on the live HQ corpus being present.
    fn make_timing_fixture_brain_root(n: usize) -> std::path::PathBuf {
        let dir = crate::testsupport::unique_temp_dir("bastion-board-timing-fixture");
        let planning_dir = dir.join("bastion").join("planning");
        std::fs::create_dir_all(&planning_dir).unwrap();

        std::fs::write(
            dir.join("brain.toml"),
            r#"
[[repos]]
slug = "bastion"
tier = "core"
repo_path = "bastion"
status_file = "docs/status.md"
cache_doc = "docs/projects/bastion.md"
heading = "Bastion"
"#,
        )
        .unwrap();

        let mut blocks_json = String::new();
        for i in 0..n {
            if i > 0 {
                blocks_json.push(',');
            }
            let id = format!("BA.{i}");
            if i == 0 {
                blocks_json.push_str(&format!(
                    r#"{{"id": "{id}", "title": "{id} title", "status": "open"}}"#
                ));
            } else {
                let dep = format!("BA.{}", i - 1);
                blocks_json.push_str(&format!(
                    r#"{{"id": "{id}", "title": "{id} title", "status": "open", "depends_on": [{{"type": "block", "repo": "bastion", "id": "{dep}"}}]}}"#
                ));
            }
        }
        let state_json = format!(
            r#"{{
  "repo": "bastion",
  "kind": "project",
  "updated": "2026-08-01",
  "tracks": [
    {{
      "title": "Phase 1",
      "blocks": [{blocks_json}]
    }}
  ]
}}"#
        );
        std::fs::write(planning_dir.join("state.json"), state_json).unwrap();

        dir
    }

    /// Time `build_board` alone, then `build_board` plus one unscoped
    /// `mev::build_block_graph_export` call, over the same `BoardAssembly`.
    /// Also prints the full `assemble_board` (discover → load → build-graph →
    /// derive-rollup → `derive_last_touched`) time for context — that I/O
    /// walk, not `build_board` itself, is most of what a real `/api/board`
    /// request pays today. Returns `(build_board_ms, build_board_plus_graph_ms)`.
    fn measure_board_vs_board_plus_graph(root: &std::path::Path) -> (u128, u128) {
        let t_assemble = std::time::Instant::now();
        let assembly = assemble_board(root, &TierScope::All, false)
            .expect("fixture corpus should assemble cleanly");
        let assemble_board_ms = t_assemble.elapsed().as_millis();
        println!("[task1 measurement]   (context) assemble_board={assemble_board_ms}ms");

        let t0 = std::time::Instant::now();
        let _board = build_board(
            BoardScope::Hq,
            None,
            &assembly.rollups,
            &assembly.files,
            assembly.stale,
            &assembly.last_touched,
            &assembly.block_graph,
        );
        let build_board_ms = t0.elapsed().as_millis();

        // Unscoped — TierScope::All, no epic/repo restriction, include_closed
        // so nothing is dropped, max_nodes large enough to cover the whole
        // corpus — matching the corpus-invariance requirement task 3 will
        // enforce in production code.
        let scope = mev::BlockGraphScope {
            tier: TierScope::All,
            epic: None,
            repo: None,
            include_closed: true,
            include_boundary: false,
            max_nodes: usize::MAX,
        };

        let t1 = std::time::Instant::now();
        let _board_again = build_board(
            BoardScope::Hq,
            None,
            &assembly.rollups,
            &assembly.files,
            assembly.stale,
            &assembly.last_touched,
            &assembly.block_graph,
        );
        let _export = mev::build_block_graph_export(
            root,
            &assembly.config,
            &assembly.graph,
            &assembly.files,
            &scope,
        );
        let build_board_plus_graph_ms = t1.elapsed().as_millis();

        (build_board_ms, build_board_plus_graph_ms)
    }

    // ── Manual benchmarks (not part of the default suite) ──────────────────────
    //
    // The two `task1_measure_*` tests below are **benchmarks, not tests**:
    // their only assertion (`a >= a.min(b)`) is mathematically true for every
    // possible pair of measurements, so they can never fail and carry no
    // signal. Meanwhile each default-suite pass rebuilt a 500-block on-disk
    // fixture and re-walked the live HQ corpus. The numbers they produce were
    // transcribed once into `plan-board-graph-enrichment`'s Notes and
    // `docs/serve-api.md`; re-run them by hand when that cost-model question
    // is reopened:
    //
    //     cargo test -- --ignored task1_measure --nocapture
    //
    // (The live-HQ one still early-returns off-machine, so it is safe to run
    // anywhere.)

    #[test]
    #[ignore = "manual benchmark: vacuous assertion, expensive fixture — run with `cargo test -- --ignored task1_measure`"]
    fn task1_measure_build_block_graph_export_cost_on_synthetic_corpus() {
        // 500 blocks in one repo — larger than any other fixture in this test
        // module, used as a stand-in "largest available fixture corpus" per
        // the task 1 description.
        let dir = make_timing_fixture_brain_root(500);

        let (build_board_ms, build_board_plus_graph_ms) = measure_board_vs_board_plus_graph(&dir);

        println!(
            "[task1 measurement] synthetic 500-block corpus: build_board={build_board_ms}ms, \
             build_board+build_block_graph_export={build_board_plus_graph_ms}ms"
        );

        // No hardcoded ms budget (CI hardware varies too much for that to be
        // meaningful) — this test's job is to prove the harness runs cleanly
        // end-to-end and produces two comparable, non-negative measurements.
        // The transcribed absolute figures and the unconditional-vs-gated
        // decision live in tasks.md's Notes, not in a test assertion.
        assert!(build_board_plus_graph_ms >= build_board_ms.min(build_board_plus_graph_ms));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "manual benchmark: vacuous assertion, expensive fixture — run with `cargo test -- --ignored task1_measure`"]
    fn task1_measure_build_block_graph_export_cost_on_live_hq_corpus_if_reachable() {
        // "if reachable read-only" per the task 1 description — the live HQ
        // brain root sits some number of parent directories above this crate
        // (a plain checkout has it two levels above `bastion/`; an SDLC
        // worktree checkout adds an extra `trees/<branch>/` level), so this
        // walks up from `CARGO_MANIFEST_DIR` with the same
        // `find_brain_root` the production `get_board` handler uses, and
        // degrades to a no-op (rather than failing) when no `brain.toml` is
        // found, e.g. on a CI runner that only checks out this repo.
        let start = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf();
        let Ok(hq_root) = find_brain_root(&start) else {
            println!("[task1 measurement] live HQ brain root not reachable — skipping");
            return;
        };

        let (build_board_ms, build_board_plus_graph_ms) =
            measure_board_vs_board_plus_graph(&hq_root);

        println!(
            "[task1 measurement] live HQ corpus ({}): build_board={build_board_ms}ms, \
             build_board+build_block_graph_export={build_board_plus_graph_ms}ms",
            hq_root.display()
        );

        assert!(build_board_plus_graph_ms >= build_board_ms.min(build_board_plus_graph_ms));
    }
}
