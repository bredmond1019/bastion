//! Machine-caller HMAC signature verification for `bastion serve`.
//!
//! Ports the qm `source-auth-sign.ts` shape (recorded in
//! `planning/archive/qm-comparison-findings/notes.md` §7(b)) to bastion: a
//! request signs `method\npath\nbody` with HMAC-SHA256 under a shared
//! `BASTION_SERVE_SIGNING_KEY`, prefixed with `v0:<unix-ts>:`, and sends the
//! timestamp and hex-encoded signature as the `x-timestamp` / `x-signature`
//! headers. A request to a protected route may present this signature tier
//! INSTEAD OF a bearer token — see [`crate::serve::auth`] for the bearer
//! tier, which this tier is additive to, never a replacement for.
//!
//! # Design
//! Every function here is pure: no server, no network, and — critically —
//! no implicit clock read. [`signature_matches`] and
//! [`timestamp_within_skew`] take the current time as a plain `i64`
//! parameter rather than calling `SystemTime::now()` internally, so the
//! skew-window tests below run with no sleeping and no environment
//! dependency. This mirrors exactly how `src/serve/auth.rs` factors
//! `token_matches` as a pure, exhaustively-unit-tested core with a thin I/O
//! shell around it.
//!
//! # Replay limit (read this before trusting this scheme for more than it does)
//!
//! This scheme BOUNDS replay; it does NOT prevent it. Any signature accepted
//! once is valid for every subsequent request within the same skew window —
//! there is no nonce store, no single-use enforcement, nothing that
//! distinguishes a legitimate retry from a captured-and-replayed request. A
//! narrower `skew_secs` shrinks the replay window; it does not close it.
//! Do not describe this module elsewhere as replay-proof.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

/// Build the canonical string a signature is computed over: `method`, a
/// newline, `path`, a newline, then the raw request `body`.
///
/// The method is uppercased for consistency between caller and verifier.
/// The path and body are used verbatim — this function must never trim,
/// normalise, percent-decode, or otherwise re-encode any of the three
/// components. A canonicalisation that discards or reinterprets any of them
/// reopens exactly the request-mutation attack this scheme exists to close:
/// a signature that matches after canonicalisation but not before it would
/// let a man-in-the-middle alter the request the server actually sees.
pub fn canonical_string(method: &str, path: &str, body: &[u8]) -> String {
    let mut s = String::with_capacity(method.len() + path.len() + body.len() + 2);
    s.push_str(&method.to_ascii_uppercase());
    s.push('\n');
    s.push_str(path);
    s.push('\n');
    // Body is arbitrary bytes, not necessarily UTF-8 text — treat it as
    // Latin-1-ish "raw bytes as chars" so we never re-encode it. Since the
    // canonical string is only ever hashed (never displayed or parsed back),
    // a lossy-but-deterministic byte-for-byte mapping is exactly what we
    // want: every distinct byte sequence maps to a distinct String.
    s.extend(body.iter().map(|&b| b as char));
    s
}

/// Prefix the canonical string with `v0:<unix-ts>:`, producing the exact
/// payload the HMAC is computed over.
pub fn signing_payload(timestamp: i64, canonical: &str) -> String {
    format!("v0:{timestamp}:{canonical}")
}

/// Hex-encode `bytes` as lowercase hex, with no external crate dependency.
fn to_hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Verify `provided_signature` (lowercase hex) against the HMAC-SHA256 of
/// `signing_payload(timestamp, canonical)` under `signing_key`.
///
/// Returns `false` immediately — without computing or comparing any HMAC —
/// when `signing_key` or `provided_signature` is empty, mirroring
/// `token_matches`'s server-misconfiguration guard in `src/serve/auth.rs`.
/// The final comparison uses [`subtle::ConstantTimeEq`], the same
/// constant-time primitive `token_matches` uses, so a caller probing the
/// signature byte-by-byte gains nothing from response timing.
pub fn verify_signature(
    signing_key: &str,
    provided_signature: &str,
    timestamp: i64,
    canonical: &str,
) -> bool {
    if signing_key.is_empty() || provided_signature.is_empty() {
        return false;
    }
    let payload = signing_payload(timestamp, canonical);
    let Ok(mut mac) = HmacSha256::new_from_slice(signing_key.as_bytes()) else {
        // HMAC-SHA256 accepts any key length, so this branch is unreachable
        // in practice; kept as a safe, non-panicking fallback.
        return false;
    };
    mac.update(payload.as_bytes());
    let expected = to_hex_lower(&mac.finalize().into_bytes());
    bool::from(expected.as_bytes().ct_eq(provided_signature.as_bytes()))
}

/// True when `timestamp` is within `skew_secs` seconds of `now`, in either
/// direction — a far-future timestamp is rejected exactly as a stale one is.
pub fn timestamp_within_skew(timestamp: i64, now: i64, skew_secs: u64) -> bool {
    let skew_secs = skew_secs as i64;
    (now - timestamp).abs() <= skew_secs
}

/// The composed entry point: verify that a request carries a valid
/// machine-caller signature.
///
/// Returns `false` when:
/// - `signing_key` is `None` (server is running bearer-only, machine-caller
///   tier is disabled),
/// - `timestamp_header` or `signature_header` is absent,
/// - `timestamp_header` does not parse as an `i64`,
/// - the timestamp falls outside `skew_secs` of `now`, or
/// - the HMAC itself does not verify.
///
/// Otherwise returns the AND of the skew check and [`verify_signature`].
#[allow(clippy::too_many_arguments)]
pub fn signature_matches(
    signing_key: Option<&str>,
    timestamp_header: Option<&str>,
    signature_header: Option<&str>,
    method: &str,
    path: &str,
    body: &[u8],
    now: i64,
    skew_secs: u64,
) -> bool {
    let Some(signing_key) = signing_key else {
        return false;
    };
    let Some(timestamp_header) = timestamp_header else {
        return false;
    };
    let Some(signature_header) = signature_header else {
        return false;
    };
    let Ok(timestamp) = timestamp_header.parse::<i64>() else {
        return false;
    };
    if !timestamp_within_skew(timestamp, now, skew_secs) {
        return false;
    }
    let canonical = canonical_string(method, path, body);
    verify_signature(signing_key, signature_header, timestamp, &canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Known signing key / timestamp / method / path / body, with the
    // expected hex digest written out literally (computed independently via
    // Python's hmac/hashlib against the same canonical construction) so the
    // canonicalisation itself is pinned, not merely self-consistent.
    const KNOWN_KEY: &str = "test-signing-key";
    const KNOWN_TS: i64 = 1_700_000_000;
    const KNOWN_METHOD: &str = "POST";
    const KNOWN_PATH: &str = "/api/foo";
    const KNOWN_BODY: &[u8] = b"hello";
    const KNOWN_SIG: &str = "4e3e0124ba7758d37410f7dc98dbdbfea1778b3e8b1672cd4354454f6e316f2f";

    #[test]
    fn canonical_string_shape() {
        assert_eq!(
            canonical_string(KNOWN_METHOD, KNOWN_PATH, KNOWN_BODY),
            "POST\n/api/foo\nhello"
        );
    }

    #[test]
    fn canonical_string_uppercases_method() {
        assert_eq!(
            canonical_string("post", KNOWN_PATH, KNOWN_BODY),
            "POST\n/api/foo\nhello"
        );
    }

    #[test]
    fn signing_payload_prefix() {
        let canonical = canonical_string(KNOWN_METHOD, KNOWN_PATH, KNOWN_BODY);
        assert_eq!(
            signing_payload(KNOWN_TS, &canonical),
            "v0:1700000000:POST\n/api/foo\nhello"
        );
    }

    /// Known-vector round trip: pins the canonical string AND the resulting
    /// hex digest against literal expected bytes computed independently
    /// (Python `hmac.new(key, payload, hashlib.sha256).hexdigest()`), so a
    /// bug that changes the canonicalisation but leaves the code
    /// self-consistent would still be caught.
    #[test]
    fn known_vector_round_trip() {
        let canonical = canonical_string(KNOWN_METHOD, KNOWN_PATH, KNOWN_BODY);
        assert!(verify_signature(KNOWN_KEY, KNOWN_SIG, KNOWN_TS, &canonical));
    }

    #[test]
    fn rejects_signature_over_different_method() {
        // Signature was computed over GET, but the request claims POST.
        let wrong_canonical = canonical_string("GET", KNOWN_PATH, KNOWN_BODY);
        let wrong_sig_payload = signing_payload(KNOWN_TS, &wrong_canonical);
        let wrong_sig = {
            let mut mac = HmacSha256::new_from_slice(KNOWN_KEY.as_bytes()).unwrap();
            mac.update(wrong_sig_payload.as_bytes());
            to_hex_lower(&mac.finalize().into_bytes())
        };
        // Verify against the ORIGINAL (POST) canonical — must fail.
        let real_canonical = canonical_string(KNOWN_METHOD, KNOWN_PATH, KNOWN_BODY);
        assert!(!verify_signature(
            KNOWN_KEY,
            &wrong_sig,
            KNOWN_TS,
            &real_canonical
        ));
    }

    #[test]
    fn rejects_signature_over_different_path() {
        let wrong_canonical = canonical_string(KNOWN_METHOD, "/api/bar", KNOWN_BODY);
        let wrong_sig_payload = signing_payload(KNOWN_TS, &wrong_canonical);
        let wrong_sig = {
            let mut mac = HmacSha256::new_from_slice(KNOWN_KEY.as_bytes()).unwrap();
            mac.update(wrong_sig_payload.as_bytes());
            to_hex_lower(&mac.finalize().into_bytes())
        };
        let real_canonical = canonical_string(KNOWN_METHOD, KNOWN_PATH, KNOWN_BODY);
        assert!(!verify_signature(
            KNOWN_KEY,
            &wrong_sig,
            KNOWN_TS,
            &real_canonical
        ));
    }

    #[test]
    fn rejects_signature_over_different_body() {
        let wrong_canonical = canonical_string(KNOWN_METHOD, KNOWN_PATH, b"goodbye");
        let wrong_sig_payload = signing_payload(KNOWN_TS, &wrong_canonical);
        let wrong_sig = {
            let mut mac = HmacSha256::new_from_slice(KNOWN_KEY.as_bytes()).unwrap();
            mac.update(wrong_sig_payload.as_bytes());
            to_hex_lower(&mac.finalize().into_bytes())
        };
        let real_canonical = canonical_string(KNOWN_METHOD, KNOWN_PATH, KNOWN_BODY);
        assert!(!verify_signature(
            KNOWN_KEY,
            &wrong_sig,
            KNOWN_TS,
            &real_canonical
        ));
    }

    #[test]
    fn timestamp_one_second_past_window_rejected_future() {
        // now is 100s after timestamp; skew is 99s -> just outside window.
        assert!(!timestamp_within_skew(1000, 1099 + 1, 99));
    }

    #[test]
    fn timestamp_one_second_past_window_rejected_past() {
        // timestamp is in the future relative to now by 100s; skew 99s -> outside.
        assert!(!timestamp_within_skew(1099 + 1, 1000, 99));
    }

    #[test]
    fn timestamp_within_skew_boundary_accepted() {
        assert!(timestamp_within_skew(1000, 1099, 99));
        assert!(timestamp_within_skew(1099, 1000, 99));
    }

    /// A timestamp one second outside the skew window is rejected in BOTH
    /// directions even though the underlying HMAC verifies fine — the skew
    /// check must short-circuit the signature check, not merely accompany it.
    #[test]
    fn signature_matches_rejects_stale_timestamp_even_though_hmac_verifies() {
        let canonical = canonical_string(KNOWN_METHOD, KNOWN_PATH, KNOWN_BODY);
        assert!(verify_signature(KNOWN_KEY, KNOWN_SIG, KNOWN_TS, &canonical));

        let too_late_now = KNOWN_TS + 61; // skew window below is 60s
        assert!(!signature_matches(
            Some(KNOWN_KEY),
            Some(&KNOWN_TS.to_string()),
            Some(KNOWN_SIG),
            KNOWN_METHOD,
            KNOWN_PATH,
            KNOWN_BODY,
            too_late_now,
            60,
        ));
    }

    #[test]
    fn signature_matches_rejects_future_timestamp_even_though_hmac_verifies() {
        let canonical = canonical_string(KNOWN_METHOD, KNOWN_PATH, KNOWN_BODY);
        assert!(verify_signature(KNOWN_KEY, KNOWN_SIG, KNOWN_TS, &canonical));

        let too_early_now = KNOWN_TS - 61; // request timestamp is 61s in "the future" of now
        assert!(!signature_matches(
            Some(KNOWN_KEY),
            Some(&KNOWN_TS.to_string()),
            Some(KNOWN_SIG),
            KNOWN_METHOD,
            KNOWN_PATH,
            KNOWN_BODY,
            too_early_now,
            60,
        ));
    }

    /// A timestamp inside the window accepted twice with the SAME signature.
    /// This IS accepted replay within the window — a known, documented
    /// limit of this scheme (see the module doc comment), not a bug.
    #[test]
    fn same_signature_accepted_twice_within_window_is_known_accepted_replay() {
        let first_call = signature_matches(
            Some(KNOWN_KEY),
            Some(&KNOWN_TS.to_string()),
            Some(KNOWN_SIG),
            KNOWN_METHOD,
            KNOWN_PATH,
            KNOWN_BODY,
            KNOWN_TS + 10,
            60,
        );
        let second_call_same_signature_replayed = signature_matches(
            Some(KNOWN_KEY),
            Some(&KNOWN_TS.to_string()),
            Some(KNOWN_SIG),
            KNOWN_METHOD,
            KNOWN_PATH,
            KNOWN_BODY,
            KNOWN_TS + 10,
            60,
        );
        assert!(
            first_call && second_call_same_signature_replayed,
            "replay within the skew window is an accepted, documented limit of this scheme \
             (see module doc comment) — this scheme bounds replay, it does not prevent it"
        );
    }

    #[test]
    fn signature_matches_rejects_unparseable_timestamp() {
        assert!(!signature_matches(
            Some(KNOWN_KEY),
            Some("not-a-number"),
            Some(KNOWN_SIG),
            KNOWN_METHOD,
            KNOWN_PATH,
            KNOWN_BODY,
            KNOWN_TS,
            60,
        ));
    }

    #[test]
    fn signature_matches_rejects_absent_signing_key() {
        assert!(!signature_matches(
            None,
            Some(&KNOWN_TS.to_string()),
            Some(KNOWN_SIG),
            KNOWN_METHOD,
            KNOWN_PATH,
            KNOWN_BODY,
            KNOWN_TS,
            60,
        ));
    }

    #[test]
    fn signature_matches_rejects_absent_timestamp_header() {
        assert!(!signature_matches(
            Some(KNOWN_KEY),
            None,
            Some(KNOWN_SIG),
            KNOWN_METHOD,
            KNOWN_PATH,
            KNOWN_BODY,
            KNOWN_TS,
            60,
        ));
    }

    #[test]
    fn signature_matches_rejects_absent_signature_header() {
        assert!(!signature_matches(
            Some(KNOWN_KEY),
            Some(&KNOWN_TS.to_string()),
            None,
            KNOWN_METHOD,
            KNOWN_PATH,
            KNOWN_BODY,
            KNOWN_TS,
            60,
        ));
    }

    #[test]
    fn verify_signature_rejects_empty_signing_key() {
        let canonical = canonical_string(KNOWN_METHOD, KNOWN_PATH, KNOWN_BODY);
        assert!(!verify_signature("", KNOWN_SIG, KNOWN_TS, &canonical));
    }

    #[test]
    fn verify_signature_rejects_empty_provided_signature() {
        let canonical = canonical_string(KNOWN_METHOD, KNOWN_PATH, KNOWN_BODY);
        assert!(!verify_signature(KNOWN_KEY, "", KNOWN_TS, &canonical));
    }
}
