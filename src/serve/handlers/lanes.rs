//! Fleet-scoped lane-segment availability REST handler for `bastion serve`
//! (`BA.19.C`).
//!
//! Read-only (D25) — this route never mutates any brain/tier/repo
//! `state.json`. It is a pure pass-through over `mev::lanes_brain`'s
//! corpus-wide segment-availability computation: **serve computes nothing**.
//!
//! # Route
//! - `GET /api/lanes` — no query params yet (`?epic=<slug>` lands in a later
//!   task in this spec). Returns one aggregate row per lane SEGMENT across
//!   every registered roadmap in a single call.
//!
//! # Pure core vs I/O shell (Rule 6)
//! [`segment_to_dto`] / [`artifact_to_dto`] / [`availability_to_string`] are
//! pure — unit-tested directly, no filesystem access. [`get_lanes`] is the
//! thin async handler: it resolves a starting path from the shared
//! [`FileConfig`] registry, walks up to the brain root
//! (`mev::brain::config::find_brain_root`), then calls `mev::lanes_brain`
//! under `web::block` — mirroring `handlers/epics.rs::get_epics`'s shape —
//! and hands the result to the pure mapping functions.
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
//! - `web::block` thread-pool failure → 500 + `C010` via
//!   [`board::blocking_error_response`].

use std::path::PathBuf;

use actix_web::{HttpResponse, web};

use crate::config::{FileConfig, resolve_workspace_root};
use crate::serve::dto::{LaneSegmentDto, LanesDto};
use crate::serve::handlers::board;

use mev::brain::availability::{
    LaneAvailabilityArtifact, LaneAvailabilityEntry, SegmentAvailability,
};
use mev::brain::config::find_brain_root;

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

// ── I/O shell ──────────────────────────────────────────────────────────────────

/// `GET /api/lanes` — one aggregate per lane SEGMENT across every registered
/// roadmap, pass-through over `mev::lanes_brain` (`BA.19.C`).
///
/// Bearer auth is inherited from the `/api` scope's `BearerAuthMiddleware` — a
/// request without a valid token never reaches this handler (401 upstream).
pub async fn get_lanes(registry: web::Data<FileConfig>) -> HttpResponse {
    let start: PathBuf =
        resolve_workspace_root(None, None, &registry).unwrap_or_else(|_| PathBuf::from("."));

    match web::block(move || -> Result<LanesDto, String> {
        let root = find_brain_root(&start)
            .map_err(|e| format!("could not resolve brain root from {}: {e}", start.display()))?;
        let artifact = mev::lanes_brain(&root).map_err(|e| e.to_string())?;
        Ok(artifact_to_dto(artifact))
    })
    .await
    {
        Ok(Ok(dto)) => HttpResponse::Ok().json(dto),
        Ok(Err(msg)) => board::brain_root_error_response(msg),
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
}
