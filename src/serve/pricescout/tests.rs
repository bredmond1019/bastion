//! Tests for the family `pricescout` bot's pure core
//! (`BA.ticket.pricescout-telegram-bot` task 2).
//!
//! All pure — no I/O, no network call, no bot token needed anywhere in this
//! file.
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
