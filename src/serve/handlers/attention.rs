//! Attention / carryover board REST handler for `bastion serve` (BA.11.P).
//!
//! Read-only (D25) — this route never mutates any brain/tier/repo `state.json`. It
//! projects the same stale-`carryover[]` / aging-`backlog[]` / orphaned-capture split
//! `mev emit-state` already splices into `status.md` (via
//! `mev::brain::emit::plan_attention_board` / `render_attention_section`) over HTTP,
//! re-expressed against typed DTOs instead of markdown rows.
//!
//! # Route
//! - `GET /api/attention?scope=hq|tier|project|business[&tier=<name>]`
//!
//! Query-param semantics are identical to `GET /api/board` (`handlers::board`) — this
//! handler reuses [`crate::serve::handlers::board::resolve_scope`] rather than
//! re-deriving the mapping.
//!
//! # Pure core vs I/O shell (Rule 6)
//! [`tier_of_repo`] and [`build_attention`] are pure — unit-tested directly, no
//! filesystem access, `today` threaded in as a parameter so results are
//! deterministic. [`get_attention`] is the thin async handler: it resolves a
//! starting path from the shared [`FileConfig`] registry, walks up to the brain
//! root (`mev::brain::config::find_brain_root`), then runs a discover → load
//! pipeline (no rollup/graph — attention doesn't need one) under `web::block`,
//! and hands the pure functions the resulting files.
//!
//! # Error mapping
//! - Brain root unresolvable (no `brain.toml` walking up from the workspace root)
//!   → 500 + `C010` (mirrors `handlers/board.rs`'s mapping; there is no dedicated
//!   "brain not found" C-code and this is an operator-configuration problem, not
//!   a per-request one).
//! - `web::block` thread-pool failure → 500 + `C010`.
//! - Malformed `scope`/`tier` query parsing is handled by actix's `web::Query`
//!   extractor before the handler runs (surfaced as 400).

use std::cmp::Reverse;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use actix_web::{HttpResponse, web};
use chrono::NaiveDate;

use crate::config::{FileConfig, resolve_workspace_root};
use crate::serve::dto::{
    AttentionBacklogDto, AttentionCarryoverDto, AttentionDto, AttentionLanesDto,
    AttentionThresholdsDto, BoardScope, ErrorPayload, render_clears_when,
};
use crate::serve::handlers::board::resolve_scope;

use mev::CarryoverRanking;
use mev::brain::carryover::{evaluate_carryover, rank_carryover};
use mev::brain::config::{AttentionThresholds, BrainConfig, find_brain_root, load_brain_config};
use mev::brain::state::{
    StateSource, TierScope, backlog_stale_age, block_status_map, build_state_graph,
    discover_state_files, effective_priorities, load_state, tier_scope_for,
};
use okf_core::{Backlog, Carryover, StateFile};

/// This handler's own version of `BoardQuery` — identical shape to
/// `handlers::board::BoardQuery`, kept as a separate type so each handler owns
/// its own `web::Query` extractor (they happen to have the same fields today,
/// but nothing requires them to stay that way).
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct AttentionQuery {
    /// `scope=hq|tier|project|business`; missing defaults to [`BoardScope::Hq`].
    #[serde(default)]
    pub scope: BoardScope,
    /// `tier=<name>`; only consulted for `scope=tier`/`scope=project` (default `"core"`).
    #[serde(default)]
    pub tier: Option<String>,
}

// ── Pure core ────────────────────────────────────────────────────────────────────

/// The four-line `[[repos]]` tier lookup that is private in `mev::brain::emit` —
/// re-implemented here rather than patching mev (see the block's Context Pointers).
pub fn tier_of_repo<'a>(slug: &str, config: &'a BrainConfig) -> Option<&'a str> {
    config
        .repos
        .iter()
        .find(|r| r.slug == slug)
        .map(|r| r.tier.as_str())
}

fn thresholds_dto(thresholds: &AttentionThresholds) -> AttentionThresholdsDto {
    AttentionThresholdsDto {
        env_days: thresholds.env_days,
        deferred_days: thresholds.deferred_days,
        known_issue_days: thresholds.known_issue_days,
        constraint_days: thresholds.constraint_days,
        backlog_days: thresholds.backlog_days,
    }
}

/// Project one `mev::CarryoverRanking` (lane/priority/effective_priority/
/// unmet_blocks/finding_id/clears_when_satisfied — never re-derived here, contract
/// §6 rule 1) plus the original loaded `Carryover` item (text/clears_when/created/
/// reviewed, which `CarryoverRanking` does not carry) into the wire DTO.
fn carryover_dto(
    ranking: &CarryoverRanking,
    item: &Carryover,
    threshold_days: i64,
) -> AttentionCarryoverDto {
    AttentionCarryoverDto {
        repo: ranking.repo.clone(),
        slug: ranking.slug.clone(),
        kind: ranking.kind.clone(),
        text: item.text.clone(),
        clears_when: item.clears_when.as_ref().map(render_clears_when),
        created: Some(item.created.clone()),
        reviewed: item.reviewed.clone(),
        age_days: ranking.age_days,
        threshold_days,
        lane: ranking.lane,
        priority: ranking.priority,
        effective_priority: ranking.effective_priority,
        unmet_blocks: ranking.unmet_blocks.clone(),
        finding_id: ranking.finding_id.clone(),
        clears_when_satisfied: ranking.clears_when_satisfied,
    }
}

fn backlog_dto(item: &Backlog, age_days: i64, threshold_days: i64) -> AttentionBacklogDto {
    let is_capture = item.origin.as_ref().is_some_and(|o| o.kind == "capture");
    let notes = if is_capture {
        item.origin
            .as_ref()
            .and_then(|o| o.notes.clone())
            .or_else(|| item.notes.clone())
    } else {
        item.notes.clone()
    };

    AttentionBacklogDto {
        repo: item.repo.clone(),
        slug: item.slug.clone(),
        title: item.title.clone(),
        kind: item.kind.clone(),
        status: item.status.clone(),
        notes,
        created: item.created.clone(),
        reviewed: item.reviewed.clone(),
        age_days,
        threshold_days,
    }
}

/// Project the loaded `(StateSource, StateFile)` pairs into an [`AttentionDto`]
/// for the resolved `tier_scope`, mirroring `mev::brain::emit::plan_attention_board`'s
/// scoping + `render_attention_section`'s lane split against typed DTOs instead
/// of markdown rows.
///
/// Carryover lane membership, priority, and ordering are entirely delegated to
/// `mev::evaluate_carryover` + `mev::rank_carryover` (contract §6 rule 1) — this
/// function builds their inputs (a tier-scoped file set, the block status map, the
/// block effective-priority map, and a `repo → repo_path` lookup) and projects the
/// returned `CarryoverRanking`s verbatim, in the order `rank_carryover` returned
/// them. It never re-derives a lane, re-sorts, or reimplements
/// `mev::brain::state::carryover_stale_age` — that call now lives entirely inside
/// `evaluate_carryover`, still the single staleness definition, still called
/// exactly once per entry. The **full** entry set reaches the ranking (contract
/// §2) — membership is no longer gated on staleness alone.
///
/// Backlog staleness filtering (the anchor rule, snoozing, the backlog status
/// filter) is delegated to `mev::brain::state::backlog_stale_age` — this function
/// never reimplements that predicate either.
///
/// `root` is the resolved brain root, threaded through (never read from disk by
/// this function itself) so `evaluate_carryover` can resolve Class B path
/// references and build each repo's working directory for an opted-in
/// `command_exits_zero` predicate — see the `allow_exec: false` call below.
pub fn build_attention(
    scope: BoardScope,
    resolved_tier: Option<String>,
    tier_scope: &TierScope,
    files: &[(StateSource, StateFile)],
    config: &BrainConfig,
    root: &Path,
    today: NaiveDate,
) -> AttentionDto {
    let thresholds = &config.attention;

    // ── Carryover, ranked by mev ─────────────────────────────────────────────
    let in_scope_files: Vec<(StateSource, StateFile)> = files
        .iter()
        .filter(|(src, _)| match tier_scope {
            TierScope::All => true,
            TierScope::Tier(t) => {
                src.repo_slug == *t || tier_of_repo(&src.repo_slug, config) == Some(t.as_str())
            }
        })
        .cloned()
        .collect();

    // Keyed by (repo, slug) so each `CarryoverRanking` — which does not carry
    // `text`/`clears_when`/`created`/`reviewed` — can be paired back with the
    // loaded item those display fields come from.
    let mut items_by_key: HashMap<(String, String), &Carryover> = HashMap::new();
    for (src, file) in &in_scope_files {
        for item in &file.carryover {
            items_by_key.insert((src.repo_slug.clone(), item.slug.clone()), item);
        }
    }

    let status_map = block_status_map(&in_scope_files);
    let graph = build_state_graph(&in_scope_files);
    let block_priorities = effective_priorities(&graph, &in_scope_files);

    let mut repo_paths: HashMap<String, PathBuf> = HashMap::new();
    for repo in &config.repos {
        let repo_root = if repo.repo_path == "." || repo.repo_path.is_empty() {
            root.to_path_buf()
        } else {
            root.join(&repo.repo_path)
        };
        repo_paths.insert(repo.slug.clone(), repo_root);
    }

    let today_str = today.format("%Y-%m-%d").to_string();
    let report = evaluate_carryover(
        &in_scope_files,
        &status_map,
        root,
        &repo_paths,
        &today_str,
        thresholds,
        None,
        // `allow_exec: false` is a security boundary, not a tuning knob. This is
        // an unauthenticated-by-default read route (`GET /api/attention`) whose
        // input is corpus data — a `command_exits_zero` predicate is authored in
        // `clears_when`, and `allow_exec: true` would let anyone who can write a
        // `planning/state.json` execute an arbitrary shell command on the serve
        // host. Never flip this to `true` or plumb it to a config flag.
        false,
    );

    let ranked = rank_carryover(&report.entries, &block_priorities, &status_map);

    // Preserve `rank_carryover`'s order verbatim — no re-sort.
    let mut stale_carryover: Vec<AttentionCarryoverDto> = Vec::new();
    for ranking in &ranked {
        let Some(item) = items_by_key.get(&(ranking.repo.clone(), ranking.slug.clone())) else {
            // Every ranking is produced from an entry inside `in_scope_files`, so
            // this should never miss — skip rather than panic if the corpus ever
            // violates that invariant (degrade gracefully, per this module's
            // documented error-handling stance).
            continue;
        };
        let threshold_days = thresholds.carryover_threshold(&ranking.kind);
        stale_carryover.push(carryover_dto(ranking, item, threshold_days));
    }

    // ── HQ backlog, tier-filtered ────────────────────────────────────────────
    let hq_backlog: &[Backlog] = files
        .iter()
        .find(|(_, f)| f.kind == "brain" && matches!(tier_scope_for(f, config), TierScope::All))
        .map(|(_, f)| f.backlog.as_slice())
        .unwrap_or(&[]);

    let mut aging_backlog: Vec<AttentionBacklogDto> = Vec::new();
    let mut orphaned_captures: Vec<AttentionBacklogDto> = Vec::new();
    for item in hq_backlog {
        let in_scope = match tier_scope {
            TierScope::All => true,
            TierScope::Tier(t) => tier_of_repo(&item.repo, config) == Some(t.as_str()),
        };
        if !in_scope {
            continue;
        }
        let Some(age) = backlog_stale_age(item, today, thresholds) else {
            continue;
        };
        let dto = backlog_dto(item, age, thresholds.backlog_days);
        let is_capture = item.origin.as_ref().is_some_and(|o| o.kind == "capture");
        if is_capture {
            orphaned_captures.push(dto);
        } else {
            aging_backlog.push(dto);
        }
    }
    aging_backlog.sort_by_key(|b| Reverse(b.age_days));
    orphaned_captures.sort_by_key(|b| Reverse(b.age_days));

    AttentionDto {
        scope,
        tier: resolved_tier,
        as_of: today.format("%Y-%m-%d").to_string(),
        lanes: AttentionLanesDto {
            stale_carryover,
            aging_backlog,
            orphaned_captures,
        },
        thresholds: thresholds_dto(thresholds),
    }
}

// ── I/O shell ──────────────────────────────────────────────────────────────────

/// Build a 500 response from a `BlockingError` (thread panic / runtime shutdown),
/// mirroring `handlers/board.rs::blocking_error_response`.
fn blocking_error_response(err: actix_web::error::BlockingError) -> HttpResponse {
    HttpResponse::InternalServerError().json(ErrorPayload {
        code: "C010".to_owned(),
        message: format!("blocking thread error: {err}"),
    })
}

/// Build a 500 response for a brain-root resolution failure, mirroring
/// `handlers/board.rs::brain_root_error_response`.
fn brain_root_error_response(message: impl std::fmt::Display) -> HttpResponse {
    HttpResponse::InternalServerError().json(ErrorPayload {
        code: "C010".to_owned(),
        message: message.to_string(),
    })
}

/// Assemble the brain-walk inputs `build_attention` needs: `brain.toml`'s
/// `BrainConfig` plus the loaded `(StateSource, StateFile)` pairs. Unlike
/// `assemble_board`, attention needs no `build_state_graph` / `derive_rollup` —
/// it only reads `carryover[]` / `backlog[]` straight off the loaded files.
/// Malformed/unreadable individual `state.json` files are skipped (degrade
/// gracefully) rather than failing the whole request — only an unresolvable
/// brain root is a hard error.
fn assemble_attention(root: &Path) -> Result<(BrainConfig, Vec<(StateSource, StateFile)>), String> {
    let config = load_brain_config(&root.join("brain.toml"))
        .map_err(|e| format!("could not load brain.toml at {}: {e}", root.display()))?;

    let (sources, _discovery_diags) = discover_state_files(root, &config);

    let mut loaded: Vec<(StateSource, StateFile)> = Vec::new();
    for src in &sources {
        if let Ok(file) = load_state(&src.abs_path) {
            loaded.push((src.clone(), file));
        }
    }

    Ok((config, loaded))
}

/// `GET /api/attention?scope=hq|tier|project|business[&tier=<name>]` — stale
/// carryover / aging backlog / orphaned captures Attention board (BA.11.P).
///
/// Bearer auth is inherited from the `/api` scope's `BearerAuthMiddleware` — a
/// request without a valid token never reaches this handler (401 upstream).
pub async fn get_attention(
    query: web::Query<AttentionQuery>,
    registry: web::Data<FileConfig>,
) -> HttpResponse {
    let AttentionQuery { scope, tier } = query.into_inner();
    let (tier_scope, resolved_tier) = resolve_scope(scope, tier.as_deref());
    let today = chrono::Local::now().date_naive();

    let start: PathBuf =
        resolve_workspace_root(None, None, &registry).unwrap_or_else(|_| PathBuf::from("."));

    match web::block(move || -> Result<AttentionDto, String> {
        let root = find_brain_root(&start)
            .map_err(|e| format!("could not resolve brain root from {}: {e}", start.display()))?;
        let (config, files) = assemble_attention(&root)?;
        Ok(build_attention(
            scope,
            resolved_tier,
            &tier_scope,
            &files,
            &config,
            &root,
            today,
        ))
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
    use mev::TriageLane;
    use mev::brain::config::RepoEntry;
    use okf_core::{BacklogOrigin, BlockedBy, CarryoverScope};

    fn sample_config(repos: Vec<RepoEntry>) -> BrainConfig {
        BrainConfig {
            vocab: Default::default(),
            crawl: Default::default(),
            attention: AttentionThresholds {
                env_days: 3,
                deferred_days: 5,
                known_issue_days: 10,
                constraint_days: 10,
                backlog_days: 7,
                knowledge_days: 45,
                memory_days: 30,
            },
            history: Default::default(),
            repos,
        }
    }

    fn repo_entry(slug: &str, tier: &str) -> RepoEntry {
        RepoEntry {
            slug: slug.to_owned(),
            tier: tier.to_owned(),
            repo_path: slug.to_owned(),
            status_file: String::new(),
            cache_doc: String::new(),
            heading: String::new(),
            prefix: None,
        }
    }

    // ── tier_of_repo ─────────────────────────────────────────────────────────

    #[test]
    fn tier_of_repo_hit() {
        let config = sample_config(vec![repo_entry("bastion", "core")]);
        assert_eq!(tier_of_repo("bastion", &config), Some("core"));
    }

    #[test]
    fn tier_of_repo_miss() {
        let config = sample_config(vec![repo_entry("bastion", "core")]);
        assert_eq!(tier_of_repo("nonexistent", &config), None);
    }

    // ── sample fixtures for build_attention ─────────────────────────────────

    /// A brain-root path for `build_attention`'s `evaluate_carryover`/`repo_paths`
    /// plumbing. Never actually read from disk by any fixture in this module — every
    /// `sample_carryover` here authors `clears_when: None`, so `evaluate_carryover`
    /// never reaches a `FileExists`/`FileContains`/`CommandExitsZero` arm that would
    /// touch it.
    fn sample_root() -> PathBuf {
        PathBuf::from("/tmp/bastion-attention-test-root")
    }

    fn sample_source(repo: &str) -> StateSource {
        StateSource {
            repo_slug: repo.to_owned(),
            abs_path: PathBuf::from(format!("/tmp/{repo}/planning/state.json")),
            expected_kind: "project",
        }
    }

    fn sample_carryover(slug: &str, kind: &str, created: &str) -> Carryover {
        Carryover {
            slug: slug.to_owned(),
            scope: CarryoverScope {
                repo: None,
                tier: None,
                cross_repo: None,
            },
            kind: kind.to_owned(),
            text: format!("{slug} text"),
            related: Vec::new(),
            clears_when: None,
            created: created.to_owned(),
            reviewed: None,
            snoozed_until: None,
            ..Default::default()
        }
    }

    /// Like [`sample_carryover`], but authors `priority`/`blocks[]` too — for
    /// fixtures spanning `mev::TriageLane`'s BLOCKING/HOT membership rules, which
    /// `sample_carryover` alone (no priority, no blocks) can never reach.
    fn sample_carryover_ranked(
        slug: &str,
        kind: &str,
        created: &str,
        priority: Option<u8>,
        blocks: Vec<BlockedBy>,
    ) -> Carryover {
        Carryover {
            priority,
            blocks,
            ..sample_carryover(slug, kind, created)
        }
    }

    fn sample_backlog(
        slug: &str,
        repo: &str,
        status: &str,
        created: &str,
        origin: Option<BacklogOrigin>,
    ) -> Backlog {
        Backlog {
            slug: slug.to_owned(),
            title: format!("{slug} title"),
            repo: repo.to_owned(),
            kind: "feature".to_owned(),
            status: status.to_owned(),
            depends_on: Vec::new(),
            block: None,
            notes: Some("fallback notes".to_owned()),
            origin,
            created: Some(created.to_owned()),
            reviewed: None,
            snoozed_until: None,
            ..Default::default()
        }
    }

    fn brain_file(
        repo: &str,
        kind: &str,
        backlog: Vec<Backlog>,
        carryover: Vec<Carryover>,
    ) -> StateFile {
        StateFile {
            epics: Vec::new(),
            repo: repo.to_owned(),
            kind: kind.to_owned(),
            updated: "2026-07-24".to_owned(),
            focus: Default::default(),
            tracks: Vec::new(),
            repos: Vec::new(),
            cross_repo: Vec::new(),
            tiers: Vec::new(),
            note: None,
            backlog,
            carryover,
            ..Default::default()
        }
    }

    fn old_date() -> &'static str {
        "2026-01-01"
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 24).unwrap()
    }

    // ── build_attention — carryover scoping + threshold ──────────────────────

    #[test]
    fn build_attention_hq_unions_carryover_across_all_files() {
        let config = sample_config(vec![repo_entry("bastion", "core")]);
        let files = vec![
            (
                sample_source("."),
                brain_file(
                    ".",
                    "brain",
                    Vec::new(),
                    vec![sample_carryover("hq-item", "known_issue", old_date())],
                ),
            ),
            (
                sample_source("bastion"),
                brain_file(
                    "bastion",
                    "project",
                    Vec::new(),
                    vec![sample_carryover("bastion-item", "known_issue", old_date())],
                ),
            ),
        ];

        let dto = build_attention(
            BoardScope::Hq,
            None,
            &TierScope::All,
            &files,
            &config,
            &sample_root(),
            today(),
        );

        assert_eq!(dto.lanes.stale_carryover.len(), 2);
    }

    #[test]
    fn build_attention_tier_scope_includes_tier_own_file_and_member_repos() {
        let config = sample_config(vec![repo_entry("bastion", "core")]);
        let files = vec![
            (
                sample_source("core"),
                brain_file(
                    "core",
                    "brain",
                    Vec::new(),
                    vec![sample_carryover("core-item", "known_issue", old_date())],
                ),
            ),
            (
                sample_source("bastion"),
                brain_file(
                    "bastion",
                    "project",
                    Vec::new(),
                    vec![sample_carryover("bastion-item", "known_issue", old_date())],
                ),
            ),
            (
                sample_source("other-tier-repo"),
                brain_file(
                    "other-tier-repo",
                    "project",
                    Vec::new(),
                    vec![sample_carryover("other-item", "known_issue", old_date())],
                ),
            ),
        ];

        let dto = build_attention(
            BoardScope::Tier,
            Some("core".to_owned()),
            &TierScope::Tier("core".to_owned()),
            &files,
            &config,
            &sample_root(),
            today(),
        );

        let slugs: Vec<&str> = dto
            .lanes
            .stale_carryover
            .iter()
            .map(|c| c.slug.as_str())
            .collect();
        assert!(slugs.contains(&"core-item"));
        assert!(slugs.contains(&"bastion-item"));
        assert!(!slugs.contains(&"other-item"));
    }

    #[test]
    fn build_attention_fresh_non_stale_carryover_still_appears() {
        // Regression test for the semantic change (contract §2 — Testing Strategy
        // item 3): membership no longer gates on staleness. A carryover created
        // yesterday is not stale, but it must still reach the board — with no
        // authored priority or blocks[], it lands in STANDING. This fails against
        // the old `carryover_stale_age(...).is_some()` membership filter.
        let config = sample_config(vec![repo_entry("bastion", "core")]);
        let recent = today().pred_opt().unwrap().to_string();
        let files = vec![(
            sample_source("bastion"),
            brain_file(
                "bastion",
                "project",
                Vec::new(),
                vec![sample_carryover("fresh-item", "known_issue", &recent)],
            ),
        )];

        let dto = build_attention(
            BoardScope::Hq,
            None,
            &TierScope::All,
            &files,
            &config,
            &sample_root(),
            today(),
        );
        assert_eq!(dto.lanes.stale_carryover.len(), 1);
        assert_eq!(dto.lanes.stale_carryover[0].slug, "fresh-item");
        assert_eq!(dto.lanes.stale_carryover[0].lane, TriageLane::Standing);
    }

    #[test]
    fn build_attention_snoozed_carryover_appears_in_standing_with_no_age() {
        // Snoozing pauses the staleness clock (`age_days` -> `None`, `stale` ->
        // `false`, both derived inside `evaluate_carryover`) but no longer removes
        // the entry from the board — the full entry set reaches the ranking
        // (contract §2). With no authored priority or blocks[] it lands in
        // STANDING, distinct from being hidden entirely as before this block.
        let config = sample_config(vec![repo_entry("bastion", "core")]);
        let mut item = sample_carryover("snoozed-item", "known_issue", old_date());
        item.snoozed_until = Some("2099-01-01".to_owned());
        let files = vec![(
            sample_source("bastion"),
            brain_file("bastion", "project", Vec::new(), vec![item]),
        )];

        let dto = build_attention(
            BoardScope::Hq,
            None,
            &TierScope::All,
            &files,
            &config,
            &sample_root(),
            today(),
        );
        assert_eq!(dto.lanes.stale_carryover.len(), 1);
        assert_eq!(dto.lanes.stale_carryover[0].slug, "snoozed-item");
        assert_eq!(dto.lanes.stale_carryover[0].age_days, None);
        assert_eq!(dto.lanes.stale_carryover[0].lane, TriageLane::Standing);
    }

    #[test]
    fn build_attention_reports_per_kind_threshold_days() {
        let config = sample_config(vec![repo_entry("bastion", "core")]);
        let files = vec![(
            sample_source("bastion"),
            brain_file(
                "bastion",
                "project",
                Vec::new(),
                vec![sample_carryover("env-item", "env", old_date())],
            ),
        )];

        let dto = build_attention(
            BoardScope::Hq,
            None,
            &TierScope::All,
            &files,
            &config,
            &sample_root(),
            today(),
        );
        assert_eq!(dto.lanes.stale_carryover[0].threshold_days, 3);
    }

    #[test]
    fn build_attention_orders_carryover_oldest_first() {
        let config = sample_config(vec![repo_entry("bastion", "core")]);
        let files = vec![(
            sample_source("bastion"),
            brain_file(
                "bastion",
                "project",
                Vec::new(),
                vec![
                    sample_carryover("newer", "known_issue", "2026-06-01"),
                    sample_carryover("older", "known_issue", "2026-01-01"),
                ],
            ),
        )];

        let dto = build_attention(
            BoardScope::Hq,
            None,
            &TierScope::All,
            &files,
            &config,
            &sample_root(),
            today(),
        );
        assert_eq!(dto.lanes.stale_carryover[0].slug, "older");
        assert_eq!(dto.lanes.stale_carryover[1].slug, "newer");
        assert!(dto.lanes.stale_carryover[0].age_days > dto.lanes.stale_carryover[1].age_days);
    }

    #[test]
    fn build_attention_lane_membership_and_order_come_from_mev_not_bastion() {
        // Testing Strategy item 2 (contract §6 rule 1): a fixture set spanning all
        // four lanes, entered in an order that does NOT match lane priority — the
        // projected DTOs must carry exactly the `lane` values `rank_carryover`
        // assigned, in the order it returned them. This function must never
        // re-sort.
        let config = sample_config(vec![repo_entry("bastion", "core")]);
        let files = vec![(
            sample_source("bastion"),
            brain_file(
                "bastion",
                "project",
                Vec::new(),
                vec![
                    // Authored last, but BLOCKING outranks every other lane —
                    // an unmet External block edge is always unmet.
                    sample_carryover_ranked(
                        "blocking-item",
                        "known_issue",
                        old_date(),
                        None,
                        vec![BlockedBy::External {
                            what: "waiting on vendor API".to_owned(),
                        }],
                    ),
                    // Authored first, but HOT (priority 0) ranks below BLOCKING.
                    sample_carryover_ranked("hot-item", "known_issue", old_date(), Some(0), vec![]),
                    // Stale, no priority, no blocks -> AGING.
                    sample_carryover("aging-item", "known_issue", old_date()),
                    // Fresh, no priority, no blocks -> STANDING.
                    sample_carryover(
                        "standing-item",
                        "known_issue",
                        &today().pred_opt().unwrap().to_string(),
                    ),
                ],
            ),
        )];

        let dto = build_attention(
            BoardScope::Hq,
            None,
            &TierScope::All,
            &files,
            &config,
            &sample_root(),
            today(),
        );

        let observed: Vec<(&str, TriageLane)> = dto
            .lanes
            .stale_carryover
            .iter()
            .map(|c| (c.slug.as_str(), c.lane))
            .collect();
        assert_eq!(
            observed,
            vec![
                ("blocking-item", TriageLane::Blocking),
                ("hot-item", TriageLane::Hot),
                ("aging-item", TriageLane::Aging),
                ("standing-item", TriageLane::Standing),
            ]
        );
        assert!(!dto.lanes.stale_carryover[0].unmet_blocks.is_empty());
    }

    #[test]
    fn build_attention_blocking_lane_membership_follows_unmet_blocks_non_emptiness() {
        // Testing Strategy item 5 (contract §6 rule 2): there is no `blocking:
        // bool` field anywhere on the DTO; BLOCKING-lane membership follows
        // `unmet_blocks` non-emptiness exactly.
        let config = sample_config(vec![repo_entry("bastion", "core")]);
        let files = vec![(
            sample_source("bastion"),
            brain_file(
                "bastion",
                "project",
                Vec::new(),
                vec![
                    sample_carryover_ranked(
                        "blocked-item",
                        "known_issue",
                        old_date(),
                        None,
                        vec![BlockedBy::External {
                            what: "waiting on vendor API".to_owned(),
                        }],
                    ),
                    sample_carryover("unblocked-item", "known_issue", old_date()),
                ],
            ),
        )];

        let dto = build_attention(
            BoardScope::Hq,
            None,
            &TierScope::All,
            &files,
            &config,
            &sample_root(),
            today(),
        );

        let blocked = dto
            .lanes
            .stale_carryover
            .iter()
            .find(|c| c.slug == "blocked-item")
            .expect("blocked-item present");
        assert_eq!(blocked.lane, TriageLane::Blocking);
        assert!(!blocked.unmet_blocks.is_empty());

        let unblocked = dto
            .lanes
            .stale_carryover
            .iter()
            .find(|c| c.slug == "unblocked-item")
            .expect("unblocked-item present");
        assert_ne!(unblocked.lane, TriageLane::Blocking);
        assert!(unblocked.unmet_blocks.is_empty());
    }

    #[test]
    fn build_attention_never_executes_a_command_exits_zero_predicate() {
        // Testing Strategy item 4 — `allow_exec: false` is pinned at the
        // `evaluate_carryover` call site inside `build_attention`. Prove it
        // behaviourally: this entry's `clears_when` command would trivially exit
        // zero if it were ever run, so if execution were allowed the entry would
        // land `Cleared` and `clears_when_satisfied` would be `true`. Pinned at
        // `false`, the predicate is never run at all and the entry must never
        // read as satisfied.
        let config = sample_config(vec![repo_entry("bastion", "core")]);
        let mut item = sample_carryover("exec-item", "known_issue", old_date());
        item.clears_when = Some(okf_core::ClearsWhen::Predicate(
            okf_core::ClearsWhenPredicate::CommandExitsZero {
                command: "true".to_owned(),
                note: None,
            },
        ));
        let files = vec![(
            sample_source("bastion"),
            brain_file("bastion", "project", Vec::new(), vec![item]),
        )];

        let dto = build_attention(
            BoardScope::Hq,
            None,
            &TierScope::All,
            &files,
            &config,
            &sample_root(),
            today(),
        );

        assert_eq!(dto.lanes.stale_carryover.len(), 1);
        assert!(!dto.lanes.stale_carryover[0].clears_when_satisfied);
    }

    #[test]
    fn build_attention_empty_blocking_and_hot_is_correct_not_broken() {
        // Testing Strategy item 7 — the live-corpus condition today: no entry
        // authors `priority`, `blocks[]`, or `finding_id`. BLOCKING and HOT must
        // be empty and every entry lands in AGING or STANDING. Pins the
        // expected-empty state so a future reader does not "fix" it.
        let config = sample_config(vec![repo_entry("bastion", "core")]);
        let files = vec![(
            sample_source("bastion"),
            brain_file(
                "bastion",
                "project",
                Vec::new(),
                vec![
                    sample_carryover("stale-item", "known_issue", old_date()),
                    sample_carryover(
                        "fresh-item",
                        "known_issue",
                        &today().pred_opt().unwrap().to_string(),
                    ),
                ],
            ),
        )];

        let dto = build_attention(
            BoardScope::Hq,
            None,
            &TierScope::All,
            &files,
            &config,
            &sample_root(),
            today(),
        );

        assert_eq!(dto.lanes.stale_carryover.len(), 2);
        for entry in &dto.lanes.stale_carryover {
            assert_ne!(entry.lane, TriageLane::Blocking);
            assert_ne!(entry.lane, TriageLane::Hot);
            assert!(matches!(
                entry.lane,
                TriageLane::Aging | TriageLane::Standing
            ));
        }
    }

    // ── build_attention — backlog / capture lane split ────────────────────────

    #[test]
    fn build_attention_splits_capture_from_regular_backlog() {
        let config = sample_config(vec![repo_entry("bastion", "core")]);
        let capture_origin = BacklogOrigin {
            kind: "capture".to_owned(),
            notes: Some("planning/idea-x/notes.md".to_owned()),
        };
        let files = vec![(
            sample_source("."),
            brain_file(
                ".",
                "brain",
                vec![
                    sample_backlog("regular-item", "bastion", "idea", old_date(), None),
                    sample_backlog(
                        "capture-item",
                        "bastion",
                        "idea",
                        old_date(),
                        Some(capture_origin),
                    ),
                ],
                Vec::new(),
            ),
        )];

        let dto = build_attention(
            BoardScope::Hq,
            None,
            &TierScope::All,
            &files,
            &config,
            &sample_root(),
            today(),
        );

        assert_eq!(dto.lanes.aging_backlog.len(), 1);
        assert_eq!(dto.lanes.aging_backlog[0].slug, "regular-item");
        assert_eq!(dto.lanes.orphaned_captures.len(), 1);
        assert_eq!(dto.lanes.orphaned_captures[0].slug, "capture-item");
        assert_eq!(
            dto.lanes.orphaned_captures[0].notes.as_deref(),
            Some("planning/idea-x/notes.md")
        );
    }

    #[test]
    fn build_attention_capture_falls_back_to_node_notes_when_origin_notes_absent() {
        let config = sample_config(vec![repo_entry("bastion", "core")]);
        let capture_origin = BacklogOrigin {
            kind: "capture".to_owned(),
            notes: None,
        };
        let files = vec![(
            sample_source("."),
            brain_file(
                ".",
                "brain",
                vec![sample_backlog(
                    "capture-item",
                    "bastion",
                    "idea",
                    old_date(),
                    Some(capture_origin),
                )],
                Vec::new(),
            ),
        )];

        let dto = build_attention(
            BoardScope::Hq,
            None,
            &TierScope::All,
            &files,
            &config,
            &sample_root(),
            today(),
        );
        assert_eq!(
            dto.lanes.orphaned_captures[0].notes.as_deref(),
            Some("fallback notes")
        );
    }

    #[test]
    fn build_attention_hq_backlog_filtered_by_tier_for_tier_scope() {
        let config = sample_config(vec![
            repo_entry("bastion", "core"),
            repo_entry("other-app", "side"),
        ]);
        let files = vec![(
            sample_source("."),
            brain_file(
                ".",
                "brain",
                vec![
                    sample_backlog("core-item", "bastion", "idea", old_date(), None),
                    sample_backlog("side-item", "other-app", "idea", old_date(), None),
                ],
                Vec::new(),
            ),
        )];

        let dto = build_attention(
            BoardScope::Tier,
            Some("core".to_owned()),
            &TierScope::Tier("core".to_owned()),
            &files,
            &config,
            &sample_root(),
            today(),
        );

        assert_eq!(dto.lanes.aging_backlog.len(), 1);
        assert_eq!(dto.lanes.aging_backlog[0].slug, "core-item");
    }

    #[test]
    fn build_attention_only_hq_backlog_file_is_used_not_tier_files() {
        // A tier brain file's own `backlog[]` (if it had one) must not leak in —
        // only the single kind=="brain" file whose tier_scope_for is All.
        let config = sample_config(vec![repo_entry("bastion", "core")]);
        let files = vec![
            (
                sample_source("."),
                brain_file(
                    ".",
                    "brain",
                    vec![sample_backlog(
                        "hq-item",
                        "bastion",
                        "idea",
                        old_date(),
                        None,
                    )],
                    Vec::new(),
                ),
            ),
            (
                sample_source("core"),
                brain_file(
                    "core",
                    "brain",
                    vec![sample_backlog(
                        "tier-item",
                        "bastion",
                        "idea",
                        old_date(),
                        None,
                    )],
                    Vec::new(),
                ),
            ),
        ];

        let dto = build_attention(
            BoardScope::Hq,
            None,
            &TierScope::All,
            &files,
            &config,
            &sample_root(),
            today(),
        );
        let slugs: Vec<&str> = dto
            .lanes
            .aging_backlog
            .iter()
            .map(|b| b.slug.as_str())
            .collect();
        assert_eq!(slugs, vec!["hq-item"]);
    }

    #[test]
    fn build_attention_under_threshold_backlog_is_absent() {
        let config = sample_config(vec![repo_entry("bastion", "core")]);
        let recent = today().pred_opt().unwrap().to_string();
        let files = vec![(
            sample_source("."),
            brain_file(
                ".",
                "brain",
                vec![sample_backlog(
                    "fresh-item",
                    "bastion",
                    "idea",
                    &recent,
                    None,
                )],
                Vec::new(),
            ),
        )];

        let dto = build_attention(
            BoardScope::Hq,
            None,
            &TierScope::All,
            &files,
            &config,
            &sample_root(),
            today(),
        );
        assert!(dto.lanes.aging_backlog.is_empty());
        assert!(dto.lanes.orphaned_captures.is_empty());
    }

    #[test]
    fn build_attention_snoozed_backlog_is_absent() {
        let config = sample_config(vec![repo_entry("bastion", "core")]);
        let mut item = sample_backlog("snoozed-item", "bastion", "idea", old_date(), None);
        item.snoozed_until = Some("2099-01-01".to_owned());
        let files = vec![(
            sample_source("."),
            brain_file(".", "brain", vec![item], Vec::new()),
        )];

        let dto = build_attention(
            BoardScope::Hq,
            None,
            &TierScope::All,
            &files,
            &config,
            &sample_root(),
            today(),
        );
        assert!(dto.lanes.aging_backlog.is_empty());
    }

    #[test]
    fn build_attention_backlog_with_no_created_or_reviewed_is_absent() {
        let config = sample_config(vec![repo_entry("bastion", "core")]);
        let mut item = sample_backlog("no-anchor-item", "bastion", "idea", old_date(), None);
        item.created = None;
        let files = vec![(
            sample_source("."),
            brain_file(".", "brain", vec![item], Vec::new()),
        )];

        let dto = build_attention(
            BoardScope::Hq,
            None,
            &TierScope::All,
            &files,
            &config,
            &sample_root(),
            today(),
        );
        assert!(dto.lanes.aging_backlog.is_empty());
    }

    #[test]
    fn build_attention_backlog_status_promoted_is_absent() {
        let config = sample_config(vec![repo_entry("bastion", "core")]);
        let item = sample_backlog("promoted-item", "bastion", "promoted", old_date(), None);
        let files = vec![(
            sample_source("."),
            brain_file(".", "brain", vec![item], Vec::new()),
        )];

        let dto = build_attention(
            BoardScope::Hq,
            None,
            &TierScope::All,
            &files,
            &config,
            &sample_root(),
            today(),
        );
        assert!(dto.lanes.aging_backlog.is_empty());
    }

    #[test]
    fn build_attention_orders_backlog_oldest_first() {
        let config = sample_config(vec![repo_entry("bastion", "core")]);
        let files = vec![(
            sample_source("."),
            brain_file(
                ".",
                "brain",
                vec![
                    sample_backlog("newer", "bastion", "idea", "2026-06-01", None),
                    sample_backlog("older", "bastion", "idea", "2026-01-01", None),
                ],
                Vec::new(),
            ),
        )];

        let dto = build_attention(
            BoardScope::Hq,
            None,
            &TierScope::All,
            &files,
            &config,
            &sample_root(),
            today(),
        );
        assert_eq!(dto.lanes.aging_backlog[0].slug, "older");
        assert_eq!(dto.lanes.aging_backlog[1].slug, "newer");
    }

    #[test]
    fn build_attention_backlog_threshold_days_is_backlog_days() {
        let config = sample_config(vec![repo_entry("bastion", "core")]);
        let item = sample_backlog("item", "bastion", "idea", old_date(), None);
        let files = vec![(
            sample_source("."),
            brain_file(".", "brain", vec![item], Vec::new()),
        )];

        let dto = build_attention(
            BoardScope::Hq,
            None,
            &TierScope::All,
            &files,
            &config,
            &sample_root(),
            today(),
        );
        assert_eq!(dto.lanes.aging_backlog[0].threshold_days, 7);
    }

    #[test]
    fn build_attention_business_scope_yields_dto_with_business_tier() {
        let config = sample_config(vec![repo_entry("bastion", "core")]);
        let dto = build_attention(
            BoardScope::Business,
            Some("business".to_owned()),
            &TierScope::Tier("business".to_owned()),
            &[],
            &config,
            &sample_root(),
            today(),
        );
        assert_eq!(dto.scope, BoardScope::Business);
        assert_eq!(dto.tier.as_deref(), Some("business"));
        assert!(dto.lanes.stale_carryover.is_empty());
        assert!(dto.lanes.aging_backlog.is_empty());
        assert!(dto.lanes.orphaned_captures.is_empty());
    }

    #[test]
    fn build_attention_echoes_as_of_and_thresholds() {
        let config = sample_config(vec![repo_entry("bastion", "core")]);
        let dto = build_attention(
            BoardScope::Hq,
            None,
            &TierScope::All,
            &[],
            &config,
            &sample_root(),
            today(),
        );
        assert_eq!(dto.as_of, "2026-07-24");
        assert_eq!(dto.thresholds.env_days, 3);
        assert_eq!(dto.thresholds.deferred_days, 5);
        assert_eq!(dto.thresholds.known_issue_days, 10);
        assert_eq!(dto.thresholds.constraint_days, 10);
        assert_eq!(dto.thresholds.backlog_days, 7);
    }

    // ── assemble_attention — I/O shell, unresolvable brain root degrades cleanly ──

    #[test]
    fn assemble_attention_on_missing_brain_toml_errors_cleanly() {
        let dir = crate::testsupport::unique_temp_dir("bastion-attention-assemble-test");
        std::fs::create_dir_all(&dir).unwrap();
        let result = assemble_attention(&dir);
        assert!(
            result.is_err(),
            "expected an error with no brain.toml present"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
