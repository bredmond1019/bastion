//! Pure lease classification for `bastion serve`'s human send paths
//! (`BA.ticket.human-send-into-leased-session-warns`).
//!
//! `engine-rs` nodes that type into a tmux pane route every send through
//! `term_core::driver::GuardedSender`, which renews `@engine_lease@<session>`
//! and honours an operator hold. `bastion serve`'s human input paths (REST
//! `send`/`send_key`, WS `Send`/`SendKey`) bypass all of that and call
//! `term_core::tmux::send_keys` directly — so a person can type into a
//! session an engine run currently holds with nothing recorded anywhere.
//!
//! This module does not refuse or delay the send (the operator's
//! audit-and-warn decision, 2026-09-12) — it only classifies whatever the
//! `@engine_lease@<session>` tmux user-option currently holds so the caller
//! can log a structured warning and, on REST, surface an `X-Bastion-Warning`
//! response header. Everything here is pure except [`read_engine_lease`],
//! which is a thin blocking I/O shell over `term_core::tmux::run_tmux`.

use term_core::lease::{LEASE_OPTION, Lease};
use term_core::tmux::{run_tmux, show_option_args};

/// A live lease found on a session's `@engine_lease@<session>` tmux option,
/// ready to be logged and reflected in an `X-Bastion-Warning` header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeasedSendWarning {
    pub run_id: String,
    pub identity: String,
    pub expires_at_ms: u64,
}

/// Compose the tmux user-option name a session's engine lease is stashed
/// under: `@engine_lease@<session>`.
///
/// Built from `term_core::lease::LEASE_OPTION` rather than duplicating the
/// literal, so this tracks the constant if it ever changes upstream.
#[must_use]
pub fn engine_lease_option_name(session: &str) -> String {
    format!("{LEASE_OPTION}@{session}")
}

/// Classify a raw `@engine_lease@<session>` tmux option value against a
/// caller-supplied "now" (milliseconds since the Unix epoch).
///
/// - `None` or an empty/whitespace-only value -> no lease.
/// - A value that does not parse into `term_core::lease::Lease` (not
///   exactly four `:`-separated fields, or a non-numeric expiry) -> no
///   lease — malformed is never guessed at or partially accepted.
/// - A value that parses but has already expired (`now_ms >= expires_at_ms`)
///   -> no lease.
/// - Otherwise -> `Some` warning carrying `run_id`, `identity` and
///   `expires_at_ms`.
#[must_use]
pub fn classify_lease(raw: Option<&str>, now_ms: u64) -> Option<LeasedSendWarning> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let lease = Lease::parse(raw)?;
    if now_ms >= lease.expires_at_ms {
        return None;
    }
    Some(LeasedSendWarning {
        run_id: lease.run_id,
        identity: lease.identity,
        expires_at_ms: lease.expires_at_ms,
    })
}

/// Format an `X-Bastion-Warning` header value for a live lease, naming the
/// run and identity that hold the session.
#[must_use]
pub fn warning_header_value(warning: &LeasedSendWarning) -> String {
    format!(
        "leased-session; run_id={}; identity={}",
        warning.run_id, warning.identity
    )
}

/// Read a session's `@engine_lease@<session>` tmux option, or `None` if it
/// is unset or the underlying tmux call fails for any reason.
///
/// This is a thin I/O shell: it contains no classification logic beyond
/// mapping `Err` to `None`. A tmux read error (no server, unset user
/// option, tmux not installed, …) must never fail or delay the send it
/// gates — it reads as "no lease", exactly like a genuinely absent option.
#[must_use]
pub fn read_engine_lease(session: &str) -> Option<String> {
    let option_name = engine_lease_option_name(session);
    run_tmux(&show_option_args(&option_name)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_name_is_composed_exactly() {
        assert_eq!(engine_lease_option_name("s"), "@engine_lease@s");
    }

    #[test]
    fn classify_none_is_no_lease() {
        assert_eq!(classify_lease(None, 1_000), None);
    }

    #[test]
    fn classify_empty_is_no_lease() {
        assert_eq!(classify_lease(Some(""), 1_000), None);
    }

    #[test]
    fn classify_whitespace_only_is_no_lease() {
        assert_eq!(classify_lease(Some("   "), 1_000), None);
    }

    #[test]
    fn classify_malformed_wrong_field_count_is_no_lease() {
        // Only three fields, not the required four.
        assert_eq!(classify_lease(Some("run:nonce:identity"), 1_000), None);
    }

    #[test]
    fn classify_malformed_non_numeric_expiry_is_no_lease() {
        assert_eq!(
            classify_lease(Some("run:nonce:identity:not-a-number"), 1_000),
            None
        );
    }

    #[test]
    fn classify_expired_at_exactly_now_is_no_lease() {
        // now_ms == expires_at_ms counts as expired.
        assert_eq!(classify_lease(Some("run:nonce:identity:1000"), 1_000), None);
    }

    #[test]
    fn classify_expired_in_the_past_is_no_lease() {
        assert_eq!(classify_lease(Some("run:nonce:identity:999"), 1_000), None);
    }

    #[test]
    fn classify_live_lease_returns_warning() {
        let warning = classify_lease(Some("run-1:nonce-1:node-1:2000"), 1_000)
            .expect("live lease must classify as Some");
        assert_eq!(
            warning,
            LeasedSendWarning {
                run_id: "run-1".to_string(),
                identity: "node-1".to_string(),
                expires_at_ms: 2000,
            }
        );
    }

    #[test]
    fn classify_live_lease_trims_surrounding_whitespace() {
        let warning = classify_lease(Some("  run-1:nonce-1:node-1:2000  "), 1_000)
            .expect("whitespace-padded live lease must still classify as Some");
        assert_eq!(warning.run_id, "run-1");
    }

    #[test]
    fn warning_header_value_names_run_id_and_identity() {
        let warning = LeasedSendWarning {
            run_id: "run-1".to_string(),
            identity: "node-1".to_string(),
            expires_at_ms: 2000,
        };
        let value = warning_header_value(&warning);
        assert!(value.contains("run_id=run-1"), "value was: {value}");
        assert!(value.contains("identity=node-1"), "value was: {value}");
    }

    #[test]
    fn read_engine_lease_maps_tmux_error_to_none() {
        // No tmux server / no such session in this test environment — the
        // call must never propagate an error, only classify as no lease.
        assert_eq!(read_engine_lease("no-such-session-leased-send-tests"), None);
    }
}
