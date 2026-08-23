//! The pure core of `bastion notify send|ask` (`BA.ticket.notify-operator-cli`
//! task 3).
//!
//! This module holds only pure logic and its unit tests — no HTTP, no clap
//! wiring, no lock. Per `CLAUDE.md` standing rule 6, this is where
//! essentially all of this block's coverage lives; the thin I/O shell that
//! calls into these functions is wired up by a later task.
//!
//! **Reuse discipline (non-negotiable for this file):** everything here
//! delegates to the already-tested protocol code in
//! `crate::serve::notify::telegram` and `engine_core::operator`. This file
//! contains no `callback_data` encoder, no `getUpdates` offset arithmetic,
//! and no digest comparison of its own — [`decide_batch`] calls the
//! existing [`resolve_response`](crate::serve::notify::telegram::resolve_response)
//! rather than re-deriving its verdict.

use engine_core::operator::{OperatorResponse, OperatorResponseOption, ValidatedOperatorPayload};

use crate::serve::notify::telegram::resolve_response;

// ── Option parsing (`--option key:Label`) ───────────────────────────────

/// Parse one `--option key:Label` argument into an
/// [`OperatorResponseOption`].
///
/// Splits on the **first** `:` only, so a label may itself contain `:`
/// (e.g. `approve:Approve: ship it`). An empty key or an empty label is
/// rejected with a distinct message so a caller can tell which half was
/// wrong without re-parsing the raw string themselves.
pub fn parse_option(raw: &str) -> Result<OperatorResponseOption, String> {
    let Some((key, label)) = raw.split_once(':') else {
        return Err(format!(
            "--option '{raw}' is missing a ':' separator; expected key:Label"
        ));
    };

    if key.is_empty() {
        return Err(format!("--option '{raw}' has an empty key"));
    }
    if label.is_empty() {
        return Err(format!("--option '{raw}' has an empty label"));
    }

    Ok(OperatorResponseOption::new(key, label))
}

/// Parse every `--option` argument, in order, additionally rejecting a
/// duplicate key — two buttons sharing a key make a tap ambiguous, and
/// `resolve_response` would happily accept either, so this must be caught
/// before any option ever reaches the transport.
pub fn parse_options(raw: &[String]) -> Result<Vec<OperatorResponseOption>, String> {
    let mut options = Vec::with_capacity(raw.len());
    let mut seen_keys: Vec<String> = Vec::with_capacity(raw.len());

    for one in raw {
        let option = parse_option(one)?;
        if seen_keys.iter().any(|k| k == &option.key) {
            return Err(format!("duplicate --option key '{}'", option.key));
        }
        seen_keys.push(option.key.clone());
        options.push(option);
    }

    Ok(options)
}

// ── The ask outcome contract (stdout JSON + exit code) ──────────────────

/// The terminal outcome of `bastion notify ask`. This is the CLI's stdout
/// contract: exactly one of these is printed as one flat JSON object, and
/// every variant maps to a distinct, total exit code via [`Self::exit_code`].
///
/// Exit code 1 is reserved for unconfigured-bot / usage errors and is
/// deliberately not a variant here — those are reported before an
/// `AskOutcome` can even be constructed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskOutcome {
    /// A tap resolved to the expected gate and digest.
    Answered {
        gate_id: String,
        option_key: String,
        decided_at: chrono::DateTime<chrono::Utc>,
    },
    /// No resolving tap arrived before the ask's timeout elapsed.
    Timeout,
    /// A tap resolved to the expected gate but a stale (already
    /// re-rendered) digest — the payload changed after this tap's prompt
    /// was shown, so it must not be treated as an answer.
    StaleDigest {
        gate_id: String,
        option_key: String,
        digest: String,
        decided_at: chrono::DateTime<chrono::Utc>,
    },
    /// A concurrent `notify ask` already holds the per-bot ask lock.
    Busy,
}

impl AskOutcome {
    /// The process exit code for this outcome — a total function over all
    /// four variants: `Answered` 0, `Timeout` 2, `StaleDigest` 3, `Busy` 4.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        match self {
            AskOutcome::Answered { .. } => 0,
            AskOutcome::Timeout => 2,
            AskOutcome::StaleDigest { .. } => 3,
            AskOutcome::Busy => 4,
        }
    }

    /// Render this outcome as the single flat JSON object printed to
    /// stdout. Every field is asserted key-by-key in this module's tests —
    /// this shape is a cross-repo skill's contract, and a rename here must
    /// fail a test in this file rather than silently break a caller
    /// elsewhere.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            AskOutcome::Answered {
                gate_id,
                option_key,
                decided_at,
            } => serde_json::json!({
                "status": "answered",
                "gate_id": gate_id,
                "option_key": option_key,
                "decided_at": decided_at.to_rfc3339(),
            }),
            AskOutcome::Timeout => serde_json::json!({
                "status": "timeout",
            }),
            AskOutcome::StaleDigest {
                gate_id,
                option_key,
                digest,
                decided_at,
            } => serde_json::json!({
                "status": "stale_digest",
                "gate_id": gate_id,
                "option_key": option_key,
                "digest": digest,
                "decided_at": decided_at.to_rfc3339(),
            }),
            AskOutcome::Busy => serde_json::json!({
                "status": "busy",
            }),
        }
    }
}

// ── The per-batch poll decision ─────────────────────────────────────────

/// The result of running one `getUpdates` batch through [`decide_batch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchDecision {
    /// A response in this batch resolved to the expected gate and digest.
    Answered {
        gate_id: String,
        option_key: String,
        decided_at: chrono::DateTime<chrono::Utc>,
    },
    /// A response in this batch resolved to the expected gate but a stale
    /// digest.
    Stale {
        gate_id: String,
        option_key: String,
        digest: String,
        decided_at: chrono::DateTime<chrono::Utc>,
    },
    /// Nothing in this batch resolves the ask — either the batch was
    /// empty, or every response in it answered a different gate. The
    /// caller must keep polling, and (per the cursor rule below) must
    /// still advance its cursor past this batch regardless.
    KeepPolling,
}

/// Decide what one `getUpdates` batch means for an outstanding `ask`.
///
/// Delegates verdict resolution entirely to the existing
/// [`resolve_response`] — this function never compares digests or gate ids
/// itself. Responses are scanned in order; the first response that
/// resolves to [`ResponseVerdict::Accepted`](engine_core::operator::ResponseVerdict::Accepted)
/// wins immediately (an answer always takes precedence over a stale tap
/// seen earlier in the same batch). If nothing in the batch is `Accepted`
/// but at least one response resolves to `StaleDigest`, the first such tap
/// is returned. A response resolving to `UnknownGate` (a different gate's
/// tap) is neither an error nor a match — it is simply skipped while
/// scanning continues.
///
/// **CURSOR RULE** (enforced by the caller, per task 5 — not by this
/// function): the cursor `getUpdates`/`parse_updates` returns must be
/// advanced unconditionally after every batch, including a batch that
/// yields `KeepPolling` because it held only foreign-gate updates. Not
/// advancing past an update this ask chose to ignore replays it forever.
#[must_use]
pub fn decide_batch(
    responses: &[OperatorResponse],
    expected: &ValidatedOperatorPayload,
) -> BatchDecision {
    use engine_core::operator::ResponseVerdict;

    let mut stale: Option<BatchDecision> = None;

    for resp in responses {
        match resolve_response(resp, expected) {
            ResponseVerdict::Accepted {
                gate_id,
                option_key,
                decided_at,
                ..
            } => {
                return BatchDecision::Answered {
                    gate_id,
                    option_key,
                    decided_at,
                };
            }
            ResponseVerdict::StaleDigest {
                gate_id,
                option_key,
                digest,
                decided_at,
            } => {
                if stale.is_none() {
                    stale = Some(BatchDecision::Stale {
                        gate_id,
                        option_key,
                        digest,
                        decided_at,
                    });
                }
            }
            ResponseVerdict::UnknownGate => {}
        }
    }

    stale.unwrap_or(BatchDecision::KeepPolling)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::operator::{AckHandle, MessageHandle, OperatorPayload, OperatorPayloadLimits};

    // ── parse_option / parse_options ─────────────────────────────────

    #[test]
    fn parse_option_splits_on_first_colon_only() {
        let opt = parse_option("approve:Approve: ship it").expect("should parse");
        assert_eq!(opt.key, "approve");
        assert_eq!(opt.label, "Approve: ship it");
    }

    #[test]
    fn parse_option_rejects_missing_separator() {
        let err = parse_option("approve-Approve").unwrap_err();
        assert!(err.contains("':'"), "unexpected message: {err}");
    }

    #[test]
    fn parse_option_rejects_empty_key() {
        let err = parse_option(":Approve").unwrap_err();
        assert!(err.contains("empty key"), "unexpected message: {err}");
    }

    #[test]
    fn parse_option_rejects_empty_label() {
        let err = parse_option("approve:").unwrap_err();
        assert!(err.contains("empty label"), "unexpected message: {err}");
    }

    #[test]
    fn parse_option_empty_key_and_empty_label_messages_are_distinct() {
        let empty_key = parse_option(":Approve").unwrap_err();
        let empty_label = parse_option("approve:").unwrap_err();
        assert_ne!(empty_key, empty_label);
    }

    #[test]
    fn parse_options_rejects_duplicate_keys() {
        let raw = vec![
            "approve:Approve".to_string(),
            "approve:Also approve".to_string(),
        ];
        let err = parse_options(&raw).unwrap_err();
        assert!(err.contains("duplicate"), "unexpected message: {err}");
        assert!(err.contains("approve"), "unexpected message: {err}");
    }

    #[test]
    fn parse_options_accepts_distinct_keys_in_order() {
        let raw = vec!["approve:Approve".to_string(), "reject:Reject".to_string()];
        let opts = parse_options(&raw).expect("should parse");
        assert_eq!(opts.len(), 2);
        assert_eq!(opts[0].key, "approve");
        assert_eq!(opts[1].key, "reject");
    }

    // ── AskOutcome::exit_code ─────────────────────────────────────────

    #[test]
    fn exit_code_is_total_over_all_four_variants() {
        let now = chrono::Utc::now();
        assert_eq!(
            AskOutcome::Answered {
                gate_id: "g".to_string(),
                option_key: "approve".to_string(),
                decided_at: now,
            }
            .exit_code(),
            0
        );
        assert_eq!(AskOutcome::Timeout.exit_code(), 2);
        assert_eq!(
            AskOutcome::StaleDigest {
                gate_id: "g".to_string(),
                option_key: "approve".to_string(),
                digest: "deadbeef".to_string(),
                decided_at: now,
            }
            .exit_code(),
            3
        );
        assert_eq!(AskOutcome::Busy.exit_code(), 4);
    }

    // ── AskOutcome::to_json, key by key ─────────────────────────────────

    #[test]
    fn to_json_answered_carries_expected_keys() {
        let now = chrono::Utc::now();
        let json = AskOutcome::Answered {
            gate_id: "gate-1".to_string(),
            option_key: "approve".to_string(),
            decided_at: now,
        }
        .to_json();
        assert_eq!(json["status"], "answered");
        assert_eq!(json["gate_id"], "gate-1");
        assert_eq!(json["option_key"], "approve");
        assert_eq!(json["decided_at"], now.to_rfc3339());
        assert_eq!(
            json.as_object().expect("object").len(),
            4,
            "unexpected extra/missing key in answered shape"
        );
    }

    #[test]
    fn to_json_timeout_carries_only_status() {
        let json = AskOutcome::Timeout.to_json();
        assert_eq!(json["status"], "timeout");
        assert_eq!(json.as_object().expect("object").len(), 1);
    }

    #[test]
    fn to_json_stale_digest_carries_expected_keys() {
        let now = chrono::Utc::now();
        let json = AskOutcome::StaleDigest {
            gate_id: "gate-1".to_string(),
            option_key: "approve".to_string(),
            digest: "deadbeef".to_string(),
            decided_at: now,
        }
        .to_json();
        assert_eq!(json["status"], "stale_digest");
        assert_eq!(json["gate_id"], "gate-1");
        assert_eq!(json["option_key"], "approve");
        assert_eq!(json["digest"], "deadbeef");
        assert_eq!(json["decided_at"], now.to_rfc3339());
        assert_eq!(json.as_object().expect("object").len(), 5);
    }

    #[test]
    fn to_json_busy_carries_only_status() {
        let json = AskOutcome::Busy.to_json();
        assert_eq!(json["status"], "busy");
        assert_eq!(json.as_object().expect("object").len(), 1);
    }

    // ── decide_batch ──────────────────────────────────────────────────

    fn approve_reject() -> Vec<OperatorResponseOption> {
        vec![
            OperatorResponseOption::new("approve", "Approve"),
            OperatorResponseOption::new("reject", "Reject"),
        ]
    }

    fn validated(gate_id: &str, summary: &str) -> ValidatedOperatorPayload {
        let payload = OperatorPayload::new(gate_id, summary, approve_reject());
        engine_core::operator::validate(payload, &OperatorPayloadLimits::default())
            .expect("payload validates")
    }

    fn response_for(gate_id: &str, digest: &str, option_key: &str) -> OperatorResponse {
        OperatorResponse {
            gate_id: gate_id.to_string(),
            digest: digest.to_string(),
            option_key: option_key.to_string(),
            received_at: chrono::Utc::now(),
            ack: Some(AckHandle("ack-1".to_string())),
            message: Some(MessageHandle {
                chat_id: "chat-1".to_string(),
                message_id: 1,
            }),
        }
    }

    /// The truncated digest prefix `resolve_response` compares against,
    /// derived from `expected` the same way the transport does — see
    /// `crate::serve::notify::telegram::CALLBACK_DIGEST_PREFIX_LEN`.
    fn matching_prefix(expected: &ValidatedOperatorPayload) -> String {
        expected
            .payload()
            .digest
            .chars()
            .take(crate::serve::notify::telegram::CALLBACK_DIGEST_PREFIX_LEN)
            .collect()
    }

    #[test]
    fn decide_batch_empty_batch_keeps_polling() {
        let expected = validated("gate-1", "diff summary");
        assert_eq!(decide_batch(&[], &expected), BatchDecision::KeepPolling);
    }

    #[test]
    fn decide_batch_one_matching_tap_answers() {
        let expected = validated("gate-1", "diff summary");
        let prefix = matching_prefix(&expected);
        let responses = vec![response_for("gate-1", &prefix, "approve")];

        match decide_batch(&responses, &expected) {
            BatchDecision::Answered {
                gate_id,
                option_key,
                ..
            } => {
                assert_eq!(gate_id, "gate-1");
                assert_eq!(option_key, "approve");
            }
            other => panic!("expected Answered, got {other:?}"),
        }
    }

    #[test]
    fn decide_batch_foreign_gate_tap_keeps_polling_not_an_error() {
        let expected = validated("gate-1", "diff summary");
        let prefix = matching_prefix(&expected);
        let responses = vec![response_for("gate-other", &prefix, "approve")];

        assert_eq!(
            decide_batch(&responses, &expected),
            BatchDecision::KeepPolling
        );
    }

    #[test]
    fn decide_batch_stale_digest_tap_is_stale_never_answered() {
        let expected = validated("gate-1", "diff summary");
        let responses = vec![response_for("gate-1", "stale-prefix", "approve")];

        match decide_batch(&responses, &expected) {
            BatchDecision::Stale {
                gate_id,
                option_key,
                digest,
                ..
            } => {
                assert_eq!(gate_id, "gate-1");
                assert_eq!(option_key, "approve");
                assert_eq!(digest, "stale-prefix");
            }
            other => panic!("expected Stale, got {other:?}"),
        }
    }

    #[test]
    fn decide_batch_two_responses_only_second_matches_answers_from_second() {
        let expected = validated("gate-1", "diff summary");
        let prefix = matching_prefix(&expected);
        let responses = vec![
            response_for("gate-other", "irrelevant", "reject"),
            response_for("gate-1", &prefix, "approve"),
        ];

        match decide_batch(&responses, &expected) {
            BatchDecision::Answered {
                gate_id,
                option_key,
                ..
            } => {
                assert_eq!(gate_id, "gate-1");
                assert_eq!(option_key, "approve");
            }
            other => panic!("expected Answered, got {other:?}"),
        }
    }

    #[test]
    fn decide_batch_foreign_gate_tap_precedes_matching_tap_still_answers() {
        let expected = validated("gate-1", "diff summary");
        let prefix = matching_prefix(&expected);
        let responses = vec![
            response_for("gate-foreign", "whatever", "reject"),
            response_for("gate-1", &prefix, "approve"),
        ];

        match decide_batch(&responses, &expected) {
            BatchDecision::Answered { gate_id, .. } => assert_eq!(gate_id, "gate-1"),
            other => panic!("expected Answered, got {other:?}"),
        }
    }
}

// ── The reuse discipline itself ─────────────────────────────────────────
//
// Mechanical guard, kept OUTSIDE this file so `include_str!` on
// `notify_cli.rs` cannot trivially match its own guard strings: this file
// must never grow its own callback_data encoder, offset-advance
// arithmetic, or digest comparison — all three already exist, tested, in
// `src/serve/notify/telegram.rs`. This module's own reliance on
// `resolve_response` (rather than a local digest comparison) is the
// positive half of that guarantee and is exercised by `decide_batch`'s
// tests above; a later task's binary-level contract test carries the
// mechanical grep-style enforcement over the file itself.
