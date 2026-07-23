//! Pure parsing for the momentum/metrics rollup: the `## Metrics` section
//! extractor and the `RepoRollup` assembly helper.
//!
//! Mirrors the section-scan style of `parse_momentum` in
//! `src/serve/status/repo.rs`: start capturing after the target `## `
//! heading, stop at the next `## ` heading (or EOF), and skip `> `
//! blockquote and blank lines while collecting `- ` bullet text.

use super::{RepoStatus, parse_status};

/// A single workspace's parsed rollup: the reused [`RepoStatus`] (frontmatter
/// scalars + momentum queues) plus this module's own `## Metrics` bullets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRollup {
    /// Workspace registry name (not derivable from `status.md` content alone).
    pub name: String,
    /// Frontmatter scalars + momentum queues, reused from `serve::status::repo`.
    pub status: RepoStatus,
    /// `## Metrics` section bullet text, one entry per `- ` line.
    pub metrics: Vec<String>,
}

/// Scan `content` for a `## Metrics` heading and return each `- ` bullet's
/// text (the text after the leading `- `).
///
/// A missing `## Metrics` section, or a section with no bullet lines (e.g.
/// only a `> ` blockquote note), yields an empty `Vec`. Bullet capture stops
/// at the next `## ` heading or EOF, whichever comes first.
pub fn parse_metrics(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_section = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("## ") {
            in_section = trimmed.trim_start_matches('#').trim() == "Metrics";
            continue;
        }

        if !in_section {
            continue;
        }

        if trimmed.is_empty() || trimmed.starts_with('>') {
            continue;
        }

        if let Some(text) = trimmed.strip_prefix("- ") {
            out.push(text.trim().to_string());
        }
    }

    out
}

/// Parse `content` into a [`RepoRollup`] for workspace `name`.
///
/// Returns `None` when [`parse_status`] cannot find well-formed frontmatter
/// (mirrors that function's failure mode — no separate failure path for
/// `## Metrics`, which is always best-effort and defaults to an empty `Vec`).
pub fn parse_repo_rollup(content: &str, name: &str) -> Option<RepoRollup> {
    let status = parse_status(content)?;
    let metrics = parse_metrics(content);

    Some(RepoRollup {
        name: name.to_string(),
        status,
        metrics,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Single-bullet `## Metrics` section at EOF (no trailing heading).
    const WELL_FORMED: &str = include_str!("../serve/status/fixtures/status_well_formed.md");
    /// No frontmatter and no `## Metrics` heading at all.
    const NO_METRICS_SECTION: &str = include_str!("../serve/status/fixtures/status_malformed.md");
    /// `## Metrics` heading present, only a blockquote note, no bullets.
    const EMPTY_METRICS: &str = include_str!("fixtures/status_metrics_empty.md");
    /// Multi-bullet `## Metrics` section followed by a `## Notes` heading.
    const MULTI_METRICS: &str = include_str!("fixtures/status_metrics_multi.md");

    #[test]
    fn parses_single_bullet_metrics_section() {
        let metrics = parse_metrics(WELL_FORMED);
        assert_eq!(metrics, vec!["placeholder".to_string()]);
    }

    #[test]
    fn missing_metrics_section_yields_empty_vec() {
        let metrics = parse_metrics(NO_METRICS_SECTION);
        assert_eq!(metrics, Vec::<String>::new());
    }

    #[test]
    fn empty_metrics_section_yields_empty_vec() {
        let metrics = parse_metrics(EMPTY_METRICS);
        assert_eq!(metrics, Vec::<String>::new());
    }

    #[test]
    fn empty_input_yields_empty_vec() {
        assert_eq!(parse_metrics(""), Vec::<String>::new());
    }

    #[test]
    fn parses_multiple_bullets_element_by_element() {
        let metrics = parse_metrics(MULTI_METRICS);
        assert_eq!(
            metrics,
            vec![
                "blocks shipped this week: 3".to_string(),
                "open backlog tickets: 12".to_string(),
                "days since last handoff: 2".to_string(),
            ]
        );
    }

    #[test]
    fn bullet_capture_stops_at_next_heading() {
        let metrics = parse_metrics(MULTI_METRICS);
        assert!(
            !metrics.iter().any(|m| m.contains("must not appear")),
            "bullets under ## Notes must not be captured: {metrics:?}"
        );
        assert_eq!(metrics.len(), 3);
    }

    #[test]
    fn parse_repo_rollup_assembles_status_and_metrics() {
        let rollup = parse_repo_rollup(MULTI_METRICS, "bastion").expect("should parse");
        assert_eq!(rollup.name, "bastion");
        assert_eq!(rollup.status.now, "BA.7.D in progress — momentum module");
        assert_eq!(rollup.status.next, "Wire the CLI subcommand");
        assert_eq!(rollup.status.blocked, "[]");
        assert_eq!(
            rollup.metrics,
            vec![
                "blocks shipped this week: 3".to_string(),
                "open backlog tickets: 12".to_string(),
                "days since last handoff: 2".to_string(),
            ]
        );
    }

    #[test]
    fn parse_repo_rollup_returns_none_without_frontmatter() {
        assert!(parse_repo_rollup(NO_METRICS_SECTION, "bastion").is_none());
    }
}
