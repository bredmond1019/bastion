//! BA.ticket.notify-operator-cli task 6: binary-level contract test for
//! `bastion notify send|ask`'s network-free paths (D64 evidence task).
//!
//! Invokes the ACTUAL COMPILED BINARY (`CARGO_BIN_EXE_bastion`) as a separate process —
//! per D64, the exit-code contract and the pre-flight (config/portability) rejections are
//! only observable at that boundary, not from `src/notify_cli.rs`'s unit tests, which call
//! the pure functions directly and never go through `clap` or `main`'s dispatch.
//!
//! SCOPE NOTE (stated explicitly per D64, and per this block's own task-6 description):
//! this test closes exactly two things — the exit-code/stderr contract for an unconfigured
//! or unknown `--bot`, and the pre-flight portability rejection (over-limit option count,
//! over-length label) firing before any network attempt. **It does NOT close, and does not
//! claim to close, the live Telegram round trip** (a real send + a real operator tap
//! resolving `notify ask`) — that remainder is exactly one un-gateable thing, run by hand
//! once against `--bot codesessions` per the task's own instructions, and recorded in this
//! spec's task notes rather than in `harness.json`.
//!
//! Every case here:
//!   - runs the child process in a freshly created temp directory, because `dotenvy::dotenv()`
//!     (called from `src/config.rs`) loads a `.env` from the process's CURRENT DIRECTORY —
//!     running from this repo's own working directory would let a real `.env` (with real
//!     bot credentials) make an "unconfigured" case pass for the wrong reason;
//!   - explicitly clears the specific env vars a case depends on being absent via
//!     `.env_remove`, because a developer's shell may export them ambiently, which a temp
//!     working directory alone would not hide (only the `.env`-file load is directory-scoped).

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

/// A value that must never appear in any stdout/stderr this test asserts against — stands
/// in for a real bot token so a redaction regression is directly observable.
const DUMMY_TOKEN: &str = "dummy-token-should-never-be-printed-12345";
const DUMMY_CHAT_ID: &str = "dummy-chat-id-67890";

/// Build a `Command` for the compiled binary, rooted in a fresh temp directory so an
/// ambient `.env` in this repo cannot leak real credentials into an "unconfigured" case.
fn bastion_cmd_in(dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_bastion"));
    cmd.current_dir(dir);
    cmd
}

fn assert_no_secret_leak(stdout: &str, stderr: &str) {
    assert!(
        !stdout.contains(DUMMY_TOKEN) && !stderr.contains(DUMMY_TOKEN),
        "output leaked the dummy token value — stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        !stdout.contains(DUMMY_CHAT_ID) && !stderr.contains(DUMMY_CHAT_ID),
        "output leaked the dummy chat id value — stdout={stdout:?} stderr={stderr:?}"
    );
}

// ── Unconfigured default bot (`lane`) — both verbs ──────────────────────────

#[test]
fn ask_with_unconfigured_lane_bot_exits_1_names_both_vars_empty_stdout() {
    let dir = tempfile::tempdir().expect("tempdir");
    let output = bastion_cmd_in(dir.path())
        .args([
            "notify",
            "ask",
            "--gate-id",
            "g",
            "--summary",
            "s",
            "--option",
            "a:A",
            "--option",
            "b:B",
        ])
        .env_remove("BASTION_LANE_BOT_TOKEN")
        .env_remove("BASTION_LANE_CHAT_ID")
        .output()
        .expect("failed to spawn bastion binary");

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.is_empty(), "stdout should be empty, got {stdout:?}");
    assert!(
        stderr.contains("BASTION_LANE_BOT_TOKEN"),
        "stderr should name BASTION_LANE_BOT_TOKEN: {stderr:?}"
    );
    assert!(
        stderr.contains("BASTION_LANE_CHAT_ID"),
        "stderr should name BASTION_LANE_CHAT_ID: {stderr:?}"
    );
    assert_no_secret_leak(&stdout, &stderr);
}

#[test]
fn send_with_unconfigured_lane_bot_exits_1_names_both_vars_empty_stdout() {
    let dir = tempfile::tempdir().expect("tempdir");
    let output = bastion_cmd_in(dir.path())
        .args(["notify", "send", "--text", "hi"])
        .env_remove("BASTION_LANE_BOT_TOKEN")
        .env_remove("BASTION_LANE_CHAT_ID")
        .output()
        .expect("failed to spawn bastion binary");

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.is_empty(), "stdout should be empty, got {stdout:?}");
    assert!(
        stderr.contains("BASTION_LANE_BOT_TOKEN"),
        "stderr should name BASTION_LANE_BOT_TOKEN: {stderr:?}"
    );
    assert!(
        stderr.contains("BASTION_LANE_CHAT_ID"),
        "stderr should name BASTION_LANE_CHAT_ID: {stderr:?}"
    );
    assert_no_secret_leak(&stdout, &stderr);
}

// ── Portability gate fires before any network attempt ───────────────────────

#[test]
fn ask_with_too_many_options_is_rejected_before_network() {
    let dir = tempfile::tempdir().expect("tempdir");
    let start = Instant::now();
    let output = bastion_cmd_in(dir.path())
        .args([
            "notify",
            "ask",
            "--gate-id",
            "g",
            "--summary",
            "s",
            "--option",
            "a:A",
            "--option",
            "b:B",
            "--option",
            "c:C",
            "--option",
            "d:D",
        ])
        // Set to obvious dummy values: validation runs before credentials are ever read,
        // so this must reject on option count alone, without attempting a connection.
        .env("BASTION_LANE_BOT_TOKEN", DUMMY_TOKEN)
        .env("BASTION_LANE_CHAT_ID", DUMMY_CHAT_ID)
        .output()
        .expect("failed to spawn bastion binary");
    let elapsed = start.elapsed();

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("options") && stderr.contains('4') && stderr.contains('3'),
        "stderr should name the option count (4) against the max (3): {stderr:?}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "rejection should be prompt (no network attempt), took {elapsed:?}"
    );
    assert_no_secret_leak(&stdout, &stderr);
}

#[test]
fn ask_with_over_length_label_is_rejected_before_network() {
    let dir = tempfile::tempdir().expect("tempdir");
    let long_label = "a".repeat(21);
    let start = Instant::now();
    let output = bastion_cmd_in(dir.path())
        .args([
            "notify",
            "ask",
            "--gate-id",
            "g",
            "--summary",
            "s",
            "--option",
            &format!("a:{long_label}"),
            "--option",
            "b:B",
        ])
        .env("BASTION_LANE_BOT_TOKEN", DUMMY_TOKEN)
        .env("BASTION_LANE_CHAT_ID", DUMMY_CHAT_ID)
        .output()
        .expect("failed to spawn bastion binary");
    let elapsed = start.elapsed();

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("21") && stderr.contains("characters"),
        "stderr should name the label length (21): {stderr:?}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "rejection should be prompt (no network attempt), took {elapsed:?}"
    );
    assert_no_secret_leak(&stdout, &stderr);
}

// ── `--bot` cases (operator amendment, 2026-08-23) ───────────────────────────

#[test]
fn send_with_unknown_bot_slug_exits_1_names_both_derived_vars_and_lists_configured() {
    let dir = tempfile::tempdir().expect("tempdir");
    let output = bastion_cmd_in(dir.path())
        .args([
            "notify",
            "send",
            "--text",
            "hi",
            "--bot",
            "totally-unknown-slug",
        ])
        .output()
        .expect("failed to spawn bastion binary");

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.is_empty(), "stdout should be empty, got {stdout:?}");
    assert!(
        stderr.contains("BASTION_TOTALLY-UNKNOWN-SLUG_BOT_TOKEN"),
        "stderr should name the derived token var: {stderr:?}"
    );
    assert!(
        stderr.contains("BASTION_TOTALLY-UNKNOWN-SLUG_CHAT_ID"),
        "stderr should name the derived chat-id var: {stderr:?}"
    );
    assert!(
        stderr.contains("configured"),
        "stderr should mention configured bots (empty or a list): {stderr:?}"
    );
    assert_no_secret_leak(&stdout, &stderr);
}

#[test]
fn send_with_unconfigured_known_bot_slug_exits_1_names_both_derived_vars() {
    let dir = tempfile::tempdir().expect("tempdir");
    let output = bastion_cmd_in(dir.path())
        .args(["notify", "send", "--text", "hi", "--bot", "codesessions"])
        .env_remove("BASTION_CODESESSIONS_BOT_TOKEN")
        .env_remove("BASTION_CODESESSIONS_CHAT_ID")
        .output()
        .expect("failed to spawn bastion binary");

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.is_empty(), "stdout should be empty, got {stdout:?}");
    assert!(
        stderr.contains("BASTION_CODESESSIONS_BOT_TOKEN"),
        "stderr should name the token var: {stderr:?}"
    );
    assert!(
        stderr.contains("BASTION_CODESESSIONS_CHAT_ID"),
        "stderr should name the chat-id var: {stderr:?}"
    );
    assert_no_secret_leak(&stdout, &stderr);
}

#[test]
fn ask_with_no_bot_flag_defaults_to_lane() {
    // No --bot at all — the default slug is `lane`, so the error must name the
    // BASTION_LANE_* pair even though it was never mentioned on the command line.
    let dir = tempfile::tempdir().expect("tempdir");
    let output = bastion_cmd_in(dir.path())
        .args([
            "notify",
            "ask",
            "--gate-id",
            "g",
            "--summary",
            "s",
            "--option",
            "a:A",
            "--option",
            "b:B",
        ])
        .env_remove("BASTION_LANE_BOT_TOKEN")
        .env_remove("BASTION_LANE_CHAT_ID")
        .output()
        .expect("failed to spawn bastion binary");

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("BASTION_LANE_BOT_TOKEN") && stderr.contains("BASTION_LANE_CHAT_ID"),
        "no --bot flag should default to the `lane` slug: {stderr:?}"
    );
}
