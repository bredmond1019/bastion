//! Pure `handoff.md` reader for the `bastion serve` repo-status surface.
//!
//! Extracts a title (from frontmatter `title:` if present, else the
//! `# Handoff —` / `# Handoff -` heading) and the raw markdown body from a
//! `handoff.md` file's content string. No I/O happens here — callers (Task 4
//! handlers) read the file and pass the content in.

use serde::{Deserialize, Serialize};

use crate::validate::frontmatter::parse_frontmatter;

// ── Shared types ──────────────────────────────────────────────────────────────

/// Parsed view of a sub-project's `planning/handoff.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffInfo {
    /// Title — frontmatter `title:` scalar if present, else the text after
    /// the first `# Handoff —`/`# Handoff -` heading, else an empty string.
    pub title: String,
    /// The full raw markdown content (including frontmatter, if any).
    pub body: String,
}

// ── Pure parsing ──────────────────────────────────────────────────────────────

/// Parse `content` (the full text of a `handoff.md` file) into a [`HandoffInfo`].
///
/// Returns `None` when `content` is empty/whitespace-only. Otherwise always
/// succeeds — title extraction is best-effort and falls back to an empty
/// string when neither a frontmatter `title:` nor a `# Handoff —` heading
/// is found.
pub fn read_handoff(content: &str) -> Option<HandoffInfo> {
    if content.trim().is_empty() {
        return None;
    }

    let title = parse_frontmatter(content)
        .and_then(|fm| fm.fields.get("title").map(|(v, _)| unquote(v)))
        .or_else(|| heading_title(content))
        .unwrap_or_default();

    Some(HandoffInfo {
        title,
        body: content.to_string(),
    })
}

/// Strip a single layer of surrounding double quotes, if present.
fn unquote(v: &str) -> String {
    let trimmed = v.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}

/// Find the first `# Handoff —` (em dash) or `# Handoff -` (hyphen) heading
/// line and return the text after the dash, trimmed.
fn heading_title(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("# Handoff") {
            let rest = rest.trim_start();
            let text = rest
                .strip_prefix('—')
                .or_else(|| rest.strip_prefix('-'))
                .unwrap_or(rest)
                .trim();
            if !text.is_empty() {
                return Some(text.to_string());
            }
            // "# Handoff" with no trailing text — fall back to "Handoff".
            return Some("Handoff".to_string());
        }
    }
    None
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const WITH_FRONTMATTER: &str = include_str!("fixtures/handoff_minimal.md");
    const HEADING_ONLY: &str = include_str!("fixtures/handoff_heading_only.md");

    #[test]
    fn parses_title_from_frontmatter() {
        let info = read_handoff(WITH_FRONTMATTER).expect("should parse");
        assert_eq!(info.title, "Handoff — minimal fixture");
        assert!(info.body.contains("# Handoff"));
    }

    #[test]
    fn falls_back_to_heading_when_no_frontmatter() {
        let info = read_handoff(HEADING_ONLY).expect("should parse");
        assert_eq!(info.title, "no frontmatter here");
    }

    #[test]
    fn empty_input_returns_none() {
        assert!(read_handoff("").is_none());
        assert!(read_handoff("   \n\n  ").is_none());
    }

    #[test]
    fn body_preserves_full_content() {
        let info = read_handoff(WITH_FRONTMATTER).expect("should parse");
        assert_eq!(info.body, WITH_FRONTMATTER);
    }

    #[test]
    fn no_title_source_yields_empty_title() {
        let content = "Just some text with no heading or frontmatter.\n";
        let info = read_handoff(content).expect("should parse");
        assert_eq!(info.title, "");
    }

    #[test]
    fn heading_title_handles_bare_heading() {
        assert_eq!(
            heading_title("# Handoff\n\nbody"),
            Some("Handoff".to_string())
        );
        assert_eq!(heading_title("# Handoff — text"), Some("text".to_string()));
        assert_eq!(heading_title("no heading here"), None);
    }
}
