// Report rendering — greppable per-error lines + summary.

use crate::validate::ValidationError;
use std::collections::BTreeMap;

/// Render a greppable validation report.
///
/// Output format:
/// - One line per error: `<file>:<line>: <kind-label>: <message>`
/// - Errors sorted by file path (lexicographic) then by line number.
/// - A trailing summary line: `N error(s) across M file(s)` when errors exist,
///   or `no issues found across M file(s)` when the set is clean.
pub fn render_report(errors: &[ValidationError], files_scanned: usize) -> String {
    let mut lines: Vec<String> = Vec::new();

    // Group errors by file, preserving sorted file order via BTreeMap.
    // Within each file, sort by line number.
    let mut by_file: BTreeMap<&std::path::PathBuf, Vec<&ValidationError>> = BTreeMap::new();
    for e in errors {
        by_file.entry(&e.file).or_default().push(e);
    }

    for (_file, mut file_errors) in by_file {
        file_errors.sort_by_key(|e| e.line);
        for e in file_errors {
            lines.push(format!(
                "{}:{}: {}: {}",
                e.file.display(),
                e.line,
                e.kind.label(),
                e.message
            ));
        }
    }

    // Summary line.
    if errors.is_empty() {
        lines.push(format!("no issues found across {files_scanned} file(s)"));
    } else {
        let file_count = count_unique_files(errors);
        lines.push(format!(
            "{} error(s) across {} file(s)",
            errors.len(),
            file_count
        ));
    }

    lines.join("\n")
}

/// Count the number of distinct files referenced in the error set.
fn count_unique_files(errors: &[ValidationError]) -> usize {
    let mut seen = std::collections::HashSet::new();
    for e in errors {
        seen.insert(&e.file);
    }
    seen.len()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::{ErrorKind, ValidationError};
    use std::path::PathBuf;

    fn err(file: &str, line: usize, kind: ErrorKind, message: &str) -> ValidationError {
        ValidationError {
            file: PathBuf::from(file),
            line,
            kind,
            message: message.to_string(),
        }
    }

    // ── render_report — empty set ─────────────────────────────────────────────

    #[test]
    fn empty_errors_zero_files() {
        let report = render_report(&[], 0);
        assert_eq!(report, "no issues found across 0 file(s)");
    }

    #[test]
    fn empty_errors_multiple_files_scanned() {
        let report = render_report(&[], 5);
        assert_eq!(report, "no issues found across 5 file(s)");
    }

    #[test]
    fn empty_errors_summary_contains_no_issues() {
        let report = render_report(&[], 3);
        assert!(report.contains("no issues found"), "report: {report}");
    }

    // ── render_report — single error ──────────────────────────────────────────

    #[test]
    fn single_error_format() {
        let errors = vec![err(
            "docs/file.md",
            5,
            ErrorKind::MissingFrontmatter,
            "no frontmatter",
        )];
        let report = render_report(&errors, 1);
        let lines: Vec<&str> = report.lines().collect();

        // First line: the error itself.
        assert_eq!(
            lines[0],
            "docs/file.md:5: missing-frontmatter: no frontmatter"
        );
        // Second line: summary.
        assert_eq!(lines[1], "1 error(s) across 1 file(s)");
    }

    #[test]
    fn single_error_kind_labels_in_output() {
        for (kind, expected_label) in [
            (ErrorKind::MissingFrontmatter, "missing-frontmatter"),
            (ErrorKind::MalformedFrontmatter, "malformed-frontmatter"),
            (ErrorKind::MissingField, "missing-field"),
            (ErrorKind::EmptyField, "empty-field"),
            (ErrorKind::BrokenLink, "broken-link"),
        ] {
            let errors = vec![err("f.md", 1, kind, "msg")];
            let report = render_report(&errors, 1);
            let first_line = report.lines().next().unwrap();
            assert!(
                first_line.contains(expected_label),
                "expected label {expected_label} in: {first_line}"
            );
        }
    }

    // ── render_report — multi-file sorted ordering ────────────────────────────

    #[test]
    fn errors_sorted_by_file_then_line() {
        let errors = vec![
            err("z-file.md", 3, ErrorKind::MissingField, "msg-z3"),
            err("a-file.md", 10, ErrorKind::BrokenLink, "msg-a10"),
            err("a-file.md", 2, ErrorKind::EmptyField, "msg-a2"),
            err("m-file.md", 1, ErrorKind::MissingFrontmatter, "msg-m1"),
        ];
        let report = render_report(&errors, 3);
        let lines: Vec<&str> = report.lines().collect();

        // 4 errors + 1 summary line = 5 lines total
        assert_eq!(lines.len(), 5);

        // a-file.md comes first (sorted), with its line 2 before line 10.
        assert!(lines[0].starts_with("a-file.md:2:"), "line0: {}", lines[0]);
        assert!(lines[1].starts_with("a-file.md:10:"), "line1: {}", lines[1]);
        // m-file.md is next.
        assert!(lines[2].starts_with("m-file.md:1:"), "line2: {}", lines[2]);
        // z-file.md is last.
        assert!(lines[3].starts_with("z-file.md:3:"), "line3: {}", lines[3]);
        // Summary.
        assert_eq!(lines[4], "4 error(s) across 3 file(s)");
    }

    #[test]
    fn summary_counts_unique_files() {
        // 3 errors across 2 distinct files.
        let errors = vec![
            err("a.md", 1, ErrorKind::MissingField, "f1"),
            err("a.md", 2, ErrorKind::EmptyField, "f2"),
            err("b.md", 5, ErrorKind::BrokenLink, "f3"),
        ];
        let report = render_report(&errors, 2);
        let summary = report.lines().last().unwrap();
        assert_eq!(summary, "3 error(s) across 2 file(s)");
    }

    #[test]
    fn summary_line_is_last() {
        let errors = vec![err("x.md", 1, ErrorKind::MissingFrontmatter, "msg")];
        let report = render_report(&errors, 1);
        let last = report.lines().last().unwrap();
        assert!(
            last.contains("error(s)") || last.contains("no issues"),
            "last: {last}"
        );
    }

    #[test]
    fn error_line_format_is_greppable() {
        // Verify `file:line: kind: message` format (no ANSI codes).
        let errors = vec![err(
            "path/to/doc.md",
            42,
            ErrorKind::MissingField,
            "required field `type` is missing",
        )];
        let report = render_report(&errors, 1);
        let first = report.lines().next().unwrap();
        assert_eq!(
            first,
            "path/to/doc.md:42: missing-field: required field `type` is missing"
        );
    }

    // ── Integration tests with fixture files ──────────────────────────────────

    #[test]
    fn fixture_good_md_no_errors() {
        use crate::validate::{frontmatter, links};

        // Locate the fixture relative to this source file's directory.
        // At test time, CARGO_MANIFEST_DIR points to the crate root.
        let fixtures_dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/validate/fixtures");
        let good_path = fixtures_dir.join("good.md");

        let content = std::fs::read_to_string(&good_path)
            .unwrap_or_else(|e| panic!("could not read {}: {e}", good_path.display()));

        let fm_errors = frontmatter::validate_frontmatter(&content, &good_path);
        let link_errors = links::validate_links(&content, &good_path);

        assert!(
            fm_errors.is_empty(),
            "good.md should have no frontmatter errors, got: {fm_errors:?}"
        );
        assert!(
            link_errors.is_empty(),
            "good.md should have no link errors, got: {link_errors:?}"
        );
    }

    #[test]
    fn fixture_bad_frontmatter_md_has_errors() {
        use crate::validate::frontmatter;

        let fixtures_dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/validate/fixtures");
        let bad_path = fixtures_dir.join("bad-frontmatter.md");

        let content = std::fs::read_to_string(&bad_path)
            .unwrap_or_else(|e| panic!("could not read {}: {e}", bad_path.display()));

        let fm_errors = frontmatter::validate_frontmatter(&content, &bad_path);
        assert!(
            !fm_errors.is_empty(),
            "bad-frontmatter.md should have at least one frontmatter error"
        );

        // Verify each error has a populated kind and message.
        for e in &fm_errors {
            assert!(!e.message.is_empty(), "error message should not be empty");
            assert!(e.line > 0, "error line should be > 0");
        }
    }

    #[test]
    fn fixture_broken_links_md_has_link_errors_not_frontmatter_errors() {
        use crate::validate::{frontmatter, links};

        let fixtures_dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/validate/fixtures");
        let bl_path = fixtures_dir.join("broken-links.md");

        let content = std::fs::read_to_string(&bl_path)
            .unwrap_or_else(|e| panic!("could not read {}: {e}", bl_path.display()));

        // Frontmatter should be valid in this fixture.
        let fm_errors = frontmatter::validate_frontmatter(&content, &bl_path);
        assert!(
            fm_errors.is_empty(),
            "broken-links.md should have valid frontmatter, got: {fm_errors:?}"
        );

        // Should have exactly one broken link error.
        let link_errors = links::validate_links(&content, &bl_path);
        assert!(
            !link_errors.is_empty(),
            "broken-links.md should have at least one broken link error"
        );

        let broken: Vec<_> = link_errors
            .iter()
            .filter(|e| e.kind == ErrorKind::BrokenLink)
            .collect();
        assert!(!broken.is_empty(), "expected at least one BrokenLink error");
    }

    #[test]
    fn fixture_broken_links_external_and_anchor_not_flagged() {
        use crate::validate::links;

        let fixtures_dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/validate/fixtures");
        let bl_path = fixtures_dir.join("broken-links.md");

        let content = std::fs::read_to_string(&bl_path)
            .unwrap_or_else(|e| panic!("could not read {}: {e}", bl_path.display()));

        let link_errors = links::validate_links(&content, &bl_path);

        // No error should mention the external URL or anchor.
        for e in &link_errors {
            assert!(
                !e.message.contains("https://"),
                "external URL should not be flagged: {}",
                e.message
            );
            assert!(
                !e.message.starts_with("broken link: #"),
                "pure anchor should not be flagged: {}",
                e.message
            );
        }
    }

    #[test]
    fn render_report_output_shape_for_representative_errors() {
        let errors = vec![
            err(
                "src/validate/fixtures/bad-frontmatter.md",
                1,
                ErrorKind::MissingFrontmatter,
                "no frontmatter block",
            ),
            err(
                "src/validate/fixtures/broken-links.md",
                10,
                ErrorKind::BrokenLink,
                "broken link: nonexistent.md",
            ),
        ];
        let report = render_report(&errors, 3);
        let lines: Vec<&str> = report.lines().collect();

        // 2 error lines + 1 summary = 3 lines.
        assert_eq!(lines.len(), 3, "report:\n{report}");

        // Files sorted: bad-frontmatter before broken-links.
        assert!(
            lines[0].contains("bad-frontmatter.md"),
            "line0: {}",
            lines[0]
        );
        assert!(lines[1].contains("broken-links.md"), "line1: {}", lines[1]);

        // Summary.
        assert_eq!(lines[2], "2 error(s) across 2 file(s)");
    }
}
