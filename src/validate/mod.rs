// `bastion validate <path>` — markdown/MDX content validation.

pub mod frontmatter;
pub mod links;
pub mod report;

use anyhow::Result;
use std::path::{Path, PathBuf};

// ── Shared types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub file: PathBuf,
    pub line: usize,
    pub kind: ErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    MissingFrontmatter,
    MalformedFrontmatter,
    MissingField,
    EmptyField,
    BrokenLink,
}

impl ErrorKind {
    /// Stable lowercase label for greppable output.
    pub fn label(&self) -> &str {
        match self {
            ErrorKind::MissingFrontmatter => "missing-frontmatter",
            ErrorKind::MalformedFrontmatter => "malformed-frontmatter",
            ErrorKind::MissingField => "missing-field",
            ErrorKind::EmptyField => "empty-field",
            ErrorKind::BrokenLink => "broken-link",
        }
    }
}

// ── File discovery ────────────────────────────────────────────────────────────

/// Recursively collect `.md` and `.mdx` files under `root`.
///
/// Rules:
/// - Skips hidden directories/files (leading `.`).
/// - Skips the `target/` directory.
/// - If `root` itself is a `.md`/`.mdx` file, returns just that file.
/// - Returns a deterministically sorted list (lexicographic by full path).
pub fn find_markdown_files(root: &Path) -> Vec<PathBuf> {
    // Single-file shortcut.
    if root.is_file() {
        if is_markdown(root) {
            return vec![root.to_path_buf()];
        }
        return vec![];
    }

    let mut results: Vec<PathBuf> = Vec::new();
    collect_markdown(root, &mut results);
    results.sort();
    results
}

fn is_markdown(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("md") | Some("mdx")
    )
}

fn collect_markdown(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();

        // Skip hidden entries (leading `.`) and the build artefact directory.
        if file_name.starts_with('.') || file_name == "target" {
            continue;
        }

        if path.is_dir() {
            collect_markdown(&path, out);
        } else if is_markdown(&path) {
            out.push(path);
        }
    }
}

// ── I/O shell ─────────────────────────────────────────────────────────────────

/// Entry point called by `main.rs`. Synchronous filesystem work; the `async`
/// signature matches the dispatch site but no `.await` is used inside.
pub async fn run(path: PathBuf) -> Result<()> {
    let files = find_markdown_files(&path);
    let files_scanned = files.len();
    let mut all_errors: Vec<ValidationError> = Vec::new();

    for file in &files {
        let content = std::fs::read_to_string(file)?;
        let mut errs = frontmatter::validate_frontmatter(&content, file);
        errs.extend(links::validate_links(&content, file));
        all_errors.extend(errs);
    }

    let report = report::render_report(&all_errors, files_scanned);
    println!("{report}");

    if !all_errors.is_empty() {
        anyhow::bail!("{} error(s) found", all_errors.len());
    }
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Minimal temp-dir helper that cleans up on drop (avoids adding `tempfile` dep).
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            let dir = std::env::temp_dir().join(format!("bastion_validate_test_{}_{}", pid, id));
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    // ── ErrorKind label mapping ───────────────────────────────────────────────

    #[test]
    fn error_kind_labels_all_variants() {
        assert_eq!(ErrorKind::MissingFrontmatter.label(), "missing-frontmatter");
        assert_eq!(
            ErrorKind::MalformedFrontmatter.label(),
            "malformed-frontmatter"
        );
        assert_eq!(ErrorKind::MissingField.label(), "missing-field");
        assert_eq!(ErrorKind::EmptyField.label(), "empty-field");
        assert_eq!(ErrorKind::BrokenLink.label(), "broken-link");
    }

    // ── find_markdown_files ───────────────────────────────────────────────────

    fn make_file(dir: &Path, rel: &str) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&p, "content").unwrap();
    }

    #[test]
    fn single_md_file_returns_that_file() {
        let tmp = TempDir::new();
        let f = tmp.path().join("hello.md");
        fs::write(&f, "").unwrap();
        assert_eq!(find_markdown_files(&f), vec![f]);
    }

    #[test]
    fn single_mdx_file_returns_that_file() {
        let tmp = TempDir::new();
        let f = tmp.path().join("page.mdx");
        fs::write(&f, "").unwrap();
        assert_eq!(find_markdown_files(&f), vec![f]);
    }

    #[test]
    fn single_non_markdown_file_returns_empty() {
        let tmp = TempDir::new();
        let f = tmp.path().join("note.txt");
        fs::write(&f, "").unwrap();
        assert_eq!(find_markdown_files(&f), Vec::<PathBuf>::new());
    }

    #[test]
    fn empty_directory_returns_empty() {
        let tmp = TempDir::new();
        assert_eq!(find_markdown_files(tmp.path()), Vec::<PathBuf>::new());
    }

    #[test]
    fn collects_md_and_mdx_extensions() {
        let tmp = TempDir::new();
        let md = tmp.path().join("a.md");
        let mdx = tmp.path().join("b.mdx");
        let txt = tmp.path().join("c.txt");
        fs::write(&md, "").unwrap();
        fs::write(&mdx, "").unwrap();
        fs::write(&txt, "").unwrap();
        let result = find_markdown_files(tmp.path());
        assert_eq!(result.len(), 2);
        assert!(result.contains(&md));
        assert!(result.contains(&mdx));
    }

    #[test]
    fn recursion_into_subdirs() {
        let tmp = TempDir::new();
        make_file(tmp.path(), "top.md");
        make_file(tmp.path(), "sub/nested.md");
        make_file(tmp.path(), "sub/deep/deeper.mdx");
        let result = find_markdown_files(tmp.path());
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn skips_hidden_directories() {
        let tmp = TempDir::new();
        make_file(tmp.path(), "visible.md");
        make_file(tmp.path(), ".hidden/secret.md");
        let result = find_markdown_files(tmp.path());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].file_name().unwrap(), "visible.md");
    }

    #[test]
    fn skips_hidden_files() {
        let tmp = TempDir::new();
        make_file(tmp.path(), "visible.md");
        make_file(tmp.path(), ".secret.md");
        let result = find_markdown_files(tmp.path());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].file_name().unwrap(), "visible.md");
    }

    #[test]
    fn skips_target_directory() {
        let tmp = TempDir::new();
        make_file(tmp.path(), "real.md");
        make_file(tmp.path(), "target/build.md");
        let result = find_markdown_files(tmp.path());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].file_name().unwrap(), "real.md");
    }

    #[test]
    fn returns_sorted_order() {
        let tmp = TempDir::new();
        make_file(tmp.path(), "z.md");
        make_file(tmp.path(), "a.md");
        make_file(tmp.path(), "m.md");
        let result = find_markdown_files(tmp.path());
        assert_eq!(result.len(), 3);
        // Verify sorted.
        let mut sorted = result.clone();
        sorted.sort();
        assert_eq!(result, sorted);
    }

    #[test]
    fn sorting_is_deterministic_across_calls() {
        let tmp = TempDir::new();
        for name in ["c.md", "b.mdx", "a.md", "d.mdx"] {
            make_file(tmp.path(), name);
        }
        let first = find_markdown_files(tmp.path());
        let second = find_markdown_files(tmp.path());
        assert_eq!(first, second);
    }
}
