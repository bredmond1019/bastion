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

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use super::notify::NotifyError;
use super::session_qa::commands::{is_authorized, parse_command};
use super::session_qa::{HttpQaTelegramClient, QaTelegramClient};
use crate::config::BotCredentials;

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

// ── I/O shell (task 3) ──────────────────────────────────────────────────
//
// Everything above this line is the pure core from task 2 (no I/O, no
// async, no network). Everything below is the thin shell that wires it
// into an actual `getUpdates` long-poll loop against the dedicated
// `pricescout` bot token, and submits `/shop` to price-scout's
// `POST /api/lists`. The shell is deliberately thin over the pure core —
// per CLAUDE.md rule 6, the untestable I/O parts here are limited to the
// Telegram HTTP calls (reused wholesale from `session_qa`'s already-tested
// `QaTelegramClient`/`HttpQaTelegramClient`) and this module's own
// `HttpPriceScoutListClient`, both injected as trait objects so
// `PricescoutBridge`'s dispatch logic is fully unit-testable with no
// network and no bot token (`tests.rs`).

/// Default base URL for price-scout's `POST /api/lists`, used when
/// `BASTION_PRICESCOUT_LIST_URL` is unset — price-scout's local FastAPI dev
/// server's default port. Override via that env var for any other
/// deployment (documented alongside the bot's own two env vars in
/// `.env.example` / `docs/config.md`).
pub const DEFAULT_LIST_URL: &str = "http://localhost:8000/api/lists";

/// Seam over price-scout's `POST /api/lists`, boxed as a trait object so
/// tests can inject a fake with no real network call — mirrors
/// `QaTelegramClient`'s shape in `session_qa`.
///
/// Deliberately returns `Result<(), String>` rather than a typed error:
/// nothing downstream branches on *why* the submit failed, only whether it
/// did — the failure is logged and otherwise swallowed, since the
/// acknowledgement has already gone out by the time this resolves (see
/// [`PricescoutBridge::handle_shop`]) and the completion notification is
/// PS.9.D's job, not this loop's.
#[async_trait]
pub trait PriceScoutListClient: Send + Sync {
    async fn submit_list(&self, body: Value) -> Result<(), String>;
}

/// Real `reqwest`-backed [`PriceScoutListClient`] against price-scout's
/// `POST /api/lists`.
///
/// **Attaches no credential of any kind.** `POST /api/lists` is
/// unauthenticated today (`routes_lists.py`'s only FastAPI dependencies are
/// `get_store`, `get_job_runner`, `get_list_notifier` — no API-key
/// dependency), and price-scout's `integration-contract.md` gates only the
/// four `/api/jobs*` routes with `X-API-Key`. That the route is ungated is
/// a real finding, filed as carryover against price-scout — not this
/// block's to fix — and is exactly why this client must never be tempted
/// to attach `PRICE_SCOUT_JOBS_API_KEY` or any other secret "just in case".
pub struct HttpPriceScoutListClient {
    list_url: String,
    client: reqwest::Client,
}

impl HttpPriceScoutListClient {
    #[must_use]
    pub fn new(list_url: String) -> Self {
        Self {
            list_url,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl PriceScoutListClient for HttpPriceScoutListClient {
    async fn submit_list(&self, body: Value) -> Result<(), String> {
        let resp = self
            .client
            .post(&self.list_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("POST {} failed: {e}", self.list_url))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("POST {} returned HTTP {status}", self.list_url));
        }
        Ok(())
    }
}

/// The runtime bridge: runs one `getUpdates` long-poll loop against the
/// dedicated `pricescout` bot token, and on `/shop` submits to
/// price-scout's list endpoint without blocking the acknowledgement.
///
/// No follow-up conversation state (unlike `SessionQaBridge`) — this loop
/// has nothing to collide with, by construction (see the module doc
/// comment above).
pub struct PricescoutBridge {
    chat_id: String,
    client: Arc<dyn QaTelegramClient>,
    list_client: Arc<dyn PriceScoutListClient>,
}

impl PricescoutBridge {
    /// Long-poll timeout asked of Telegram for `getUpdates` — mirrors
    /// `HttpQaTelegramClient::GETUPDATES_TIMEOUT_SECS`, which is private to
    /// `session_qa` and so cannot be referenced from here directly.
    const GETUPDATES_TIMEOUT_SECS: u64 = 30;

    /// Construct a bridge against the real Telegram API (the dedicated
    /// `pricescout` token) and the real price-scout `POST /api/lists`
    /// endpoint at `list_url`.
    #[must_use]
    pub fn new(creds: BotCredentials, list_url: String) -> Self {
        Self::with_seams(
            creds.chat_id,
            Arc::new(HttpQaTelegramClient::new(creds.bot_token)),
            Arc::new(HttpPriceScoutListClient::new(list_url)),
        )
    }

    /// Construct a bridge with every I/O seam injected — the constructor
    /// `tests.rs`'s hermetic tests use.
    #[must_use]
    pub fn with_seams(
        chat_id: String,
        client: Arc<dyn QaTelegramClient>,
        list_client: Arc<dyn PriceScoutListClient>,
    ) -> Self {
        Self {
            chat_id,
            client,
            list_client,
        }
    }

    /// Run one `getUpdates` long-poll loop, handling every update as it
    /// arrives, forever. Mirrors `SessionQaBridge::run_outbound` exactly —
    /// every failure path is logged and the loop continues, no failure ever
    /// terminates it, and this is the `pricescout` token's ONLY
    /// `getUpdates` consumer (its own dedicated poller, never a second
    /// consumer of `telegram` or `codesessions`).
    pub async fn run_outbound(&self) {
        let mut cursor: Option<String> = None;
        loop {
            let mut query = Vec::new();
            if let Some(offset) = &cursor {
                query.push(("offset".to_string(), offset.clone()));
            }
            query.push((
                "timeout".to_string(),
                Self::GETUPDATES_TIMEOUT_SECS.to_string(),
            ));

            let raw = match self.client.get_updates(&query).await {
                Ok(raw) => raw,
                Err(NotifyError::RateLimited { retry_after_secs }) => {
                    tracing::debug!(
                        retry_after_secs,
                        "pricescout: getUpdates rate limited; backing off"
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(retry_after_secs)).await;
                    continue;
                }
                Err(err) => {
                    tracing::warn!(error = %err, "pricescout: getUpdates failed; retrying");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
            };

            let Some(updates) = raw.get("result").and_then(Value::as_array) else {
                tracing::warn!("pricescout: getUpdates response missing result array");
                continue;
            };

            for update in updates {
                if let Some(update_id) = update.get("update_id").and_then(Value::as_i64) {
                    cursor = Some((update_id + 1).to_string());
                }
                self.handle_update(update).await;
            }
        }
    }

    /// Dispatch one raw Telegram update: only plain-text messages carry a
    /// `/shop` command on this bot — anything else (callback queries,
    /// edited messages, unrelated update types) is silently ignored, since
    /// this loop has no inline-keyboard or callback surface at all.
    async fn handle_update(&self, update: &Value) {
        if let Some(message) = update.get("message") {
            self.handle_message(message).await;
        }
    }

    async fn handle_message(&self, message: &Value) {
        let Some(text) = message.get("text").and_then(Value::as_str) else {
            return;
        };
        let Some(chat_id) = message
            .get("chat")
            .and_then(|c| c.get("id"))
            .map(|id| id.to_string())
        else {
            return;
        };

        // Chat-id pin BEFORE anything else — a message from a chat other
        // than the configured one is rejected and performs no action.
        // Never log `text`: an unauthorized sender's message content has no
        // business in the log even at warn level.
        if !is_family_chat(&chat_id, &self.chat_id) {
            tracing::warn!(
                chat_id = %chat_id,
                "pricescout: message from non-configured chat id; rejecting"
            );
            return;
        }

        match route_pricescout_message(text) {
            PricescoutRoute::Shop { queries } => self.handle_shop(queries).await,
            // Every built-in the operator's router recognizes, every
            // operator `[telegram_commands]` entry, and plain text are all
            // refused identically and silently — this bot has exactly one
            // command, and the family's allow-list must never grow an
            // operator-facing reply surface even in its refusal text.
            PricescoutRoute::Refused { .. } => {}
        }
    }

    /// Handle a parsed `/shop`: submit the built list body to price-scout
    /// WITHOUT waiting on its response — `POST /api/lists` runs the whole
    /// batch synchronously and can take minutes for a family-sized list —
    /// then send exactly one immediate acknowledgement.
    async fn handle_shop(&self, queries: Vec<String>) {
        let body = build_shop_list_body(&queries, None);
        let list_client = Arc::clone(&self.list_client);
        // Detached: this task's completion is never awaited here, which is
        // what makes the acknowledgement below arrive without waiting on
        // price-scout's batch. Failure is logged only — PS.9.D's
        // `notify_list_ready` seam is what tells the family the batch
        // finished, not this loop.
        tokio::spawn(async move {
            if let Err(err) = list_client.submit_list(body).await {
                tracing::warn!(error = %err, "pricescout: POST /api/lists failed");
            }
        });

        let ack_body = json!({
            "chat_id": self.chat_id,
            "text": shop_ack_text(&queries),
        });
        if let Err(err) = self.client.send_message(ack_body).await {
            tracing::warn!(error = %err, "pricescout: acknowledgement sendMessage failed");
        }
    }
}

#[cfg(test)]
mod tests;
