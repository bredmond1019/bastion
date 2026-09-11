//! `bastion sweep --once --roadmap <slug> [--dry-run] [--profile <name>]` — CLI face for
//! engine-core's `SWEEP` workflow (`BA.25.D` task 1).
//!
//! This module owns NO sweep pipeline logic of its own — every measurement, diff, and routing
//! step below calls the exact same public functions
//! `engine_core::workflows::sweep::run_sweep_pass` itself composes
//! (`crates/engine-core/src/workflows/sweep/mod.rs`, its own doc comment numbers the 7 steps):
//! [`build_raw_snapshot`], [`list_snapshot_files`]/[`load_snapshot`], [`diff_snapshots`],
//! [`build_dedup_history`], [`route_escalation`], [`route_non_escalation_diff`]. The composition
//! is reproduced here — not delegated straight to `run_sweep_pass` — for exactly one reason:
//! `run_sweep_pass` always performs its own final `sweeps_dir/<ts>.json` write and takes no
//! `dry_run` flag at all (its own doc comment says so explicitly: "minus the Python's dry_run
//! flag, out of scope here — the CLI face, BA.25.D"). `--dry-run` on this CLI is therefore
//! implemented by skipping ONLY that final write, never by writing to a temp path and deleting it
//! afterward.
//!
//! ## The profile is read here and passed through — never defaulted to `Unrestricted`
//!
//! The permission-profile resolution helper (`resolve_requested_profile`) moved to
//! [`crate::permission_profile`] in task 2, so `drain_cli.rs` can share the exact same
//! resolution logic rather than duplicating it — see that module's own doc comment for the
//! full rationale (AC-3's REFUSAL requirement, why the `engine_core::policy::permission`
//! resolvers are the wrong fit here).
//!
//! ## The refire window also gates whether anything is written at all
//!
//! Beyond the `--dry-run` gate, this CLI never writes a second snapshot when the most recently
//! written one is younger than [`DEFAULT_REFIRE_HOURS`] — an operator hand-firing `sweep --once`
//! repeatedly in quick succession (there is no schedule in this cut, Fork 4) must not spam
//! `sweeps_dir` with a near-duplicate file every time. `run_sweep_pass` itself has no such gate
//! (every dispatch through the registered workflow always writes); this is this CLI's own
//! addition, checked BEFORE any measurement or routing runs.
//!
//! ## What this module does NOT do
//!
//! No `Commands::Sweep` CLI variant is wired here — that is task 3's job
//! (`planning/BA.25.D/tasks.json`). This module is dispatched to, not dispatched from.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;

use engine_core::workflows::sweep::{
    Budget, DEFAULT_REFIRE_HOURS, DedupEntry, EscalationKey, NoopLaneWake, NoopOperatorTransport,
    RouteInputs, RouteOutcome, build_dedup_history, build_raw_snapshot, diff_snapshots,
    escalation_key, list_snapshot_files, load_snapshot, parse_iso, route_escalation,
    route_non_escalation_diff, snapshot_filename, sweeps_dir,
};

use crate::permission_profile::resolve_requested_profile;

/// `bastion sweep --once --roadmap <slug> [--dry-run] [--profile <name>]` — resolve `brain_root`
/// exactly as every other bastion write verb does
/// (`engine_core::brain_root::resolve_brain_root()`) and dispatch to [`run_sweep_once_at`] with
/// the real current instant.
pub async fn run_sweep_once(roadmap: &str, dry_run: bool, profile: Option<&str>) -> Result<()> {
    let brain_root =
        engine_core::brain_root::resolve_brain_root().context("cannot resolve brain root")?;
    run_sweep_once_at(&brain_root, roadmap, dry_run, profile, Utc::now()).await
}

/// `run_sweep_once` against an already-resolved `root` and an explicit `now` — the pure(ish) core
/// [`run_sweep_once`] wraps, mirroring `coord_cli.rs`'s `run_status`/`view_for` split so a test
/// can drive this against a `tempdir()` fixture at a deterministic instant.
///
/// Resolves the permission profile FIRST via [`resolve_requested_profile`] — a refusal returns
/// `Err` before `root`'s `brain.toml` is used for anything else and before any snapshot
/// measurement or write happens. Then, if the most recently written snapshot under
/// `sweeps_dir(root, roadmap)` is younger than [`DEFAULT_REFIRE_HOURS`], returns `Ok(())` having
/// written nothing at all (the refire-window gate, this module's own doc comment). Otherwise
/// reproduces `run_sweep_pass`'s own 7-step composition, gating only the final
/// `sweeps_dir/<ts>.json` write on `!dry_run`.
pub async fn run_sweep_once_at(
    root: &Path,
    roadmap: &str,
    dry_run: bool,
    profile: Option<&str>,
    now: DateTime<Utc>,
) -> Result<()> {
    let brain_toml_path = root.join("brain.toml");
    let config = mev::brain::config::load_brain_config(&brain_toml_path)
        .with_context(|| format!("failed to load brain.toml at {}", brain_toml_path.display()))?;
    let profile = resolve_requested_profile(&config.permission_profiles, profile)
        .map_err(|message| anyhow::anyhow!(message))?;

    let sweeps_directory = sweeps_dir(root, roadmap);
    let prev_files = list_snapshot_files(&sweeps_directory);
    let prev_snapshot = prev_files.last().and_then(|path| load_snapshot(path));

    if let Some(prev) = &prev_snapshot
        && let Some(last_ts) = parse_iso(Some(&prev.ts_utc))
    {
        let age_hours = (now - last_ts).num_seconds() as f64 / 3600.0;
        if age_hours < DEFAULT_REFIRE_HOURS {
            println!(
                "bastion sweep: 0 routing decision(s) for '{roadmap}'\n\
                 bastion sweep: nothing written — last snapshot at {} is only {age_hours:.2}h \
                 old (refire window is {DEFAULT_REFIRE_HOURS:.1}h)",
                prev.ts_utc
            );
            return Ok(());
        }
    }

    let raw = build_raw_snapshot(root, roadmap, now)?;
    let diff = diff_snapshots(prev_snapshot.as_ref(), &raw);
    let mut dedup_history = build_dedup_history(&sweeps_directory, None);

    let inputs = RouteInputs {
        now,
        current_sha: raw.git_sha.as_deref(),
        refire_hours: DEFAULT_REFIRE_HOURS,
        profile,
    };
    let mut budget = Budget::default();
    let mut routed: Vec<RouteOutcome> = Vec::new();
    let transport = NoopOperatorTransport;
    let waker = NoopLaneWake;

    // --- 1. every NEW escalation ----------------------------------------------------------
    for escalation in &diff.new_escalations {
        let result = route_escalation(
            escalation,
            &dedup_history,
            inputs,
            &mut budget,
            &transport,
            &waker,
        )
        .await;
        if result.routed
            && let Some(gate_id) = escalation.get("gate_id").and_then(Value::as_str)
        {
            dedup_history.insert(
                gate_id.to_string(),
                DedupEntry {
                    ts: result.ts_routed.clone(),
                    severity: result.severity.clone(),
                },
            );
        }
        routed.push(result);
    }

    // --- 2. STANDING escalations past their re-fire threshold ------------------------------
    let new_keys: HashSet<EscalationKey> =
        diff.new_escalations.iter().map(escalation_key).collect();
    for escalation in &raw.escalations {
        if new_keys.contains(&escalation_key(escalation)) {
            continue;
        }
        let Some(gate_id) = escalation.get("gate_id").and_then(Value::as_str) else {
            continue;
        };
        if !dedup_history.contains_key(gate_id) {
            continue;
        }
        let result = route_escalation(
            escalation,
            &dedup_history,
            inputs,
            &mut budget,
            &transport,
            &waker,
        )
        .await;
        if result.action == "skip-dedup" {
            continue;
        }
        if result.routed {
            dedup_history.insert(
                gate_id.to_string(),
                DedupEntry {
                    ts: result.ts_routed.clone(),
                    severity: result.severity.clone(),
                },
            );
        }
        routed.push(result);
    }

    // --- 3. bare non-escalation drift, at most once, only if nothing else routed -----------
    if routed.is_empty() && diff.non_escalation_diff {
        routed.push(route_non_escalation_diff(
            &diff, roadmap, now, profile, &waker,
        ));
    }

    let decision_count = routed.len();

    if dry_run {
        println!(
            "bastion sweep: {decision_count} routing decision(s) for '{roadmap}'\n\
             bastion sweep: nothing written (--dry-run)"
        );
        return Ok(());
    }

    let mut document = serde_json::to_value(&raw).context("failed to serialize raw snapshot")?;
    if let Value::Object(map) = &mut document {
        map.insert(
            "diff".to_string(),
            serde_json::to_value(&diff).context("failed to serialize diff")?,
        );
        map.insert(
            "routed".to_string(),
            serde_json::to_value(&routed).context("failed to serialize routed decisions")?,
        );
    }

    fs::create_dir_all(&sweeps_directory).with_context(|| {
        format!(
            "failed to create sweeps directory {}",
            sweeps_directory.display()
        )
    })?;
    let out_path: PathBuf = sweeps_directory.join(snapshot_filename(&raw.ts_utc));
    let rendered =
        serde_json::to_string_pretty(&document).context("failed to render composed snapshot")?;
    fs::write(&out_path, format!("{rendered}\n"))
        .with_context(|| format!("failed to write {}", out_path.display()))?;

    println!(
        "bastion sweep: {decision_count} routing decision(s) for '{roadmap}'\n\
         bastion sweep: wrote {}",
        out_path.display()
    );
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // `resolve_requested_profile`'s own unit tests moved to `crate::permission_profile` in task
    // 2, alongside the function itself — no duplicate coverage kept here.

    // ── fixture plumbing shared by the run_sweep_once_at tests ──────────────────────

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

[permission_profiles.levels.unrestricted]
id = "unrestricted"
meaning = ""
mini_install = true
main_push = true
cross_repo_write = true
"#;

    fn write_valid_brain_toml(root: &Path) {
        std::fs::write(root.join("brain.toml"), VALID_BRAIN_TOML).expect("write brain.toml");
    }

    fn make_roadmap(root: &Path, roadmap: &str) -> PathBuf {
        let roadmap_dir = root.join("planning/roadmaps").join(roadmap);
        std::fs::create_dir_all(&roadmap_dir).expect("create roadmap dir");
        std::fs::write(roadmap_dir.join("lane-log.jsonl"), "").expect("write lane-log");
        roadmap_dir
    }

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-08T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    /// Every regular file under `dir`, recursively, as `(relative path, size, mtime)` — a
    /// structural snapshot of the tree used to prove `--dry-run` performs zero writes.
    fn tree_fingerprint(dir: &Path) -> Vec<(PathBuf, u64, std::time::SystemTime)> {
        fn walk(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, u64, std::time::SystemTime)>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(root, &path, out);
                } else if let Ok(meta) = entry.metadata() {
                    out.push((
                        path.strip_prefix(root).unwrap_or(&path).to_path_buf(),
                        meta.len(),
                        meta.modified().unwrap_or(std::time::UNIX_EPOCH),
                    ));
                }
            }
        }
        let mut out = Vec::new();
        walk(dir, dir, &mut out);
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    // ── AC-3: --profile unrestricted absent from brain.toml is refused end-to-end ──

    #[tokio::test]
    async fn run_sweep_once_at_refuses_an_undeclared_profile_before_touching_sweeps_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut config_toml = VALID_BRAIN_TOML.to_string();
        // Strip the `unrestricted` level entirely so the fixture genuinely lacks it.
        let cut = config_toml
            .find("[permission_profiles.levels.unrestricted]")
            .unwrap();
        config_toml.truncate(cut);
        std::fs::write(tmp.path().join("brain.toml"), config_toml).expect("write brain.toml");
        make_roadmap(tmp.path(), "demo-roadmap");

        let before = tree_fingerprint(tmp.path());
        let err = run_sweep_once_at(
            tmp.path(),
            "demo-roadmap",
            false,
            Some("unrestricted"),
            fixed_now(),
        )
        .await
        .expect_err("an undeclared --profile must be refused");
        assert!(err.to_string().contains("unrestricted"));

        let after = tree_fingerprint(tmp.path());
        assert_eq!(
            before, after,
            "a refused profile must touch nothing on disk"
        );
        assert!(
            !sweeps_dir(tmp.path(), "demo-roadmap").exists(),
            "sweeps_dir must never be created on a refused profile"
        );
    }

    // ── AC: --dry-run performs zero filesystem writes ───────────────────────────────

    #[tokio::test]
    async fn dry_run_writes_nothing_at_all() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_valid_brain_toml(tmp.path());
        make_roadmap(tmp.path(), "demo-roadmap");

        let before = tree_fingerprint(tmp.path());
        run_sweep_once_at(tmp.path(), "demo-roadmap", true, None, fixed_now())
            .await
            .expect("dry-run sweep should succeed");
        let after = tree_fingerprint(tmp.path());

        assert_eq!(
            before, after,
            "--dry-run must perform zero filesystem writes"
        );
        assert!(
            !sweeps_dir(tmp.path(), "demo-roadmap").exists(),
            "--dry-run must never create sweeps_dir"
        );
    }

    /// A real (non-dry-run) sweep, by contrast, does write exactly one snapshot — the control
    /// proving `dry_run_writes_nothing_at_all` above is a meaningful negative, not an artifact of
    /// a broken fixture.
    #[tokio::test]
    async fn non_dry_run_writes_exactly_one_snapshot() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_valid_brain_toml(tmp.path());
        make_roadmap(tmp.path(), "demo-roadmap");

        run_sweep_once_at(tmp.path(), "demo-roadmap", false, None, fixed_now())
            .await
            .expect("sweep should succeed");

        let files = list_snapshot_files(&sweeps_dir(tmp.path(), "demo-roadmap"));
        assert_eq!(files.len(), 1, "exactly one snapshot must be written");
    }

    // ── AC: refire-window gate over a fixture seeded with real prior snapshots ──────

    /// Locate the sibling `engine-rs` checkout's own `SWEEP` golden-replay fixtures
    /// (`tests/fixtures/sweep_snapshots/`, 13 files — `EN.15.E` task 6) without copying a 7+MB
    /// fixture tree into this repo. Skips loudly (never silently passes) when this checkout has
    /// no sibling `engine-rs` to find them in, mirroring `coord_cli.rs`'s own
    /// `find_oracle_script` pattern for a sibling-repo fixture dependency.
    fn find_real_sweep_fixtures_dir() -> Option<PathBuf> {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let Some(core_dir) = manifest_dir.parent() else {
            eprintln!(
                "SKIPPING sweep refire-window test: {} has no parent",
                manifest_dir.display()
            );
            return None;
        };
        let candidate =
            core_dir.join("engine-rs/crates/engine-core/tests/fixtures/sweep_snapshots");
        if candidate.is_dir() {
            Some(candidate)
        } else {
            eprintln!(
                "SKIPPING sweep refire-window test: no sibling engine-rs checkout found at {}",
                candidate.display()
            );
            None
        }
    }

    fn seed_sweeps_dir_from_real_fixtures(sweeps_directory: &Path, source: &Path) -> usize {
        std::fs::create_dir_all(sweeps_directory).expect("create sweeps_dir");
        let mut count = 0;
        for entry in std::fs::read_dir(source)
            .expect("read fixture dir")
            .flatten()
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let bytes = std::fs::read(&path).expect("read fixture file");
            std::fs::write(
                sweeps_directory.join(path.file_name().expect("fixture has a filename")),
                bytes,
            )
            .expect("write fixture copy into sweeps_dir");
            count += 1;
        }
        count
    }

    /// AC-4/AC-5 (block record acceptance criteria): with the 13 existing snapshots as the tree,
    /// a second `sweep --once` INSIDE the refire window writes no snapshot; outside the refire
    /// window it writes exactly one.
    #[tokio::test]
    async fn refire_window_gates_whether_a_new_snapshot_is_written() {
        let Some(source) = find_real_sweep_fixtures_dir() else {
            return;
        };

        let tmp = tempfile::tempdir().expect("tempdir");
        write_valid_brain_toml(tmp.path());
        let roadmap = "autonomous-foundation";
        make_roadmap(tmp.path(), roadmap);
        let sweeps_directory = sweeps_dir(tmp.path(), roadmap);
        let seeded = seed_sweeps_dir_from_real_fixtures(&sweeps_directory, &source);
        assert_eq!(
            seeded, 13,
            "expected exactly the 13 checked-in golden fixtures"
        );

        // The most recent real fixture's own ts_utc is 2026-08-28T07:01:10Z (sorted last by
        // filename). A `now` only 1h later is well inside DEFAULT_REFIRE_HOURS (6.0).
        let inside_window = DateTime::parse_from_rfc3339("2026-08-28T08:01:10Z")
            .unwrap()
            .with_timezone(&Utc);
        run_sweep_once_at(tmp.path(), roadmap, false, None, inside_window)
            .await
            .expect("sweep inside the refire window should succeed");
        assert_eq!(
            list_snapshot_files(&sweeps_directory).len(),
            13,
            "a sweep inside the refire window must write no new snapshot"
        );

        // 7h after the last fixture's ts_utc is outside the 6h window.
        let outside_window = DateTime::parse_from_rfc3339("2026-08-28T14:01:11Z")
            .unwrap()
            .with_timezone(&Utc);
        run_sweep_once_at(tmp.path(), roadmap, false, None, outside_window)
            .await
            .expect("sweep outside the refire window should succeed");
        assert_eq!(
            list_snapshot_files(&sweeps_directory).len(),
            14,
            "a sweep outside the refire window must write exactly one new snapshot"
        );
    }

    // ── AC-5: outside the refire window, the write matches `roadmap_sweep.py --dry-run` ──
    //
    // The block record's acceptance criterion (and this task's own `testing_strategy`) requires
    // the write outside the refire window to match what the Python oracle reports for routes on
    // the IDENTICAL fixture tree — not merely that a write happened (that half is already covered
    // by `refire_window_gates_whether_a_new_snapshot_is_written` above). This shells out to the
    // real `scripts/roadmap_sweep.py --dry-run` (mirroring `coord_cli.rs`'s own
    // `find_oracle_script`/`python3_available` pattern for a sibling-repo oracle) against a
    // byte-for-byte copy of the same 13-seeded-snapshot tree, then compares the DECISION fields
    // that matter — `action`/`channel`/`severity`/`stale`/`gate_id` — never a byte-for-byte JSON
    // diff: `engine-rs`'s own `sweep_replay.rs` (`EN.15.E` task 6) already documents why a
    // byte-for-byte compare is the wrong bar (the Python's dry-run dict carries `dry_run`/
    // `summary` keys `RouteOutcome` has no slot for, and vice versa `ts_routed`/`routed`(bool)
    // are real-clock-dependent on the Python side since its CLI has no `--now` override).

    /// The decision-relevant slice of one `routed[]` entry, exactly `sweep_replay.rs`'s own
    /// `Decision` shape — kept as a **local, second, independently-written** copy rather than a
    /// shared import: this crate has no dependency on `engine-core`'s `tests/` directory (it is
    /// not a library target), and re-deriving the same five fields here is one Decision struct
    /// two crates would otherwise have to keep in lockstep for no reuse benefit.
    #[derive(Debug, PartialEq)]
    struct SweepDecision {
        action: Option<String>,
        channel: Option<String>,
        severity: Option<String>,
        stale: Option<bool>,
        gate_id: Option<String>,
    }

    /// `route_non_escalation_diff`'s Rust sentinel gate_id — the Python dict for that same route
    /// has no `gate_id`/`stale` key at all (that route is about the diff as a whole, not one
    /// escalation), so a JSON entry carrying this sentinel treats both as absent, exactly
    /// `sweep_replay.rs`'s own `decision_from_outcome` does for the struct side.
    const NON_ESCALATION_SENTINEL: &str = "<non-escalation-diff>";

    fn decision_from_json(entry: &Value) -> SweepDecision {
        let gate_id = entry.get("gate_id").and_then(Value::as_str);
        let is_non_escalation = gate_id == Some(NON_ESCALATION_SENTINEL);
        SweepDecision {
            action: entry
                .get("action")
                .and_then(Value::as_str)
                .map(String::from),
            channel: entry
                .get("channel")
                .and_then(Value::as_str)
                .map(String::from),
            severity: entry
                .get("severity")
                .and_then(Value::as_str)
                .map(String::from),
            stale: (!is_non_escalation)
                .then(|| entry.get("stale").and_then(Value::as_bool))
                .flatten(),
            gate_id: (!is_non_escalation)
                .then(|| gate_id.map(String::from))
                .flatten(),
        }
    }

    /// Walk up from `start` looking for `brain.toml` — the company-brain vault root that houses
    /// both the sibling `engine-rs` fixtures and the oracle script itself
    /// (`scripts/roadmap_sweep.py`).
    fn find_brain_root(start: &Path) -> Option<PathBuf> {
        crate::config::walk_up_from(start, "brain.toml")
            .and_then(|toml| toml.parent().map(Path::to_path_buf))
    }

    /// `<brain_root>/scripts/roadmap_sweep.py`, or `None` (after a loud `eprintln!`) when this
    /// checkout has no sibling brain root to find it in.
    fn find_oracle_script() -> Option<PathBuf> {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let Some(brain_root) = find_brain_root(manifest_dir) else {
            eprintln!(
                "SKIPPING sweep parity test: no brain.toml found walking up from {} \
                 (this checkout has no sibling brain root to locate the oracle in)",
                manifest_dir.display()
            );
            return None;
        };
        let script = brain_root.join("scripts").join("roadmap_sweep.py");
        if !script.is_file() {
            eprintln!(
                "SKIPPING sweep parity test: brain root found at {} but {} does not exist",
                brain_root.display(),
                script.display()
            );
            return None;
        }
        Some(script)
    }

    /// `true` iff `python3` is on `PATH` and runs. Spawn failure (interpreter absent) is
    /// distinguished from a `python3` that exists but crashes on `--version`.
    fn python3_available() -> bool {
        match std::process::Command::new("python3")
            .arg("--version")
            .output()
        {
            Ok(output) => output.status.success(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => panic!("python3 --version failed unexpectedly: {e}"),
        }
    }

    #[tokio::test]
    async fn outside_refire_window_write_matches_the_python_oracles_dry_run_routes() {
        let Some(fixtures_source) = find_real_sweep_fixtures_dir() else {
            return;
        };
        let Some(oracle_script) = find_oracle_script() else {
            return;
        };
        if !python3_available() {
            eprintln!("SKIPPING sweep parity test: python3 is not available on PATH");
            return;
        }

        let roadmap = "autonomous-foundation";

        // --- bastion side: run_sweep_once_at, outside the refire window ------------------------
        let rust_root = tempfile::tempdir().expect("tempdir");
        write_valid_brain_toml(rust_root.path());
        make_roadmap(rust_root.path(), roadmap);
        let rust_sweeps_dir = sweeps_dir(rust_root.path(), roadmap);
        seed_sweeps_dir_from_real_fixtures(&rust_sweeps_dir, &fixtures_source);

        let outside_window = DateTime::parse_from_rfc3339("2026-08-28T14:01:11Z")
            .unwrap()
            .with_timezone(&Utc);
        run_sweep_once_at(rust_root.path(), roadmap, false, None, outside_window)
            .await
            .expect("sweep outside the refire window should succeed");

        let rust_files = list_snapshot_files(&rust_sweeps_dir);
        assert_eq!(
            rust_files.len(),
            14,
            "bastion sweep must have written exactly one new snapshot"
        );
        let rust_written: Value = serde_json::from_str(
            &std::fs::read_to_string(rust_files.last().expect("a snapshot was just written"))
                .expect("read the snapshot bastion just wrote"),
        )
        .expect("bastion's written snapshot must be valid JSON");
        let rust_routed: Vec<Value> = rust_written
            .get("routed")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let rust_decisions: Vec<SweepDecision> =
            rust_routed.iter().map(decision_from_json).collect();

        // --- oracle side: the SAME 13-snapshot tree, byte-for-byte, `--dry-run` -----------------
        // The oracle's own CLI has no `--now` override (real `datetime.now(timezone.utc)`), but
        // every fixture's `ts_utc` is 2026-08-28 — real wall-clock time is always outside the 6h
        // refire window against them, so this reproduces the same "outside the window" case
        // without needing to fake the oracle's clock.
        let python_root = tempfile::tempdir().expect("tempdir");
        write_valid_brain_toml(python_root.path());
        make_roadmap(python_root.path(), roadmap);
        let python_sweeps_dir = sweeps_dir(python_root.path(), roadmap);
        seed_sweeps_dir_from_real_fixtures(&python_sweeps_dir, &fixtures_source);

        let output = std::process::Command::new("python3")
            .arg(&oracle_script)
            .arg("--roadmap")
            .arg(roadmap)
            .arg("--root")
            .arg(python_root.path())
            .arg("--dry-run")
            .arg("--quiet")
            .output()
            .unwrap_or_else(|e| panic!("failed to spawn python3 {}: {e}", oracle_script.display()));
        assert!(
            output.status.success(),
            "roadmap_sweep.py --dry-run failed: stdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let python_files = list_snapshot_files(&python_sweeps_dir);
        assert_eq!(
            python_files.len(),
            14,
            "the oracle always writes its one sweeps/<ts>.json file, dry-run or not"
        );
        let python_written: Value = serde_json::from_str(
            &std::fs::read_to_string(python_files.last().expect("the oracle just wrote a file"))
                .expect("read the snapshot the oracle just wrote"),
        )
        .expect("the oracle's written snapshot must be valid JSON");
        let python_routed: Vec<Value> = python_written
            .get("routed")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let python_decisions: Vec<SweepDecision> =
            python_routed.iter().map(decision_from_json).collect();

        assert_eq!(
            rust_decisions, python_decisions,
            "bastion's routes outside the refire window must match roadmap_sweep.py --dry-run's \
             routes on the identical fixture tree"
        );
    }

    /// A first-ever sweep (no prior snapshot at all) is always outside "the refire window" —
    /// there is nothing to be inside the window OF — and always writes.
    #[tokio::test]
    async fn first_sweep_has_no_prior_snapshot_to_gate_against() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_valid_brain_toml(tmp.path());
        make_roadmap(tmp.path(), "demo-roadmap");

        assert!(list_snapshot_files(&sweeps_dir(tmp.path(), "demo-roadmap")).is_empty());
        run_sweep_once_at(tmp.path(), "demo-roadmap", false, None, fixed_now())
            .await
            .expect("first sweep should succeed");
        assert_eq!(
            list_snapshot_files(&sweeps_dir(tmp.path(), "demo-roadmap")).len(),
            1
        );
    }
}
