//! Pure cross-repo table render for the momentum/metrics rollup.
//!
//! Takes an already-assembled `&[RepoRollup]` (no I/O) and produces a
//! deterministic, glanceable String: one row per repo with the frontmatter
//! `now`/`next`/`blocked` scalars, followed by a rolled-up `## Metrics`
//! section listing each repo's metrics bullets.

use super::parse::RepoRollup;

/// Render `rollups` into a cross-repo table + metrics rollup String.
///
/// - A header row (`Repo | Now | Next | Blocked`) plus one row per rollup,
///   in the order given (callers — e.g. [`super::collect::collect_rollups`]
///   — are responsible for sorting).
/// - A trailing `Metrics` section: one heading per repo (skipped if that
///   repo has no metrics bullets), followed by `- ` bullet lines.
/// - The empty-slice case renders a header plus an explicit "no repos"
///   line — never panics, never produces an empty string.
pub fn render_table(rollups: &[RepoRollup]) -> String {
    let mut out = String::new();

    out.push_str(
        "Repo       | Now                            | Next                           | Blocked\n",
    );
    out.push_str(
        "-----------|--------------------------------|--------------------------------|--------\n",
    );

    if rollups.is_empty() {
        out.push_str("(no repos in workspace registry)\n");
    } else {
        for rollup in rollups {
            out.push_str(&format!(
                "{:<10} | {:<30} | {:<30} | {}\n",
                rollup.name, rollup.status.now, rollup.status.next, rollup.status.blocked
            ));
        }
    }

    out.push('\n');
    out.push_str("Metrics\n");
    out.push_str("-------\n");

    if rollups.is_empty() {
        out.push_str("(no repos in workspace registry)\n");
    } else {
        let mut any_metrics = false;
        for rollup in rollups {
            if rollup.metrics.is_empty() {
                continue;
            }
            any_metrics = true;
            out.push_str(&format!("{}:\n", rollup.name));
            for bullet in &rollup.metrics {
                out.push_str(&format!("  - {bullet}\n"));
            }
        }
        if !any_metrics {
            out.push_str("(no metrics reported)\n");
        }
    }

    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serve::status::repo::RepoStatus;

    fn status(now: &str, next: &str, blocked: &str) -> RepoStatus {
        RepoStatus {
            name: String::new(),
            now: now.to_string(),
            next: next.to_string(),
            blocked: blocked.to_string(),
            has_handoff: false,
            momentum_now: String::new(),
            momentum_next: String::new(),
            momentum_blocked: String::new(),
            momentum_improve: String::new(),
            momentum_recurring: String::new(),
        }
    }

    #[test]
    fn renders_header_row() {
        let out = render_table(&[]);
        assert!(out.contains("Repo"));
        assert!(out.contains("Now"));
        assert!(out.contains("Next"));
        assert!(out.contains("Blocked"));
    }

    #[test]
    fn empty_slice_renders_no_repos_line_without_panic() {
        let out = render_table(&[]);
        assert!(out.contains("(no repos in workspace registry)"));
    }

    #[test]
    fn renders_known_row_cells() {
        let rollups = vec![RepoRollup {
            name: "bastion".to_string(),
            status: status("BA.7.D in progress", "Wire the CLI", "[]"),
            metrics: vec!["blocks shipped: 3".to_string()],
        }];
        let out = render_table(&rollups);
        assert!(out.contains("bastion"));
        assert!(out.contains("BA.7.D in progress"));
        assert!(out.contains("Wire the CLI"));
        assert!(out.contains("[]"));
    }

    #[test]
    fn renders_metrics_rollup_lines() {
        let rollups = vec![
            RepoRollup {
                name: "bastion".to_string(),
                status: status("now-a", "next-a", "[]"),
                metrics: vec!["metric one".to_string(), "metric two".to_string()],
            },
            RepoRollup {
                name: "bella".to_string(),
                status: status("now-b", "next-b", "[]"),
                metrics: vec![],
            },
        ];
        let out = render_table(&rollups);
        assert!(out.contains("bastion:"));
        assert!(out.contains("- metric one"));
        assert!(out.contains("- metric two"));
        // bella has no metrics bullets, so no "bella:" heading should appear
        // in the metrics section rollup.
        assert!(!out.contains("bella:"));
    }

    #[test]
    fn no_metrics_anywhere_renders_explicit_line() {
        let rollups = vec![RepoRollup {
            name: "bastion".to_string(),
            status: status("now-a", "next-a", "[]"),
            metrics: vec![],
        }];
        let out = render_table(&rollups);
        assert!(out.contains("(no metrics reported)"));
    }

    #[test]
    fn multiple_rows_preserve_given_order() {
        let rollups = vec![
            RepoRollup {
                name: "bastion".to_string(),
                status: status("a", "b", "c"),
                metrics: vec![],
            },
            RepoRollup {
                name: "amistad".to_string(),
                status: status("d", "e", "f"),
                metrics: vec![],
            },
        ];
        let out = render_table(&rollups);
        let bastion_pos = out.find("bastion").expect("bastion row present");
        let amistad_pos = out.find("amistad").expect("amistad row present");
        assert!(
            bastion_pos < amistad_pos,
            "render_table must not reorder rollups"
        );
    }
}
