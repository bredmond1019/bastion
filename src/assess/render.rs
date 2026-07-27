//! Renderers for `bastion assess` (Phase 15, Block BA.15.9, task 6): a human summary and
//! a `--json` envelope.
//!
//! Both mirror `brainval::render_json`'s envelope convention (a stable top-level shape
//! carrying the validator/assessor name, resolved root, and per-family detail) without
//! reusing `mev::JsonReport` directly, since `assess` emits a report shape, not a flat
//! diagnostic list (see `AssessReport`'s own doc comment for why).
//!
//! Neither renderer ever emits an id-convention section: that family is deferred to
//! `BA.15.6` and must be absent from output, not present as a zero-count entry.

use anyhow::Result;

use super::AssessReport;

/// Severity ranking for [`top_gaps`] — lower numbers sort first (more severe).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Severity {
    /// Invalid frontmatter (the file couldn't even be parsed) and state-file load
    /// failures rank most severe.
    Critical,
    /// Missing required OKF fields and dangling graph edges.
    Major,
    /// Empty focus/tracks in an otherwise-loaded state file.
    Minor,
}

/// One ranked gap surfaced in the `Top gaps` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gap {
    pub family: &'static str,
    pub label: String,
    pub count: usize,
    severity: Severity,
}

/// Rank every non-zero gap across the three shipped families, most severe (and, within
/// a severity tier, highest count) first, capped at three. Pure — no I/O, no
/// randomness; ties break on family declaration order (okf, graph, state) for
/// deterministic output.
pub fn top_gaps(report: &AssessReport) -> Vec<Gap> {
    let mut gaps: Vec<Gap> = Vec::new();

    if report.okf.invalid_frontmatter_count > 0 {
        gaps.push(Gap {
            family: "okf",
            label: "invalid frontmatter".to_string(),
            count: report.okf.invalid_frontmatter_count,
            severity: Severity::Critical,
        });
    }
    if report.okf.missing_required_count > 0 {
        gaps.push(Gap {
            family: "okf",
            label: "missing required fields".to_string(),
            count: report.okf.missing_required_count,
            severity: Severity::Major,
        });
    }

    if report.graph.dangling_count > 0 {
        gaps.push(Gap {
            family: "graph",
            label: "dangling related: references".to_string(),
            count: report.graph.dangling_count,
            severity: Severity::Major,
        });
    }

    for finding in &report.state.findings {
        use super::StateFinding;
        let (label, severity) = match finding {
            StateFinding::NoPlanningRoot => ("no planning/ resolved", Severity::Critical),
            StateFinding::StateFileAbsent { .. } => ("state.json absent", Severity::Critical),
            StateFinding::StateFileUnparseable { .. } => {
                ("state.json unparseable", Severity::Critical)
            }
            StateFinding::EmptyFocus => ("focus is empty", Severity::Minor),
            StateFinding::EmptyTracks => ("tracks is empty", Severity::Minor),
        };
        gaps.push(Gap {
            family: "state",
            label: label.to_string(),
            count: 1,
            severity,
        });
    }

    gaps.sort_by(|a, b| a.severity.cmp(&b.severity).then(b.count.cmp(&a.count)));
    gaps.truncate(3);
    gaps
}

/// Render a human-readable summary: resolved root, files scanned, one section per
/// shipped family, and a capped `Top gaps` block. No id-convention section.
pub fn render_human(report: &AssessReport) -> String {
    let mut out = String::new();

    out.push_str(&format!("assess: {}\n", report.root));
    out.push_str(&format!("files scanned: {}\n\n", report.files_scanned));

    out.push_str("OKF coverage\n");
    out.push_str(&format!(
        "  valid frontmatter: {}/{}\n",
        report.okf.files_with_valid_frontmatter, report.okf.files_scanned
    ));
    out.push_str(&format!(
        "  missing required fields: {}\n",
        report.okf.missing_required_count
    ));
    out.push_str(&format!(
        "  invalid frontmatter: {}\n\n",
        report.okf.invalid_frontmatter_count
    ));

    out.push_str("Graph readiness\n");
    out.push_str(&format!("  nodes: {}\n", report.graph.node_count));
    out.push_str(&format!("  edges: {}\n", report.graph.edge_count));
    out.push_str(&format!(
        "  resolved: {}  leaf targets: {}  dangling: {}\n\n",
        report.graph.resolved_count, report.graph.leaf_target_count, report.graph.dangling_count
    ));

    out.push_str("State readiness\n");
    out.push_str(&format!("  ready: {}\n", report.state.ready));
    out.push_str(&format!(
        "  tracks: {}  blocks: {}\n\n",
        report.state.track_count, report.state.block_count
    ));

    let gaps = top_gaps(report);
    out.push_str("Top gaps\n");
    if gaps.is_empty() {
        out.push_str("  none\n");
    } else {
        for gap in &gaps {
            out.push_str(&format!(
                "  [{}] {} ({})\n",
                gap.family, gap.label, gap.count
            ));
        }
    }

    out
}

/// Render an `AssessReport` as pretty-printed JSON, mirroring
/// `brainval::render_json`'s envelope convention. The struct's own field set already
/// follows that convention (`assessor` + `root` + counts + per-family objects), so this
/// is a direct pretty-print rather than a wrapping type.
pub fn render_json(report: &AssessReport) -> Result<String> {
    Ok(serde_json::to_string_pretty(report)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assess::graph::GraphFamily;
    use crate::assess::okf::{InvalidFrontmatterFinding, MissingRequiredFinding, OkfFamily};
    use crate::assess::state::{StateFamily, StateFinding};

    fn clean_report() -> AssessReport {
        AssessReport {
            assessor: "assess".to_string(),
            root: "/tmp/example-repo".to_string(),
            files_scanned: 3,
            okf: OkfFamily {
                files_scanned: 3,
                files_with_valid_frontmatter: 3,
                ..Default::default()
            },
            graph: GraphFamily {
                node_count: 3,
                edge_count: 0,
                ..Default::default()
            },
            state: StateFamily {
                ready: true,
                track_count: 1,
                block_count: 2,
                ..Default::default()
            },
        }
    }

    fn messy_report() -> AssessReport {
        let mut report = clean_report();
        report.okf.invalid_frontmatter_count = 2;
        report.okf.invalid_frontmatter_findings = vec![
            InvalidFrontmatterFinding {
                file: "a.md".to_string(),
                kind: "no_frontmatter".to_string(),
                line: None,
            },
            InvalidFrontmatterFinding {
                file: "b.md".to_string(),
                kind: "malformed_line".to_string(),
                line: Some(2),
            },
        ];
        report.okf.missing_required_count = 1;
        report.okf.missing_required_findings = vec![MissingRequiredFinding {
            file: "c.md".to_string(),
            field: "title".to_string(),
            line: None,
        }];
        report.graph.dangling_count = 1;
        report.state.ready = false;
        report.state.findings = vec![StateFinding::EmptyFocus];
        report
    }

    // ── top_gaps ordering + cap ───────────────────────────────────────────────

    #[test]
    fn top_gaps_over_a_clean_report_is_empty() {
        assert!(top_gaps(&clean_report()).is_empty());
    }

    #[test]
    fn top_gaps_ranks_critical_before_major_before_minor() {
        // messy_report() has one critical-tier gap (invalid frontmatter), two
        // major-tier gaps (missing required fields, dangling edges), and one
        // minor-tier gap (focus is empty) — four total, capped at three, so the
        // minor-tier gap is the one that gets dropped.
        let gaps = top_gaps(&messy_report());
        assert_eq!(gaps.len(), 3);
        assert_eq!(gaps[0].label, "invalid frontmatter");
        assert_eq!(gaps[0].family, "okf");
        let major_labels: Vec<&str> = gaps[1..3].iter().map(|g| g.label.as_str()).collect();
        assert!(major_labels.contains(&"missing required fields"));
        assert!(major_labels.contains(&"dangling related: references"));
        assert!(!gaps.iter().any(|g| g.label == "focus is empty"));
    }

    #[test]
    fn top_gaps_is_capped_at_three_even_with_more_findings() {
        let mut report = messy_report();
        report.state.findings.push(StateFinding::EmptyTracks);
        let gaps = top_gaps(&report);
        assert_eq!(gaps.len(), 3);
    }

    #[test]
    fn top_gaps_yields_fewer_than_three_when_fewer_exist() {
        let mut report = clean_report();
        report.graph.dangling_count = 1;
        let gaps = top_gaps(&report);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].label, "dangling related: references");
    }

    // ── render_human ──────────────────────────────────────────────────────────

    #[test]
    fn render_human_names_each_family_heading_and_no_id_convention_heading() {
        let human = render_human(&messy_report());
        assert!(human.contains("OKF coverage"));
        assert!(human.contains("Graph readiness"));
        assert!(human.contains("State readiness"));
        assert!(human.contains("Top gaps"));
        assert!(human.contains(&clean_report().root));
        for needle in [
            "ID convention",
            "Id convention",
            "id_convention",
            "id-convention",
        ] {
            assert!(
                !human.contains(needle),
                "unexpected id-convention heading `{needle}` in human output:\n{human}"
            );
        }
    }

    #[test]
    fn render_human_over_a_clean_report_shows_no_gaps() {
        let human = render_human(&clean_report());
        assert!(human.contains("Top gaps"));
        assert!(human.contains("none"));
    }

    // ── render_json ───────────────────────────────────────────────────────────

    #[test]
    fn render_json_round_trips_into_assess_report() {
        let report = messy_report();
        let json = render_json(&report).expect("render_json should succeed");
        let round_tripped: AssessReport = serde_json::from_str(&json).expect("valid json");
        assert_eq!(round_tripped, report);
    }

    #[test]
    fn render_json_carries_no_id_convention_key() {
        let json = render_json(&messy_report()).expect("render_json should succeed");
        for needle in ["id_convention", "id-convention", "idConvention", "\"ids\""] {
            assert!(
                !json.contains(needle),
                "unexpected id-convention key `{needle}` in JSON:\n{json}"
            );
        }
    }
}
