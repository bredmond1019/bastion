//! The family's `pricescout` bot loop — pure core
//! (`BA.ticket.pricescout-telegram-bot` task 2).
//!
//! Everything here is PURE — no I/O, no async, no Telegram client, no HTTP
//! call — matching this crate's established `sendmessage_body` /
//! `parse_command` split (CLAUDE.md rule 6). The I/O shell that wires this
//! into an actual inbound Telegram loop and calls price-scout's
//! `POST /api/lists` lands in task 3, kept thin over this core.
//!
//! **This module deliberately does NOT reuse
//! `session_qa::commands::route_command`.** That router's built-in set
//! (`/status`, `/lanes`, `/attention`, `/help`) resolves unconditionally,
//! regardless of any allow-list passed to it — by design, for the
//! operator's bot, where those are always-on read-only commands. This bot's
//! audience is the family, and `/status`/`/lanes`/`/attention` leak
//! operator-facing information (what workflows are running, what's stuck)
//! that must never reach a bot handed to non-operators. So the family's
//! router is its own, narrower dispatch: exactly one name, `shop`, and
//! everything else — including every one of the operator's built-ins and
//! every entry in the operator's `[telegram_commands]` allow-list — is
//! refused identically. What IS reused: `parse_command` (splitting `/name
//! args` the same way Telegram messages are split everywhere else in this
//! crate) and `is_authorized` (the same chat-id pin), both from
//! `super::session_qa::commands`. No follow-up conversation state exists
//! here — `SessionQaBridge`'s follow-up machinery lives on `codesessions`
//! only; replicating it on this loop would import the precedence hazard a
//! dedicated bot exists to avoid.

use serde_json::{Value, json};

use super::session_qa::commands::{is_authorized, parse_command};

/// The only command name this bot answers. Exported so the spawn/dispatch
/// shell (task 3) and tests share one literal rather than each hardcoding
/// `"shop"` independently.
pub const SHOP_COMMAND: &str = "shop";

/// The family-safe allow-list, as a plain name set rather than the
/// operator's `HashMap<String, TelegramCommandEntry>` shape — `/shop`
/// dispatches to price-scout's list endpoint directly, it does not trigger
/// an in-process workflow the way an operator `[telegram_commands]` entry
/// does, so `TelegramCommandEntry` (workflow_type + params) does not fit
/// what this command needs. A future second family command would be added
/// here as another literal.
pub const PRICESCOUT_ALLOWED_COMMANDS: &[&str] = &[SHOP_COMMAND];

/// Where a parsed message routes to on the family's bot.
#[derive(Debug, Clone, PartialEq)]
pub enum PricescoutRoute {
    /// `/shop <items>` — carries the already-parsed, trimmed, blank-free
    /// query list.
    Shop { queries: Vec<String> },
    /// Not `/shop` — every built-in the operator's router recognizes
    /// (`status`, `lanes`, `attention`, `help`/`commands`), every operator
    /// `[telegram_commands]` entry, and any other text, are all refused
    /// identically. `name` is `None` for a message that was not even a
    /// `/command` (plain text).
    Refused { name: Option<String> },
}

/// Split `/shop <items>` into a trimmed, blank-free list of queries.
///
/// Accepts both a comma-separated list (the natural shape for a shopping
/// list typed on a phone: `/shop milk, eggs, bread`) and a newline-separated
/// one (multi-line paste). Entries are trimmed; empty entries — including
/// ones that were only whitespace — are dropped, since price-scout's
/// `POST /api/lists` rejects a blank query with a 422 before its route body
/// even runs, and there is no reason to round-trip that error back to the
/// family instead of just not sending the blank entry.
#[must_use]
pub fn parse_shop_items(rest: &str) -> Vec<String> {
    rest.split([',', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Route a raw Telegram message `text` for the family's bot.
///
/// Not a `/command` at all -> `Refused { name: None }`. `/shop` -> `Shop`
/// with its parsed query list (which may be empty, if every entry was
/// blank — the I/O shell decides whether an empty list still gets an
/// acknowledgement or a usage reply; that is not this pure function's call).
/// Anything else, built-in or not, -> `Refused { name: Some(name) }`.
#[must_use]
pub fn route_pricescout_message(text: &str) -> PricescoutRoute {
    let Some(parsed) = parse_command(text) else {
        return PricescoutRoute::Refused { name: None };
    };
    if parsed.name == SHOP_COMMAND {
        return PricescoutRoute::Shop {
            queries: parse_shop_items(&parsed.rest),
        };
    }
    PricescoutRoute::Refused {
        name: Some(parsed.name),
    }
}

/// The chat-id pin for this bot — a thin re-export of
/// `session_qa::commands::is_authorized` under this module's name, so
/// call sites in `pricescout` never need to reach into `session_qa`
/// directly for it.
#[must_use]
pub fn is_family_chat(message_chat_id: &str, configured_chat_id: &str) -> bool {
    is_authorized(message_chat_id, configured_chat_id)
}

/// Build the `POST /api/lists` request body for a parsed `/shop` command.
///
/// PURE — returns the JSON value; sending it is task 3's job. Mirrors
/// `JobRequest`'s shape (`price_scout/models.py`): `queries[]`, `sources[]`
/// (fixed to `["mercado_livre"]` — the only adapter this bot targets),
/// `pages` (fixed to `1`), plus `name?` and `source`. `source` is fixed to
/// `"telegram"` — `ListSource` (`price_scout/models.py:606`) is
/// `Literal["api","telegram","ui"]` and that variant exists for exactly
/// this caller; it must never default to `"api"`.
#[must_use]
pub fn build_shop_list_body(queries: &[String], name: Option<&str>) -> Value {
    let mut body = json!({
        "source": "telegram",
        "queries": queries,
        "sources": ["mercado_livre"],
        "pages": 1,
    });
    if let Some(name) = name {
        body["name"] = json!(name);
    }
    body
}

/// The immediate acknowledgement text sent back on `/shop`, before
/// price-scout's batch has even started — task 3 wires this to a reply that
/// does not block on the `POST /api/lists` response, which can take minutes
/// for a family-sized list.
#[must_use]
pub fn shop_ack_text(queries: &[String]) -> String {
    match queries.len() {
        0 => "No items to shop for — send /shop followed by a comma-separated list, \
              e.g. /shop milk, eggs, bread"
            .to_string(),
        1 => "Got it — looking up 1 item. You'll hear back when it's ready.".to_string(),
        n => format!("Got it — looking up {n} items. You'll hear back when it's ready."),
    }
}

#[cfg(test)]
mod tests;
