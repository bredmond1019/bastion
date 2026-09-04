//! Pure cross-repo table render for the momentum/metrics rollup.
//!
//! Takes an already-assembled `&[RepoRollup]` (no I/O) and produces a
//! deterministic, glanceable String: one row per repo with the frontmatter
//! `now`/`next`/`blocked` scalars, followed by a rolled-up `## Metrics`
//! section listing each repo's metrics bullets.

use std::path::Path;

use super::parse::RepoRollup;

/// Render the provenance line(s) naming (a) the config file the
/// `[workspaces]` registry was read from, and (b) the workspaces rolled up.
///
/// Pure: `config_path` and `workspace_names` arrive as parameters — no
/// filesystem access, no env reads, no consultation of the operator's real
/// `~/.config/bastion/config.toml`.
///
/// - `config_path` is `None` when [`crate::config::config_path`] found
///   neither `XDG_CONFIG_HOME` nor `HOME` set; that case renders an explicit
///   "no config resolved" line rather than a blank/omitted one.
/// - An empty `workspace_names` slice still names the config file that WAS
///   consulted, so "the registry is empty" is distinguishable from "the
///   config file was never found".
/// - The workspace name list is truncated for readability past a handful of
///   entries, but the printed COUNT is always exact.
pub fn render_provenance(config_path: Option<&Path>, workspace_names: &[String]) -> String {
    let registry_line = match config_path {
        Some(path) => format!("registry: {}", path.display()),
        None => "registry: none resolved (no XDG_CONFIG_HOME or HOME)".to_string(),
    };

    const MAX_LISTED: usize = 12;
    let count = workspace_names.len();
    let workspaces_line = if count == 0 {
        "0 workspaces registered".to_string()
    } else if count <= MAX_LISTED {
        format!("{count} workspaces: {}", workspace_names.join(", "))
    } else {
        let shown = workspace_names[..MAX_LISTED].join(", ");
        format!(
            "{count} workspaces: {shown}, ... ({} more)",
            count - MAX_LISTED
        )
    };

    format!("{registry_line} — {workspaces_line}\n")
}

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

    // ── render_provenance ────────────────────────────────────────────────

    #[test]
    fn provenance_populated_registry_shows_path_and_exact_count() {
        let path = std::path::PathBuf::from("/Users/alice/.config/bastion/config.toml");
        let names = vec![
            "amistad".to_string(),
            "bastion".to_string(),
            "brain".to_string(),
        ];
        let out = render_provenance(Some(&path), &names);
        assert!(out.contains("/Users/alice/.config/bastion/config.toml"));
        assert!(out.contains("3 workspaces"));
        assert!(out.contains("amistad"));
        assert!(out.contains("bastion"));
        assert!(out.contains("brain"));
    }

    #[test]
    fn provenance_empty_registry_still_names_config_file() {
        let path = std::path::PathBuf::from("/Users/alice/.config/bastion/config.toml");
        let out = render_provenance(Some(&path), &[]);
        assert!(out.contains("/Users/alice/.config/bastion/config.toml"));
        assert!(out.contains("0 workspaces"));
    }

    #[test]
    fn provenance_none_config_path_renders_explicit_message() {
        let out = render_provenance(None, &[]);
        assert!(out.contains("none resolved"));
        assert!(out.contains("XDG_CONFIG_HOME"));
        assert!(out.contains("HOME"));
    }

    #[test]
    fn provenance_none_config_path_with_workspaces_still_explicit() {
        let names = vec!["bastion".to_string()];
        let out = render_provenance(None, &names);
        assert!(out.contains("none resolved"));
        assert!(out.contains("1 workspaces"));
    }

    #[test]
    fn provenance_truncates_long_workspace_list_but_keeps_exact_count() {
        let path = std::path::PathBuf::from("/x/config.toml");
        let names: Vec<String> = (0..20).map(|i| format!("repo{i}")).collect();
        let out = render_provenance(Some(&path), &names);
        assert!(out.contains("20 workspaces"));
        assert!(out.contains("more"));
        // truncated: not every one of the 20 names should be present verbatim
        assert!(!out.contains("repo19"));
    }

    #[test]
    fn provenance_never_empty_string() {
        assert!(!render_provenance(None, &[]).is_empty());
        let path = std::path::PathBuf::from("/x/config.toml");
        assert!(!render_provenance(Some(&path), &[]).is_empty());
    }
}
