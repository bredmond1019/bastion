//! HQ `epics[]` cross-repo initiative registry REST handler for `bastion serve`
//! (BA.11.R).
//!
//! Read-only (D25) — this route never mutates any brain/tier/repo `state.json`.
//! It projects the single HQ `epics[]` registry `handlers/board.rs`'s
//! `scope=epic` projection also validates `&epic=<slug>` against, over HTTP.
//!
//! # Route
//! - `GET /api/epics` — no query params. Returns every entry in the HQ
//!   `epics[]` registry.
//!
//! # Pure core vs I/O shell (Rule 6)
//! [`hq_epic_registry`] and [`build_epics`] are pure — unit-tested directly,
//! no filesystem access. [`get_epics`] is the thin async handler: it resolves
//! a starting path from the shared [`FileConfig`] registry, walks up to the
//! brain root (`mev::brain::config::find_brain_root`), then runs a discover →
//! load pipeline (no rollup/graph — the registry doesn't need one, mirroring
//! `handlers/attention.rs`) under `web::block`, and hands the pure functions
//! the resulting files.
//!
//! # Error mapping
//! - Brain root unresolvable (no `brain.toml` walking up from the workspace
//!   root) or `brain.toml` unloadable → 500 + `C010` (mirrors
//!   `handlers/board.rs` / `handlers/attention.rs`'s mapping; there is no
//!   dedicated "brain not found" C-code and this is an operator-configuration
//!   problem, not a per-request one).
//! - `web::block` thread-pool failure → 500 + `C010`.
//! - An absent HQ registry (no `kind:"brain"`/`TierScope::All` file found) is
//!   **not** an error — `get_epics` returns `200` with `[]`; the route reports
//!   the registry it found, and an absent HQ file is an operator-config
//!   problem the board route already surfaces.

use std::path::{Path, PathBuf};

use actix_web::{HttpResponse, web};

use crate::config::{FileConfig, resolve_workspace_root};
use crate::serve::dto::EpicDto;

use mev::brain::config::{BrainConfig, find_brain_root, load_brain_config};
use mev::brain::emit::{epic_members, epic_progress};
use mev::brain::state::{
    StateSource, TierScope, build_state_graph, discover_state_files, load_state, tier_scope_for,
};
use okf_core::{Epic, StateFile};

// ── Pure core ────────────────────────────────────────────────────────────────

/// Select the single HQ `epics[]` registry from the loaded `(StateSource,
/// StateFile)` pairs: the file whose `kind == "brain"` **and** whose
/// [`tier_scope_for`] resolves to [`TierScope::All`].
///
/// Mirrors mev's private `epic_registry` helper (`../mev/src/brain/state.rs:817`,
/// not exported — reimplemented here for the same reason
/// `handlers/board.rs::unmet_deps` reimplements `derive_focus`'s closure). A
/// brain with no such file (or none matching) yields an empty slice — not an
/// error; the registry-miss case is the caller's to handle (an absent HQ
/// registry is a `200` with `[]`, per the block's acceptance criteria).
pub fn hq_epic_registry<'a>(
    config: &BrainConfig,
    files: &'a [(StateSource, StateFile)],
) -> &'a [Epic] {
    files
        .iter()
        .find(|(_, f)| f.kind == "brain" && matches!(tier_scope_for(f, config), TierScope::All))
        .map(|(_, f)| f.epics.as_slice())
        .unwrap_or(&[])
}

/// Project the selected HQ registry slice into the wire [`EpicDto`] shape,
/// one-for-one, preserving order. An empty registry yields an empty `Vec`.
///
/// Each entry is joined against the corpus for its member-block tallies, so a
/// Surface can render an epic's shape — and in particular whether it is entirely
/// parked — without fetching a board per epic. Counts come from
/// [`epic_progress`], the same predicate that drives the collapsed markdown
/// board and `mev sync-epics`, so the three cannot disagree about what a
/// deferred epic is.
pub fn build_epics(registry: &[Epic], files: &[(StateSource, StateFile)]) -> Vec<EpicDto> {
    let graph = build_state_graph(files);
    registry
        .iter()
        .map(|epic| {
            let members = epic_members(&graph, files, &epic.slug);
            let p = epic_progress(&members);
            EpicDto {
                slug: epic.slug.clone(),
                title: epic.title.clone(),
                description: epic.description.clone(),
                status: epic.status.clone(),
                // Verbatim passthrough — mev's `check_epics` owns the 0..=100
                // range policy, so bastion never clamps or defaults it.
                weight: epic.weight,
                plan: epic.plan.clone(),
                repos: epic.repos.clone(),
                closed: p.closed as u32,
                in_progress: p.in_progress as u32,
                open: p.open as u32,
                deferred: p.deferred as u32,
                total: p.total() as u32,
                fully_deferred: p.is_fully_deferred(),
            }
        })
        .collect()
}

// ── I/O shell ──────────────────────────────────────────────────────────────────

/// Build a 500 response from a `BlockingError` (thread panic / runtime shutdown),
/// mirroring `handlers/board.rs::blocking_error_response`.
fn blocking_error_response(err: actix_web::error::BlockingError) -> HttpResponse {
    HttpResponse::InternalServerError().json(crate::serve::dto::ErrorPayload {
        code: "C010".to_owned(),
        message: format!("blocking thread error: {err}"),
    })
}

/// Build a 500 response for a brain-root resolution failure, mirroring
/// `handlers/board.rs::brain_root_error_response`.
fn brain_root_error_response(message: impl std::fmt::Display) -> HttpResponse {
    HttpResponse::InternalServerError().json(crate::serve::dto::ErrorPayload {
        code: "C010".to_owned(),
        message: message.to_string(),
    })
}

/// Assemble the brain-walk inputs [`hq_epic_registry`] needs: `brain.toml`'s
/// [`BrainConfig`] plus the loaded `(StateSource, StateFile)` pairs. Mirrors
/// `handlers/attention.rs::assemble_attention` — no `build_state_graph` /
/// `derive_rollup`, the registry only needs the loaded files themselves.
/// Malformed/unreadable individual `state.json` files are skipped (degrade
/// gracefully) rather than failing the whole request — only an unresolvable
/// brain root / unloadable `brain.toml` is a hard error.
fn assemble_epics(root: &Path) -> Result<(BrainConfig, Vec<(StateSource, StateFile)>), String> {
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

/// `GET /api/epics` — the HQ `epics[]` cross-repo initiative registry
/// (BA.11.R).
///
/// Bearer auth is inherited from the `/api` scope's `BearerAuthMiddleware` — a
/// request without a valid token never reaches this handler (401 upstream).
///
/// An absent HQ registry (no `kind:"brain"`/`TierScope::All` file found) is
/// not an error — this returns `200` with `[]`.
pub async fn get_epics(registry: web::Data<FileConfig>) -> HttpResponse {
    let start: PathBuf =
        resolve_workspace_root(None, None, &registry).unwrap_or_else(|_| PathBuf::from("."));

    match web::block(move || -> Result<Vec<EpicDto>, String> {
        let root = find_brain_root(&start)
            .map_err(|e| format!("could not resolve brain root from {}: {e}", start.display()))?;
        let (config, files) = assemble_epics(&root)?;
        Ok(build_epics(hq_epic_registry(&config, &files), &files))
    })
    .await
    {
        Ok(Ok(epics)) => HttpResponse::Ok().json(epics),
        Ok(Err(msg)) => brain_root_error_response(msg),
        Err(err) => blocking_error_response(err),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use mev::brain::config::BrainConfig;
    use okf_core::{Epic, Track};

    fn sample_epic(slug: &str) -> Epic {
        Epic {
            extra: Default::default(),
            slug: slug.to_owned(),
            title: format!("{slug} title"),
            description: None,
            status: None,
            weight: None,
            plan: None,
            repos: Vec::new(),
        }
    }

    fn sample_state_file(kind: &str, epics: Vec<Epic>) -> StateFile {
        StateFile {
            extra: Default::default(),
            epics,
            repo: "hq".to_owned(),
            kind: kind.to_owned(),
            updated: "2026-07-25".to_owned(),
            focus: Default::default(),
            tracks: Vec::<Track>::new(),
            repos: Vec::new(),
            cross_repo: Vec::new(),
            tiers: Vec::new(),
            note: None,
            backlog: Vec::new(),
            carryover: Vec::new(),
        }
    }

    fn sample_source(repo: &str) -> StateSource {
        StateSource {
            repo_slug: repo.to_owned(),
            abs_path: std::path::PathBuf::from(format!("/tmp/{repo}/planning/state.json")),
            expected_kind: "brain",
        }
    }

    // ── hq_epic_registry ─────────────────────────────────────────────────────

    #[test]
    fn hq_epic_registry_found_on_hq_file() {
        let config = BrainConfig::default();
        let file = sample_state_file("brain", vec![sample_epic("bastion-surfaces")]);
        let files = vec![(sample_source("hq"), file)];

        let registry = hq_epic_registry(&config, &files);
        assert_eq!(registry.len(), 1);
        assert_eq!(registry[0].slug, "bastion-surfaces");
    }

    #[test]
    fn hq_epic_registry_ignores_non_brain_kind_file() {
        let config = BrainConfig::default();
        let file = sample_state_file("project", vec![sample_epic("bastion-surfaces")]);
        let files = vec![(sample_source("bastion"), file)];

        assert!(hq_epic_registry(&config, &files).is_empty());
    }

    #[test]
    fn hq_epic_registry_empty_when_no_files() {
        let config = BrainConfig::default();
        let files: Vec<(StateSource, StateFile)> = Vec::new();

        assert!(hq_epic_registry(&config, &files).is_empty());
    }

    #[test]
    fn hq_epic_registry_empty_when_kind_brain_file_is_tier_scoped_not_all() {
        // A `kind: "brain"` file whose `repo` matches a configured tier's
        // `tier` value scopes to `TierScope::Tier(..)`, not `All` — a
        // tier-container sub-brain, not the HQ root. It must not be selected
        // as the HQ registry even though its `kind` is `"brain"`.
        let config = BrainConfig {
            repos: vec![mev::brain::config::RepoEntry {
                slug: "core".to_owned(),
                tier: "core".to_owned(),
                repo_path: "core".to_owned(),
                status_file: String::new(),
                cache_doc: String::new(),
                heading: String::new(),
                prefix: None,
            }],
            ..Default::default()
        };
        let mut file = sample_state_file("brain", vec![sample_epic("bastion-surfaces")]);
        file.repo = "core".to_owned();
        let files = vec![(sample_source("core"), file)];

        assert!(hq_epic_registry(&config, &files).is_empty());
    }

    #[test]
    fn hq_epic_registry_picks_hq_file_among_multiple() {
        let config = BrainConfig {
            repos: vec![mev::brain::config::RepoEntry {
                slug: "core".to_owned(),
                tier: "core".to_owned(),
                repo_path: "core".to_owned(),
                status_file: String::new(),
                cache_doc: String::new(),
                heading: String::new(),
                prefix: None,
            }],
            ..Default::default()
        };
        let mut tier_file = sample_state_file("brain", vec![sample_epic("not-hq")]);
        tier_file.repo = "core".to_owned();
        let mut hq_file = sample_state_file("brain", vec![sample_epic("bastion-surfaces")]);
        hq_file.repo = "hq".to_owned();
        let files = vec![
            (sample_source("core"), tier_file),
            (sample_source("hq"), hq_file),
        ];

        let registry = hq_epic_registry(&config, &files);
        assert_eq!(registry.len(), 1);
        assert_eq!(registry[0].slug, "bastion-surfaces");
    }

    // ── build_epics ──────────────────────────────────────────────────────────

    /// Build a project file whose blocks carry `(id, status)` in epic `slug`.
    fn member_file(slug: &str, blocks: &[(&str, &str)]) -> StateFile {
        StateFile {
            repo: "alpha".to_owned(),
            kind: "project".to_owned(),
            tracks: vec![Track {
                extra: Default::default(),
                title: "P1".to_owned(),
                blocks: blocks
                    .iter()
                    .map(|(id, status)| okf_core::TrackBlock {
                        extra: Default::default(),
                        id: (*id).to_owned(),
                        title: (*id).to_owned(),
                        status: Some((*status).to_owned()),
                        depends_on: Vec::new(),
                        wave: None,
                        origin: None,
                        priority: None,
                        due: None,
                        sdlc_workflow: None,
                        model: None,
                        epics: vec![slug.to_owned()],
                        note: None,
                        description: None,
                    })
                    .collect(),
            }],
            ..sample_state_file("project", Vec::new())
        }
    }

    #[test]
    fn build_epics_tallies_member_blocks_and_flags_a_fully_deferred_epic() {
        let epic = Epic {
            extra: Default::default(),
            slug: "tui".to_owned(),
            title: "Bastion TUI".to_owned(),
            description: None,
            status: Some("paused".to_owned()),
            weight: None,
            plan: None,
            repos: Vec::new(),
        };
        // Remaining work is entirely deferred; one member is already closed.
        let files = vec![(
            sample_source("alpha"),
            member_file(
                "tui",
                &[
                    ("AL.1.A", "deferred"),
                    ("AL.1.B", "deferred"),
                    ("AL.1.C", "closed"),
                ],
            ),
        )];

        let dtos = build_epics(std::slice::from_ref(&epic), &files);
        let dto = &dtos[0];

        assert_eq!(dto.deferred, 2);
        assert_eq!(dto.closed, 1);
        assert_eq!(dto.open, 0);
        assert_eq!(dto.in_progress, 0);
        assert_eq!(dto.total, 3);
        assert!(
            dto.fully_deferred,
            "a Surface needs this to say 'whole epic parked' on load"
        );
    }

    #[test]
    fn build_epics_does_not_flag_an_epic_with_live_work_as_fully_deferred() {
        let epic = Epic {
            extra: Default::default(),
            slug: "tui".to_owned(),
            title: "Bastion TUI".to_owned(),
            description: None,
            status: Some("active".to_owned()),
            weight: None,
            plan: None,
            repos: Vec::new(),
        };
        let files = vec![(
            sample_source("alpha"),
            member_file("tui", &[("AL.1.A", "deferred"), ("AL.1.B", "open")]),
        )];

        let dto = &build_epics(std::slice::from_ref(&epic), &files)[0];
        assert_eq!(dto.deferred, 1);
        assert_eq!(dto.open, 1);
        assert!(!dto.fully_deferred);
    }

    #[test]
    fn build_epics_does_not_flag_a_fully_closed_epic_as_deferred() {
        let epic = Epic {
            extra: Default::default(),
            slug: "tui".to_owned(),
            title: "Done Initiative".to_owned(),
            description: None,
            status: Some("complete".to_owned()),
            weight: None,
            plan: None,
            repos: Vec::new(),
        };
        let files = vec![(
            sample_source("alpha"),
            member_file("tui", &[("AL.1.A", "closed"), ("AL.1.B", "closed")]),
        )];

        let dto = &build_epics(std::slice::from_ref(&epic), &files)[0];
        assert_eq!(dto.closed, 2);
        assert!(
            !dto.fully_deferred,
            "all-closed is COMPLETE, not deferred — the two must not be conflated"
        );
    }

    #[test]
    fn build_epics_maps_all_seven_fields() {
        let epic = Epic {
            extra: Default::default(),
            slug: "bastion-surfaces".to_owned(),
            title: "Bastion Surfaces".to_owned(),
            description: Some("Cross-repo surfaces initiative".to_owned()),
            status: Some("active".to_owned()),
            weight: Some(80),
            plan: Some("core/planning/master-plan.md".to_owned()),
            repos: vec!["bastion".to_owned(), "bastion-ui".to_owned()],
        };

        let dtos = build_epics(std::slice::from_ref(&epic), &[]);
        assert_eq!(dtos.len(), 1);
        let dto = &dtos[0];
        assert_eq!(dto.slug, "bastion-surfaces");
        assert_eq!(dto.title, "Bastion Surfaces");
        assert_eq!(
            dto.description.as_deref(),
            Some("Cross-repo surfaces initiative")
        );
        assert_eq!(dto.status.as_deref(), Some("active"));
        assert_eq!(
            dto.weight,
            Some(80),
            "authored weight must survive verbatim"
        );
        assert_eq!(dto.plan.as_deref(), Some("core/planning/master-plan.md"));
        assert_eq!(
            dto.repos,
            vec!["bastion".to_owned(), "bastion-ui".to_owned()]
        );
    }

    #[test]
    fn build_epics_minimal_entry_defaults_repos_to_empty() {
        let epic = sample_epic("minimal");
        let dtos = build_epics(std::slice::from_ref(&epic), &[]);
        assert_eq!(dtos.len(), 1);
        assert_eq!(dtos[0].slug, "minimal");
        assert_eq!(dtos[0].title, "minimal title");
        assert!(dtos[0].description.is_none());
        assert!(dtos[0].status.is_none());
        assert!(dtos[0].plan.is_none());
        assert!(dtos[0].repos.is_empty());
        assert!(
            dtos[0].weight.is_none(),
            "an unauthored weight must stay None"
        );
    }

    #[test]
    fn build_epics_passes_out_of_policy_weight_through_untouched() {
        // mev's `check_epics` owns the 0..=100 range policy
        // (`E_STATE_EPIC_BAD_WEIGHT`). bastion derives nothing: an out-of-policy
        // authored value must reach the DTO unclamped, not be silently corrected.
        let epic = Epic {
            weight: Some(200),
            ..sample_epic("out-of-policy")
        };
        let dtos = build_epics(std::slice::from_ref(&epic), &[]);
        assert_eq!(dtos[0].weight, Some(200));
    }

    #[test]
    fn build_epics_preserves_boundary_weights_verbatim() {
        for authored in [0u8, 100u8, 255u8] {
            let epic = Epic {
                weight: Some(authored),
                ..sample_epic("boundary")
            };
            let dtos = build_epics(std::slice::from_ref(&epic), &[]);
            assert_eq!(
                dtos[0].weight,
                Some(authored),
                "weight {authored} must pass through unchanged"
            );
        }
    }

    #[test]
    fn build_epics_empty_registry_yields_empty_vec() {
        let registry: Vec<Epic> = Vec::new();
        assert!(build_epics(&registry, &[]).is_empty());
    }

    #[test]
    fn build_epics_preserves_registry_order() {
        let registry = vec![sample_epic("first"), sample_epic("second")];
        let dtos = build_epics(&registry, &[]);
        assert_eq!(dtos[0].slug, "first");
        assert_eq!(dtos[1].slug, "second");
    }

    // ── assemble_epics — I/O shell, unresolvable brain root degrades cleanly ──

    #[test]
    fn assemble_epics_on_missing_brain_toml_errors_cleanly() {
        let dir = crate::testsupport::unique_temp_dir("bastion-epics-assemble-test");
        std::fs::create_dir_all(&dir).unwrap();
        let result = assemble_epics(&dir);
        assert!(
            result.is_err(),
            "expected an error with no brain.toml present"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
