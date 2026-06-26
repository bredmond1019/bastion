//! Minimal WebSocket echo actor for `bastion serve`.
//!
//! [`EchoActor`] accepts a WS upgrade on the `/ws` route and echoes every
//! received text frame back to the client.  This is the live socket the Flutter
//! `bastion-ui` foundation needs before the real hub (Block C) lands.
//!
//! # Design
//! - Text frames are echoed back **unchanged** (raw string pass-through).
//! - Binary frames are silently ignored at v0.
//! - Ping frames are handled automatically by actix-web-actors; the actor
//!   does not need to respond to them explicitly.
//! - The pure frame-text helper [`echo_text`] is unit-tested; the actor
//!   I/O shell is smoke-tested (see `## Notes` in the task spec).
//!
//! # Auth policy
//! The `/ws` route is protected by [`crate::serve::auth::BearerAuthMiddleware`]
//! at the scope level; see `docs/serve-api.md` v0.

use actix::{Actor, StreamHandler};
use actix_web::{HttpRequest, HttpResponse, web};
use actix_web_actors::ws;
use anyhow::Result;

// ── Pure helpers (unit-tested) ─────────────────────────────────────────────────

/// Build the echo text to send back for a received text frame.
///
/// At v0 this is a direct pass-through of the input; this function exists as
/// a seam so tests can assert the echo logic without spinning up a WS connection.
pub fn echo_text(received: &str) -> String {
    received.to_owned()
}

// ── WebSocket actor ────────────────────────────────────────────────────────────

/// Actix WS actor that echoes every received text frame back to the client.
pub struct EchoActor;

impl Actor for EchoActor {
    type Context = ws::WebsocketContext<Self>;
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for EchoActor {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Text(text)) => {
                let reply = echo_text(&text);
                ctx.text(reply);
            }
            Ok(ws::Message::Binary(_)) => {
                // Binary frames are silently dropped at v0.
            }
            Ok(ws::Message::Ping(bytes)) => {
                // Respond to pings to keep the connection alive.
                ctx.pong(&bytes);
            }
            Ok(ws::Message::Close(reason)) => {
                ctx.close(reason);
            }
            Ok(ws::Message::Pong(_)) | Ok(ws::Message::Nop) | Ok(ws::Message::Continuation(_)) => {
                // No action needed.
            }
            Err(_) => {
                // Protocol error — close the connection.
                ctx.close(None);
            }
        }
    }
}

// ── Route handler ──────────────────────────────────────────────────────────────

/// HTTP handler that upgrades the connection to a WebSocket and starts the echo actor.
///
/// Mount this via [`ws_route`] at `/ws`; the bearer middleware wrapping the
/// parent scope enforces auth before this handler is called.
pub async fn ws_handler(
    req: HttpRequest,
    stream: web::Payload,
) -> Result<HttpResponse, actix_web::Error> {
    ws::start(EchoActor, &req, stream)
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── echo_text — pure helper ────────────────────────────────────────────

    #[test]
    fn echo_text_returns_same_string() {
        assert_eq!(echo_text("hello"), "hello");
    }

    #[test]
    fn echo_text_returns_empty_string() {
        assert_eq!(echo_text(""), "");
    }

    #[test]
    fn echo_text_preserves_unicode() {
        let input = "héllo wörld 🦀";
        assert_eq!(echo_text(input), input);
    }

    #[test]
    fn echo_text_preserves_json_payload() {
        let input = r#"{"kind":"echo","payload":{"text":"hi"}}"#;
        assert_eq!(echo_text(input), input);
    }

    #[test]
    fn echo_text_preserves_whitespace() {
        let input = "  text with spaces  \n";
        assert_eq!(echo_text(input), input);
    }

    #[test]
    fn echo_text_preserves_long_string() {
        let input = "a".repeat(65536);
        assert_eq!(echo_text(&input), input);
    }
}
