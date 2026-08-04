//! `GET /api/blocks/graph` — mechanical projection of mev's enriched block-graph
//! export over `bastion serve` (BA.17.A, program block `BA.2.A`).
//!
//! Bastion performs **zero derivation** of its own here: [`block_graph_dto`] copies
//! every field straight across from `mev::brain::block_graph::BlockGraphExport`. The
//! handler reuses [`board::assemble_board`] for the brain-walk (config + loaded
//! files + [`mev::brain::state::StateGraph`] + `stale`) so the graph and the board
//! read one corpus in one request shape, then calls
//! [`mev::build_block_graph_export`] directly — never `mev::block_graph_brain`,
//! which would re-plumb its own discover → load → build pipeline.
//!
//! # Route
//! - `GET /api/blocks/graph?scope=hq|tier|project|business|epic[&tier=<name>]
//!   [&epic=<slug>][&repo=<slug>][&include_closed=bool][&include_boundary=bool]
//!   [&max_nodes=<n>]` — `max_nodes` defaults to 400, clamped to 2000.
//!
//! # Pure core vs I/O shell (Rule 6)
//! [`resolve_max_nodes`], [`block_graph_scope_from_query`], and [`block_graph_dto`]
//! are pure — unit-tested directly, no filesystem access. [`get_block_graph`] is the
//! thin async handler.
//!
//! # Error mapping
//! Identical to `handlers/board.rs::get_board` — reuses its helpers verbatim:
//! - `scope=epic` with a missing/blank `&epic=` param, or an unknown slug →
//!   404 + `C005` via [`board::epic_param_missing`] / [`board::epic_known`] /
//!   [`board::epic_error_response`].
//! - Brain root unresolvable → 500 + `C010` via [`board::brain_root_error_response`].
//! - `web::block` thread-pool failure → 500 + `C010` via
//!   [`board::blocking_error_response`].
//! - An individual malformed `state.json` is skipped, not fatal (same posture as
//!   [`board::assemble_board`]).

use std::path::PathBuf;

use actix_web::{HttpResponse, web};
use serde::Deserialize;

use crate::config::{FileConfig, resolve_workspace_root};
use crate::serve::dto::{
    BlockEdgeKindDto, BlockGraphDto, BlockGraphEdgeDto, BlockGraphNodeDto, BlockLaneDto, BoardScope,
};
use crate::serve::handlers::board::{self, BoardAssembly};
use crate::serve::handlers::epics::hq_epic_registry;

use mev::brain::config::find_brain_root;
use mev::{BlockGraphEdge, BlockGraphExport, BlockGraphNode, BlockGraphScope, BlockLane};
use okf_core::StateEdgeKind;

/// Default `max_nodes` when the query param is absent.
const DEFAULT_MAX_NODES: usize = 400;
/// Hard clamp on `max_nodes` regardless of the requested value.
const MAX_NODES_CLAMP: usize = 2000;

// ── Query params ────────────────────────────────────────────────────────────────

/// `GET /api/blocks/graph` query params.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BlockGraphQuery {
    /// `scope=hq|tier|project|business|epic`; missing defaults to [`BoardScope::Hq`].
    #[serde(default)]
    pub scope: BoardScope,
    /// `tier=<name>`; only consulted for `scope=tier`/`scope=project` (default `"core"`).
    #[serde(default)]
    pub tier: Option<String>,
    /// `epic=<slug>`; required (and only consulted) for `scope=epic`.
    #[serde(default)]
    pub epic: Option<String>,
    /// `repo=<slug>`; narrows the export down to a single repo.
    #[serde(default)]
    pub repo: Option<String>,
    /// `include_closed=<bool>`; whether `Closed`-lane nodes are retained.
    #[serde(default)]
    pub include_closed: bool,
    /// `include_boundary=<bool>`; whether direct neighbours of the in-scope set
    /// are re-added as boundary nodes.
    #[serde(default)]
    pub include_boundary: bool,
    /// `max_nodes=<n>`; absent defaults to 400, clamped to 2000 by
    /// [`resolve_max_nodes`].
    #[serde(default)]
    pub max_nodes: Option<usize>,
}

// ── Pure core ────────────────────────────────────────────────────────────────────

/// Resolve the `max_nodes` query param into the actual cap passed to
/// [`mev::BlockGraphScope`]: absent defaults to [`DEFAULT_MAX_NODES`] (400); any
/// value (including `0`) is clamped to at most [`MAX_NODES_CLAMP`] (2000).
pub fn resolve_max_nodes(param: Option<usize>) -> usize {
    param.unwrap_or(DEFAULT_MAX_NODES).min(MAX_NODES_CLAMP)
}

/// Build the [`mev::BlockGraphScope`] the export pipeline needs from the query
/// params, reusing [`board::resolve_scope`] for the `scope`/`tier` → `TierScope`
/// mapping so the block-graph and board endpoints resolve tier scoping
/// identically. `scope=epic`'s `TierScope::All` is discarded in favour of the
/// `epic` field, which overrides tier in `build_block_graph_export`'s own
/// pipeline — same posture as `board::resolve_scope`'s doc comment.
pub fn block_graph_scope_from_query(query: &BlockGraphQuery) -> BlockGraphScope {
    let (tier_scope, _resolved_tier) = board::resolve_scope(query.scope, query.tier.as_deref());
    let epic = if query.scope == BoardScope::Epic {
        query.epic.clone()
    } else {
        None
    };

    BlockGraphScope {
        tier: tier_scope,
        epic,
        repo: query.repo.clone(),
        include_closed: query.include_closed,
        include_boundary: query.include_boundary,
        max_nodes: resolve_max_nodes(query.max_nodes),
    }
}

fn lane_dto(lane: BlockLane) -> BlockLaneDto {
    match lane {
        BlockLane::Now => BlockLaneDto::Now,
        BlockLane::Next => BlockLaneDto::Next,
        BlockLane::Blocked => BlockLaneDto::Blocked,
        BlockLane::Deferred => BlockLaneDto::Deferred,
        BlockLane::Closed => BlockLaneDto::Closed,
        BlockLane::Other => BlockLaneDto::Other,
    }
}

fn edge_kind_dto(kind: StateEdgeKind) -> BlockEdgeKindDto {
    match kind {
        StateEdgeKind::BlockedBy => BlockEdgeKindDto::BlockedBy,
        StateEdgeKind::CrossRepo => BlockEdgeKindDto::CrossRepo,
    }
}

fn node_dto(node: &BlockGraphNode) -> BlockGraphNodeDto {
    BlockGraphNodeDto {
        key: node.key.clone(),
        repo: node.repo.clone(),
        id: node.id.clone(),
        title: node.title.clone(),
        status: node.status.clone(),
        lane: lane_dto(node.lane),
        track: node.track.clone(),
        wave: node.wave,
        priority: node.priority,
        effective_priority: node.effective_priority,
        due: node.due.clone(),
        epics: node.epics.clone(),
        layer: node.layer,
        topo_index: node.topo_index,
        ready: node.ready,
        in_cycle: node.in_cycle,
        in_scope: node.in_scope,
        external_deps: node.external_deps.clone(),
        unmet_count: node.unmet_count,
        dependent_count: node.dependent_count,
    }
}

fn edge_dto(edge: &BlockGraphEdge) -> BlockGraphEdgeDto {
    BlockGraphEdgeDto {
        from: edge.from.clone(),
        to_ref: edge.to_ref.clone(),
        kind: edge_kind_dto(edge.kind.clone()),
        target_node_id: edge.target_node_id.clone(),
        blocking: edge.blocking,
    }
}

/// Project a [`BlockGraphExport`] into a [`BlockGraphDto`] — a **mechanical
/// projection only**, copying every field straight across; no derivation of any
/// kind happens here. `scope` is the response-level [`BoardScope`] echo (the
/// request's resolved scope, distinct from `export.scope`'s tier/epic/repo/
/// closed/boundary echo, which is copied field-for-field too); `stale` is
/// threaded through unchanged from [`BoardAssembly`].
pub fn block_graph_dto(export: &BlockGraphExport, scope: BoardScope, stale: bool) -> BlockGraphDto {
    BlockGraphDto {
        version: export.version.clone(),
        root: export.root.clone(),
        scope,
        tier: export.scope.tier.clone(),
        epic: export.scope.epic.clone(),
        repo: export.scope.repo.clone(),
        include_closed: export.scope.include_closed,
        include_boundary: export.scope.include_boundary,
        nodes: export.nodes.iter().map(node_dto).collect(),
        edges: export.edges.iter().map(edge_dto).collect(),
        cycles: export.cycles.clone(),
        total_nodes: export.total_nodes,
        truncated: export.truncated,
        stale,
    }
}

// ── I/O shell ──────────────────────────────────────────────────────────────────

/// The two error shapes [`get_block_graph`]'s `web::block` closure can fail
/// with — mirrors `board::BoardError` exactly (same two variants, same meaning).
enum BlockGraphError {
    BrainRoot(String),
    UnknownEpic(String),
}

/// `GET /api/blocks/graph?scope=...&tier=...&epic=...&repo=...&include_closed=...
/// &include_boundary=...&max_nodes=...` — read-only (D25) block-graph export.
///
/// Bearer auth is inherited from the `/api` scope's `BearerAuthMiddleware` — a
/// request without a valid token never reaches this handler (401 upstream).
pub async fn get_block_graph(
    query: web::Query<BlockGraphQuery>,
    registry: web::Data<FileConfig>,
) -> HttpResponse {
    let query = query.into_inner();

    if query.scope == BoardScope::Epic && board::epic_param_missing(query.epic.as_deref()) {
        return board::epic_error_response(
            "scope=epic requires a non-empty &epic=<slug> query param",
        );
    }

    let block_graph_scope = block_graph_scope_from_query(&query);
    let response_scope = query.scope;
    let epic = query.epic.clone();

    let start: PathBuf =
        resolve_workspace_root(None, None, &registry).unwrap_or_else(|_| PathBuf::from("."));

    match web::block(move || -> Result<BlockGraphDto, BlockGraphError> {
        let root = find_brain_root(&start).map_err(|e| {
            BlockGraphError::BrainRoot(format!(
                "could not resolve brain root from {}: {e}",
                start.display()
            ))
        })?;
        let BoardAssembly {
            config,
            rollups: _rollups,
            files,
            graph,
            stale,
            last_touched: _last_touched,
            block_graph: _block_graph,
        } = board::assemble_board(&root, &block_graph_scope.tier, false)
            .map_err(BlockGraphError::BrainRoot)?;

        if response_scope == BoardScope::Epic {
            // Checked non-empty above.
            let slug = epic.expect("scope=epic requires &epic=, checked before web::block");
            if !board::epic_known(&slug, hq_epic_registry(&config, &files)) {
                return Err(BlockGraphError::UnknownEpic(format!(
                    "unknown epic: {slug}"
                )));
            }
        }

        let export =
            mev::build_block_graph_export(&root, &config, &graph, &files, &block_graph_scope);
        Ok(block_graph_dto(&export, response_scope, stale))
    })
    .await
    {
        Ok(Ok(dto)) => HttpResponse::Ok().json(dto),
        Ok(Err(BlockGraphError::BrainRoot(msg))) => board::brain_root_error_response(msg),
        Ok(Err(BlockGraphError::UnknownEpic(msg))) => board::epic_error_response(msg),
        Err(err) => board::blocking_error_response(err),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use mev::brain::state::TierScope;

    // ── resolve_max_nodes ────────────────────────────────────────────────────

    #[test]
    fn resolve_max_nodes_defaults_to_400_when_absent() {
        assert_eq!(resolve_max_nodes(None), 400);
    }

    #[test]
    fn resolve_max_nodes_zero_stays_zero() {
        assert_eq!(resolve_max_nodes(Some(0)), 0);
    }

    #[test]
    fn resolve_max_nodes_below_clamp_passes_through() {
        assert_eq!(resolve_max_nodes(Some(150)), 150);
    }

    #[test]
    fn resolve_max_nodes_at_clamp_passes_through() {
        assert_eq!(resolve_max_nodes(Some(2000)), 2000);
    }

    #[test]
    fn resolve_max_nodes_above_clamp_is_capped() {
        assert_eq!(resolve_max_nodes(Some(5000)), 2000);
    }

    #[test]
    fn resolve_max_nodes_default_below_clamp_when_absent_and_default_smaller() {
        // Sanity: the absent-param default must itself be within the clamp.
        assert!(resolve_max_nodes(None) <= MAX_NODES_CLAMP);
    }

    // ── block_graph_scope_from_query ─────────────────────────────────────────

    fn base_query() -> BlockGraphQuery {
        BlockGraphQuery {
            scope: BoardScope::Hq,
            tier: None,
            epic: None,
            repo: None,
            include_closed: false,
            include_boundary: false,
            max_nodes: None,
        }
    }

    #[test]
    fn scope_from_query_hq_is_all_tier_no_epic() {
        let q = base_query();
        let scope = block_graph_scope_from_query(&q);
        assert_eq!(scope.tier, TierScope::All);
        assert_eq!(scope.epic, None);
        assert_eq!(scope.max_nodes, 400);
    }

    #[test]
    fn scope_from_query_tier_defaults_to_core() {
        let mut q = base_query();
        q.scope = BoardScope::Tier;
        let scope = block_graph_scope_from_query(&q);
        assert_eq!(scope.tier, TierScope::Tier("core".to_owned()));
        assert_eq!(scope.epic, None);
    }

    #[test]
    fn scope_from_query_tier_uses_given_tier() {
        let mut q = base_query();
        q.scope = BoardScope::Tier;
        q.tier = Some("side".to_owned());
        let scope = block_graph_scope_from_query(&q);
        assert_eq!(scope.tier, TierScope::Tier("side".to_owned()));
    }

    #[test]
    fn scope_from_query_project_mirrors_tier() {
        let mut q = base_query();
        q.scope = BoardScope::Project;
        q.tier = Some("client".to_owned());
        let scope = block_graph_scope_from_query(&q);
        assert_eq!(scope.tier, TierScope::Tier("client".to_owned()));
    }

    #[test]
    fn scope_from_query_business_is_shortcut() {
        let mut q = base_query();
        q.scope = BoardScope::Business;
        q.tier = Some("core".to_owned());
        let scope = block_graph_scope_from_query(&q);
        assert_eq!(scope.tier, TierScope::Tier("business".to_owned()));
    }

    #[test]
    fn scope_from_query_epic_carries_slug_and_all_tier() {
        let mut q = base_query();
        q.scope = BoardScope::Epic;
        q.epic = Some("bastion-surfaces".to_owned());
        let scope = block_graph_scope_from_query(&q);
        assert_eq!(scope.tier, TierScope::All);
        assert_eq!(scope.epic, Some("bastion-surfaces".to_owned()));
    }

    #[test]
    fn scope_from_query_non_epic_scope_ignores_stray_epic_param() {
        let mut q = base_query();
        q.scope = BoardScope::Hq;
        q.epic = Some("stray".to_owned());
        let scope = block_graph_scope_from_query(&q);
        assert_eq!(scope.epic, None);
    }

    #[test]
    fn scope_from_query_threads_repo_and_booleans() {
        let mut q = base_query();
        q.repo = Some("bastion".to_owned());
        q.include_closed = true;
        q.include_boundary = true;
        q.max_nodes = Some(50);
        let scope = block_graph_scope_from_query(&q);
        assert_eq!(scope.repo, Some("bastion".to_owned()));
        assert!(scope.include_closed);
        assert!(scope.include_boundary);
        assert_eq!(scope.max_nodes, 50);
    }

    // ── block_graph_dto ──────────────────────────────────────────────────────

    fn sample_export() -> BlockGraphExport {
        BlockGraphExport {
            version: "1".to_owned(),
            root: "/hq".to_owned(),
            scope: mev::BlockGraphScopeEcho {
                tier: Some("core".to_owned()),
                epic: None,
                repo: Some("bastion".to_owned()),
                include_closed: true,
                include_boundary: false,
            },
            nodes: vec![BlockGraphNode {
                key: "bastion:BA.1".to_owned(),
                repo: "bastion".to_owned(),
                id: "BA.1".to_owned(),
                title: "Some block".to_owned(),
                status: Some("open".to_owned()),
                lane: BlockLane::Next,
                track: Some("Phase 1".to_owned()),
                wave: Some(2),
                priority: Some(1),
                effective_priority: Some(1),
                due: None,
                epics: vec!["bastion-surfaces".to_owned()],
                layer: 0,
                topo_index: 0,
                ready: true,
                in_cycle: false,
                in_scope: true,
                external_deps: Vec::new(),
                unmet_count: 0,
                dependent_count: 1,
                last_touched: None,
            }],
            edges: vec![BlockGraphEdge {
                from: "bastion:BA.2".to_owned(),
                to_ref: "bastion:BA.1".to_owned(),
                kind: StateEdgeKind::BlockedBy,
                target_node_id: Some("bastion:BA.1".to_owned()),
                blocking: true,
            }],
            cycles: vec![vec!["bastion:BA.1".to_owned(), "bastion:BA.2".to_owned()]],
            total_nodes: 1,
            truncated: false,
        }
    }

    #[test]
    fn block_graph_dto_copies_every_field_verbatim() {
        let export = sample_export();
        let dto = block_graph_dto(&export, BoardScope::Tier, true);

        assert_eq!(dto.version, export.version);
        assert_eq!(dto.root, export.root);
        assert_eq!(dto.scope, BoardScope::Tier);
        assert_eq!(dto.tier, export.scope.tier);
        assert_eq!(dto.epic, export.scope.epic);
        assert_eq!(dto.repo, export.scope.repo);
        assert_eq!(dto.include_closed, export.scope.include_closed);
        assert_eq!(dto.include_boundary, export.scope.include_boundary);
        assert_eq!(dto.cycles, export.cycles);
        assert_eq!(dto.total_nodes, export.total_nodes);
        assert_eq!(dto.truncated, export.truncated);
        assert!(dto.stale);

        assert_eq!(dto.nodes.len(), 1);
        let node = &dto.nodes[0];
        let src_node = &export.nodes[0];
        assert_eq!(node.key, src_node.key);
        assert_eq!(node.repo, src_node.repo);
        assert_eq!(node.id, src_node.id);
        assert_eq!(node.title, src_node.title);
        assert_eq!(node.status, src_node.status);
        assert_eq!(node.lane, BlockLaneDto::Next);
        assert_eq!(node.track, src_node.track);
        assert_eq!(node.wave, src_node.wave);
        assert_eq!(node.priority, src_node.priority);
        assert_eq!(node.effective_priority, src_node.effective_priority);
        assert_eq!(node.due, src_node.due);
        assert_eq!(node.epics, src_node.epics);
        assert_eq!(node.layer, src_node.layer);
        assert_eq!(node.topo_index, src_node.topo_index);
        assert_eq!(node.ready, src_node.ready);
        assert_eq!(node.in_cycle, src_node.in_cycle);
        assert_eq!(node.in_scope, src_node.in_scope);
        assert_eq!(node.external_deps, src_node.external_deps);
        assert_eq!(node.unmet_count, src_node.unmet_count);
        assert_eq!(node.dependent_count, src_node.dependent_count);

        assert_eq!(dto.edges.len(), 1);
        let edge = &dto.edges[0];
        let src_edge = &export.edges[0];
        assert_eq!(edge.from, src_edge.from);
        assert_eq!(edge.to_ref, src_edge.to_ref);
        assert_eq!(edge.kind, BlockEdgeKindDto::BlockedBy);
        assert_eq!(edge.target_node_id, src_edge.target_node_id);
        assert_eq!(edge.blocking, src_edge.blocking);
    }

    #[test]
    fn block_graph_dto_lane_mapping_covers_all_six_variants() {
        let mappings = [
            (BlockLane::Now, BlockLaneDto::Now),
            (BlockLane::Next, BlockLaneDto::Next),
            (BlockLane::Blocked, BlockLaneDto::Blocked),
            (BlockLane::Deferred, BlockLaneDto::Deferred),
            (BlockLane::Closed, BlockLaneDto::Closed),
            (BlockLane::Other, BlockLaneDto::Other),
        ];
        for (lane, expected) in mappings {
            assert_eq!(lane_dto(lane), expected);
        }
    }

    #[test]
    fn block_graph_dto_edge_kind_mapping_covers_both_variants() {
        assert_eq!(
            edge_kind_dto(StateEdgeKind::BlockedBy),
            BlockEdgeKindDto::BlockedBy
        );
        assert_eq!(
            edge_kind_dto(StateEdgeKind::CrossRepo),
            BlockEdgeKindDto::CrossRepo
        );
    }

    #[test]
    fn block_graph_dto_empty_nodes_and_edges_when_export_is_empty() {
        let mut export = sample_export();
        export.nodes.clear();
        export.edges.clear();
        let dto = block_graph_dto(&export, BoardScope::Hq, false);
        assert!(dto.nodes.is_empty());
        assert!(dto.edges.is_empty());
    }

    // ── error-branch pure classification (no actix) ──────────────────────────

    #[test]
    fn epic_param_missing_reused_from_board_for_none() {
        assert!(board::epic_param_missing(None));
    }

    #[test]
    fn epic_param_missing_reused_from_board_for_blank() {
        assert!(board::epic_param_missing(Some("   ")));
    }

    #[test]
    fn epic_param_missing_reused_from_board_false_for_present() {
        assert!(!board::epic_param_missing(Some("bastion-surfaces")));
    }

    #[test]
    fn epic_known_reused_from_board() {
        let registry = vec![okf_core::Epic {
            extra: Default::default(),
            slug: "bastion-surfaces".to_owned(),
            title: "Bastion Surfaces".to_owned(),
            description: None,
            status: None,
            weight: None,
            plan: None,
            repos: Vec::new(),
        }];
        assert!(board::epic_known("bastion-surfaces", &registry));
        assert!(!board::epic_known("unknown-slug", &registry));
    }

    #[test]
    fn brain_root_error_response_is_500_c010() {
        let resp = board::brain_root_error_response("boom");
        assert_eq!(
            resp.status(),
            actix_web::http::StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn epic_error_response_is_404_c005() {
        let resp = board::epic_error_response("unknown epic");
        assert_eq!(resp.status(), actix_web::http::StatusCode::NOT_FOUND);
    }

    // ── one-derivation cross-check (BA.17.A task 5) ──────────────────────────
    //
    // Mechanically enforces "one derivation": builds one fixture corpus on
    // disk, runs it through `assemble_board` once, then feeds the *same*
    // `(config, files, graph)` triple into both `build_block_graph_export` →
    // `block_graph_dto` and `build_board`. Any divergence in node count, edge
    // count, or per-node lane between the two independent read paths fails
    // the test.

    /// Build a temp brain root with one repo and five blocks covering every
    /// `BlockLane` the board also classifies (now/next/blocked/deferred/
    /// closed): `BA.1` (`in_progress` → now), `BA.2` (`closed` → finished),
    /// `BA.3` (`open`, depends on the already-closed `BA.2` → next, met dep),
    /// `BA.4` (`open`, depends on the still-open `BA.3` → blocked, unmet dep),
    /// `BA.5` (`deferred` → deferred). Deliberately avoids any block that
    /// would land in `BlockLane::Other` — `board::build_board` has no lane for
    /// it, so it would break the count comparison below for a reason that
    /// isn't a real divergence between the two derivations.
    ///
    /// `BA.1` additionally gets an on-disk spec folder
    /// (`bastion/planning/BA.1-in-progress/sdlc/sdlc-flow-state.json`) with an
    /// `updated_at`, so `mev::brain::last_touched::derive_last_touched` has
    /// exactly one resolvable block (BA.11.S task 3) — `BA.5` is deliberately
    /// left with no spec folder at all, so it must come back `None` on both
    /// read paths.
    fn make_cross_check_brain_root() -> std::path::PathBuf {
        let dir = crate::testsupport::unique_temp_dir("bastion-block-graph-cross-check");
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
  "updated": "2026-07-28",
  "tracks": [
    {
      "title": "Phase 1",
      "blocks": [
        {"id": "BA.1", "title": "In progress", "status": "in_progress"},
        {"id": "BA.2", "title": "Closed", "status": "closed"},
        {"id": "BA.3", "title": "Ready", "status": "open", "depends_on": [{"type": "block", "repo": "bastion", "id": "BA.2"}]},
        {"id": "BA.4", "title": "Waiting", "status": "open", "depends_on": [{"type": "block", "repo": "bastion", "id": "BA.3"}]},
        {"id": "BA.5", "title": "Parked", "status": "deferred"}
      ]
    }
  ]
}"#,
        )
        .unwrap();

        // A resolvable SDLC spec folder for BA.1 only — BA.5 (and every other
        // block) deliberately has no spec folder, so `derive_last_touched`
        // must report `None` for it on both read paths.
        let ba1_sdlc_dir = planning_dir.join("BA.1-in-progress").join("sdlc");
        std::fs::create_dir_all(&ba1_sdlc_dir).unwrap();
        std::fs::write(
            ba1_sdlc_dir.join("sdlc-flow-state.json"),
            r#"{"updated_at": "2026-07-28T12:00:00Z"}"#,
        )
        .unwrap();

        dir
    }

    #[test]
    fn export_to_dto_matches_build_board_node_count_edge_count_and_lanes() {
        let dir = make_cross_check_brain_root();

        // `include_graph: true` — this test extends to also cross-check
        // `dependent_count`/`ready` (`plan-board-graph-enrichment` task 5), which
        // requires `assembly.block_graph` to actually be populated; with
        // `include_graph: false` every board entry would report `None` and the
        // comparison below would be vacuous.
        let assembly = board::assemble_board(&dir, &TierScope::All, true)
            .expect("fixture corpus should assemble cleanly");

        // Same corpus (config, files, graph) feeds both independent read paths.
        let block_graph_scope = BlockGraphScope {
            tier: TierScope::All,
            epic: None,
            repo: None,
            include_closed: true,
            include_boundary: false,
            max_nodes: 2000,
        };
        let export = mev::build_block_graph_export(
            &dir,
            &assembly.config,
            &assembly.graph,
            &assembly.files,
            &block_graph_scope,
        );
        let dto = block_graph_dto(&export, BoardScope::Hq, assembly.stale);

        let board_dto = board::build_board(
            BoardScope::Hq,
            None,
            &assembly.rollups,
            &assembly.files,
            assembly.stale,
            &assembly.last_touched,
            &assembly.block_graph,
        );

        // ── node count ───────────────────────────────────────────────────
        let board_node_count = board_dto.lanes.now.len()
            + board_dto.lanes.next.len()
            + board_dto.lanes.blocked.len()
            + board_dto.lanes.deferred.len()
            + board_dto.lanes.finished.len();
        assert_eq!(dto.nodes.len(), 5, "fixture authors exactly five blocks");
        assert_eq!(
            dto.nodes.len(),
            board_node_count,
            "block-graph node count must equal build_board's total across all five lanes"
        );

        // ── edge count ───────────────────────────────────────────────────
        // BA.3 -> BA.2 and BA.4 -> BA.3 are the only two authored BlockedBy
        // deps; both survive the scope pipeline (both endpoints in scope).
        assert_eq!(dto.edges.len(), 2);

        // ── per-node lane ────────────────────────────────────────────────
        let mut board_lane_of: std::collections::HashMap<String, BlockLaneDto> =
            std::collections::HashMap::new();
        for b in &board_dto.lanes.now {
            board_lane_of.insert(format!("{}:{}", b.repo, b.id), BlockLaneDto::Now);
        }
        for b in &board_dto.lanes.next {
            board_lane_of.insert(format!("{}:{}", b.repo, b.id), BlockLaneDto::Next);
        }
        for b in &board_dto.lanes.blocked {
            board_lane_of.insert(format!("{}:{}", b.repo, b.id), BlockLaneDto::Blocked);
        }
        for b in &board_dto.lanes.deferred {
            board_lane_of.insert(format!("{}:{}", b.repo, b.id), BlockLaneDto::Deferred);
        }
        for b in &board_dto.lanes.finished {
            board_lane_of.insert(format!("{}:{}", b.repo, b.id), BlockLaneDto::Closed);
        }

        assert_eq!(
            board_lane_of.len(),
            5,
            "every fixture block must land in exactly one build_board lane"
        );

        for node in &dto.nodes {
            let expected = board_lane_of
                .get(&node.key)
                .unwrap_or_else(|| panic!("{} missing from every build_board lane", node.key));
            assert_eq!(
                &node.lane, expected,
                "lane mismatch for {}: block_graph says {:?}, build_board says {:?}",
                node.key, node.lane, expected
            );
        }

        // ── last_touched one-derivation cross-check (BA.11.S task 3) ─────
        //
        // Both `build_board` (via `board::board_block_from` /
        // `finished_blocks_for_repo`) and `mev::build_block_graph_export`
        // (via `BlockGraphNode.last_touched`) source their per-block value
        // from the very same `derive_last_touched` map computed once inside
        // `assemble_board`. Compare the two independent read paths directly
        // — a divergence in either direction (mismatched value, or one side
        // populating what the other omits) must fail this test.
        let mut board_last_touched: std::collections::HashMap<String, Option<String>> =
            std::collections::HashMap::new();
        for b in &board_dto.lanes.now {
            board_last_touched.insert(format!("{}:{}", b.repo, b.id), b.last_touched.clone());
        }
        for b in &board_dto.lanes.next {
            board_last_touched.insert(format!("{}:{}", b.repo, b.id), b.last_touched.clone());
        }
        for b in &board_dto.lanes.blocked {
            board_last_touched.insert(format!("{}:{}", b.repo, b.id), b.last_touched.clone());
        }
        for b in &board_dto.lanes.deferred {
            board_last_touched.insert(format!("{}:{}", b.repo, b.id), b.last_touched.clone());
        }
        for b in &board_dto.lanes.finished {
            board_last_touched.insert(format!("{}:{}", b.repo, b.id), b.last_touched.clone());
        }
        assert_eq!(
            board_last_touched.len(),
            5,
            "every fixture block must appear exactly once across build_board's lanes"
        );

        let mut graph_last_touched: std::collections::HashMap<String, Option<String>> =
            std::collections::HashMap::new();
        for node in &export.nodes {
            graph_last_touched.insert(node.key.clone(), node.last_touched.clone());
        }
        assert_eq!(
            graph_last_touched.len(),
            5,
            "every fixture block must appear exactly once across build_block_graph_export's nodes"
        );

        assert_eq!(
            board_last_touched, graph_last_touched,
            "last_touched must agree between build_board and build_block_graph_export for every block"
        );

        // Concrete, non-vacuous assertions: BA.1 has a resolvable spec folder,
        // BA.5 has none.
        assert_eq!(
            board_last_touched.get("bastion:BA.1").cloned().flatten(),
            Some("2026-07-28T12:00:00Z".to_owned()),
            "BA.1 must report its spec folder's updated_at verbatim on build_board's side"
        );
        assert_eq!(
            graph_last_touched.get("bastion:BA.1").cloned().flatten(),
            Some("2026-07-28T12:00:00Z".to_owned()),
            "BA.1 must report its spec folder's updated_at verbatim on build_block_graph_export's side"
        );
        assert_eq!(
            board_last_touched.get("bastion:BA.5").cloned().flatten(),
            None,
            "BA.5 has no spec folder and must be None on build_board's side"
        );
        assert_eq!(
            graph_last_touched.get("bastion:BA.5").cloned().flatten(),
            None,
            "BA.5 has no spec folder and must be None on build_block_graph_export's side"
        );

        // ── dependent_count / ready one-derivation cross-check
        // (`plan-board-graph-enrichment` task 5) ─────────────────────────
        //
        // Same shape as the `last_touched` cross-check above: both
        // `build_board` (via `board::board_block_from` /
        // `finished_blocks_for_repo`, reading `assembly.block_graph`) and
        // `mev::build_block_graph_export` (via `BlockGraphNode.dependent_count`
        // / `.ready`) ultimately source these values from the very same
        // unscoped `build_block_graph_export` call `assemble_board` makes once
        // internally (`board::build_block_graph_enrichment`). bastion derives
        // nothing itself — every value must agree, verbatim, on both read
        // paths, value-and-absence, for every fixture block.
        let mut board_dependent_count: std::collections::HashMap<String, Option<u32>> =
            std::collections::HashMap::new();
        let mut board_ready: std::collections::HashMap<String, Option<bool>> =
            std::collections::HashMap::new();
        for b in board_dto
            .lanes
            .now
            .iter()
            .chain(board_dto.lanes.next.iter())
            .chain(board_dto.lanes.blocked.iter())
            .chain(board_dto.lanes.deferred.iter())
            .chain(board_dto.lanes.finished.iter())
        {
            let key = format!("{}:{}", b.repo, b.id);
            board_dependent_count.insert(key.clone(), b.dependent_count);
            board_ready.insert(key, b.ready);
        }
        assert_eq!(
            board_dependent_count.len(),
            5,
            "every fixture block must appear exactly once across build_board's lanes"
        );

        let mut graph_dependent_count: std::collections::HashMap<String, Option<u32>> =
            std::collections::HashMap::new();
        let mut graph_ready: std::collections::HashMap<String, Option<bool>> =
            std::collections::HashMap::new();
        for node in &export.nodes {
            graph_dependent_count.insert(node.key.clone(), Some(node.dependent_count));
            graph_ready.insert(node.key.clone(), Some(node.ready));
        }
        assert_eq!(
            graph_dependent_count.len(),
            5,
            "every fixture block must appear exactly once across build_block_graph_export's nodes"
        );

        assert_eq!(
            board_dependent_count, graph_dependent_count,
            "dependent_count must agree between build_board and build_block_graph_export for every block"
        );
        assert_eq!(
            board_ready, graph_ready,
            "ready must agree between build_board and build_block_graph_export for every block"
        );

        // Concrete, non-vacuous assertions: BA.3 depends on the already-closed
        // BA.2 (met dep, so BA.3 has one dependent-of-BA.2 relationship) and
        // BA.4 depends on the still-open BA.3 (unmet dep) — both directions
        // give BA.2/BA.3 a non-zero dependent_count to assert against, ruling
        // out a test that would pass by accident on an all-zero default.
        assert_eq!(
            board_dependent_count.get("bastion:BA.2").copied().flatten(),
            Some(1),
            "BA.3 depends on BA.2, so BA.2 has one dependent (build_board's side)"
        );
        assert_eq!(
            graph_dependent_count.get("bastion:BA.2").copied().flatten(),
            Some(1),
            "BA.3 depends on BA.2, so BA.2 has one dependent (build_block_graph_export's side)"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
