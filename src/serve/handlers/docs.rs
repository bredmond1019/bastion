//! Docs read REST handlers for `bastion serve` (BA.11.Q).
//!
//! Thin shells over the pure/allowlist core in [`crate::serve::docs`] — no
//! path-validation or traversal logic lives here. This module's only job is
//! query-param shaping, resolving `{repo}` against the shared workspace
//! registry, walking markdown via `mev::brain::crawl::crawl_brain`, and
//! mapping [`crate::serve::docs::DocsError`] onto the HTTP status/`C0xx` code
//! table the block's spec defines.
//!
//! # Route
//! - `GET /api/docs/{repo}/tree?path=<rel-dir>` — allowlisted markdown tree,
//!   directories-first then by name. `?path=` is optional; absent walks
//!   every allowlisted root and returns `root: ""`.
//! - `GET /api/docs/{repo}/file?path=<rel-file>` — raw markdown read.
//!   `?path=` is REQUIRED; absent maps to the same `403`/`C003` response as
//!   any other rejected path (never distinguished from a real traversal
//!   attempt).
//!
//! # Pure core vs I/O shell (Rule 6)
//! [`docs_error_response`] and the `*Query` extractors are pure — unit-tested
//! directly. Everything that touches the filesystem
//! ([`skip_dirs_for`], [`repo_root_markdown_files`], [`walk_markdown_rel`],
//! [`assemble_md_paths`], and the handlers themselves) runs inside
//! `web::block`.
//!
//! # Error mapping
//! | Condition | Status | Code |
//! |---|---|---|
//! | Traversal, disallowed extension, outside every allowlisted root, or a
//! | missing required `?path=` on the file route | 403 | `C003` |
//! | Unknown `{repo}` (registry miss) | 404 | `C005` |
//! | File absent inside an allowlisted root | 404 | `C002` |
//! | Non-UTF-8 content | 500 | `C014` |
//! | Other read failure | 500 | `C009` |
//! | `web::block` failure | 500 | `C010` |

use std::path::{Path, PathBuf};

use actix_web::{HttpResponse, web};

use crate::config::{FileConfig, resolve_workspace_root};
use crate::serve::docs::{self, DocsError};
use crate::serve::dto::{DocFileDto, DocTreeDto, ErrorPayload};

/// `GET /api/docs/{repo}/tree` query params.
#[derive(Debug, Clone, PartialEq, Default, serde::Deserialize)]
pub struct TreeQuery {
    /// Optional repo-root-relative subtree to list; absent walks the whole
    /// allowlist and yields `root: ""`.
    #[serde(default)]
    pub path: Option<String>,
}

/// `GET /api/docs/{repo}/file` query params.
#[derive(Debug, Clone, PartialEq, Default, serde::Deserialize)]
pub struct FileQuery {
    /// Repo-root-relative file to read. REQUIRED — a missing `path` is
    /// rejected with the same uniform `403`/`C003` response as any other
    /// disallowed path.
    #[serde(default)]
    pub path: Option<String>,
}

// ── Pure core ────────────────────────────────────────────────────────────────

/// Resolve `name` against the shared registry, returning the workspace root
/// or `None` when `name` is not registered. Mirrors
/// `handlers/status.rs::resolve_root`.
fn resolve_root(name: &str, registry: &FileConfig) -> Option<PathBuf> {
    resolve_workspace_root(None, Some(name), registry).ok()
}

/// Build a 404 response for an unknown workspace registry name. Mirrors
/// `handlers/status.rs::unknown_workspace_response`.
fn unknown_workspace_response(name: &str) -> HttpResponse {
    HttpResponse::NotFound().json(ErrorPayload {
        code: "C005".to_owned(),
        message: format!("unknown workspace: {name}"),
    })
}

/// Build a 500 response from a `BlockingError` (thread panic / runtime
/// shutdown). Mirrors `handlers/status.rs::blocking_error_response`.
fn blocking_error_response(err: actix_web::error::BlockingError) -> HttpResponse {
    HttpResponse::InternalServerError().json(ErrorPayload {
        code: "C010".to_owned(),
        message: format!("blocking thread error: {err}"),
    })
}

/// Map a [`DocsError`] onto its documented HTTP status + `C0xx` code.
///
/// `Forbidden` is deliberately a single response shape regardless of which
/// of the three underlying reasons (traversal, bad extension, outside every
/// allowlisted root) produced it — the caller must never be able to
/// distinguish them.
fn docs_error_response(err: DocsError) -> HttpResponse {
    match err {
        DocsError::Forbidden => HttpResponse::Forbidden().json(ErrorPayload {
            code: "C003".to_owned(),
            message: "path is outside every allowlisted root".to_owned(),
        }),
        DocsError::NotFound => HttpResponse::NotFound().json(ErrorPayload {
            code: "C002".to_owned(),
            message: "file not found".to_owned(),
        }),
        DocsError::NotUtf8 => HttpResponse::InternalServerError().json(ErrorPayload {
            code: "C014".to_owned(),
            message: "file content is not valid UTF-8".to_owned(),
        }),
        DocsError::Io(msg) => HttpResponse::InternalServerError().json(ErrorPayload {
            code: "C009".to_owned(),
            message: msg,
        }),
    }
}

// ── I/O shell ──────────────────────────────────────────────────────────────

/// Resolve `repo_root`'s `[crawl].skip_dirs` the same way
/// `handlers/attention.rs` obtains brain config, but degrading to an empty
/// skip list rather than failing the request when no `brain.toml` is
/// resolvable from `repo_root` (a repo need not be a brain-managed workspace
/// for its docs to be readable).
fn skip_dirs_for(repo_root: &Path) -> Vec<String> {
    mev::brain::config::find_brain_root(repo_root)
        .ok()
        .and_then(|brain_root| {
            mev::brain::config::load_brain_config(&brain_root.join("brain.toml")).ok()
        })
        .map(|config| config.crawl.skip_dirs)
        .unwrap_or_default()
}

/// Enumerate direct repo-root-level `*.md`/`.mdx` files (`README.md`,
/// `CLAUDE.md`, …) — the repo-root markdown allowance `docs::ALLOWED_ROOTS`
/// doesn't cover (it only lists directories).
fn repo_root_markdown_files(repo_root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(repo_root) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_md = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("mdx"))
            .unwrap_or(false);
        if is_md && let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            out.push(name.to_string());
        }
    }
    out.sort();
    out
}

/// Walk `walk_root` for markdown via `mev::brain::crawl::crawl_brain`,
/// returning paths relative to `walk_root` with backslashes normalised to
/// forward slashes for a stable wire format.
fn walk_markdown_rel(walk_root: &Path, skip_dirs: &[String]) -> Vec<String> {
    let (files, _diagnostics) = mev::brain::crawl::crawl_brain(walk_root, skip_dirs);
    files
        .into_iter()
        .map(|f| f.rel.to_string_lossy().replace('\\', "/"))
        .collect()
}

/// Assemble the full set of repo-root-relative markdown paths under the
/// requested subtree (or the whole allowlist when `requested` is `None`),
/// plus the `root` field the resulting [`DocTreeDto`] should echo.
///
/// `requested`, when present, must already have passed
/// [`docs::validate_rel_path`] — this function only adds the
/// filesystem-touching allowlist resolution and the recursive walk.
fn assemble_md_paths(
    repo_root: &Path,
    skip_dirs: &[String],
    requested: Option<&Path>,
) -> Result<(String, Vec<String>), DocsError> {
    match requested {
        Some(rel) => {
            let resolved = docs::resolve_allowlisted_path(repo_root, rel)?;
            let root_field = rel.to_string_lossy().replace('\\', "/");
            let paths = walk_markdown_rel(&resolved, skip_dirs)
                .into_iter()
                .map(|r| {
                    if r.is_empty() {
                        root_field.clone()
                    } else {
                        format!("{root_field}/{r}")
                    }
                })
                .collect();
            Ok((root_field, paths))
        }
        None => {
            let mut paths = repo_root_markdown_files(repo_root);
            for root_name in docs::ALLOWED_ROOTS {
                let Ok(resolved) = docs::resolve_allowlisted_path(repo_root, Path::new(root_name))
                else {
                    // Root doesn't exist in this repo, or (defensively) fails
                    // to resolve — omit it from the top level rather than
                    // failing the whole request.
                    continue;
                };
                paths.extend(
                    walk_markdown_rel(&resolved, skip_dirs)
                        .into_iter()
                        .map(|r| format!("{root_name}/{r}")),
                );
            }
            Ok((String::new(), paths))
        }
    }
}

/// Read `resolved` (already allowlist-checked) into a [`DocFileDto`].
fn read_doc_file(repo: &str, rel_path: &str, resolved: &Path) -> Result<DocFileDto, DocsError> {
    let bytes = std::fs::read(resolved).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            DocsError::NotFound
        } else {
            DocsError::Io(e.to_string())
        }
    })?;
    let content = String::from_utf8(bytes).map_err(|_| DocsError::NotUtf8)?;
    let bytes = content.len() as u64;

    let modified = std::fs::metadata(resolved)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .map(|mtime| {
            let dt: chrono::DateTime<chrono::Utc> = mtime.into();
            dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        });

    Ok(DocFileDto {
        repo: repo.to_owned(),
        path: rel_path.to_owned(),
        content,
        bytes,
        modified,
    })
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// `GET /api/docs/{repo}/tree?path=<rel-dir>` — allowlisted markdown tree.
///
/// Bearer auth is inherited from the `/api` scope's `BearerAuthMiddleware` —
/// a request without a valid token never reaches this handler (401
/// upstream).
pub async fn get_docs_tree(
    repo: web::Path<String>,
    query: web::Query<TreeQuery>,
    registry: web::Data<FileConfig>,
) -> HttpResponse {
    let repo_name = repo.into_inner();
    let Some(root) = resolve_root(&repo_name, &registry) else {
        return unknown_workspace_response(&repo_name);
    };

    let requested = query.into_inner().path;
    let rel = match &requested {
        Some(p) => match docs::validate_rel_path(p, false) {
            Ok(rel) => Some(rel),
            Err(err) => return docs_error_response(err),
        },
        None => None,
    };

    let repo_for_dto = repo_name.clone();
    match web::block(move || -> Result<DocTreeDto, DocsError> {
        let skip_dirs = skip_dirs_for(&root);
        let (root_field, md_paths) = assemble_md_paths(&root, &skip_dirs, rel.as_deref())?;
        let entries = docs::build_doc_tree(&root_field, &md_paths);
        Ok(DocTreeDto {
            repo: repo_for_dto,
            root: root_field,
            entries,
        })
    })
    .await
    {
        Ok(Ok(dto)) => HttpResponse::Ok().json(dto),
        Ok(Err(err)) => docs_error_response(err),
        Err(err) => blocking_error_response(err),
    }
}

/// `GET /api/docs/{repo}/file?path=<rel-file>` — raw markdown read.
///
/// Bearer auth is inherited from the `/api` scope's `BearerAuthMiddleware` —
/// a request without a valid token never reaches this handler (401
/// upstream).
pub async fn get_docs_file(
    repo: web::Path<String>,
    query: web::Query<FileQuery>,
    registry: web::Data<FileConfig>,
) -> HttpResponse {
    let repo_name = repo.into_inner();
    let Some(root) = resolve_root(&repo_name, &registry) else {
        return unknown_workspace_response(&repo_name);
    };

    let Some(requested_path) = query.into_inner().path else {
        // Absent `?path=` on the file route is rejected with the exact same
        // uniform response as any other disallowed path.
        return docs_error_response(DocsError::Forbidden);
    };

    let rel = match docs::validate_rel_path(&requested_path, true) {
        Ok(rel) => rel,
        Err(err) => return docs_error_response(err),
    };

    let repo_for_dto = repo_name.clone();
    match web::block(move || -> Result<DocFileDto, DocsError> {
        let resolved = docs::resolve_allowlisted_path(&root, &rel)?;
        read_doc_file(&repo_for_dto, &requested_path, &resolved)
    })
    .await
    {
        Ok(Ok(dto)) => HttpResponse::Ok().json(dto),
        Ok(Err(err)) => docs_error_response(err),
        Err(err) => blocking_error_response(err),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Minimal temp-dir helper that cleans up on drop (mirrors
    /// `handlers/status.rs`'s test helper).
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            let dir =
                std::env::temp_dir().join(format!("bastion_docs_handler_test_{name}_{pid}_{id}"));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    // ── TreeQuery / FileQuery — query-param shaping ─────────────────────────

    #[test]
    fn tree_query_defaults_path_to_none() {
        let query: TreeQuery = serde_json::from_str("{}").unwrap();
        assert_eq!(query.path, None);
    }

    #[test]
    fn tree_query_parses_explicit_path() {
        let query: TreeQuery = serde_json::from_str(r#"{"path":"planning"}"#).unwrap();
        assert_eq!(query.path.as_deref(), Some("planning"));
    }

    #[test]
    fn file_query_defaults_path_to_none() {
        let query: FileQuery = serde_json::from_str("{}").unwrap();
        assert_eq!(query.path, None);
    }

    #[test]
    fn file_query_parses_explicit_path() {
        let query: FileQuery = serde_json::from_str(r#"{"path":"planning/status.md"}"#).unwrap();
        assert_eq!(query.path.as_deref(), Some("planning/status.md"));
    }

    // ── docs_error_response — error → HTTP mapping ──────────────────────────

    #[test]
    fn forbidden_maps_to_403_c003() {
        let resp = docs_error_response(DocsError::Forbidden);
        assert_eq!(resp.status(), actix_web::http::StatusCode::FORBIDDEN);
    }

    #[test]
    fn not_found_maps_to_404_c002() {
        let resp = docs_error_response(DocsError::NotFound);
        assert_eq!(resp.status(), actix_web::http::StatusCode::NOT_FOUND);
    }

    #[test]
    fn not_utf8_maps_to_500_c014() {
        let resp = docs_error_response(DocsError::NotUtf8);
        assert_eq!(
            resp.status(),
            actix_web::http::StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn io_error_maps_to_500_c009() {
        let resp = docs_error_response(DocsError::Io("disk full".to_owned()));
        assert_eq!(
            resp.status(),
            actix_web::http::StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    // ── resolve_root ─────────────────────────────────────────────────────

    #[test]
    fn resolve_root_unknown_name_is_none() {
        let registry = FileConfig::default();
        assert!(resolve_root("ghost", &registry).is_none());
    }

    #[test]
    fn resolve_root_known_name_returns_path() {
        let tmp = TempDir::new("resolve-root");
        let mut workspaces = HashMap::new();
        workspaces.insert("known".to_string(), tmp.path().to_path_buf());
        let registry = FileConfig {
            workspaces: Some(workspaces),
            ..Default::default()
        };
        assert_eq!(
            resolve_root("known", &registry),
            Some(tmp.path().to_path_buf())
        );
    }

    // ── skip_dirs_for — degrades to empty, never fails ──────────────────────

    #[test]
    fn skip_dirs_for_no_brain_toml_returns_empty() {
        let tmp = TempDir::new("skip-dirs-none");
        assert_eq!(skip_dirs_for(tmp.path()), Vec::<String>::new());
    }

    // ── repo_root_markdown_files ─────────────────────────────────────────

    #[test]
    fn repo_root_markdown_files_finds_md_and_mdx_only() {
        let tmp = TempDir::new("root-md");
        write(&tmp.path().join("README.md"), "# readme");
        write(&tmp.path().join("guide.mdx"), "# guide");
        write(&tmp.path().join("notes.txt"), "not markdown");
        std::fs::create_dir_all(tmp.path().join("docs")).unwrap();

        let mut files = repo_root_markdown_files(tmp.path());
        files.sort();
        assert_eq!(
            files,
            vec!["README.md".to_string(), "guide.mdx".to_string()]
        );
    }

    #[test]
    fn repo_root_markdown_files_missing_dir_returns_empty() {
        let tmp = TempDir::new("root-md-missing");
        let ghost = tmp.path().join("does-not-exist");
        assert_eq!(repo_root_markdown_files(&ghost), Vec::<String>::new());
    }

    // ── walk_markdown_rel ────────────────────────────────────────────────

    #[test]
    fn walk_markdown_rel_finds_nested_markdown() {
        let tmp = TempDir::new("walk-md");
        write(&tmp.path().join("status.md"), "# status");
        write(&tmp.path().join("decisions/d1.md"), "# d1");

        let mut rels = walk_markdown_rel(tmp.path(), &[]);
        rels.sort();
        assert_eq!(
            rels,
            vec!["decisions/d1.md".to_string(), "status.md".to_string()]
        );
    }

    #[test]
    fn walk_markdown_rel_honours_skip_dirs() {
        let tmp = TempDir::new("walk-md-skip");
        write(&tmp.path().join("status.md"), "# status");
        write(&tmp.path().join("archive/old.md"), "# old");

        let rels = walk_markdown_rel(tmp.path(), &["archive".to_string()]);
        assert_eq!(rels, vec!["status.md".to_string()]);
    }

    // ── assemble_md_paths ────────────────────────────────────────────────

    #[test]
    fn assemble_md_paths_top_level_combines_roots_and_repo_root_files() {
        let tmp = TempDir::new("assemble-top");
        write(&tmp.path().join("README.md"), "# readme");
        write(&tmp.path().join("docs/serve-api.md"), "# serve api");
        write(&tmp.path().join("planning/status.md"), "# status");

        let (root_field, paths) = assemble_md_paths(tmp.path(), &[], None).unwrap();
        assert_eq!(root_field, "");
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            vec![
                "README.md".to_string(),
                "docs/serve-api.md".to_string(),
                "planning/status.md".to_string(),
            ]
        );
    }

    #[test]
    fn assemble_md_paths_top_level_omits_missing_allowlisted_roots() {
        let tmp = TempDir::new("assemble-top-missing");
        write(&tmp.path().join("docs/serve-api.md"), "# serve api");
        // No `planning/` or `business/` directory created.

        let (_, paths) = assemble_md_paths(tmp.path(), &[], None).unwrap();
        assert_eq!(paths, vec!["docs/serve-api.md".to_string()]);
    }

    #[test]
    fn assemble_md_paths_subtree_returns_paths_prefixed_with_requested_root() {
        let tmp = TempDir::new("assemble-subtree");
        write(&tmp.path().join("planning/status.md"), "# status");
        write(&tmp.path().join("planning/decisions/d1.md"), "# d1");

        let rel = docs::validate_rel_path("planning", false).unwrap();
        let (root_field, mut paths) = assemble_md_paths(tmp.path(), &[], Some(&rel)).unwrap();
        paths.sort();
        assert_eq!(root_field, "planning");
        assert_eq!(
            paths,
            vec![
                "planning/decisions/d1.md".to_string(),
                "planning/status.md".to_string(),
            ]
        );
    }

    #[test]
    fn assemble_md_paths_subtree_outside_allowlist_is_forbidden() {
        let tmp = TempDir::new("assemble-subtree-forbidden");
        std::fs::create_dir_all(tmp.path().join("src")).unwrap();
        write(&tmp.path().join("src/main.md"), "# not allowlisted");

        let rel = docs::validate_rel_path("src", false).unwrap();
        let result = assemble_md_paths(tmp.path(), &[], Some(&rel));
        assert_eq!(result, Err(DocsError::Forbidden));
    }

    // ── read_doc_file ────────────────────────────────────────────────────

    #[test]
    fn read_doc_file_returns_raw_content_and_byte_count() {
        let tmp = TempDir::new("read-file");
        let target = tmp.path().join("status.md");
        write(&target, "# status\n\nhello");

        let dto = read_doc_file("bastion", "planning/status.md", &target).unwrap();
        assert_eq!(dto.repo, "bastion");
        assert_eq!(dto.path, "planning/status.md");
        assert_eq!(dto.content, "# status\n\nhello");
        assert_eq!(dto.bytes, "# status\n\nhello".len() as u64);
        assert!(dto.modified.is_some());
    }

    #[test]
    fn read_doc_file_missing_file_is_not_found() {
        let tmp = TempDir::new("read-file-missing");
        let target = tmp.path().join("ghost.md");
        let result = read_doc_file("bastion", "planning/ghost.md", &target);
        assert_eq!(result, Err(DocsError::NotFound));
    }

    #[test]
    fn read_doc_file_non_utf8_content_is_not_utf8_error() {
        let tmp = TempDir::new("read-file-binary");
        let target = tmp.path().join("bad.md");
        std::fs::write(&target, [0xff, 0xfe, 0x00, 0x01]).unwrap();

        let result = read_doc_file("bastion", "docs/bad.md", &target);
        assert_eq!(result, Err(DocsError::NotUtf8));
    }
}
