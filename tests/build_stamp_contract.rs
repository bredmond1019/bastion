//! BA.ticket.build-stamp-for-corpus-writer task 4: end-to-end contract test for
//! `bastion --build-stamp`.
//!
//! Invokes the ACTUAL COMPILED BINARY (`CARGO_BIN_EXE_bastion`) as a separate process and
//! parses its stdout as JSON. This exists per D64: the acceptance criterion "`bastion
//! --build-stamp` emits parseable JSON" has its evidence in a built artefact invoked as a
//! separate process, which in-language unit tests (`src/buildstamp.rs`) structurally cannot
//! observe — those tests can assert the serialiser's output while the `--build-stamp` flag
//! itself is unreachable through clap. `CARGO_BIN_EXE_bastion` is what pulls that evidence
//! back inside `cargo test`, converting an otherwise un-gateable criterion into a gated one.
//!
//! SCOPE NOTE (stated explicitly per D64): this test checks the binary **cargo just built
//! from source** for this test run, NOT the copy installed on PATH or on the Mac Mini. The
//! installed-vs-source divergence is precisely the incident this whole ticket is about, and
//! no in-repo check can close that gap — mev's side
//! (`mev:MV.ticket.toolchain-freshness-covers-the-writer`) is what queries registered
//! writers on the machine, and it is out of scope here.

use std::path::Path;
use std::process::Command;

use serde_json::Value;

/// Runs the source-built `bastion` binary with `--build-stamp` and asserts the
/// `{git_sha, dirty, source_dir}` JSON contract end to end.
///
/// See the module doc comment: this verifies the binary cargo just compiled from THIS
/// source tree for this test run, not any binary installed on PATH or deployed elsewhere.
#[test]
fn build_stamp_emits_parseable_three_key_contract() {
    let bin = env!("CARGO_BIN_EXE_bastion");

    let output = Command::new(bin)
        .arg("--build-stamp")
        .output()
        .expect("failed to spawn the compiled bastion binary");

    assert!(
        output.status.success(),
        "bastion --build-stamp exited non-zero: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout was not valid UTF-8");
    let value: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout was not parseable JSON: {e}\nstdout={stdout:?}"));

    let obj = value
        .as_object()
        .expect("--build-stamp output must be a JSON object");

    // Exactly the pinned three-key contract with mev's toolchain-freshness check — not a
    // superset, not a subset.
    let mut keys: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec!["dirty", "git_sha", "source_dir"],
        "the --build-stamp JSON contract must be exactly {{git_sha, dirty, source_dir}}"
    );

    // git_sha: 40 hex chars, or the literal "unknown" — never empty, never guessed.
    let git_sha = obj
        .get("git_sha")
        .and_then(Value::as_str)
        .expect("git_sha must be a JSON string");
    assert!(!git_sha.is_empty(), "git_sha must not be empty");
    let is_40_hex = git_sha.len() == 40 && git_sha.chars().all(|c| c.is_ascii_hexdigit());
    assert!(
        is_40_hex || git_sha == "unknown",
        "git_sha must be 40 hex chars or the literal \"unknown\", got {git_sha:?}"
    );

    // dirty: a JSON boolean, or the literal string "unknown" — never anything else.
    let dirty = obj.get("dirty").expect("dirty key must be present");
    let dirty_is_valid = dirty.is_boolean() || dirty.as_str() == Some("unknown");
    assert!(
        dirty_is_valid,
        "dirty must be a JSON boolean or the literal string \"unknown\", got {dirty:?}"
    );

    // source_dir: must point at a directory that actually exists.
    let source_dir = obj
        .get("source_dir")
        .and_then(Value::as_str)
        .expect("source_dir must be a JSON string");
    assert!(
        Path::new(source_dir).is_dir(),
        "source_dir must point at a directory that exists, got {source_dir:?}"
    );
}
