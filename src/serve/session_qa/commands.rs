//! The Telegram command router's pure core (`BA.ticket.telegram-command-router`
//! task 2).
//!
//! Everything here is PURE — no I/O, no async, no Telegram client — matching
//! this module's established `sendmessage_body` / `resolve_question_response`
//! / `ChatFollowUpState` split (CLAUDE.md rule 6). The I/O shell that wires
//! this into `handle_message` and dispatches through `POST /events/` lands in
//! later tasks.

use std::collections::HashMap;

use engine_contract::envelope::{ChannelType, IngressEnvelope, SourcePayload};
use serde_json::{Value, json};

use crate::config::{TelegramCommandEntry, TelegramCommandParam, TelegramParamSource};

/// Telegram's ceiling on a `sendMessage` `text` field, in characters —
/// restated here for the same independent-provability reason as
/// [`super::QA_ANSWER_CALLBACK_TEXT_MAX_CHARS`].
pub const TELEGRAM_MESSAGE_MAX_CHARS: usize = 4096;

/// A message parsed as a `/command`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommand {
    /// The command name, lowercased, with the leading `/` and any trailing
    /// `@botname` stripped.
    pub name: String,
    /// The remainder of the message, whitespace-split into tokens.
    pub args: Vec<String>,
    /// The remainder of the message, trimmed but otherwise verbatim — a URL
    /// or a multi-word company name both need the unsplit form.
    pub rest: String,
}

/// Parse `text` as a `/command`. `None` for any text not beginning with `/`
/// (including a bare `/` with nothing after it).
#[must_use]
pub fn parse_command(text: &str) -> Option<ParsedCommand> {
    let trimmed = text.trim();
    let without_slash = trimmed.strip_prefix('/')?;
    let mut parts = without_slash.splitn(2, char::is_whitespace);
    let raw_name = parts.next().unwrap_or("");
    if raw_name.is_empty() {
        return None;
    }
    // Telegram appends `@botname` to commands in groups — strip it.
    let name = raw_name
        .split('@')
        .next()
        .unwrap_or(raw_name)
        .to_lowercase();
    if name.is_empty() {
        return None;
    }
    let rest = parts.next().unwrap_or("").trim().to_string();
    let args = if rest.is_empty() {
        Vec::new()
    } else {
        rest.split_whitespace().map(str::to_string).collect()
    };
    Some(ParsedCommand { name, args, rest })
}

/// The built-in, non-configurable commands every deployment answers,
/// regardless of the `[telegram_commands]` allow-list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOnlyCommand {
    Status,
    Lanes,
    Attention,
    Help,
}

/// Where a parsed command routes to.
#[derive(Debug, Clone, PartialEq)]
pub enum CommandRoute {
    /// A configured allow-list entry — dispatches a workflow.
    Trigger {
        name: String,
        entry: TelegramCommandEntry,
    },
    /// A built-in, read-only command.
    ReadOnly(ReadOnlyCommand),
    /// Neither a built-in nor a configured command name.
    Unknown { name: String },
}

/// Route a parsed command to what it should do.
///
/// Precedence: the built-in set (`status`, `lanes`, `attention`,
/// `help`/`commands`) resolves FIRST and is never overridable by config;
/// otherwise a name present as an allow-list KEY yields `Trigger`; otherwise
/// `Unknown`. There is deliberately no branch that dispatches a workflow
/// named directly by the message — a command spelled exactly as a real
/// registered workflow type but absent from the allow-list must come back
/// `Unknown`. That is the whole authorisation boundary.
#[must_use]
pub fn route_command(
    parsed: &ParsedCommand,
    allow_list: &HashMap<String, TelegramCommandEntry>,
) -> CommandRoute {
    match parsed.name.as_str() {
        "status" => return CommandRoute::ReadOnly(ReadOnlyCommand::Status),
        "lanes" => return CommandRoute::ReadOnly(ReadOnlyCommand::Lanes),
        "attention" => return CommandRoute::ReadOnly(ReadOnlyCommand::Attention),
        "help" | "commands" => return CommandRoute::ReadOnly(ReadOnlyCommand::Help),
        _ => {}
    }
    if let Some(entry) = allow_list.get(&parsed.name) {
        return CommandRoute::Trigger {
            name: parsed.name.clone(),
            entry: entry.clone(),
        };
    }
    CommandRoute::Unknown {
        name: parsed.name.clone(),
    }
}

/// Extract a YouTube video id from a pasted link. Pure and total — anything
/// that does not parse as a recognized YouTube URL shape is returned trimmed
/// and unchanged, presumed to already BE a bare id.
///
/// Recognized shapes: `youtube.com/watch?v=<id>` (any query-param order),
/// `youtu.be/<id>`, `youtube.com/shorts/<id>`, `youtube.com/embed/<id>` — a
/// trailing query string or fragment on any of these is stripped.
#[must_use]
pub fn youtube_video_id(raw: &str) -> String {
    let trimmed = raw.trim();

    // Strip a fragment first — it never carries meaningful content here.
    let no_fragment = trimmed.split('#').next().unwrap_or(trimmed);

    // `watch?v=<id>` — the id is a query param, not a path segment.
    if let Some(query_start) = no_fragment.find('?') {
        let (path, query) = no_fragment.split_at(query_start);
        let query = &query[1..]; // drop the leading '?'
        if path.contains("/watch") {
            for pair in query.split('&') {
                if let Some(id) = pair.strip_prefix("v=") {
                    return id.to_string();
                }
            }
        }
    }

    // Path-based shapes: `youtu.be/<id>`, `/shorts/<id>`, `/embed/<id>`.
    for marker in ["youtu.be/", "/shorts/", "/embed/"] {
        if let Some(idx) = no_fragment.find(marker) {
            let after = &no_fragment[idx + marker.len()..];
            let id = after.split(['?', '/']).next().unwrap_or(after);
            if !id.is_empty() {
                return id.to_string();
            }
        }
    }

    // Not a recognized YouTube URL shape — presumed to already be a bare id.
    trimmed.to_string()
}

/// Everything needed to build a dispatched workflow's event payload that
/// isn't already carried by the [`ParsedCommand`] or [`TelegramCommandEntry`]
/// — passed in rather than read from the clock so every case is
/// deterministically testable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerContext {
    pub chat_id: String,
    pub message_id: Option<i64>,
    pub now_rfc3339: String,
}

/// Why [`build_trigger_data`] refused to build a payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerDataError {
    /// The entry's fixed `data` is present but not a JSON object.
    FixedDataNotAnObject,
    /// An `Arg`-sourced param has no `index`.
    MissingIndex,
    /// An `Envelope`-sourced param has no `source_kind`.
    MissingSourceKind,
    /// One or more `required` params resolved to nothing. Carries every
    /// missing key in one shot, so a two-parameter command reports both in
    /// one reply rather than one per round trip.
    MissingParam { keys: Vec<String> },
}

/// Build the JSON payload a [`TelegramCommandEntry`] dispatches, from the
/// entry's fixed `data` base plus its `params` applied in order over
/// `parsed`/`ctx`. A param overwrites a same-named fixed field.
pub fn build_trigger_data(
    entry: &TelegramCommandEntry,
    parsed: &ParsedCommand,
    ctx: &TriggerContext,
) -> Result<Value, TriggerDataError> {
    let base = entry.data.clone().unwrap_or_else(|| json!({}));
    let mut map = match base {
        Value::Object(map) => map,
        _ => return Err(TriggerDataError::FixedDataNotAnObject),
    };

    let mut missing: Vec<String> = Vec::new();

    for param in &entry.params {
        let resolved = resolve_param_value(param, parsed, ctx)?;
        match resolved {
            Some(value) => {
                map.insert(param.key.clone(), value);
            }
            None if param.required => missing.push(param.key.clone()),
            None => {}
        }
    }

    if !missing.is_empty() {
        return Err(TriggerDataError::MissingParam { keys: missing });
    }

    Ok(Value::Object(map))
}

/// Resolve one param's value, or `Ok(None)` when it is legitimately absent
/// (an empty `rest`/`args`, or an out-of-range `arg` index).
fn resolve_param_value(
    param: &TelegramCommandParam,
    parsed: &ParsedCommand,
    ctx: &TriggerContext,
) -> Result<Option<Value>, TriggerDataError> {
    match param.from {
        TelegramParamSource::Rest => {
            if parsed.rest.is_empty() {
                Ok(None)
            } else {
                Ok(Some(Value::String(parsed.rest.clone())))
            }
        }
        TelegramParamSource::Args => {
            if parsed.args.is_empty() {
                Ok(None)
            } else {
                Ok(Some(Value::Array(
                    parsed.args.iter().cloned().map(Value::String).collect(),
                )))
            }
        }
        TelegramParamSource::Arg => {
            let Some(index) = param.index else {
                return Err(TriggerDataError::MissingIndex);
            };
            Ok(parsed.args.get(index).cloned().map(Value::String))
        }
        TelegramParamSource::Envelope => {
            if parsed.rest.is_empty() {
                return Ok(None);
            }
            let Some(source_kind) = param.source_kind else {
                return Err(TriggerDataError::MissingSourceKind);
            };
            let source = match source_kind {
                crate::config::TelegramSourceKind::Url => SourcePayload::Url {
                    url: parsed.rest.clone(),
                },
                crate::config::TelegramSourceKind::VideoId => SourcePayload::VideoId {
                    video_id: youtube_video_id(&parsed.rest),
                },
                crate::config::TelegramSourceKind::Text => SourcePayload::ChannelMessage {
                    text: parsed.rest.clone(),
                    attachments: vec![],
                },
            };
            let envelope = IngressEnvelope {
                envelope_id: format!("telegram-{}-{}", ctx.chat_id, ctx.message_id.unwrap_or(0)),
                channel_type: ChannelType::Telegram,
                sender_id: Some(ctx.chat_id.clone()),
                reply_context: None,
                timestamp: ctx.now_rfc3339.clone(),
                source,
                raw_payload: json!({"command": parsed.name, "text": parsed.rest}),
            };
            let value = serde_json::to_value(envelope).map_err(|_| {
                // `IngressEnvelope` always serializes; this arm exists only
                // to keep the function total instead of panicking.
                TriggerDataError::MissingSourceKind
            })?;
            Ok(Some(value))
        }
    }
}

/// Render a command's usage line from its own `params`, e.g.
/// `/research <company_name>`, `/linkedin <since> <until>`, `/status`.
/// Derived, never hand-written per command — this is what keeps a newly
/// configured command self-documenting.
#[must_use]
pub fn usage_for(name: &str, entry: &TelegramCommandEntry) -> String {
    if entry.params.is_empty() {
        return format!("/{name}");
    }
    let parts: Vec<String> = entry
        .params
        .iter()
        .map(|param| format!("<{}>", param.key))
        .collect();
    format!("/{name} {}", parts.join(" "))
}

/// Render one line per [`TriggerDataError`] variant, ending in [`usage_for`].
#[must_use]
pub fn trigger_data_error_reply(
    name: &str,
    entry: &TelegramCommandEntry,
    err: &TriggerDataError,
) -> String {
    let usage = usage_for(name, entry);
    match err {
        TriggerDataError::FixedDataNotAnObject => {
            format!("/{name} is misconfigured (fixed data is not an object).\nUsage: {usage}")
        }
        TriggerDataError::MissingIndex => {
            format!("/{name} is misconfigured (an `arg` param has no index).\nUsage: {usage}")
        }
        TriggerDataError::MissingSourceKind => {
            format!(
                "/{name} is misconfigured (an `envelope` param has no source_kind).\nUsage: {usage}"
            )
        }
        TriggerDataError::MissingParam { keys } => {
            format!(
                "/{name} is missing required parameter(s): {}.\nUsage: {usage}",
                keys.join(", ")
            )
        }
    }
}

/// Every built-in plus every allow-list key, sorted (deterministic), each
/// rendered through [`usage_for`]. A command added to the config appears
/// here with no code change.
#[must_use]
pub fn available_commands_reply(allow_list: &HashMap<String, TelegramCommandEntry>) -> String {
    let mut lines: Vec<String> = vec![
        "/status".to_string(),
        "/lanes".to_string(),
        "/attention".to_string(),
        "/help (alias /commands)".to_string(),
    ];

    let mut names: Vec<&String> = allow_list.keys().collect();
    names.sort();
    for name in names {
        let entry = &allow_list[name];
        lines.push(usage_for(name, entry));
    }

    format!("Available commands:\n{}", lines.join("\n"))
}

/// Name the unrecognised command, then delegate to
/// [`available_commands_reply`].
#[must_use]
pub fn unknown_command_reply(
    name: &str,
    allow_list: &HashMap<String, TelegramCommandEntry>,
) -> String {
    format!(
        "Unknown command: /{name}\n\n{}",
        available_commands_reply(allow_list)
    )
}

/// The chat-id pin: exact string equality between the message's `chat_id`
/// and the configured one.
#[must_use]
pub fn is_authorized(message_chat_id: &str, configured_chat_id: &str) -> bool {
    message_chat_id == configured_chat_id
}

/// Truncate `text` to at most [`TELEGRAM_MESSAGE_MAX_CHARS`] characters,
/// appending a visible marker when truncation happens. Char-based like
/// `clamp_chars`, so a multi-byte char is never split.
#[must_use]
pub fn truncate_for_telegram(text: &str) -> String {
    if text.chars().count() <= TELEGRAM_MESSAGE_MAX_CHARS {
        return text.to_string();
    }
    const MARKER: &str = "\n… [truncated]";
    let marker_chars = MARKER.chars().count();
    let keep = TELEGRAM_MESSAGE_MAX_CHARS.saturating_sub(marker_chars);
    let truncated: String = text.chars().take(keep).collect();
    format!("{truncated}{MARKER}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{TelegramCommandParam, TelegramSourceKind};

    // ── parse_command ────────────────────────────────────────────────────

    #[test]
    fn plain_text_is_not_a_command() {
        assert_eq!(parse_command("hello there"), None);
    }

    #[test]
    fn status_parses() {
        let parsed = parse_command("/status").unwrap();
        assert_eq!(parsed.name, "status");
        assert!(parsed.args.is_empty());
        assert_eq!(parsed.rest, "");
    }

    #[test]
    fn botname_suffix_is_stripped() {
        let parsed = parse_command("/status@CodeSessionsBot").unwrap();
        assert_eq!(parsed.name, "status");
    }

    #[test]
    fn research_with_two_word_company_name() {
        let parsed = parse_command("/research Acme Corp").unwrap();
        assert_eq!(parsed.name, "research");
        assert_eq!(parsed.args, vec!["Acme".to_string(), "Corp".to_string()]);
        assert_eq!(parsed.rest, "Acme Corp");
    }

    #[test]
    fn bare_slash_is_none() {
        assert_eq!(parse_command("/"), None);
    }

    #[test]
    fn surrounding_whitespace_is_tolerated() {
        let parsed = parse_command("  /status  ").unwrap();
        assert_eq!(parsed.name, "status");
    }

    #[test]
    fn name_is_lowercased() {
        let parsed = parse_command("/STATUS").unwrap();
        assert_eq!(parsed.name, "status");
    }

    // ── route_command ────────────────────────────────────────────────────

    fn empty_allow_list() -> HashMap<String, TelegramCommandEntry> {
        HashMap::new()
    }

    #[test]
    fn each_builtin_routes_to_its_variant() {
        let allow_list = empty_allow_list();
        assert_eq!(
            route_command(&parse_command("/status").unwrap(), &allow_list),
            CommandRoute::ReadOnly(ReadOnlyCommand::Status)
        );
        assert_eq!(
            route_command(&parse_command("/lanes").unwrap(), &allow_list),
            CommandRoute::ReadOnly(ReadOnlyCommand::Lanes)
        );
        assert_eq!(
            route_command(&parse_command("/attention").unwrap(), &allow_list),
            CommandRoute::ReadOnly(ReadOnlyCommand::Attention)
        );
    }

    #[test]
    fn help_and_commands_both_route_to_help() {
        let allow_list = empty_allow_list();
        assert_eq!(
            route_command(&parse_command("/help").unwrap(), &allow_list),
            CommandRoute::ReadOnly(ReadOnlyCommand::Help)
        );
        assert_eq!(
            route_command(&parse_command("/commands").unwrap(), &allow_list),
            CommandRoute::ReadOnly(ReadOnlyCommand::Help)
        );
    }

    #[test]
    fn configured_name_routes_to_trigger() {
        let mut allow_list = empty_allow_list();
        allow_list.insert(
            "research".to_string(),
            TelegramCommandEntry {
                workflow_type: "RESEARCH_AGENT".to_string(),
                params: vec![],
                data: None,
            },
        );
        let route = route_command(&parse_command("/research Acme").unwrap(), &allow_list);
        assert!(matches!(route, CommandRoute::Trigger { name, .. } if name == "research"));
    }

    #[test]
    fn name_not_in_allow_list_is_unknown_even_when_a_real_workflow_type() {
        // `sdlc_flow` is a real registered workflow type but is not a key
        // in the allow-list — this is the whole authorisation boundary.
        let allow_list = empty_allow_list();
        let route = route_command(&parse_command("/sdlc_flow").unwrap(), &allow_list);
        assert_eq!(
            route,
            CommandRoute::Unknown {
                name: "sdlc_flow".to_string()
            }
        );
    }

    // ── build_trigger_data ───────────────────────────────────────────────

    fn ctx() -> TriggerContext {
        TriggerContext {
            chat_id: "12345".to_string(),
            message_id: Some(999),
            now_rfc3339: "2026-08-31T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn one_required_string_from_rest_round_trips_diagnostic_intake() {
        let entry = TelegramCommandEntry {
            workflow_type: "DIAGNOSTIC_INTAKE".to_string(),
            params: vec![TelegramCommandParam {
                key: "notes".to_string(),
                from: TelegramParamSource::Rest,
                index: None,
                source_kind: None,
                required: true,
            }],
            data: None,
        };
        let parsed = parse_command("/intake we ship slowly").unwrap();
        let data = build_trigger_data(&entry, &parsed, &ctx()).unwrap();
        assert_eq!(data, json!({"notes": "we ship slowly"}));

        let schema: engine_core::workflows::diagnostic_intake::schema::DiagnosticIntakeEventSchema =
            serde_json::from_value(data).expect("should deserialize into the real schema");
        assert_eq!(schema.notes, "we ship slowly");
    }

    #[test]
    fn enum_plus_argument_round_trips_research_agent() {
        let entry = TelegramCommandEntry {
            workflow_type: "RESEARCH_AGENT".to_string(),
            params: vec![TelegramCommandParam {
                key: "company_name".to_string(),
                from: TelegramParamSource::Rest,
                index: None,
                source_kind: None,
                required: true,
            }],
            data: Some(json!({"mode": "company", "profile": "thorough"})),
        };
        let parsed = parse_command("/research Acme Corp").unwrap();
        let data = build_trigger_data(&entry, &parsed, &ctx()).unwrap();
        assert_eq!(
            data,
            json!({"mode": "company", "profile": "thorough", "company_name": "Acme Corp"})
        );

        let schema: engine_core::workflows::research_agent::schema::ResearchAgentEventSchema =
            serde_json::from_value(data).expect("should deserialize into the real schema");
        assert_eq!(schema.company_name, Some("Acme Corp".to_string()));
        assert_eq!(schema.profile, Some("thorough".to_string()));
    }

    #[test]
    fn two_positionals_round_trip_linkedin_post() {
        let entry = TelegramCommandEntry {
            workflow_type: "LINKEDIN_POST".to_string(),
            params: vec![
                TelegramCommandParam {
                    key: "since".to_string(),
                    from: TelegramParamSource::Arg,
                    index: Some(0),
                    source_kind: None,
                    required: true,
                },
                TelegramCommandParam {
                    key: "until".to_string(),
                    from: TelegramParamSource::Arg,
                    index: Some(1),
                    source_kind: None,
                    required: true,
                },
            ],
            data: None,
        };
        let parsed = parse_command("/linkedin 2026-08-01 2026-08-31").unwrap();
        let data = build_trigger_data(&entry, &parsed, &ctx()).unwrap();
        assert_eq!(data, json!({"since": "2026-08-01", "until": "2026-08-31"}));

        let schema: engine_core::workflows::linkedin_post::schema::LinkedInPostEventSchema =
            serde_json::from_value(data).expect("should deserialize into the real schema");
        assert_eq!(schema.since, "2026-08-01");
        assert_eq!(schema.until, "2026-08-31");
    }

    #[test]
    fn two_positionals_with_only_one_token_reports_missing_until() {
        let entry = TelegramCommandEntry {
            workflow_type: "LINKEDIN_POST".to_string(),
            params: vec![
                TelegramCommandParam {
                    key: "since".to_string(),
                    from: TelegramParamSource::Arg,
                    index: Some(0),
                    source_kind: None,
                    required: true,
                },
                TelegramCommandParam {
                    key: "until".to_string(),
                    from: TelegramParamSource::Arg,
                    index: Some(1),
                    source_kind: None,
                    required: true,
                },
            ],
            data: None,
        };
        let parsed = parse_command("/linkedin 2026-08-01").unwrap();
        let err = build_trigger_data(&entry, &parsed, &ctx()).unwrap_err();
        assert_eq!(
            err,
            TriggerDataError::MissingParam {
                keys: vec!["until".to_string()]
            }
        );
    }

    #[test]
    fn list_from_args_round_trips_with_fixed_data() {
        let entry = TelegramCommandEntry {
            workflow_type: "PRICE_SCOUT".to_string(),
            params: vec![TelegramCommandParam {
                key: "items".to_string(),
                from: TelegramParamSource::Args,
                index: None,
                source_kind: None,
                required: true,
            }],
            data: Some(json!({"region": "BR"})),
        };
        let parsed = parse_command("/shop nike-shoes coffee-beans").unwrap();
        let data = build_trigger_data(&entry, &parsed, &ctx()).unwrap();
        assert_eq!(
            data,
            json!({"region": "BR", "items": ["nike-shoes", "coffee-beans"]})
        );
    }

    #[test]
    fn envelope_url_round_trips_content_pipeline() {
        let entry = TelegramCommandEntry {
            workflow_type: "CONTENT_PIPELINE".to_string(),
            params: vec![TelegramCommandParam {
                key: "envelope".to_string(),
                from: TelegramParamSource::Envelope,
                index: None,
                source_kind: Some(TelegramSourceKind::Url),
                required: true,
            }],
            data: None,
        };
        let parsed = parse_command("/article https://example.com/post").unwrap();
        let data = build_trigger_data(&entry, &parsed, &ctx()).unwrap();

        let envelope_val = data.get("envelope").unwrap().clone();
        let envelope: IngressEnvelope = serde_json::from_value(envelope_val).unwrap();
        assert_eq!(
            envelope.source,
            SourcePayload::Url {
                url: "https://example.com/post".to_string()
            }
        );
        assert_eq!(envelope.channel_type, ChannelType::Telegram);
        assert_eq!(envelope.sender_id, Some("12345".to_string()));

        let full: engine_core::workflows::content_pipeline::schema::ContentPipelineInput =
            serde_json::from_value(data).expect("should deserialize into the real schema");
        assert_eq!(full.envelope.channel_type, ChannelType::Telegram);
        assert_eq!(full.envelope.sender_id, Some("12345".to_string()));
    }

    #[test]
    fn envelope_video_id_extracts_id_from_link_not_raw_url() {
        let entry = TelegramCommandEntry {
            workflow_type: "CONTENT_PIPELINE".to_string(),
            params: vec![TelegramCommandParam {
                key: "envelope".to_string(),
                from: TelegramParamSource::Envelope,
                index: None,
                source_kind: Some(TelegramSourceKind::VideoId),
                required: true,
            }],
            data: None,
        };
        let parsed = parse_command("/yt https://youtu.be/dQw4w9WgXcQ").unwrap();
        let data = build_trigger_data(&entry, &parsed, &ctx()).unwrap();

        let full: engine_core::workflows::content_pipeline::schema::ContentPipelineInput =
            serde_json::from_value(data).expect("should deserialize into the real schema");
        assert_eq!(
            full.envelope.source,
            SourcePayload::VideoId {
                video_id: "dQw4w9WgXcQ".to_string()
            }
        );
        assert_eq!(full.envelope.channel_type, ChannelType::Telegram);
        assert_eq!(full.envelope.sender_id, Some("12345".to_string()));
    }

    #[test]
    fn param_wins_over_colliding_fixed_data_key() {
        let entry = TelegramCommandEntry {
            workflow_type: "RESEARCH_AGENT".to_string(),
            params: vec![TelegramCommandParam {
                key: "mode".to_string(),
                from: TelegramParamSource::Rest,
                index: None,
                source_kind: None,
                required: true,
            }],
            data: Some(json!({"mode": "prospecting"})),
        };
        let parsed = parse_command("/research company").unwrap();
        let data = build_trigger_data(&entry, &parsed, &ctx()).unwrap();
        assert_eq!(data["mode"], json!("company"));
    }

    #[test]
    fn optional_missing_param_is_simply_absent() {
        let entry = TelegramCommandEntry {
            workflow_type: "RESEARCH_AGENT".to_string(),
            params: vec![TelegramCommandParam {
                key: "company_url".to_string(),
                from: TelegramParamSource::Rest,
                index: None,
                source_kind: None,
                required: false,
            }],
            data: Some(json!({"mode": "company"})),
        };
        let parsed = parse_command("/research").unwrap();
        let data = build_trigger_data(&entry, &parsed, &ctx()).unwrap();
        assert_eq!(data, json!({"mode": "company"}));
        assert!(data.get("company_url").is_none());
    }

    #[test]
    fn two_missing_required_params_report_both() {
        let entry = TelegramCommandEntry {
            workflow_type: "LINKEDIN_POST".to_string(),
            params: vec![
                TelegramCommandParam {
                    key: "since".to_string(),
                    from: TelegramParamSource::Arg,
                    index: Some(0),
                    source_kind: None,
                    required: true,
                },
                TelegramCommandParam {
                    key: "until".to_string(),
                    from: TelegramParamSource::Arg,
                    index: Some(1),
                    source_kind: None,
                    required: true,
                },
            ],
            data: None,
        };
        let parsed = parse_command("/linkedin").unwrap();
        let err = build_trigger_data(&entry, &parsed, &ctx()).unwrap_err();
        assert_eq!(
            err,
            TriggerDataError::MissingParam {
                keys: vec!["since".to_string(), "until".to_string()]
            }
        );
    }

    #[test]
    fn arg_with_no_index_is_missing_index_error() {
        let entry = TelegramCommandEntry {
            workflow_type: "LINKEDIN_POST".to_string(),
            params: vec![TelegramCommandParam {
                key: "since".to_string(),
                from: TelegramParamSource::Arg,
                index: None,
                source_kind: None,
                required: true,
            }],
            data: None,
        };
        let parsed = parse_command("/linkedin 2026-08-01").unwrap();
        let err = build_trigger_data(&entry, &parsed, &ctx()).unwrap_err();
        assert_eq!(err, TriggerDataError::MissingIndex);
    }

    #[test]
    fn envelope_with_no_source_kind_is_missing_source_kind_error() {
        let entry = TelegramCommandEntry {
            workflow_type: "CONTENT_PIPELINE".to_string(),
            params: vec![TelegramCommandParam {
                key: "envelope".to_string(),
                from: TelegramParamSource::Envelope,
                index: None,
                source_kind: None,
                required: true,
            }],
            data: None,
        };
        let parsed = parse_command("/article https://example.com").unwrap();
        let err = build_trigger_data(&entry, &parsed, &ctx()).unwrap_err();
        assert_eq!(err, TriggerDataError::MissingSourceKind);
    }

    #[test]
    fn non_object_fixed_data_is_error_not_panic() {
        let entry = TelegramCommandEntry {
            workflow_type: "RESEARCH_AGENT".to_string(),
            params: vec![],
            data: Some(json!("not an object")),
        };
        let parsed = parse_command("/research").unwrap();
        let err = build_trigger_data(&entry, &parsed, &ctx()).unwrap_err();
        assert_eq!(err, TriggerDataError::FixedDataNotAnObject);
    }

    #[test]
    fn no_params_and_no_fixed_data_yields_empty_object() {
        let entry = TelegramCommandEntry {
            workflow_type: "STATUS_PING".to_string(),
            params: vec![],
            data: None,
        };
        let parsed = parse_command("/status").unwrap();
        let data = build_trigger_data(&entry, &parsed, &ctx()).unwrap();
        assert_eq!(data, json!({}));
    }

    // ── youtube_video_id ─────────────────────────────────────────────────

    #[test]
    fn watch_url_extracts_id() {
        assert_eq!(
            youtube_video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
            "dQw4w9WgXcQ"
        );
    }

    #[test]
    fn watch_url_with_extra_query_params_in_any_order_extracts_id() {
        assert_eq!(
            youtube_video_id("https://www.youtube.com/watch?list=PL123&v=dQw4w9WgXcQ&t=5s"),
            "dQw4w9WgXcQ"
        );
    }

    #[test]
    fn short_link_extracts_id() {
        assert_eq!(
            youtube_video_id("https://youtu.be/dQw4w9WgXcQ"),
            "dQw4w9WgXcQ"
        );
    }

    #[test]
    fn short_link_with_query_tail_strips_it() {
        assert_eq!(
            youtube_video_id("https://youtu.be/dQw4w9WgXcQ?t=42"),
            "dQw4w9WgXcQ"
        );
    }

    #[test]
    fn shorts_path_extracts_id() {
        assert_eq!(
            youtube_video_id("https://www.youtube.com/shorts/dQw4w9WgXcQ"),
            "dQw4w9WgXcQ"
        );
    }

    #[test]
    fn embed_path_extracts_id() {
        assert_eq!(
            youtube_video_id("https://www.youtube.com/embed/dQw4w9WgXcQ"),
            "dQw4w9WgXcQ"
        );
    }

    #[test]
    fn bare_id_is_returned_unchanged() {
        assert_eq!(youtube_video_id("dQw4w9WgXcQ"), "dQw4w9WgXcQ");
    }

    #[test]
    fn whitespace_is_trimmed() {
        assert_eq!(youtube_video_id("  dQw4w9WgXcQ  "), "dQw4w9WgXcQ");
    }

    #[test]
    fn non_url_string_is_unchanged_and_does_not_panic() {
        assert_eq!(youtube_video_id("not a url at all"), "not a url at all");
    }

    // ── usage_for / available_commands_reply / unknown_command_reply ────

    fn example_entries() -> HashMap<String, TelegramCommandEntry> {
        let mut map = HashMap::new();
        map.insert(
            "status_ping".to_string(),
            TelegramCommandEntry {
                workflow_type: "STATUS_PING".to_string(),
                params: vec![],
                data: None,
            },
        );
        map.insert(
            "research".to_string(),
            TelegramCommandEntry {
                workflow_type: "RESEARCH_AGENT".to_string(),
                params: vec![TelegramCommandParam {
                    key: "company_name".to_string(),
                    from: TelegramParamSource::Rest,
                    index: None,
                    source_kind: None,
                    required: true,
                }],
                data: Some(json!({"mode": "company"})),
            },
        );
        map.insert(
            "linkedin".to_string(),
            TelegramCommandEntry {
                workflow_type: "LINKEDIN_POST".to_string(),
                params: vec![
                    TelegramCommandParam {
                        key: "since".to_string(),
                        from: TelegramParamSource::Arg,
                        index: Some(0),
                        source_kind: None,
                        required: true,
                    },
                    TelegramCommandParam {
                        key: "until".to_string(),
                        from: TelegramParamSource::Arg,
                        index: Some(1),
                        source_kind: None,
                        required: true,
                    },
                ],
                data: None,
            },
        );
        map.insert(
            "yt".to_string(),
            TelegramCommandEntry {
                workflow_type: "CONTENT_PIPELINE".to_string(),
                params: vec![TelegramCommandParam {
                    key: "envelope".to_string(),
                    from: TelegramParamSource::Envelope,
                    index: None,
                    source_kind: Some(TelegramSourceKind::VideoId),
                    required: true,
                }],
                data: None,
            },
        );
        map.insert(
            "article".to_string(),
            TelegramCommandEntry {
                workflow_type: "CONTENT_PIPELINE".to_string(),
                params: vec![TelegramCommandParam {
                    key: "envelope".to_string(),
                    from: TelegramParamSource::Envelope,
                    index: None,
                    source_kind: Some(TelegramSourceKind::Url),
                    required: true,
                }],
                data: None,
            },
        );
        map.insert(
            "shop".to_string(),
            TelegramCommandEntry {
                workflow_type: "PRICE_SCOUT".to_string(),
                params: vec![TelegramCommandParam {
                    key: "items".to_string(),
                    from: TelegramParamSource::Args,
                    index: None,
                    source_kind: None,
                    required: true,
                }],
                data: Some(json!({"region": "BR"})),
            },
        );
        map
    }

    #[test]
    fn usage_for_renders_each_example_entry_correctly() {
        let entries = example_entries();
        assert_eq!(
            usage_for("status_ping", &entries["status_ping"]),
            "/status_ping"
        );
        assert_eq!(
            usage_for("research", &entries["research"]),
            "/research <company_name>"
        );
        assert_eq!(
            usage_for("linkedin", &entries["linkedin"]),
            "/linkedin <since> <until>"
        );
        assert_eq!(usage_for("yt", &entries["yt"]), "/yt <envelope>");
        assert_eq!(
            usage_for("article", &entries["article"]),
            "/article <envelope>"
        );
        assert_eq!(usage_for("shop", &entries["shop"]), "/shop <items>");
    }

    #[test]
    fn available_commands_reply_contains_every_builtin_and_allow_list_key() {
        let entries = example_entries();
        let reply = available_commands_reply(&entries);
        assert!(reply.contains("/status"));
        assert!(reply.contains("/lanes"));
        assert!(reply.contains("/attention"));
        assert!(reply.contains("/help"));
        for name in entries.keys() {
            assert!(reply.contains(name), "reply missing entry: {name}");
        }
    }

    #[test]
    fn adding_an_entry_changes_only_that_entrys_line() {
        let mut entries = example_entries();
        let before = available_commands_reply(&entries);
        entries.insert(
            "newcmd".to_string(),
            TelegramCommandEntry {
                workflow_type: "NEW_WORKFLOW".to_string(),
                params: vec![],
                data: None,
            },
        );
        let after = available_commands_reply(&entries);
        assert!(!before.contains("newcmd"));
        assert!(after.contains("/newcmd"));
        // Every line from before still appears in after.
        for line in before.lines() {
            assert!(after.contains(line));
        }
    }

    #[test]
    fn unknown_command_reply_names_the_command_and_lists_available() {
        let entries = example_entries();
        let reply = unknown_command_reply("bogus", &entries);
        assert!(reply.contains("/bogus"));
        assert!(reply.contains("/status"));
    }

    // ── is_authorized ────────────────────────────────────────────────────

    #[test]
    fn is_authorized_exact_match() {
        assert!(is_authorized("12345", "12345"));
        assert!(!is_authorized("12345", "99999"));
    }

    // ── truncate_for_telegram ────────────────────────────────────────────

    #[test]
    fn short_string_truncates_to_itself() {
        assert_eq!(truncate_for_telegram("hello"), "hello");
    }

    #[test]
    fn long_string_truncates_to_exactly_max_chars_with_marker() {
        let long = "a".repeat(5000);
        let result = truncate_for_telegram(&long);
        assert_eq!(result.chars().count(), TELEGRAM_MESSAGE_MAX_CHARS);
        assert!(result.ends_with("[truncated]"));
    }

    #[test]
    fn multi_byte_string_does_not_panic() {
        let long = "é".repeat(5000);
        let result = truncate_for_telegram(&long);
        assert!(result.chars().count() <= TELEGRAM_MESSAGE_MAX_CHARS);
    }
}
