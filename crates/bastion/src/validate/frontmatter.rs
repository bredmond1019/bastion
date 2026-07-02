// Frontmatter validation — OKF required fields (`type`, `title`, `description`).

use crate::validate::{ErrorKind, ValidationError};
use std::path::Path;

// `Frontmatter` is not referenced by name in this module, but is re-exported so that
// downstream consumers (and any future callers in this crate) can name the type via
// `crate::validate::frontmatter::Frontmatter` without reaching into `okf_core` directly.
#[allow(unused_imports)]
pub use okf_core::{Frontmatter, ParseResult, extract_frontmatter, parse_frontmatter};

/// Required OKF frontmatter fields.
const REQUIRED_FIELDS: &[&str] = &["type", "title", "description"];

// ── Validation ────────────────────────────────────────────────────────────────

/// Validate the OKF frontmatter of `content` belonging to `file`.
///
/// Errors produced:
/// - `MissingFrontmatter` (line 1) — file does not start with a `---` fence.
/// - `MalformedFrontmatter` — opening fence with no closing `---`, or a non-`key: value`
///   line inside the block. Line points at the offending source line.
/// - `MissingField` — a required field is entirely absent from the block.
/// - `EmptyField` — a required field is present but its value is empty/whitespace.
pub fn validate_frontmatter(content: &str, file: &Path) -> Vec<ValidationError> {
    let mut errors: Vec<ValidationError> = Vec::new();

    match extract_frontmatter(content) {
        ParseResult::NoFrontmatter => {
            errors.push(ValidationError {
                file: file.to_path_buf(),
                line: 1,
                kind: ErrorKind::MissingFrontmatter,
                message: "file does not have a leading frontmatter block".into(),
            });
        }

        ParseResult::UnterminatedFence { open_line } => {
            errors.push(ValidationError {
                file: file.to_path_buf(),
                line: open_line,
                kind: ErrorKind::MalformedFrontmatter,
                message: "frontmatter block opened with `---` but has no closing `---`".into(),
            });
        }

        ParseResult::MalformedLine { source_line } => {
            errors.push(ValidationError {
                file: file.to_path_buf(),
                line: source_line,
                kind: ErrorKind::MalformedFrontmatter,
                message: "frontmatter line is not a valid `key: value` pair".into(),
            });
        }

        ParseResult::Ok(fm) => {
            for &field in REQUIRED_FIELDS {
                match fm.fields.get(field) {
                    None => {
                        errors.push(ValidationError {
                            file: file.to_path_buf(),
                            // Point at the close fence line — it is the last line of the block.
                            line: fm.close_line,
                            kind: ErrorKind::MissingField,
                            message: format!("required field `{field}` is missing"),
                        });
                    }
                    Some((value, src_line)) if value.trim().is_empty() => {
                        errors.push(ValidationError {
                            file: file.to_path_buf(),
                            line: *src_line,
                            kind: ErrorKind::EmptyField,
                            message: format!("required field `{field}` is present but empty"),
                        });
                    }
                    Some(_) => {} // field present and non-empty — OK
                }
            }
        }
    }

    errors
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn path() -> PathBuf {
        PathBuf::from("test.md")
    }

    // ── validate_frontmatter — structural errors ──────────────────────────────

    #[test]
    fn validate_no_frontmatter() {
        let errs = validate_frontmatter("# Heading\n", &path());
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].kind, ErrorKind::MissingFrontmatter);
        assert_eq!(errs[0].line, 1);
    }

    #[test]
    fn validate_unterminated_fence() {
        let content = "---\ntype: Doc\ntitle: T\ndescription: D\n";
        let errs = validate_frontmatter(content, &path());
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].kind, ErrorKind::MalformedFrontmatter);
        assert_eq!(errs[0].line, 1);
        assert!(errs[0].message.contains("no closing"));
    }

    #[test]
    fn validate_malformed_inner_line() {
        let content = "---\ntype: Doc\nnot-kv-format\n---\n";
        let errs = validate_frontmatter(content, &path());
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].kind, ErrorKind::MalformedFrontmatter);
        assert_eq!(errs[0].line, 3);
        assert!(errs[0].message.contains("key: value"));
    }

    // ── validate_frontmatter — field checks ───────────────────────────────────

    #[test]
    fn validate_valid_full_frontmatter_no_errors() {
        let content = "---\ntype: Doc\ntitle: My Title\ndescription: A description.\n---\n";
        let errs = validate_frontmatter(content, &path());
        assert!(errs.is_empty(), "expected no errors, got {errs:?}");
    }

    #[test]
    fn validate_missing_type_field() {
        let content = "---\ntitle: My Title\ndescription: A description.\n---\n";
        let errs = validate_frontmatter(content, &path());
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].kind, ErrorKind::MissingField);
        assert!(
            errs[0].message.contains("`type`"),
            "message: {}",
            errs[0].message
        );
    }

    #[test]
    fn validate_missing_title_field() {
        let content = "---\ntype: Doc\ndescription: A description.\n---\n";
        let errs = validate_frontmatter(content, &path());
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].kind, ErrorKind::MissingField);
        assert!(
            errs[0].message.contains("`title`"),
            "message: {}",
            errs[0].message
        );
    }

    #[test]
    fn validate_missing_description_field() {
        let content = "---\ntype: Doc\ntitle: My Title\n---\n";
        let errs = validate_frontmatter(content, &path());
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].kind, ErrorKind::MissingField);
        assert!(
            errs[0].message.contains("`description`"),
            "message: {}",
            errs[0].message
        );
    }

    #[test]
    fn validate_all_fields_missing() {
        let content = "---\nauthor: Someone\n---\n";
        let errs = validate_frontmatter(content, &path());
        // All three required fields missing.
        assert_eq!(errs.len(), 3);
        let kinds: Vec<_> = errs.iter().map(|e| &e.kind).collect();
        assert!(kinds.iter().all(|k| **k == ErrorKind::MissingField));
        let messages: Vec<_> = errs.iter().map(|e| e.message.as_str()).collect();
        assert!(messages.iter().any(|m| m.contains("`type`")));
        assert!(messages.iter().any(|m| m.contains("`title`")));
        assert!(messages.iter().any(|m| m.contains("`description`")));
    }

    #[test]
    fn validate_empty_type_value() {
        let content = "---\ntype: \ntitle: T\ndescription: D\n---\n";
        let errs = validate_frontmatter(content, &path());
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].kind, ErrorKind::EmptyField);
        assert_eq!(errs[0].line, 2); // line of the `type:` entry
        assert!(errs[0].message.contains("`type`"));
    }

    #[test]
    fn validate_empty_title_value() {
        let content = "---\ntype: Doc\ntitle:  \ndescription: D\n---\n";
        let errs = validate_frontmatter(content, &path());
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].kind, ErrorKind::EmptyField);
        assert_eq!(errs[0].line, 3); // line of the `title:` entry
        assert!(errs[0].message.contains("`title`"));
    }

    #[test]
    fn validate_empty_description_value() {
        let content = "---\ntype: Doc\ntitle: T\ndescription:\n---\n";
        let errs = validate_frontmatter(content, &path());
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].kind, ErrorKind::EmptyField);
        assert_eq!(errs[0].line, 4); // line of the `description:` entry
        assert!(errs[0].message.contains("`description`"));
    }

    #[test]
    fn validate_whitespace_only_value_is_empty() {
        // A value of only spaces/tabs should be treated as empty.
        let content = "---\ntype: Doc\ntitle: T\ndescription:   \t  \n---\n";
        let errs = validate_frontmatter(content, &path());
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].kind, ErrorKind::EmptyField);
        assert!(errs[0].message.contains("`description`"));
    }

    #[test]
    fn validate_missing_field_line_points_at_close_fence() {
        // When a field is absent, line should be the close-fence line (inside the block span).
        let content = "---\ntype: Doc\ntitle: T\n---\n";
        let errs = validate_frontmatter(content, &path());
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].kind, ErrorKind::MissingField);
        // close_line is 4 (the `---` on line 4).
        assert_eq!(errs[0].line, 4);
    }

    #[test]
    fn validate_file_path_is_preserved() {
        let p = PathBuf::from("some/dir/file.md");
        let content = "# no frontmatter";
        let errs = validate_frontmatter(content, &p);
        assert_eq!(errs[0].file, p);
    }
}
