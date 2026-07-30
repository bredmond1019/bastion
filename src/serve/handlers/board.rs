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
fn board_block_from(
    block: &okf_core::Block,
    repo: &str,
    last_touched: &HashMap<String, String>,
) -> BoardBlockDto {
    let key = format!("{repo}:{}", block.id);
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

/// Fill `epics`/`wave`/`priority`/`due`/`track` on `dto` from the authoring
/// `TrackBlock` + enclosing track title, when `entry` matches. An unmatched id
/// (no `tracks[]` entry with this block's id) leaves the DTO's existing
/// defaults (`epics: []`, four `None`s) untouched.
fn enrich_block(dto: &mut BoardBlockDto, entry: Option<(&TrackBlock, &str)>) {
    let Some((track_block, track_title)) = entry else {
        return;
    };

    dto.epics = track_block.epics.clone();
    dto.wave = track_block.wave;
    dto.priority = track_block.priority;
    dto.due = track_block.due.clone();
    dto.track = Some(track_title.to_owned());
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
/// [`BlockedBy::External`], or a [`BlockedBy::Block`] whose target's mapped
/// authored status is not `Some("closed")` — including a target absent from
/// `status_map` entirely (an unresolvable/missing dependency is unmet, not
/// vacuously satisfied).
fn unmet_deps(deps: &[BlockedBy], status_map: &HashMap<String, Option<String>>) -> Vec<BlockedBy> {
    deps.iter()
        .filter(|d| match d {
            BlockedBy::External { .. } => true,
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
) -> Vec<BoardBlockDto> {
    let mut entries: Vec<&(&TrackBlock, &str)> = index.values().collect();
    entries.sort_by_key(|(block, _)| block.id.as_str());

    entries
        .into_iter()
        .filter(|(block, _)| block.status.as_deref() == Some(CLOSED_STATUS))
        .map(|&(block, track_title)| {
            let key = format!("{repo}:{}", block.id);
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
pub fn build_board(
    scope: BoardScope,
    resolved_tier: Option<String>,
    rollups: &[RepoRollup],
    files: &[(StateSource, StateFile)],
    stale: bool,
    last_touched: &HashMap<String, String>,
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
                let mut dto = board_block_from(b, &rollup.repo, last_touched);
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
                let mut dto = board_block_from(b, &rollup.repo, last_touched);
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
        let blocked: Vec<BoardBlockDto> = rollup
            .blocked
            .iter()
            .map(|b| {
                let mut dto = board_block_from(b, &rollup.repo, last_touched);
                let entry = index.get(b.id.as_str()).copied();
                enrich_block(&mut dto, entry);
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
                let mut dto = board_block_from(b, &rollup.repo, last_touched);
                let entry = index.get(b.id.as_str()).copied();
                enrich_block(&mut dto, entry);
                if let Some((track_block, _)) = entry {
                    dto.blocked_by = unmet_deps(&track_block.depends_on, &status_map);
                }
                dto
            })
            .collect();
        let finished = finished_blocks_for_repo(&rollup.repo, &index, &status_map, last_touched);

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
pub(crate) fn assemble_board(root: &Path, tier_scope: &TierScope) -> Result<BoardAssembly, String> {
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

    let last_touched = derive_last_touched(root, &config, &loaded);

    Ok(BoardAssembly {
        config,
        rollups,
        files: loaded,
        graph,
        stale,
        last_touched,
    })
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
    let BoardQuery { scope, tier, epic } = query.into_inner();

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
        } = assemble_board(&root, &tier_scope).map_err(BoardError::BrainRoot)?;
        let board = build_board(scope, resolved_tier, &rollups, &files, stale, &last_touched);

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
            }],
            repos: Vec::new(),
            cross_repo: Vec::new(),
            tiers: Vec::new(),
            note: None,
            backlog: Vec::new(),
            carryover: Vec::new(),
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

        let finished = finished_blocks_for_repo("bastion", &index, &status_map, &HashMap::new());
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
            finished_blocks_for_repo("bastion", &index, &status_map, &HashMap::new()).is_empty()
        );
    }

    #[test]
    fn finished_blocks_empty_when_no_closed_blocks() {
        let file = sample_state_file(vec![sample_track_block("BA.1.A", Some("open"))]);
        let files = vec![(sample_source("bastion"), file)];
        let index = track_block_index("bastion", &files);
        let status_map = block_status_map(&files);
        assert!(
            finished_blocks_for_repo("bastion", &index, &status_map, &HashMap::new()).is_empty()
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
                },
                Track {
                    title: "Phase 2".to_owned(),
                    blocks: vec![sample_track_block("BA.2.A", Some("open"))],
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
                },
                Track {
                    title: "Phase 2".to_owned(),
                    blocks: vec![sample_track_block("BA.1.A", Some("closed"))],
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
    fn enrich_block_fills_all_five_fields_when_matched() {
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
        );

        enrich_block(&mut dto, Some((&track_block, "Phase 11")));

        assert_eq!(dto.epics, vec!["epic-alpha".to_owned()]);
        assert_eq!(dto.wave, Some(3));
        assert_eq!(dto.priority, Some(1));
        assert_eq!(dto.due, Some("2026-08-01".to_owned()));
        assert_eq!(dto.track, Some("Phase 11".to_owned()));
    }

    #[test]
    fn enrich_block_leaves_defaults_when_no_fields_authored() {
        let track_block = sample_track_block("BA.1.A", Some("open"));
        let mut dto = board_block_from(
            &sample_block("BA.1.A", Some("open"), Vec::new()),
            "bastion",
            &HashMap::new(),
        );

        enrich_block(&mut dto, Some((&track_block, "Phase 11")));

        assert!(dto.epics.is_empty());
        assert_eq!(dto.wave, None);
        assert_eq!(dto.priority, None);
        assert_eq!(dto.due, None);
        assert_eq!(dto.track, Some("Phase 11".to_owned()));
    }

    #[test]
    fn enrich_block_untouched_when_id_unmatched() {
        let mut dto = board_block_from(
            &sample_block("BA.1.A", Some("open"), Vec::new()),
            "bastion",
            &HashMap::new(),
        );

        enrich_block(&mut dto, None);

        assert!(dto.epics.is_empty());
        assert_eq!(dto.wave, None);
        assert_eq!(dto.priority, None);
        assert_eq!(dto.due, None);
        assert_eq!(dto.track, None);
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

    #[test]
    fn build_board_populates_last_touched_on_every_lane_when_key_matches() {
        let rollups = vec![all_lanes_rollup("bastion")];
        let files = all_lanes_files("bastion");
        let mut last_touched = HashMap::new();
        last_touched.insert(
            "bastion:BA.1.A".to_owned(),
            "2026-07-28T12:00:00Z".to_owned(),
        );

        let dto = build_board(BoardScope::Hq, None, &rollups, &files, false, &last_touched);

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

        let dto = build_board(BoardScope::Hq, None, &rollups, &files, false, &last_touched);

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

        let dto = build_board(BoardScope::Hq, None, &rollups, &files, false, &last_touched);

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
        );

        assert_eq!(dto.lanes.now[0].last_touched, None);
        assert_eq!(dto.lanes.next[0].last_touched, None);
        assert_eq!(dto.lanes.blocked[0].last_touched, None);
        assert_eq!(dto.lanes.deferred[0].last_touched, None);
        assert_eq!(dto.lanes.finished[0].last_touched, None);
    }

    #[test]
    fn build_board_empty_rollups_yields_empty_board() {
        let dto = build_board(BoardScope::Hq, None, &[], &[], false, &HashMap::new());
        assert!(dto.lanes.now.is_empty());
        assert!(dto.lanes.next.is_empty());
        assert!(dto.lanes.blocked.is_empty());
        assert!(dto.lanes.finished.is_empty());
        assert!(dto.repos.is_empty());
    }

    #[test]
    fn build_board_threads_stale_flag() {
        let dto = build_board(BoardScope::Hq, None, &[], &[], true, &HashMap::new());
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
            }],
            ..sample_state_file(Vec::new())
        };
        let mev_file = StateFile {
            repo: "mev".to_owned(),
            tracks: vec![Track {
                title: "Phase 1".to_owned(),
                blocks: vec![sample_track_block("BA.1.A", Some("in_progress"))],
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
            plan: None,
            repos: Vec::new(),
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
        let dir = std::env::temp_dir().join(format!(
            "bastion-board-assemble-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let result = assemble_board(&dir, &TierScope::All);
        assert!(
            result.is_err(),
            "expected an error with no brain.toml present"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn assemble_board_returns_graph_matching_loaded_fixture_corpus() {
        let dir = std::env::temp_dir().join(format!(
            "bastion-board-assemble-graph-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
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

        let assembly =
            assemble_board(&dir, &TierScope::All).expect("fixture corpus should assemble cleanly");

        let mut node_keys: Vec<String> =
            assembly.graph.nodes.iter().map(|n| n.key.clone()).collect();
        node_keys.sort();
        assert_eq!(node_keys, vec!["bastion:BA.1.A", "bastion:BA.1.B"]);
        assert_eq!(assembly.rollups.len(), 1);
        assert_eq!(assembly.files.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
