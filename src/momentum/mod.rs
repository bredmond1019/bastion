//! Cross-repo momentum & metrics rollup (BA.7.D).
//!
//! Surfaces the D30 momentum queues (`now`/`next`/`blocked`/`improve`/
//! `recurring`) and the `## Metrics` section across every workspace's
//! `planning/status.md` in one glanceable cross-repo view.
//!
//! This module deliberately reuses [`crate::serve::status::repo::parse_status`]
//! for the frontmatter scalars and momentum queues rather than duplicating
//! that parser — see `planning/7.D-console-momentum-metrics/tasks.md` for the
//! reuse rationale (Phase 11 Block D shipped the parser first). The one gap
//! `parse_status` leaves — the `## Metrics` section — is filled by
//! [`parse::parse_metrics`] here.

pub mod collect;
pub mod parse;
pub mod render;

// Reuse the shared parser instead of duplicating frontmatter/momentum parsing.
pub use crate::serve::status::repo::{RepoStatus, parse_status};

/// Load the `[workspaces]` registry, walk each workspace's `status.md`,
/// render the cross-repo momentum/metrics table, and print it to stdout.
///
/// A trivial wrapper over already-tested pure functions — DB-free (D25):
/// this is a read-only console surface, never a write path back into
/// `status.md`. Config load / file-walk degradation contracts are the
/// established ones (`config::load_workspace_registry`,
/// `collect::collect_rollups`); this fn adds no new degradation logic of
/// its own.
pub fn run() -> anyhow::Result<()> {
    let registry = crate::config::load_workspace_registry(
        std::env::var("XDG_CONFIG_HOME").ok(),
        std::env::var("HOME").ok(),
    )?;

    let workspaces = registry.workspaces.unwrap_or_default();
    let rollups = collect::collect_rollups(&workspaces);
    let table = render::render_table(&rollups);

    println!("{table}");

    Ok(())
}
