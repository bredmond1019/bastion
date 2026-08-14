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
    // Trim trailing whitespace first.
    while matches!(chars.last(), Some(c) if c.is_whitespace()) {
        chars.pop();
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

/// Parse a captured `AskUserQuestion` pane into structured data.
///
/// Returns `None` when the pane does not carry the AskUserQuestion footer marker, or
/// when it does but no numbered options could be found (a marker with zero options is
/// treated as unparseable, not as an empty prompt).
pub fn parse_ask_question(screen: &str) -> Option<AskQuestionPrompt> {
    if !screen.contains(ASK_QUESTION_MARKER) {
        return None;
    }

    // Classify each line: strip decoration, record its content and indentation.
    let lines: Vec<(String, usize)> = screen.lines().map(strip_decoration).collect();

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

    for (content, indent) in &lines {
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

    Some(AskQuestionPrompt { question, options })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../detect/fixtures/claude_awaiting_question.txt");

    #[test]
    fn parses_real_fixture_happy_path() {
        // Fixture provenance (see planning/BA.20.B/tasks.md Notes): this fixture is
        // SYNTHESIZED, not a real `tmux capture-pane` capture (BA.20.A task 2 had no
        // live AskUserQuestion session available). Its three options are all genuine
        // retry-policy choices — none of them reads as a free-text escape hatch, so
        // this happy-path test intentionally does NOT assert `is_escape_hatch == true`
        // on the trailing option. See this spec's Amendment Log for the deviation from
        // the literal tasks.json wording.
        let parsed = parse_ask_question(FIXTURE).expect("fixture should parse");

        assert_eq!(
            parsed.question,
            "Which approach should I take for the retry policy?"
        );
        assert_eq!(parsed.options.len(), 3);

        assert_eq!(parsed.options[0].number, 1);
        assert_eq!(parsed.options[0].label, "Exponential backoff with jitter");
        assert_eq!(parsed.options[1].number, 2);
        assert_eq!(parsed.options[1].label, "Fixed delay between retries");
        assert_eq!(parsed.options[2].number, 3);
        assert_eq!(parsed.options[2].label, "No retries — fail fast");

        // No option in this synthesized fixture soft-matches the escape-hatch hints —
        // proving the parser does not fabricate the flag from structural position alone.
        for opt in &parsed.options {
            assert!(!opt.is_escape_hatch);
        }
    }
}
