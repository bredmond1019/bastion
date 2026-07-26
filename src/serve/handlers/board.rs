//! Cross-brain now/next/blocked/finished board REST handler for `bastion serve` (BA.11.K).
//!
//! Read-only (D25) — this route never mutates any brain/tier/repo `state.json`. It
//! projects the same rollup the mev/okf-core brain walk already computes for
//! `bastion emit-state` / `bastion validate-brain --state` over HTTP.
//!
//! # Route
//! - `GET /api/board?scope=hq|tier|project|business[&tier=<name>]`
//!
//! # Pure core vs I/O shell (Rule 6)
//! [`resolve_scope`] and [`build_board`] (plus [`is_stale_for_scope`]) are pure —
//! unit-tested directly, no filesystem access. [`get_board`] is the thin async
//! handler: it resolves a starting path from the shared [`FileConfig`] registry,
//! walks up to the brain root (`mev::brain::config::find_brain_root`), then runs
//! the same discover → load → build-graph → derive-rollup pipeline
//! `mev::validate_brain_state` / `bastion emit-state` already use — see
//! `src/brainval/mod.rs` — under `web::block`, and hands the pure functions the
//! resulting rollups/files.
//!
//! # Error mapping
//! - Brain root unresolvable (no `brain.toml` walking up from the workspace root)
//!   → 500 + `C010` (mirrors the `web::block` failure code used by
//!   `handlers/status.rs`; there is no dedicated "brain not found" C-code and this
//!   is an operator-configuration problem, not a per-request one).
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

use mev::Diagnostic;
use mev::brain::config::{find_brain_root, load_brain_config};
use mev::brain::state::{
    RepoRollup, StateSource, TierScope, build_state_graph, derive_rollup, discover_state_files,
    load_state,
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
    /// `scope=hq|tier|project|business`; missing defaults to [`BoardScope::Hq`].
    #[serde(default)]
    pub scope: BoardScope,
    /// `tier=<name>`; only consulted for `scope=tier`/`scope=project` (default `"core"`).
    #[serde(default)]
    pub tier: Option<String>,
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
        // `Epic` is a cross-repo projection (`TierScope::All`, same as `Hq`) further
        // filtered by `&epic=<slug>` — that filtering is not implemented by this task
        // (BA.11.R task 1 is DTO-surface only); wired up in a follow-on task.
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
fn board_block_from(block: &okf_core::Block, repo: &str) -> BoardBlockDto {
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
) -> Vec<BoardBlockDto> {
    let mut entries: Vec<&(&TrackBlock, &str)> = index.values().collect();
    entries.sort_by_key(|(block, _)| block.id.as_str());

    entries
        .into_iter()
        .filter(|(block, _)| block.status.as_deref() == Some(CLOSED_STATUS))
        .map(|&(block, track_title)| {
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
) -> BoardDto {
    let mut repos: Vec<RepoBoardDto> = Vec::new();
    let mut agg_now = Vec::new();
    let mut agg_next = Vec::new();
    let mut agg_blocked = Vec::new();
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
                let mut dto = board_block_from(b, &rollup.repo);
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
                let mut dto = board_block_from(b, &rollup.repo);
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
                let mut dto = board_block_from(b, &rollup.repo);
                let entry = index.get(b.id.as_str()).copied();
                enrich_block(&mut dto, entry);
                dto
            })
            .collect();
        let finished = finished_blocks_for_repo(&rollup.repo, &index, &status_map);

        agg_now.extend(now.iter().cloned());
        agg_next.extend(next.iter().cloned());
        agg_blocked.extend(blocked.iter().cloned());
        agg_finished.extend(finished.iter().cloned());

        repos.push(RepoBoardDto {
            repo: rollup.repo.clone(),
            tier: rollup.tier.clone(),
            lanes: BoardLaneDto {
                now,
                next,
                blocked,
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
            finished: agg_finished,
        },
        repos,
        stale,
    }
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
/// mirroring `handlers/status.rs::blocking_error_response`.
fn blocking_error_response(err: actix_web::error::BlockingError) -> HttpResponse {
    HttpResponse::InternalServerError().json(ErrorPayload {
        code: "C010".to_owned(),
        message: format!("blocking thread error: {err}"),
    })
}

/// Build a 500 response for a brain-root resolution failure (no `brain.toml`
/// found walking up from the resolved workspace root, or the file failed to
/// parse). This is an operator-configuration problem, not a per-request one —
/// mirrored on the same `C010` code used for other I/O-shell failures since
/// there is no dedicated brain-root C-code.
fn brain_root_error_response(message: impl std::fmt::Display) -> HttpResponse {
    HttpResponse::InternalServerError().json(ErrorPayload {
        code: "C010".to_owned(),
        message: message.to_string(),
    })
}

/// Loaded `(StateSource, StateFile)` pairs, the in-scope `RepoRollup`s for a
/// resolved [`TierScope`], and the `stale` freshness flag — the three inputs
/// [`build_board`] needs, assembled by [`assemble_board`].
type BoardAssembly = (Vec<RepoRollup>, Vec<(StateSource, StateFile)>, bool);

/// Assemble the brain-walk inputs `build_board` needs: the loaded `(StateSource,
/// StateFile)` pairs, the in-scope `RepoRollup`s for `tier_scope`, and the
/// `stale` flag. Reuses the exact discover → load → build-graph → derive-rollup
/// pipeline `mev::validate_brain_state` runs (see `src/brainval/mod.rs`) instead
/// of re-plumbing it. Malformed/unreadable individual `state.json` files are
/// skipped (degrade gracefully) rather than failing the whole request — only an
/// unresolvable brain root is a hard error.
fn assemble_board(root: &Path, tier_scope: &TierScope) -> Result<BoardAssembly, String> {
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

    Ok((rollups, loaded, stale))
}

/// `GET /api/board?scope=hq|tier|project|business[&tier=<name>]` — cross-brain
/// now/next/blocked/finished board (BA.11.K).
///
/// Bearer auth is inherited from the `/api` scope's `BearerAuthMiddleware` — a
/// request without a valid token never reaches this handler (401 upstream).
pub async fn get_board(
    query: web::Query<BoardQuery>,
    registry: web::Data<FileConfig>,
) -> HttpResponse {
    let BoardQuery { scope, tier } = query.into_inner();
    let (tier_scope, resolved_tier) = resolve_scope(scope, tier.as_deref());

    let start: PathBuf =
        resolve_workspace_root(None, None, &registry).unwrap_or_else(|_| PathBuf::from("."));

    match web::block(move || -> Result<BoardDto, String> {
        let root = find_brain_root(&start)
            .map_err(|e| format!("could not resolve brain root from {}: {e}", start.display()))?;
        let (rollups, files, stale) = assemble_board(&root, &tier_scope)?;
        Ok(build_board(scope, resolved_tier, &rollups, &files, stale))
    })
    .await
    {
        Ok(Ok(dto)) => HttpResponse::Ok().json(dto),
        Ok(Err(msg)) => brain_root_error_response(msg),
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

        let finished = finished_blocks_for_repo("bastion", &index, &status_map);
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
        assert!(finished_blocks_for_repo("bastion", &index, &status_map).is_empty());
    }

    #[test]
    fn finished_blocks_empty_when_no_closed_blocks() {
        let file = sample_state_file(vec![sample_track_block("BA.1.A", Some("open"))]);
        let files = vec![(sample_source("bastion"), file)];
        let index = track_block_index("bastion", &files);
        let status_map = block_status_map(&files);
        assert!(finished_blocks_for_repo("bastion", &index, &status_map).is_empty());
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
        let mut dto =
            board_block_from(&sample_block("BA.1.A", Some("open"), Vec::new()), "bastion");

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
        let mut dto =
            board_block_from(&sample_block("BA.1.A", Some("open"), Vec::new()), "bastion");

        enrich_block(&mut dto, Some((&track_block, "Phase 11")));

        assert!(dto.epics.is_empty());
        assert_eq!(dto.wave, None);
        assert_eq!(dto.priority, None);
        assert_eq!(dto.due, None);
        assert_eq!(dto.track, Some("Phase 11".to_owned()));
    }

    #[test]
    fn enrich_block_untouched_when_id_unmatched() {
        let mut dto =
            board_block_from(&sample_block("BA.1.A", Some("open"), Vec::new()), "bastion");

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
        );

        assert_eq!(dto.scope, BoardScope::Tier);
        assert_eq!(dto.tier, Some("core".to_owned()));
        assert!(!dto.stale);
        assert_eq!(dto.lanes.now.len(), 1);
        assert_eq!(dto.lanes.now[0].id, "BA.1.A");
        assert_eq!(dto.lanes.now[0].repo, "bastion");
        assert_eq!(dto.lanes.next[0].id, "BA.1.B");
        assert_eq!(dto.lanes.blocked[0].id, "BA.1.C");
        assert_eq!(dto.repos.len(), 1);
        assert_eq!(dto.repos[0].repo, "bastion");
        assert_eq!(dto.repos[0].tier, Some("core".to_owned()));
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

        let dto = build_board(BoardScope::Hq, None, &rollups, &files, false);

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

        let dto = build_board(BoardScope::Hq, None, &rollups, &files, false);

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

        let dto = build_board(BoardScope::Hq, None, &rollups, &files, false);

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

        let dto = build_board(BoardScope::Hq, None, &rollups, &files, false);

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

        let dto = build_board(BoardScope::Hq, None, &rollups, &files, false);

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

    #[test]
    fn build_board_empty_rollups_yields_empty_board() {
        let dto = build_board(BoardScope::Hq, None, &[], &[], false);
        assert!(dto.lanes.now.is_empty());
        assert!(dto.lanes.next.is_empty());
        assert!(dto.lanes.blocked.is_empty());
        assert!(dto.lanes.finished.is_empty());
        assert!(dto.repos.is_empty());
    }

    #[test]
    fn build_board_threads_stale_flag() {
        let dto = build_board(BoardScope::Hq, None, &[], &[], true);
        assert!(dto.stale);
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
}
