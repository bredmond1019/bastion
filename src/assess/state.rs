//! State-readiness family for `bastion assess` (Phase 15, Block BA.15.9, task 5).
//!
//! Pure classification over an already-loaded state result — no filesystem access in
//! [`classify_state`] itself — plus a thin [`assess_state`] shell that resolves
//! `<planning_root>/state.json` via `okf_core::state::load_state`. Never panics and
//! never propagates an error out of the family: an unreadable/invalid/absent
//! `state.json` is itself a *finding*, since `assess` is a diagnostic, not a validator
//! that should abort on a bad input.

use std::path::Path;

use okf_core::{StateFile, StateLoadError, load_state};
use serde::{Deserialize, Serialize};

/// One distinct state-readiness gap. Each variant is mutually exclusive with the
/// others for a single `assess_state` run — exactly one of `NoPlanningRoot` /
/// `StateFileAbsent` / `StateFileUnparseable` fires when the file could not be loaded
/// at all, and `EmptyFocus`/`EmptyTracks` can both fire together against a
/// successfully-loaded-but-hollow file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StateFinding {
    /// No `planning/` directory was resolved at all (so there is nowhere to even look
    /// for a `state.json`).
    NoPlanningRoot,
    /// `planning/` was resolved but `state.json` does not exist under it.
    StateFileAbsent { path: String },
    /// `state.json` exists but could not be read or parsed.
    StateFileUnparseable { path: String, detail: String },
    /// `state.json` loaded fine but `focus` (`now`/`next`/`blocked`/`deferred`) is
    /// entirely empty.
    EmptyFocus,
    /// `state.json` loaded fine but `tracks[]` is empty.
    EmptyTracks,
}

/// State-readiness family: whether `planning/state.json` resolved, loaded, and
/// carries real content, plus a summary count when it did.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StateFamily {
    /// True only when the file loaded and neither `focus` nor `tracks[]` is empty.
    pub ready: bool,
    /// Every distinct gap found (empty when `ready` is true).
    pub findings: Vec<StateFinding>,
    /// `tracks[].len()`, when the file loaded (0 otherwise).
    pub track_count: usize,
    /// Total blocks summed across every track, when the file loaded (0 otherwise).
    pub block_count: usize,
}

/// Pure classification of an already-loaded `state.json` result.
///
/// `loaded` is `Ok(&StateFile)` on a successful load, or `Err(&StateLoadError)` when
/// [`okf_core::state::load_state`] failed (either the file doesn't exist, or it
/// exists but couldn't be read/parsed as JSON). Distinguishes "absent" from
/// "unparseable" by inspecting the underlying I/O error kind: a `NotFound` I/O error
/// is absence, any other I/O error or a JSON parse error is unparseable.
pub fn classify_state(loaded: Result<&StateFile, &StateLoadError>) -> StateFamily {
    match loaded {
        Err(StateLoadError::Io { path, source }) => {
            let finding = if source.kind() == std::io::ErrorKind::NotFound {
                StateFinding::StateFileAbsent {
                    path: path.display().to_string(),
                }
            } else {
                StateFinding::StateFileUnparseable {
                    path: path.display().to_string(),
                    detail: source.to_string(),
                }
            };
            StateFamily {
                ready: false,
                findings: vec![finding],
                track_count: 0,
                block_count: 0,
            }
        }
        Err(StateLoadError::Parse { path, source }) => StateFamily {
            ready: false,
            findings: vec![StateFinding::StateFileUnparseable {
                path: path.display().to_string(),
                detail: source.to_string(),
            }],
            track_count: 0,
            block_count: 0,
        },
        Ok(state) => {
            let mut findings = Vec::new();

            let focus_empty = state.focus.now.is_empty()
                && state.focus.next.is_empty()
                && state.focus.blocked.is_empty()
                && state.focus.deferred.is_empty();
            if focus_empty {
                findings.push(StateFinding::EmptyFocus);
            }
            if state.tracks.is_empty() {
                findings.push(StateFinding::EmptyTracks);
            }

            let track_count = state.tracks.len();
            let block_count: usize = state.tracks.iter().map(|t| t.blocks.len()).sum();
            let ready = findings.is_empty();

            StateFamily {
                ready,
                findings,
                track_count,
                block_count,
            }
        }
    }
}

/// Thin I/O shell: load `<planning_root>/state.json` (when a `planning_root` was
/// resolved at all) and classify the result.
///
/// Never panics — `planning_root: None`, a missing file, and an unparseable file are
/// each surfaced as a distinct [`StateFinding`] rather than propagated as an error.
pub fn assess_state(planning_root: Option<&Path>) -> StateFamily {
    let Some(root) = planning_root else {
        return StateFamily {
            ready: false,
            findings: vec![StateFinding::NoPlanningRoot],
            track_count: 0,
            block_count: 0,
        };
    };

    let path = root.join("state.json");
    let loaded = load_state(&path);
    classify_state(loaded.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use okf_core::{Block, Focus, Track, TrackBlock};
    use std::fs;
    use tempfile::TempDir;

    fn sample_block() -> Block {
        Block {
            id: "A.1".to_string(),
            title: "one".to_string(),
            status: None,
            note: None,
            repo: None,
            blocked_by: vec![],
            priority: None,
            due: None,
            epics: vec![],
        }
    }

    fn io_not_found(path: &str) -> StateLoadError {
        StateLoadError::Io {
            path: path.into(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "no such file"),
        }
    }

    fn io_other(path: &str) -> StateLoadError {
        StateLoadError::Io {
            path: path.into(),
            source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
        }
    }

    fn parse_error(path: &str) -> StateLoadError {
        let bad: Result<serde_json::Value, _> = serde_json::from_str("{ not json");
        StateLoadError::Parse {
            path: path.into(),
            source: bad.unwrap_err(),
        }
    }

    fn healthy_state() -> StateFile {
        StateFile {
            repo: "bastion".to_string(),
            kind: "project".to_string(),
            updated: "2026-07-27".to_string(),
            focus: Focus {
                now: vec![],
                next: vec![],
                blocked: vec![],
                deferred: vec![],
            },
            tracks: vec![Track {
                title: "Phase 1".to_string(),
                blocks: vec![
                    TrackBlock {
                        id: "A.1".to_string(),
                        title: "one".to_string(),
                        status: None,
                        depends_on: vec![],
                        wave: None,
                        origin: None,
                        priority: None,
                        due: None,
                        sdlc_workflow: None,
                        model: None,
                        epics: vec![],
                        note: None,
                        description: None,
                    },
                    TrackBlock {
                        id: "A.2".to_string(),
                        title: "two".to_string(),
                        status: None,
                        depends_on: vec![],
                        wave: None,
                        origin: None,
                        priority: None,
                        due: None,
                        sdlc_workflow: None,
                        model: None,
                        epics: vec![],
                        note: None,
                        description: None,
                    },
                ],
            }],
            repos: vec![],
            cross_repo: vec![],
            tiers: vec![],
            epics: vec![],
            note: None,
            backlog: vec![],
            carryover: vec![],
        }
    }

    #[test]
    fn classify_state_reports_absent_on_not_found_io_error() {
        let err = io_not_found("planning/state.json");
        let family = classify_state(Err(&err));
        assert!(!family.ready);
        assert_eq!(
            family.findings,
            vec![StateFinding::StateFileAbsent {
                path: "planning/state.json".to_string(),
            }]
        );
    }

    #[test]
    fn classify_state_reports_unparseable_on_non_not_found_io_error() {
        let err = io_other("planning/state.json");
        let family = classify_state(Err(&err));
        assert!(!family.ready);
        match &family.findings[..] {
            [StateFinding::StateFileUnparseable { path, .. }] => {
                assert_eq!(path, "planning/state.json");
            }
            other => panic!("expected StateFileUnparseable, got {other:?}"),
        }
    }

    #[test]
    fn classify_state_reports_unparseable_on_parse_error() {
        let err = parse_error("planning/state.json");
        let family = classify_state(Err(&err));
        assert!(!family.ready);
        match &family.findings[..] {
            [StateFinding::StateFileUnparseable { path, .. }] => {
                assert_eq!(path, "planning/state.json");
            }
            other => panic!("expected StateFileUnparseable, got {other:?}"),
        }
    }

    #[test]
    fn classify_state_reports_empty_focus_and_empty_tracks_when_both_hollow() {
        let mut state = healthy_state();
        state.tracks = vec![];
        let family = classify_state(Ok(&state));
        assert!(!family.ready);
        assert!(family.findings.contains(&StateFinding::EmptyFocus));
        assert!(family.findings.contains(&StateFinding::EmptyTracks));
        assert_eq!(family.track_count, 0);
        assert_eq!(family.block_count, 0);
    }

    #[test]
    fn classify_state_reports_empty_tracks_only_when_focus_populated() {
        let mut state = healthy_state();
        state.tracks = vec![];
        state.focus.now.push(sample_block());
        let family = classify_state(Ok(&state));
        assert!(!family.ready);
        assert_eq!(family.findings, vec![StateFinding::EmptyTracks]);
    }

    #[test]
    fn classify_state_reports_empty_focus_only_when_tracks_populated() {
        let state = healthy_state();
        let family = classify_state(Ok(&state));
        assert!(!family.ready);
        assert_eq!(family.findings, vec![StateFinding::EmptyFocus]);
        assert_eq!(family.track_count, 1);
        assert_eq!(family.block_count, 2);
    }

    #[test]
    fn classify_state_reports_ready_on_a_healthy_file() {
        let mut state = healthy_state();
        state.focus.now.push(sample_block());
        let family = classify_state(Ok(&state));
        assert!(family.ready);
        assert!(family.findings.is_empty());
        assert_eq!(family.track_count, 1);
        assert_eq!(family.block_count, 2);
    }

    #[test]
    fn assess_state_reports_no_planning_root_when_none_resolved() {
        let family = assess_state(None);
        assert!(!family.ready);
        assert_eq!(family.findings, vec![StateFinding::NoPlanningRoot]);
    }

    #[test]
    fn assess_state_reports_absent_over_a_tempfile_fixture_with_no_state_json() {
        let tmp = TempDir::new().unwrap();
        let family = assess_state(Some(tmp.path()));
        assert!(!family.ready);
        match &family.findings[..] {
            [StateFinding::StateFileAbsent { path }] => {
                assert!(path.ends_with("state.json"));
            }
            other => panic!("expected StateFileAbsent, got {other:?}"),
        }
    }

    #[test]
    fn assess_state_reports_ready_over_a_tempfile_fixture_with_a_healthy_state_json() {
        let tmp = TempDir::new().unwrap();
        let json = serde_json::json!({
            "repo": "bastion",
            "kind": "project",
            "updated": "2026-07-27",
            "focus": {
                "now": [{"id": "A.1", "title": "one"}],
                "next": [],
                "blocked": []
            },
            "tracks": [
                {"title": "Phase 1", "blocks": [{"id": "A.1", "title": "one"}]}
            ]
        });
        fs::write(
            tmp.path().join("state.json"),
            serde_json::to_string_pretty(&json).unwrap(),
        )
        .unwrap();

        let family = assess_state(Some(tmp.path()));
        assert!(family.ready);
        assert!(family.findings.is_empty());
        assert_eq!(family.track_count, 1);
        assert_eq!(family.block_count, 1);
    }
}
