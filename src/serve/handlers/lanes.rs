//! Fleet-scoped lane-segment availability REST handler for `bastion serve`
//! (`BA.19.C`).
//!
//! Read-only (D25) — this route never mutates any brain/tier/repo
//! `state.json`. It is a pure pass-through over `mev::lanes_brain`'s
//! corpus-wide segment-availability computation: **serve computes nothing**.
//!
//! # Route
//! - `GET /api/lanes[?epic=<slug>]` — returns one aggregate row per lane
//!   SEGMENT across every registered roadmap in a single call. `?epic=<slug>`
//!   filters that same aggregate in the same call (no per-roadmap fan-out) —
//!   a segment belongs to `?epic=<slug>` when its `roadmap` field equals the
//!   slug (`SegmentStatus` carries `roadmap`, not `epic`; deriving a
//!   per-segment epic from its head block's `epics[]` would be serve
//!   computing something this block forbids). The slug is validated against
//!   the same HQ `epics[]` registry `scope=epic` on `/api/board` uses
//!   ([`board::epic_known`] /
//!   [`crate::serve::handlers::epics::hq_epic_registry`]) — an unknown slug
//!   is a 404/`C005` via [`board::epic_error_response`]; a known slug that
//!   matches no segment is a 200 with an empty `segments` array (a real
//!   answer, not an error). `?epic=` present but blank is treated the same
//!   way `board.rs::epic_param_missing` treats a blank `scope=epic` slug —
//!   not silently ignored.
//!
//! # Pure core vs I/O shell (Rule 6)
//! [`segment_to_dto`] / [`artifact_to_dto`] / [`availability_to_string`] /
//! [`filter_segments_to_epic`] are pure — unit-tested directly, no
//! filesystem access. [`get_lanes`] is the thin async handler: it resolves a
//! starting path from the shared [`FileConfig`] registry, walks up to the
//! brain root (`mev::brain::config::find_brain_root`), then calls
//! `mev::lanes_brain` under `web::block` — mirroring
//! `handlers/epics.rs::get_epics`'s shape — and hands the result to the pure
//! mapping functions. When `?epic=<slug>` is present and non-blank, the same
//! `web::block` closure additionally loads the brain config + state files
//! (mirroring `board::assemble_board`'s discover/load step) to validate the
//! slug against
//! [`crate::serve::handlers::epics::hq_epic_registry`] before filtering.
//!
//! `mev::lanes_brain` walks the corpus and builds an untruncated block
//! graph; it is emphatically not cheap and must never run on the actix
//! worker thread.
//!
//! # Error mapping
//! - Brain root unresolvable (no `brain.toml` walking up from the workspace
//!   root), OR `mev::lanes_brain` itself failing (missing/unreadable
//!   `brain.toml`, or the block-graph export reporting `truncated: true`) →
//!   500 + `C010` via [`board::brain_root_error_response`], message intact.
//!   A `lanes_brain` failure is never mapped to an empty `segments` list —
//!   that would read as "nothing to do" when the truth is "the corpus could
//!   not be measured".
//! - `?epic=` present but blank, or present and not found in the HQ
//!   `epics[]` registry → 404 + `C005` via [`board::epic_error_response`].
//! - `web::block` thread-pool failure → 500 + `C010` via
//!   [`board::blocking_error_response`].

use std::path::PathBuf;

use actix_web::{HttpResponse, web};
use serde::Deserialize;

use crate::config::{FileConfig, resolve_workspace_root};
use crate::serve::dto::{LaneSegmentDto, LanesDto};
use crate::serve::handlers::board;
use crate::serve::handlers::epics::hq_epic_registry;

use mev::brain::availability::{
    LaneAvailabilityArtifact, LaneAvailabilityEntry, SegmentAvailability,
};
use mev::brain::config::{find_brain_root, load_brain_config};
use mev::brain::state::{StateFile, StateSource, discover_state_files, load_state};

// ── Pure core ────────────────────────────────────────────────────────────────

/// `mev::brain::availability::SegmentAvailability`'s kebab-case variant
/// string, via its own `Serialize` impl rather than a hand-matched list —
/// mev owns the vocabulary, and a hand-matched list here would silently stop
/// covering a seventh state the moment mev adds one.
fn availability_to_string(availability: &SegmentAvailability) -> String {
    serde_json::to_value(availability)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned())
}

/// Map one `mev` [`LaneAvailabilityEntry`] onto the wire [`LaneSegmentDto`] —
/// field-for-field, no derived logic. `leverage.lanes_freed` is copied
/// verbatim even on a `done` segment (carryover
/// `lanes-freed-is-history-on-done-segments`); bastion does not zero it.
fn segment_to_dto(entry: LaneAvailabilityEntry) -> LaneSegmentDto {
    let LaneAvailabilityEntry { status, leverage } = entry;
    LaneSegmentDto {
        roadmap: status.roadmap,
        lane: status.lane,
        segment: status.segment,
        repo: status.repo,
        head: status.head,
        availability: availability_to_string(&status.availability),
        reason: status.reason,
        leverage_lanes_freed: leverage.lanes_freed,
    }
}

/// Map a full `mev` [`LaneAvailabilityArtifact`] onto the wire [`LanesDto`] —
/// `derived_at` and `degraded` carried verbatim, `segments` mapped
/// element-for-element via [`segment_to_dto`].
fn artifact_to_dto(artifact: LaneAvailabilityArtifact) -> LanesDto {
    LanesDto {
        derived_at: artifact.derived_at,
        degraded: artifact.degraded,
        segments: artifact.segments.into_iter().map(segment_to_dto).collect(),
    }
}

/// Filter a mapped [`LanesDto`]'s `segments` to those whose `roadmap` field
/// equals `slug`, in place.
///
/// A segment belongs to `?epic=<slug>` when its `roadmap` matches — mev's
/// [`mev::brain::availability::SegmentStatus`] carries `roadmap`, not
/// `epic`; deriving a per-segment epic from its head block's `epics[]`
/// would be serve computing something this block forbids. In this corpus a
/// roadmap is registered as an epic under the same slug, which is what makes
/// the equality meaningful. A known-but-unmatched slug legitimately yields
/// an empty `segments` Vec — that is a real answer ("no lanes for that
/// epic right now"), not an error; the caller is responsible for rejecting
/// an *unknown* slug before this runs.
fn filter_segments_to_epic(dto: &mut LanesDto, slug: &str) {
    dto.segments.retain(|s| s.roadmap == slug);
}

// ── I/O shell ──────────────────────────────────────────────────────────────────

/// `GET /api/lanes` query params. `epic=<slug>` is optional; when present and
/// non-blank it filters the aggregate to segments whose `roadmap` matches. A
/// present-but-blank value is treated as an error (not silently ignored),
/// matching `board.rs::epic_param_missing`'s convention for `scope=epic`.
#[derive(Debug, Deserialize)]
pub struct LanesQuery {
    #[serde(default)]
    pub epic: Option<String>,
}

/// The two error shapes [`get_lanes`]'s `web::block` closure can fail with:
/// an operator-configuration brain-root problem (500/`C010`), or — only when
/// `?epic=<slug>` is present — a slug absent from the HQ `epics[]` registry
/// (404/`C005`). The present-but-blank `epic=` case is checked synchronously
/// before the closure ever runs, so it isn't a variant here.
enum LanesError {
    BrainRoot(String),
    UnknownEpic(String),
}

/// `GET /api/lanes` — one aggregate per lane SEGMENT across every registered
/// roadmap, pass-through over `mev::lanes_brain` (`BA.19.C`).
///
/// Bearer auth is inherited from the `/api` scope's `BearerAuthMiddleware` — a
/// request without a valid token never reaches this handler (401 upstream).
pub async fn get_lanes(
    query: web::Query<LanesQuery>,
    registry: web::Data<FileConfig>,
) -> HttpResponse {
    let epic = query.into_inner().epic;

    if let Some(raw) = epic.as_deref()
        && board::epic_param_missing(Some(raw))
    {
        return board::epic_error_response(
            "?epic=<slug> must be non-empty when the query param is present",
        );
    }

    let start: PathBuf =
        resolve_workspace_root(None, None, &registry).unwrap_or_else(|_| PathBuf::from("."));

    match web::block(move || -> Result<LanesDto, LanesError> {
        let root = find_brain_root(&start).map_err(|e| {
            LanesError::BrainRoot(format!(
                "could not resolve brain root from {}: {e}",
                start.display()
            ))
        })?;
        let artifact = mev::lanes_brain(&root).map_err(|e| LanesError::BrainRoot(e.to_string()))?;
        let mut dto = artifact_to_dto(artifact);

        if let Some(slug) = epic {
            let config = load_brain_config(&root.join("brain.toml")).map_err(|e| {
                LanesError::BrainRoot(format!(
                    "could not load brain.toml at {}: {e}",
                    root.display()
                ))
            })?;
            let (sources, _discovery_diags) = discover_state_files(&root, &config);
            let mut loaded: Vec<(StateSource, StateFile)> = Vec::new();
            for src in &sources {
                if let Ok(file) = load_state(&src.abs_path) {
                    loaded.push((src.clone(), file));
                }
            }
            if !board::epic_known(&slug, hq_epic_registry(&config, &loaded)) {
                return Err(LanesError::UnknownEpic(format!("unknown epic: {slug}")));
            }
            filter_segments_to_epic(&mut dto, &slug);
        }

        Ok(dto)
    })
    .await
    {
        Ok(Ok(dto)) => HttpResponse::Ok().json(dto),
        Ok(Err(LanesError::BrainRoot(msg))) => board::brain_root_error_response(msg),
        Ok(Err(LanesError::UnknownEpic(msg))) => board::epic_error_response(msg),
        Err(err) => board::blocking_error_response(err),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use mev::brain::availability::{LaneLeverage, SegmentStatus};

    fn sample_status(availability: SegmentAvailability) -> SegmentStatus {
        SegmentStatus {
            roadmap: "engine-orchestration".to_owned(),
            lane: "derive".to_owned(),
            segment: 0,
            repo: "mev".to_owned(),
            head: Some("mev:MV.13.C".to_owned()),
            availability,
            reason: None,
        }
    }

    // ── availability_to_string ──────────────────────────────────────────────

    #[test]
    fn availability_to_string_is_kebab_case() {
        assert_eq!(
            availability_to_string(&SegmentAvailability::Startable),
            "startable"
        );
        assert_eq!(
            availability_to_string(&SegmentAvailability::HeldRepoBusy),
            "held-repo-busy"
        );
        assert_eq!(
            availability_to_string(&SegmentAvailability::HeldOperator),
            "held-operator"
        );
        assert_eq!(availability_to_string(&SegmentAvailability::Done), "done");
    }

    // ── segment_to_dto ───────────────────────────────────────────────────────

    #[test]
    fn segment_to_dto_maps_fields_verbatim() {
        let entry = LaneAvailabilityEntry {
            status: sample_status(SegmentAvailability::Startable),
            leverage: LaneLeverage {
                lanes_freed: 2,
                lanes: vec!["engine-orchestration/derive".to_owned()],
            },
        };
        let dto = segment_to_dto(entry);
        assert_eq!(dto.roadmap, "engine-orchestration");
        assert_eq!(dto.lane, "derive");
        assert_eq!(dto.segment, 0);
        assert_eq!(dto.repo, "mev");
        assert_eq!(dto.head.as_deref(), Some("mev:MV.13.C"));
        assert_eq!(dto.availability, "startable");
        assert_eq!(dto.reason, None);
        assert_eq!(dto.leverage_lanes_freed, 2);
    }

    #[test]
    fn segment_to_dto_carries_leverage_verbatim_on_done_segment() {
        // `lanes-freed-is-history-on-done-segments`: mev can report a
        // non-zero lanes_freed on a `done` segment even though the lanes it
        // gated are already free. Bastion must not zero it out.
        let entry = LaneAvailabilityEntry {
            status: SegmentStatus {
                head: None,
                ..sample_status(SegmentAvailability::Done)
            },
            leverage: LaneLeverage {
                lanes_freed: 3,
                lanes: vec!["engine-orchestration/derive".to_owned()],
            },
        };
        let dto = segment_to_dto(entry);
        assert_eq!(dto.availability, "done");
        assert_eq!(dto.head, None);
        assert_eq!(dto.leverage_lanes_freed, 3);
    }

    #[test]
    fn segment_to_dto_preserves_reason_when_held() {
        let entry = LaneAvailabilityEntry {
            status: SegmentStatus {
                reason: Some("blocked by bastion:BA.19.C".to_owned()),
                ..sample_status(SegmentAvailability::HeldBlock)
            },
            leverage: LaneLeverage::default(),
        };
        let dto = segment_to_dto(entry);
        assert_eq!(dto.availability, "held-block");
        assert_eq!(dto.reason.as_deref(), Some("blocked by bastion:BA.19.C"));
    }

    // ── artifact_to_dto ──────────────────────────────────────────────────────

    #[test]
    fn artifact_to_dto_maps_derived_at_and_degraded_verbatim() {
        let artifact = LaneAvailabilityArtifact {
            derived_at: "2026-08-18T10:00:00-07:00".to_owned(),
            degraded: true,
            segments: vec![LaneAvailabilityEntry {
                status: sample_status(SegmentAvailability::Startable),
                leverage: LaneLeverage::default(),
            }],
        };
        let dto = artifact_to_dto(artifact);
        assert_eq!(dto.derived_at, "2026-08-18T10:00:00-07:00");
        assert!(dto.degraded);
        assert_eq!(dto.segments.len(), 1);
    }

    #[test]
    fn artifact_to_dto_empty_segments_maps_to_empty_vec() {
        let artifact = LaneAvailabilityArtifact {
            derived_at: "2026-08-18T10:00:00-07:00".to_owned(),
            degraded: false,
            segments: Vec::new(),
        };
        let dto = artifact_to_dto(artifact);
        assert!(dto.segments.is_empty());
    }

    // ── filter_segments_to_epic ─────────────────────────────────────────────

    fn dto_with_roadmaps(roadmaps: &[&str]) -> LanesDto {
        LanesDto {
            derived_at: "2026-08-18T10:00:00-07:00".to_owned(),
            degraded: false,
            segments: roadmaps
                .iter()
                .map(|roadmap| {
                    segment_to_dto(LaneAvailabilityEntry {
                        status: SegmentStatus {
                            roadmap: (*roadmap).to_owned(),
                            ..sample_status(SegmentAvailability::Startable)
                        },
                        leverage: LaneLeverage::default(),
                    })
                })
                .collect(),
        }
    }

    #[test]
    fn filter_segments_to_epic_keeps_only_matching_roadmap() {
        let mut dto = dto_with_roadmaps(&["engine-orchestration", "other-roadmap"]);
        filter_segments_to_epic(&mut dto, "engine-orchestration");
        assert_eq!(dto.segments.len(), 1);
        assert_eq!(dto.segments[0].roadmap, "engine-orchestration");
    }

    #[test]
    fn filter_segments_to_epic_known_but_unmatched_slug_yields_empty_vec() {
        let mut dto = dto_with_roadmaps(&["engine-orchestration"]);
        filter_segments_to_epic(&mut dto, "no-such-roadmap");
        assert!(dto.segments.is_empty());
    }

    #[test]
    fn filter_segments_to_epic_no_filter_leaves_all_segments() {
        // Sanity check for the "epic absent -> no filtering runs at all" path
        // exercised at the handler level: the pure fn itself, when never
        // called, must not have altered anything (nothing to assert beyond
        // construction succeeding — this documents intent for readers).
        let dto = dto_with_roadmaps(&["engine-orchestration", "other-roadmap"]);
        assert_eq!(dto.segments.len(), 2);
    }
}
