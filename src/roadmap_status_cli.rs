//! `bastion roadmap-status --roadmap <slug> [--json]` — CLI face for engine-core's typed
//! roadmap-status join (`EN.15.H`), kept BESIDE `/roadmap-status`'s existing Python path
//! rather than replacing it (`BA.25.E` task 1).
//!
//! This module owns NO discovery logic of its own. It calls
//! `engine_core::roadmap_status::discover` directly — the same four-artifact join
//! (`lane-log.jsonl`, `planning/orchestration-run/`, per-spec `sdlc/*state.json`, and each
//! repo's `state.json`) engine-rs's own typed route joins — resolves `brain_root` the same
//! way `coord_cli::run_status` does (`engine_core::brain_root::resolve_brain_root()`), and
//! either prints a human-readable summary or the same `serde_json` serialisation of the
//! result.
//!
//! Reimplementing the join, the lane-log parser, or a second brain-root resolution path is
//! out of scope — that's engine-rs's `EN.15.H`. This module also invents no new malformed-line
//! detection: `RoadmapStatusResult::malformed_lines` already carries it
//! (`crates/engine-core/src/roadmap_status.rs:807`); this module's only job is to surface it in
//! BOTH output modes, since AC-1 requires that, not only in `--json`.

use std::path::Path;

use anyhow::{Context, Result};

use engine_core::roadmap_status::{RoadmapStatusError, RoadmapStatusResult};

/// `bastion roadmap-status --roadmap <slug> [--json]`. Resolves `brain_root` exactly as
/// `coord_cli::run_status` does, runs the typed join, and prints either the human summary or
/// the `serde_json` serialisation of the result. A `NotFound`/`Ambiguous` resolution failure
/// exits non-zero, naming the slug and which case it was — never a bare debug string.
pub fn run_roadmap_status(roadmap: &str, json: bool) -> Result<()> {
    let brain_root =
        engine_core::brain_root::resolve_brain_root().context("cannot resolve brain root")?;
    run_roadmap_status_at(&brain_root, roadmap, json)
}

/// Same as [`run_roadmap_status`], against an already-resolved `root` — the sibling every
/// fixture test in this module drives directly, without going through brain-root discovery.
pub fn run_roadmap_status_at(root: &Path, roadmap: &str, json: bool) -> Result<()> {
    let result = engine_core::roadmap_status::discover(root, roadmap)
        .map_err(|e| describe_error(roadmap, &e))?;

    let output = if json {
        json_output(&result)?
    } else {
        human_summary(&result)
    };
    println!("{output}");
    Ok(())
}

/// Render a [`RoadmapStatusError`] as an `anyhow::Error` that names the slug and which case it
/// was, distinguishable from one another — never a bare `{:?}` debug string.
fn describe_error(roadmap: &str, err: &RoadmapStatusError) -> anyhow::Error {
    match err {
        RoadmapStatusError::NotFound {
            new_dir,
            legacy_dir,
            ..
        } => anyhow::anyhow!(
            "roadmap '{roadmap}' could not be found — no directory at {} or {} (legacy)",
            new_dir.display(),
            legacy_dir.display()
        ),
        RoadmapStatusError::Ambiguous {
            new_dir,
            legacy_dir,
            ..
        } => anyhow::anyhow!(
            "roadmap '{roadmap}' is ambiguous — exists at BOTH {} and {} (legacy); not resolving",
            new_dir.display(),
            legacy_dir.display()
        ),
    }
}

/// Serialise `result` exactly as engine-rs's typed route does — `serde_json`'s default
/// (compact) serialisation of the same [`RoadmapStatusResult`] type. Pure and independently
/// testable from the read, mirroring `coord_cli::json_output`'s split.
fn json_output(result: &RoadmapStatusResult) -> Result<String> {
    serde_json::to_string(result).context("failed to serialise RoadmapStatusResult")
}

/// Render a human-readable summary of `result`: the roadmap slug, its directory, the lane
/// count, and — critically, per AC-1 — every malformed `lane-log.jsonl` line with its byte
/// offset when the list is non-empty. Pure and independently testable from the read.
fn human_summary(result: &RoadmapStatusResult) -> String {
    let mut lines = Vec::new();

    lines.push(format!("roadmap: {}", result.roadmap));
    lines.push(format!("roadmap dir: {}", result.roadmap_dir.display()));
    lines.push(format!("lanes: {}", result.lanes.len()));
    lines.push(format!(
        "repos in lane-log: {}",
        result.repos_in_lane_log.len()
    ));
    lines.push(format!(
        "repos with run-record only: {}",
        result.repos_with_run_record_only.len()
    ));
    lines.push(format!(
        "operator coverage total: {}",
        result.operator_coverage_total
    ));

    if result.malformed_lines.is_empty() {
        lines.push("malformed lane-log lines: none".to_string());
    } else {
        lines.push(format!(
            "malformed lane-log lines: {}",
            result.malformed_lines.len()
        ));
        for m in &result.malformed_lines {
            lines.push(format!(
                "  - {} (line {}, byte offset {}): {}",
                m.path.display(),
                m.line_number,
                m.byte_offset,
                m.error
            ));
        }
    }

    lines.join("\n")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    /// AC-1: human mode (`json: false`) against a fixture roadmap whose `lane-log.jsonl`
    /// carries one malformed (non-JSON) line reports that line's byte offset in its printed
    /// human output — not only in `--json`.
    #[test]
    fn human_summary_reports_malformed_line_byte_offset() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let good_line = r#"{"repo":"engine-rs","lane":"a","block":"EN.1.A","status":"done"}"#;
        let bad_line = "{not valid json";
        let content = format!("{good_line}\n{bad_line}\n");
        write(
            &root.join("planning/roadmaps/demo/lane-log.jsonl"),
            &content,
        );

        let result = engine_core::roadmap_status::discover(root, "demo").expect("resolves");
        assert_eq!(result.malformed_lines.len(), 1);
        let expected_offset = result.malformed_lines[0].byte_offset;

        let summary = human_summary(&result);
        assert!(
            summary.contains(&format!("byte offset {expected_offset}")),
            "expected human summary to report byte offset {expected_offset}, got: {summary}"
        );
        assert!(summary.contains("malformed lane-log lines: 1"));
    }

    /// AC-2: `--json` (`json: true`) against the same fixture serialises `malformed_lines` in
    /// its JSON output — the field is not silently dropped when non-empty.
    #[test]
    fn json_output_serialises_malformed_lines() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let good_line = r#"{"repo":"engine-rs","lane":"a","block":"EN.1.A","status":"done"}"#;
        let bad_line = "{not valid json";
        let content = format!("{good_line}\n{bad_line}\n");
        write(
            &root.join("planning/roadmaps/demo/lane-log.jsonl"),
            &content,
        );

        let result = engine_core::roadmap_status::discover(root, "demo").expect("resolves");
        let json = json_output(&result).expect("json_output");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        let malformed = parsed
            .get("malformed_lines")
            .expect("malformed_lines field present")
            .as_array()
            .expect("malformed_lines is an array");
        assert_eq!(malformed.len(), 1);
    }

    /// AC-3: a nonexistent roadmap slug returns `Err` naming the slug and stating the roadmap
    /// could not be found, distinguishable from the ambiguous-slug case.
    #[test]
    fn run_roadmap_status_at_reports_not_found_naming_slug() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let err =
            run_roadmap_status_at(tmp.path(), "nonexistent-slug", false).expect_err("must err");
        let msg = err.to_string();
        assert!(msg.contains("nonexistent-slug"));
        assert!(msg.contains("could not be found"));
        assert!(!msg.contains("ambiguous"));
    }

    /// The ambiguous case is distinguishable from the not-found case by message text.
    #[test]
    fn run_roadmap_status_at_reports_ambiguous_naming_slug() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        fs::create_dir_all(root.join("planning/roadmaps/demo")).unwrap();
        fs::create_dir_all(root.join("planning/demo")).unwrap();

        let err = run_roadmap_status_at(root, "demo", false).expect_err("must err");
        let msg = err.to_string();
        assert!(msg.contains("demo"));
        assert!(msg.contains("ambiguous"));
    }

    /// `describe_error` never falls back to a bare debug string — each variant gets its own
    /// human-readable, slug-naming text.
    #[test]
    fn describe_error_distinguishes_not_found_from_ambiguous() {
        use std::path::PathBuf;

        let not_found = RoadmapStatusError::NotFound {
            slug: "x".to_string(),
            new_dir: PathBuf::from("/a"),
            legacy_dir: PathBuf::from("/b"),
        };
        let ambiguous = RoadmapStatusError::Ambiguous {
            slug: "x".to_string(),
            new_dir: PathBuf::from("/a"),
            legacy_dir: PathBuf::from("/b"),
        };

        let not_found_msg = describe_error("x", &not_found).to_string();
        let ambiguous_msg = describe_error("x", &ambiguous).to_string();
        assert_ne!(not_found_msg, ambiguous_msg);
        assert!(not_found_msg.contains("could not be found"));
        assert!(ambiguous_msg.contains("ambiguous"));
    }

    /// A clean roadmap (no malformed lines) reports "none" in the human summary, never an
    /// empty/blank section.
    #[test]
    fn human_summary_reports_none_when_no_malformed_lines() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        fs::create_dir_all(root.join("planning/roadmaps/demo")).unwrap();

        let result = engine_core::roadmap_status::discover(root, "demo").expect("resolves");
        let summary = human_summary(&result);
        assert!(summary.contains("malformed lane-log lines: none"));
    }

    /// BA.25.E's cross-tree doc edit: the HQ command doc at
    /// `.claude/commands/roadmap-status.md` must document BOTH the pre-existing Python path
    /// (`roadmap_status_discovery.py`) and this block's Rust face (`bastion roadmap-status`) —
    /// per the spec's acceptance criterion "the Python path still works ... both paths are
    /// offered". This repo sits at `<brain_root>/core/bastion`, so the doc is two levels up —
    /// in the PRIVATE company-brain vault (`agentic-portfolio`), not in this repo's own git
    /// index. GitHub CI clones only `bastion` itself (a public/standalone remote), so that
    /// path is genuinely absent there; it exists only on a developer machine with the full
    /// monorepo checked out. **Skip rather than fail when the doc is unreachable** — this is
    /// the same "public CI checkout can't see the private HQ vault" class every cross-repo
    /// doc check in this fleet has to account for, not a flaky test. This exact edge was hand-
    /// verified against the real doc post-edit (commit `0ba007a2b`, HQ root) — see
    /// `planning/BA.25.E/review.md` — so this test's job is to catch a REGRESSION on a machine
    /// where the doc IS visible, never to gate CI on a path CI cannot see.
    #[test]
    fn hq_roadmap_status_doc_documents_both_paths() {
        let doc_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.claude/commands/roadmap-status.md");
        let Ok(contents) = std::fs::read_to_string(&doc_path) else {
            eprintln!(
                "SKIP: {} not reachable from this checkout (expected in a bastion-only CI clone \
                 — the file lives in the private company-brain vault, not this repo)",
                doc_path.display()
            );
            return;
        };

        assert!(
            contents.contains("roadmap_status_discovery.py"),
            "doc must still document the Python discovery path"
        );
        assert!(
            contents.contains("bastion roadmap-status"),
            "doc must document the Rust `bastion roadmap-status` face"
        );
    }
}
