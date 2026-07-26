//! HQ `epics[]` registry selection, shared between the (forthcoming) `GET
//! /api/epics` route and `handlers/board.rs`'s `scope=epic` projection
//! (BA.11.R).
//!
//! Only the pure registry-selection helper is implemented here so far —
//! [`hq_epic_registry`], which `handlers/board.rs` needs to validate
//! `&epic=<slug>` against the HQ registry before filtering the board. The
//! `GET /api/epics` route itself (module doc banners, `build_epics`,
//! `get_epics` I/O shell) lands in a follow-on task in this same block; this
//! module grows in place rather than being duplicated.
//!
//! # Pure core
//! [`hq_epic_registry`] is pure — unit-tested directly, no filesystem access.

use mev::brain::config::BrainConfig;
use mev::brain::state::{StateSource, TierScope, tier_scope_for};
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use mev::brain::config::BrainConfig;
    use okf_core::{Epic, Track};

    fn sample_epic(slug: &str) -> Epic {
        Epic {
            slug: slug.to_owned(),
            title: format!("{slug} title"),
            description: None,
            status: None,
            plan: None,
            repos: Vec::new(),
        }
    }

    fn sample_state_file(kind: &str, epics: Vec<Epic>) -> StateFile {
        StateFile {
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
}
