//! Tests for the family `pricescout` bot — the pure core (task 2) and the
//! I/O shell's dispatch logic against injected seams (task 3).
//!
//! Everything in this file is hermetic: no real network call, no bot
//! token, no live price-scout instance anywhere. The I/O-shell section
//! (below the `--- I/O shell ---` marker) stubs `QaTelegramClient` and
//! `PriceScoutListClient` the same way `session_qa::tests` stubs its own.
use super::*;

// ── PRICESCOUT_ALLOWED_COMMANDS: family-safe, not the operator's ──────────

#[test]
fn allow_list_contains_shop() {
    assert!(PRICESCOUT_ALLOWED_COMMANDS.contains(&"shop"));
}

#[test]
fn allow_list_excludes_operator_facing_commands() {
    // The operator's router's always-on built-ins.
    for operator_builtin in ["status", "lanes", "attention", "help", "commands"] {
        assert!(
            !PRICESCOUT_ALLOWED_COMMANDS.contains(&operator_builtin),
            "operator-facing built-in {operator_builtin:?} must not be in the family allow-list"
        );
    }
}

#[test]
fn allow_list_has_exactly_one_entry() {
    // Pinned explicitly: today it is `/shop` and nothing else. A second
    // family command would grow this deliberately, not by accident.
    assert_eq!(PRICESCOUT_ALLOWED_COMMANDS.len(), 1);
}

// ── route_pricescout_message: only /shop resolves ──────────────────────────

#[test]
fn shop_command_routes_to_shop() {
    let route = route_pricescout_message("/shop milk, eggs, bread");
    assert_eq!(
        route,
        PricescoutRoute::Shop {
            queries: vec!["milk".to_string(), "eggs".to_string(), "bread".to_string()],
        }
    );
}

#[test]
fn operator_builtin_status_is_refused() {
    // The mechanical proof that audience separation is real: `/status`
    // resolves unconditionally on the OPERATOR's router (built-ins are
    // never overridable by an allow-list there), but on the family's own
    // dispatch it must come back refused, exactly like an unrecognised
    // command.
    let route = route_pricescout_message("/status");
    assert_eq!(
        route,
        PricescoutRoute::Refused {
            name: Some("status".to_string())
        }
    );
}

#[test]
fn operator_builtin_lanes_is_refused() {
    let route = route_pricescout_message("/lanes");
    assert_eq!(
        route,
        PricescoutRoute::Refused {
            name: Some("lanes".to_string())
        }
    );
}

#[test]
fn operator_builtin_attention_is_refused() {
    let route = route_pricescout_message("/attention");
    assert_eq!(
        route,
        PricescoutRoute::Refused {
            name: Some("attention".to_string())
        }
    );
}

#[test]
fn operator_builtin_help_is_refused() {
    let route = route_pricescout_message("/help");
    assert_eq!(
        route,
        PricescoutRoute::Refused {
            name: Some("help".to_string())
        }
    );
}

#[test]
fn arbitrary_operator_allow_list_command_is_refused() {
    // Stand-in for an entry that would live in the operator's
    // `[telegram_commands]` TOML table (e.g. a `RESEARCH_AGENT` trigger).
    // This dispatch never even looks at that table, so any such name is
    // refused the same way an unknown command is.
    let route = route_pricescout_message("/research some company");
    assert_eq!(
        route,
        PricescoutRoute::Refused {
            name: Some("research".to_string())
        }
    );
}

#[test]
fn plain_text_is_refused_with_no_name() {
    let route = route_pricescout_message("just chatting, not a command");
    assert_eq!(route, PricescoutRoute::Refused { name: None });
}

#[test]
fn unknown_command_is_refused() {
    let route = route_pricescout_message("/nonsense");
    assert_eq!(
        route,
        PricescoutRoute::Refused {
            name: Some("nonsense".to_string())
        }
    );
}

// ── parse_shop_items ─────────────────────────────────────────────────────

#[test]
fn shop_items_comma_separated() {
    assert_eq!(
        parse_shop_items("a, b, c"),
        vec!["a".to_string(), "b".to_string(), "c".to_string()]
    );
}

#[test]
fn shop_items_newline_separated() {
    assert_eq!(
        parse_shop_items("milk\neggs\nbread"),
        vec!["milk".to_string(), "eggs".to_string(), "bread".to_string()]
    );
}

#[test]
fn shop_items_trims_entries() {
    assert_eq!(
        parse_shop_items("  milk  ,   eggs   "),
        vec!["milk".to_string(), "eggs".to_string()]
    );
}

#[test]
fn shop_items_drops_blank_and_whitespace_only_entries() {
    assert_eq!(
        parse_shop_items("milk, , eggs,   ,bread,"),
        vec!["milk".to_string(), "eggs".to_string(), "bread".to_string()]
    );
}

#[test]
fn shop_items_all_blank_yields_empty_list() {
    assert_eq!(parse_shop_items(" , , \n , "), Vec::<String>::new());
}

#[test]
fn shop_items_empty_rest_yields_empty_list() {
    assert_eq!(parse_shop_items(""), Vec::<String>::new());
}

#[test]
fn shop_command_with_only_blank_items_routes_to_empty_shop() {
    let route = route_pricescout_message("/shop , ,  ");
    assert_eq!(
        route,
        PricescoutRoute::Shop {
            queries: Vec::new()
        }
    );
}

// ── build_shop_list_body ─────────────────────────────────────────────────

#[test]
fn shop_body_carries_telegram_source() {
    let body = build_shop_list_body(&["milk".to_string()], None);
    assert_eq!(body["source"], "telegram");
}

#[test]
fn shop_body_carries_mercado_livre_source_adapter() {
    let body = build_shop_list_body(&["milk".to_string()], None);
    assert_eq!(body["sources"], serde_json::json!(["mercado_livre"]));
}

#[test]
fn shop_body_carries_pages_one() {
    let body = build_shop_list_body(&["milk".to_string()], None);
    assert_eq!(body["pages"], 1);
}

#[test]
fn shop_body_carries_queries_verbatim() {
    let queries = vec!["milk".to_string(), "eggs".to_string()];
    let body = build_shop_list_body(&queries, None);
    assert_eq!(body["queries"], serde_json::json!(["milk", "eggs"]));
}

#[test]
fn shop_body_omits_name_when_none() {
    let body = build_shop_list_body(&["milk".to_string()], None);
    assert!(body.get("name").is_none());
}

#[test]
fn shop_body_includes_name_when_given() {
    let body = build_shop_list_body(&["milk".to_string()], Some("weekly run"));
    assert_eq!(body["name"], "weekly run");
}

// ── is_family_chat ───────────────────────────────────────────────────────

#[test]
fn matching_chat_id_is_authorized() {
    assert!(is_family_chat("12345", "12345"));
}

#[test]
fn wrong_chat_id_is_rejected() {
    assert!(!is_family_chat("99999", "12345"));
}

// ── shop_ack_text ────────────────────────────────────────────────────────

#[test]
fn ack_text_singular_for_one_item() {
    let text = shop_ack_text(&["milk".to_string()]);
    assert!(text.contains('1'));
    assert!(!text.contains("items"));
}

#[test]
fn ack_text_plural_for_multiple_items() {
    let text = shop_ack_text(&["milk".to_string(), "eggs".to_string(), "bread".to_string()]);
    assert!(text.contains('3'));
    assert!(text.contains("items"));
}

#[test]
fn ack_text_for_empty_list_asks_for_items() {
    let text = shop_ack_text(&[]);
    assert!(text.contains("No items"));
}

// ─────────────────────────── I/O shell (task 3) ────────────────────────────
//
// Fakes for both injected seams, plus dispatch-logic tests over
// `PricescoutBridge` — no network, no bot token, no live price-scout call.

mod io_shell {
    use std::collections::VecDeque;
    use std::sync::Mutex as StdMutex;

    use serde_json::json;
    use tokio::sync::Notify;

    use super::super::*;
    use crate::serve::notify::NotifyError;
    use crate::serve::session_qa::QaTelegramClient;

    // ── Fake Telegram client ────────────────────────────────────────────

    #[derive(Debug, Clone)]
    enum Call {
        SendMessage(Value),
        GetUpdates,
    }

    /// In-memory [`QaTelegramClient`]: records every call, no network
    /// anywhere. `answer_callback_query`/`edit_message_text` are no-ops —
    /// this bot never calls either.
    #[derive(Default)]
    struct FakeQaTelegramClient {
        calls: StdMutex<Vec<Call>>,
        get_updates_queue: StdMutex<VecDeque<Result<Value, NotifyError>>>,
    }

    impl FakeQaTelegramClient {
        fn new() -> Self {
            Self::default()
        }

        fn calls(&self) -> Vec<Call> {
            self.calls.lock().expect("calls mutex poisoned").clone()
        }

        fn send_message_calls(&self) -> Vec<Value> {
            self.calls()
                .into_iter()
                .filter_map(|c| match c {
                    Call::SendMessage(body) => Some(body),
                    Call::GetUpdates => None,
                })
                .collect()
        }
    }

    #[async_trait]
    impl QaTelegramClient for FakeQaTelegramClient {
        async fn send_message(&self, body: Value) -> Result<Value, NotifyError> {
            self.calls
                .lock()
                .expect("calls mutex poisoned")
                .push(Call::SendMessage(body));
            Ok(json!({"ok": true, "result": {"message_id": 1}}))
        }

        async fn answer_callback_query(&self, _body: Value) -> Result<(), NotifyError> {
            Ok(())
        }

        async fn edit_message_text(&self, _body: Value) -> Result<(), NotifyError> {
            Ok(())
        }

        async fn get_updates(&self, _query: &[(String, String)]) -> Result<Value, NotifyError> {
            self.calls
                .lock()
                .expect("calls mutex poisoned")
                .push(Call::GetUpdates);
            self.get_updates_queue
                .lock()
                .expect("queue mutex poisoned")
                .pop_front()
                .unwrap_or_else(|| Ok(json!({"ok": true, "result": []})))
        }
    }

    // ── Fake price-scout list client ────────────────────────────────────

    /// In-memory [`PriceScoutListClient`]: records every submitted body. If
    /// `block_forever` is set, `submit_list` never resolves — used to prove
    /// the acknowledgement does not wait on it.
    #[derive(Default)]
    struct FakeListClient {
        calls: StdMutex<Vec<Value>>,
        block_forever: bool,
    }

    impl FakeListClient {
        fn new() -> Self {
            Self::default()
        }

        fn blocking() -> Self {
            Self {
                calls: StdMutex::new(Vec::new()),
                block_forever: true,
            }
        }

        fn calls(&self) -> Vec<Value> {
            self.calls.lock().expect("calls mutex poisoned").clone()
        }
    }

    #[async_trait]
    impl PriceScoutListClient for FakeListClient {
        async fn submit_list(&self, body: Value) -> Result<(), String> {
            self.calls.lock().expect("calls mutex poisoned").push(body);
            if self.block_forever {
                // Never notified — awaits forever, simulating price-scout's
                // multi-minute synchronous batch.
                Notify::new().notified().await;
            }
            Ok(())
        }
    }

    fn message_update(chat_id: i64, text: &str) -> Value {
        json!({
            "update_id": 1,
            "message": {
                "message_id": 7,
                "chat": {"id": chat_id},
                "text": text,
            }
        })
    }

    const FAMILY_CHAT: &str = "42";

    // ── /shop dispatch ───────────────────────────────────────────────────

    #[tokio::test]
    async fn shop_submits_list_and_acknowledges() {
        let client = Arc::new(FakeQaTelegramClient::new());
        let list_client = Arc::new(FakeListClient::new());
        let bridge = PricescoutBridge::with_seams(
            FAMILY_CHAT.to_string(),
            client.clone(),
            list_client.clone(),
        );

        let update = message_update(42, "/shop milk, eggs, bread");
        bridge.handle_update(&update).await;
        // The spawned submit task runs on the same tokio runtime as this
        // test — yield once so it gets scheduled before asserting on it.
        tokio::task::yield_now().await;

        let submitted = list_client.calls();
        assert_eq!(submitted.len(), 1);
        assert_eq!(submitted[0]["source"], json!("telegram"));
        assert_eq!(
            submitted[0]["queries"],
            json!(["milk".to_string(), "eggs".to_string(), "bread".to_string()])
        );

        let sent = client.send_message_calls();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["chat_id"], json!(FAMILY_CHAT));
        assert!(sent[0]["text"].as_str().unwrap().contains('3'));
    }

    /// Load-bearing: the acknowledgement must not wait on `submit_list`'s
    /// response. `FakeListClient::blocking()` never resolves; if
    /// `handle_update` awaited it before acknowledging, this test would
    /// hang and the timeout below would fail it.
    #[tokio::test]
    async fn shop_acknowledges_without_waiting_on_submit() {
        let client = Arc::new(FakeQaTelegramClient::new());
        let list_client = Arc::new(FakeListClient::blocking());
        let bridge = PricescoutBridge::with_seams(
            FAMILY_CHAT.to_string(),
            client.clone(),
            list_client.clone(),
        );

        let update = message_update(42, "/shop milk");
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            bridge.handle_update(&update),
        )
        .await;
        assert!(
            result.is_ok(),
            "handle_update must not block on submit_list's response"
        );

        assert_eq!(
            client.send_message_calls().len(),
            1,
            "exactly one acknowledgement must be sent"
        );
    }

    #[tokio::test]
    async fn shop_attaches_no_credential_to_the_submitted_body() {
        let client = Arc::new(FakeQaTelegramClient::new());
        let list_client = Arc::new(FakeListClient::new());
        let bridge =
            PricescoutBridge::with_seams(FAMILY_CHAT.to_string(), client, list_client.clone());

        let update = message_update(42, "/shop milk");
        bridge.handle_update(&update).await;
        tokio::task::yield_now().await;

        let submitted = list_client.calls();
        assert_eq!(submitted.len(), 1);
        assert!(
            submitted[0].get("api_key").is_none()
                && submitted[0].get("X-API-Key").is_none()
                && submitted[0].get("token").is_none(),
            "no credential field may appear in the submitted body: {:?}",
            submitted[0]
        );
    }

    // ── Authorization: wrong chat id ─────────────────────────────────────

    #[tokio::test]
    async fn wrong_chat_id_is_rejected_and_performs_no_action() {
        let client = Arc::new(FakeQaTelegramClient::new());
        let list_client = Arc::new(FakeListClient::new());
        let bridge = PricescoutBridge::with_seams(
            FAMILY_CHAT.to_string(),
            client.clone(),
            list_client.clone(),
        );

        let update = message_update(99_999, "/shop milk");
        bridge.handle_update(&update).await;
        tokio::task::yield_now().await;

        assert!(list_client.calls().is_empty());
        assert!(client.send_message_calls().is_empty());
    }

    // ── Refused commands: no reply, no dispatch ─────────────────────────

    #[tokio::test]
    async fn operator_command_is_refused_with_no_reply() {
        let client = Arc::new(FakeQaTelegramClient::new());
        let list_client = Arc::new(FakeListClient::new());
        let bridge = PricescoutBridge::with_seams(
            FAMILY_CHAT.to_string(),
            client.clone(),
            list_client.clone(),
        );

        let update = message_update(42, "/status");
        bridge.handle_update(&update).await;
        tokio::task::yield_now().await;

        assert!(list_client.calls().is_empty());
        assert!(client.send_message_calls().is_empty());
    }

    // ── Non-message updates are ignored ─────────────────────────────────

    #[tokio::test]
    async fn non_message_update_is_ignored() {
        let client = Arc::new(FakeQaTelegramClient::new());
        let list_client = Arc::new(FakeListClient::new());
        let bridge = PricescoutBridge::with_seams(
            FAMILY_CHAT.to_string(),
            client.clone(),
            list_client.clone(),
        );

        let update = json!({"update_id": 1, "callback_query": {"id": "cbq1"}});
        bridge.handle_update(&update).await;

        assert!(client.send_message_calls().is_empty());
        assert!(list_client.calls().is_empty());
    }
}
