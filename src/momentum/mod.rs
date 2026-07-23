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

pub mod parse;
pub mod render;

// Reuse the shared parser instead of duplicating frontmatter/momentum parsing.
pub use crate::serve::status::repo::{RepoStatus, parse_status};
