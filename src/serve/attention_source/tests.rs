//! Task 1: pure parsing tests for `mev attention-queue --notify-only`
//! payloads, against the checked-in fixture captured 2026-09-01
//! (`planning/BA.21.D/attention-queue-notify-only.fixture.json`, 2 items
//! from a live 548-item board).

use super::*;

const FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/planning/BA.21.D/attention-queue-notify-only.fixture.json"
));

#[test]
fn fixture_parses_to_exactly_two_items() {
    let items = parse_attention_queue_payload(FIXTURE).expect("fixture should parse");
    assert_eq!(items.len(), 2);
}

#[test]
fn fixture_first_item_fields_match_the_captured_payload() {
    let items = parse_attention_queue_payload(FIXTURE).expect("fixture should parse");
    let first = &items[0];

    assert_eq!(
        first.item_id,
        "14e25af6d375d891b8ae1cfd0faa53571b8fa5e9bb9e38494c7274cab15965b3"
    );
    assert_eq!(
        first.gate_id,
        "attention:14e25af6d375d891b8ae1cfd0faa53571b8fa5e9bb9e38494c7274cab15965b3"
    );
    assert!(first.rendered_summary.starts_with(
        "[engine-rs] HOT kind=defect slug=budget-max-cost-usd-cannot-fire-on-any-sdlc-run"
    ));
    assert_eq!(
        first.digest,
        "d298430f49717b35c37979c3c8e4db78d113f7c2f0b0d342d11e451ba214655b"
    );
    assert_eq!(first.effective_priority, 0);
    assert_eq!(first.lane, "hot");
    assert_eq!(first.repo, "engine-rs");
    assert_eq!(first.source, "attention-board");

    assert_eq!(first.options.len(), 3);
    assert_eq!(first.options[0].key, "promote");
    assert_eq!(first.options[0].label, "Promote");
    assert_eq!(first.options[1].key, "snooze");
    assert_eq!(first.options[1].label, "Snooze");
    assert_eq!(first.options[2].key, "session");
    assert_eq!(first.options[2].label, "Open session");
}

#[test]
fn fixture_second_item_fields_match_the_captured_payload() {
    let items = parse_attention_queue_payload(FIXTURE).expect("fixture should parse");
    let second = &items[1];

    assert_eq!(
        second.item_id,
        "ec7b3433bbc43d24d4c4cbcdeda73a3b2e2321b003422d0de1b71d1174cb2358"
    );
    assert_eq!(
        second.gate_id,
        "attention:ec7b3433bbc43d24d4c4cbcdeda73a3b2e2321b003422d0de1b71d1174cb2358"
    );
    assert!(
        second
            .rendered_summary
            .starts_with("[bastion-web] BLOCKING kind=deferred")
    );
    assert_eq!(
        second.digest,
        "8af47ccea144b5352a8b434dc4aaff3de5be68922efc1a7cf2f69afb5184df38"
    );
    assert_eq!(second.effective_priority, 3);
    assert_eq!(second.lane, "blocking");
    assert_eq!(second.repo, "bastion-web");
    assert_eq!(second.source, "attention-board");
    assert_eq!(second.options.len(), 3);
}

#[test]
fn round_trip_serializes_back_to_an_equivalent_value() {
    let items = parse_attention_queue_payload(FIXTURE).expect("fixture should parse");
    let json = serde_json::to_string(&items).expect("serialize should succeed");
    let reparsed = parse_attention_queue_payload(&json).expect("re-parse should succeed");
    assert_eq!(items, reparsed);
}

#[test]
fn empty_array_parses_to_an_empty_vec_and_is_not_an_error() {
    // `mev attention-queue --notify-only` prints `[]` and exits 0 when the
    // admitted set is empty — the healthy, common case. Treating it as a
    // failure would alarm on exactly the case this source should stay quiet
    // for.
    let items = parse_attention_queue_payload("[]").expect("empty array is not an error");
    assert!(items.is_empty());
}

#[test]
fn whitespace_only_empty_array_also_parses_cleanly() {
    let items = parse_attention_queue_payload("  [ ]\n").expect("should parse");
    assert!(items.is_empty());
}

#[test]
fn malformed_json_returns_a_typed_error_not_a_panic() {
    let result = parse_attention_queue_payload("not json at all");
    match result {
        Err(ParseError::Malformed(_)) => {}
        Ok(items) => panic!("expected a parse error, got Ok({items:?})"),
    }
}

#[test]
fn missing_required_field_returns_a_typed_error() {
    let json = r#"[{"item_id": "a", "gate_id": "g"}]"#;
    let result = parse_attention_queue_payload(json);
    assert!(matches!(result, Err(ParseError::Malformed(_))));
}

#[test]
fn wrong_shape_top_level_value_returns_a_typed_error() {
    // A single object, not wrapped in an array, is not the shape `mev`
    // emits — must be a typed error, never a panic or a silent single-item
    // vec.
    let json = r#"{"item_id": "a"}"#;
    let result = parse_attention_queue_payload(json);
    assert!(matches!(result, Err(ParseError::Malformed(_))));
}

#[test]
fn parse_error_display_names_the_payload_as_malformed() {
    let err = parse_attention_queue_payload("{{{").unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("malformed attention-queue payload"),
        "unexpected error message: {message}"
    );
}
