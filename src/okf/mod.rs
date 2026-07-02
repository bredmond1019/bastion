// OKF frontmatter model + serializer — the write path.
//
// This module is the in-repo prototype of the future `okf-core` crate. It is pure
// (no I/O) and dependency-light: the serializer is hand-rolled to match the
// house-style hand-rolled parser in `crate::validate::frontmatter` rather than pull
// in `serde_yaml`.
//
// The OKF contract (brain D27): three REQUIRED fields — `type`, `title`,
// `description` — plus six OPTIONAL structured fields — `doc_id`, `layer`,
// `project`, `status`, `keywords`, `related`. `serialize_frontmatter` is the write
// direction that does not exist anywhere else in the stack today; it is what
// `bastion init` (and later `adopt`'s backfill) use to emit compliant frontmatter.

use serde::{Deserialize, Serialize};

// ── Model ───────────────────────────────────────────────────────────────────

/// The OKF frontmatter of a single document.
///
/// Required scalars are `Option<String>` so a partially-filled stamp (e.g. from
/// `adopt`'s backfill) is representable; `serialize_frontmatter` still emits the
/// required keys even when unset, so `validate_frontmatter` flags them as empty.
/// List fields use `Vec<String>` where empty means "absent".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OkfFrontmatter {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub type_: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layer: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<String>,
}

// ── Serializer (the write path) ───────────────────────────────────────────────

/// Serialize OKF frontmatter into a canonical `---`-fenced YAML block.
///
/// Field order is fixed: `type`, `title`, `description`, `doc_id`, `layer`,
/// `project`, `status`, `keywords`, `related`. The three REQUIRED scalars are
/// always emitted (as a bare `key:` when unset, which `validate_frontmatter` reports
/// as an empty field — the intended "needs filling" signal for backfill). Optional
/// fields are emitted only when present/non-empty. Lists render inline: `[a, b, c]`.
/// The returned string includes both fences and a trailing newline.
pub fn serialize_frontmatter(fm: &OkfFrontmatter) -> String {
    let mut out = String::from("---\n");

    // Required scalars — always emitted, even when unset.
    push_scalar(&mut out, "type", fm.type_.as_deref().unwrap_or(""));
    push_scalar(&mut out, "title", fm.title.as_deref().unwrap_or(""));
    push_scalar(
        &mut out,
        "description",
        fm.description.as_deref().unwrap_or(""),
    );

    // Optional fields — emitted only when they carry a value.
    if let Some(v) = fm.doc_id.as_deref().filter(|s| !s.is_empty()) {
        push_scalar(&mut out, "doc_id", v);
    }
    if !fm.layer.is_empty() {
        push_list(&mut out, "layer", &fm.layer);
    }
    if let Some(v) = fm.project.as_deref().filter(|s| !s.is_empty()) {
        push_scalar(&mut out, "project", v);
    }
    if let Some(v) = fm.status.as_deref().filter(|s| !s.is_empty()) {
        push_scalar(&mut out, "status", v);
    }
    if !fm.keywords.is_empty() {
        push_list(&mut out, "keywords", &fm.keywords);
    }
    if !fm.related.is_empty() {
        push_list(&mut out, "related", &fm.related);
    }

    out.push_str("---\n");
    out
}

// ── Internal formatting helpers (pure) ──────────────────────────────────────

/// Append `key: value` (or a bare `key:` when `value` is empty) plus a newline.
fn push_scalar(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    if value.is_empty() {
        out.push(':');
    } else {
        out.push_str(": ");
        out.push_str(&yaml_scalar(value));
    }
    out.push('\n');
}

/// Append `key: [a, b, c]` plus a newline.
fn push_list(out: &mut String, key: &str, items: &[String]) {
    out.push_str(key);
    out.push_str(": [");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&yaml_scalar(item));
    }
    out.push_str("]\n");
}

/// Render a single scalar, double-quoting (with escaping) only when a bare plain
/// scalar would be misparsed by YAML.
fn yaml_scalar(value: &str) -> String {
    if needs_quote(value) {
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    } else {
        value.to_string()
    }
}

/// Whether `value` must be double-quoted to round-trip as a YAML string.
///
/// Conservative by design: it prefers to quote in ambiguous cases. Covers
/// significant leading/trailing whitespace, YAML indicator/structural/comment
/// characters, values YAML would coerce to bool/null, and values that parse as a
/// number (so `title: "1.0"` stays a string rather than a float).
fn needs_quote(v: &str) -> bool {
    if v.is_empty() {
        // Empty scalars are emitted as a bare `key:` by the caller, never quoted.
        return false;
    }
    // Significant leading/trailing whitespace.
    if v != v.trim() {
        return true;
    }
    // Leading indicator characters that start a non-plain YAML node.
    if let Some(first) = v.chars().next()
        && "#@&*!|>%`?,[]{}\"'-".contains(first)
    {
        return true;
    }
    // Structural, flow, comment, or quote characters anywhere.
    if v.contains(':')
        || v.contains('#')
        || v.contains('[')
        || v.contains(']')
        || v.contains('{')
        || v.contains('}')
        || v.contains(',')
        || v.contains('"')
        || v.contains('\n')
    {
        return true;
    }
    // Plain scalars YAML coerces to a non-string type.
    if matches!(
        v.to_ascii_lowercase().as_str(),
        "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "~"
    ) {
        return true;
    }
    // Numeric-looking scalars (ints, floats, exponents) would deserialize as numbers.
    v.parse::<f64>().is_ok()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::frontmatter::{parse_frontmatter, validate_frontmatter};
    use std::path::PathBuf;

    fn full() -> OkfFrontmatter {
        OkfFrontmatter {
            type_: Some("Guideline".into()),
            title: Some("My Title".into()),
            description: Some("A one-line summary.".into()),
            doc_id: Some("my-title".into()),
            layer: vec!["brain".into(), "console".into()],
            project: Some("bastion".into()),
            status: Some("active".into()),
            keywords: vec!["okf".into(), "frontmatter".into(), "scaffold".into()],
            related: vec!["okf-core".into()],
        }
    }

    // ── Field emission & ordering ─────────────────────────────────────────────

    #[test]
    fn serialize_default_emits_only_required_keys_empty() {
        let out = serialize_frontmatter(&OkfFrontmatter::default());
        assert_eq!(out, "---\ntype:\ntitle:\ndescription:\n---\n");
    }

    #[test]
    fn serialize_full_exact_output() {
        let out = serialize_frontmatter(&full());
        let expected = "\
---
type: Guideline
title: My Title
description: A one-line summary.
doc_id: my-title
layer: [brain, console]
project: bastion
status: active
keywords: [okf, frontmatter, scaffold]
related: [okf-core]
---
";
        assert_eq!(out, expected);
    }

    #[test]
    fn serialize_omits_absent_optional_fields() {
        let fm = OkfFrontmatter {
            type_: Some("Log".into()),
            title: Some("T".into()),
            description: Some("D".into()),
            ..Default::default()
        };
        let out = serialize_frontmatter(&fm);
        assert_eq!(out, "---\ntype: Log\ntitle: T\ndescription: D\n---\n");
        assert!(!out.contains("doc_id"));
        assert!(!out.contains("layer"));
        assert!(!out.contains("keywords"));
    }

    #[test]
    fn serialize_empty_optional_scalars_are_dropped() {
        // Optional scalars that are Some("") must not emit a bare key.
        let fm = OkfFrontmatter {
            type_: Some("Doc".into()),
            title: Some("T".into()),
            description: Some("D".into()),
            doc_id: Some(String::new()),
            project: Some(String::new()),
            ..Default::default()
        };
        let out = serialize_frontmatter(&fm);
        assert_eq!(out, "---\ntype: Doc\ntitle: T\ndescription: D\n---\n");
    }

    #[test]
    fn serialize_canonical_field_order() {
        let out = serialize_frontmatter(&full());
        let pos = |k: &str| out.find(k).unwrap_or_else(|| panic!("missing {k}"));
        let order = [
            "type:",
            "title:",
            "description:",
            "doc_id:",
            "layer:",
            "project:",
            "status:",
            "keywords:",
            "related:",
        ];
        let positions: Vec<usize> = order.iter().map(|k| pos(k)).collect();
        for w in positions.windows(2) {
            assert!(w[0] < w[1], "fields must be in canonical order");
        }
    }

    #[test]
    fn serialize_single_item_list() {
        let fm = OkfFrontmatter {
            type_: Some("Doc".into()),
            title: Some("T".into()),
            description: Some("D".into()),
            layer: vec!["meta".into()],
            ..Default::default()
        };
        assert!(serialize_frontmatter(&fm).contains("layer: [meta]\n"));
    }

    // ── Quoting rules ─────────────────────────────────────────────────────────

    #[test]
    fn quote_value_containing_colon() {
        assert!(needs_quote("Hello: World"));
        assert_eq!(yaml_scalar("Hello: World"), "\"Hello: World\"");
    }

    #[test]
    fn quote_value_with_hash() {
        assert!(needs_quote("issue #42"));
    }

    #[test]
    fn quote_leading_and_trailing_whitespace() {
        assert!(needs_quote(" leading"));
        assert!(needs_quote("trailing "));
    }

    #[test]
    fn quote_bool_and_null_like() {
        for v in ["true", "False", "null", "yes", "NO", "on", "off", "~"] {
            assert!(needs_quote(v), "{v} should be quoted");
        }
    }

    #[test]
    fn quote_numeric_like() {
        for v in ["1", "1.0", "-3", "1e5", "42"] {
            assert!(needs_quote(v), "{v} should be quoted");
        }
    }

    #[test]
    fn quote_leading_indicator_chars() {
        for v in ["- item", "* star", "@ handle", "? q", "[bracket"] {
            assert!(needs_quote(v), "{v:?} should be quoted");
        }
    }

    #[test]
    fn no_quote_plain_words() {
        for v in ["active", "bastion", "My Title", "A one-line summary."] {
            assert!(!needs_quote(v), "{v:?} should NOT be quoted");
        }
    }

    #[test]
    fn quote_embedded_double_quote_is_escaped() {
        assert_eq!(yaml_scalar("say \"hi\""), "\"say \\\"hi\\\"\"");
    }

    #[test]
    fn quoted_list_item() {
        let fm = OkfFrontmatter {
            type_: Some("Doc".into()),
            title: Some("T".into()),
            description: Some("D".into()),
            keywords: vec!["plain".into(), "has: colon".into()],
            ..Default::default()
        };
        assert!(serialize_frontmatter(&fm).contains("keywords: [plain, \"has: colon\"]\n"));
    }

    // ── Round-trip against the house parser / validator ───────────────────────

    #[test]
    fn roundtrip_full_parses_and_validates_clean() {
        let out = serialize_frontmatter(&full());
        let path = PathBuf::from("generated.md");

        // The hand-rolled parser recovers all three required scalars.
        let fm = parse_frontmatter(&out).expect("serialized block must parse");
        assert_eq!(fm.fields["type"].0, "Guideline");
        assert_eq!(fm.fields["title"].0, "My Title");
        assert_eq!(fm.fields["description"].0, "A one-line summary.");
        assert_eq!(fm.fields["doc_id"].0, "my-title");

        // And the OKF validator finds no required-field errors.
        assert!(
            validate_frontmatter(&out, &path).is_empty(),
            "serialized frontmatter must pass required-field validation"
        );
    }

    #[test]
    fn roundtrip_default_flags_all_required_empty() {
        // A bare stamp (all required unset) must serialize, parse, and then fail
        // validation with exactly the three empty-field errors — the backfill signal.
        let out = serialize_frontmatter(&OkfFrontmatter::default());
        assert!(parse_frontmatter(&out).is_some());
        let errs = validate_frontmatter(&out, &PathBuf::from("stamp.md"));
        assert_eq!(errs.len(), 3, "expected type/title/description flagged");
    }

    #[test]
    fn roundtrip_quoted_colon_value_survives_body_append() {
        // A serialized block with a quoted value should still parse as a valid block
        // when a document body is appended after the closing fence.
        let fm = OkfFrontmatter {
            type_: Some("Doc".into()),
            title: Some("Ratio 3:1 explained".into()),
            description: Some("D".into()),
            ..Default::default()
        };
        let doc = format!("{}\n# Body\n", serialize_frontmatter(&fm));
        assert!(parse_frontmatter(&doc).is_some());
        assert!(validate_frontmatter(&doc, &PathBuf::from("d.md")).is_empty());
    }
}
