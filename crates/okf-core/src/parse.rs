// OKF frontmatter parser.

use std::collections::HashMap;

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
pub enum ParseResult {
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
pub fn extract_frontmatter(content: &str) -> ParseResult {
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
/// errors) use this instead of a validation-layer wrapper.
pub fn parse_frontmatter(content: &str) -> Option<Frontmatter> {
    match extract_frontmatter(content) {
        ParseResult::Ok(fm) => Some(fm),
        _ => None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
}
