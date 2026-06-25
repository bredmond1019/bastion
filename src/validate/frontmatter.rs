// Frontmatter validation — OKF required fields (`type`, `title`, `description`).

use crate::validate::{ErrorKind, ValidationError};
use std::collections::HashMap;
use std::path::Path;

/// Required OKF frontmatter fields.
const REQUIRED_FIELDS: &[&str] = &["type", "title", "description"];

// ── Internal types ────────────────────────────────────────────────────────────

/// A parsed frontmatter block.
///
/// - `fields`: map from field name to `(value, 1-based line number in the source file)`.
/// - `open_line`: 1-based line of the opening `---` fence.
/// - `close_line`: 1-based line of the closing `---` fence.
#[derive(Debug, PartialEq, Eq)]
pub struct Frontmatter {
    /// field name -> (trimmed value, 1-based source line number)
    pub fields: HashMap<String, (String, usize)>,
    pub open_line: usize,
    pub close_line: usize,
}

/// Result of attempting to parse a frontmatter block.
#[derive(Debug, PartialEq, Eq)]
enum ParseResult {
    /// Block found and fully parsed.
    Ok(Frontmatter),
    /// Opening `---` found but no closing fence before EOF.
    UnterminatedFence { open_line: usize },
    /// A line inside the block is not a valid `key: value` pair.
    MalformedLine { source_line: usize },
    /// File does not start with a frontmatter fence.
    NoFrontmatter,
}

// ── Pure parsing ──────────────────────────────────────────────────────────────

/// Parse the leading YAML-style `---` frontmatter from `content`.
///
/// Rules:
/// - The very first line must be exactly `---` (after trimming trailing whitespace).
/// - Lines between the fences must be `key: value` pairs (trimmed). Blank lines inside
///   the block are treated as malformed to match strict OKF expectations.
/// - The closing `---` terminates the block. Returns `ParseResult::Ok` on success.
fn extract_frontmatter(content: &str) -> ParseResult {
    let mut lines = content.lines().enumerate().peekable(); // (0-based idx, &str)

    // First line must be the opening fence.
    match lines.next() {
        Some((_, line)) if line.trim_end() == "---" => {}
        _ => return ParseResult::NoFrontmatter,
    }
    let open_line = 1usize; // 1-based

    let mut fields: HashMap<String, (String, usize)> = HashMap::new();

    for (idx, line) in lines {
        let source_line = idx + 1; // convert 0-based to 1-based
        let trimmed = line.trim_end();

        if trimmed == "---" {
            // Closing fence found.
            return ParseResult::Ok(Frontmatter {
                fields,
                open_line,
                close_line: source_line,
            });
        }

        // Try to parse as `key: value`.
        if let Some(colon_pos) = trimmed.find(':') {
            let key = trimmed[..colon_pos].trim().to_string();
            let value = trimmed[colon_pos + 1..].trim().to_string();

            if key.is_empty() {
                // A line like `: value` or just `:` has an empty key — malformed.
                return ParseResult::MalformedLine { source_line };
            }

            // Duplicate keys: last wins (YAML semantics), no error emitted.
            fields.insert(key, (value, source_line));
        } else {
            // Line inside the block is not a `key: value` pair.
            return ParseResult::MalformedLine { source_line };
        }
    }

    // Reached EOF without finding the closing fence.
    ParseResult::UnterminatedFence { open_line }
}

/// Parse frontmatter from `content`, returning the block if present and well-formed.
///
/// Returns `None` if the file has no frontmatter, the fence is unterminated, or any
/// interior line is malformed. Callers that only need field values (not validation
/// errors) use this instead of `validate_frontmatter`.
pub(crate) fn parse_frontmatter(content: &str) -> Option<Frontmatter> {
    match extract_frontmatter(content) {
        ParseResult::Ok(fm) => Some(fm),
        _ => None,
    }
}

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

    // ── extract_frontmatter ───────────────────────────────────────────────────

    #[test]
    fn extract_valid_frontmatter() {
        let content = "---\ntype: Doc\ntitle: Hello\ndescription: A test.\n---\n# Body\n";
        match extract_frontmatter(content) {
            ParseResult::Ok(fm) => {
                assert_eq!(fm.open_line, 1);
                assert_eq!(fm.close_line, 5);
                assert_eq!(fm.fields["type"], ("Doc".into(), 2));
                assert_eq!(fm.fields["title"], ("Hello".into(), 3));
                assert_eq!(fm.fields["description"], ("A test.".into(), 4));
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[test]
    fn extract_no_frontmatter_plain_text() {
        let content = "# Just a heading\n\nNo frontmatter here.\n";
        assert_eq!(extract_frontmatter(content), ParseResult::NoFrontmatter);
    }

    #[test]
    fn extract_no_frontmatter_empty_file() {
        assert_eq!(extract_frontmatter(""), ParseResult::NoFrontmatter);
    }

    #[test]
    fn extract_unterminated_fence() {
        let content = "---\ntype: Doc\ntitle: Hello\n";
        assert_eq!(
            extract_frontmatter(content),
            ParseResult::UnterminatedFence { open_line: 1 }
        );
    }

    #[test]
    fn extract_malformed_inner_line_no_colon() {
        let content = "---\ntype: Doc\nthis is not kv\n---\n";
        assert_eq!(
            extract_frontmatter(content),
            ParseResult::MalformedLine { source_line: 3 }
        );
    }

    #[test]
    fn extract_malformed_empty_key() {
        let content = "---\n: value\n---\n";
        assert_eq!(
            extract_frontmatter(content),
            ParseResult::MalformedLine { source_line: 2 }
        );
    }

    #[test]
    fn extract_value_with_colon_in_it() {
        // Values may contain colons — only the first colon splits key/value.
        let content = "---\ntitle: Hello: World\n---\n";
        match extract_frontmatter(content) {
            ParseResult::Ok(fm) => {
                assert_eq!(fm.fields["title"].0, "Hello: World");
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[test]
    fn extract_empty_value_is_ok_at_parse_level() {
        // An empty value is structurally valid; the semantic check happens in validate.
        let content = "---\ntitle: \n---\n";
        match extract_frontmatter(content) {
            ParseResult::Ok(fm) => {
                assert_eq!(fm.fields["title"].0, "");
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[test]
    fn extract_line_numbers_are_one_based() {
        let content = "---\ntype: T\ntitle: Ti\ndescription: D\n---\n";
        match extract_frontmatter(content) {
            ParseResult::Ok(fm) => {
                assert_eq!(fm.fields["type"].1, 2);
                assert_eq!(fm.fields["title"].1, 3);
                assert_eq!(fm.fields["description"].1, 4);
                assert_eq!(fm.close_line, 5);
            }
            other => panic!("expected Ok, got {other:?}"),
        }
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
