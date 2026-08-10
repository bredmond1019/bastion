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
use std::path::{Path, PathBuf};

use actix_web::{HttpResponse, web};
use chrono::NaiveDate;

use crate::config::{FileConfig, resolve_workspace_root};
use crate::serve::dto::{
    AttentionBacklogDto, AttentionCarryoverDto, AttentionDto, AttentionLanesDto,
    AttentionThresholdsDto, BoardScope, ErrorPayload, render_clears_when,
};
use crate::serve::handlers::board::resolve_scope;

use mev::brain::config::{AttentionThresholds, BrainConfig, find_brain_root, load_brain_config};
use mev::brain::state::{
    StateSource, TierScope, backlog_stale_age, carryover_stale_age, discover_state_files,
    load_state, tier_scope_for,
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

fn carryover_dto(
    repo: &str,
    item: &Carryover,
    age_days: i64,
    threshold_days: i64,
) -> AttentionCarryoverDto {
    AttentionCarryoverDto {
        repo: repo.to_owned(),
        slug: item.slug.clone(),
        kind: item.kind.clone(),
        text: item.text.clone(),
        clears_when: item.clears_when.as_ref().map(render_clears_when),
        created: Some(item.created.clone()),
        reviewed: item.reviewed.clone(),
        age_days,
        threshold_days,
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
/// Staleness filtering (the anchor rule, snoozing, the backlog status filter) is
/// entirely delegated to `mev::brain::state::{carryover_stale_age, backlog_stale_age}`
/// — this function never reimplements that predicate.
pub fn build_attention(
    scope: BoardScope,
    resolved_tier: Option<String>,
    tier_scope: &TierScope,
    files: &[(StateSource, StateFile)],
    config: &BrainConfig,
    today: NaiveDate,
) -> AttentionDto {
    let thresholds = &config.attention;

    // ── Stale carryover ──────────────────────────────────────────────────────
    let mut stale_carryover: Vec<AttentionCarryoverDto> = Vec::new();
    for (src, file) in files {
        let include = match tier_scope {
            TierScope::All => true,
            TierScope::Tier(t) => {
                src.repo_slug == *t || tier_of_repo(&src.repo_slug, config) == Some(t.as_str())
            }
        };
        if !include {
            continue;
        }
        for item in &file.carryover {
            if let Some(age) = carryover_stale_age(item, today, thresholds) {
                let threshold_days = thresholds.carryover_threshold(&item.kind);
                stale_carryover.push(carryover_dto(&src.repo_slug, item, age, threshold_days));
            }
        }
    }
    stale_carryover.sort_by_key(|c| Reverse(c.age_days));

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
    use mev::brain::config::RepoEntry;
    use okf_core::{BacklogOrigin, CarryoverScope};

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
    fn build_attention_under_threshold_carryover_is_absent() {
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
            today(),
        );
        assert!(dto.lanes.stale_carryover.is_empty());
    }

    #[test]
    fn build_attention_snoozed_carryover_is_absent() {
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
            today(),
        );
        assert!(dto.lanes.stale_carryover.is_empty());
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
            today(),
        );
        assert_eq!(dto.lanes.stale_carryover[0].slug, "older");
        assert_eq!(dto.lanes.stale_carryover[1].slug, "newer");
        assert!(dto.lanes.stale_carryover[0].age_days > dto.lanes.stale_carryover[1].age_days);
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
        let dto = build_attention(BoardScope::Hq, None, &TierScope::All, &[], &config, today());
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
