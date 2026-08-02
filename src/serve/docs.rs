//! Pure path validator + allowlist resolution for the `bastion serve` docs read
//! routes (`GET /api/docs/{repo}/tree`, `GET /api/docs/{repo}/file`) — BA.11.Q.
//!
//! This is the block's primary deliverable. There is no existing traversal
//! guard anywhere in the codebase to copy (`grep -rn canonicalize src/`
//! returns nothing before this file) — every read shipped so far has been a
//! fixed path (`planning/status.md`, `planning/handoff.md`). This module
//! establishes the pattern the handlers in `handlers/docs.rs` (Task 3) sit on
//! top of.
//!
//! # Two-phase guard
//!
//! 1. [`validate_rel_path`] — a PURE pre-check with no filesystem access.
//!    Rejects absolute paths, `..` components, root/prefix components,
//!    embedded NULs, backslashes, empty paths, and (when `require_md` is
//!    set) a non-`.md`/`.mdx` extension. This is what lets the handler
//!    short-circuit obviously-malicious input before touching disk.
//! 2. [`resolve_allowlisted_path`] — the I/O-touching containment check that
//!    completes the guard: canonicalize the candidate *and* the allowlisted
//!    root, then require the former to start with the latter. This is what
//!    catches a symlink *inside* an allowlisted directory that points
//!    outside it — the pure check alone cannot see that.
//!
//! **Critical detail:** each allowlisted root is canonicalized independently
//! as `repo_root.join(dir)`, never as a subpath of a canonicalized repo
//! root. `planning/` in every repo in this fleet is a symlink into the
//! company-brain vault (`core/_planning/<repo>/`), so canonicalizing the
//! repo root first and then checking containment against *that* would
//! reject every legitimate `planning/` read.
//!
//! # Error mapping (Task 3 owns the HTTP translation)
//! [`DocsError::Forbidden`] is deliberately the single variant for
//! traversal, bad extension, and "outside every allowlisted root" — the
//! handler must map all three to the same `403`/`C003` response so it never
//! discloses whether an out-of-allowlist path exists.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use thiserror::Error;

use crate::serve::dto::DocEntryDto;

// ── Allowlist ────────────────────────────────────────────────────────────────

/// Repo-root-relative directories readable via `GET /api/docs/*`.
///
/// Repo-root-level `*.md`/`.mdx` files (e.g. `README.md`, `CLAUDE.md`) are
/// also readable — see [`is_repo_root_markdown`]. This list is deliberately
/// not configurable (out of scope for BA.11.Q).
pub const ALLOWED_ROOTS: [&str; 3] = ["docs", "planning", "business"];

// ── Errors ───────────────────────────────────────────────────────────────────

/// Errors the pure/allowlist core can raise. `handlers/docs.rs` (Task 3) maps
/// each to an HTTP status + `C0xx` code.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DocsError {
    /// Traversal attempt, disallowed extension, or a path outside every
    /// allowlisted root — one uniform variant for all three so the HTTP
    /// response never discloses which of the three actually happened.
    #[error("path is outside every allowlisted root")]
    Forbidden,
    /// The path validated and resolved inside an allowlisted root, but no
    /// file exists there.
    #[error("file not found")]
    NotFound,
    /// The file exists but its content is not valid UTF-8.
    #[error("file content is not valid UTF-8")]
    NotUtf8,
    /// Any other filesystem failure (permissions, transient I/O, etc.).
    #[error("I/O error: {0}")]
    Io(String),
}

// ── Pure core: path validation ──────────────────────────────────────────────

/// Validate a client-supplied, repo-root-relative path with **no filesystem
/// access**.
///
/// Rejects:
/// - an absolute path, or any [`Component::RootDir`]/[`Component::Prefix`]
///   component (catches `/etc/passwd` and Windows-style `C:\…` roots);
/// - any [`Component::ParentDir`] (`..`) component, anywhere in the path;
/// - an embedded NUL byte;
/// - a backslash (Windows-style separators are never valid here, and this
///   also catches `C:\Windows\x.md`-style paths that a Unix `Path` parser
///   would otherwise treat as one opaque component);
/// - an empty or whitespace-only path;
/// - when `require_md` is `true`, any extension other than `.md`/`.mdx`
///   (case-insensitive).
///
/// On success, returns the validated path as a [`PathBuf`] — still
/// repo-root-relative and not yet checked against the allowlist or the
/// filesystem; that is [`resolve_allowlisted_path`]'s job.
pub fn validate_rel_path(path: &str, require_md: bool) -> Result<PathBuf, DocsError> {
    if path.trim().is_empty() {
        return Err(DocsError::Forbidden);
    }
    if path.contains('\0') || path.contains('\\') {
        return Err(DocsError::Forbidden);
    }

    let candidate = Path::new(path);
    for component in candidate.components() {
        match component {
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(DocsError::Forbidden);
            }
            Component::CurDir | Component::Normal(_) => {}
        }
    }

    if require_md {
        let ext_ok = candidate
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("mdx"))
            .unwrap_or(false);
        if !ext_ok {
            return Err(DocsError::Forbidden);
        }
    }

    Ok(candidate.to_path_buf())
}

// ── I/O shell: allowlist containment ────────────────────────────────────────

/// Return `true` when `rel` is a single-component path that is not itself
/// one of [`ALLOWED_ROOTS`] — i.e. a candidate repo-root-level file such as
/// `README.md` or `CLAUDE.md`.
fn is_repo_root_markdown(rel: &Path) -> bool {
    rel.components().count() == 1 && !ALLOWED_ROOTS.iter().any(|root| rel == Path::new(root))
}

/// Resolve a validated, repo-root-relative path against the allowlist,
/// canonicalizing to defeat symlink escapes.
///
/// `rel` must already have passed [`validate_rel_path`] — this function does
/// not re-check for `..`/absolute components, it only adds the
/// filesystem-touching containment step: canonicalize the candidate *and*
/// the allowlisted root it falls under, then require the former to start
/// with the latter.
///
/// Each allowlisted root is canonicalized as `repo_root.join(dir)`,
/// independently of the repo root itself — this is deliberate (see the
/// module doc's "Critical detail") so a symlinked `planning/` resolves
/// correctly instead of being rejected.
pub fn resolve_allowlisted_path(repo_root: &Path, rel: &Path) -> Result<PathBuf, DocsError> {
    if is_repo_root_markdown(rel) {
        let canonical_root = repo_root
            .canonicalize()
            .map_err(|e| DocsError::Io(e.to_string()))?;
        let candidate = repo_root.join(rel);
        let canonical_target = match candidate.canonicalize() {
            Ok(p) => p,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(DocsError::NotFound),
            Err(e) => return Err(DocsError::Io(e.to_string())),
        };
        // `is_repo_root_markdown` only checked the path *shape* (a single
        // component not equal to an allowed root's name) — it cannot see
        // whether the target is actually a file. A single-component
        // directory name that happens not to collide with an allowed root
        // (e.g. `src`) would otherwise sail through here. Require it to be a
        // real file, matching the "repo-root-level `*.md` FILES" contract.
        if canonical_target.parent() == Some(canonical_root.as_path()) && canonical_target.is_file()
        {
            return Ok(canonical_target);
        }
        return Err(DocsError::Forbidden);
    }

    for root in ALLOWED_ROOTS {
        if !rel.starts_with(root) {
            continue;
        }

        let allowed_root = repo_root.join(root);
        let canonical_root = match allowed_root.canonicalize() {
            Ok(p) => p,
            // This allowlisted root doesn't exist in this repo — fall through
            // to the final `Forbidden` rather than leaking an I/O error.
            Err(_) => return Err(DocsError::Forbidden),
        };

        let candidate = repo_root.join(rel);
        let canonical_target = match candidate.canonicalize() {
            Ok(p) => p,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(DocsError::NotFound),
            Err(e) => return Err(DocsError::Io(e.to_string())),
        };

        return if canonical_target.starts_with(&canonical_root) {
            Ok(canonical_target)
        } else {
            Err(DocsError::Forbidden)
        };
    }

    Err(DocsError::Forbidden)
}

// ── Pure core: tree assembly ────────────────────────────────────────────────

/// Build a single-level, directories-first-then-by-name sorted list of
/// [`DocEntryDto`] for `root`, from `md_paths` — the full set of
/// repo-root-relative markdown paths under `root` (as produced by a
/// recursive crawl, e.g. `mev::brain::crawl::crawl_brain`).
///
/// `root` is `""` to list the whole allowlist's top level (each allowed
/// root plus any repo-root-level markdown files), or an allowlisted
/// subtree such as `"planning"` or `"planning/decisions"`.
///
/// Intermediate directories are implied from the paths themselves — a path
/// like `planning/decisions/d1.md` under `root = "planning"` contributes a
/// single `decisions` directory entry, not a `decisions/d1.md` file entry.
/// Because entries are derived only from paths that carry markdown
/// somewhere beneath them, a directory with no markdown at any depth never
/// appears — there is nothing to drop.
pub fn build_doc_tree(root: &str, md_paths: &[String]) -> Vec<DocEntryDto> {
    let mut seen: HashMap<String, DocEntryDto> = HashMap::new();

    for raw in md_paths {
        let normalized = raw.replace('\\', "/");
        let rel = match strip_root(&normalized, root) {
            Some(rel) if !rel.is_empty() => rel,
            _ => continue,
        };

        let mut parts = rel.splitn(2, '/');
        let first = match parts.next() {
            Some(p) if !p.is_empty() => p,
            _ => continue,
        };
        let is_dir = parts.next().is_some();

        let entry_path = if root.is_empty() {
            first.to_string()
        } else {
            format!("{root}/{first}")
        };

        seen.entry(first.to_string())
            .and_modify(|entry| entry.is_dir = entry.is_dir || is_dir)
            .or_insert(DocEntryDto {
                path: entry_path,
                name: first.to_string(),
                is_dir,
            });
    }

    let mut entries: Vec<DocEntryDto> = seen.into_values().collect();
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.cmp(&b.name),
    });
    entries
}

/// Strip `root/` from the front of `path`, returning the remainder. When
/// `root` is empty, returns `path` with any leading `/` trimmed (a no-op
/// for the repo-root-relative paths this module deals in).
fn strip_root<'a>(path: &'a str, root: &str) -> Option<&'a str> {
    if root.is_empty() {
        return Some(path.trim_start_matches('/'));
    }
    let prefix = format!("{root}/");
    path.strip_prefix(prefix.as_str())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // --- validate_rel_path: rejection vectors (pure, no filesystem access) ---

    #[test]
    fn rejects_parent_dir_traversal() {
        assert_eq!(validate_rel_path("../x", false), Err(DocsError::Forbidden));
    }

    #[test]
    fn rejects_embedded_parent_dir_traversal() {
        assert_eq!(
            validate_rel_path("a/../../b", false),
            Err(DocsError::Forbidden)
        );
    }

    #[test]
    fn rejects_absolute_unix_path() {
        assert_eq!(
            validate_rel_path("/etc/passwd", false),
            Err(DocsError::Forbidden)
        );
    }

    #[test]
    fn rejects_windows_style_path() {
        assert_eq!(
            validate_rel_path("C:\\Windows\\x.md", false),
            Err(DocsError::Forbidden)
        );
    }

    #[test]
    fn rejects_embedded_nul() {
        assert_eq!(
            validate_rel_path("planning/status\0.md", false),
            Err(DocsError::Forbidden)
        );
    }

    #[test]
    fn rejects_backslash() {
        assert_eq!(
            validate_rel_path("planning\\status.md", false),
            Err(DocsError::Forbidden)
        );
    }

    #[test]
    fn rejects_empty_path() {
        assert_eq!(validate_rel_path("", false), Err(DocsError::Forbidden));
        assert_eq!(validate_rel_path("   ", false), Err(DocsError::Forbidden));
    }

    #[test]
    fn rejects_traversal_after_filename() {
        assert_eq!(
            validate_rel_path("foo.md/../../.env", false),
            Err(DocsError::Forbidden)
        );
    }

    #[test]
    fn rejects_bare_env_when_md_required() {
        assert_eq!(validate_rel_path(".env", true), Err(DocsError::Forbidden));
    }

    #[test]
    fn rejects_non_md_extension_when_required() {
        assert_eq!(
            validate_rel_path("planning/notes.txt", true),
            Err(DocsError::Forbidden)
        );
    }

    #[test]
    fn pure_rejections_never_touch_the_filesystem() {
        // A path under a directory that cannot possibly exist — if
        // `validate_rel_path` did any filesystem access it would still
        // reject on stat failure, so this alone doesn't prove purity, but
        // combined with the traversal vectors above (which target real
        // system paths like `/etc/passwd` without erroring on access) it
        // demonstrates the pure check short-circuits before any I/O: these
        // all return `Forbidden`, not an `Io` variant that would indicate a
        // failed filesystem call.
        assert_eq!(validate_rel_path("../x", true), Err(DocsError::Forbidden));
        assert_eq!(
            validate_rel_path("/etc/passwd", true),
            Err(DocsError::Forbidden)
        );
    }

    // --- validate_rel_path: acceptance cases ---

    #[test]
    fn accepts_planning_status() {
        assert_eq!(
            validate_rel_path("planning/status.md", true),
            Ok(PathBuf::from("planning/status.md"))
        );
    }

    #[test]
    fn accepts_docs_serve_api() {
        assert_eq!(
            validate_rel_path("docs/serve-api.md", true),
            Ok(PathBuf::from("docs/serve-api.md"))
        );
    }

    #[test]
    fn accepts_repo_root_readme() {
        assert_eq!(
            validate_rel_path("README.md", true),
            Ok(PathBuf::from("README.md"))
        );
    }

    #[test]
    fn accepts_nested_path() {
        assert_eq!(
            validate_rel_path("planning/decisions/d1.md", true),
            Ok(PathBuf::from("planning/decisions/d1.md"))
        );
    }

    #[test]
    fn accepts_mdx_extension() {
        assert_eq!(
            validate_rel_path("docs/guide.mdx", true),
            Ok(PathBuf::from("docs/guide.mdx"))
        );
    }

    #[test]
    fn accepts_directory_path_without_extension_when_md_not_required() {
        assert_eq!(
            validate_rel_path("planning", false),
            Ok(PathBuf::from("planning"))
        );
    }

    // --- DocsError::Forbidden collapses the three rejection reasons ---

    #[test]
    fn forbidden_is_single_variant_for_all_three_reasons() {
        let traversal = validate_rel_path("../x", true).unwrap_err();
        let bad_ext = validate_rel_path("planning/notes.txt", true).unwrap_err();
        assert_eq!(traversal, DocsError::Forbidden);
        assert_eq!(bad_ext, DocsError::Forbidden);
        assert_eq!(traversal, bad_ext);
    }

    // --- resolve_allowlisted_path: real temp-dir fixtures ---

    /// Minimal temp-dir helper that cleans up on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = crate::testsupport::unique_temp_dir(&format!("bastion-docs-test-{name}"));
            fs::create_dir_all(&path).expect("create temp dir");
            Self(path)
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

    #[test]
    fn symlinked_allowlist_root_is_accepted() {
        // repo_root/planning -> vault (a sibling real directory), mirroring
        // every repo's real `planning/` -> `core/_planning/<repo>/` symlink.
        let vault = TempDir::new("vault");
        fs::write(vault.path().join("status.md"), "# status").expect("write status.md");

        let repo = TempDir::new("repo");
        std::os::unix::fs::symlink(vault.path(), repo.path().join("planning"))
            .expect("symlink planning -> vault");

        let rel = validate_rel_path("planning/status.md", true).expect("validates");
        let resolved =
            resolve_allowlisted_path(repo.path(), &rel).expect("symlinked root should resolve");

        let expected = vault
            .path()
            .canonicalize()
            .unwrap()
            .join("status.md")
            .canonicalize()
            .unwrap();
        assert_eq!(resolved, expected);
    }

    #[test]
    fn symlink_inside_allowlisted_root_pointing_outside_is_rejected() {
        // repo_root/docs is a REAL directory; repo_root/docs/escape is a
        // symlink pointing at a directory outside the repo entirely. The
        // pure check cannot see this — only canonicalize-then-contain can.
        let outside = TempDir::new("outside");
        fs::write(outside.path().join("secret.md"), "# secret").expect("write secret.md");

        let repo = TempDir::new("repo2");
        let docs_dir = repo.path().join("docs");
        fs::create_dir_all(&docs_dir).expect("create docs dir");
        std::os::unix::fs::symlink(outside.path(), docs_dir.join("escape"))
            .expect("symlink docs/escape -> outside");

        let rel = validate_rel_path("docs/escape/secret.md", true).expect("validates");
        let result = resolve_allowlisted_path(repo.path(), &rel);
        assert_eq!(result, Err(DocsError::Forbidden));
    }

    #[test]
    fn accepts_file_directly_inside_a_real_allowlisted_root() {
        let repo = TempDir::new("repo3");
        let docs_dir = repo.path().join("docs");
        fs::create_dir_all(&docs_dir).expect("create docs dir");
        fs::write(docs_dir.join("guide.md"), "# guide").expect("write guide.md");

        let rel = validate_rel_path("docs/guide.md", true).expect("validates");
        let resolved = resolve_allowlisted_path(repo.path(), &rel).expect("should resolve");
        assert_eq!(resolved, docs_dir.canonicalize().unwrap().join("guide.md"));
    }

    #[test]
    fn accepts_repo_root_markdown_file() {
        let repo = TempDir::new("repo4");
        fs::write(repo.path().join("README.md"), "# readme").expect("write README.md");

        let rel = validate_rel_path("README.md", true).expect("validates");
        let resolved = resolve_allowlisted_path(repo.path(), &rel).expect("should resolve");
        assert_eq!(
            resolved,
            repo.path().canonicalize().unwrap().join("README.md")
        );
    }

    #[test]
    fn rejects_path_outside_every_allowlisted_root() {
        let repo = TempDir::new("repo5");
        let src_dir = repo.path().join("src");
        fs::create_dir_all(&src_dir).expect("create src dir");
        fs::write(src_dir.join("main.md"), "# not allowlisted").expect("write main.md");

        let rel = validate_rel_path("src/main.md", true).expect("validates");
        let result = resolve_allowlisted_path(repo.path(), &rel);
        assert_eq!(result, Err(DocsError::Forbidden));
    }

    #[test]
    fn missing_file_inside_allowlisted_root_is_not_found() {
        let repo = TempDir::new("repo6");
        fs::create_dir_all(repo.path().join("planning")).expect("create planning dir");

        let rel = validate_rel_path("planning/missing.md", true).expect("validates");
        let result = resolve_allowlisted_path(repo.path(), &rel);
        assert_eq!(result, Err(DocsError::NotFound));
    }

    #[test]
    fn rejects_repo_root_level_directory_not_on_the_allowlist() {
        // `src` is a single path component that doesn't collide with any
        // `ALLOWED_ROOTS` entry, so the repo-root-markdown-file shortcut
        // must not treat it as readable just because it canonicalizes and
        // lives directly under the repo root — it is a directory, not a
        // markdown file.
        let repo = TempDir::new("repo8");
        let src_dir = repo.path().join("src");
        fs::create_dir_all(&src_dir).expect("create src dir");
        fs::write(src_dir.join("main.md"), "# not allowlisted").expect("write main.md");

        let rel = validate_rel_path("src", false).expect("validates");
        let result = resolve_allowlisted_path(repo.path(), &rel);
        assert_eq!(result, Err(DocsError::Forbidden));
    }

    #[test]
    fn missing_allowlisted_root_itself_is_forbidden_not_io_error() {
        let repo = TempDir::new("repo7");
        // No `business/` directory created at all.
        let rel = validate_rel_path("business/plan.md", true).expect("validates");
        let result = resolve_allowlisted_path(repo.path(), &rel);
        assert_eq!(result, Err(DocsError::Forbidden));
    }

    // --- build_doc_tree ---

    #[test]
    fn build_doc_tree_orders_directories_first_then_by_name() {
        let paths = vec![
            "planning/status.md".to_string(),
            "planning/decisions/d1.md".to_string(),
            "planning/handoff.md".to_string(),
        ];
        let entries = build_doc_tree("planning", &paths);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["decisions", "handoff.md", "status.md"]);
        assert!(entries[0].is_dir);
        assert!(!entries[1].is_dir);
        assert!(!entries[2].is_dir);
    }

    #[test]
    fn build_doc_tree_implies_intermediate_directories() {
        let paths = vec!["planning/decisions/nested/d1.md".to_string()];
        let entries = build_doc_tree("planning", &paths);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "decisions");
        assert_eq!(entries[0].path, "planning/decisions");
        assert!(entries[0].is_dir);
    }

    #[test]
    fn build_doc_tree_drops_directories_with_no_markdown() {
        // A directory that carries no markdown anywhere beneath it never
        // appears in `md_paths` at all, so it can never appear in the tree.
        let paths = vec!["planning/status.md".to_string()];
        let entries = build_doc_tree("planning", &paths);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "status.md");
    }

    #[test]
    fn build_doc_tree_at_repo_root_lists_top_level_allowlist() {
        let paths = vec![
            "docs/serve-api.md".to_string(),
            "planning/status.md".to_string(),
            "README.md".to_string(),
        ];
        let entries = build_doc_tree("", &paths);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["docs", "planning", "README.md"]);
        assert!(entries[0].is_dir);
        assert!(entries[1].is_dir);
        assert!(!entries[2].is_dir);
    }

    #[test]
    fn build_doc_tree_ignores_paths_outside_the_requested_root() {
        let paths = vec![
            "planning/status.md".to_string(),
            "business/plan.md".to_string(),
        ];
        let entries = build_doc_tree("planning", &paths);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "status.md");
    }

    // --- DocsError ---

    #[test]
    fn docs_error_variants_have_distinct_messages() {
        assert_eq!(
            DocsError::Forbidden.to_string(),
            "path is outside every allowlisted root"
        );
        assert_eq!(DocsError::NotFound.to_string(), "file not found");
        assert_eq!(
            DocsError::NotUtf8.to_string(),
            "file content is not valid UTF-8"
        );
        assert_eq!(
            DocsError::Io("disk full".to_string()).to_string(),
            "I/O error: disk full"
        );
    }
}
