//! `bastion coord status [--json]` — CLI face for engine-core's coordination reader
//! (`BA.25.A` task 1).
//!
//! This module owns NO reader logic of its own. It calls
//! `engine_core::coord::read_coordination_view` — the exact same function
//! `engine-serve`'s `GET /api/coordination` handler calls (`engine-serve/src/http.rs`,
//! `get_coordination`) — resolves `brain_root` the same way that handler does
//! (`engine_core::brain_root::resolve_brain_root()`), and either prints a
//! human-readable summary or the same `serde_json` serialisation the route emits.
//!
//! Reimplementing the reader, the degradation classification, or a second brain-root
//! resolution path is out of scope — that's engine-rs's `EN.15.A`.
//!
//! ## What "Live" does and does not mean here
//!
//! A `CoordinationStatus::Live` result only means every artifact this reader looked at
//! parsed cleanly and every cross-check agreed — it is silent on whether any
//! coordination activity has ever happened. An absent `.fleet-locks` directory (or an
//! empty one) is reported `Live` with zero entries everywhere; that is "nothing has run
//! yet", not "the coordination surface is healthy and populated". Nothing in this
//! module, and no test in this file, treats a clean `Live` verdict as evidence the
//! surface is populated or that its records are current-shape.

use std::path::Path;

use anyhow::{Context, Result};

use engine_core::coord::{CoordinationStatus, CoordinationView};

/// Handler for `bastion coord status [--json]`. Resolves `brain_root` exactly as the
/// route handler does, reads the joined coordination view, and prints either the
/// human summary or the `serde_json` serialisation of the view.
pub fn run_status(json: bool) -> Result<()> {
    let brain_root =
        engine_core::brain_root::resolve_brain_root().context("cannot resolve brain root")?;
    let view = view_for(&brain_root);
    let output = if json {
        json_output(&view)?
    } else {
        human_summary(&view)
    };
    println!("{output}");
    Ok(())
}

/// Read the joined coordination view for `brain_root`. A one-line wrapper kept
/// separate so callers (and tests) can supply a fixture root without going through
/// `resolve_brain_root()`'s cwd/env resolution. Calls
/// `engine_core::coord::read_coordination_view` directly — no reimplementation.
fn view_for(brain_root: &Path) -> CoordinationView {
    engine_core::coord::read_coordination_view(brain_root)
}

/// Serialise `view` exactly as `engine-serve`'s `GET /api/coordination` route does —
/// `HttpResponse::Ok().json(view)`, which is `serde_json`'s default (compact)
/// serialisation of the same `CoordinationView` type. Pure and independently testable
/// from the read (per this repo's construction-vs-execution split, standing rule 6).
fn json_output(view: &CoordinationView) -> Result<String> {
    serde_json::to_string(view).context("failed to serialise CoordinationView")
}

/// Render a human-readable summary of `view`. Pure and independently testable from the
/// read. Never characterises a `Live` status as "healthy" or "populated" — only as
/// "every artifact read parsed cleanly", which is what the type actually guarantees.
fn human_summary(view: &CoordinationView) -> String {
    let mut lines = Vec::new();

    let status_line = match view.status {
        CoordinationStatus::Live => "status: live (every artifact read parsed cleanly)",
        CoordinationStatus::Degraded => "status: degraded",
    };
    lines.push(status_line.to_string());

    lines.push(format!("registry claims: {}", view.registry.len()));
    lines.push(format!("leases: {}", view.leases.len()));
    lines.push(format!("slots: {}", view.slots.len()));
    lines.push(format!("messages: {}", view.messages.len()));
    lines.push(format!("heartbeats: {}", view.heartbeats.len()));
    lines.push(format!("escalations: {}", view.escalations.len()));
    lines.push(format!("run records: {}", view.run_records.len()));

    if view.degradation_reasons.is_empty() {
        lines.push("degradation reasons: none".to_string());
    } else {
        lines.push(format!(
            "degradation reasons: {}",
            view.degradation_reasons.len()
        ));
        for reason in &view.degradation_reasons {
            lines.push(format!("  - {}: {}", reason.path, reason.reason));
        }
    }

    lines.join("\n")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// AC-1's gated half (task 1): the CLI's `--json` output bytes equal
    /// `serde_json::to_string(&engine_core::coord::read_coordination_view(root))` for
    /// the identical root — same function, same serialisation the route handler uses.
    /// The live `curl` comparison against a running `bastion serve` is task 4's
    /// declared un-gateable half, not this test.
    #[test]
    fn json_output_matches_route_serialization() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // No .fleet-locks subtree at all — a legitimate empty/Live tree, used here only
        // to exercise the serialisation path, not to assert anything about liveness.
        let view = view_for(tmp.path());

        let cli_bytes = json_output(&view).expect("json_output");
        let route_bytes =
            serde_json::to_string(&engine_core::coord::read_coordination_view(tmp.path()))
                .expect("route serialisation");

        assert_eq!(
            cli_bytes, route_bytes,
            "CLI --json output must byte-equal serde_json of the same read_coordination_view call"
        );
    }

    /// The CLI's own read (`view_for`) and a fresh call to the underlying reader must
    /// agree field-for-field on the same fixture root, not merely serialise the same.
    #[test]
    fn view_for_delegates_to_engine_core_reader() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cli_view = view_for(tmp.path());
        let direct_view = engine_core::coord::read_coordination_view(tmp.path());
        assert_eq!(cli_view, direct_view);
    }

    #[test]
    fn human_summary_reports_status_and_counts() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let view = view_for(tmp.path());
        let summary = human_summary(&view);
        assert!(summary.contains("status:"));
        assert!(summary.contains("registry claims: 0"));
        assert!(summary.contains("slots: 0"));
    }

    #[test]
    fn human_summary_lists_degradation_reasons_when_present() {
        use engine_core::coord::DegradationReason;

        let view = CoordinationView {
            status: CoordinationStatus::Degraded,
            degradation_reasons: vec![DegradationReason {
                path: "/tmp/bad.json".to_string(),
                reason: "could not parse JSON".to_string(),
            }],
            registry: Vec::new(),
            leases: Vec::new(),
            slots: Vec::new(),
            messages: Vec::new(),
            heartbeats: Vec::new(),
            escalations: Vec::new(),
            run_records: Vec::new(),
        };
        let summary = human_summary(&view);
        assert!(summary.contains("status: degraded"));
        assert!(summary.contains("degradation reasons: 1"));
        assert!(summary.contains("/tmp/bad.json"));
        assert!(summary.contains("could not parse JSON"));
    }
}
