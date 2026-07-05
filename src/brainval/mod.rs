//! `bastion validate-brain` (and siblings in later tasks) — thin pass-through handlers over
//! the `mev` crate's brain-ops library functions (Phase 15, Block BA.15.2 — see D15).
//!
//! Design: keep flag→function selection, exit-code derivation, and output rendering as
//! **pure** functions (unit-tested without touching the filesystem); the actual `mev::*` calls
//! (which walk the filesystem to resolve `brain.toml` and crawl the corpus) are a thin I/O
//! shell over that pure core, smoke-tested and recorded in the task spec's `## Notes`.

use std::path::Path;

use anyhow::Result;

/// Which `mev::validate_brain*` function a `bastion validate-brain` invocation should call,
/// selected from mev's own documented flag precedence:
/// `--links > --structure > --state > --graph > --sync > (base OKF pass)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidateBrainMode {
    Links,
    Structure,
    State,
    Graph,
    Sync,
    Base,
}

impl ValidateBrainMode {
    /// Stable label used in the human summary line (mirrors the mev fn name it maps to).
    pub fn label(&self) -> &'static str {
        match self {
            ValidateBrainMode::Links => "links",
            ValidateBrainMode::Structure => "structure",
            ValidateBrainMode::State => "state",
            ValidateBrainMode::Graph => "graph",
            ValidateBrainMode::Sync => "sync",
            ValidateBrainMode::Base => "base",
        }
    }
}

/// Pure flag→mode selection, mirroring mev's `main.rs` dispatch precedence exactly:
/// `--links > --structure > --state > --graph > --sync > base`. First matching flag wins.
pub fn select_validate_brain_mode(
    sync: bool,
    graph: bool,
    state: bool,
    links: bool,
    structure: bool,
) -> ValidateBrainMode {
    if links {
        ValidateBrainMode::Links
    } else if structure {
        ValidateBrainMode::Structure
    } else if state {
        ValidateBrainMode::State
    } else if graph {
        ValidateBrainMode::Graph
    } else if sync {
        ValidateBrainMode::Sync
    } else {
        ValidateBrainMode::Base
    }
}

/// Exit code from a `mev::Report`: 1 when it carries any error-severity diagnostic, else 0.
pub fn report_to_exit_code(report: &mev::Report) -> u8 {
    if report.is_failure() { 1 } else { 0 }
}

/// Render a `mev::Report` as a human-readable summary: one line per diagnostic followed by
/// a totals line. Mirrors mev's own `main.rs` `print_diagnostic` + summary shape (without
/// mev's terminal color theming, since that's private to mev's binary).
pub fn render_human(report: &mev::Report, root: &Path) -> String {
    let mut out = String::new();
    for d in &report.diagnostics {
        out.push_str(&format!(
            "{} [{}] {} — {}\n",
            d.severity,
            d.locator,
            d.file.display(),
            d.message
        ));
    }
    out.push_str(&format!(
        "validated {}: {} error(s), {} warning(s)",
        root.display(),
        report.error_count(),
        report.warning_count()
    ));
    out
}

/// Serialize a `mev::Report` into mev's machine-readable `JsonReport` envelope — byte-identical
/// to what `mev validate-brain --json` (or the equivalent subcommand) would print, since we
/// reuse mev's own `JsonReport` type rather than defining our own.
pub fn render_json(validator: &str, root: &Path, report: &mev::Report) -> Result<String> {
    mev::JsonReport::new(validator, root, report).to_json()
}

/// Handler for `bastion validate-brain [--sync|--graph|--state|--links|--structure] [--json]`.
///
/// Resolves `brain.toml` by walking up from `path` (mev's own resolution, never a panic —
/// an unresolved config surfaces as an `E_CONFIG_NOT_FOUND` diagnostic inside the `Report`),
/// dispatches to the selected `mev::validate_brain*` function, prints the result (human or
/// `--json`), and returns `Err` when the report is a failure so the process exits non-zero
/// (matching the existing `validate::run` pattern in this binary).
#[allow(clippy::too_many_arguments)]
pub fn run(
    path: std::path::PathBuf,
    sync: bool,
    graph: bool,
    state: bool,
    links: bool,
    structure: bool,
    json: bool,
) -> Result<()> {
    let root = mev::brain::config::find_brain_root(&path)
        .map_err(|e| anyhow::anyhow!("error resolving brain root: {e}"))?;

    let mode = select_validate_brain_mode(sync, graph, state, links, structure);
    let report = match mode {
        ValidateBrainMode::Links => mev::validate_brain_links(&root)?,
        ValidateBrainMode::Structure => mev::validate_brain_structure(&root)?,
        ValidateBrainMode::State => mev::validate_brain_state(&root)?,
        ValidateBrainMode::Graph => mev::validate_brain_graph(&root)?,
        ValidateBrainMode::Sync => mev::validate_brain_sync(&root)?,
        ValidateBrainMode::Base => mev::validate_brain(&root)?,
    };

    if json {
        println!("{}", render_json("brain", &root, &report)?);
    } else {
        println!("{}", render_human(&report, &root));
    }

    if report.is_failure() {
        anyhow::bail!(
            "validate-brain ({}) found {} error(s)",
            mode.label(),
            report.error_count()
        );
    }
    Ok(())
}

/// Pure serialization of a `mev::Manifest` to JSON — compact by default, pretty when
/// `pretty` is set. Mirrors mev's own `main.rs` `Manifest` command output exactly.
pub fn render_manifest_json(manifest: &mev::Manifest, pretty: bool) -> Result<String> {
    let json = if pretty {
        serde_json::to_string_pretty(manifest)?
    } else {
        serde_json::to_string(manifest)?
    };
    Ok(json)
}

/// Handler for `bastion manifest [--pretty]`. Thin pass-through to `mev::manifest_brain`.
pub fn run_manifest(path: std::path::PathBuf, pretty: bool) -> Result<()> {
    let root = mev::brain::config::find_brain_root(&path)
        .map_err(|e| anyhow::anyhow!("error resolving brain root: {e}"))?;
    let manifest = mev::manifest_brain(&root)?;
    println!("{}", render_manifest_json(&manifest, pretty)?);
    Ok(())
}

/// Pure serialization of a `mev::GraphExport` to compact JSON. Mirrors mev's own `main.rs`
/// `EmitGraph` command's default (non-pretty) output exactly.
pub fn render_graph_json(export: &mev::GraphExport) -> Result<String> {
    Ok(serde_json::to_string(export)?)
}

/// Handler for `bastion graph`. Thin pass-through to `mev::graph_brain`.
pub fn run_graph(path: std::path::PathBuf) -> Result<()> {
    let root = mev::brain::config::find_brain_root(&path)
        .map_err(|e| anyhow::anyhow!("error resolving brain root: {e}"))?;
    let export = mev::graph_brain(&root)?;
    println!("{}", render_graph_json(&export)?);
    Ok(())
}

/// Handler for `bastion emit-state [--write]`. Thin pass-through to `mev::emit_state` —
/// dry-run by default, reports planned (or applied) actions via the same human summary
/// shape used by mev's own `EmitState` command.
pub fn run_emit_state(path: std::path::PathBuf, write: bool) -> Result<()> {
    let root = mev::brain::config::find_brain_root(&path)
        .map_err(|e| anyhow::anyhow!("error resolving brain root: {e}"))?;
    let report = mev::emit_state(&root, write)?;

    for d in &report.diagnostics {
        println!(
            "{} [{}] {} — {}",
            d.severity,
            d.locator,
            d.file.display(),
            d.message
        );
    }
    let mode = if write { "write" } else { "dry-run" };
    println!(
        "emit-state {} {}: {} error(s), {} warning(s)",
        mode,
        root.display(),
        report.error_count(),
        report.warning_count()
    );

    if report.is_failure() {
        anyhow::bail!(
            "emit-state ({}) found {} error(s)",
            mode,
            report.error_count()
        );
    }
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use mev::{Diagnostic, Report};

    // ── select_validate_brain_mode — precedence: links > structure > state > graph > sync > base ──

    #[test]
    fn selects_base_when_no_flags() {
        assert_eq!(
            select_validate_brain_mode(false, false, false, false, false),
            ValidateBrainMode::Base
        );
    }

    #[test]
    fn selects_sync_when_only_sync() {
        assert_eq!(
            select_validate_brain_mode(true, false, false, false, false),
            ValidateBrainMode::Sync
        );
    }

    #[test]
    fn selects_graph_when_only_graph() {
        assert_eq!(
            select_validate_brain_mode(false, true, false, false, false),
            ValidateBrainMode::Graph
        );
    }

    #[test]
    fn selects_state_when_only_state() {
        assert_eq!(
            select_validate_brain_mode(false, false, true, false, false),
            ValidateBrainMode::State
        );
    }

    #[test]
    fn selects_links_when_only_links() {
        assert_eq!(
            select_validate_brain_mode(false, false, false, true, false),
            ValidateBrainMode::Links
        );
    }

    #[test]
    fn selects_structure_when_only_structure() {
        assert_eq!(
            select_validate_brain_mode(false, false, false, false, true),
            ValidateBrainMode::Structure
        );
    }

    #[test]
    fn graph_beats_sync() {
        assert_eq!(
            select_validate_brain_mode(true, true, false, false, false),
            ValidateBrainMode::Graph
        );
    }

    #[test]
    fn state_beats_graph_and_sync() {
        assert_eq!(
            select_validate_brain_mode(true, true, true, false, false),
            ValidateBrainMode::State
        );
    }

    #[test]
    fn structure_beats_state_graph_sync() {
        assert_eq!(
            select_validate_brain_mode(true, true, true, false, true),
            ValidateBrainMode::Structure
        );
    }

    #[test]
    fn links_beats_everything() {
        assert_eq!(
            select_validate_brain_mode(true, true, true, true, true),
            ValidateBrainMode::Links
        );
    }

    #[test]
    fn mode_labels_are_stable() {
        assert_eq!(ValidateBrainMode::Links.label(), "links");
        assert_eq!(ValidateBrainMode::Structure.label(), "structure");
        assert_eq!(ValidateBrainMode::State.label(), "state");
        assert_eq!(ValidateBrainMode::Graph.label(), "graph");
        assert_eq!(ValidateBrainMode::Sync.label(), "sync");
        assert_eq!(ValidateBrainMode::Base.label(), "base");
    }

    // ── report_to_exit_code ────────────────────────────────────────────────────

    #[test]
    fn exit_code_zero_for_empty_report() {
        let report = Report::default();
        assert_eq!(report_to_exit_code(&report), 0);
    }

    #[test]
    fn exit_code_zero_for_warnings_only() {
        let mut report = Report::default();
        report
            .diagnostics
            .push(Diagnostic::warning("f.md", "loc", "just a warning"));
        assert_eq!(report_to_exit_code(&report), 0);
    }

    #[test]
    fn exit_code_one_for_any_error() {
        let mut report = Report::default();
        report
            .diagnostics
            .push(Diagnostic::warning("f.md", "loc", "a warning"));
        report
            .diagnostics
            .push(Diagnostic::error("f.md", "loc", "an error"));
        assert_eq!(report_to_exit_code(&report), 1);
    }

    // ── render_human ───────────────────────────────────────────────────────────

    #[test]
    fn render_human_empty_report() {
        let report = Report::default();
        let out = render_human(&report, Path::new("/brain"));
        assert_eq!(out, "validated /brain: 0 error(s), 0 warning(s)");
    }

    #[test]
    fn render_human_includes_each_diagnostic() {
        let mut report = Report::default();
        report
            .diagnostics
            .push(Diagnostic::error("docs/a.md", "E_LOC", "bad thing"));
        report
            .diagnostics
            .push(Diagnostic::warning("docs/b.md", "W_LOC", "minor thing"));
        let out = render_human(&report, Path::new("/brain"));
        assert!(out.contains("docs/a.md"));
        assert!(out.contains("E_LOC"));
        assert!(out.contains("bad thing"));
        assert!(out.contains("docs/b.md"));
        assert!(out.contains("W_LOC"));
        assert!(out.contains("minor thing"));
        assert!(out.contains("1 error(s), 1 warning(s)"));
    }

    // ── render_json ────────────────────────────────────────────────────────────

    #[test]
    fn render_json_round_trips_counts() {
        let mut report = Report::default();
        report
            .diagnostics
            .push(Diagnostic::error("a.md", "E_X", "boom"));
        let json = render_json("brain", Path::new("/brain"), &report).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["validator"], "brain");
        assert_eq!(parsed["root"], "/brain");
        assert_eq!(parsed["errors"], 1);
        assert_eq!(parsed["warnings"], 0);
        assert_eq!(parsed["diagnostics"].as_array().unwrap().len(), 1);
    }

    // ── render_manifest_json ───────────────────────────────────────────────────

    fn sample_manifest() -> mev::Manifest {
        mev::Manifest {
            version: "1".to_string(),
            root: "/brain".to_string(),
            entries: vec![mev::ManifestEntry {
                rel: "docs/a.md".to_string(),
                scope: "brain".to_string(),
                doc_id: Some("a".to_string()),
                doc_type: Some("Guideline".to_string()),
                title: Some("A".to_string()),
                description: Some("desc".to_string()),
                layer: None,
                project: None,
                status: None,
                keywords: None,
                related: None,
                synced_from: None,
            }],
        }
    }

    #[test]
    fn render_manifest_json_compact_has_no_indentation() {
        let manifest = sample_manifest();
        let json = render_manifest_json(&manifest, false).unwrap();
        assert!(!json.contains('\n'));
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["root"], "/brain");
        assert_eq!(parsed["entries"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn render_manifest_json_pretty_is_indented() {
        let manifest = sample_manifest();
        let json = render_manifest_json(&manifest, true).unwrap();
        assert!(json.contains('\n'));
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["root"], "/brain");
    }

    // ── render_graph_json ──────────────────────────────────────────────────────

    #[test]
    fn render_graph_json_round_trips() {
        let export = mev::GraphExport {
            version: "1".to_string(),
            root: "/brain".to_string(),
            nodes: vec![],
            edges: vec![],
            leaves: vec!["brain:x".to_string()],
        };
        let json = render_graph_json(&export).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["root"], "/brain");
        assert_eq!(parsed["leaves"].as_array().unwrap().len(), 1);
    }

    // ── run — I/O shell smoke coverage (missing brain.toml degrades to a diagnostic) ──

    #[test]
    fn run_on_path_without_brain_toml_errors_cleanly() {
        // A path with no brain.toml anywhere up its ancestry (a fresh tempdir under the
        // OS temp root) surfaces as an anyhow error from find_brain_root — no panic.
        let dir = std::env::temp_dir().join(format!(
            "bastion-brainval-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let result = run(dir.clone(), false, false, false, false, false, false);
        assert!(
            result.is_err(),
            "expected an error when brain.toml is unresolvable"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_manifest_on_path_without_brain_toml_errors_cleanly() {
        let dir = std::env::temp_dir().join(format!(
            "bastion-brainval-manifest-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let result = run_manifest(dir.clone(), false);
        assert!(
            result.is_err(),
            "expected an error when brain.toml is unresolvable"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_graph_on_path_without_brain_toml_errors_cleanly() {
        let dir = std::env::temp_dir().join(format!(
            "bastion-brainval-graph-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let result = run_graph(dir.clone());
        assert!(
            result.is_err(),
            "expected an error when brain.toml is unresolvable"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Build a temp brain root containing a minimal valid `brain.toml` plus a minimal
    /// leaf-shaped `planning/state.json`, so `find_brain_root`/`find_brain_config`
    /// resolve successfully and the state pipeline (`discover_state_files` /
    /// `load_state`) has something well-formed to load. Returns the directory —
    /// callers are responsible for `remove_dir_all` teardown.
    fn make_temp_brain_root(name_prefix: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bastion-{}-{}-{}",
            name_prefix,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let planning_dir = dir.join("planning");
        std::fs::create_dir_all(&planning_dir).unwrap();

        std::fs::write(
            dir.join("brain.toml"),
            r#"[vocab]
layer = ["console"]
status = ["active"]

[crawl]
skip_dirs = ["target", ".git"]

[[repos]]
slug = "bastion"
tier = "core"
repo_path = "."
status_file = "planning/status.md"
cache_doc = "docs/projects/bastion.md"
heading = "bastion"
"#,
        )
        .unwrap();

        std::fs::write(
            planning_dir.join("state.json"),
            r#"{
  "repo": "bastion",
  "kind": "project",
  "updated": "2026-07-04",
  "focus": {
    "now": [{ "id": "BA.16.A", "title": "State surface viewer safety", "status": "in_progress" }],
    "next": [],
    "blocked": []
  },
  "tracks": [
    {
      "title": "Phase 16",
      "blocks": [
        { "id": "BA.16.A", "title": "State surface viewer safety", "status": "open" }
      ]
    }
  ]
}"#,
        )
        .unwrap();

        dir
    }

    #[test]
    fn run_emit_state_on_valid_brain_root_succeeds() {
        let dir = make_temp_brain_root("brainval-emit-state-ok");
        let result = run_emit_state(dir.clone(), false);
        assert!(
            result.is_ok(),
            "expected Ok(()) for a valid brain root, got: {result:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validate_brain_run_on_valid_brain_root_succeeds() {
        let dir = make_temp_brain_root("brainval-validate-ok");
        let mode = select_validate_brain_mode(false, false, false, false, false);
        assert_eq!(mode, ValidateBrainMode::Base);

        let result = run(dir.clone(), false, false, false, false, false, false);
        assert!(
            result.is_ok(),
            "expected Ok(()) for a valid brain root (base validate-brain mode), got: {result:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_emit_state_on_path_without_brain_toml_reports_config_error() {
        // run_emit_state resolves the root via find_brain_root first (same as the other
        // handlers) — a path with no brain.toml anywhere up its ancestry surfaces as an
        // anyhow error there, before mev::emit_state is ever called.
        let dir = std::env::temp_dir().join(format!(
            "bastion-brainval-emit-state-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let result = run_emit_state(dir.clone(), false);
        assert!(
            result.is_err(),
            "expected an error when brain.toml is unresolvable"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
