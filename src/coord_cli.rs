//! `bastion coord status [--json]` — CLI face for engine-core's coordination reader
//! (`BA.25.A` task 1).
//!
//! This module owns NO reader logic of its own. It calls
//! `engine_core::coord::read_coordination_view` — the exact same function
//! `engine-serve`'s `GET /api/coordination` handler calls (`engine-serve/src/http.rs`,
//! `get_coordination`) — resolves `brain_root` the same way that handler does
//! (`engine_core::brain_root::resolve_brain_root()`), and either prints a
//! human-readable summary or the same `serde_json` serialisation the route emits.
//!
//! Reimplementing the reader, the degradation classification, or a second brain-root
//! resolution path is out of scope — that's engine-rs's `EN.15.A`.
//!
//! ## What "Live" does and does not mean here
//!
//! A `CoordinationStatus::Live` result only means every artifact this reader looked at
//! parsed cleanly and every cross-check agreed — it is silent on whether any
//! coordination activity has ever happened. An absent `.fleet-locks` directory (or an
//! empty one) is reported `Live` with zero entries everywhere; that is "nothing has run
//! yet", not "the coordination surface is healthy and populated". Nothing in this
//! module, and no test in this file, treats a clean `Live` verdict as evidence the
//! surface is populated or that its records are current-shape.

use std::path::Path;

use anyhow::{Context, Result};

use engine_core::coord::{CoordinationStatus, CoordinationView};

/// Handler for `bastion coord status [--json]`. Resolves `brain_root` exactly as the
/// route handler does, reads the joined coordination view, and prints either the
/// human summary or the `serde_json` serialisation of the view.
///
/// A [`CoordinationStatus::Degraded`] view is not a silent warning: this returns `Err`
/// naming every offending path (task 2, AC-2) so the process exits non-zero and the
/// operator's next action — open that file — is right there in the error. An absent
/// `.fleet-locks` subtree, or the whole directory, is a legitimate "nothing has run
/// yet" `Live` result and exits zero, same as any other clean read; only a record that
/// exists and cannot be parsed (or a failed cross-check) is degraded.
pub fn run_status(json: bool) -> Result<()> {
    let brain_root =
        engine_core::brain_root::resolve_brain_root().context("cannot resolve brain root")?;
    let view = view_for(&brain_root);
    let output = if json {
        json_output(&view)?
    } else {
        human_summary(&view)
    };
    println!("{output}");
    degraded_result(&view)
}

/// `Err` naming the offending path(s) when `view.status` is `Degraded`; `Ok(())`
/// (including the "nothing has run yet" empty/absent-tree case) otherwise. Split out
/// from `run_status` so the exit-code decision is testable without going through
/// `resolve_brain_root()` or stdout.
fn degraded_result(view: &CoordinationView) -> Result<()> {
    if view.status != CoordinationStatus::Degraded {
        return Ok(());
    }
    let paths = view
        .degradation_reasons
        .iter()
        .map(|r| r.path.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    anyhow::bail!("coordination surface degraded — offending path(s): {paths}");
}

/// Read the joined coordination view for `brain_root`. A one-line wrapper kept
/// separate so callers (and tests) can supply a fixture root without going through
/// `resolve_brain_root()`'s cwd/env resolution. Calls
/// `engine_core::coord::read_coordination_view` directly — no reimplementation.
fn view_for(brain_root: &Path) -> CoordinationView {
    engine_core::coord::read_coordination_view(brain_root)
}

/// Serialise `view` exactly as `engine-serve`'s `GET /api/coordination` route does —
/// `HttpResponse::Ok().json(view)`, which is `serde_json`'s default (compact)
/// serialisation of the same `CoordinationView` type. Pure and independently testable
/// from the read (per this repo's construction-vs-execution split, standing rule 6).
fn json_output(view: &CoordinationView) -> Result<String> {
    serde_json::to_string(view).context("failed to serialise CoordinationView")
}

/// Render a human-readable summary of `view`. Pure and independently testable from the
/// read. Never characterises a `Live` status as "healthy" or "populated" — only as
/// "every artifact read parsed cleanly", which is what the type actually guarantees.
fn human_summary(view: &CoordinationView) -> String {
    let mut lines = Vec::new();

    let status_line = match view.status {
        CoordinationStatus::Live => "status: live (every artifact read parsed cleanly)",
        CoordinationStatus::Degraded => "status: degraded",
    };
    lines.push(status_line.to_string());

    lines.push(format!("registry claims: {}", view.registry.len()));
    lines.push(format!("leases: {}", view.leases.len()));
    lines.push(format!("slots: {}", view.slots.len()));
    lines.push(format!("messages: {}", view.messages.len()));
    lines.push(format!("heartbeats: {}", view.heartbeats.len()));
    lines.push(format!("escalations: {}", view.escalations.len()));
    lines.push(format!("run records: {}", view.run_records.len()));

    if view.degradation_reasons.is_empty() {
        lines.push("degradation reasons: none".to_string());
    } else {
        lines.push(format!(
            "degradation reasons: {}",
            view.degradation_reasons.len()
        ));
        for reason in &view.degradation_reasons {
            lines.push(format!("  - {}: {}", reason.path, reason.reason));
        }
    }

    lines.join("\n")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// AC-1's gated half (task 1): the CLI's `--json` output bytes equal
    /// `serde_json::to_string(&engine_core::coord::read_coordination_view(root))` for
    /// the identical root — same function, same serialisation the route handler uses.
    /// The live `curl` comparison against a running `bastion serve` is task 4's
    /// declared un-gateable half, not this test.
    #[test]
    fn json_output_matches_route_serialization() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // No .fleet-locks subtree at all — a legitimate empty/Live tree, used here only
        // to exercise the serialisation path, not to assert anything about liveness.
        let view = view_for(tmp.path());

        let cli_bytes = json_output(&view).expect("json_output");
        let route_bytes =
            serde_json::to_string(&engine_core::coord::read_coordination_view(tmp.path()))
                .expect("route serialisation");

        assert_eq!(
            cli_bytes, route_bytes,
            "CLI --json output must byte-equal serde_json of the same read_coordination_view call"
        );
    }

    /// The CLI's own read (`view_for`) and a fresh call to the underlying reader must
    /// agree field-for-field on the same fixture root, not merely serialise the same.
    #[test]
    fn view_for_delegates_to_engine_core_reader() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cli_view = view_for(tmp.path());
        let direct_view = engine_core::coord::read_coordination_view(tmp.path());
        assert_eq!(cli_view, direct_view);
    }

    #[test]
    fn human_summary_reports_status_and_counts() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let view = view_for(tmp.path());
        let summary = human_summary(&view);
        assert!(summary.contains("status:"));
        assert!(summary.contains("registry claims: 0"));
        assert!(summary.contains("slots: 0"));
    }

    #[test]
    fn human_summary_lists_degradation_reasons_when_present() {
        use engine_core::coord::DegradationReason;

        let view = CoordinationView {
            status: CoordinationStatus::Degraded,
            degradation_reasons: vec![DegradationReason {
                path: "/tmp/bad.json".to_string(),
                reason: "could not parse JSON".to_string(),
            }],
            registry: Vec::new(),
            leases: Vec::new(),
            slots: Vec::new(),
            messages: Vec::new(),
            heartbeats: Vec::new(),
            escalations: Vec::new(),
            run_records: Vec::new(),
        };
        let summary = human_summary(&view);
        assert!(summary.contains("status: degraded"));
        assert!(summary.contains("degradation reasons: 1"));
        assert!(summary.contains("/tmp/bad.json"));
        assert!(summary.contains("could not parse JSON"));
    }

    // ── Task 2: degraded-vs-empty exit-code split ───────────────────────────────

    /// A malformed record that EXISTS (real fixture file, never the live
    /// `.fleet-locks/`) must make `degraded_result` return `Err` naming the offending
    /// path. Same test also asserts an absent `.fleet-locks` subtree, and an absent
    /// `.fleet-locks` directory entirely, each go `Ok(())` — so degraded and empty are
    /// provably distinguishable in one run, not two tests that could each drift.
    #[test]
    fn malformed_record_is_degraded_absent_tree_is_not() {
        // Case 1: a real malformed artifact under lane-agents/ — invalid JSON syntax,
        // not a missing subcommand, per D68's gate-shape requirement.
        let tmp = tempfile::tempdir().expect("tempdir");
        let lock_dir = tmp.path().join(".fleet-locks");
        let lane_agents = lock_dir.join("lane-agents");
        std::fs::create_dir_all(&lane_agents).expect("create lane-agents");
        let bad_file = lane_agents.join("bastion__agent-probe.json");
        std::fs::write(&bad_file, b"{ this is not valid json").expect("write malformed fixture");

        let view = view_for(tmp.path());
        assert_eq!(view.status, CoordinationStatus::Degraded);
        let err = degraded_result(&view).expect_err("malformed record must exit non-zero");
        let msg = err.to_string();
        assert!(
            msg.contains(&bad_file.display().to_string()),
            "error must name the offending file path, got: {msg}"
        );

        // Case 2: an absent `.fleet-locks/lane-agents` subtree (root exists, subdir
        // doesn't) is a legitimate empty/Live state and must exit zero.
        let tmp2 = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp2.path().join(".fleet-locks")).expect("create lock dir");
        let empty_subtree_view = view_for(tmp2.path());
        assert_eq!(empty_subtree_view.status, CoordinationStatus::Live);
        assert!(degraded_result(&empty_subtree_view).is_ok());

        // Case 3: the whole `.fleet-locks` directory is absent — also Live, also zero.
        let tmp3 = tempfile::tempdir().expect("tempdir");
        let absent_dir_view = view_for(tmp3.path());
        assert_eq!(absent_dir_view.status, CoordinationStatus::Live);
        assert!(degraded_result(&absent_dir_view).is_ok());
    }

    /// `degraded_result` never touches the live shared `.fleet-locks/` — every case
    /// above builds its own `tempdir()` fixture root, so this suite cannot interfere
    /// with another lane's concurrent registry/lease/slot state.
    #[test]
    fn degraded_result_ok_on_clean_view() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let view = view_for(tmp.path());
        assert!(degraded_result(&view).is_ok());
    }

    // ── Task 3: parity with `fleet_concurrency_check.py status` ─────────────────
    //
    // AC-3. This section never points either side at the live shared `.fleet-locks/` —
    // every fixture below is its own `tempdir()`. It also never hardcodes any constant
    // out of the Python oracle (`DEFAULT_TTL_SECONDS` included) — every shared value is
    // parsed from the script's own source text at test time, mirroring engine-core's
    // own `coord_parity.rs` (`EN.15.A` task 2), which established this exact pattern
    // for this same oracle pair.

    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// Walk up from `start` looking for a `brain.toml` — the company-brain vault root
    /// that houses the sibling `base-template/` this task's oracle script lives in.
    fn find_brain_root(start: &Path) -> Option<PathBuf> {
        crate::config::walk_up_from(start, "brain.toml")
            .and_then(|toml| toml.parent().map(Path::to_path_buf))
    }

    /// `<brain_root>/base-template/scripts/fleet_concurrency_check.py`.
    fn oracle_script_path(brain_root: &Path) -> PathBuf {
        brain_root
            .join("base-template")
            .join("scripts")
            .join("fleet_concurrency_check.py")
    }

    /// Resolve the real oracle script, or `None` (after a loud `eprintln!`) when this
    /// checkout has no sibling `base-template` to find it in — e.g. an isolated clone
    /// of just this repo. Skips loudly rather than silently passing.
    fn find_oracle_script() -> Option<PathBuf> {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let Some(brain_root) = find_brain_root(manifest_dir) else {
            eprintln!(
                "SKIPPING coord parity test: no brain.toml found walking up from {} \
                 (this checkout has no sibling base-template to locate the oracle in)",
                manifest_dir.display()
            );
            return None;
        };
        let script = oracle_script_path(&brain_root);
        if !script.is_file() {
            eprintln!(
                "SKIPPING coord parity test: brain root found at {} but {} does not exist",
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
        match Command::new("python3").arg("--version").output() {
            Ok(output) => output.status.success(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => panic!("python3 --version failed unexpectedly: {e}"),
        }
    }

    /// The single point where every parity test below decides whether it can run at
    /// all. `None` (after an `eprintln!`) when either half of the pair — interpreter or
    /// oracle script — is unavailable.
    fn require_parity_environment() -> Option<PathBuf> {
        if !python3_available() {
            eprintln!("SKIPPING coord parity test: python3 is not available on PATH");
            return None;
        }
        find_oracle_script()
    }

    /// Parse `DEFAULT_TTL_SECONDS = <int>` out of the oracle's own source text. The
    /// literal `5400` must never appear in this module as the TTL under test — only a
    /// value derived by reading the Python at test time.
    fn parse_default_ttl_seconds(source: &str) -> u64 {
        for line in source.lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix("DEFAULT_TTL_SECONDS") else {
                continue;
            };
            let rest = rest.trim_start();
            let Some(rest) = rest.strip_prefix('=') else {
                continue;
            };
            let rest = rest.trim_start();
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !digits.is_empty() {
                return digits.parse().unwrap_or_else(|e| {
                    panic!("DEFAULT_TTL_SECONDS digits '{digits}' not a u64: {e}")
                });
            }
        }
        panic!("could not find a `DEFAULT_TTL_SECONDS = <int>` line in the given source text");
    }

    /// Read the oracle script and parse its `DEFAULT_TTL_SECONDS`.
    fn real_default_ttl_seconds(script: &Path) -> u64 {
        let source = std::fs::read_to_string(script)
            .unwrap_or_else(|e| panic!("could not read oracle script {}: {e}", script.display()));
        parse_default_ttl_seconds(&source)
    }

    /// Current wall-clock time as epoch seconds — the same clock the oracle's
    /// `time.time()` reads and a slot's `started_at` is measured against.
    fn now_epoch() -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_secs_f64()
    }

    /// Write one fleet-concurrency slot file at `<lock_dir>/<repo>__agent-<agent>.json`
    /// — the exact flat-at-root, `pid_source: "self"` shape
    /// `fleet_concurrency_check.py::register` writes. `pid_source: "self"`
    /// deliberately: it keeps the pid-liveness branch of the oracle's staleness sweep
    /// out of play, so age-vs-TTL is the only axis either side needs to agree on.
    fn write_slot(lock_dir: &Path, repo: &str, agent: &str, category: &str, started_at_epoch: f64) {
        std::fs::create_dir_all(lock_dir).expect("create lock_dir");
        let path = lock_dir.join(format!("{repo}__agent-{agent}.json"));
        let body = serde_json::json!({
            "repo": repo,
            "pid": std::process::id(),
            "pid_source": "self",
            "agent": agent,
            "category": category,
            "started_at": started_at_epoch,
        });
        std::fs::write(&path, body.to_string())
            .unwrap_or_else(|e| panic!("write slot file {}: {e}", path.display()));
    }

    /// Run `python3 <script> status --lock-dir <lock_dir> --ttl <ttl>` and parse its
    /// JSON stdout.
    fn run_python_status(script: &Path, lock_dir: &Path, ttl: u64) -> serde_json::Value {
        let output = Command::new("python3")
            .arg(script)
            .arg("status")
            .arg("--lock-dir")
            .arg(lock_dir)
            .arg("--ttl")
            .arg(ttl.to_string())
            .output()
            .unwrap_or_else(|e| panic!("failed to spawn python3 {}: {e}", script.display()));
        assert!(
            output.status.success(),
            "fleet_concurrency_check.py status failed: stdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
            panic!(
                "status output was not valid JSON: {e}\nstdout={}",
                String::from_utf8_lossy(&output.stdout)
            )
        })
    }

    /// The `"active"` array from a `status` JSON payload, sorted for order-independent
    /// comparison.
    fn active_lanes_from_python_json(payload: &serde_json::Value) -> Vec<String> {
        let mut active: Vec<String> = payload["active"]
            .as_array()
            .expect("status JSON must carry an `active` array")
            .iter()
            .map(|v| v.as_str().expect("active entries are strings").to_string())
            .collect();
        active.sort();
        active
    }

    /// bastion's own slot view (`view_for` — i.e. exactly what `bastion coord status
    /// --json` serialises), reduced to the SAME `"{repo} ({category})"` shape the
    /// oracle's `status` reports and filtered to the SAME `ttl_seconds` window — the
    /// reader itself performs no TTL filtering (that's the write-path sweep's job), so
    /// this helper is where the comparison's TTL window is applied, once, on both
    /// sides.
    fn bastion_active_heavy_lanes(lock_dir_root: &Path, ttl_seconds: u64, now: f64) -> Vec<String> {
        let view = view_for(lock_dir_root);
        let mut active: Vec<String> = view
            .slots
            .iter()
            .filter_map(|entry| {
                let slot: &okf_core::SlotRecord = entry.slot.typed()?;
                let age = now - slot.started_at;
                if age <= ttl_seconds as f64 {
                    Some(format!("{} ({})", slot.repo, slot.category))
                } else {
                    None
                }
            })
            .collect();
        active.sort();
        active
    }

    /// AC-3, positive half: bastion's slot view and `fleet_concurrency_check.py
    /// status` agree over the same mktemp fixture tree.
    #[test]
    fn bastion_slot_view_agrees_with_python_oracle_on_a_shared_fixture() {
        let Some(script) = require_parity_environment() else {
            return;
        };
        let ttl = real_default_ttl_seconds(&script);
        let now = now_epoch();

        // `bastion_active_heavy_lanes` reads `<lock_dir_root>/.fleet-locks` (matching
        // `engine_core::coord::resolve_lock_dir`'s default), so the slot fixture files
        // must live one level under the tempdir handed to it.
        let root = tempfile::tempdir().expect("tempdir");
        let lock_dir = root.path().join(".fleet-locks");

        write_slot(
            &lock_dir,
            "bastion",
            "bastion-a1",
            "native-build",
            now - 30.0,
        );
        write_slot(
            &lock_dir,
            "price-scout",
            "price-scout-b2",
            "browser-automation",
            now - 60.0,
        );

        let python_active =
            active_lanes_from_python_json(&run_python_status(&script, &lock_dir, ttl));
        let bastion_active = bastion_active_heavy_lanes(root.path(), ttl, now);

        assert_eq!(
            bastion_active, python_active,
            "bastion's slot view and fleet_concurrency_check.py status disagree on the \
             same fixture tree"
        );
        assert_eq!(
            bastion_active,
            vec![
                "bastion (native-build)".to_string(),
                "price-scout (browser-automation)".to_string(),
            ],
            "expected exactly the two fresh fixture slots, sorted"
        );
    }

    /// AC-3, negative half (runtime inversion, not a committed red baseline — `cargo
    /// test` is a gates:true harness row, so a committed-red case would fail every
    /// later task's gate including the one that fixes it): the Python oracle reporting
    /// an EXTRA active repo the bastion side never saw must turn the comparison RED;
    /// comparing against the unmodified fixture again afterwards must turn it green.
    /// Both are asserted in this one test run.
    #[test]
    fn extra_python_side_repo_turns_the_comparison_red_then_green() {
        let Some(script) = require_parity_environment() else {
            return;
        };
        let ttl = real_default_ttl_seconds(&script);
        let now = now_epoch();

        let root = tempfile::tempdir().expect("tempdir");
        let lock_dir = root.path().join(".fleet-locks");
        write_slot(
            &lock_dir,
            "bastion",
            "bastion-a1",
            "native-build",
            now - 30.0,
        );

        let bastion_active = bastion_active_heavy_lanes(root.path(), ttl, now);
        let python_active_baseline =
            active_lanes_from_python_json(&run_python_status(&script, &lock_dir, ttl));
        assert_eq!(
            bastion_active, python_active_baseline,
            "baseline fixture must agree before the inversion is introduced"
        );

        // Introduce the drift ONLY on the Python side's fixture copy — an extra active
        // repo the bastion side never read — and show the same comparison now
        // disagrees (goes RED). The bastion side's own read (`bastion_active`) is
        // never recomputed here; the drifted directory is queried by the oracle only.
        let drifted_lock_dir = root.path().join(".fleet-locks-drifted");
        write_slot(
            &drifted_lock_dir,
            "bastion",
            "bastion-a1",
            "native-build",
            now - 30.0,
        );
        write_slot(
            &drifted_lock_dir,
            "amistad",
            "amistad-extra",
            "browser-automation",
            now - 10.0,
        );
        let python_active_drifted =
            active_lanes_from_python_json(&run_python_status(&script, &drifted_lock_dir, ttl));
        assert_ne!(
            bastion_active, python_active_drifted,
            "an extra Python-side active repo must make the comparison disagree"
        );
        assert!(
            python_active_drifted.contains(&"amistad (browser-automation)".to_string()),
            "expected the drifted oracle read to report the extra repo, got: {python_active_drifted:?}"
        );

        // Restore: compare against the original, unmodified fixture again — green.
        let python_active_restored =
            active_lanes_from_python_json(&run_python_status(&script, &lock_dir, ttl));
        assert_eq!(
            bastion_active, python_active_restored,
            "comparing against the unmodified fixture again must agree"
        );
    }
}
