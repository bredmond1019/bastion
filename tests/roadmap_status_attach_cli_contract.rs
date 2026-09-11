//! Binary-level contract test for `bastion roadmap-status` / `bastion attach` (`BA.25.E`
//! task 3).
//!
//! Tasks 1/2 covered `run_roadmap_status_at`/`attach_lane_at` at the function level
//! (`src/roadmap_status_cli.rs`/`src/sessions/commands.rs`'s own `#[cfg(test)]` modules).
//! Task 3's own acceptance criteria require each be exercised again through the REAL CLI
//! entry point — `Commands::RoadmapStatus`/`Commands::Attach`, parsed by `clap`, dispatched
//! through `main.rs`'s `dispatch` — to confirm the wiring itself (the `cli.rs` variants + the
//! `main.rs` match arms added in this task) doesn't reintroduce a bug task 1/2's own unit
//! tests can't see.
//!
//! Same pattern as `tests/sweep_drain_cli_contract.rs`: invoke `CARGO_BIN_EXE_bastion` as a
//! real child process, rooted in a fresh temp directory, with `ENGINE_BRAIN_ROOT` pointed at
//! that directory so brain-root resolution is deterministic.

use std::path::Path;
use std::process::Command;

/// Build a `Command` for the compiled `bastion` binary, rooted in `dir` and with
/// `ENGINE_BRAIN_ROOT` pointed at `dir` so brain-root resolution is deterministic —
/// independent of this test process's own cwd or any real `brain.toml` above it.
fn bastion_cmd_in(dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_bastion"));
    cmd.current_dir(dir);
    cmd.env("ENGINE_BRAIN_ROOT", dir);
    cmd
}

fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

// ── `bastion roadmap-status --json` — reproduces task 1's malformed_lines output ───────────

#[test]
fn roadmap_status_cli_json_reports_malformed_lines_end_to_end() {
    let dir = tempfile::tempdir().expect("tempdir");
    let roadmap_dir = dir.path().join("planning/roadmaps/demo");
    std::fs::create_dir_all(&roadmap_dir).expect("create roadmap dir");
    let good_line = r#"{"repo":"engine-rs","lane":"a","block":"EN.1.A","status":"done"}"#;
    let bad_line = "{not valid json";
    std::fs::write(
        roadmap_dir.join("lane-log.jsonl"),
        format!("{good_line}\n{bad_line}\n"),
    )
    .expect("write lane-log");

    let output = bastion_cmd_in(dir.path())
        .args(["roadmap-status", "--roadmap", "demo", "--json"])
        .output()
        .expect("run bastion roadmap-status");

    assert!(
        output.status.success(),
        "roadmap-status --json should succeed — {}",
        combined(&output)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid json");
    let malformed = parsed
        .get("malformed_lines")
        .expect("malformed_lines field present")
        .as_array()
        .expect("malformed_lines is an array");
    assert_eq!(
        malformed.len(),
        1,
        "expected exactly one malformed line — got {stdout}"
    );
}

#[test]
fn roadmap_status_cli_unknown_roadmap_exits_nonzero_naming_slug() {
    let dir = tempfile::tempdir().expect("tempdir");

    let output = bastion_cmd_in(dir.path())
        .args(["roadmap-status", "--roadmap", "nonexistent-slug"])
        .output()
        .expect("run bastion roadmap-status");

    assert!(
        !output.status.success(),
        "an unknown roadmap slug must exit non-zero — {}",
        combined(&output)
    );
    assert!(
        combined(&output).contains("nonexistent-slug"),
        "error output must name the literal slug — got {:?}",
        combined(&output)
    );
}

// ── `bastion attach` — malformed lane is refused before any file I/O ───────────────────────

#[test]
fn attach_cli_rejects_a_value_with_no_slash_before_any_file_io() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Deliberately no brain.toml, no lock dir, nothing on disk for the lane resolution to
    // read — if the parse check ran after any file I/O this would fail differently (a
    // brain-root/lock-dir resolution error) instead of naming the literal argument.

    let output = bastion_cmd_in(dir.path())
        .args(["attach", "not-a-repo-slash-lane"])
        .output()
        .expect("run bastion attach");

    assert!(
        !output.status.success(),
        "a malformed attach argument must exit non-zero — {}",
        combined(&output)
    );
    assert!(
        combined(&output).contains("not-a-repo-slash-lane"),
        "error output must name the literal malformed value — got {:?}",
        combined(&output)
    );
}

#[test]
fn attach_cli_rejects_a_value_with_more_than_one_slash() {
    let dir = tempfile::tempdir().expect("tempdir");

    let output = bastion_cmd_in(dir.path())
        .args(["attach", "a/b/c"])
        .output()
        .expect("run bastion attach");

    assert!(
        !output.status.success(),
        "an attach argument with more than one slash must exit non-zero — {}",
        combined(&output)
    );
    assert!(
        combined(&output).contains("a/b/c"),
        "error output must name the literal malformed value — got {:?}",
        combined(&output)
    );
}
