//! Repo / workflow status REST handlers for `bastion serve` (BA.11.D).
//!
//! All four routes resolve a workspace root from the shared [`FileConfig`]
//! registry (loaded once at server startup and shared via `web::Data`), then
//! read + parse plain files under that root.  Parsing is delegated entirely
//! to the pure functions in [`crate::serve::status`] — this module is the
//! thin I/O shell over them (per the project's pure-logic/I/O split, Rule 6).
//!
//! # Routes
//! - `GET /api/repos`                  — list workspace registry entries
//! - `GET /api/repos/{name}/status`    — parsed `planning/status.md`
//! - `GET /api/repos/{name}/handoff`   — parsed `planning/handoff.md`
//! - `GET /api/repos/{name}/workflows` — parsed `sdlc-flow-state.json` files
//!
//! # Error mapping
//! - Unknown workspace name (not in the registry) → 404 + `C005`
//!   (ConfigError — a registry miss, distinct from a registered repo with no
//!   handoff).
//! - Known workspace but `status.md`/`handoff.md` missing or malformed →
//!   404 + `C002` (status/handoff routes only — `/repos` and `/workflows`
//!   degrade to an empty/partial result instead of failing the whole route).
//! - Thread-pool failure (`web::block` panic) → 500 + `C010`.

use std::path::{Path, PathBuf};

use actix_web::{HttpResponse, web};

use crate::config::{FileConfig, resolve_workspace_root};
use crate::serve::dto::{
    ErrorPayload, HandoffInfoDto, RepoStatusDto, RepoSummaryDto, RepoWorkflowStateDto,
    WorkflowStateDto,
};
use crate::serve::status::flow::{FlowState, parse_flow_state};
use crate::serve::status::handoff::{HandoffInfo, read_handoff};
use crate::serve::status::repo::parse_status;

// ── Handler helpers ───────────────────────────────────────────────────────────

/// Build a 404 response for an unknown workspace registry name.
///
/// Uses `C005` (ConfigError — a workspace absent from the registry is a
/// config/registry miss) so it is distinguishable from a registered repo
/// that is merely missing `status.md`/`handoff.md` (`C002`). See
/// `planning/serve-ui-contract-gaps/tasks.md` Gap 4.
fn unknown_workspace_response(name: &str) -> HttpResponse {
    HttpResponse::NotFound().json(ErrorPayload {
        code: "C005".to_owned(),
        message: format!("unknown workspace: {name}"),
    })
}

/// Build a 500 response from a `BlockingError` (thread panic / runtime shutdown).
fn blocking_error_response(err: actix_web::error::BlockingError) -> HttpResponse {
    HttpResponse::InternalServerError().json(ErrorPayload {
        code: "C010".to_owned(),
        message: format!("blocking thread error: {err}"),
    })
}

/// Resolve `name` against the shared registry, returning the workspace root
/// or `None` when `name` is not registered.
fn resolve_root(name: &str, registry: &FileConfig) -> Option<PathBuf> {
    resolve_workspace_root(None, Some(name), registry).ok()
}

// ── I/O shells (file reads — not pure, kept thin) ──────────────────────────────

/// Read + parse `{root}/planning/status.md`, filling in `name`/`has_handoff`
/// from the caller's context.  Returns `None` when the file is missing or
/// fails to parse (no frontmatter).
fn read_repo_status(name: &str, root: &Path) -> Option<RepoStatusDto> {
    let content = std::fs::read_to_string(root.join("planning/status.md")).ok()?;
    let mut status = parse_status(&content)?;
    status.name = name.to_string();
    status.has_handoff = root.join("planning/handoff.md").is_file();
    Some(status.into())
}

/// Build a best-effort [`RepoSummaryDto`] for one registry entry.
///
/// Degrades gracefully: an unreadable/malformed `status.md` yields an empty
/// `now` rather than excluding the repo from the list.
fn build_repo_summary(name: &str, root: &Path) -> RepoSummaryDto {
    let now = std::fs::read_to_string(root.join("planning/status.md"))
        .ok()
        .and_then(|content| parse_status(&content))
        .map(|s| s.now)
        .unwrap_or_default();
    let has_handoff = root.join("planning/handoff.md").is_file();

    RepoSummaryDto {
        name: name.to_string(),
        now,
        has_handoff,
    }
}

/// Build the full `GET /repos` list from the registry, sorted by name for
/// deterministic output.
fn build_repo_summaries(registry: &FileConfig) -> Vec<RepoSummaryDto> {
    let Some(workspaces) = registry.workspaces.as_ref() else {
        return Vec::new();
    };

    let mut names: Vec<&String> = workspaces.keys().collect();
    names.sort();

    names
        .into_iter()
        .map(|name| build_repo_summary(name, &workspaces[name]))
        .collect()
}

/// Read + parse `{root}/planning/handoff.md`. Returns `None` when the file
/// is missing or empty.
fn read_repo_handoff(root: &Path) -> Option<HandoffInfo> {
    let content = std::fs::read_to_string(root.join("planning/handoff.md")).ok()?;
    read_handoff(&content)
}

/// Walk `{root}/planning/*/sdlc/sdlc-flow-state.json`, parsing each match via
/// [`parse_flow_state`]. Missing/malformed entries are skipped silently —
/// the route (and the flow-watch poll loop, `src/serve/ws/server.rs`) get
/// whatever parses, empty when none do.
///
/// This is the shared enumeration core both `collect_repo_workflows` (the
/// REST handler) and the WS flow-watch loop build on — see
/// `planning/plan-serve-workflow-done-ws-push/tasks.md` Task 2.
pub(crate) fn collect_flow_states(root: &Path) -> Vec<FlowState> {
    let mut out = Vec::new();

    let Ok(entries) = std::fs::read_dir(root.join("planning")) else {
        return out;
    };

    for entry in entries.flatten() {
        let spec_dir = entry.path();
        if !spec_dir.is_dir() {
            continue;
        }

        let flow_path = spec_dir.join("sdlc").join("sdlc-flow-state.json");
        if let Ok(content) = std::fs::read_to_string(&flow_path)
            && let Some(state) = parse_flow_state(&content)
        {
            out.push(state);
        }
    }

    out.sort_by(|a: &FlowState, b: &FlowState| a.spec_slug.cmp(&b.spec_slug));
    out
}

/// `GET /api/repos/{name}/workflows` DTO projection over [`collect_flow_states`].
fn collect_repo_workflows(root: &Path) -> Vec<WorkflowStateDto> {
    collect_flow_states(root)
        .into_iter()
        .map(Into::into)
        .collect()
}

/// `GET /api/workflows` — cross-repo flow-state aggregate over every
/// registered workspace, following the `build_repo_summaries` (`:97`)
/// registry-iteration precedent exactly: empty/absent registry -> empty
/// Vec, else sort workspace names and walk each in order.
///
/// Reuses [`collect_flow_states`] verbatim per-repo (no second flow-state
/// walk) and tags each resulting entry with its owning repo name via
/// [`RepoWorkflowStateDto`]. A repo whose root fails to resolve, or which
/// has no `planning/` dir / only malformed flow-state files, contributes
/// zero entries without aborting the walk — the same degrade-gracefully
/// behaviour `collect_flow_states` and `build_repo_summaries` already have.
///
/// Output is ordered by `(repo name, then spec_slug)`: `collect_flow_states`
/// already sorts by `spec_slug` within a repo, and iterating the
/// (pre-sorted) repo names in order composes that into the full ordering.
pub(crate) fn collect_all_workflows(registry: &FileConfig) -> Vec<RepoWorkflowStateDto> {
    let Some(workspaces) = registry.workspaces.as_ref() else {
        return Vec::new();
    };

    let mut names: Vec<&String> = workspaces.keys().collect();
    names.sort();

    names
        .into_iter()
        .flat_map(|name| {
            let root = resolve_root(name, registry);
            root.into_iter().flat_map(move |root| {
                collect_flow_states(&root).into_iter().map(move |state| {
                    RepoWorkflowStateDto::from((name.clone(), WorkflowStateDto::from(state)))
                })
            })
        })
        .collect()
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// `GET /api/repos` — summarize every workspace registry entry.
///
/// Returns 200 with a JSON array of [`RepoSummaryDto`]; an empty/absent
/// registry yields `[]`.
pub async fn list_repos(registry: web::Data<FileConfig>) -> HttpResponse {
    match web::block(move || build_repo_summaries(&registry)).await {
        Ok(list) => HttpResponse::Ok().json(list),
        Err(err) => blocking_error_response(err),
    }
}

/// `GET /api/repos/{name}/status` — parsed `planning/status.md` for `name`.
///
/// Returns 404 when `name` is not a registered workspace, or when its
/// `status.md` is missing/malformed.
pub async fn get_repo_status(
    name: web::Path<String>,
    registry: web::Data<FileConfig>,
) -> HttpResponse {
    let name = name.into_inner();
    let Some(root) = resolve_root(&name, &registry) else {
        return unknown_workspace_response(&name);
    };

    match web::block(move || read_repo_status(&name, &root)).await {
        Ok(Some(dto)) => HttpResponse::Ok().json(dto),
        Ok(None) => HttpResponse::NotFound().json(ErrorPayload {
            code: "C002".to_owned(),
            message: "status.md not found or malformed".to_owned(),
        }),
        Err(err) => blocking_error_response(err),
    }
}

/// `GET /api/repos/{name}/handoff` — parsed `planning/handoff.md` for `name`.
///
/// Returns 404 when `name` is not a registered workspace, or when
/// `handoff.md` does not exist.
pub async fn get_repo_handoff(
    name: web::Path<String>,
    registry: web::Data<FileConfig>,
) -> HttpResponse {
    let name = name.into_inner();
    let Some(root) = resolve_root(&name, &registry) else {
        return unknown_workspace_response(&name);
    };

    match web::block(move || read_repo_handoff(&root)).await {
        Ok(Some(info)) => HttpResponse::Ok().json(HandoffInfoDto::from(info)),
        Ok(None) => HttpResponse::NotFound().json(ErrorPayload {
            code: "C002".to_owned(),
            message: "handoff.md not found".to_owned(),
        }),
        Err(err) => blocking_error_response(err),
    }
}

/// `GET /api/repos/{name}/workflows` — parsed `sdlc-flow-state.json` entries
/// under `name`'s `planning/` tree.
///
/// Returns 404 only when `name` is not a registered workspace; an absent or
/// empty `planning/` tree yields `[]`.
pub async fn get_repo_workflows(
    name: web::Path<String>,
    registry: web::Data<FileConfig>,
) -> HttpResponse {
    let name = name.into_inner();
    let Some(root) = resolve_root(&name, &registry) else {
        return unknown_workspace_response(&name);
    };

    match web::block(move || collect_repo_workflows(&root)).await {
        Ok(list) => HttpResponse::Ok().json(list),
        Err(err) => blocking_error_response(err),
    }
}

/// `GET /api/workflows` — cross-repo flow-state aggregate over every
/// registered workspace (A2). Shaped exactly like [`list_repos`]: wraps the
/// pure [`collect_all_workflows`] in `web::block`, returning 200 with the
/// resulting (possibly empty) list, or a 500 on thread-pool failure.
///
/// Never 404s — an empty/absent registry degrades to `200 []`, matching
/// `collect_all_workflows`'s own degrade-gracefully contract.
pub async fn list_all_workflows(registry: web::Data<FileConfig>) -> HttpResponse {
    match web::block(move || collect_all_workflows(&registry)).await {
        Ok(list) => HttpResponse::Ok().json(list),
        Err(err) => blocking_error_response(err),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Minimal temp-dir helper that cleans up on drop (avoids adding `tempfile` dep
    /// — mirrors `src/validate/mod.rs`'s test helper).
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            let dir = std::env::temp_dir().join(format!("bastion_status_handler_test_{pid}_{id}"));
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

    const STATUS_MD: &str = include_str!("../status/fixtures/status_well_formed.md");
    const HANDOFF_MD: &str = include_str!("../status/fixtures/handoff_minimal.md");
    const FLOW_JSON: &str = include_str!("../status/fixtures/flow_state_valid.json");

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    fn registry_with(name: &str, root: &Path) -> FileConfig {
        let mut workspaces = HashMap::new();
        workspaces.insert(name.to_string(), root.to_path_buf());
        FileConfig {
            workspaces: Some(workspaces),
            ..Default::default()
        }
    }

    // ── build_repo_summaries / build_repo_summary ──────────────────────────

    #[test]
    fn build_repo_summaries_empty_registry_returns_empty_vec() {
        let registry = FileConfig::default();
        assert_eq!(build_repo_summaries(&registry), Vec::new());
    }

    #[test]
    fn build_repo_summary_reads_now_and_has_handoff() {
        let tmp = TempDir::new();
        write(&tmp.path().join("planning/status.md"), STATUS_MD);
        write(&tmp.path().join("planning/handoff.md"), HANDOFF_MD);

        let summary = build_repo_summary("repo-a", tmp.path());
        assert_eq!(summary.name, "repo-a");
        assert_eq!(summary.now, "BA.11.D in progress — repo status API");
        assert!(summary.has_handoff);
    }

    #[test]
    fn build_repo_summary_missing_status_md_degrades_to_empty_now() {
        let tmp = TempDir::new();
        let summary = build_repo_summary("repo-b", tmp.path());
        assert_eq!(summary.now, "");
        assert!(!summary.has_handoff);
    }

    #[test]
    fn build_repo_summaries_sorted_by_name() {
        let tmp = TempDir::new();
        let mut workspaces = HashMap::new();
        workspaces.insert("zeta".to_string(), tmp.path().to_path_buf());
        workspaces.insert("alpha".to_string(), tmp.path().to_path_buf());
        let registry = FileConfig {
            workspaces: Some(workspaces),
            ..Default::default()
        };

        let summaries = build_repo_summaries(&registry);
        let names: Vec<&str> = summaries.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "zeta"]);
    }

    // ── read_repo_status ─────────────────────────────────────────────────

    #[test]
    fn read_repo_status_parses_and_fills_caller_fields() {
        let tmp = TempDir::new();
        write(&tmp.path().join("planning/status.md"), STATUS_MD);
        write(&tmp.path().join("planning/handoff.md"), HANDOFF_MD);

        let dto = read_repo_status("repo-c", tmp.path()).expect("should parse");
        assert_eq!(dto.name, "repo-c");
        assert!(dto.has_handoff);
        assert_eq!(dto.momentum_next, "Wire WS event push");
    }

    #[test]
    fn read_repo_status_missing_file_returns_none() {
        let tmp = TempDir::new();
        assert!(read_repo_status("repo-d", tmp.path()).is_none());
    }

    // ── read_repo_handoff ────────────────────────────────────────────────

    #[test]
    fn read_repo_handoff_parses_title() {
        let tmp = TempDir::new();
        write(&tmp.path().join("planning/handoff.md"), HANDOFF_MD);

        let info = read_repo_handoff(tmp.path()).expect("should parse");
        assert_eq!(info.title, "Handoff — minimal fixture");
    }

    #[test]
    fn read_repo_handoff_missing_file_returns_none() {
        let tmp = TempDir::new();
        assert!(read_repo_handoff(tmp.path()).is_none());
    }

    // ── collect_flow_states ───────────────────────────────────────────────

    #[test]
    fn collect_flow_states_finds_nested_flow_state() {
        let tmp = TempDir::new();
        write(
            &tmp.path()
                .join("planning/phase6-blockA/sdlc/sdlc-flow-state.json"),
            FLOW_JSON,
        );

        let flows = collect_flow_states(tmp.path());
        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].spec_slug, "phase6-blockA");
        assert_eq!(flows[0].status, "done");
    }

    #[test]
    fn collect_flow_states_no_planning_dir_returns_empty() {
        let tmp = TempDir::new();
        assert_eq!(collect_flow_states(tmp.path()), Vec::new());
    }

    #[test]
    fn collect_flow_states_skips_malformed_entries() {
        let tmp = TempDir::new();
        write(
            &tmp.path()
                .join("planning/bad-spec/sdlc/sdlc-flow-state.json"),
            "{ not json",
        );
        write(
            &tmp.path()
                .join("planning/good-spec/sdlc/sdlc-flow-state.json"),
            FLOW_JSON,
        );

        let flows = collect_flow_states(tmp.path());
        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].spec_slug, "phase6-blockA");
    }

    #[test]
    fn collect_flow_states_sorted_by_spec_slug() {
        let tmp = TempDir::new();
        let zeta_json = FLOW_JSON.replace("phase6-blockA", "zeta-spec");
        let alpha_json = FLOW_JSON.replace("phase6-blockA", "alpha-spec");
        write(
            &tmp.path().join("planning/z-dir/sdlc/sdlc-flow-state.json"),
            &zeta_json,
        );
        write(
            &tmp.path().join("planning/a-dir/sdlc/sdlc-flow-state.json"),
            &alpha_json,
        );

        let flows = collect_flow_states(tmp.path());
        let slugs: Vec<&str> = flows.iter().map(|f| f.spec_slug.as_str()).collect();
        assert_eq!(slugs, vec!["alpha-spec", "zeta-spec"]);
    }

    // ── collect_repo_workflows ───────────────────────────────────────────

    #[test]
    fn collect_repo_workflows_finds_nested_flow_state() {
        let tmp = TempDir::new();
        write(
            &tmp.path()
                .join("planning/phase6-blockA/sdlc/sdlc-flow-state.json"),
            FLOW_JSON,
        );

        let flows = collect_repo_workflows(tmp.path());
        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].spec_slug, "phase6-blockA");
        assert_eq!(flows[0].status, "done");
    }

    #[test]
    fn collect_repo_workflows_no_planning_dir_returns_empty() {
        let tmp = TempDir::new();
        assert_eq!(collect_repo_workflows(tmp.path()), Vec::new());
    }

    #[test]
    fn collect_repo_workflows_skips_malformed_entries() {
        let tmp = TempDir::new();
        write(
            &tmp.path()
                .join("planning/bad-spec/sdlc/sdlc-flow-state.json"),
            "{ not json",
        );
        write(
            &tmp.path()
                .join("planning/good-spec/sdlc/sdlc-flow-state.json"),
            FLOW_JSON,
        );

        let flows = collect_repo_workflows(tmp.path());
        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].spec_slug, "phase6-blockA");
    }

    // ── collect_all_workflows ────────────────────────────────────────────

    #[test]
    fn collect_all_workflows_empty_registry_returns_empty_vec() {
        let registry = FileConfig::default();
        assert_eq!(collect_all_workflows(&registry), Vec::new());
    }

    #[test]
    fn collect_all_workflows_two_repos_returns_tagged_and_ordered_entries() {
        let tmp_a = TempDir::new();
        let tmp_z = TempDir::new();
        write(
            &tmp_a
                .path()
                .join("planning/phase6-blockA/sdlc/sdlc-flow-state.json"),
            FLOW_JSON,
        );
        let zeta_json = FLOW_JSON.replace("phase6-blockA", "zeta-spec");
        write(
            &tmp_z
                .path()
                .join("planning/z-dir/sdlc/sdlc-flow-state.json"),
            &zeta_json,
        );

        let mut workspaces = HashMap::new();
        workspaces.insert("alpha-repo".to_string(), tmp_a.path().to_path_buf());
        workspaces.insert("zeta-repo".to_string(), tmp_z.path().to_path_buf());
        let registry = FileConfig {
            workspaces: Some(workspaces),
            ..Default::default()
        };

        let entries = collect_all_workflows(&registry);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].repo, "alpha-repo");
        assert_eq!(entries[0].spec_slug, "phase6-blockA");
        assert_eq!(entries[1].repo, "zeta-repo");
        assert_eq!(entries[1].spec_slug, "zeta-spec");
    }

    #[test]
    fn collect_all_workflows_orders_by_repo_then_spec_slug_within_repo() {
        let tmp_a = TempDir::new();
        let zeta_json = FLOW_JSON.replace("phase6-blockA", "zeta-spec");
        let alpha_json = FLOW_JSON.replace("phase6-blockA", "alpha-spec");
        write(
            &tmp_a
                .path()
                .join("planning/z-dir/sdlc/sdlc-flow-state.json"),
            &zeta_json,
        );
        write(
            &tmp_a
                .path()
                .join("planning/a-dir/sdlc/sdlc-flow-state.json"),
            &alpha_json,
        );

        let registry = registry_with("only-repo", tmp_a.path());
        let entries = collect_all_workflows(&registry);
        let slugs: Vec<&str> = entries.iter().map(|e| e.spec_slug.as_str()).collect();
        assert_eq!(slugs, vec!["alpha-spec", "zeta-spec"]);
    }

    #[test]
    fn collect_all_workflows_malformed_repo_does_not_suppress_sibling_repo() {
        let tmp_bad = TempDir::new();
        let tmp_good = TempDir::new();
        write(
            &tmp_bad
                .path()
                .join("planning/bad-spec/sdlc/sdlc-flow-state.json"),
            "{ not json",
        );
        write(
            &tmp_good
                .path()
                .join("planning/good-spec/sdlc/sdlc-flow-state.json"),
            FLOW_JSON,
        );

        let mut workspaces = HashMap::new();
        workspaces.insert("bad-repo".to_string(), tmp_bad.path().to_path_buf());
        workspaces.insert("good-repo".to_string(), tmp_good.path().to_path_buf());
        let registry = FileConfig {
            workspaces: Some(workspaces),
            ..Default::default()
        };

        let entries = collect_all_workflows(&registry);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].repo, "good-repo");
        assert_eq!(entries[0].spec_slug, "phase6-blockA");
    }

    #[test]
    fn collect_all_workflows_nonexistent_repo_path_is_skipped() {
        let tmp_good = TempDir::new();
        write(
            &tmp_good
                .path()
                .join("planning/good-spec/sdlc/sdlc-flow-state.json"),
            FLOW_JSON,
        );

        let mut workspaces = HashMap::new();
        workspaces.insert(
            "ghost-repo".to_string(),
            PathBuf::from("/nonexistent/path/should/not/exist"),
        );
        workspaces.insert("good-repo".to_string(), tmp_good.path().to_path_buf());
        let registry = FileConfig {
            workspaces: Some(workspaces),
            ..Default::default()
        };

        let entries = collect_all_workflows(&registry);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].repo, "good-repo");
    }

    // ── resolve_root ──────────────────────────────────────────────────────

    #[test]
    fn resolve_root_unknown_name_is_none() {
        let registry = FileConfig::default();
        assert!(resolve_root("ghost", &registry).is_none());
    }

    #[test]
    fn resolve_root_known_name_returns_path() {
        let tmp = TempDir::new();
        let registry = registry_with("known", tmp.path());
        assert_eq!(
            resolve_root("known", &registry),
            Some(tmp.path().to_path_buf())
        );
    }
}
