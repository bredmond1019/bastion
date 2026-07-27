//! CLI-facing shell for `bastion assess <path> [--json]` (Phase 15, Block BA.15.9, task 7).
//!
//! Thin I/O shell: resolve the context, read every discovered markdown file into
//! `(path, content)` pairs, run the three pure family functions, assemble the
//! [`super::AssessReport`], and print either the human summary or the `--json`
//! envelope. Read-only end to end — no file is created, written, or mutated anywhere
//! under the assessed path.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::AssessReport;
use super::discover::resolve_context;
use super::graph::assess_graph;
use super::okf::assess_okf;
use super::render::{render_human, render_json};
use super::state::assess_state;

/// Scope slug used to build graph node/leaf keys (`scope:doc_id` / `scope:stem`).
/// `assess` only ever looks at one corpus per run, unlike mev's multi-scope crawl,
/// so a single fixed slug is sufficient — it never leaves this process.
const ASSESS_SCOPE: &str = "assess";

/// Handler for `bastion assess <path> [--json]`. Read-only: reads `<path>`'s markdown
/// files and (if present) `planning/state.json`, and prints a report. Never writes,
/// creates, or deletes anything.
pub fn run(path: PathBuf, json: bool) -> Result<()> {
    let report = build_report(&path)?;
    let output = if json {
        render_json(&report)?
    } else {
        render_human(&report)
    };
    println!("{output}");
    Ok(())
}

/// Assemble an [`AssessReport`] for `path`: resolve context, read markdown files,
/// run the three family functions, and package the result. The only I/O here is
/// reading files that were already discovered read-only by `resolve_context`.
fn build_report(path: &Path) -> Result<AssessReport> {
    let ctx = resolve_context(path);

    let docs: Vec<(PathBuf, String)> = ctx
        .markdown
        .iter()
        .map(|p| {
            let content = std::fs::read_to_string(p)
                .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", p.display()))?;
            Ok((p.clone(), content))
        })
        .collect::<Result<Vec<_>>>()?;

    let okf = assess_okf(&docs);
    let graph = assess_graph(ASSESS_SCOPE, &docs);
    let state = assess_state(ctx.planning_root.as_deref());

    Ok(AssessReport {
        assessor: "assess".to_string(),
        root: ctx.root.display().to_string(),
        files_scanned: docs.len(),
        okf,
        graph,
        state,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(dir: &Path, rel: &str, contents: &str) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, contents).unwrap();
    }

    /// Recursively snapshot every file's relative path + contents under `root`,
    /// sorted for a stable comparison.
    fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        fn walk(dir: &Path, root: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
            let mut entries: Vec<_> = fs::read_dir(dir)
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect();
            entries.sort();
            for entry in entries {
                if entry.is_dir() {
                    walk(&entry, root, out);
                } else {
                    let rel = entry.strip_prefix(root).unwrap().to_path_buf();
                    let bytes = fs::read(&entry).unwrap();
                    out.push((rel, bytes));
                }
            }
        }
        let mut out = Vec::new();
        walk(root, root, &mut out);
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    #[test]
    fn build_report_over_a_repo_with_no_frontmatter_reports_the_gap() {
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            "bare.md",
            "just some text, no frontmatter at all",
        );

        let report = build_report(tmp.path()).expect("build_report should succeed");

        assert_eq!(report.files_scanned, 1);
        assert_eq!(report.okf.invalid_frontmatter_count, 1);
    }

    #[test]
    fn assess_pipeline_performs_zero_filesystem_writes() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "brain.toml", "");
        write(
            tmp.path(),
            "planning/state.json",
            r#"{"focus": {}, "tracks": []}"#,
        );
        write(
            tmp.path(),
            "docs/a.md",
            "---\ntype: Doc\ntitle: A\ndescription: d\n---\nbody",
        );
        write(tmp.path(), "docs/b.md", "no frontmatter here");

        let before = snapshot(tmp.path());

        // Exercise both the human and JSON paths — neither should write anything.
        let report = build_report(tmp.path()).expect("build_report should succeed");
        let _ = render_human(&report);
        let _ = render_json(&report).expect("render_json should succeed");

        let after = snapshot(tmp.path());

        assert_eq!(
            before, after,
            "assess must perform zero filesystem writes, but the tree changed"
        );
    }

    #[test]
    fn build_report_names_the_resolved_root() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "docs/a.md", "content");

        let report = build_report(tmp.path()).expect("build_report should succeed");

        assert_eq!(report.root, tmp.path().display().to_string());
    }
}
