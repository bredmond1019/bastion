//! Pure `status.md` parser for the `bastion serve` repo-status surface.
//!
//! Extracts the D30 frontmatter scalars (`now`, `next`, `blocked`) and the
//! five `## Momentum` queue lines from a `status.md` file's content string.
//! No I/O happens here — callers (Task 4 handlers) read the file and pass the
//! content in; `repo` (the workspace name) and `has_handoff` are not derivable
//! from the file content alone, so callers fill them in after parsing.

use serde::{Deserialize, Serialize};

use crate::validate::frontmatter::parse_frontmatter;

// ── Shared types ──────────────────────────────────────────────────────────────

/// Parsed view of a sub-project's `planning/status.md`.
///
/// `name` and `has_handoff` are not present in `status.md` itself — they
/// default to `String::new()` / `false` out of [`parse_status`] and are set by
/// the caller (the `GET /repos*` handlers in Task 4) from the workspace
/// registry entry and a separate `handoff.md` existence check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoStatus {
    /// Workspace registry name — filled in by the caller, empty from `parse_status`.
    #[serde(default)]
    pub name: String,
    /// Frontmatter `now:` scalar.
    pub now: String,
    /// Frontmatter `next:` scalar.
    pub next: String,
    /// Frontmatter `blocked:` scalar (raw string — may be a YAML-flow-style
    /// list like `[]` or a quoted sentence; this parser does not interpret
    /// YAML sequences, only scalar lines).
    pub blocked: String,
    /// Whether `planning/handoff.md` exists — filled in by the caller.
    #[serde(default)]
    pub has_handoff: bool,
    /// Body `## Momentum` → `- **now** — ...` line text (after the dash).
    pub momentum_now: String,
    /// Body `## Momentum` → `- **next** — ...` line text.
    pub momentum_next: String,
    /// Body `## Momentum` → `- **blocked** — ...` line text.
    pub momentum_blocked: String,
    /// Body `## Momentum` → `- **improve** — ...` line text.
    pub momentum_improve: String,
    /// Body `## Momentum` → `- **recurring** — ...` line text.
    pub momentum_recurring: String,
}

// ── Pure parsing ──────────────────────────────────────────────────────────────

/// Parse `content` (the full text of a `status.md` file) into a [`RepoStatus`].
///
/// Returns `None` when the file has no well-formed leading `---` frontmatter
/// block (matches [`parse_frontmatter`]'s rules: missing fence, unterminated
/// fence, or a malformed interior line all yield `None`).
///
/// The `## Momentum` queue lines are best-effort: a missing `## Momentum`
/// section, or a section missing one or more of the five queue bullets,
/// yields empty strings for the missing entries rather than failing the
/// whole parse.
pub fn parse_status(content: &str) -> Option<RepoStatus> {
    let fm = parse_frontmatter(content)?;

    let now = fm
        .fields
        .get("now")
        .map(|(v, _)| unquote(v))
        .unwrap_or_default();
    let next = fm
        .fields
        .get("next")
        .map(|(v, _)| unquote(v))
        .unwrap_or_default();
    let blocked = fm
        .fields
        .get("blocked")
        .map(|(v, _)| unquote(v))
        .unwrap_or_default();

    let momentum = parse_momentum(content);

    Some(RepoStatus {
        name: String::new(),
        now,
        next,
        blocked,
        has_handoff: false,
        momentum_now: momentum.now,
        momentum_next: momentum.next,
        momentum_blocked: momentum.blocked,
        momentum_improve: momentum.improve,
        momentum_recurring: momentum.recurring,
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

/// The five `## Momentum` queue line texts, parsed independently of frontmatter.
#[derive(Debug, Default)]
struct Momentum {
    now: String,
    next: String,
    blocked: String,
    improve: String,
    recurring: String,
}

/// Scan the body for `## Momentum` and extract `- **<key>** — <text>` bullet
/// lines for the five known keys. Lines outside the `## Momentum` section
/// (terminated by the next `##` heading, or EOF) are ignored.
fn parse_momentum(content: &str) -> Momentum {
    let mut out = Momentum::default();

    let mut in_section = false;
    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("## ") {
            in_section = trimmed.trim_start_matches('#').trim() == "Momentum";
            continue;
        }

        if !in_section {
            continue;
        }

        if let Some((key, text)) = parse_bullet(trimmed) {
            match key.as_str() {
                "now" => out.now = text,
                "next" => out.next = text,
                "blocked" => out.blocked = text,
                "improve" => out.improve = text,
                "recurring" => out.recurring = text,
                _ => {}
            }
        }
    }

    out
}

/// Parse a single `- **key** — text` bullet line into `(key, text)`.
///
/// Accepts either an em dash (`—`) or a plain hyphen (`-`) as the
/// key/text separator, matching loose authoring style in `status.md` files.
fn parse_bullet(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("- ")?;
    let rest = rest.strip_prefix("**")?;
    let (key, rest) = rest.split_once("**")?;
    let rest = rest.trim_start();
    let text = rest
        .strip_prefix('—')
        .or_else(|| rest.strip_prefix('-'))
        .unwrap_or(rest)
        .trim();
    Some((key.to_string(), text.to_string()))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const WELL_FORMED: &str = include_str!("fixtures/status_well_formed.md");
    const MALFORMED: &str = include_str!("fixtures/status_malformed.md");
    const EMPTY_MOMENTUM: &str = include_str!("fixtures/status_empty_momentum.md");

    #[test]
    fn parses_well_formed_status() {
        let status = parse_status(WELL_FORMED).expect("should parse");
        assert_eq!(status.now, "BA.11.D in progress — repo status API");
        assert_eq!(status.next, "Wire WS event push");
        assert_eq!(status.blocked, "[]");
        assert_eq!(status.momentum_now, "BA.11.D in progress — repo status API");
        assert_eq!(status.momentum_next, "Wire WS event push");
        assert_eq!(status.momentum_blocked, "nothing blocked");
        assert_eq!(status.momentum_improve, "tighten parser edge cases");
        assert_eq!(status.momentum_recurring, "none yet");
        // Caller-filled fields default empty/false out of the pure parser.
        assert_eq!(status.name, "");
        assert!(!status.has_handoff);
    }

    #[test]
    fn missing_frontmatter_returns_none() {
        assert!(parse_status(MALFORMED).is_none());
    }

    #[test]
    fn empty_input_returns_none() {
        assert!(parse_status("").is_none());
    }

    #[test]
    fn empty_momentum_section_yields_empty_strings() {
        let status = parse_status(EMPTY_MOMENTUM).expect("should parse");
        assert_eq!(status.momentum_now, "");
        assert_eq!(status.momentum_next, "");
        assert_eq!(status.momentum_blocked, "");
        assert_eq!(status.momentum_improve, "");
        assert_eq!(status.momentum_recurring, "");
        // Frontmatter scalars still parse even when the body has no momentum bullets.
        assert_eq!(status.now, "Just started");
    }

    #[test]
    fn round_trips_through_serde() {
        let status = parse_status(WELL_FORMED).expect("should parse");
        let json = serde_json::to_string(&status).expect("serialize");
        let back: RepoStatus = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(status, back);
    }

    #[test]
    fn unquote_strips_surrounding_quotes() {
        assert_eq!(unquote("\"hello\""), "hello");
        assert_eq!(unquote("hello"), "hello");
        assert_eq!(unquote("\"\""), "");
    }

    #[test]
    fn parse_bullet_accepts_em_dash_and_hyphen() {
        assert_eq!(
            parse_bullet("- **now** — doing things"),
            Some(("now".to_string(), "doing things".to_string()))
        );
        assert_eq!(
            parse_bullet("- **now** - doing things"),
            Some(("now".to_string(), "doing things".to_string()))
        );
        assert_eq!(parse_bullet("not a bullet"), None);
    }
}
