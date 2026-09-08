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
//! ## The live route this CLI is diffed against is currently unreachable (task 4, 2026-09-08)
//!
//! `BA.25.A` task 4's live comparison (`bastion coord status --json` vs. `curl -s
//! $ENGINE/api/coordination` against an installed, running `bastion serve`) FAILED, not
//! blocked: the server came up cleanly and every other route tested (`/api/repos`,
//! `/events/`) answered correctly, but `GET /api/coordination` itself 404s. Root cause
//! is in `src/serve/mod.rs`, not here: bastion's own bearer-protected `web::scope("/api")`
//! is registered as its own top-level service before the engine's route table is mounted
//! at `web::scope("")`, and actix-web does not fall through to that sibling scope when a
//! request under `/api/*` matches no resource bastion itself registered — so the engine's
//! literal `/api/coordination` resource, while genuinely mounted, is unreachable through
//! the live process as currently wired. This module and its `--json` output are unaffected
//! — the failure is in HTTP routing, not in this reader path — but AC-1's live half cannot
//! be closed until that scope ordering is fixed in a follow-up `bastion` block. Full
//! invocation, response bodies and the confirming positive controls are recorded in the
//! block record's `notes` (`planning/blocks/BA.25.A.json`).
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

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use engine_core::coord::write::{
    CoordWriteError, HeartbeatRequest, RegisterOutcome, RegisterRequest,
};
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

// ── `register` / `heartbeat` / `release` — `bastion coord`'s write verbs (BA.25.C task 1) ──
//
// These three are thin CLI faces over `engine_core::coord::write::{register,heartbeat,
// release}` — the SAME functions `engine-serve`'s `POST /api/coordination/{register,
// heartbeat,release}` routes call (`engine-serve/src/http.rs`, `coord_register`/
// `coord_heartbeat`/`coord_release`). No write logic — schema validation, `.prev/`
// snapshotting, capacity enforcement — is reimplemented here; that seam is `EN.15.C`'s.
//
// Each public `run_*` entry point resolves the REAL lock directory (via `resolve_brain_root()`
// then `engine_core::coord::resolve_lock_dir`, unless `--lock-dir` overrides it) and delegates
// to a `*_at` sibling that takes the resolved directory directly — the same split `run_status`/
// `view_for` already established, so a test can drive the write functions against a `tempdir()`
// fixture without going through brain-root discovery at all, and the live `.fleet-locks/` is
// never touched by anything in `#[cfg(test)]` below.
//
// `register`'s exit code is the one place this module's write verbs diverge from a plain
// `Result<()>` → exit-1-on-Err contract: `fleet_concurrency_check.py register` exits 0 when
// allowed (including the degraded-advisory case) and 3 when refused at capacity (see that
// script's own module doc comment and `main()`'s `return 0 if result.allowed else 3`) — a
// caller-visible contract this CLI must hold, not an internal implementation detail. Mirroring
// `notify_cli::AskOutcome::exit_code`'s pattern: the 0/3 mapping lives in one pure, directly
// testable function (`register_exit_code`), and the only `std::process::exit` call sits in
// `run_register`, after the outcome's JSON has already been printed to stdout.
//
// `heartbeat` and `release` have no such second exit code. `release` always reports success
// (`{"removed": bool}`) on a real seam I/O outcome, matching `fleet_concurrency_check.py
// release`'s own always-0 contract — a genuine filesystem error is still a real failure and
// exits 1 via the ordinary `anyhow::Result` path, same as any other bastion command's I/O
// fault. `heartbeat` errors when `agent_name` names no existing registry claim
// (`engine_core::coord::write::heartbeat`'s own refusal — "nothing to heartbeat"); the Python
// oracle has NO direct `heartbeat` subcommand to compare this against (its own re-register-is-
// a-heartbeat idiom never refuses this way — an absent claim there just becomes a fresh
// `register`), so this case is deliberately left to bastion's ordinary exit-1 error path
// rather than inventing a code with nothing on the other side of the parity contract to match.

/// Resolve the lock directory a coord write verb writes into: `lock_dir_override` when given
/// (mirrors `fleet_concurrency_check.py`'s own `--lock-dir`, and is how every fixture test in
/// this module points a write verb at a `tempdir()` instead of the live tree), else the same
/// `resolve_brain_root()` → `engine_core::coord::resolve_lock_dir` path `run_status` already
/// uses for reads.
fn resolve_write_lock_dir(lock_dir_override: Option<&Path>) -> Result<PathBuf> {
    if let Some(dir) = lock_dir_override {
        return Ok(dir.to_path_buf());
    }
    let brain_root =
        engine_core::brain_root::resolve_brain_root().context("cannot resolve brain root")?;
    Ok(engine_core::coord::resolve_lock_dir(&brain_root))
}

/// The exit code for a `register` outcome — total over both cases, mirroring
/// `fleet_concurrency_check.py register`'s own contract: `0` when allowed, `3` when refused at
/// capacity. Kept as its own pure function (not inlined into `run_register`) so it is testable
/// directly, without spawning a process — same shape as `notify_cli::AskOutcome::exit_code`.
fn register_exit_code(outcome: &RegisterOutcome) -> i32 {
    if outcome.allowed { 0 } else { 3 }
}

/// `register` against an already-resolved `lock_dir` — the pure(ish) core `run_register`
/// wraps, and what every fixture test in this module calls directly against a `tempdir()`.
/// Stamps `now`/`pid` itself (mirroring `coord_register`'s own route handler) rather than
/// trusting a caller-supplied clock; writes no `host` (single-host fleet, per
/// `engine_core::coord::write`'s own module doc comment on Fork 1).
#[allow(clippy::too_many_arguments)]
fn register_at(
    lock_dir: &Path,
    agent_name: &str,
    repo: &str,
    lane: &str,
    roadmap: &str,
    category: Option<&str>,
) -> Result<RegisterOutcome, CoordWriteError> {
    let now = chrono::Utc::now();
    let now_iso = now.to_rfc3339();
    let now_epoch = now.timestamp() as f64 + f64::from(now.timestamp_subsec_nanos()) / 1e9;
    let req = RegisterRequest {
        agent_name,
        repo,
        lane,
        roadmap,
        host: None,
        category,
        now_iso: &now_iso,
        now_epoch,
        pid: std::process::id() as i64,
    };
    engine_core::coord::write::register(lock_dir, &req)
}

/// `bastion coord register --agent-name <n> --repo <r> --lane <l> --roadmap <rm> [--category
/// <c>] [--lock-dir <dir>]` — write the lane-agent registry claim and, when `--category` is
/// given, enforce and write the heavy-lane capacity slot first (`engine_core::coord::write::
/// register`'s own all-or-nothing contract: a capacity refusal writes nothing at all).
///
/// Always prints the outcome as one JSON line — `{"allowed":true,"reason":null,"active":[]}`
/// on success, `{"allowed":false,"reason":"...","active":[...]}` on refusal — then exits `0` or
/// `3` per [`register_exit_code`]. A `CoordWriteError` (invalid record shape, I/O failure)
/// bubbles up as an ordinary `anyhow::Result` error and exits `1`, same as any other bastion
/// command fault.
#[allow(clippy::too_many_arguments)]
pub fn run_register(
    agent_name: &str,
    repo: &str,
    lane: &str,
    roadmap: &str,
    category: Option<&str>,
    lock_dir_override: Option<&Path>,
) -> Result<()> {
    let lock_dir = resolve_write_lock_dir(lock_dir_override)?;
    let outcome = register_at(&lock_dir, agent_name, repo, lane, roadmap, category)
        .with_context(|| format!("register failed for agent `{agent_name}`"))?;
    let json = serde_json::json!({
        "allowed": outcome.allowed,
        "reason": outcome.reason,
        "active": outcome.active,
    });
    println!(
        "{}",
        serde_json::to_string(&json).context("failed to serialise register outcome")?
    );
    let code = register_exit_code(&outcome);
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}

/// `heartbeat` against an already-resolved `lock_dir` — mirrors [`register_at`]'s split.
fn heartbeat_at(
    lock_dir: &Path,
    agent_name: &str,
    current_block: Option<&str>,
    block_started_at: Option<&str>,
) -> Result<(), CoordWriteError> {
    let now_iso = chrono::Utc::now().to_rfc3339();
    let req = HeartbeatRequest {
        agent_name,
        host: None,
        now_iso: &now_iso,
        current_block,
        block_started_at,
    };
    engine_core::coord::write::heartbeat(lock_dir, &req)
}

/// `bastion coord heartbeat --agent-name <n> [--current-block <b>] [--block-started-at <ts>]
/// [--lock-dir <dir>]` — re-stamp an existing registry claim's `heartbeat` field (and, when
/// given, `current_block`/`block_started_at`); `started_at` is left untouched, unlike
/// `register`'s own idempotent-refresh path.
///
/// Prints `{"ok":true}` and exits `0` on success. Errors — including "no existing registry
/// claim for this agent", `engine_core::coord::write::heartbeat`'s own refusal — bubble up as
/// an ordinary `anyhow::Result` error and exit `1`; see this module's doc comment for why that
/// case gets no invented exit code of its own.
pub fn run_heartbeat(
    agent_name: &str,
    current_block: Option<&str>,
    block_started_at: Option<&str>,
    lock_dir_override: Option<&Path>,
) -> Result<()> {
    let lock_dir = resolve_write_lock_dir(lock_dir_override)?;
    heartbeat_at(&lock_dir, agent_name, current_block, block_started_at)
        .with_context(|| format!("heartbeat failed for agent `{agent_name}`"))?;
    println!("{}", serde_json::json!({ "ok": true }));
    Ok(())
}

/// `release` against an already-resolved `lock_dir` — mirrors [`register_at`]'s split.
fn release_at(lock_dir: &Path, agent_name: &str) -> Result<bool, CoordWriteError> {
    engine_core::coord::write::release(lock_dir, agent_name)
}

/// `bastion coord release --agent-name <n> [--lock-dir <dir>]` — remove `agent_name`'s registry
/// claim, if any. Idempotent, matching `fleet_concurrency_check.py release`'s own
/// always-succeeds contract: prints `{"removed":true|false}` and exits `0` whether or not a
/// claim actually existed to remove. A genuine I/O failure removing the file still bubbles up
/// as an ordinary `anyhow::Result` error and exits `1`.
pub fn run_release(agent_name: &str, lock_dir_override: Option<&Path>) -> Result<()> {
    let lock_dir = resolve_write_lock_dir(lock_dir_override)?;
    let removed = release_at(&lock_dir, agent_name)
        .with_context(|| format!("release failed for agent `{agent_name}`"))?;
    println!("{}", serde_json::json!({ "removed": removed }));
    Ok(())
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

    // ── BA.25.C task 1: register / heartbeat / release ──────────────────────────
    //
    // Every fixture below is its own `tempdir()`, matching this module's Task-3 parity tests
    // above — none of these ever reads or writes the live shared `.fleet-locks/`.

    /// `python3 <script> register --repo <r> --agent <a> --category <c> --lock-dir <dir>`'s
    /// captured exit code and parsed JSON stdout.
    struct PythonRegisterOutput {
        exit_code: i32,
        json: serde_json::Value,
    }

    fn run_python_register(
        script: &Path,
        lock_dir: &Path,
        repo: &str,
        agent: &str,
        category: &str,
    ) -> PythonRegisterOutput {
        let output = Command::new("python3")
            .arg(script)
            .arg("register")
            .arg("--repo")
            .arg(repo)
            .arg("--agent")
            .arg(agent)
            .arg("--category")
            .arg(category)
            .arg("--lock-dir")
            .arg(lock_dir)
            .output()
            .unwrap_or_else(|e| panic!("failed to spawn python3 register: {e}"));
        let exit_code = output.status.code().unwrap_or(-1);
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
            panic!(
                "register output was not valid JSON: {e}\nstdout={}\nstderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        PythonRegisterOutput { exit_code, json }
    }

    /// Parse `MAX_LANES_BY_CATEGORY`'s per-category cap out of the oracle's own source text —
    /// e.g. the `"native-build": 4,` line — never a number copied out of this spec. Mirrors
    /// `parse_default_ttl_seconds`'s own line-scan approach.
    fn parse_cap_for_category(source: &str, category: &str) -> usize {
        let needle = format!("\"{category}\":");
        for line in source.lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix(needle.as_str()) else {
                continue;
            };
            let rest = rest.trim_start();
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !digits.is_empty() {
                return digits
                    .parse()
                    .unwrap_or_else(|e| panic!("cap digits '{digits}' not a usize: {e}"));
            }
        }
        panic!("could not find a `\"{category}\": <int>` line in the given source text");
    }

    /// AC-1 (task 1): a category already filled to capacity by Python-registered slots makes
    /// `register` exit 3 with the SAME message the Python emits for the identical refusal —
    /// both captured live in this one test run, never a copied string literal. The category's
    /// cap is parsed from the oracle's own source (`parse_cap_for_category`), not hardcoded,
    /// since D66 made the cap vary per category and a frozen "three" from an older spec
    /// revision would no longer describe either side's real behaviour.
    #[test]
    fn register_full_category_exits_3_with_pythons_own_message() {
        let Some(script) = require_parity_environment() else {
            return;
        };
        let source = std::fs::read_to_string(&script).expect("read oracle script");
        let category = "native-build";
        let cap = parse_cap_for_category(&source, category);

        let root = tempfile::tempdir().expect("tempdir");
        let lock_dir = root.path().join(".fleet-locks");

        // Fill the category to capacity THROUGH THE PYTHON oracle itself — never via a
        // hand-written fixture file — so the "already filled" state is genuinely
        // Python-registered, per the AC's own wording.
        for i in 0..cap {
            let repo = format!("filler-repo-{i}");
            let agent = format!("filler-agent-{i}");
            let filler = run_python_register(&script, &lock_dir, &repo, &agent, category);
            assert_eq!(
                filler.exit_code, 0,
                "python filler register #{i} into `{category}` must succeed (cap={cap}): {:?}",
                filler.json
            );
        }

        // A DISTINCT (repo, agent) attempting to register into the now-full category. Run
        // through the Python oracle first, on the SAME fixture directory, to capture its
        // actual refusal — a register refusal writes nothing (both sides' `register` is
        // all-or-nothing), so this call leaves the fixture exactly as filled as it was.
        let overflow_repo = "overflow-repo";
        let overflow_agent = "overflow-agent";
        let python_overflow =
            run_python_register(&script, &lock_dir, overflow_repo, overflow_agent, category);
        assert_eq!(
            python_overflow.exit_code, 3,
            "python register into a full `{category}` category must exit 3, got: {:?}",
            python_overflow.json
        );
        assert_eq!(python_overflow.json["allowed"], serde_json::json!(false));
        let python_message = python_overflow.json["reason"]
            .as_str()
            .expect("python refusal must carry a `reason` string")
            .to_string();

        // Now the RUST verb, against the SAME (still-full, untouched-by-the-refusal-above)
        // fixture directory.
        let outcome = register_at(
            &lock_dir,
            overflow_agent,
            overflow_repo,
            "some-lane",
            "some-roadmap",
            Some(category),
        )
        .expect("register_at must not error on an ordinary capacity refusal");

        assert!(
            !outcome.allowed,
            "rust register must also refuse: {outcome:?}"
        );
        assert_eq!(
            register_exit_code(&outcome),
            python_overflow.exit_code,
            "rust's exit code must match python's captured exit code, not a hardcoded 3"
        );
        assert_eq!(
            outcome.reason.as_deref(),
            Some(python_message.as_str()),
            "rust's refusal message must byte-equal python's captured refusal message"
        );
    }

    /// AC-5 (positive half): an ordinary, under-capacity registration exits `0` on both sides
    /// for the identical (repo, agent, category) input, on a shared fixture directory.
    #[test]
    fn register_allowed_exits_0_matching_python() {
        let Some(script) = require_parity_environment() else {
            return;
        };
        let root = tempfile::tempdir().expect("tempdir");
        let lock_dir = root.path().join(".fleet-locks");

        let python_result = run_python_register(
            &script,
            &lock_dir,
            "probe-repo",
            "probe-agent",
            "native-build",
        );
        assert_eq!(python_result.exit_code, 0, "{:?}", python_result.json);
        assert_eq!(python_result.json["allowed"], serde_json::json!(true));

        // A DIFFERENT agent/repo pair, against the SAME directory the python call above just
        // wrote one slot into — asserts the rust side's own capacity read agrees, not just
        // that an empty directory trivially allows.
        let outcome = register_at(
            &lock_dir,
            "probe-agent-2",
            "probe-repo-2",
            "some-lane",
            "some-roadmap",
            Some("native-build"),
        )
        .expect("register_at must succeed");
        assert!(outcome.allowed);
        assert_eq!(register_exit_code(&outcome), python_result.exit_code);
    }

    /// `register_exit_code` is a total function over both `RegisterOutcome` shapes — asserted
    /// directly, with no process spawned, mirroring
    /// `notify_cli::exit_code_is_total_over_all_four_variants`'s own style.
    #[test]
    fn register_exit_code_is_total_over_both_outcomes() {
        assert_eq!(
            register_exit_code(&RegisterOutcome {
                allowed: true,
                reason: None,
                active: Vec::new(),
            }),
            0
        );
        assert_eq!(
            register_exit_code(&RegisterOutcome {
                allowed: false,
                reason: Some("fleet at capacity".to_string()),
                active: vec!["bastion".to_string()],
            }),
            3
        );
    }

    /// A capacity refusal must write NOTHING — no slot, no registry claim — matching
    /// `engine_core::coord::write::register`'s own all-or-nothing contract (mirrored from the
    /// Python's). Fixture is a `tempdir()`; the live `.fleet-locks/` is never touched.
    #[test]
    fn register_refusal_writes_nothing() {
        let root = tempfile::tempdir().expect("tempdir");
        let lock_dir = root.path().join(".fleet-locks");

        for i in 0..4 {
            let repo = format!("filler-{i}");
            let agent = format!("filler-agent-{i}");
            let outcome = register_at(
                &lock_dir,
                &agent,
                &repo,
                "lane",
                "roadmap",
                Some("native-build"),
            )
            .expect("filler register must succeed (native-build cap is 4)");
            assert!(outcome.allowed, "filler #{i} must be allowed: {outcome:?}");
        }

        let before: Vec<_> = std::fs::read_dir(&lock_dir)
            .expect("read lock_dir")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();

        let outcome = register_at(
            &lock_dir,
            "overflow-agent",
            "overflow-repo",
            "lane",
            "roadmap",
            Some("native-build"),
        )
        .expect("refused register_at must not error");
        assert!(!outcome.allowed);

        let after: Vec<_> = std::fs::read_dir(&lock_dir)
            .expect("read lock_dir")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        assert_eq!(
            before.len(),
            after.len(),
            "a capacity refusal must not write a new file: before={before:?} after={after:?}"
        );
    }

    /// `heartbeat` re-stamps an existing claim's `heartbeat`/`current_block`/
    /// `block_started_at` fields while leaving `started_at` untouched — read back through
    /// `view_for` (the same reader `bastion coord status` uses), never a second parse path.
    #[test]
    fn heartbeat_updates_claim_leaves_started_at_untouched() {
        let root = tempfile::tempdir().expect("tempdir");
        let lock_dir = root.path().join(".fleet-locks");

        register_at(
            &lock_dir,
            "hb-agent",
            "hb-repo",
            "hb-lane",
            "hb-roadmap",
            None,
        )
        .expect("initial register must succeed");

        let before = view_for(root.path());
        let claim_before = before
            .registry
            .iter()
            .find_map(|e| e.claim.typed())
            .expect("registry claim must exist after register");
        let started_at_before = claim_before.started_at.clone();

        heartbeat_at(
            &lock_dir,
            "hb-agent",
            Some("BA.25.C"),
            Some("2026-09-08T00:00:00Z"),
        )
        .expect("heartbeat must succeed for an existing claim");

        let after = view_for(root.path());
        let claim_after = after
            .registry
            .iter()
            .find_map(|e| e.claim.typed())
            .expect("registry claim must still exist after heartbeat");

        assert_eq!(
            claim_after.started_at, started_at_before,
            "heartbeat must never touch started_at"
        );
        assert_eq!(claim_after.current_block.as_deref(), Some("BA.25.C"));
        assert_eq!(
            claim_after.block_started_at.as_deref(),
            Some("2026-09-08T00:00:00Z")
        );
        assert_ne!(
            claim_after.heartbeat, claim_before.heartbeat,
            "heartbeat field itself must be re-stamped"
        );
    }

    /// Heartbeating an agent with no existing registry claim errors rather than silently
    /// creating one — `engine_core::coord::write::heartbeat`'s own refusal, surfaced here as
    /// an ordinary `Err`. No Python counterpart exists for this case (see this module's doc
    /// comment above `run_heartbeat`), so this only asserts `Err`, never a specific exit code.
    #[test]
    fn heartbeat_errors_when_no_existing_claim() {
        let root = tempfile::tempdir().expect("tempdir");
        let lock_dir = root.path().join(".fleet-locks");
        std::fs::create_dir_all(&lock_dir).expect("create lock dir");

        let result = heartbeat_at(&lock_dir, "ghost-agent", None, None);
        assert!(
            result.is_err(),
            "heartbeating a never-registered agent must error, not silently create a claim"
        );
    }

    /// `release` reports `removed: true` for a real claim and `removed: false` the second time
    /// — idempotent, matching `fleet_concurrency_check.py release`'s own always-succeeds
    /// contract. Effect verified through `view_for`, same as the heartbeat test above.
    #[test]
    fn release_removes_claim_then_reports_false() {
        let root = tempfile::tempdir().expect("tempdir");
        let lock_dir = root.path().join(".fleet-locks");

        register_at(
            &lock_dir,
            "release-agent",
            "release-repo",
            "release-lane",
            "release-roadmap",
            None,
        )
        .expect("initial register must succeed");

        let before = view_for(root.path());
        assert!(
            before.registry.iter().any(|e| e
                .claim
                .typed()
                .is_some_and(|c| c.agent_name == "release-agent")),
            "claim must exist before release"
        );

        let removed_first = release_at(&lock_dir, "release-agent").expect("release must succeed");
        assert!(
            removed_first,
            "first release of a real claim must report removed: true"
        );

        let after = view_for(root.path());
        assert!(
            !after.registry.iter().any(|e| e
                .claim
                .typed()
                .is_some_and(|c| c.agent_name == "release-agent")),
            "claim must be gone after release"
        );

        let removed_second =
            release_at(&lock_dir, "release-agent").expect("second release must not error");
        assert!(
            !removed_second,
            "releasing an already-absent claim must report removed: false, not error"
        );
    }

    /// `run_register`/`run_heartbeat`/`run_release` all honour `--lock-dir`'s override rather
    /// than falling through to `resolve_brain_root()` — the property every fixture test above
    /// relies on to never touch the live shared `.fleet-locks/`. Exercised through the public
    /// `run_*` entry points themselves (not just the `*_at` helpers), so a regression that
    /// dropped the override before it reached `resolve_write_lock_dir` would be caught here.
    #[test]
    fn run_entry_points_honour_lock_dir_override() {
        let root = tempfile::tempdir().expect("tempdir");
        let lock_dir = root.path().join(".fleet-locks");

        run_register(
            "override-agent",
            "override-repo",
            "override-lane",
            "override-roadmap",
            None,
            Some(lock_dir.as_path()),
        )
        .expect("run_register with --lock-dir override must succeed");
        assert!(
            lock_dir.join("lane-agents").exists(),
            "register must have written under the overridden lock_dir"
        );

        run_heartbeat("override-agent", None, None, Some(lock_dir.as_path()))
            .expect("run_heartbeat with --lock-dir override must succeed");

        run_release("override-agent", Some(lock_dir.as_path()))
            .expect("run_release with --lock-dir override must succeed");
        let claim_path = lock_dir
            .join("lane-agents")
            .join("agent-override-agent.json");
        assert!(
            !claim_path.exists(),
            "release must have removed the claim under the overridden lock_dir"
        );
    }
}
