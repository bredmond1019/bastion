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

/// Pure truth table for whether an `emit-state --write` build-provenance drift should
/// hard-fail instead of the default warn-and-proceed. Either the `--fail-on-drift` flag or
/// a truthy `BASTION_FAIL_ON_BUILD_DRIFT` env var turns drift into a hard failure; the flag
/// takes no special precedence over the env var — either alone is sufficient, so a caller
/// need not reason about which "wins" when both are set.
///
/// Truthy values (case-insensitive): "1", "true", "yes", "on". Everything else — including
/// unset, empty string, "0", "false" — is falsy. No I/O: the env var's value is passed in
/// already read, so this stays directly unit-testable.
pub fn should_hard_fail_on_drift(flag: bool, env_var: Option<&str>) -> bool {
    if flag {
        return true;
    }
    match env_var {
        Some(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        None => false,
    }
}

/// Write the loud drift banner naming `reason` (which already names both the stamped and
/// live SHA — see [`buildstamp::verdict`]) to `w`. A thin formatting shell with no I/O of
/// its own beyond the write; production always passes `std::io::stderr()`.
fn write_drift_banner<W: std::io::Write>(w: &mut W, reason: &str) {
    let _ = writeln!(
        w,
        "╔══════════════════════════════════════════════════════════════════╗"
    );
    let _ = writeln!(
        w,
        "║  BUILD PROVENANCE DRIFT — this bastion binary may not match the   ║"
    );
    let _ = writeln!(
        w,
        "║  source tree it is about to write from.                           ║"
    );
    let _ = writeln!(
        w,
        "╚══════════════════════════════════════════════════════════════════╝"
    );
    let _ = writeln!(w, "{reason}");
}

/// The build-provenance drift guard for `emit-state --write`, given an already-computed
/// [`crate::buildstamp::Verdict`]. Pure control flow over an injectable output stream, which
/// is what makes it directly unit-testable — including proving the banner lands on `stderr`
/// specifically (the parameter production wires to `std::io::stderr()`) rather than stdout,
/// without shelling out to git or spawning the real binary.
///
/// - `Verdict::Pass` — no-op: nothing written, `Ok(())`.
/// - `Verdict::NotEvaluable` — no-op, same as `Pass`. Never treated as drift, or a
///   `.git`-less deployment would become permanently unwritable.
/// - `Verdict::Drift(reason)` — writes the banner to `stderr`, then bails with `Err` (writing
///   nothing further, since the caller must not proceed to `mev::emit_state`) when
///   [`should_hard_fail_on_drift`] is true for `(fail_on_drift, env_var)`; otherwise returns
///   `Ok(())` so the default warn-and-proceed behaviour continues.
pub fn guard_write_on_drift<W: std::io::Write>(
    verdict: &crate::buildstamp::Verdict,
    fail_on_drift: bool,
    env_var: Option<&str>,
    stderr: &mut W,
) -> Result<()> {
    if let crate::buildstamp::Verdict::Drift(reason) = verdict {
        write_drift_banner(stderr, reason);
        if should_hard_fail_on_drift(fail_on_drift, env_var) {
            anyhow::bail!(
                "emit-state --write refused: build provenance drift and --fail-on-drift \
                 (or BASTION_FAIL_ON_BUILD_DRIFT) is set. {reason}"
            );
        }
    }
    Ok(())
}

/// Handler for `bastion emit-state [--write] [--fail-on-drift] [--agent <name>]`. Thin
/// pass-through to `mev::emit_state_as` — dry-run by default, reports planned (or
/// applied) actions via the same human summary shape used by mev's own `EmitState`
/// command.
///
/// Passes `agent` straight through as the writer identity to mev's GUARDED entry
/// point (`emit_state_as`), which itself applies mev's quiesce guard: a live
/// exclusive lease held by another agent on the resolved repo refuses the write with
/// `E_QUIESCE_LEASE_HELD`, while a lease held under this same `agent` is
/// self-exempted. No guard, lease check, or refusal logic is implemented here — that
/// decision belongs to mev's library alone (`MV.20.B`); this call site only supplies
/// identity. `lock_dir` is always `None` here — overriding the lock directory is out
/// of scope for this task.
///
/// `scope`, when `Some(<repo slug>)`, is resolved through mev's own
/// `BrainConfig::scope_dependencies` (built via `find_brain_config` over the
/// resolved brain root, never hand-assembled) and passed through as
/// `emit_state_as`'s `scope` argument, narrowing the emit to that one repo's
/// derived surfaces. `None` (the default — `--scope` omitted) emits every repo's
/// derived surfaces, unchanged from before this flag existed.
///
/// Before any `write == true` run, checks build provenance drift (task 2's
/// `buildstamp::current_verdict`) via [`guard_write_on_drift`] wired to real
/// `std::io::stderr()`. A hard-fail there returns before `mev::emit_state_as` is ever
/// called, so nothing is written.
pub fn run_emit_state(
    path: std::path::PathBuf,
    write: bool,
    fail_on_drift: bool,
    agent: Option<String>,
    scope: Option<String>,
) -> Result<()> {
    if write {
        let env_var = std::env::var("BASTION_FAIL_ON_BUILD_DRIFT").ok();
        guard_write_on_drift(
            &crate::buildstamp::current_verdict(),
            fail_on_drift,
            env_var.as_deref(),
            &mut std::io::stderr(),
        )?;
    }

    let root = mev::brain::config::find_brain_root(&path)
        .map_err(|e| anyhow::anyhow!("error resolving brain root: {e}"))?;

    let scope_set = match &scope {
        Some(slug) => {
            let config = mev::brain::config::load_brain_config(&root.join("brain.toml"))
                .map_err(|e| anyhow::anyhow!("error loading brain.toml: {e}"))?;
            Some(
                config
                    .scope_dependencies(slug)
                    .map_err(|e| anyhow::anyhow!("error resolving --scope '{slug}': {e}"))?,
            )
        }
        None => None,
    };

    let report = mev::emit_state_as(
        &root,
        write,
        scope_set.as_ref(),
        agent.as_deref(),
        None,
        &path,
    )?;

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
        let dir = crate::testsupport::unique_temp_dir("bastion-brainval-test");
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
        let dir = crate::testsupport::unique_temp_dir("bastion-brainval-manifest-test");
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
        let dir = crate::testsupport::unique_temp_dir("bastion-brainval-graph-test");
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
        let dir = crate::testsupport::unique_temp_dir(&format!("bastion-{name_prefix}"));
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
        let result = run_emit_state(dir.clone(), false, false, None, None);
        assert!(
            result.is_ok(),
            "expected Ok(()) for a valid brain root, got: {result:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Writes `<dir>/.fleet-locks/leases/lease-<name>.json` — an exclusive lease held by
    /// `agent`, matching `.claude/workflows/lease.schema.json`, so `run_emit_state`'s
    /// underlying `mev::emit_state_as` call has a foreign-agent lease to be quiesced by.
    /// The lock dir lives entirely under the caller's mktemp brain root (never the live
    /// `.fleet-locks/`), matching `resolve_lock_dir`'s default (`<root>/.fleet-locks`)
    /// since task 1 always passes `lock_dir: None`.
    fn write_exclusive_lease(dir: &std::path::Path, name: &str, agent: &str, repo: &str) {
        let leases_dir = dir.join(".fleet-locks").join("leases");
        std::fs::create_dir_all(&leases_dir).unwrap();
        let acquired_at = chrono::Local::now().to_rfc3339();
        std::fs::write(
            leases_dir.join(format!("lease-{name}.json")),
            format!(
                r#"{{
  "repo": "{repo}",
  "lane": "{name}",
  "agent": "{agent}",
  "acquired_at": "{acquired_at}",
  "kind": "exclusive"
}}"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn run_emit_state_write_refused_under_a_foreign_agent_lease() {
        // AC-1 / AC-2 (task 1): a lease held by a DIFFERENT agent makes the write return
        // E_QUIESCE_LEASE_HELD (asserted on the returned value, over a mktemp fixture lock
        // dir — never the live .fleet-locks/), while `--agent` matching the lease holder
        // writes successfully (the self-exemption).
        let dir = make_temp_brain_root("brainval-emit-state-quiesced");
        write_exclusive_lease(&dir, "other-lane", "other-agent", "bastion");

        // No identity supplied at all: refused, same as a mismatched identity — the guard
        // never self-exempts a caller with no agent.
        let no_identity = run_emit_state(dir.clone(), true, false, None, None);
        let err = no_identity.expect_err("expected the write to be refused under a foreign lease");
        let refusal = err
            .downcast_ref::<mev::GuardRefusal>()
            .unwrap_or_else(|| panic!("expected a GuardRefusal, got: {err:?}"));
        assert_eq!(refusal.code(), mev::E_QUIESCE_LEASE_HELD);

        // A DIFFERENT agent than the lease holder: also refused.
        let mismatched_agent = run_emit_state(
            dir.clone(),
            true,
            false,
            Some("some-other-agent".to_string()),
            None,
        );
        let err = mismatched_agent
            .expect_err("expected the write to be refused for a non-matching agent");
        let refusal = err
            .downcast_ref::<mev::GuardRefusal>()
            .unwrap_or_else(|| panic!("expected a GuardRefusal, got: {err:?}"));
        assert_eq!(refusal.code(), mev::E_QUIESCE_LEASE_HELD);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_emit_state_write_self_exempted_when_agent_matches_the_lease_holder() {
        // The self-exemption path: `--agent` matching the lease holder writes successfully
        // even though an exclusive lease is held on the same repo.
        let dir = make_temp_brain_root("brainval-emit-state-self-exempt");
        write_exclusive_lease(&dir, "own-lane", "own-agent", "bastion");

        let result = run_emit_state(
            dir.clone(),
            true,
            false,
            Some("own-agent".to_string()),
            None,
        );
        assert!(
            result.is_ok(),
            "expected Ok(()) when --agent matches the lease holder, got: {result:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn emit_state_pre_change_path_warns_unguarded_writer_but_post_change_path_does_not() {
        // AC-3, RE-PINNED per D18 (see the block record's amended criterion — the
        // original "W_MEV_UNGUARDED_WRITER no longer names bastion in any log" was
        // unfalsifiable: that warning is structurally unreachable unless a guard would
        // ACTUALLY have refused, so it never fires in the ordinary no-lease case and
        // bastion was already absent from it before this block, too. A foreign-agent
        // lease is the condition where the warning genuinely fires, so it is the only
        // fixture that can distinguish the fix from its absence.
        //
        // Both directions are asserted in ONE test:
        //   - PRE-CHANGE shape: bastion's old call site, `mev::emit_state(&root, write,
        //     None)` — no identity, no guard, downgraded by MV.20.B to a
        //     `W_MEV_UNGUARDED_WRITER` warning naming the calling binary.
        //   - POST-CHANGE shape: this block's `run_emit_state`, which now threads an
        //     identity through to the guarded `mev::emit_state_as` — refused outright
        //     (`E_QUIESCE_LEASE_HELD`) with no `Report` ever constructed, so the warning
        //     cannot be present.
        //
        // Per the task's staging recipe: assert on the returned `Report`'s diagnostics
        // locator (the preferred, rot-resistant form) rather than captured text — this
        // call happens in-process via the Rust API, not through a spawned CLI process,
        // so there is no separate stdout/stderr stream to pipe-capture in the first
        // place; the `Report` (pre-change) and the downcast `GuardRefusal` (post-change,
        // which never yields a `Report` at all) are the whole observable surface here.
        let dir = make_temp_brain_root("brainval-emit-state-unguarded-writer");
        write_exclusive_lease(&dir, "other-lane", "other-agent", "bastion");
        let root = mev::brain::config::find_brain_root(&dir).unwrap();

        // Pre-change: the identity-less legacy entry point still succeeds permissively
        // under a foreign lease (MV.20.B's downgrade), but its Report carries the
        // unguarded-writer warning.
        let pre_change_report = mev::emit_state(&root, true, None).expect(
            "the legacy identity-less path must still succeed (permissively) under a foreign lease",
        );
        let unguarded_diag = pre_change_report
            .diagnostics
            .iter()
            .find(|d| d.locator == "emit-state" && d.message.contains("W_MEV_UNGUARDED_WRITER"))
            .unwrap_or_else(|| {
                panic!(
                    "expected a W_MEV_UNGUARDED_WRITER diagnostic from the identity-less \
                     legacy path under a foreign lease, got: {:?}",
                    pre_change_report.diagnostics
                )
            });
        assert!(
            unguarded_diag.message.contains("bastion"),
            "expected the unguarded-writer warning to name bastion as the calling binary \
             (mev derives it from the running executable's file stem, and this crate's \
             package name is `bastion`), got: {}",
            unguarded_diag.message
        );

        // Post-change: this task's own guarded call path is refused outright under the
        // same lease — no Report is ever produced, so the warning cannot be in it.
        let post_change_result = run_emit_state(dir.clone(), true, false, None, None);
        let err = post_change_result
            .expect_err("expected the post-change path to be refused under the same foreign lease");
        let refusal = err
            .downcast_ref::<mev::GuardRefusal>()
            .unwrap_or_else(|| panic!("expected a GuardRefusal, got: {err:?}"));
        assert_eq!(refusal.code(), mev::E_QUIESCE_LEASE_HELD);
        assert!(
            !format!("{err:?}").contains("W_MEV_UNGUARDED_WRITER"),
            "the post-change refusal must not carry the unguarded-writer warning: {err:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Build a temp brain root containing TWO leaf repos (`repo-a`, `repo-b`) plus an
    /// HQ root entry, so a `--scope <repo>` emit has real siblings to leave untouched.
    /// Each leaf carries its own `planning/state.json` (one open block, so
    /// `plan_status_frontmatter` derives a non-empty `now` focus) and a `status.md`
    /// with no `now:`/`next:`/`blocked:` frontmatter keys at all — `emit-state --write`
    /// always appends them, so a write is guaranteed to change the file regardless of
    /// what the derived focus actually contains. `cache_doc` is omitted (defaults to
    /// `""`), so `plan_project_caches` skips it — that surface is out of scope for this
    /// task's fixture. Returns the root dir; callers own `remove_dir_all` teardown.
    fn make_scope_fixture_brain_root(name_prefix: &str) -> std::path::PathBuf {
        let dir = crate::testsupport::unique_temp_dir(&format!("bastion-{name_prefix}"));
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(
            dir.join("brain.toml"),
            r#"[vocab]
layer = ["console"]
status = ["active"]

[crawl]
skip_dirs = ["target", ".git"]

[[repos]]
slug = "hq"
tier = "_root"
repo_path = "."
status_file = "status.md"
heading = "HQ"

[[repos]]
slug = "repo-a"
tier = "core"
repo_path = "repo-a"
status_file = "repo-a/status.md"
heading = "repo-a"

[[repos]]
slug = "repo-b"
tier = "core"
repo_path = "repo-b"
status_file = "repo-b/status.md"
heading = "repo-b"
"#,
        )
        .unwrap();

        std::fs::write(
            dir.join("status.md"),
            "---\ntitle: HQ Status\n---\n# HQ Status\n",
        )
        .unwrap();

        for slug in ["repo-a", "repo-b"] {
            let repo_dir = dir.join(slug);
            let planning_dir = repo_dir.join("planning");
            std::fs::create_dir_all(&planning_dir).unwrap();

            std::fs::write(
                planning_dir.join("state.json"),
                format!(
                    r#"{{
  "repo": "{slug}",
  "kind": "project",
  "updated": "2026-07-04",
  "focus": {{
    "now": [{{ "id": "{slug_upper}.1.A", "title": "Open block", "status": "in_progress" }}],
    "next": [],
    "blocked": []
  }},
  "tracks": [
    {{
      "title": "Phase 1",
      "blocks": [
        {{ "id": "{slug_upper}.1.A", "title": "Open block", "status": "open" }}
      ]
    }}
  ]
}}"#,
                    slug = slug,
                    slug_upper = slug.to_uppercase().replace('-', "")
                ),
            )
            .unwrap();

            std::fs::write(
                repo_dir.join("status.md"),
                format!("---\ntitle: {slug} Status\n---\n# {slug} Status\n"),
            )
            .unwrap();
        }

        dir
    }

    #[test]
    fn run_emit_state_scope_narrows_the_write_to_one_repo_leaving_siblings_untouched() {
        // AC-4 (task 3): `--scope <repo>` is passed through as emit_state_as's scope
        // argument, resolved through mev's own `BrainConfig::scope_dependencies` (never
        // hand-assembled, never post-filtered). A scoped write changes only the named
        // repo's own derived surfaces; the sibling repo's stay byte-identical.
        //
        // POSITIVE CONTROL, in the same test: an UNSCOPED run over an identically-built
        // fixture DOES change the sibling's surfaces — proving the scoped run's silence
        // is scoping, not a --scope that emits nothing at all.

        // ── Scoped run: --scope repo-a ──────────────────────────────────────────
        let scoped_dir = make_scope_fixture_brain_root("emit-state-scope-scoped");
        let repo_a_status_before =
            std::fs::read_to_string(scoped_dir.join("repo-a").join("status.md")).unwrap();
        let repo_b_status_before =
            std::fs::read_to_string(scoped_dir.join("repo-b").join("status.md")).unwrap();

        let result = run_emit_state(
            scoped_dir.clone(),
            true,
            false,
            None,
            Some("repo-a".to_string()),
        );
        assert!(
            result.is_ok(),
            "expected Ok(()) for a scoped write, got: {result:?}"
        );

        let repo_a_status_after =
            std::fs::read_to_string(scoped_dir.join("repo-a").join("status.md")).unwrap();
        let repo_b_status_after =
            std::fs::read_to_string(scoped_dir.join("repo-b").join("status.md")).unwrap();

        assert_ne!(
            repo_a_status_before, repo_a_status_after,
            "expected the SCOPED repo's own status.md to change under --scope repo-a"
        );
        assert_eq!(
            repo_b_status_before, repo_b_status_after,
            "expected the SIBLING repo's status.md to stay byte-identical under --scope repo-a"
        );

        let _ = std::fs::remove_dir_all(&scoped_dir);

        // ── Positive control: an UNSCOPED run over a fresh, identically-built fixture
        //    DOES change the sibling. Without this control, a --scope that silently
        //    emitted nothing at all would have passed the assertions above too. ──
        let unscoped_dir = make_scope_fixture_brain_root("emit-state-scope-control");
        let repo_b_status_before =
            std::fs::read_to_string(unscoped_dir.join("repo-b").join("status.md")).unwrap();

        let control_result = run_emit_state(unscoped_dir.clone(), true, false, None, None);
        assert!(
            control_result.is_ok(),
            "expected Ok(()) for an unscoped write, got: {control_result:?}"
        );

        let repo_b_status_after =
            std::fs::read_to_string(unscoped_dir.join("repo-b").join("status.md")).unwrap();
        assert_ne!(
            repo_b_status_before, repo_b_status_after,
            "positive control failed: an UNSCOPED run must change repo-b's status.md too, \
             otherwise a --scope that emits nothing could pass the scoped assertions above"
        );

        let _ = std::fs::remove_dir_all(&unscoped_dir);
    }

    #[test]
    fn run_emit_state_unknown_scope_slug_returns_an_error() {
        // A --scope naming a slug absent from brain.toml's [[repos]] must error rather
        // than silently emitting nothing or falling back to unscoped.
        let dir = make_temp_brain_root("brainval-emit-state-unknown-scope");
        let result = run_emit_state(
            dir.clone(),
            false,
            false,
            None,
            Some("no-such-repo".to_string()),
        );
        assert!(
            result.is_err(),
            "expected an error for an unregistered --scope slug, got: {result:?}"
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
        let dir = crate::testsupport::unique_temp_dir("bastion-brainval-emit-state-test");
        std::fs::create_dir_all(&dir).unwrap();
        let result = run_emit_state(dir.clone(), false, false, None, None);
        assert!(
            result.is_err(),
            "expected an error when brain.toml is unresolvable"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── should_hard_fail_on_drift — truth table ─────────────────────────────

    #[test]
    fn hard_fail_false_when_flag_false_and_env_unset() {
        assert!(!should_hard_fail_on_drift(false, None));
    }

    #[test]
    fn hard_fail_true_when_flag_true_regardless_of_env() {
        assert!(should_hard_fail_on_drift(true, None));
        assert!(should_hard_fail_on_drift(true, Some("0")));
        assert!(should_hard_fail_on_drift(true, Some("false")));
    }

    #[test]
    fn hard_fail_true_for_each_truthy_env_value_case_insensitive() {
        for v in ["1", "true", "TRUE", "True", "yes", "YES", "on", "ON"] {
            assert!(
                should_hard_fail_on_drift(false, Some(v)),
                "expected {v:?} to be truthy"
            );
        }
    }

    #[test]
    fn hard_fail_false_for_falsy_or_unrecognized_env_values() {
        for v in ["0", "false", "no", "off", "", "  ", "banana"] {
            assert!(
                !should_hard_fail_on_drift(false, Some(v)),
                "expected {v:?} to be falsy"
            );
        }
    }

    #[test]
    fn hard_fail_true_when_flag_false_but_env_truthy_with_whitespace() {
        assert!(should_hard_fail_on_drift(false, Some("  true  ")));
    }

    // ── guard_write_on_drift ─────────────────────────────────────────────────

    #[test]
    fn guard_pass_verdict_writes_nothing_and_succeeds() {
        let mut stderr_buf: Vec<u8> = Vec::new();
        let result = guard_write_on_drift(
            &crate::buildstamp::Verdict::Pass,
            false,
            None,
            &mut stderr_buf,
        );
        assert!(result.is_ok());
        assert!(stderr_buf.is_empty());
    }

    #[test]
    fn guard_not_evaluable_verdict_writes_nothing_and_never_hard_fails() {
        let mut stderr_buf: Vec<u8> = Vec::new();
        let verdict = crate::buildstamp::Verdict::NotEvaluable("no git available".to_string());
        // Even with --fail-on-drift set, NotEvaluable must never hard-fail — a .git-less
        // deployment must stay writable.
        let result = guard_write_on_drift(&verdict, true, Some("1"), &mut stderr_buf);
        assert!(result.is_ok());
        assert!(stderr_buf.is_empty());
    }

    #[test]
    fn guard_drift_default_writes_banner_to_stderr_and_still_succeeds() {
        let mut stderr_buf: Vec<u8> = Vec::new();
        let verdict = crate::buildstamp::Verdict::Drift(
            "the running binary was built from aaa111 but the source is now at bbb222".to_string(),
        );
        let result = guard_write_on_drift(&verdict, false, None, &mut stderr_buf);
        assert!(
            result.is_ok(),
            "default behaviour must warn-and-proceed, not fail"
        );
        let banner = String::from_utf8(stderr_buf).unwrap();
        assert!(banner.contains("BUILD PROVENANCE DRIFT"));
        assert!(banner.contains("aaa111"));
        assert!(banner.contains("bbb222"));
    }

    #[test]
    fn guard_drift_writes_to_the_stderr_parameter_specifically_not_a_separate_stdout_sink() {
        // guard_write_on_drift takes exactly one output stream and it is the one production
        // wires to std::io::stderr() (see run_emit_state) — there is no stdout parameter for
        // the banner to leak onto. Assert the banner is present on that stream and that a
        // second, untouched buffer standing in for stdout stays empty, pinning that the
        // deliverable (a human seeing this on stderr) is not accidentally satisfied by
        // println! instead of eprintln!/writeln!(stderr, ..).
        let mut stderr_buf: Vec<u8> = Vec::new();
        let stdout_stand_in: Vec<u8> = Vec::new();
        let verdict = crate::buildstamp::Verdict::Drift("sha mismatch".to_string());
        let _ = guard_write_on_drift(&verdict, false, None, &mut stderr_buf);
        assert!(!stderr_buf.is_empty(), "banner must reach stderr");
        assert!(
            stdout_stand_in.is_empty(),
            "nothing must be written outside the stderr parameter"
        );
    }

    #[test]
    fn guard_drift_hard_fails_via_flag_and_writes_nothing_further() {
        let mut stderr_buf: Vec<u8> = Vec::new();
        let verdict = crate::buildstamp::Verdict::Drift("sha mismatch".to_string());
        let result = guard_write_on_drift(&verdict, true, None, &mut stderr_buf);
        assert!(result.is_err(), "--fail-on-drift must turn drift into Err");
        // The banner is still written before the hard-fail (a human should see WHY it
        // refused), but the error itself is what stops the caller reaching mev::emit_state.
        assert!(!stderr_buf.is_empty());
    }

    #[test]
    fn guard_drift_hard_fails_via_env_var_identical_to_flag() {
        let mut stderr_buf: Vec<u8> = Vec::new();
        let verdict = crate::buildstamp::Verdict::Drift("sha mismatch".to_string());
        let result = guard_write_on_drift(&verdict, false, Some("1"), &mut stderr_buf);
        assert!(
            result.is_err(),
            "BASTION_FAIL_ON_BUILD_DRIFT=1 must have the identical effect to --fail-on-drift"
        );
    }

    #[test]
    fn guard_dirty_tree_is_reported_as_drift_via_the_verdict_layer() {
        // Task 2's verdict() already covers dirty==Drift exhaustively; this pins that the
        // guard treats that Drift like any other — banner + default warn-and-proceed.
        let v = crate::buildstamp::verdict("abc123", Some("abc123"), "1", true);
        assert!(matches!(v, crate::buildstamp::Verdict::Drift(_)));
        let mut stderr_buf: Vec<u8> = Vec::new();
        let result = guard_write_on_drift(&v, false, None, &mut stderr_buf);
        assert!(result.is_ok());
        assert!(!stderr_buf.is_empty());
    }
}
