//! BA.26.E task 3: regression test pinning that
//! `com.brandon.bastion-serve.plist` and `com.brandon.engine-serve.plist`
//! name the same `DATABASE_URL`. Drift between the two databases the two
//! services point at would otherwise silently empty the history pane rather
//! than fail a build (block record `why`).
//!
//! Two-part shape (block AC, D68 shown-failing recipe):
//! 1. A fixture pair (GATED) proves the comparison logic can go RED when the
//!    two plists disagree, and green when they agree.
//! 2. The REAL files at the HQ root (NOT gated in CI — the private HQ vault
//!    is invisible to a public checkout) are read and compared when
//!    resolvable; otherwise this SKIPS with a named reason, exactly like
//!    `tests/harness_capture_gate.rs`'s existing SKIP-not-FAIL pattern for
//!    the same reason.

use std::path::Path;

/// Extract the `<string>` value immediately following a
/// `<key>DATABASE_URL</key>` line in a launchd plist. Simple line-oriented
/// extraction is sufficient for this known plist shape — no plist-parsing
/// crate dependency is needed for two fixed-shape files.
fn extract_database_url(raw: &str) -> Option<String> {
    let mut lines = raw.lines();
    while let Some(line) = lines.next() {
        if line.trim() == "<key>DATABASE_URL</key>" {
            let value_line = lines.next()?;
            let trimmed = value_line.trim();
            let inner = trimmed
                .strip_prefix("<string>")?
                .strip_suffix("</string>")?;
            return Some(inner.to_string());
        }
    }
    None
}

/// True when both plist bodies name the same `DATABASE_URL`. `None` on
/// either side (key missing/malformed) is never treated as agreement.
fn database_urls_agree(bastion_serve_plist: &str, engine_serve_plist: &str) -> bool {
    match (
        extract_database_url(bastion_serve_plist),
        extract_database_url(engine_serve_plist),
    ) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

const FIXTURE_HEADER: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>EnvironmentVariables</key>
	<dict>
"#;

const FIXTURE_FOOTER: &str = r#"	</dict>
</dict>
</plist>
"#;

fn fixture_plist(database_url: &str) -> String {
    format!(
        "{FIXTURE_HEADER}\t\t<key>DATABASE_URL</key>\n\t\t<string>{database_url}</string>\n{FIXTURE_FOOTER}"
    )
}

/// Test 1 (GATED): the fixture pair. Proves the comparison function can go
/// RED on a genuine mismatch and green on a genuine match — the D68
/// shown-failing recipe, run against a deliberately-controlled pair rather
/// than the real files this test does not depend on.
#[test]
fn fixture_pair_comparison_goes_red_on_mismatch_and_green_on_match() {
    let matching_a = fixture_plist("postgres://brandon@localhost:5432/orchestration_dev");
    let matching_b = fixture_plist("postgres://brandon@localhost:5432/orchestration_dev");
    assert!(
        database_urls_agree(&matching_a, &matching_b),
        "matching fixture pair must be reported as agreeing"
    );

    let mismatched_a = fixture_plist("postgres://brandon@localhost:5432/orchestration_dev");
    let mismatched_b = fixture_plist("postgres://brandon@localhost:5432/some_other_db");
    assert!(
        !database_urls_agree(&mismatched_a, &mismatched_b),
        "mismatched fixture pair must be reported as disagreeing (shown-failing recipe)"
    );
}

#[test]
fn extract_database_url_returns_none_when_key_absent() {
    let no_key_plist = format!("{FIXTURE_HEADER}{FIXTURE_FOOTER}");
    assert_eq!(extract_database_url(&no_key_plist), None);
    assert!(!database_urls_agree(&no_key_plist, &no_key_plist));
}

/// Test 2 (NOT gated in CI): the real files at the HQ root. SKIPS — never
/// fails — when the HQ root or either plist cannot be resolved, which is
/// every hosted CI checkout (the private HQ vault is not part of this
/// public repo). When both ARE present, asserts they name the same
/// `DATABASE_URL`.
#[test]
fn real_plists_at_hq_root_name_the_same_database_url() {
    let brain_root = match engine_core::brain_root::resolve_brain_root() {
        Ok(root) => root,
        Err(e) => {
            eprintln!(
                "SKIPPED real_plists_at_hq_root_name_the_same_database_url: \
                 cannot resolve HQ brain root ({e}) — expected in hosted CI, \
                 which cannot see the private HQ vault"
            );
            return;
        }
    };

    let bastion_serve_path = brain_root.join("scripts/launchd/com.brandon.bastion-serve.plist");
    let engine_serve_path = brain_root.join("scripts/launchd/com.brandon.engine-serve.plist");

    let (bastion_serve_raw, engine_serve_raw) = match (
        read_if_present(&bastion_serve_path),
        read_if_present(&engine_serve_path),
    ) {
        (Some(a), Some(b)) => (a, b),
        _ => {
            eprintln!(
                "SKIPPED real_plists_at_hq_root_name_the_same_database_url: \
                     one or both plists not found at {} / {} — expected in hosted CI",
                bastion_serve_path.display(),
                engine_serve_path.display()
            );
            return;
        }
    };

    let bastion_serve_url = extract_database_url(&bastion_serve_raw);
    let engine_serve_url = extract_database_url(&engine_serve_raw);

    assert!(
        database_urls_agree(&bastion_serve_raw, &engine_serve_raw),
        "com.brandon.bastion-serve.plist and com.brandon.engine-serve.plist must name the \
         same DATABASE_URL — got bastion-serve={bastion_serve_url:?}, engine-serve={engine_serve_url:?}"
    );
}

fn read_if_present(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}
