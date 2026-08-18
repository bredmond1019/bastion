// sessions/ask_question.rs — pure parser for the AskUserQuestion pane.
//
// Turns a captured tmux pane (raw text, as produced by `tmux capture-pane -p`) into a
// structured `AskQuestionPrompt`: the question text, its numbered options, and each
// option's `OptionKind` (an ordinary choice, the inline free-text option, or the
// widget-closing "chat about this" option).
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

/// Heuristic substrings (case-insensitive) that softly confirm a STRUCTURALLY classified
/// `FreeText` option's label reads as an inline free-text prompt (e.g. "Type
/// something."). SECONDARY confirmation only — position (immediately above the widget's
/// separating horizontal rule) is what actually sets the classification. This has to be
/// secondary: once the free-text option is filled in with typed text (e.g.
/// `teal, actually`), the placeholder wording is gone entirely, so a text-first
/// classifier would misclassify it. See `parse_ask_question`.
const FREE_TEXT_HINTS: &[&str] = &["type something"];

/// Heuristic substrings (case-insensitive) that softly confirm a STRUCTURALLY classified
/// `ChatAbout` option's label (e.g. "Chat about this", "Something else", "Other").
/// SECONDARY confirmation only — position (immediately below the widget's separating
/// horizontal rule) is what actually sets the classification; the operator confirmed
/// (2026-08-14) that the label text is not guaranteed to read any particular string
/// verbatim.
const CHAT_ABOUT_HINTS: &[&str] = &["chat about", "something else", "other"];

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
    /// What selecting this option actually does. See [`OptionKind`].
    pub kind: OptionKind,
}

/// The three distinct behaviours a trailing `AskUserQuestion` option can carry,
/// replacing the old single `is_escape_hatch: bool` flag that conflated two genuinely
/// different things (operator-confirmed 2026-08-14, reproduced live). Classified
/// STRUCTURALLY by `parse_ask_question`: whether an option sits above, at, or below the
/// horizontal rule separating the widget's numbered block from its trailing option is
/// the reliable discriminator. Label text is only ever a secondary, non-authoritative
/// confirmation signal — never the primary classifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionKind {
    /// An ordinary choice. Selecting it — digit, then Enter — answers the question with
    /// that choice.
    Choice,
    /// The option immediately above the separating rule. Selecting it lets the operator
    /// type inline: the typed text replaces the option's placeholder label in place, and
    /// Enter submits that typed text AS THE ANSWER to the question (digit, then the
    /// text, then Enter — verified: `Which colour do you prefer? -> teal, actually`).
    FreeText,
    /// The option below the separating rule. Selecting it — digit, then Enter — CLOSES
    /// the widget and returns to the normal session prompt, where the agent replies with
    /// something variable. It does not answer the question at all.
    ChatAbout,
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

/// Does this label read as the inline free-text option, per the heuristic hint list?
/// Case-insensitive substring match — never a hard equality, and never the primary
/// classifier (see [`OptionKind::FreeText`]).
fn looks_like_free_text(label: &str) -> bool {
    let lower = label.to_lowercase();
    FREE_TEXT_HINTS.iter().any(|hint| lower.contains(hint))
}

/// Does this label read as the widget-closing "chat about this" option, per the
/// heuristic hint list? Case-insensitive substring match — never a hard equality, and
/// never the primary classifier (see [`OptionKind::ChatAbout`]).
fn looks_like_chat_about(label: &str) -> bool {
    let lower = label.to_lowercase();
    CHAT_ABOUT_HINTS.iter().any(|hint| lower.contains(hint))
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

    // The number of options parsed so far (options already flushed, plus the
    // in-progress `last_option`, if any) at the moment the widget's separating
    // horizontal rule is encountered. `Some(n)` means: options[0..n-1] are ordinary
    // choices, options[n-1] is the FreeText candidate immediately above the rule, and
    // any options after it are ChatAbout. Only the FIRST such rule counts — the widget
    // has exactly one.
    let mut rule_boundary: Option<usize> = None;

    for (i, (content, indent)) in lines.iter().enumerate() {
        if i < scroll_boundary {
            // Scrollback (or the header-chip / rule line itself): discard.
            continue;
        }
        if content.is_empty() {
            if rule_boundary.is_none() && seen_first_option && is_rule_line(raw_lines[i]) {
                rule_boundary = Some(options.len() + usize::from(last_option.is_some()));
            }
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
                kind: OptionKind::Choice,
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

    // Classify STRUCTURALLY: only when a separating rule was found, and only when
    // there is at least one option after it (otherwise the rule is decorative — a
    // trailing rule with nothing following it fabricates nothing).
    if let Some(boundary) = rule_boundary
        && boundary >= 1
        && boundary < options.len()
    {
        let free_text_opt = &mut options[boundary - 1];
        free_text_opt.kind = OptionKind::FreeText;
        if !looks_like_free_text(&free_text_opt.label) {
            tracing::debug!(
                label = %free_text_opt.label,
                "ask_question: FreeText option classified structurally; label did not \
                 soft-match the free-text hint list"
            );
        }
        for opt in &mut options[boundary..] {
            opt.kind = OptionKind::ChatAbout;
            if !looks_like_chat_about(&opt.label) {
                tracing::debug!(
                    label = %opt.label,
                    "ask_question: ChatAbout option classified structurally; label did \
                     not soft-match the chat-about hint list"
                );
            }
        }
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

        assert_eq!(
            parsed.options.iter().map(|o| o.kind).collect::<Vec<_>>(),
            vec![
                OptionKind::Choice,
                OptionKind::Choice,
                OptionKind::Choice,
                OptionKind::FreeText,
                OptionKind::ChatAbout,
            ]
        );
    }

    const FREETEXT_FILLED_FIXTURE: &str =
        include_str!("fixtures/claude_ask_question_freetext_filled.txt");
    const FREETEXT_SELECTED_FIXTURE: &str =
        include_str!("fixtures/claude_ask_question_freetext_selected.txt");

    /// The kinds every one of the three real fixtures should yield, in order — the
    /// widget's structural layout (numbered block, then a rule, then the two trailing
    /// options) is identical across all three; only the label text or the `❯` marker
    /// position differs between captures.
    fn expected_real_fixture_kinds() -> Vec<OptionKind> {
        vec![
            OptionKind::Choice,
            OptionKind::Choice,
            OptionKind::Choice,
            OptionKind::FreeText,
            OptionKind::ChatAbout,
        ]
    }

    #[test]
    fn freetext_filled_label_still_classifies_as_freetext() {
        // The case a text-first classifier gets wrong: once the free-text option is
        // filled in with typed text, the placeholder wording ("Type something.") is
        // gone, so only structural position (immediately above the separating rule)
        // can still classify it correctly.
        let parsed = parse_ask_question(FREETEXT_FILLED_FIXTURE).expect("should parse");
        assert_eq!(
            parsed.options.iter().map(|o| o.kind).collect::<Vec<_>>(),
            expected_real_fixture_kinds()
        );
        assert_eq!(parsed.options[3].label, "teal, actually");
    }

    #[test]
    fn freetext_selected_marker_position_does_not_affect_classification() {
        // The `❯` selection marker sits on option 4 in this capture (vs. option 1 in
        // `claude_awaiting_question.txt`) — classification must be identical regardless
        // of which option currently carries the marker, because it is structural, not
        // marker-driven.
        let parsed = parse_ask_question(FREETEXT_SELECTED_FIXTURE).expect("should parse");
        assert_eq!(
            parsed.options.iter().map(|o| o.kind).collect::<Vec<_>>(),
            expected_real_fixture_kinds()
        );
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

    // --- OPTION KIND ---

    const NO_RULE_NO_HINT: &str = "\
  What should we name the module?

  ❯ 1. parser
    2. reader
    3. scanner

  Enter to select · ↑/↓ to navigate · Esc to cancel
";

    #[test]
    fn no_rule_and_no_soft_match_yields_choice_for_every_option() {
        let parsed = parse_ask_question(NO_RULE_NO_HINT).expect("should parse");
        assert_eq!(parsed.options.len(), 3);
        for opt in &parsed.options {
            assert_eq!(opt.kind, OptionKind::Choice);
        }
    }

    const MATCHING_TEXT_BUT_NO_RULE: &str = "\
  Should we proceed with the deploy?

  ❯ 1. Yes, deploy now
    2. Something else

  Enter to select · ↑/↓ to navigate · Esc to cancel
";

    #[test]
    fn matching_hint_text_without_a_separating_rule_does_not_fabricate_chat_about() {
        // Structure is the discriminator, not text: a label that soft-matches the
        // chat-about hint list must NOT be classified as `ChatAbout` when there is no
        // separating rule above it.
        let parsed = parse_ask_question(MATCHING_TEXT_BUT_NO_RULE).expect("should parse");
        assert_eq!(parsed.options.len(), 2);
        assert_eq!(parsed.options[0].kind, OptionKind::Choice);
        assert_eq!(parsed.options[1].kind, OptionKind::Choice);
    }

    const RULE_WITH_NO_RECOGNISABLE_TEXT: &str = "\
  Which environment should we target?

  ❯ 1. staging
    2. production
────────────────────────────────────────────
    3. xyz123

  Enter to select · ↑/↓ to navigate · Esc to cancel
";

    #[test]
    fn separating_rule_with_no_recognisable_text_still_classifies_structurally() {
        // The reliable discriminator is structural, not textual: neither "production"
        // nor "xyz123" soft-match either hint list, yet the rule alone must still
        // classify the option above it as `FreeText` and the option below it as
        // `ChatAbout`.
        let parsed = parse_ask_question(RULE_WITH_NO_RECOGNISABLE_TEXT).expect("should parse");
        assert_eq!(parsed.options.len(), 3);
        assert_eq!(parsed.options[0].kind, OptionKind::Choice);
        assert_eq!(parsed.options[1].kind, OptionKind::FreeText);
        assert_eq!(parsed.options[2].kind, OptionKind::ChatAbout);
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
    fn trailing_matching_text_without_rule_is_choice_in_multi_option_prompt() {
        // MULTI_OPTION's last label reads exactly like the chat-about hint list
        // ("Chat about this"), but there is no separating rule above it — structure,
        // not text, is what would set `ChatAbout`, so this must stay `Choice`.
        let parsed = parse_ask_question(MULTI_OPTION).expect("should parse");
        assert_eq!(parsed.options[0].kind, OptionKind::Choice);
        assert_eq!(parsed.options[1].kind, OptionKind::Choice);
        assert_eq!(parsed.options[2].kind, OptionKind::Choice);
        assert_eq!(parsed.options[3].kind, OptionKind::Choice);
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

    // --- parse_option_marker boundary cases (private fn, exercised directly) ---

    #[test]
    fn parse_option_marker_rejects_no_leading_digit() {
        assert_eq!(parse_option_marker("no digit here"), None);
    }

    #[test]
    fn parse_option_marker_rejects_digit_with_no_separator() {
        // "1x" — digits followed directly by non-separator content.
        assert_eq!(parse_option_marker("1x label"), None);
    }

    #[test]
    fn parse_option_marker_rejects_empty_label_after_separator() {
        assert_eq!(parse_option_marker("1."), None);
        assert_eq!(parse_option_marker("1. "), None);
    }

    #[test]
    fn parse_option_marker_accepts_dot_paren_and_dash_styles() {
        assert_eq!(parse_option_marker("1. Red"), Some((1, "Red".to_string())));
        assert_eq!(
            parse_option_marker("2) Green"),
            Some((2, "Green".to_string()))
        );
        assert_eq!(
            parse_option_marker("3 - Blue"),
            Some((3, "Blue".to_string()))
        );
    }

    #[test]
    fn parse_option_marker_multi_digit_number() {
        assert_eq!(
            parse_option_marker("12. Twelfth option"),
            Some((12, "Twelfth option".to_string()))
        );
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
