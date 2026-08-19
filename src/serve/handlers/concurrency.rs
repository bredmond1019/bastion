//! Fleet-scoped concurrency-slot REST handler for `bastion serve` (`BA.19.D`).
//!
//! Read-only (D25) — this route never mutates any `.fleet-locks` entry, and
//! it never sweeps stale entries the way `fleet_concurrency_check.py`'s
//! `status` action can. It is a pure pass-through over
//! `mev::brain::availability`'s live heavy-lane registry read
//! (`compute_fleet_slot_view` / `heavy_category`): **serve computes nothing
//! about lane liveness or heavy-category classification itself**, exactly
//! like `handlers/lanes.rs` over `mev::lanes_brain`.
//!
//! # Route
//! - `GET /api/concurrency[?repo=<slug>]` — one row per category the
//!   registry knows about (`native-build`, `browser-automation`), each
//!   carrying its cap, live-hold count, sorted live-repo names, and
//!   remaining slots. `?repo=<slug>` adds a per-repo `repo` object answering
//!   whether that repo may start a heavy lane right now. A blank
//!   `?repo=` is not silently ignored (mirrors `lanes.rs`'s `?epic=`
//!   convention), and an unknown repo slug is a 404/`C005` via
//!   [`board::epic_error_response`]'s sibling error path — the same
//!   uniform "no such registry entry" shape, not a hand-rolled one.
//!
//! # Pure core vs I/O shell (Rule 6)
//! [`category_to_dto`] and [`repo_to_dto`] are pure — unit-tested directly,
//! no filesystem access. [`get_concurrency`] is the thin async handler: it
//! resolves a starting path from the shared [`FileConfig`] registry, walks
//! up to the brain root (`mev::brain::config::find_brain_root`), then under
//! `web::block` calls `mev::brain::availability::compute_fleet_slot_view`
//! plus, when `?repo=<slug>` is present and non-blank, resolves that repo's
//! root from the brain config's `[[repos]]` registry (the same pattern
//! `handlers/attention.rs::build_attention` uses for `repo_path`) and calls
//! `mev::brain::availability::heavy_category` on it.
//!
//! # Error mapping
//! - Brain root unresolvable (no `brain.toml` walking up from the workspace
//!   root) → 500 + `C010` via [`board::brain_root_error_response`].
//! - `?repo=` present but blank, or present and not found in the brain's
//!   `[[repos]]` registry → 404 + `C005` via [`board::epic_error_response`].
//! - `web::block` thread-pool failure → 500 + `C010` via
//!   [`board::blocking_error_response`].
//! - An unreadable/missing `.fleet-locks` directory is NOT an error — mev
//!   reports it as `degraded: true`, and this handler serves it as a 200
//!   with `degraded: true` plus a reason, never a hard failure and never
//!   `allowed: false`.

use std::path::PathBuf;

use actix_web::{HttpResponse, web};
use serde::Deserialize;

use crate::config::{FileConfig, resolve_workspace_root};
use crate::serve::dto::{ConcurrencyCategoryDto, ConcurrencyDto, ConcurrencyRepoDto};
use crate::serve::handlers::board;

use mev::brain::availability::{
    FleetSlotView, category_capacity, compute_fleet_slot_view, heavy_category,
};
use mev::brain::config::{find_brain_root, load_brain_config};

/// Reason text used when the fleet-lock read degraded (directory missing or
/// unreadable). Mirrors `fleet_concurrency_check.py`'s degrade-to-advisory
/// message.
const DEGRADED_REASON: &str = "fleet-lock registry unreadable; degrading to advisory (allowed)";

// ── Pure core ────────────────────────────────────────────────────────────────

/// Map one category's `(name, cap, live repo names)` onto the wire
/// [`ConcurrencyCategoryDto`]. `active_repos` is sorted by the caller
/// (`FleetSlotView::live_repos` already sorts, but this fn re-sorts
/// defensively so it never depends on that invariant holding). `cap -
/// active_count` saturates at zero — the Python `register` honours
/// `--max-heavy-lanes`, which can push a registry over the documented cap.
fn category_to_dto(
    category: &str,
    cap: usize,
    mut active_repos: Vec<String>,
) -> ConcurrencyCategoryDto {
    active_repos.sort();
    let active_count = active_repos.len();
    ConcurrencyCategoryDto {
        category: category.to_owned(),
        cap,
        active_count,
        active_repos,
        slots_available: cap.saturating_sub(active_count),
    }
}

/// Build the full sorted `categories[]` list for a [`FleetSlotView`] — one
/// row per [`FleetSlotView::known_categories`] entry.
fn categories_to_dtos(view: &FleetSlotView) -> Vec<ConcurrencyCategoryDto> {
    view.known_categories()
        .into_iter()
        .map(|category| {
            let cap = category_capacity(&category);
            let live_repos = view.live_repos(&category);
            category_to_dto(&category, cap, live_repos)
        })
        .collect()
}

/// Build the `?repo=<slug>` answer for a repo whose heavy classification is
/// `category` (`None` = light), given the fleet's `categories` view and
/// whether the read is `degraded`.
///
/// - A light repo (`category == None`) is always `allowed: true`.
/// - A degraded read is always `allowed: true, degraded: true` with
///   [`DEGRADED_REASON`] — never `allowed: false`, matching
///   `fleet_concurrency_check.py`'s degrade-to-advisory contract.
/// - A heavy repo whose category is at or over capacity is
///   `allowed: false`, with a reason naming the holding repos.
/// - Otherwise (heavy, below capacity, not degraded) is `allowed: true`.
fn repo_to_dto(
    repo: &str,
    category: Option<&str>,
    categories: &[ConcurrencyCategoryDto],
    degraded: bool,
) -> ConcurrencyRepoDto {
    if degraded {
        return ConcurrencyRepoDto {
            repo: repo.to_owned(),
            category: category.map(str::to_owned),
            allowed: true,
            degraded: true,
            reason: Some(DEGRADED_REASON.to_owned()),
        };
    }

    let Some(category) = category else {
        return ConcurrencyRepoDto {
            repo: repo.to_owned(),
            category: None,
            allowed: true,
            degraded: false,
            reason: None,
        };
    };

    // Fall back to the browser-automation cap for a category the registry
    // itself has never heard of (defensive — `heavy_category` and
    // `known_categories` share the same vocabulary today, but the wire
    // contract should not silently break if that ever drifts).
    let row = categories
        .iter()
        .find(|c| c.category == category)
        .cloned()
        .unwrap_or_else(|| category_to_dto(category, category_capacity(category), Vec::new()));

    if row.slots_available == 0 {
        let holders = if row.active_repos.is_empty() {
            "no holders recorded".to_owned()
        } else {
            row.active_repos.join(", ")
        };
        return ConcurrencyRepoDto {
            repo: repo.to_owned(),
            category: Some(category.to_owned()),
            allowed: false,
            degraded: false,
            reason: Some(format!(
                "category '{category}' is at capacity ({}/{}), held by: {holders}",
                row.active_count, row.cap
            )),
        };
    }

    ConcurrencyRepoDto {
        repo: repo.to_owned(),
        category: Some(category.to_owned()),
        allowed: true,
        degraded: false,
        reason: None,
    }
}

// ── I/O shell ──────────────────────────────────────────────────────────────────

/// `GET /api/concurrency` query params. `repo=<slug>` is optional; when
/// present and non-blank it adds a per-repo answer to the response. A
/// present-but-blank value is treated as an error (not silently ignored),
/// matching `lanes.rs`'s `?epic=` convention.
#[derive(Debug, Deserialize)]
pub struct ConcurrencyQuery {
    #[serde(default)]
    pub repo: Option<String>,
}

/// The two error shapes [`get_concurrency`]'s `web::block` closure can fail
/// with: an operator-configuration brain-root problem (500/`C010`), or —
/// only when `?repo=<slug>` is present — a slug absent from the brain's
/// `[[repos]]` registry (404/`C005`). The present-but-blank `repo=` case is
/// checked synchronously before the closure ever runs, so it isn't a
/// variant here.
enum ConcurrencyError {
    BrainRoot(String),
    UnknownRepo(String),
}

/// `GET /api/concurrency[?repo=<slug>]` — one row per known heavy-lane
/// category, plus an optional per-repo answer, pass-through over
/// `mev::brain::availability` (`BA.19.D`).
///
/// Bearer auth is inherited from the `/api` scope's `BearerAuthMiddleware` —
/// a request without a valid token never reaches this handler (401
/// upstream).
pub async fn get_concurrency(
    query: web::Query<ConcurrencyQuery>,
    registry: web::Data<FileConfig>,
) -> HttpResponse {
    let repo_slug = query.into_inner().repo;

    if let Some(raw) = repo_slug.as_deref()
        && raw.trim().is_empty()
    {
        return board::epic_error_response(
            "?repo=<slug> must be non-empty when the query param is present",
        );
    }

    let start: PathBuf =
        resolve_workspace_root(None, None, &registry).unwrap_or_else(|_| PathBuf::from("."));

    match web::block(move || -> Result<ConcurrencyDto, ConcurrencyError> {
        let root = find_brain_root(&start).map_err(|e| {
            ConcurrencyError::BrainRoot(format!(
                "could not resolve brain root from {}: {e}",
                start.display()
            ))
        })?;

        let view = compute_fleet_slot_view(&root);
        let categories = categories_to_dtos(&view);

        let repo = match repo_slug {
            None => None,
            Some(slug) => {
                let config = load_brain_config(&root.join("brain.toml")).map_err(|e| {
                    ConcurrencyError::BrainRoot(format!(
                        "could not load brain.toml at {}: {e}",
                        root.display()
                    ))
                })?;
                let entry = config
                    .repos
                    .iter()
                    .find(|r| r.slug == slug)
                    .ok_or_else(|| {
                        ConcurrencyError::UnknownRepo(format!("unknown repo: {slug}"))
                    })?;
                let repo_root = if entry.repo_path == "." || entry.repo_path.is_empty() {
                    root.clone()
                } else {
                    root.join(&entry.repo_path)
                };
                let category = heavy_category(&repo_root);
                Some(repo_to_dto(
                    &slug,
                    category.as_deref(),
                    &categories,
                    view.degraded,
                ))
            }
        };

        Ok(ConcurrencyDto {
            degraded: view.degraded,
            reason: if view.degraded {
                Some(DEGRADED_REASON.to_owned())
            } else {
                None
            },
            categories,
            repo,
        })
    })
    .await
    {
        Ok(Ok(dto)) => HttpResponse::Ok().json(dto),
        Ok(Err(ConcurrencyError::BrainRoot(msg))) => board::brain_root_error_response(msg),
        Ok(Err(ConcurrencyError::UnknownRepo(msg))) => board::epic_error_response(msg),
        Err(err) => board::blocking_error_response(err),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── category_to_dto ──────────────────────────────────────────────────────

    #[test]
    fn category_to_dto_sorts_active_repos_and_computes_slots_available() {
        let dto = category_to_dto(
            "native-build",
            4,
            vec!["mev".to_owned(), "bastion".to_owned()],
        );
        assert_eq!(dto.category, "native-build");
        assert_eq!(dto.cap, 4);
        assert_eq!(dto.active_count, 2);
        assert_eq!(
            dto.active_repos,
            vec!["bastion".to_owned(), "mev".to_owned()]
        );
        assert_eq!(dto.slots_available, 2);
    }

    #[test]
    fn category_to_dto_at_capacity_reports_zero_slots() {
        let dto = category_to_dto(
            "browser-automation",
            2,
            vec!["price-scout".to_owned(), "amistad".to_owned()],
        );
        assert_eq!(dto.slots_available, 0);
    }

    #[test]
    fn category_to_dto_over_capacity_saturates_at_zero_not_underflow() {
        let dto = category_to_dto(
            "browser-automation",
            2,
            vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
        );
        assert_eq!(dto.active_count, 3);
        assert_eq!(dto.cap, 2);
        assert_eq!(dto.slots_available, 0);
    }

    #[test]
    fn category_to_dto_zero_holds_reports_full_capacity() {
        let dto = category_to_dto("native-build", 4, Vec::new());
        assert_eq!(dto.active_count, 0);
        assert_eq!(dto.slots_available, 4);
        assert!(dto.active_repos.is_empty());
    }

    // ── repo_to_dto ──────────────────────────────────────────────────────────

    #[test]
    fn repo_to_dto_light_repo_is_always_allowed() {
        let dto = repo_to_dto("learn-ai", None, &[], false);
        assert_eq!(dto.category, None);
        assert!(dto.allowed);
        assert!(!dto.degraded);
        assert_eq!(dto.reason, None);
    }

    #[test]
    fn repo_to_dto_heavy_repo_below_capacity_is_allowed() {
        let categories = vec![category_to_dto("native-build", 4, vec!["mev".to_owned()])];
        let dto = repo_to_dto("bastion", Some("native-build"), &categories, false);
        assert!(dto.allowed);
        assert_eq!(dto.reason, None);
        assert_eq!(dto.category.as_deref(), Some("native-build"));
    }

    #[test]
    fn repo_to_dto_heavy_repo_at_capacity_is_disallowed_with_holders_named() {
        let categories = vec![category_to_dto(
            "browser-automation",
            2,
            vec!["amistad".to_owned(), "price-scout".to_owned()],
        )];
        let dto = repo_to_dto(
            "new-heavy-repo",
            Some("browser-automation"),
            &categories,
            false,
        );
        assert!(!dto.allowed);
        let reason = dto.reason.expect("reason must be set when disallowed");
        assert!(reason.contains("amistad"));
        assert!(reason.contains("price-scout"));
        assert!(reason.contains("2/2"));
    }

    #[test]
    fn repo_to_dto_degraded_view_is_always_allowed_never_false() {
        let categories = vec![category_to_dto(
            "native-build",
            4,
            vec![
                "a".to_owned(),
                "b".to_owned(),
                "c".to_owned(),
                "d".to_owned(),
            ],
        )];
        let dto = repo_to_dto("bastion", Some("native-build"), &categories, true);
        assert!(
            dto.allowed,
            "degraded read must never report allowed: false"
        );
        assert!(dto.degraded);
        assert!(dto.reason.is_some());
    }

    #[test]
    fn repo_to_dto_unknown_category_falls_back_to_its_own_capacity() {
        // Not in `categories[]` at all -> falls back to category_capacity's
        // documented default (2, the browser-automation cap) with no holders.
        let dto = repo_to_dto("odd-repo", Some("some-future-category"), &[], false);
        assert!(dto.allowed);
        assert_eq!(dto.category.as_deref(), Some("some-future-category"));
    }

    // ── categories_to_dtos ───────────────────────────────────────────────────

    #[test]
    fn categories_to_dtos_covers_every_known_category() {
        let view = compute_fleet_slot_view(std::path::Path::new("/definitely/does/not/exist"));
        assert!(view.degraded);
        let dtos = categories_to_dtos(&view);
        let names: Vec<&str> = dtos.iter().map(|d| d.category.as_str()).collect();
        assert!(names.contains(&"native-build"));
        assert!(names.contains(&"browser-automation"));
    }

    // ── live-registry cases, through mev::brain::availability::compute_fleet_slot_view ─
    //
    // These drive the real registry reader against a temp `.fleet-locks`
    // fixture rather than restating mev's staleness rules bastion-side, per
    // task 4's instruction — this keeps the suite honest about what mev
    // actually decides.

    /// Write one raw `.fleet-locks/<repo>__<label>.json` entry, mirroring
    /// `fleet_concurrency_check.py`'s on-disk shape (and mev's own test
    /// fixture helper of the same name/shape).
    fn write_lock_entry(
        root: &std::path::Path,
        repo: &str,
        category: &str,
        pid: i64,
        started_at: f64,
        label: &str,
    ) {
        let dir = root.join(".fleet-locks");
        std::fs::create_dir_all(&dir).unwrap();
        let json = serde_json::json!({
            "repo": repo,
            "pid": pid,
            "category": category,
            "started_at": started_at,
        });
        std::fs::write(
            dir.join(format!("{repo}__{label}.json")),
            serde_json::to_string(&json).unwrap(),
        )
        .unwrap();
    }

    fn now_unix_seconds() -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
    }

    #[test]
    fn categories_to_dtos_over_a_live_fixture_excludes_stale_and_dead_pid_entries() {
        let root = crate::testsupport::unique_temp_dir("bastion-concurrency-live-fixture");
        let now = now_unix_seconds();

        // (a) a genuinely live entry — this process's own pid, fresh started_at.
        write_lock_entry(
            &root,
            "repo-live",
            "native-build",
            std::process::id() as i64,
            now,
            "p",
        );
        // (b) a stale-by-ttl entry — alive pid, started_at well past the 4h TTL.
        write_lock_entry(
            &root,
            "repo-stale-ttl",
            "native-build",
            std::process::id() as i64,
            now - (5.0 * 60.0 * 60.0),
            "p",
        );
        // (c) a dead-pid entry — an implausible pid, fresh started_at.
        write_lock_entry(
            &root,
            "repo-dead-pid",
            "native-build",
            999_999_999,
            now,
            "p",
        );

        let view = compute_fleet_slot_view(&root);
        assert!(!view.degraded);

        let dtos = categories_to_dtos(&view);
        let native = dtos
            .iter()
            .find(|d| d.category == "native-build")
            .expect("native-build row must be present");
        assert_eq!(
            native.active_repos,
            vec!["repo-live".to_owned()],
            "only the genuinely live entry counts — stale-ttl and dead-pid entries \
             must not appear in active_repos"
        );
        assert_eq!(native.active_count, 1);
        assert_eq!(native.cap, 4);
        assert_eq!(native.slots_available, 3);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_fleet_locks_dir_reports_degraded_true_never_a_hard_failure() {
        let root = crate::testsupport::unique_temp_dir("bastion-concurrency-missing-locks");
        // Deliberately no `.fleet-locks` directory created under `root`.

        let view = compute_fleet_slot_view(&root);
        assert!(
            view.degraded,
            "a missing .fleet-locks dir must degrade to advisory, not error"
        );

        let categories = categories_to_dtos(&view);
        let dto = repo_to_dto("bastion", Some("native-build"), &categories, view.degraded);
        assert!(dto.allowed, "degraded read must still report allowed: true");
        assert!(dto.degraded);
        assert!(dto.reason.is_some());

        let _ = std::fs::remove_dir_all(&root);
    }
}
