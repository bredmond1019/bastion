//! Build provenance stamp: the `bastion --build-stamp` JSON surface and the pure verdict
//! function used to detect drift before any corpus-writing path (`emit-state --write`).
//!
//! [`build.rs`](../build.rs) stamps three `cargo:rustc-env` values into the binary at
//! compile time (`BASTION_BUILD_GIT_SHA`, `BASTION_BUILD_DIRTY`, `BASTION_BUILD_SOURCE_DIR`);
//! this module re-derives the live value at runtime and compares.
//!
//! The verdict is a pure function of `(stamped_sha, live_sha, dirty, source_dir_exists)` —
//! [`verdict`] — so it is unit-tested directly without shelling out to git, mirroring
//! `core/mev/src/brain/conformance/toolchain.rs`.
//!
//! The `{git_sha, dirty, source_dir}` JSON key set emitted by [`stamp_json`] is a pinned
//! cross-repo contract with `mev:MV.ticket.toolchain-freshness-covers-the-writer` — do not
//! add, rename, or drop a key.

use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

const STAMPED_SHA: &str = env!("BASTION_BUILD_GIT_SHA");
const STAMPED_DIRTY: &str = env!("BASTION_BUILD_DIRTY");
const STAMPED_SOURCE_DIR: &str = env!("BASTION_BUILD_SOURCE_DIR");

/// Build provenance drift verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The stamped SHA matches the live source tree HEAD, and the tree was clean at build
    /// time.
    Pass,
    /// The binary's provenance is verifiable but does not match the source tree — either
    /// the SHA differs or the tree was dirty at build time. Carries a human-readable reason
    /// naming the specifics.
    Drift(String),
    /// Provenance could not be established either way (no git, no `.git/`, unknown stamp,
    /// or the source dir is missing). Never treated as drift.
    NotEvaluable(String),
}

/// The pure verdict function: given the compiled-in stamp and the live state of the source
/// tree, decide pass / drift / not-evaluable. No I/O — callers gather the live values (via
/// [`live_head`] or otherwise) and pass them in, which is what makes this directly
/// unit-testable without shelling out to git in tests.
pub fn verdict(
    stamped_sha: &str,
    live_sha: Option<&str>,
    dirty: &str,
    source_dir_exists: bool,
) -> Verdict {
    if stamped_sha == "unknown" || dirty == "unknown" || !source_dir_exists {
        return Verdict::NotEvaluable(
            "build provenance unavailable: stamped SHA, dirty flag, or source dir missing"
                .to_string(),
        );
    }

    let Some(live_sha) = live_sha else {
        return Verdict::NotEvaluable(
            "could not determine the live HEAD of the source tree (git unavailable)".to_string(),
        );
    };

    if live_sha == "unknown" {
        return Verdict::NotEvaluable(
            "could not determine the live HEAD of the source tree (git unavailable)".to_string(),
        );
    }

    if stamped_sha != live_sha {
        return Verdict::Drift(format!(
            "the running binary was built from {stamped_sha} but the source is now at \
             {live_sha}; rebuild before any --write run"
        ));
    }

    if dirty == "1" {
        return Verdict::Drift(
            "the binary was built from an uncommitted tree, so its provenance is unverifiable"
                .to_string(),
        );
    }

    Verdict::Pass
}

/// Run `git rev-parse HEAD` in `source_dir` now, returning `None` if the dir is absent or
/// git is unavailable. The only I/O in this module.
pub fn live_head(source_dir: &str) -> Option<String> {
    if !Path::new(source_dir).exists() {
        return None;
    }
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(source_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|s| s.trim().to_string())
}

/// Build the `dirty` JSON value from the raw stamped flag: a JSON boolean when the stamp
/// is `"0"`/`"1"`, or the literal string `"unknown"` when the stamp couldn't be determined
/// at build time — never guessed.
fn dirty_json_value(dirty: &str) -> Value {
    match dirty {
        "0" => Value::Bool(false),
        "1" => Value::Bool(true),
        _ => Value::String("unknown".to_string()),
    }
}

/// Build the `{git_sha, dirty, source_dir}` JSON contract from raw stamp values (pure —
/// exposed for testing independent of the compiled-in `env!` consts).
pub fn stamp_json_from(git_sha: &str, dirty: &str, source_dir: &str) -> Value {
    json!({
        "git_sha": git_sha,
        "dirty": dirty_json_value(dirty),
        "source_dir": source_dir,
    })
}

/// Build the `{git_sha, dirty, source_dir}` JSON contract from this binary's compiled-in
/// stamp. This is the exact shape `bastion --build-stamp` prints to stdout, and the
/// contract `mev:MV.ticket.toolchain-freshness-covers-the-writer` queries.
pub fn stamp_json() -> Value {
    stamp_json_from(STAMPED_SHA, STAMPED_DIRTY, STAMPED_SOURCE_DIR)
}

/// Compute the drift verdict for the currently running binary against its live source
/// tree, using the compiled-in stamp. The only public entry point that performs I/O
/// (via [`live_head`]).
pub fn current_verdict() -> Verdict {
    let source_dir_exists = Path::new(STAMPED_SOURCE_DIR).exists();
    let live_sha = if source_dir_exists {
        live_head(STAMPED_SOURCE_DIR)
    } else {
        None
    };
    verdict(
        STAMPED_SHA,
        live_sha.as_deref(),
        STAMPED_DIRTY,
        source_dir_exists,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── verdict: seven cases from the task spec ─────────────────────────────

    #[test]
    fn pass_when_sha_matches_and_clean() {
        let v = verdict("abc123", Some("abc123"), "0", true);
        assert_eq!(v, Verdict::Pass);
    }

    #[test]
    fn drift_when_sha_differs_names_both() {
        let v = verdict("abc123", Some("def456"), "0", true);
        match v {
            Verdict::Drift(reason) => {
                assert!(reason.contains("abc123"));
                assert!(reason.contains("def456"));
                assert!(reason.contains("rebuild"));
            }
            other => panic!("expected Drift, got {other:?}"),
        }
    }

    #[test]
    fn drift_when_dirty_even_with_matching_sha() {
        let v = verdict("abc123", Some("abc123"), "1", true);
        match v {
            Verdict::Drift(reason) => assert!(reason.contains("uncommitted")),
            other => panic!("expected Drift, got {other:?}"),
        }
    }

    #[test]
    fn not_evaluable_when_stamped_sha_unknown() {
        let v = verdict("unknown", Some("abc123"), "0", true);
        assert!(matches!(v, Verdict::NotEvaluable(_)));
    }

    #[test]
    fn not_evaluable_when_dirty_unknown() {
        let v = verdict("abc123", Some("abc123"), "unknown", true);
        assert!(matches!(v, Verdict::NotEvaluable(_)));
    }

    #[test]
    fn not_evaluable_when_source_dir_missing() {
        let v = verdict("abc123", Some("abc123"), "0", false);
        assert!(matches!(v, Verdict::NotEvaluable(_)));
    }

    #[test]
    fn not_evaluable_when_live_sha_none() {
        let v = verdict("abc123", None, "0", true);
        assert!(matches!(v, Verdict::NotEvaluable(_)));
    }

    #[test]
    fn not_evaluable_when_live_sha_literal_unknown() {
        let v = verdict("abc123", Some("unknown"), "0", true);
        assert!(matches!(v, Verdict::NotEvaluable(_)));
    }

    #[test]
    fn dirty_drift_message_distinct_from_stale_sha_drift_message() {
        let stale = verdict("abc123", Some("def456"), "0", true);
        let dirty = verdict("abc123", Some("abc123"), "1", true);
        let (Verdict::Drift(a), Verdict::Drift(b)) = (stale, dirty) else {
            panic!("expected both to be Drift");
        };
        assert_ne!(a, b);
    }

    // ── JSON serialisation shape ─────────────────────────────────────────────

    #[test]
    fn stamp_json_has_exactly_three_keys() {
        let v = stamp_json_from("abc123", "0", "/src/bastion");
        let obj = v.as_object().expect("stamp_json must be a JSON object");
        assert_eq!(obj.len(), 3, "expected exactly 3 keys, got {obj:?}");
        assert!(obj.contains_key("git_sha"));
        assert!(obj.contains_key("dirty"));
        assert!(obj.contains_key("source_dir"));
    }

    #[test]
    fn stamp_json_git_sha_and_source_dir_are_strings() {
        let v = stamp_json_from("abc123", "0", "/src/bastion");
        assert_eq!(v["git_sha"], Value::String("abc123".to_string()));
        assert_eq!(v["source_dir"], Value::String("/src/bastion".to_string()));
    }

    #[test]
    fn stamp_json_dirty_is_boolean_when_stamp_is_zero_or_one() {
        let clean = stamp_json_from("abc123", "0", "/src");
        assert_eq!(clean["dirty"], Value::Bool(false));

        let dirty = stamp_json_from("abc123", "1", "/src");
        assert_eq!(dirty["dirty"], Value::Bool(true));
    }

    #[test]
    fn stamp_json_dirty_is_string_unknown_when_stamp_is_unknown() {
        let v = stamp_json_from("abc123", "unknown", "/src");
        assert_eq!(v["dirty"], Value::String("unknown".to_string()));
    }

    #[test]
    fn stamp_json_matches_stamp_json_from_with_compiled_in_consts() {
        // stamp_json() must be exactly stamp_json_from() applied to the compiled-in
        // env! consts — no divergence in shape between the two entry points.
        let expected = stamp_json_from(STAMPED_SHA, STAMPED_DIRTY, STAMPED_SOURCE_DIR);
        assert_eq!(stamp_json(), expected);
    }

    #[test]
    fn current_verdict_does_not_panic() {
        // Smoke test: reads the real compiled-in stamp and shells out to the real
        // source dir. Must not panic regardless of the environment (git present or
        // not, source dir intact or not).
        let _ = current_verdict();
    }
}
