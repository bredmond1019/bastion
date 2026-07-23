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
use okf_core::StateFile;

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
    }
}

/// Convert one rollup lane entry (`okf_core::Block`) into a [`BoardBlockDto`],
/// tagging it with the owning `repo` slug (the rollup entries themselves don't carry
/// their own repo — they're already scoped to one repo by [`RepoRollup`]).
fn board_block_from(block: &okf_core::Block, repo: &str) -> BoardBlockDto {
    BoardBlockDto {
        id: block.id.clone(),
        title: block.title.clone(),
        repo: repo.to_owned(),
        status: block.status.clone(),
        blocked_by: block.blocked_by.clone(),
    }
}

/// Derive the `finished` lane (blocks with `status == "closed"`) for one repo slug
/// from the loaded `tracks[].blocks[]` in `files`.
///
/// Looks up the `StateFile` whose `StateSource::repo_slug` matches `repo`; a repo
/// with no loaded file (e.g. its `state.json` is missing/malformed) contributes an
/// empty finished lane rather than erroring, matching [`derive_rollup`]'s own
/// degrade-gracefully behavior for `now`/`next`/`blocked`.
fn finished_blocks_for_repo(repo: &str, files: &[(StateSource, StateFile)]) -> Vec<BoardBlockDto> {
    let Some((_, file)) = files.iter().find(|(src, _)| src.repo_slug == repo) else {
        return Vec::new();
    };

    file.tracks
        .iter()
        .flat_map(|track| &track.blocks)
        .filter(|block| block.status.as_deref() == Some(CLOSED_STATUS))
        .map(|block| BoardBlockDto {
            id: block.id.clone(),
            title: block.title.clone(),
            repo: repo.to_owned(),
            status: block.status.clone(),
            blocked_by: Vec::new(),
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

    for rollup in rollups {
        let now: Vec<BoardBlockDto> = rollup
            .now
            .iter()
            .map(|b| board_block_from(b, &rollup.repo))
            .collect();
        let next: Vec<BoardBlockDto> = rollup
            .next
            .iter()
            .map(|b| board_block_from(b, &rollup.repo))
            .collect();
        let blocked: Vec<BoardBlockDto> = rollup
            .blocked
            .iter()
            .map(|b| board_block_from(b, &rollup.repo))
            .collect();
        let finished = finished_blocks_for_repo(&rollup.repo, files);

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
        TrackBlock {
            id: id.to_owned(),
            title: format!("{id} title"),
            status: status.map(|s| s.to_owned()),
            depends_on: Vec::new(),
            wave: None,
            origin: None,
            priority: None,
            due: None,
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

        let finished = finished_blocks_for_repo("bastion", &files);
        assert_eq!(finished.len(), 1);
        assert_eq!(finished[0].id, "BA.1.A");
        assert_eq!(finished[0].repo, "bastion");
        assert_eq!(finished[0].status.as_deref(), Some("closed"));
    }

    #[test]
    fn finished_blocks_empty_when_repo_not_loaded() {
        let files: Vec<(StateSource, StateFile)> = Vec::new();
        assert!(finished_blocks_for_repo("bastion", &files).is_empty());
    }

    #[test]
    fn finished_blocks_empty_when_no_closed_blocks() {
        let file = sample_state_file(vec![sample_track_block("BA.1.A", Some("open"))]);
        let files = vec![(sample_source("bastion"), file)];
        assert!(finished_blocks_for_repo("bastion", &files).is_empty());
    }

    // ── build_board ────────────────────────────────────────────────────────

    fn sample_block(id: &str, status: Option<&str>, blocked_by: Vec<BlockedBy>) -> Block {
        Block {
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
