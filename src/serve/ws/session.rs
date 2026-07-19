//! Per-connection WebSocket actor for `bastion serve`.
//!
//! [`WsConn`] is created once per WS upgrade.  It:
//! - Registers with the [`Hub`] on `started` and deregisters on `stopping`.
//! - Writes [`ServerFrame`]s delivered by the hub to the underlying socket.
//! - Parses inbound text frames and dispatches to hub messages or tmux I/O.
//!
//! # Pure dispatch seam (unit-tested, Rule 6)
//! [`classify_inbound`] parses a raw JSON text frame into an [`Inbound`] enum
//! without any I/O, keeping the dispatch logic exhaustively testable without a
//! live socket.
//!
//! # I/O shell (smoke-tested, Rule 6)
//! The `StreamHandler` implementation ties [`classify_inbound`] results to hub
//! messages and `web::block`-offloaded tmux calls.  Live behaviour is
//! smoke-tested in spec Task 6.
//!
//! # Keep-alive / close
//! Ping frames are pong'd explicitly (actix-web-actors does NOT auto-pong when
//! a custom `StreamHandler` is registered).  On `Close` or protocol error the
//! actor stops, which triggers `stopping` → `Disconnect` to the hub.

use std::time::{Duration, Instant};

use actix::prelude::*;
use actix::{ActorContext, AsyncContext};
use actix_http::ws::Item;
use actix_web::web;
use actix_web_actors::ws;

use crate::serve::dto::{
    SendKeyPayload, SendPayload, SubscribePayload, Topic, WsFrame, WsFrameKind, parse_topic,
};
use crate::serve::ws::server::{
    ConnId, Connect, Disconnect, Hub, ServerFrame, Subscribe, Unsubscribe,
};
use crate::sessions::tmux;

// ── Pure dispatch helper ──────────────────────────────────────────────────────

/// Outcome of classifying one inbound text frame before any I/O.
///
/// This enum is the seam between the pure parsing phase (unit-tested) and the
/// actor dispatch phase (smoke-tested).
#[derive(Debug, PartialEq)]
pub enum Inbound {
    /// Client subscribed to the given topic.
    Subscribe(Topic),
    /// Client unsubscribed from the given topic.
    Unsubscribe(Topic),
    /// Client wants to send literal keystrokes (+ Enter) to a session.
    Send { session: String, keys: String },
    /// Client wants to send a single named key to a session.
    SendKey { session: String, key: String },
    /// A server→client kind arrived inbound — ignore per protocol.
    Ignore,
    /// Parse error or bad topic string — message is returned for the `Error` frame.
    Invalid(String),
}

/// Classify a raw inbound text frame into an [`Inbound`] variant.
///
/// No I/O is performed; this is a pure parse + dispatch decision.
pub fn classify_inbound(text: &str) -> Inbound {
    let frame: WsFrame = match serde_json::from_str(text) {
        Ok(f) => f,
        Err(e) => return Inbound::Invalid(format!("invalid JSON frame: {e}")),
    };

    match frame.kind {
        WsFrameKind::Subscribe => {
            let p: SubscribePayload = match serde_json::from_value(frame.payload) {
                Ok(v) => v,
                Err(e) => return Inbound::Invalid(format!("invalid subscribe payload: {e}")),
            };
            match parse_topic(&p.topic) {
                Some(topic) => Inbound::Subscribe(topic),
                None => Inbound::Invalid(format!("unknown or malformed topic: {:?}", p.topic)),
            }
        }

        WsFrameKind::Unsubscribe => {
            let p: SubscribePayload = match serde_json::from_value(frame.payload) {
                Ok(v) => v,
                Err(e) => return Inbound::Invalid(format!("invalid unsubscribe payload: {e}")),
            };
            match parse_topic(&p.topic) {
                Some(topic) => Inbound::Unsubscribe(topic),
                None => Inbound::Invalid(format!("unknown or malformed topic: {:?}", p.topic)),
            }
        }

        WsFrameKind::Send => {
            let p: SendPayload = match serde_json::from_value(frame.payload) {
                Ok(v) => v,
                Err(e) => return Inbound::Invalid(format!("invalid send payload: {e}")),
            };
            Inbound::Send {
                session: p.session,
                keys: p.keys,
            }
        }

        WsFrameKind::SendKey => {
            let p: SendKeyPayload = match serde_json::from_value(frame.payload) {
                Ok(v) => v,
                Err(e) => return Inbound::Invalid(format!("invalid send_key payload: {e}")),
            };
            Inbound::SendKey {
                session: p.session,
                key: p.key,
            }
        }

        // Server→client kinds arriving inbound are silently ignored.
        WsFrameKind::Echo
        | WsFrameKind::Sessions
        | WsFrameKind::Pane
        | WsFrameKind::Event
        | WsFrameKind::Error => Inbound::Ignore,
    }
}

// ── Keep-alive constants + pure timeout decision ───────────────────────────────

/// How often the server sends a `Ping` to an idle connection (§9).
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

/// How long a connection may go without client activity before it is reaped (§9).
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

/// Pure decision: has a connection gone silent for longer than `timeout`?
///
/// No I/O — this is the seam unit-tested for the keep-alive contract (Rule 6).
pub fn client_timed_out(elapsed: Duration, timeout: Duration) -> bool {
    elapsed > timeout
}

// ── Per-connection actor ──────────────────────────────────────────────────────

/// Per-connection WebSocket actor.
pub struct WsConn {
    /// Stable id allocated on creation.
    id: ConnId,
    /// Address of the hub actor.
    hub: Addr<Hub>,
    /// Accumulation buffer for fragmented text messages (Continuation frames).
    continuation_buf: Option<Vec<u8>>,
    /// Instant of the last observed client activity (inbound frame of any kind).
    last_seen: Instant,
}

impl WsConn {
    /// Create a new connection actor with a freshly allocated [`ConnId`].
    pub fn new(hub: Addr<Hub>) -> Self {
        Self {
            id: ConnId::next(),
            hub,
            continuation_buf: None,
            last_seen: Instant::now(),
        }
    }

    /// Install the server heartbeat: ping every [`HEARTBEAT_INTERVAL`], and stop
    /// the actor if the client has been silent for longer than [`CLIENT_TIMEOUT`].
    fn heartbeat(&self, ctx: &mut ws::WebsocketContext<Self>) {
        ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
            if client_timed_out(act.last_seen.elapsed(), CLIENT_TIMEOUT) {
                ctx.stop();
                return;
            }
            ctx.ping(b"");
        });
    }
}

impl Actor for WsConn {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        // Register with the hub.
        self.hub.do_send(Connect {
            id: self.id,
            addr: ctx.address().recipient(),
        });
        // Start the keep-alive heartbeat.
        self.heartbeat(ctx);
    }

    fn stopping(&mut self, _ctx: &mut Self::Context) -> Running {
        // Deregister from the hub on any stop (close, error, keep-alive timeout).
        self.hub.do_send(Disconnect { id: self.id });
        Running::Stop
    }
}

/// Hub → client: serialize the frame and write it to the WS socket.
impl Handler<ServerFrame> for WsConn {
    type Result = ();

    fn handle(&mut self, msg: ServerFrame, ctx: &mut Self::Context) {
        if let Ok(txt) = serde_json::to_string(&msg.0) {
            ctx.text(txt);
        }
    }
}

// ── Inbound frame dispatch ────────────────────────────────────────────────────

/// Build an `Error` WsFrame for protocol/parse errors.
fn error_frame(message: &str) -> String {
    let frame = WsFrame {
        kind: WsFrameKind::Error,
        payload: serde_json::json!({ "code": "WS_ERR", "message": message }),
    };
    serde_json::to_string(&frame)
        .unwrap_or_else(|_| r#"{"kind":"error","payload":null}"#.to_owned())
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for WsConn {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Text(text)) => {
                self.last_seen = Instant::now();
                self.dispatch_text(&text, ctx);
            }
            Ok(ws::Message::Binary(_)) => {
                // Binary frames are silently dropped.
            }
            Ok(ws::Message::Ping(bytes)) => {
                self.last_seen = Instant::now();
                // actix-web-actors does NOT auto-pong; must respond explicitly.
                ctx.pong(&bytes);
            }
            Ok(ws::Message::Close(reason)) => {
                ctx.close(reason);
                ctx.stop();
            }
            Ok(ws::Message::Continuation(item)) => {
                match item {
                    Item::FirstText(bytes) => {
                        self.continuation_buf = Some(bytes.to_vec());
                    }
                    Item::FirstBinary(_) => {
                        // Binary continuation unsupported; discard.
                        self.continuation_buf = None;
                    }
                    Item::Continue(bytes) => {
                        if let Some(ref mut buf) = self.continuation_buf {
                            buf.extend_from_slice(&bytes);
                        }
                    }
                    Item::Last(bytes) => {
                        if let Some(mut buf) = self.continuation_buf.take() {
                            buf.extend_from_slice(&bytes);
                            if let Ok(text) = std::str::from_utf8(&buf) {
                                self.last_seen = Instant::now();
                                self.dispatch_text(text, ctx);
                            }
                        }
                    }
                }
            }
            Ok(ws::Message::Pong(_)) => {
                self.last_seen = Instant::now();
            }
            Ok(ws::Message::Nop) => {
                // No action needed.
            }
            Err(_) => {
                // Protocol error — close and stop.
                ctx.close(None);
                ctx.stop();
            }
        }
    }
}

impl WsConn {
    /// Dispatch a (possibly continuation-assembled) text frame.
    fn dispatch_text(&mut self, text: &str, ctx: &mut ws::WebsocketContext<Self>) {
        match classify_inbound(text) {
            Inbound::Subscribe(topic) => {
                self.hub.do_send(Subscribe { id: self.id, topic });
            }
            Inbound::Unsubscribe(topic) => {
                self.hub.do_send(Unsubscribe { id: self.id, topic });
            }
            Inbound::Send { session, keys } => {
                // Offload the blocking tmux call; report errors back to the client.
                let fut = web::block(move || tmux::send_keys(&session, &keys))
                    .into_actor(self)
                    .then(|result, _act, ctx| {
                        if let Err(e) = result {
                            ctx.text(error_frame(&format!("send failed: {e}")));
                        }
                        actix::fut::ready(())
                    });
                ctx.spawn(fut);
            }
            Inbound::SendKey { session, key } => {
                let fut = web::block(move || tmux::send_named_key(&session, &key))
                    .into_actor(self)
                    .then(|result, _act, ctx| {
                        if let Err(e) = result {
                            ctx.text(error_frame(&format!("send_key failed: {e}")));
                        }
                        actix::fut::ready(())
                    });
                ctx.spawn(fut);
            }
            Inbound::Ignore => {
                // Server→client kind received inbound — documented no-op.
            }
            Inbound::Invalid(msg) => {
                ctx.text(error_frame(&msg));
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Helper: build a JSON text frame string from kind + payload.
    fn frame(kind: &str, payload: serde_json::Value) -> String {
        json!({ "kind": kind, "payload": payload }).to_string()
    }

    // ── classify_inbound — subscribe ───────────────────────────────────────

    #[test]
    fn classify_subscribe_sessions() {
        let text = frame("subscribe", json!({ "topic": "sessions" }));
        assert_eq!(classify_inbound(&text), Inbound::Subscribe(Topic::Sessions));
    }

    #[test]
    fn classify_subscribe_pane_with_name() {
        let text = frame("subscribe", json!({ "topic": "pane:work" }));
        assert_eq!(
            classify_inbound(&text),
            Inbound::Subscribe(Topic::Pane("work".to_owned()))
        );
    }

    #[test]
    fn classify_subscribe_pane_empty_name_is_invalid() {
        let text = frame("subscribe", json!({ "topic": "pane:" }));
        assert!(matches!(classify_inbound(&text), Inbound::Invalid(_)));
    }

    #[test]
    fn classify_subscribe_unknown_topic_is_invalid() {
        let text = frame("subscribe", json!({ "topic": "unknown" }));
        assert!(matches!(classify_inbound(&text), Inbound::Invalid(_)));
    }

    #[test]
    fn classify_subscribe_missing_topic_field_is_invalid() {
        let text = frame("subscribe", json!({}));
        assert!(matches!(classify_inbound(&text), Inbound::Invalid(_)));
    }

    // ── classify_inbound — unsubscribe ────────────────────────────────────

    #[test]
    fn classify_unsubscribe_sessions() {
        let text = frame("unsubscribe", json!({ "topic": "sessions" }));
        assert_eq!(
            classify_inbound(&text),
            Inbound::Unsubscribe(Topic::Sessions)
        );
    }

    #[test]
    fn classify_unsubscribe_pane() {
        let text = frame("unsubscribe", json!({ "topic": "pane:work" }));
        assert_eq!(
            classify_inbound(&text),
            Inbound::Unsubscribe(Topic::Pane("work".to_owned()))
        );
    }

    // ── classify_inbound — send ────────────────────────────────────────────

    #[test]
    fn classify_send_frame() {
        let text = frame("send", json!({ "session": "main", "keys": "cargo test" }));
        assert_eq!(
            classify_inbound(&text),
            Inbound::Send {
                session: "main".to_owned(),
                keys: "cargo test".to_owned(),
            }
        );
    }

    #[test]
    fn classify_send_missing_session_is_invalid() {
        let text = frame("send", json!({ "keys": "ls" }));
        assert!(matches!(classify_inbound(&text), Inbound::Invalid(_)));
    }

    #[test]
    fn classify_send_missing_keys_is_invalid() {
        let text = frame("send", json!({ "session": "main" }));
        assert!(matches!(classify_inbound(&text), Inbound::Invalid(_)));
    }

    // ── classify_inbound — send_key ───────────────────────────────────────

    #[test]
    fn classify_send_key_frame() {
        let text = frame("send_key", json!({ "session": "main", "key": "Escape" }));
        assert_eq!(
            classify_inbound(&text),
            Inbound::SendKey {
                session: "main".to_owned(),
                key: "Escape".to_owned(),
            }
        );
    }

    #[test]
    fn classify_send_key_missing_key_is_invalid() {
        let text = frame("send_key", json!({ "session": "main" }));
        assert!(matches!(classify_inbound(&text), Inbound::Invalid(_)));
    }

    // ── classify_inbound — server→client kinds are ignored ────────────────

    #[test]
    fn classify_pane_kind_inbound_is_ignored() {
        let text = frame("pane", json!({}));
        assert_eq!(classify_inbound(&text), Inbound::Ignore);
    }

    #[test]
    fn classify_sessions_kind_inbound_is_ignored() {
        let text = frame("sessions", json!({}));
        assert_eq!(classify_inbound(&text), Inbound::Ignore);
    }

    #[test]
    fn classify_event_kind_inbound_is_ignored() {
        let text = frame("event", json!({}));
        assert_eq!(classify_inbound(&text), Inbound::Ignore);
    }

    #[test]
    fn classify_echo_kind_inbound_is_ignored() {
        let text = frame("echo", json!({ "text": "hi" }));
        assert_eq!(classify_inbound(&text), Inbound::Ignore);
    }

    #[test]
    fn classify_error_kind_inbound_is_ignored() {
        let text = frame("error", json!({ "code": "E1", "message": "oops" }));
        assert_eq!(classify_inbound(&text), Inbound::Ignore);
    }

    // ── classify_inbound — malformed JSON ─────────────────────────────────

    #[test]
    fn classify_malformed_json_is_invalid() {
        assert!(matches!(classify_inbound("not json"), Inbound::Invalid(_)));
    }

    #[test]
    fn classify_empty_string_is_invalid() {
        assert!(matches!(classify_inbound(""), Inbound::Invalid(_)));
    }

    #[test]
    fn classify_missing_kind_field_is_invalid() {
        let text = r#"{"payload":{"text":"hi"}}"#;
        assert!(matches!(classify_inbound(text), Inbound::Invalid(_)));
    }

    // ── client_timed_out — pure keep-alive decision ────────────────────────

    #[test]
    fn client_timed_out_below_threshold_is_false() {
        assert!(!client_timed_out(
            Duration::from_secs(9),
            Duration::from_secs(10)
        ));
    }

    #[test]
    fn client_timed_out_at_threshold_is_false() {
        // Strictly-greater-than semantics: exactly at the timeout is not yet timed out.
        assert!(!client_timed_out(
            Duration::from_secs(10),
            Duration::from_secs(10)
        ));
    }

    #[test]
    fn client_timed_out_above_threshold_is_true() {
        assert!(client_timed_out(
            Duration::from_secs(11),
            Duration::from_secs(10)
        ));
    }

    #[test]
    fn client_timed_out_zero_elapsed_is_false() {
        assert!(!client_timed_out(
            Duration::from_secs(0),
            Duration::from_secs(10)
        ));
    }

    #[test]
    fn heartbeat_and_timeout_constants_match_spec() {
        assert_eq!(HEARTBEAT_INTERVAL, Duration::from_secs(5));
        assert_eq!(CLIENT_TIMEOUT, Duration::from_secs(10));
    }
}
