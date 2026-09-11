//! Binary-level contract test for `bastion sweep --once` (`BA.25.D` task 3).
//!
//! Task 1/2 covered `run_sweep_once_at`/`run_drain_once_at` at the function level
//! (`src/sweep_cli.rs`'s own `#[cfg(test)]` module). Task 3's own acceptance criteria require
//! two of those acceptance criteria be exercised again through the REAL CLI entry point —
//! `Commands::Sweep`, parsed by `clap`, dispatched through `main.rs`'s `dispatch` — to confirm
//! the wiring itself (the `cli.rs` variant + the `main.rs` match arm added in this task)
//! doesn't reintroduce a bug task 1/2's own unit tests can't see (a wrong arg forwarded, a
//! flag silently dropped, etc).
//!
//! Same pattern as `tests/notify_cli_contract.rs`: invoke `CARGO_BIN_EXE_bastion` as a real
//! child process, rooted in a fresh temp directory, with `ENGINE_BRAIN_ROOT` pointed at that
//! directory so `engine_core::brain_root::resolve_brain_root()` resolves deterministically
//! without depending on this process's actual cwd or a real `brain.toml` walked up from it.

use std::path::{Path, PathBuf};
use std::process::Command;

const VALID_BRAIN_TOML: &str = r#"
[permission_profiles]
never_allowed = ["clear_operator_gate"]
default = "standard"

[permission_profiles.levels.locked]
id = "locked"
meaning = ""
mini_install = false
main_push = false
cross_repo_write = false

[permission_profiles.levels.standard]
id = "standard"
meaning = ""
mini_install = false
main_push = true
cross_repo_write = true
"#;

/// Build a `Command` for the compiled `bastion` binary, rooted in `dir` and with
/// `ENGINE_BRAIN_ROOT` pointed at `dir` so brain-root resolution is deterministic —
/// independent of this test process's own cwd or any real `brain.toml` above it.
fn bastion_cmd_in(dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_bastion"));
    cmd.current_dir(dir);
    cmd.env("ENGINE_BRAIN_ROOT", dir);
    cmd
}

fn write_valid_brain_toml(root: &Path) {
    std::fs::write(root.join("brain.toml"), VALID_BRAIN_TOML).expect("write brain.toml");
}

fn write_brain_toml_without_unrestricted(root: &Path) {
    // `VALID_BRAIN_TOML` already declares no `unrestricted` level — reused verbatim so this
    // fixture genuinely lacks it, mirroring `src/sweep_cli.rs`'s own unit-level fixture.
    write_valid_brain_toml(root);
}

fn make_roadmap(root: &Path, roadmap: &str) {
    let roadmap_dir = root.join("planning/roadmaps").join(roadmap);
    std::fs::create_dir_all(&roadmap_dir).expect("create roadmap dir");
    std::fs::write(roadmap_dir.join("lane-log.jsonl"), "").expect("write lane-log");
}

fn sweeps_dir(root: &Path, roadmap: &str) -> PathBuf {
    root.join("planning/roadmaps").join(roadmap).join("sweeps")
}

fn count_json_files(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().and_then(|e| e.to_str()) == Some("json"))
        .count()
}

/// Every regular file under `dir`, recursively, as a sorted list of relative paths — enough
/// to assert "nothing new was written" without depending on mtime granularity.
fn file_list(dir: &Path) -> Vec<PathBuf> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

// ── AC: `--profile unrestricted` absent from brain.toml is refused end-to-end ──────────

#[test]
fn sweep_cli_refuses_undeclared_profile_without_touching_sweeps_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_brain_toml_without_unrestricted(dir.path());
    make_roadmap(dir.path(), "demo-roadmap");

    let before = file_list(dir.path());
    let output = bastion_cmd_in(dir.path())
        .args(["sweep", "demo-roadmap", "--profile", "unrestricted"])
        .output()
        .expect("run bastion sweep");

    assert!(
        !output.status.success(),
        "an undeclared --profile must exit non-zero — stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("unrestricted"),
        "error output must name the refused profile — got {combined:?}"
    );

    let after = file_list(dir.path());
    assert_eq!(
        before, after,
        "a refused profile must touch nothing on disk"
    );
    assert!(
        !sweeps_dir(dir.path(), "demo-roadmap").exists(),
        "sweeps_dir must never be created on a refused profile"
    );
}

// ── AC: two `sweep --once` invocations in immediate succession write only once ─────────

#[test]
fn sweep_cli_run_twice_in_succession_writes_only_one_snapshot() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_valid_brain_toml(dir.path());
    make_roadmap(dir.path(), "demo-roadmap");

    let first = bastion_cmd_in(dir.path())
        .args(["sweep", "demo-roadmap"])
        .output()
        .expect("run first bastion sweep");
    assert!(
        first.status.success(),
        "first sweep should succeed — stdout={} stderr={}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    let sweeps = sweeps_dir(dir.path(), "demo-roadmap");
    assert_eq!(
        count_json_files(&sweeps),
        1,
        "first sweep must write exactly one snapshot"
    );

    let second = bastion_cmd_in(dir.path())
        .args(["sweep", "demo-roadmap"])
        .output()
        .expect("run second bastion sweep");
    assert!(
        second.status.success(),
        "second sweep (inside the refire window) should still succeed — stdout={} stderr={}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(
        count_json_files(&sweeps),
        1,
        "a second sweep run immediately after the first (inside the refire window) must \
         write no additional snapshot"
    );
}

// ── AC: `--dry-run` writes nothing through the real CLI entry point either ─────────────

#[test]
fn sweep_cli_dry_run_writes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_valid_brain_toml(dir.path());
    make_roadmap(dir.path(), "demo-roadmap");

    let before = file_list(dir.path());
    let output = bastion_cmd_in(dir.path())
        .args(["sweep", "demo-roadmap", "--dry-run"])
        .output()
        .expect("run bastion sweep --dry-run");
    assert!(
        output.status.success(),
        "--dry-run should succeed — stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let after = file_list(dir.path());
    assert_eq!(
        before, after,
        "--dry-run must perform zero filesystem writes"
    );
    assert!(!sweeps_dir(dir.path(), "demo-roadmap").exists());
}

// ── `bastion drain --once` — malformed --lane is refused before any dispatch ───────────

#[test]
fn drain_cli_rejects_malformed_lane_before_any_dispatch() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_valid_brain_toml(dir.path());

    let output = bastion_cmd_in(dir.path())
        .args(["drain", "not-a-valid-lane"])
        .output()
        .expect("run bastion drain");

    assert!(
        !output.status.success(),
        "a malformed --lane must exit non-zero — stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("not-a-valid-lane"),
        "error output must name the literal malformed value — got {combined:?}"
    );
}
