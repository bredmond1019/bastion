//! OKF-coverage family for `bastion assess` (Phase 15, Block BA.15.9, task 3).
//!
//! Pure over already-read `(path, content)` pairs — no filesystem access happens here.
//! Classifies every discovered markdown file's frontmatter via
//! `okf_core::parse::extract_frontmatter`: files whose frontmatter fails to parse are
//! invalid-frontmatter findings; files that parse are checked for the three REQUIRED
//! OKF fields (`type`/`title`/`description`, present and non-empty) and tallied for
//! which of the six OPTIONAL fields (`doc_id`/`layer`/`project`/`status`/`keywords`/
//! `related`) they carry.

use std::collections::BTreeMap;
use std::path::Path;

use okf_core::{ParseResult, extract_frontmatter};
use serde::{Deserialize, Serialize};

/// The three REQUIRED OKF frontmatter fields, in canonical order.
const REQUIRED_FIELDS: [&str; 3] = ["type", "title", "description"];

/// The six OPTIONAL OKF frontmatter fields, in canonical order.
const OPTIONAL_FIELDS: [&str; 6] = [
    "doc_id", "layer", "project", "status", "keywords", "related",
];

/// A required field missing (absent entirely) or present-but-empty on one file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MissingRequiredFinding {
    pub file: String,
    pub field: String,
    /// 1-based source line of the field, when the field was present (empty) rather
    /// than absent entirely.
    pub line: Option<usize>,
}

/// One file whose frontmatter block itself failed to parse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InvalidFrontmatterFinding {
    pub file: String,
    /// `no_frontmatter` | `unterminated_fence` | `malformed_line`.
    pub kind: String,
    /// 1-based source line, when the `ParseResult` variant carries one.
    pub line: Option<usize>,
}

/// OKF-coverage family: how many discovered files carry compliant frontmatter, and
/// where the gaps are.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OkfFamily {
    pub files_scanned: usize,
    pub files_with_valid_frontmatter: usize,
    pub missing_required_count: usize,
    pub missing_required_findings: Vec<MissingRequiredFinding>,
    pub invalid_frontmatter_count: usize,
    pub invalid_frontmatter_findings: Vec<InvalidFrontmatterFinding>,
    /// Optional field name -> count of files that carry it (present, non-empty).
    pub optional_field_coverage: BTreeMap<String, usize>,
}

/// Compute the OKF-coverage family over already-read `(path, content)` pairs.
///
/// Pure: performs no filesystem access. `docs` is expected to already be the set of
/// markdown files `assess::discover::resolve_context` found.
pub fn assess_okf(docs: &[(std::path::PathBuf, String)]) -> OkfFamily {
    let mut family = OkfFamily {
        files_scanned: docs.len(),
        optional_field_coverage: OPTIONAL_FIELDS
            .iter()
            .map(|f| (f.to_string(), 0usize))
            .collect(),
        ..Default::default()
    };

    for (path, content) in docs {
        let file = display_path(path);
        match extract_frontmatter(content) {
            ParseResult::NoFrontmatter => {
                family
                    .invalid_frontmatter_findings
                    .push(InvalidFrontmatterFinding {
                        file,
                        kind: "no_frontmatter".to_string(),
                        line: None,
                    });
            }
            ParseResult::UnterminatedFence { open_line } => {
                family
                    .invalid_frontmatter_findings
                    .push(InvalidFrontmatterFinding {
                        file,
                        kind: "unterminated_fence".to_string(),
                        line: Some(open_line),
                    });
            }
            ParseResult::MalformedLine { source_line } => {
                family
                    .invalid_frontmatter_findings
                    .push(InvalidFrontmatterFinding {
                        file,
                        kind: "malformed_line".to_string(),
                        line: Some(source_line),
                    });
            }
            ParseResult::Ok(fm) => {
                family.files_with_valid_frontmatter += 1;

                for field in REQUIRED_FIELDS {
                    match fm.fields.get(field) {
                        None => {
                            family
                                .missing_required_findings
                                .push(MissingRequiredFinding {
                                    file: file.clone(),
                                    field: field.to_string(),
                                    line: None,
                                });
                        }
                        Some((value, line)) if value.trim().is_empty() => {
                            family
                                .missing_required_findings
                                .push(MissingRequiredFinding {
                                    file: file.clone(),
                                    field: field.to_string(),
                                    line: Some(*line),
                                });
                        }
                        Some(_) => {}
                    }
                }

                for field in OPTIONAL_FIELDS {
                    let present = fm
                        .fields
                        .get(field)
                        .is_some_and(|(value, _)| !value.trim().is_empty());
                    if present {
                        *family
                            .optional_field_coverage
                            .get_mut(field)
                            .expect("optional_field_coverage seeded with all OPTIONAL_FIELDS") += 1;
                    }
                }
            }
        }
    }

    family.missing_required_count = family.missing_required_findings.len();
    family.invalid_frontmatter_count = family.invalid_frontmatter_findings.len();
    family
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn doc(path: &str, content: &str) -> (PathBuf, String) {
        (PathBuf::from(path), content.to_string())
    }

    const FULL: &str = "---\ntype: Guideline\ntitle: My Title\ndescription: A summary.\ndoc_id: my-title\nlayer: [brain]\nproject: bastion\nstatus: active\nkeywords: [okf]\nrelated: [other]\n---\nBody\n";

    // ── One case per ParseResult variant ─────────────────────────────────────

    #[test]
    fn ok_variant_with_all_fields_present_reports_clean() {
        let docs = [doc("a.md", FULL)];
        let family = assess_okf(&docs);
        assert_eq!(family.files_scanned, 1);
        assert_eq!(family.files_with_valid_frontmatter, 1);
        assert_eq!(family.missing_required_count, 0);
        assert!(family.missing_required_findings.is_empty());
        assert_eq!(family.invalid_frontmatter_count, 0);
        assert!(family.invalid_frontmatter_findings.is_empty());
        for field in OPTIONAL_FIELDS {
            assert_eq!(family.optional_field_coverage[field], 1, "field {field}");
        }
    }

    #[test]
    fn no_frontmatter_variant_is_reported_as_invalid_finding() {
        // The bare-.md-with-no-frontmatter case named in the block's acceptance criteria.
        let docs = [doc("bare.md", "# Just a heading\n\nNo frontmatter here.\n")];
        let family = assess_okf(&docs);
        assert_eq!(family.files_scanned, 1);
        assert_eq!(family.files_with_valid_frontmatter, 0);
        assert_eq!(family.invalid_frontmatter_count, 1);
        let finding = &family.invalid_frontmatter_findings[0];
        assert_eq!(finding.file, "bare.md");
        assert_eq!(finding.kind, "no_frontmatter");
        assert_eq!(finding.line, None);
        // No frontmatter to check required fields against.
        assert!(family.missing_required_findings.is_empty());
    }

    #[test]
    fn unterminated_fence_variant_is_reported_with_open_line() {
        let docs = [doc("open.md", "---\ntype: Doc\ntitle: T\n")];
        let family = assess_okf(&docs);
        assert_eq!(family.invalid_frontmatter_count, 1);
        let finding = &family.invalid_frontmatter_findings[0];
        assert_eq!(finding.file, "open.md");
        assert_eq!(finding.kind, "unterminated_fence");
        assert_eq!(finding.line, Some(1));
    }

    #[test]
    fn malformed_line_variant_is_reported_with_source_line() {
        let docs = [doc("bad.md", "---\ntype: Doc\nthis is not kv\n---\n")];
        let family = assess_okf(&docs);
        assert_eq!(family.invalid_frontmatter_count, 1);
        let finding = &family.invalid_frontmatter_findings[0];
        assert_eq!(finding.file, "bad.md");
        assert_eq!(finding.kind, "malformed_line");
        assert_eq!(finding.line, Some(3));
    }

    // ── Missing required fields, individually ────────────────────────────────

    #[test]
    fn missing_type_field_is_reported() {
        let docs = [doc("t.md", "---\ntitle: T\ndescription: D\n---\n")];
        let family = assess_okf(&docs);
        assert_eq!(family.missing_required_count, 1);
        assert_eq!(family.missing_required_findings[0].field, "type");
        assert_eq!(family.missing_required_findings[0].line, None);
    }

    #[test]
    fn missing_title_field_is_reported() {
        let docs = [doc("t.md", "---\ntype: Doc\ndescription: D\n---\n")];
        let family = assess_okf(&docs);
        assert_eq!(family.missing_required_count, 1);
        assert_eq!(family.missing_required_findings[0].field, "title");
    }

    #[test]
    fn missing_description_field_is_reported() {
        let docs = [doc("t.md", "---\ntype: Doc\ntitle: T\n---\n")];
        let family = assess_okf(&docs);
        assert_eq!(family.missing_required_count, 1);
        assert_eq!(family.missing_required_findings[0].field, "description");
    }

    #[test]
    fn empty_string_required_value_is_counted_as_missing_with_its_line() {
        let docs = [doc("t.md", "---\ntype: Doc\ntitle:\ndescription: D\n---\n")];
        let family = assess_okf(&docs);
        assert_eq!(family.missing_required_count, 1);
        let finding = &family.missing_required_findings[0];
        assert_eq!(finding.field, "title");
        assert_eq!(finding.file, "t.md");
        assert_eq!(finding.line, Some(3));
    }

    #[test]
    fn all_three_required_fields_missing_reports_all_three() {
        let docs = [doc("empty.md", "---\ndoc_id: x\n---\n")];
        let family = assess_okf(&docs);
        assert_eq!(family.missing_required_count, 3);
        let fields: Vec<&str> = family
            .missing_required_findings
            .iter()
            .map(|f| f.field.as_str())
            .collect();
        assert_eq!(fields, vec!["type", "title", "description"]);
    }

    #[test]
    fn fully_populated_file_passes_clean() {
        let docs = [doc("full.md", FULL)];
        let family = assess_okf(&docs);
        assert_eq!(family.missing_required_count, 0);
        assert_eq!(family.invalid_frontmatter_count, 0);
        assert_eq!(family.files_with_valid_frontmatter, 1);
    }

    #[test]
    fn optional_fields_absent_are_tallied_as_zero() {
        let docs = [doc(
            "t.md",
            "---\ntype: Doc\ntitle: T\ndescription: D\n---\n",
        )];
        let family = assess_okf(&docs);
        for field in OPTIONAL_FIELDS {
            assert_eq!(family.optional_field_coverage[field], 0, "field {field}");
        }
    }

    #[test]
    fn multiple_files_are_scanned_and_tallied_independently() {
        let docs = [
            doc("a.md", FULL),
            doc("b.md", "# no frontmatter\n"),
            doc("c.md", "---\ntype: Doc\ntitle: T\ndescription: D\n---\n"),
        ];
        let family = assess_okf(&docs);
        assert_eq!(family.files_scanned, 3);
        assert_eq!(family.files_with_valid_frontmatter, 2);
        assert_eq!(family.invalid_frontmatter_count, 1);
        assert_eq!(family.missing_required_count, 0);
    }
}
