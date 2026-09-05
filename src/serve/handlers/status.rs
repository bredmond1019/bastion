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
use serde::Deserialize;

use crate::config::{FileConfig, resolve_workspace_root};
use crate::serve::dto::{
    ErrorPayload, HandoffInfoDto, RepoStatusAggregateDto, RepoStatusDto, RepoSummaryDto,
    RepoWorkflowStateDto, SkippedWorkspaceDto, WorkflowStateDto, WorkflowsAggregateDto,
};
use crate::serve::handlers::runs::bool_flag_from_str;
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
/// Why `GET /api/repos/status` could not report a `RepoStatusDto` for one
/// registered workspace. Wire vocabulary is fixed at two values (see
/// [`SkippedWorkspaceDto::reason`] and serve-api §11.7) — first match wins,
/// at most one reason per name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RepoStatusSkipReason {
    /// `name` was not found in the workspace registry (only reachable via
    /// `?names=` — the unscoped walk only ever iterates registered names).
    UnregisteredWorkspace,
    /// `name` is registered but its `planning/status.md` is missing or
    /// failed to parse — the same condition the per-repo route
    /// (`GET /api/repos/{name}/status`) answers with a 404 + `C002`.
    MissingOrMalformedStatus,
}

impl RepoStatusSkipReason {
    /// The wire string carried by [`SkippedWorkspaceDto::reason`].
    fn as_wire(&self) -> &'static str {
        match self {
            RepoStatusSkipReason::UnregisteredWorkspace => "unregistered_workspace",
            RepoStatusSkipReason::MissingOrMalformedStatus => "missing_or_malformed_status",
        }
    }
}

/// `GET /api/repos/status` — cross-repo `status.md` aggregate over every
/// registered workspace (BA.ticket.batch-repo-status), shaped on
/// [`collect_all_workflows_reporting`] (`:244`): a pure collector, one walk
/// of the registry, and an honest `{entries, skipped}` envelope naming every
/// workspace this request covered but could not report on.
///
/// `names`, when `Some`, scopes the walk to exactly those (deduplicated —
/// see [`dedupe_names`]) registered-or-not workspace names instead of every
/// entry in the registry; `None` walks the full sorted registry, matching
/// `build_repo_summaries` / `collect_all_workflows_reporting`'s existing
/// registry-iteration precedent (empty/absent registry -> empty result,
/// else sort names and walk each in order).
///
/// [`read_repo_status`] is reused UNCHANGED — this function never asks it to
/// degrade gracefully, so the per-repo route's 404-on-malformed contract
/// (`get_repo_status`, `:308`) is untouched by this aggregate existing.
///
/// A name that does not resolve in the registry (`resolve_root` returns
/// `None`) is named in `skipped` as [`RepoStatusSkipReason::UnregisteredWorkspace`]
/// rather than 404ing the whole request — this is the only way an
/// unregistered name reaches `skipped`, since the unscoped (`names: None`)
/// walk only ever iterates names already known to be registered. A
/// registered name whose `status.md` is missing or fails to parse is named
/// as [`RepoStatusSkipReason::MissingOrMalformedStatus`].
///
/// The registry is walked exactly once per request: each name in the
/// (deduplicated) target list is resolved and read exactly once.
pub(crate) fn collect_all_repo_status_reporting(
    registry: &FileConfig,
    names: Option<&[String]>,
) -> RepoStatusAggregateDto {
    let target_names: Vec<String> = match names {
        Some(requested) => dedupe_names(requested),
        None => {
            let Some(workspaces) = registry.workspaces.as_ref() else {
                return RepoStatusAggregateDto {
                    entries: Vec::new(),
                    skipped: Vec::new(),
                };
            };
            let mut names: Vec<String> = workspaces.keys().cloned().collect();
            names.sort();
            names
        }
    };

    let mut entries = Vec::new();
    let mut skipped = Vec::new();

    for name in target_names {
        let Some(root) = resolve_root(&name, registry) else {
            skipped.push(SkippedWorkspaceDto {
                repo: name,
                reason: RepoStatusSkipReason::UnregisteredWorkspace
                    .as_wire()
                    .to_string(),
            });
            continue;
        };

        match read_repo_status(&name, &root) {
            Some(dto) => entries.push(dto),
            None => skipped.push(SkippedWorkspaceDto {
                repo: name,
                reason: RepoStatusSkipReason::MissingOrMalformedStatus
                    .as_wire()
                    .to_string(),
            }),
        }
    }

    RepoStatusAggregateDto { entries, skipped }
}

/// Deduplicate `?names=a,b,c` into first-occurrence order, so a name
/// repeated in the query string is resolved and read exactly once.
fn dedupe_names(names: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    names
        .iter()
        .filter(|name| seen.insert((*name).clone()))
        .cloned()
        .collect()
}

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

/// Why `GET /api/workflows?with_skipped=1` could not fully report on a
/// registered workspace. Wire vocabulary is fixed at three values (see
/// [`SkippedWorkspaceDto::reason`] and serve-api §11.6) — first match wins,
/// at most one reason per repo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkipReason {
    /// The registered path is not a readable directory.
    UnreadableRoot,
    /// `root` is readable but `{root}/planning` is not a readable directory.
    NoPlanningDir,
    /// At least one `planning/*/sdlc/sdlc-flow-state.json` was found and
    /// failed to parse. Any files that DID parse are still returned — this
    /// reason means "incomplete report", not "no contribution".
    MalformedFlowState,
}

impl SkipReason {
    /// The wire string carried by [`SkippedWorkspaceDto::reason`].
    fn as_wire(&self) -> &'static str {
        match self {
            SkipReason::UnreadableRoot => "unreadable_root",
            SkipReason::NoPlanningDir => "no_planning_dir",
            SkipReason::MalformedFlowState => "malformed_flow_state",
        }
    }
}

/// Walk `{root}/planning/*/sdlc/sdlc-flow-state.json`, parsing each match via
/// [`parse_flow_state`], and classify why the report is incomplete (if it
/// is). This is the single walk both the default response and the
/// `?with_skipped=1` envelope are built from — [`collect_flow_states`] wraps
/// this and discards the classification, so `?with_skipped=1` costs no
/// second traversal (serve-api §11.6).
///
/// Classification is first-match-wins, at most one reason per repo:
/// 1. `root` is not a readable directory -> [`SkipReason::UnreadableRoot`],
///    empty states.
/// 2. `root` is readable but `{root}/planning` is not a readable directory
///    -> [`SkipReason::NoPlanningDir`], empty states.
/// 3. At least one flow-state file was found and failed to parse ->
///    [`SkipReason::MalformedFlowState`], while still returning every state
///    that DID parse.
/// 4. Otherwise `None` — a readable `planning/` with zero flow-state files
///    is a healthy "no runs" state, not a skip.
fn scan_flow_states(root: &Path) -> (Vec<FlowState>, Option<SkipReason>) {
    if !root.is_dir() {
        return (Vec::new(), Some(SkipReason::UnreadableRoot));
    }

    let Ok(entries) = std::fs::read_dir(root.join("planning")) else {
        return (Vec::new(), Some(SkipReason::NoPlanningDir));
    };

    let mut out = Vec::new();
    let mut saw_malformed = false;

    for entry in entries.flatten() {
        let spec_dir = entry.path();
        if !spec_dir.is_dir() {
            continue;
        }

        let flow_path = spec_dir.join("sdlc").join("sdlc-flow-state.json");
        let Ok(content) = std::fs::read_to_string(&flow_path) else {
            continue;
        };

        match parse_flow_state(&content) {
            Some(state) => out.push(state),
            None => saw_malformed = true,
        }
    }

    out.sort_by(|a: &FlowState, b: &FlowState| a.spec_slug.cmp(&b.spec_slug));

    let reason = if saw_malformed {
        Some(SkipReason::MalformedFlowState)
    } else {
        None
    };
    (out, reason)
}

/// Walk `{root}/planning/*/sdlc/sdlc-flow-state.json`, parsing each match via
/// [`parse_flow_state`]. Missing/malformed entries are skipped silently —
/// the route (and the flow-watch poll loop, `src/serve/ws/server.rs`) get
/// whatever parses, empty when none do.
///
/// This is the shared enumeration core both `collect_repo_workflows` (the
/// REST handler) and the WS flow-watch loop build on — see
/// `planning/plan-serve-workflow-done-ws-push/tasks.md` Task 2. Thin wrapper
/// over [`scan_flow_states`], discarding the skip classification.
pub(crate) fn collect_flow_states(root: &Path) -> Vec<FlowState> {
    scan_flow_states(root).0
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
/// result, else sort workspace names and walk each in order.
///
/// Calls [`scan_flow_states`] once per repo (no second traversal — the same
/// walk backs both `entries` and `skipped`). A repo whose root fails to
/// resolve, or which has no `planning/` dir / has malformed flow-state
/// files, contributes whatever DID parse to `entries` and is additionally
/// named in `skipped` with a reason. A repo with a readable `planning/` and
/// zero flow-state files contributes nothing to either list — "no runs" is
/// healthy, not a skip.
///
/// `entries` is ordered by `(repo name, then spec_slug)` — `scan_flow_states`
/// already sorts by `spec_slug` within a repo, and iterating the
/// (pre-sorted) repo names in order composes that into the full ordering.
/// `skipped` inherits the same sorted registry-name order for free.
pub(crate) fn collect_all_workflows_reporting(registry: &FileConfig) -> WorkflowsAggregateDto {
    let Some(workspaces) = registry.workspaces.as_ref() else {
        return WorkflowsAggregateDto {
            entries: Vec::new(),
            skipped: Vec::new(),
        };
    };

    let mut names: Vec<&String> = workspaces.keys().collect();
    names.sort();

    let mut entries = Vec::new();
    let mut skipped = Vec::new();

    for name in names {
        let Some(root) = resolve_root(name, registry) else {
            skipped.push(SkippedWorkspaceDto {
                repo: name.clone(),
                reason: SkipReason::UnreadableRoot.as_wire().to_string(),
            });
            continue;
        };

        let (states, reason) = scan_flow_states(&root);
        entries.extend(states.into_iter().map(|state| {
            RepoWorkflowStateDto::from((name.clone(), WorkflowStateDto::from(state)))
        }));
        if let Some(reason) = reason {
            skipped.push(SkippedWorkspaceDto {
                repo: name.clone(),
                reason: reason.as_wire().to_string(),
            });
        }
    }

    WorkflowsAggregateDto { entries, skipped }
}

/// `GET /api/workflows` (default, no query param) — the bare `entries` array
/// from [`collect_all_workflows_reporting`], preserving this function's
/// existing signature and doc contract for `src/serve/ws/server.rs` and
/// `handlers/runs.rs`'s `with_repo` join, both of which must keep compiling
/// and behaving byte-identically.
pub(crate) fn collect_all_workflows(registry: &FileConfig) -> Vec<RepoWorkflowStateDto> {
    collect_all_workflows_reporting(registry).entries
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

/// `GET /api/workflows` query params.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
pub struct WorkflowsQuery {
    /// `?with_skipped=1` opts into the `{entries, skipped}` envelope
    /// ([`WorkflowsAggregateDto`]) naming which registered workspaces this
    /// aggregate could not fully report on, and why. Defaults to `false`,
    /// returning the plain `entries` array — byte-identical to the pre-A9
    /// v0.23 response, since `bastion-web/lib/workflows.ts` reads that shape
    /// today. Unlike `/api/runs`'s `?with_repo=1`, this flag is NOT a cost
    /// gate — skip detection is bookkeeping over the walk this handler
    /// already performs, adding no second traversal; the gate exists solely
    /// to keep the default response non-breaking. See serve-api §11.6.
    #[serde(default, deserialize_with = "bool_flag_from_str")]
    pub with_skipped: bool,
}

/// `GET /api/workflows` — cross-repo flow-state aggregate over every
/// registered workspace (A2). Shaped like [`list_repos`]: wraps a pure
/// collector in `web::block`, returning 200 with the resulting (possibly
/// empty) body, or a 500 on thread-pool failure.
///
/// Without `?with_skipped=1` (default), returns the bare `entries` array via
/// [`collect_all_workflows`] — byte-identical to v0.23. With
/// `?with_skipped=1`, returns the `{entries, skipped}` envelope via
/// [`collect_all_workflows_reporting`], naming every registered workspace
/// whose flow-state report is incomplete and why (ask A9). Both paths walk
/// the registry exactly once; the flag only changes what is returned, not
/// what is scanned.
///
/// Never 404s — an empty/absent registry degrades to an empty body in
/// either shape, matching the underlying collectors' own degrade-gracefully
/// contract.
pub async fn list_all_workflows(
    query: web::Query<WorkflowsQuery>,
    registry: web::Data<FileConfig>,
) -> HttpResponse {
    if query.with_skipped {
        match web::block(move || collect_all_workflows_reporting(&registry)).await {
            Ok(aggregate) => HttpResponse::Ok().json(aggregate),
            Err(err) => blocking_error_response(err),
        }
    } else {
        match web::block(move || collect_all_workflows(&registry)).await {
            Ok(list) => HttpResponse::Ok().json(list),
            Err(err) => blocking_error_response(err),
        }
    }
}

/// `GET /api/repos/status` query params.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RepoStatusAggregateQuery {
    /// `?names=a,b,c` scopes the walk to those registered-workspace names
    /// instead of the full registry. A name repeated in the list is read
    /// once ([`dedupe_names`]); a name not found in the registry lands in
    /// `skipped[]` rather than 404ing the whole request. Absent/empty
    /// walks every registered workspace, same as `GET /api/workflows`
    /// with no query param.
    #[serde(default)]
    pub names: Option<String>,
}

/// Parse `?names=a,b,c` into the deduplicated, order-preserved list
/// [`collect_all_repo_status_reporting`] expects, splitting on `,` and
/// discarding empty segments (so `?names=a,,b` and `?names=a,b,` both work
/// as `["a", "b"]`, and a bare `?names=` is treated as absent rather than a
/// single empty-string name).
fn parse_names_query(raw: &str) -> Option<Vec<String>> {
    let names: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if names.is_empty() { None } else { Some(names) }
}

/// `GET /api/repos/status` — cross-repo `status.md` aggregate over every
/// registered workspace (BA.ticket.batch-repo-status). Shaped like
/// [`list_all_workflows`]: wraps a pure collector in `web::block`, returning
/// 200 with the resulting `{entries, skipped}` envelope, or a 500 on
/// thread-pool failure.
///
/// Never 404s — an empty/absent registry, or an all-unregistered `?names=`
/// list, degrades to `{entries: [], skipped: [...]}`/`{entries: [],
/// skipped: []}` rather than failing the whole request, matching
/// `GET /api/workflows`'s degrade-gracefully contract.
///
/// This route carries no `?with_skipped=1`-style opt-in — unlike `GET
/// /api/workflows`, it has no existing consumer to keep byte-identical, so
/// the honest `{entries, skipped}` envelope is the unconditional default.
pub async fn list_all_repo_status(
    query: web::Query<RepoStatusAggregateQuery>,
    registry: web::Data<FileConfig>,
) -> HttpResponse {
    let names = query.names.as_deref().and_then(parse_names_query);

    match web::block(move || collect_all_repo_status_reporting(&registry, names.as_deref())).await {
        Ok(aggregate) => HttpResponse::Ok().json(aggregate),
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

    // ── collect_all_repo_status_reporting ───────────────────────────────
    //
    // D68 evidence: these tests were written against a collector that did
    // not exist yet (renamed to `..._DISABLED` to force the failure), run
    // via `cargo build --lib --tests`, and observed to fail to compile with
    // (one instance of several identical `E0425`s, one per call site):
    //
    //   error[E0425]: cannot find function `collect_all_repo_status_reporting` in this scope
    //      --> src/serve/handlers/status.rs:575:30
    //       |
    //   575 |     match web::block(move || collect_all_repo_status_reporting(&registry, names.as_deref())).await
    //       |                              ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    //
    // Full text recorded in planning/BA.ticket.batch-repo-status/tasks.md
    // ## Notes. The collector was then implemented and this suite passes.

    const STATUS_MALFORMED: &str = include_str!("../status/fixtures/status_malformed.md");

    fn multi_registry(entries: &[(&str, &Path)]) -> FileConfig {
        let mut workspaces = HashMap::new();
        for (name, root) in entries {
            workspaces.insert((*name).to_string(), (*root).to_path_buf());
        }
        FileConfig {
            workspaces: Some(workspaces),
            ..Default::default()
        }
    }

    #[test]
    fn collect_all_repo_status_reporting_empty_registry_returns_empty_envelope() {
        let registry = FileConfig::default();
        let out = collect_all_repo_status_reporting(&registry, None);
        assert!(out.entries.is_empty());
        assert!(out.skipped.is_empty());
    }

    #[test]
    fn collect_all_repo_status_reporting_all_good_returns_all_entries_no_skips() {
        let tmp_a = TempDir::new();
        write(&tmp_a.path().join("planning/status.md"), STATUS_MD);
        let tmp_b = TempDir::new();
        write(&tmp_b.path().join("planning/status.md"), STATUS_MD);
        let registry = multi_registry(&[("alpha", tmp_a.path()), ("beta", tmp_b.path())]);

        let out = collect_all_repo_status_reporting(&registry, None);
        assert_eq!(out.entries.len(), 2);
        assert!(out.skipped.is_empty());
        let names: Vec<&str> = out.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta"]);
    }

    #[test]
    fn collect_all_repo_status_reporting_one_malformed_is_skipped_not_entry() {
        let tmp_good = TempDir::new();
        write(&tmp_good.path().join("planning/status.md"), STATUS_MD);
        let tmp_bad = TempDir::new();
        write(&tmp_bad.path().join("planning/status.md"), STATUS_MALFORMED);
        let registry = multi_registry(&[("good", tmp_good.path()), ("bad", tmp_bad.path())]);

        let out = collect_all_repo_status_reporting(&registry, None);
        assert_eq!(out.entries.len(), 1);
        assert_eq!(out.entries[0].name, "good");
        assert_eq!(out.skipped.len(), 1);
        assert_eq!(out.skipped[0].repo, "bad");
        assert_eq!(out.skipped[0].reason, "missing_or_malformed_status");
    }

    #[test]
    fn collect_all_repo_status_reporting_one_missing_is_skipped_not_entry() {
        let tmp_good = TempDir::new();
        write(&tmp_good.path().join("planning/status.md"), STATUS_MD);
        let tmp_missing = TempDir::new();
        let registry =
            multi_registry(&[("good", tmp_good.path()), ("missing", tmp_missing.path())]);

        let out = collect_all_repo_status_reporting(&registry, None);
        assert_eq!(out.entries.len(), 1);
        assert_eq!(out.skipped.len(), 1);
        assert_eq!(out.skipped[0].repo, "missing");
        assert_eq!(out.skipped[0].reason, "missing_or_malformed_status");
    }

    #[test]
    fn collect_all_repo_status_reporting_all_missing_returns_no_entries_all_skipped() {
        let tmp_a = TempDir::new();
        let tmp_b = TempDir::new();
        let registry = multi_registry(&[("a", tmp_a.path()), ("b", tmp_b.path())]);

        let out = collect_all_repo_status_reporting(&registry, None);
        assert!(out.entries.is_empty());
        assert_eq!(out.skipped.len(), 2);
    }

    #[test]
    fn collect_all_repo_status_reporting_entries_plus_skipped_equals_covered_count() {
        let tmp_a = TempDir::new();
        write(&tmp_a.path().join("planning/status.md"), STATUS_MD);
        let tmp_b = TempDir::new();
        write(&tmp_b.path().join("planning/status.md"), STATUS_MALFORMED);
        let tmp_c = TempDir::new();
        let registry = multi_registry(&[
            ("a", tmp_a.path()),
            ("b", tmp_b.path()),
            ("c", tmp_c.path()),
        ]);

        let out = collect_all_repo_status_reporting(&registry, None);
        // Asserted directly as a property, not inferred from a happy-path
        // length — two wrong sub-counts could otherwise cancel into a
        // plausible-looking total.
        assert_eq!(out.entries.len() + out.skipped.len(), 3);
    }

    #[test]
    fn collect_all_repo_status_reporting_names_scopes_the_walk() {
        let tmp_a = TempDir::new();
        write(&tmp_a.path().join("planning/status.md"), STATUS_MD);
        let tmp_b = TempDir::new();
        write(&tmp_b.path().join("planning/status.md"), STATUS_MD);
        let registry = multi_registry(&[("alpha", tmp_a.path()), ("beta", tmp_b.path())]);

        let names = vec!["alpha".to_string()];
        let out = collect_all_repo_status_reporting(&registry, Some(&names));
        assert_eq!(out.entries.len(), 1);
        assert_eq!(out.entries[0].name, "alpha");
        assert!(out.skipped.is_empty());
    }

    #[test]
    fn collect_all_repo_status_reporting_names_duplicate_is_read_once() {
        let tmp_a = TempDir::new();
        write(&tmp_a.path().join("planning/status.md"), STATUS_MD);
        let registry = multi_registry(&[("alpha", tmp_a.path())]);

        let names = vec![
            "alpha".to_string(),
            "alpha".to_string(),
            "alpha".to_string(),
        ];
        let out = collect_all_repo_status_reporting(&registry, Some(&names));
        assert_eq!(
            out.entries.len(),
            1,
            "duplicate name must be read exactly once"
        );
        assert!(out.skipped.is_empty());
    }

    #[test]
    fn collect_all_repo_status_reporting_names_unregistered_name_lands_in_skipped() {
        let tmp_a = TempDir::new();
        write(&tmp_a.path().join("planning/status.md"), STATUS_MD);
        let registry = multi_registry(&[("alpha", tmp_a.path())]);

        let names = vec!["alpha".to_string(), "no-such-repo".to_string()];
        let out = collect_all_repo_status_reporting(&registry, Some(&names));
        assert_eq!(out.entries.len(), 1);
        assert_eq!(out.skipped.len(), 1);
        assert_eq!(out.skipped[0].repo, "no-such-repo");
        assert_eq!(out.skipped[0].reason, "unregistered_workspace");
    }

    #[test]
    fn collect_all_repo_status_reporting_read_repo_status_is_unchanged_behaviour() {
        // read_repo_status is reused unchanged: same fixture, same result
        // whether called directly or through the aggregate collector.
        let tmp = TempDir::new();
        write(&tmp.path().join("planning/status.md"), STATUS_MD);
        let direct = read_repo_status("solo", tmp.path()).expect("direct call should parse");

        let registry = multi_registry(&[("solo", tmp.path())]);
        let out = collect_all_repo_status_reporting(&registry, None);
        assert_eq!(out.entries.len(), 1);
        assert_eq!(out.entries[0], direct);
    }

    // ── parse_names_query ───────────────────────────────────────────────

    #[test]
    fn parse_names_query_splits_on_comma() {
        assert_eq!(
            parse_names_query("a,b,c"),
            Some(vec!["a".to_string(), "b".to_string(), "c".to_string()])
        );
    }

    #[test]
    fn parse_names_query_discards_empty_segments() {
        assert_eq!(
            parse_names_query("a,,b,"),
            Some(vec!["a".to_string(), "b".to_string()])
        );
    }

    #[test]
    fn parse_names_query_bare_empty_string_is_none() {
        assert_eq!(parse_names_query(""), None);
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

    // ── scan_flow_states / collect_all_workflows_reporting (A9) ───────────

    #[test]
    fn scan_flow_states_unreadable_root_reports_unreadable_root() {
        let root = PathBuf::from("/nonexistent/path/should/not/exist");
        let (states, reason) = scan_flow_states(&root);
        assert_eq!(states, Vec::new());
        assert_eq!(reason, Some(SkipReason::UnreadableRoot));
    }

    #[test]
    fn scan_flow_states_missing_planning_dir_reports_no_planning_dir() {
        let tmp = TempDir::new();
        let (states, reason) = scan_flow_states(tmp.path());
        assert_eq!(states, Vec::new());
        assert_eq!(reason, Some(SkipReason::NoPlanningDir));
    }

    #[test]
    fn scan_flow_states_empty_but_healthy_planning_dir_reports_no_reason() {
        let tmp = TempDir::new();
        std::fs::create_dir_all(tmp.path().join("planning")).unwrap();
        let (states, reason) = scan_flow_states(tmp.path());
        assert_eq!(states, Vec::new());
        assert_eq!(reason, None);
    }

    #[test]
    fn scan_flow_states_malformed_only_reports_malformed_flow_state() {
        let tmp = TempDir::new();
        write(
            &tmp.path()
                .join("planning/bad-spec/sdlc/sdlc-flow-state.json"),
            "{ not json",
        );

        let (states, reason) = scan_flow_states(tmp.path());
        assert_eq!(states, Vec::new());
        assert_eq!(reason, Some(SkipReason::MalformedFlowState));
    }

    #[test]
    fn scan_flow_states_mixed_valid_and_malformed_keeps_valid_and_reports_reason() {
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

        let (states, reason) = scan_flow_states(tmp.path());
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].spec_slug, "phase6-blockA");
        assert_eq!(reason, Some(SkipReason::MalformedFlowState));
    }

    #[test]
    fn skip_reason_as_wire_matches_the_three_value_vocabulary() {
        assert_eq!(SkipReason::UnreadableRoot.as_wire(), "unreadable_root");
        assert_eq!(SkipReason::NoPlanningDir.as_wire(), "no_planning_dir");
        assert_eq!(
            SkipReason::MalformedFlowState.as_wire(),
            "malformed_flow_state"
        );
    }

    #[test]
    fn collect_all_workflows_reporting_empty_registry_returns_empty_aggregate() {
        let registry = FileConfig::default();
        let aggregate = collect_all_workflows_reporting(&registry);
        assert_eq!(aggregate.entries, Vec::new());
        assert_eq!(aggregate.skipped, Vec::new());
    }

    #[test]
    fn collect_all_workflows_reporting_healthy_empty_repo_is_not_skipped() {
        let tmp = TempDir::new();
        std::fs::create_dir_all(tmp.path().join("planning")).unwrap();
        let registry = registry_with("healthy-empty", tmp.path());

        let aggregate = collect_all_workflows_reporting(&registry);
        assert_eq!(aggregate.entries, Vec::new());
        assert_eq!(
            aggregate.skipped,
            Vec::new(),
            "a readable planning/ dir with zero flow states must not be reported as skipped"
        );
    }

    #[test]
    fn collect_all_workflows_reporting_unreadable_root_is_reported() {
        let mut workspaces = HashMap::new();
        workspaces.insert(
            "ghost-repo".to_string(),
            PathBuf::from("/nonexistent/path/should/not/exist"),
        );
        let registry = FileConfig {
            workspaces: Some(workspaces),
            ..Default::default()
        };

        let aggregate = collect_all_workflows_reporting(&registry);
        assert_eq!(aggregate.entries, Vec::new());
        assert_eq!(aggregate.skipped.len(), 1);
        assert_eq!(aggregate.skipped[0].repo, "ghost-repo");
        assert_eq!(aggregate.skipped[0].reason, "unreadable_root");
    }

    #[test]
    fn collect_all_workflows_reporting_no_planning_dir_is_reported() {
        let tmp = TempDir::new();
        let registry = registry_with("no-planning-repo", tmp.path());

        let aggregate = collect_all_workflows_reporting(&registry);
        assert_eq!(aggregate.entries, Vec::new());
        assert_eq!(aggregate.skipped.len(), 1);
        assert_eq!(aggregate.skipped[0].repo, "no-planning-repo");
        assert_eq!(aggregate.skipped[0].reason, "no_planning_dir");
    }

    #[test]
    fn collect_all_workflows_reporting_malformed_repo_still_contributes_parsed_entries() {
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
        let registry = registry_with("mixed-repo", tmp.path());

        let aggregate = collect_all_workflows_reporting(&registry);
        assert_eq!(aggregate.entries.len(), 1);
        assert_eq!(aggregate.entries[0].repo, "mixed-repo");
        assert_eq!(aggregate.entries[0].spec_slug, "phase6-blockA");
        assert_eq!(aggregate.skipped.len(), 1);
        assert_eq!(aggregate.skipped[0].repo, "mixed-repo");
        assert_eq!(aggregate.skipped[0].reason, "malformed_flow_state");
    }

    #[test]
    fn collect_all_workflows_reporting_malformed_repo_does_not_suppress_sibling_repo() {
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

        let aggregate = collect_all_workflows_reporting(&registry);
        assert_eq!(aggregate.entries.len(), 1);
        assert_eq!(aggregate.entries[0].repo, "good-repo");
        assert_eq!(aggregate.skipped.len(), 1);
        assert_eq!(aggregate.skipped[0].repo, "bad-repo");
        assert_eq!(aggregate.skipped[0].reason, "malformed_flow_state");
    }

    #[test]
    fn collect_all_workflows_reporting_skipped_is_ordered_by_repo_across_multi_repo_registry() {
        let tmp_ghost = PathBuf::from("/nonexistent/path/should/not/exist");
        let tmp_no_planning = TempDir::new();
        let tmp_healthy = TempDir::new();
        write(
            &tmp_healthy
                .path()
                .join("planning/good-spec/sdlc/sdlc-flow-state.json"),
            FLOW_JSON,
        );

        let mut workspaces = HashMap::new();
        workspaces.insert("zeta-ghost".to_string(), tmp_ghost);
        workspaces.insert(
            "beta-no-planning".to_string(),
            tmp_no_planning.path().to_path_buf(),
        );
        workspaces.insert(
            "alpha-healthy".to_string(),
            tmp_healthy.path().to_path_buf(),
        );
        let registry = FileConfig {
            workspaces: Some(workspaces),
            ..Default::default()
        };

        let aggregate = collect_all_workflows_reporting(&registry);
        let skipped_repos: Vec<&str> = aggregate.skipped.iter().map(|s| s.repo.as_str()).collect();
        assert_eq!(skipped_repos, vec!["beta-no-planning", "zeta-ghost"]);
        assert_eq!(aggregate.entries.len(), 1);
        assert_eq!(aggregate.entries[0].repo, "alpha-healthy");
    }

    #[test]
    fn collect_all_workflows_wrapper_matches_reporting_entries() {
        let tmp = TempDir::new();
        write(
            &tmp.path()
                .join("planning/phase6-blockA/sdlc/sdlc-flow-state.json"),
            FLOW_JSON,
        );
        let registry = registry_with("wrapper-repo", tmp.path());

        assert_eq!(
            collect_all_workflows(&registry),
            collect_all_workflows_reporting(&registry).entries
        );
    }

    // ── list_all_workflows handler: ?with_skipped=1 gate ───────────────────

    #[actix_web::test]
    async fn list_all_workflows_without_with_skipped_returns_bare_array() {
        let tmp = TempDir::new();
        write(
            &tmp.path()
                .join("planning/phase6-blockA/sdlc/sdlc-flow-state.json"),
            FLOW_JSON,
        );
        let registry = registry_with("repo-a", tmp.path());

        let resp = list_all_workflows(
            web::Query(WorkflowsQuery {
                with_skipped: false,
            }),
            web::Data::new(registry),
        )
        .await;

        let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            value.is_array(),
            "default response must be a bare array, got: {value}"
        );
        let entries = value.as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["repo"], "repo-a");
    }

    #[actix_web::test]
    async fn list_all_workflows_with_with_skipped_returns_two_key_object() {
        let mut workspaces = HashMap::new();
        workspaces.insert(
            "ghost-repo".to_string(),
            PathBuf::from("/nonexistent/path/should/not/exist"),
        );
        let registry = FileConfig {
            workspaces: Some(workspaces),
            ..Default::default()
        };

        let resp = list_all_workflows(
            web::Query(WorkflowsQuery { with_skipped: true }),
            web::Data::new(registry),
        )
        .await;

        let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let obj = value
            .as_object()
            .expect("with_skipped=1 must return an object");
        assert_eq!(obj.len(), 2);
        assert!(obj.contains_key("entries"));
        assert!(obj.contains_key("skipped"));
        let skipped = obj["skipped"].as_array().expect("skipped must be an array");
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0]["repo"], "ghost-repo");
        assert_eq!(skipped[0]["reason"], "unreadable_root");
    }

    // ── get_repo_status regression: unchanged by the aggregate existing ────
    //
    // BA.ticket.batch-repo-status task 3: the aggregate and this handler now
    // share `read_repo_status`. The failure mode this guards against is
    // someone "improving" that shared function to degrade gracefully so the
    // aggregate's `skipped[]` looks tidier — which would silently turn this
    // route's 404-on-malformed contract into a 200 with a half-populated
    // body. Pin both branches directly through the handler, not just through
    // `read_repo_status` in isolation.

    #[actix_web::test]
    async fn get_repo_status_malformed_status_md_still_404s_with_c002() {
        let tmp = TempDir::new();
        // Missing status.md is one of the two "malformed or missing" cases
        // `read_repo_status` answers with `None` for; present here as an
        // empty planning/ dir (no status.md at all).
        let registry = registry_with("repo-ghost", tmp.path());

        let resp = get_repo_status(
            web::Path::from("repo-ghost".to_string()),
            web::Data::new(registry),
        )
        .await;

        assert_eq!(resp.status(), actix_web::http::StatusCode::NOT_FOUND);
        let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["code"], "C002");
    }

    #[actix_web::test]
    async fn get_repo_status_good_repo_returns_unchanged_200_body() {
        let tmp = TempDir::new();
        write(&tmp.path().join("planning/status.md"), STATUS_MD);
        write(&tmp.path().join("planning/handoff.md"), HANDOFF_MD);
        let registry = registry_with("repo-good", tmp.path());

        let resp = get_repo_status(
            web::Path::from("repo-good".to_string()),
            web::Data::new(registry),
        )
        .await;

        assert_eq!(resp.status(), actix_web::http::StatusCode::OK);
        let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // Same fixture and same fields task 1 recorded from the live server
        // capture (see spec `## Notes`): name, has_handoff, momentum_next.
        assert_eq!(value["name"], "repo-good");
        assert_eq!(value["has_handoff"], true);
        assert_eq!(value["momentum_next"], "Wire WS event push");
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
