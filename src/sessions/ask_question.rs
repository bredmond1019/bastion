// sessions/ask_question.rs — pure parser for the AskUserQuestion pane.
//
// Turns a captured tmux pane (raw text, as produced by `tmux capture-pane -p`) into a
// structured `AskQuestionPrompt`: the question text, its numbered options, and a flag
// marking the trailing free-text "escape hatch" option (e.g. "Chat about this").
//
// This module is 100% pure: no `std::fs`, no `std::process`, no network calls, no clock
// reads. It is the input the Telegram bridge (BA.20.C) uses to build an inline keyboard —
// a wrong `Some` here would send a bogus keyboard for a pane that is not actually an
// AskUserQuestion prompt (e.g. a yes/no permission dialog), which is worse than not
// sending anything. When in doubt, this parser returns `None`.

/// The stable substring BA.20.A's manifest detection rule gates on to recognize an
/// AskUserQuestion pane. Deliberately narrower than the full footer line — the `·`
/// separators and arrow glyphs are the parts most likely to vary across terminal widths
/// and Claude Code versions. BA.20.C and any future detection rule should reference this
/// constant rather than repeating the string literal, so the two can never drift apart.
pub const ASK_QUESTION_MARKER: &str = "Enter to select";

/// Heuristic substrings (case-insensitive) that mark an option's label as the trailing
/// free-text "escape hatch" (e.g. "Chat about this", "Something else", "Other"). This is
/// deliberately a soft match, never a hard string equality — the operator confirmed
/// (2026-08-14) that the escape hatch is almost always present but is not guaranteed to
/// read any particular string verbatim. Combined with the "last option only" structural
/// rule in `parse_ask_question`, so structural position alone never sets the flag.
const ESCAPE_HATCH_HINTS: &[&str] = &["chat about", "something else", "other"];

/// One numbered option in an `AskUserQuestion` prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionOption {
    /// 1-indexed position in screen order.
    pub number: usize,
    /// The option's label text (the text on the numbered line itself).
    pub label: String,
    /// An optional description, joined from any indented continuation line(s)
    /// following the option. Wrapped multi-line descriptions are joined into one
    /// string with single spaces.
    pub description: Option<String>,
    /// Whether this option is the trailing free-text escape hatch. Only ever `true`
    /// for the last option, and only when its label also passes a soft text check.
    pub is_escape_hatch: bool,
}

/// A parsed `AskUserQuestion` prompt: the question text plus its numbered options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskQuestionPrompt {
    /// The widget's header chip text (e.g. `Colour`), when present. Captured
    /// separately from `question` — never concatenated into it.
    pub header: Option<String>,
    pub question: String,
    pub options: Vec<QuestionOption>,
}

/// Characters that show up as box-drawing borders or selection-marker glyphs around
/// option lines in a rendered tmux pane. Stripped from the start of each line before
/// classification so borders/markers never leak into parsed text.
const DECORATIVE_LEADING_CHARS: &[char] =
    &['│', '┃', '║', '┆', '┊', '❯', '➤', '▶', '>', '─', '━', '·'];

/// Strip decorative leading characters (box-drawing borders, selection markers) and
/// surrounding whitespace from a line, returning the trimmed content plus how many
/// leading whitespace columns preceded the first non-decorative, non-whitespace
/// character — used as a crude indentation signal to tell option lines from their
/// description continuation lines.
fn strip_decoration(line: &str) -> (String, usize) {
    let mut chars: Vec<char> = line.chars().collect();
    // Trim trailing whitespace, then a trailing decorative border character (e.g. the
    // right-hand `│` of a boxed prompt), then any whitespace that preceded it — repeat
    // until nothing more decorative remains at the end of the line.
    loop {
        while matches!(chars.last(), Some(c) if c.is_whitespace()) {
            chars.pop();
        }
        if matches!(chars.last(), Some(c) if DECORATIVE_LEADING_CHARS.contains(c)) {
            chars.pop();
        } else {
            break;
        }
    }
    let mut indent = 0usize;
    let mut i = 0usize;
    loop {
        match chars.get(i) {
            Some(c) if c.is_whitespace() => {
                indent += 1;
                i += 1;
            }
            Some(c) if DECORATIVE_LEADING_CHARS.contains(c) => {
                i += 1;
            }
            _ => break,
        }
    }
    let rest: String = chars[i..].iter().collect();
    (rest.trim().to_string(), indent)
}

/// If `content` starts with a numbered-option marker (`1.`, `1)`, or `1 -` style,
/// possibly followed by more digits), return the parsed number and the remaining label
/// text. Otherwise `None`.
fn parse_option_marker(content: &str) -> Option<(usize, String)> {
    let content = content.trim_start();
    let digits_end = content.find(|c: char| !c.is_ascii_digit())?;
    if digits_end == 0 {
        return None;
    }
    let number: usize = content[..digits_end].parse().ok()?;
    let rest = &content[digits_end..];
    let rest = rest
        .strip_prefix('.')
        .or_else(|| rest.strip_prefix(')'))
        .or_else(|| {
            // "1 - label" style: optional space(s), then a literal '-'.
            let trimmed = rest.trim_start_matches(' ');
            trimmed.strip_prefix('-')
        })?;
    let label = rest.trim().to_string();
    if label.is_empty() {
        return None;
    }
    Some((number, label))
}

/// Does this label read as the trailing free-text escape hatch, per the heuristic hint
/// list? Case-insensitive substring match — never a hard equality.
fn looks_like_escape_hatch(label: &str) -> bool {
    let lower = label.to_lowercase();
    ESCAPE_HATCH_HINTS.iter().any(|hint| lower.contains(hint))
}

/// If `raw` (an UN-decorated, original screen line) is the widget's header-chip line —
/// a short line beginning with a checkbox glyph (`☐` or `□`) — return its text with the
/// glyph and surrounding whitespace stripped. Operates on the raw line rather than the
/// decoration-stripped one because the checkbox glyphs are not in
/// `DECORATIVE_LEADING_CHARS` (stripping them there would erase the very signal this
/// function looks for).
fn parse_header_chip(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let rest = trimmed
        .strip_prefix('☐')
        .or_else(|| trimmed.strip_prefix('□'))?;
    let rest = rest.trim();
    if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    }
}

/// Is `raw` a horizontal-rule line — a run of box-drawing dash characters (`─`/`━`) with
/// nothing else on it? Used as the fallback top boundary of the widget when no header
/// chip line is present.
fn is_rule_line(raw: &str) -> bool {
    let trimmed = raw.trim();
    !trimmed.is_empty() && trimmed.chars().all(|c| c == '─' || c == '━')
}

/// Parse a captured `AskUserQuestion` pane into structured data.
///
/// Returns `None` when the pane does not carry the AskUserQuestion footer marker, or
/// when it does but no numbered options could be found (a marker with zero options is
/// treated as unparseable, not as an empty prompt).
pub fn parse_ask_question(screen: &str) -> Option<AskQuestionPrompt> {
    if !screen.contains(ASK_QUESTION_MARKER) {
        return None;
    }

    // Classify each line: strip decoration, record its content and indentation. Keep the
    // raw (un-decorated) lines alongside for structural boundary detection, since the
    // header-chip glyphs and rule-line dashes are exactly the things decoration
    // stripping erases.
    let raw_lines: Vec<&str> = screen.lines().collect();
    let lines: Vec<(String, usize)> = raw_lines.iter().map(|l| strip_decoration(l)).collect();

    // Bound the widget: walk UPWARD from the first numbered option to find the top of
    // the widget, so everything above it (scrollback: banners, warnings, the operator's
    // own prompt) is discarded before the question is read. Prefer the header-chip line;
    // failing that, the nearest horizontal rule. Finding neither (e.g. a prompt with no
    // widget framing at all, as in the synthetic tests) leaves the boundary at the very
    // start of the screen — unchanged behaviour for those cases.
    let first_option_idx = lines
        .iter()
        .position(|(content, _)| parse_option_marker(content).is_some());
    let mut header: Option<String> = None;
    let mut scroll_boundary = 0usize;
    if let Some(opt_idx) = first_option_idx {
        let mut idx = opt_idx;
        while idx > 0 {
            idx -= 1;
            let raw = raw_lines[idx];
            if let Some(chip) = parse_header_chip(raw) {
                header = Some(chip);
                scroll_boundary = idx + 1;
                break;
            }
            if is_rule_line(raw) {
                scroll_boundary = idx + 1;
                break;
            }
        }
    }

    let mut question_parts: Vec<String> = Vec::new();
    let mut options: Vec<QuestionOption> = Vec::new();
    let mut current_desc: Vec<String> = Vec::new();
    let mut option_indent: Option<usize> = None;
    let mut seen_first_option = false;

    let flush_desc = |opt: &mut Option<QuestionOption>, desc: &mut Vec<String>| {
        if let Some(o) = opt
            && !desc.is_empty()
        {
            o.description = Some(desc.join(" "));
        }
        desc.clear();
    };

    let mut last_option: Option<QuestionOption> = None;

    for (i, (content, indent)) in lines.iter().enumerate() {
        if i < scroll_boundary {
            // Scrollback (or the header-chip / rule line itself): discard.
            continue;
        }
        if content.is_empty() {
            continue;
        }
        if content.contains(ASK_QUESTION_MARKER) {
            // Footer line — stop consuming pane content.
            break;
        }
        if let Some((number, label)) = parse_option_marker(content) {
            // Starting a new option: flush the previous one's accumulated description.
            flush_desc(&mut last_option, &mut current_desc);
            if let Some(prev) = last_option.take() {
                options.push(prev);
            }
            seen_first_option = true;
            option_indent = Some(*indent);
            last_option = Some(QuestionOption {
                number,
                label,
                description: None,
                is_escape_hatch: false,
            });
            continue;
        }

        if !seen_first_option {
            // Prose above the first option: part of the question text.
            question_parts.push(content.clone());
        } else {
            // A non-numbered line after at least one option: treat it as a description
            // continuation line if it is indented at or past the option's own
            // indentation (rendered TUIs vary a column or two, so use ">=" rather than
            // a strict ">").
            let base = option_indent.unwrap_or(0);
            if *indent >= base {
                current_desc.push(content.clone());
            }
            // Otherwise (a dedented, non-numbered line) — ignore; not part of this
            // prompt's structured content.
        }
    }

    // Flush the final option, if any.
    flush_desc(&mut last_option, &mut current_desc);
    if let Some(last) = last_option.take() {
        options.push(last);
    }

    if options.is_empty() {
        return None;
    }

    // Flag the escape hatch: last option only, and only when its label soft-matches.
    if let Some(last) = options.last_mut()
        && looks_like_escape_hatch(&last.label)
    {
        last.is_escape_hatch = true;
    }

    let question = question_parts.join(" ").trim().to_string();
    if question.is_empty() {
        return None;
    }

    Some(AskQuestionPrompt {
        header,
        question,
        options,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../detect/fixtures/claude_awaiting_question.txt");

    #[test]
    fn parses_real_fixture_happy_path() {
        // Fixture provenance: this is a REAL captured `tmux capture-pane` pane
        // (verified 2026-08-14 against a live Claude Code v2.1.233 session) — it
        // replaces the old synthesized fixture of the same name. Everything above the
        // widget's top boundary (the startup banner, the MCP auth warning, the auto-mode
        // notice, and the operator's own prompt) must be discarded as scrollback, not
        // folded into `question`. See this spec's Amendment Log for context.
        let parsed = parse_ask_question(FIXTURE).expect("fixture should parse");

        assert_eq!(parsed.question, "Which colour do you prefer?");
        assert_eq!(parsed.header, Some("Colour".to_string()));
        assert_eq!(parsed.options.len(), 5);

        assert_eq!(parsed.options[0].number, 1);
        assert_eq!(parsed.options[0].label, "Red");
        assert_eq!(
            parsed.options[0].description.as_deref(),
            Some("Warm, bold, high-energy.")
        );
        assert_eq!(parsed.options[1].number, 2);
        assert_eq!(parsed.options[1].label, "Green");
        assert_eq!(parsed.options[2].number, 3);
        assert_eq!(parsed.options[2].label, "Blue");
        assert_eq!(parsed.options[3].number, 4);
        assert_eq!(parsed.options[3].label, "Type something.");
        assert_eq!(parsed.options[4].number, 5);
        assert_eq!(parsed.options[4].label, "Chat about this");
    }

    #[test]
    fn real_fixture_question_excludes_scrollback_banner_text() {
        // The cheap tripwire against a future regression to unbounded upward scanning:
        // the banner/warning/notice strings the OLD parser used to swallow whole must
        // never again appear in the parsed question.
        let parsed = parse_ask_question(FIXTURE).expect("fixture should parse");
        assert!(!parsed.question.contains("Claude Code"));
        assert!(!parsed.question.contains("auto mode"));
        assert!(!parsed.question.contains("MCP"));
    }

    const NO_HEADER_CHIP_RULE_FALLBACK: &str = "\
Claude Code v2.1.233 startup banner and scrollback that must never reach the question.
More scrollback: an MCP auth warning, an auto mode notice, the operator's own prompt.
────────────────────────────────────────────
Which fallback question applies here?

❯ 1. Yes
  2. No

Enter to select · ↑/↓ to navigate · Esc to cancel
";

    #[test]
    fn horizontal_rule_fallback_bounds_question_without_header_chip() {
        // A widget with no header-chip line still finds its top boundary via the
        // nearest horizontal rule above the first option — same question, no header.
        let parsed = parse_ask_question(NO_HEADER_CHIP_RULE_FALLBACK).expect("should parse");
        assert_eq!(parsed.header, None);
        assert_eq!(parsed.question, "Which fallback question applies here?");
        assert!(!parsed.question.contains("scrollback"));
        assert!(!parsed.question.contains("Claude Code"));
    }

    const BLOCKED_FIXTURE: &str = include_str!("../detect/fixtures/claude_blocked.txt");

    // --- NEGATIVE ---

    #[test]
    fn empty_string_returns_none() {
        assert_eq!(parse_ask_question(""), None);
    }

    #[test]
    fn unrelated_shell_output_returns_none() {
        let screen =
            "brandon@mini ~ % ls -la\ntotal 8\ndrwxr-xr-x  3 brandon staff 96 Aug 14 09:00 .\n";
        assert_eq!(parse_ask_question(screen), None);
    }

    #[test]
    fn permission_dialog_fixture_returns_none() {
        // The most important negative case: a yes/no permission dialog must never be
        // mistaken for an AskUserQuestion prompt, or BA.20.C would send a bogus
        // keyboard and inject a digit into a permission prompt.
        assert_eq!(parse_ask_question(BLOCKED_FIXTURE), None);
    }

    #[test]
    fn marker_with_no_numbered_options_returns_none() {
        let screen = "  Which approach should I take?\n\n  Enter to select · ↑/↓ to navigate · Esc to cancel\n";
        assert_eq!(parse_ask_question(screen), None);
    }

    // --- ESCAPE HATCH ---

    const NO_ESCAPE_HATCH: &str = "\
  What should we name the module?

  ❯ 1. parser
    2. reader
    3. scanner

  Enter to select · ↑/↓ to navigate · Esc to cancel
";

    #[test]
    fn trailing_option_without_soft_match_is_not_flagged() {
        let parsed = parse_ask_question(NO_ESCAPE_HATCH).expect("should parse");
        assert_eq!(parsed.options.len(), 3);
        for opt in &parsed.options {
            assert!(!opt.is_escape_hatch);
        }
    }

    const SINGLE_PLUS_ESCAPE_HATCH: &str = "\
  Should we proceed with the deploy?

  ❯ 1. Yes, deploy now
    2. Something else

  Enter to select · ↑/↓ to navigate · Esc to cancel
";

    #[test]
    fn single_option_plus_escape_hatch_flags_only_the_second() {
        let parsed = parse_ask_question(SINGLE_PLUS_ESCAPE_HATCH).expect("should parse");
        assert_eq!(parsed.options.len(), 2);
        assert!(!parsed.options[0].is_escape_hatch);
        assert!(parsed.options[1].is_escape_hatch);
    }

    // --- OPTIONS ---

    const MULTI_OPTION: &str = "\
  Which database should we use?

  ❯ 1. Postgres
    2. MySQL
       A well-known relational database.
    3. SQLite
    4. Chat about this

  Enter to select · ↑/↓ to navigate · Esc to cancel
";

    #[test]
    fn multi_option_preserves_order_and_numbering() {
        let parsed = parse_ask_question(MULTI_OPTION).expect("should parse");
        assert_eq!(parsed.options.len(), 4);
        for (idx, opt) in parsed.options.iter().enumerate() {
            assert_eq!(opt.number, idx + 1);
        }
        assert_eq!(parsed.options[0].label, "Postgres");
        assert_eq!(parsed.options[1].label, "MySQL");
        assert_eq!(parsed.options[2].label, "SQLite");
        assert_eq!(parsed.options[3].label, "Chat about this");
    }

    #[test]
    fn option_without_description_line_is_none() {
        let parsed = parse_ask_question(MULTI_OPTION).expect("should parse");
        assert_eq!(parsed.options[0].description, None);
    }

    #[test]
    fn option_with_description_line_is_captured() {
        let parsed = parse_ask_question(MULTI_OPTION).expect("should parse");
        assert_eq!(
            parsed.options[1].description.as_deref(),
            Some("A well-known relational database.")
        );
    }

    #[test]
    fn trailing_escape_hatch_option_is_flagged_in_multi_option_prompt() {
        let parsed = parse_ask_question(MULTI_OPTION).expect("should parse");
        assert!(!parsed.options[0].is_escape_hatch);
        assert!(!parsed.options[1].is_escape_hatch);
        assert!(!parsed.options[2].is_escape_hatch);
        assert!(parsed.options[3].is_escape_hatch);
    }

    const WRAPPED_DESCRIPTION: &str = "\
  Which caching strategy should we adopt?

  ❯ 1. Write-through
       Writes go to the cache and the backing
       store at the same time, keeping both
       in sync on every write.
    2. Write-back

  Enter to select · ↑/↓ to navigate · Esc to cancel
";

    #[test]
    fn multi_line_description_is_joined_with_single_spaces() {
        let parsed = parse_ask_question(WRAPPED_DESCRIPTION).expect("should parse");
        assert_eq!(
            parsed.options[0].description.as_deref(),
            Some(
                "Writes go to the cache and the backing store at the same time, keeping both in sync on every write."
            )
        );
        assert_eq!(parsed.options[1].description, None);
    }

    // --- ROBUSTNESS ---

    const PLAIN_PROMPT: &str = "\
  Which log level should we default to?

  1. debug
    2. info
    3. warn

  Enter to select · ↑/↓ to navigate · Esc to cancel
";

    const BORDERED_PROMPT: &str = "\
  │ Which log level should we default to?       │
  │                                              │
  │ ❯ 1. debug                                   │
  │   2. info                                    │
  │   3. warn                                    │

  Enter to select · ↑/↓ to navigate · Esc to cancel
";

    #[test]
    fn bordered_and_selection_marked_rendering_parses_equal_to_plain() {
        let plain = parse_ask_question(PLAIN_PROMPT).expect("plain should parse");
        let bordered = parse_ask_question(BORDERED_PROMPT).expect("bordered should parse");
        assert_eq!(plain, bordered);
    }

    // --- WIDTH ---

    const NARROW_WIDTH_PROMPT: &str = "\
  Should we enable the new
  retry policy for all
  outbound HTTP calls?

  ❯ 1. Yes, enable it
    2. No, leave as-is

  Enter to select · ↑/↓ to navigate · Esc to cancel
";

    const WIDE_WIDTH_PROMPT: &str = "\
  Should we enable the new retry policy for all outbound HTTP calls?

  ❯ 1. Yes, enable it
    2. No, leave as-is

  Enter to select · ↑/↓ to navigate · Esc to cancel
";

    #[test]
    fn narrow_and_wide_width_renderings_produce_same_question_and_labels() {
        let narrow = parse_ask_question(NARROW_WIDTH_PROMPT).expect("narrow should parse");
        let wide = parse_ask_question(WIDE_WIDTH_PROMPT).expect("wide should parse");
        assert_eq!(narrow.question, wide.question);
        assert_eq!(
            narrow
                .options
                .iter()
                .map(|o| o.label.clone())
                .collect::<Vec<_>>(),
            wide.options
                .iter()
                .map(|o| o.label.clone())
                .collect::<Vec<_>>()
        );
    }
}
