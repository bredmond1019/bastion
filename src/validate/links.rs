// Link validation — relative link existence check.

use crate::validate::{ErrorKind, ValidationError};
use std::path::{Path, PathBuf};

// ── Link extraction ───────────────────────────────────────────────────────────

/// Extract all inline markdown links `[text](target)` from `content`.
///
/// Returns `(line_number, target)` pairs with 1-based line numbers.
/// Strips any title suffix (`"title"`, `'title'`) and surrounding whitespace.
pub fn extract_links(content: &str) -> Vec<(usize, String)> {
    let mut links = Vec::new();
    for (line_idx, line) in content.lines().enumerate() {
        let sanitized = blank_code_spans(line);
        extract_links_from_line(&sanitized, line_idx + 1, &mut links);
    }
    links
}

/// Replace the contents of inline backtick code spans with spaces so the link
/// scanner never sees `[text](target)` sequences that are inside code spans.
///
/// An unclosed backtick spans to end-of-line (conservative: prefer missed links
/// over false-positive broken-link errors).
fn blank_code_spans(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            out.push(' '); // replace opening backtick
            i += 1;
            while i < bytes.len() && bytes[i] != b'`' {
                out.push(' '); // blank span content
                i += 1;
            }
            if i < bytes.len() {
                out.push(' '); // replace closing backtick
                i += 1;
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn extract_links_from_line(line: &str, line_num: usize, out: &mut Vec<(usize, String)>) {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Find '['
        if bytes[i] != b'[' {
            i += 1;
            continue;
        }
        i += 1; // skip '['

        // Find matching ']' — skip nested brackets naively
        while i < len && bytes[i] != b']' {
            i += 1;
        }
        if i >= len {
            break;
        }
        i += 1; // skip ']'

        // Must be followed by '('
        if i >= len || bytes[i] != b'(' {
            continue;
        }
        i += 1; // skip '('

        // Find matching ')' — first occurrence is enough for well-formed links
        let target_start = i;
        while i < len && bytes[i] != b')' {
            i += 1;
        }
        if i >= len {
            break;
        }
        let raw_target = &line[target_start..i];
        i += 1; // skip ')'

        let target = strip_title(raw_target).trim().to_string();
        if !target.is_empty() {
            out.push((line_num, target));
        }
    }
}

/// Strip an optional title suffix from a raw link target.
///
/// `path/to/file.md "Title"` → `path/to/file.md`
/// `path/to/file.md 'Title'` → `path/to/file.md`
fn strip_title(raw: &str) -> &str {
    let raw = raw.trim();
    let mut prev_was_space = false;
    for (byte_i, c) in raw.char_indices() {
        if prev_was_space && (c == '"' || c == '\'') {
            // Trim the trailing whitespace before the title delimiter.
            return raw[..byte_i].trim_end();
        }
        prev_was_space = c.is_ascii_whitespace();
    }
    raw
}

// ── Link classification ───────────────────────────────────────────────────────

/// Returns `true` if the link target should be skipped (not checked).
///
/// Skipped:
/// - External URLs (`http://`, `https://`, `mailto:`)
/// - Pure in-page anchors (`#...`)
pub fn is_skipped_target(target: &str) -> bool {
    target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("mailto:")
        || target.starts_with('#')
}

/// Split a link target into `(file_path, fragment)`.
///
/// `"page.md#section"` → `("page.md", Some("section"))`
/// `"page.md"` → `("page.md", None)`
pub fn split_fragment(target: &str) -> (&str, Option<&str>) {
    if let Some(hash_pos) = target.find('#') {
        (&target[..hash_pos], Some(&target[hash_pos + 1..]))
    } else {
        (target, None)
    }
}

/// Resolve a relative link target against the containing file's directory.
///
/// Returns the resolved `PathBuf` for the file portion (fragment discarded).
pub fn resolve_link_path(target: &str, containing_file: &Path) -> PathBuf {
    let (file_part, _fragment) = split_fragment(target);
    let dir = containing_file.parent().unwrap_or(Path::new("."));
    dir.join(file_part)
}

// ── Validation ────────────────────────────────────────────────────────────────

/// Validate relative links found in `content` (from `file`).
///
/// Skips external URLs and pure anchors.  For each relative link, resolves
/// against the containing file's directory and emits `BrokenLink` when the
/// target path does not exist on disk.
pub fn validate_links(content: &str, file: &Path) -> Vec<ValidationError> {
    let links = extract_links(content);
    let mut errors = Vec::new();

    for (line, target) in links {
        if is_skipped_target(&target) {
            continue;
        }
        let resolved = resolve_link_path(&target, file);
        if !resolved.exists() {
            errors.push(ValidationError {
                file: file.to_path_buf(),
                line,
                kind: ErrorKind::BrokenLink,
                message: format!("broken link: {target}"),
            });
        }
    }

    errors
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    // ── Minimal temp-dir helper (no extra dep) ────────────────────────────────

    struct TempDir(PathBuf);

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    impl TempDir {
        fn new() -> Self {
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("bastion-links-test-{}-{id}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    // Helper: create a file with given content inside the temp dir.
    fn make_file(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content).unwrap();
        path
    }

    // ── extract_links ─────────────────────────────────────────────────────────

    #[test]
    fn no_links_returns_empty() {
        assert_eq!(extract_links("No links here.\nJust text."), vec![]);
    }

    #[test]
    fn single_link_basic() {
        let result = extract_links("[Click](page.md)");
        assert_eq!(result, vec![(1, "page.md".to_string())]);
    }

    #[test]
    fn link_line_number_is_one_based() {
        let content = "First line\n[Link](file.md)\nThird line";
        let result = extract_links(content);
        assert_eq!(result, vec![(2, "file.md".to_string())]);
    }

    #[test]
    fn multiple_links_on_same_line() {
        let result = extract_links("[A](a.md) and [B](b.md) and [C](c.md)");
        assert_eq!(
            result,
            vec![
                (1, "a.md".to_string()),
                (1, "b.md".to_string()),
                (1, "c.md".to_string()),
            ]
        );
    }

    #[test]
    fn links_across_multiple_lines() {
        let content = "[First](first.md)\n\n[Second](second.md)\n[Third](third.md)";
        let result = extract_links(content);
        assert_eq!(
            result,
            vec![
                (1, "first.md".to_string()),
                (3, "second.md".to_string()),
                (4, "third.md".to_string()),
            ]
        );
    }

    #[test]
    fn link_with_double_quoted_title_strips_title() {
        let result = extract_links(r#"[Page](page.md "My Title")"#);
        assert_eq!(result, vec![(1, "page.md".to_string())]);
    }

    #[test]
    fn link_with_single_quoted_title_strips_title() {
        let result = extract_links("[Page](page.md 'My Title')");
        assert_eq!(result, vec![(1, "page.md".to_string())]);
    }

    #[test]
    fn link_with_fragment_preserved_in_target() {
        let result = extract_links("[Section](page.md#section)");
        assert_eq!(result, vec![(1, "page.md#section".to_string())]);
    }

    #[test]
    fn pure_anchor_link_included_in_extract() {
        // extract_links captures everything; classification is a separate step
        let result = extract_links("[Anchor](#heading)");
        assert_eq!(result, vec![(1, "#heading".to_string())]);
    }

    #[test]
    fn external_url_included_in_extract() {
        let result = extract_links("[Visit](https://example.com)");
        assert_eq!(result, vec![(1, "https://example.com".to_string())]);
    }

    #[test]
    fn mailto_included_in_extract() {
        let result = extract_links("[Email](mailto:user@example.com)");
        assert_eq!(result, vec![(1, "mailto:user@example.com".to_string())]);
    }

    #[test]
    fn image_syntax_is_also_captured() {
        // Image links `![alt](src)` — the `!` is before `[`, so the `[alt](src)` part
        // is still captured; callers that only want hyperlinks can filter by context,
        // but the extractor itself is indifferent.
        let result = extract_links("![Image](image.png)");
        assert_eq!(result, vec![(1, "image.png".to_string())]);
    }

    #[test]
    fn empty_target_is_skipped() {
        // `[text]()` has an empty target — should not produce an entry.
        let result = extract_links("[text]()");
        assert_eq!(result, vec![]);
    }

    // ── is_skipped_target ─────────────────────────────────────────────────────

    #[test]
    fn http_url_is_skipped() {
        assert!(is_skipped_target("http://example.com"));
    }

    #[test]
    fn https_url_is_skipped() {
        assert!(is_skipped_target("https://example.com/path?q=1"));
    }

    #[test]
    fn mailto_is_skipped() {
        assert!(is_skipped_target("mailto:someone@example.com"));
    }

    #[test]
    fn pure_anchor_is_skipped() {
        assert!(is_skipped_target("#section-heading"));
    }

    #[test]
    fn relative_path_is_not_skipped() {
        assert!(!is_skipped_target("page.md"));
    }

    #[test]
    fn relative_path_with_fragment_is_not_skipped() {
        assert!(!is_skipped_target("page.md#section"));
    }

    #[test]
    fn absolute_path_is_not_skipped() {
        assert!(!is_skipped_target("/docs/page.md"));
    }

    // ── split_fragment ────────────────────────────────────────────────────────

    #[test]
    fn split_no_fragment() {
        assert_eq!(split_fragment("page.md"), ("page.md", None));
    }

    #[test]
    fn split_with_fragment() {
        assert_eq!(
            split_fragment("page.md#section"),
            ("page.md", Some("section"))
        );
    }

    #[test]
    fn split_pure_anchor() {
        assert_eq!(split_fragment("#heading"), ("", Some("heading")));
    }

    #[test]
    fn split_fragment_with_subpath() {
        assert_eq!(
            split_fragment("docs/guide.md#intro"),
            ("docs/guide.md", Some("intro"))
        );
    }

    // ── resolve_link_path ─────────────────────────────────────────────────────

    #[test]
    fn resolve_sibling_file() {
        let containing = Path::new("/docs/index.md");
        let resolved = resolve_link_path("guide.md", containing);
        assert_eq!(resolved, PathBuf::from("/docs/guide.md"));
    }

    #[test]
    fn resolve_subdirectory_link() {
        let containing = Path::new("/docs/index.md");
        let resolved = resolve_link_path("sub/page.md", containing);
        assert_eq!(resolved, PathBuf::from("/docs/sub/page.md"));
    }

    #[test]
    fn resolve_parent_directory_link() {
        let containing = Path::new("/docs/sub/page.md");
        let resolved = resolve_link_path("../index.md", containing);
        assert_eq!(resolved, PathBuf::from("/docs/sub/../index.md"));
    }

    #[test]
    fn resolve_strips_fragment() {
        let containing = Path::new("/docs/index.md");
        let resolved = resolve_link_path("guide.md#section", containing);
        assert_eq!(resolved, PathBuf::from("/docs/guide.md"));
    }

    // ── validate_links (I/O shell with temp-file fixtures) ───────────────────

    #[test]
    fn valid_relative_link_to_existing_sibling() {
        let tmp = TempDir::new();
        let sibling = make_file(tmp.path(), "sibling.md", "# Sibling");
        let source_content = format!("[Sibling](sibling.md)");
        let source = make_file(tmp.path(), "source.md", &source_content);

        let errors = validate_links(&source_content, &source);
        assert!(
            errors.is_empty(),
            "expected no errors for existing sibling, got: {errors:?}"
        );
        drop(sibling);
    }

    #[test]
    fn broken_relative_link_emits_error() {
        let tmp = TempDir::new();
        let content = "[Missing](missing.md)";
        let source = make_file(tmp.path(), "source.md", content);

        let errors = validate_links(content, &source);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, ErrorKind::BrokenLink);
        assert_eq!(errors[0].line, 1);
        assert!(errors[0].message.contains("missing.md"));
    }

    #[test]
    fn external_url_is_not_flagged() {
        let tmp = TempDir::new();
        let content = "[Visit](https://example.com)";
        let source = make_file(tmp.path(), "source.md", content);

        let errors = validate_links(content, &source);
        assert!(errors.is_empty(), "external URL should not be flagged");
    }

    #[test]
    fn http_url_is_not_flagged() {
        let tmp = TempDir::new();
        let content = "[Visit](http://example.com/page)";
        let source = make_file(tmp.path(), "source.md", content);

        let errors = validate_links(content, &source);
        assert!(errors.is_empty());
    }

    #[test]
    fn mailto_is_not_flagged() {
        let tmp = TempDir::new();
        let content = "[Email](mailto:person@example.com)";
        let source = make_file(tmp.path(), "source.md", content);

        let errors = validate_links(content, &source);
        assert!(errors.is_empty());
    }

    #[test]
    fn pure_anchor_is_not_flagged() {
        let tmp = TempDir::new();
        let content = "[Section](#intro)";
        let source = make_file(tmp.path(), "source.md", content);

        let errors = validate_links(content, &source);
        assert!(errors.is_empty(), "pure anchor should not be flagged");
    }

    #[test]
    fn fragment_link_checks_file_portion_only() {
        let tmp = TempDir::new();
        // Create the sibling file so the link resolves successfully.
        let _sibling = make_file(tmp.path(), "guide.md", "# Guide\n## Intro\n");
        let content = "[Intro](guide.md#intro)";
        let source = make_file(tmp.path(), "source.md", content);

        let errors = validate_links(content, &source);
        assert!(
            errors.is_empty(),
            "link with fragment to existing file should not be flagged"
        );
    }

    #[test]
    fn fragment_link_with_broken_file_portion_is_flagged() {
        let tmp = TempDir::new();
        let content = "[Intro](missing.md#intro)";
        let source = make_file(tmp.path(), "source.md", content);

        let errors = validate_links(content, &source);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, ErrorKind::BrokenLink);
        assert!(errors[0].message.contains("missing.md#intro"));
    }

    #[test]
    fn mixed_content_only_broken_links_flagged() {
        let tmp = TempDir::new();
        let _good = make_file(tmp.path(), "good.md", "# Good");
        let content = [
            "[Good](good.md)",
            "[External](https://example.com)",
            "[Anchor](#section)",
            "[Broken](broken.md)",
            "[Also broken](other-missing.md)",
        ]
        .join("\n");
        let source = make_file(tmp.path(), "source.md", &content);

        let errors = validate_links(&content, &source);
        assert_eq!(errors.len(), 2, "expected exactly 2 broken-link errors");

        let targets: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
        assert!(
            targets.iter().any(|m| m.contains("broken.md")),
            "missing broken.md error"
        );
        assert!(
            targets.iter().any(|m| m.contains("other-missing.md")),
            "missing other-missing.md error"
        );
    }

    #[test]
    fn correct_line_numbers_reported() {
        let tmp = TempDir::new();
        let content = "Line 1\n[Broken](missing1.md)\nLine 3\n[Broken2](missing2.md)";
        let source = make_file(tmp.path(), "source.md", content);

        let errors = validate_links(content, &source);
        assert_eq!(errors.len(), 2);
        // Find each by target
        let err1 = errors
            .iter()
            .find(|e| e.message.contains("missing1.md"))
            .unwrap();
        let err2 = errors
            .iter()
            .find(|e| e.message.contains("missing2.md"))
            .unwrap();
        assert_eq!(err1.line, 2);
        assert_eq!(err2.line, 4);
    }

    // ── blank_code_spans / backtick suppression ───────────────────────────────

    #[test]
    fn link_inside_backtick_span_not_extracted() {
        // `[text](target)` is inside a code span — should produce no links.
        let result = extract_links("Use `[text](target)` syntax to write links.");
        assert_eq!(
            result,
            vec![],
            "link inside backtick span must be suppressed"
        );
    }

    #[test]
    fn link_outside_backtick_span_still_extracted() {
        let result = extract_links("See `code` and [real](real.md) for details.");
        assert_eq!(result, vec![(1, "real.md".to_string())]);
    }

    #[test]
    fn multiple_code_spans_on_same_line() {
        let result = extract_links("`[skip](a.md)` and [keep](b.md) and `[skip2](c.md)`");
        assert_eq!(result, vec![(1, "b.md".to_string())]);
    }

    #[test]
    fn unclosed_backtick_blanks_rest_of_line() {
        // Unclosed backtick — conservative: blank to end-of-line to avoid false positives.
        let result = extract_links("before ` unclosed [fake](fake.md)");
        assert_eq!(result, vec![]);
    }

    #[test]
    fn link_with_title_resolved_correctly() {
        let tmp = TempDir::new();
        let _sibling = make_file(tmp.path(), "page.md", "# Page");
        let content = r#"[Page](page.md "The Title")"#;
        let source = make_file(tmp.path(), "source.md", content);

        let errors = validate_links(content, &source);
        assert!(
            errors.is_empty(),
            "link with title to existing file should not be flagged"
        );
    }
}
