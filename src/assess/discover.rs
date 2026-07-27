//! Context resolution for `bastion assess <path>` (Phase 15, Block BA.15.9, task 2).
//!
//! Splits resolution logic (pure, over already-resolved `Option`s) from the thin
//! filesystem shell that actually walks the tree. `assess` takes an explicit `path`
//! rather than resolving from cwd, so it uses `config::walk_up_from` directly instead
//! of the cwd-anchored `walk_up_for`.

use std::path::{Path, PathBuf};

use crate::config::walk_up_from;
use crate::validate::find_markdown_files;

/// Everything `bastion assess` needs to run its check families over a resolved repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssessContext {
    /// The path `assess` was pointed at (not necessarily a repo root — just the anchor
    /// both `walk_up_from` and markdown discovery start from).
    pub root: PathBuf,
    /// `brain.toml`, if found walking up from `root`.
    pub brain_toml: Option<PathBuf>,
    /// `planning/`, if found walking up from `root`.
    pub planning_root: Option<PathBuf>,
    /// Every `.md`/`.mdx` file discovered under `root`.
    pub markdown: Vec<PathBuf>,
}

/// Pure assembly of an [`AssessContext`] from already-resolved pieces — no filesystem
/// access happens in this function. Kept separate from [`resolve_context`] so the
/// classification logic itself is unit-testable without a tempfile fixture.
fn classify_context(
    root: &Path,
    brain_toml: Option<PathBuf>,
    planning_root: Option<PathBuf>,
    markdown: Vec<PathBuf>,
) -> AssessContext {
    AssessContext {
        root: root.to_path_buf(),
        brain_toml,
        planning_root,
        markdown,
    }
}

/// Thin I/O shell: locate `brain.toml` and `planning/` by walking up from `path`
/// (reusing task 1's `config::walk_up_from`), and discover markdown files under `path`
/// (reusing `validate::find_markdown_files`). Never panics — an unresolved `brain.toml`
/// or `planning/` simply comes back `None`; that absence is itself diagnostic signal
/// for the state-readiness family (task 5).
pub fn resolve_context(path: &Path) -> AssessContext {
    let brain_toml = walk_up_from(path, "brain.toml");
    let planning_root = walk_up_from(path, "planning");
    let markdown = find_markdown_files(path);
    classify_context(path, brain_toml, planning_root, markdown)
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

    #[test]
    fn finds_brain_toml_and_planning_when_present() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "brain.toml", "");
        write(tmp.path(), "planning/status.md", "");
        write(tmp.path(), "docs/a.md", "content");

        let ctx = resolve_context(tmp.path());

        assert_eq!(ctx.root, tmp.path());
        assert_eq!(ctx.brain_toml, Some(tmp.path().join("brain.toml")));
        assert_eq!(ctx.planning_root, Some(tmp.path().join("planning")));
        assert_eq!(
            ctx.markdown,
            vec![
                tmp.path().join("docs/a.md"),
                tmp.path().join("planning/status.md")
            ]
        );
    }

    #[test]
    fn reports_none_when_brain_toml_and_planning_absent() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "docs/a.md", "content");

        let ctx = resolve_context(tmp.path());

        assert_eq!(ctx.brain_toml, None);
        assert_eq!(ctx.planning_root, None);
        assert_eq!(ctx.markdown, vec![tmp.path().join("docs/a.md")]);
    }

    #[test]
    fn finds_brain_toml_several_levels_up_but_root_stays_the_pointed_at_path() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "brain.toml", "");
        let nested = tmp.path().join("a/b/c");
        fs::create_dir_all(&nested).unwrap();
        write(&nested, "notes.md", "content");

        let ctx = resolve_context(&nested);

        assert_eq!(ctx.root, nested);
        assert_eq!(ctx.brain_toml, Some(tmp.path().join("brain.toml")));
        assert_eq!(ctx.markdown, vec![nested.join("notes.md")]);
    }

    #[test]
    fn discovers_no_markdown_over_an_empty_directory() {
        let tmp = TempDir::new().unwrap();

        let ctx = resolve_context(tmp.path());

        assert!(ctx.markdown.is_empty());
    }
}
