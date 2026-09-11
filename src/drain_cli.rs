//! `bastion drain --once --lane <repo>/<lane> [--profile <name>]` — CLI face for engine-core's
//! `COMMANDER` workflow (`BA.25.D` task 2).
//!
//! This module owns NO drain logic of its own — it dispatches the already-registered
//! `COMMANDER` workflow in-process (`engine_serve::workflows::register_commander` +
//! `Dispatcher::dispatch_with_event`, the same registration `engine-serve`'s own HTTP layer
//! uses) and prints a human summary of the resulting `TaskContext`. Reimplementing the
//! discover/route/complete/emit pipeline is out of scope — that's `engine-rs`'s `EN.15.F`.
//!
//! ## The permission profile flows into the event, not into `CommanderDrainNode` itself
//!
//! [`crate::permission_profile::resolve_requested_profile`] (shared with `sweep_cli.rs`,
//! moved to its own module in this task) resolves `--profile` exactly as `sweep --once` does —
//! refused, never substituted, on an unrecognized or unresolvable name. The resolved profile
//! string is threaded into the `COMMANDER` event's `"profile"` field, which
//! `CommanderTriageNode` reads for its own gated `AgentCodeStep`
//! (`crates/engine-core/src/workflows/commander/mod.rs`'s `resolve_event_profile`) —
//! `CommanderDrainNode` itself never consults it.
//!
//! ## A `refused` emit is REPORTED, not swallowed and not a process error
//!
//! `CommanderDrainNode` runs the scoped `mev` emit + manifest-only commit as one of its steps
//! and records the outcome's `status` (`committed` / `noop` / `refused` / `failed`) under
//! `ctx.nodes["CommanderDrainNode"]["emit"]["status"]`. A `refused` status — the foreign-lease
//! case (AC-1) — is printed here as a reported condition and the process still exits `0`: the
//! drain itself (discover/route/complete) succeeded independently of the emit step. A `failed`
//! emit status is printed the same way, for the same reason — the two are never conflated.
//!
//! ## What this module does NOT do
//!
//! No `Commands::Drain` CLI variant is wired here — that is task 3's job
//! (`planning/BA.25.D/tasks.json`). This module is dispatched to, not dispatched from.

use std::path::Path;

use anyhow::{Context, Result};
use engine_core::dispatch::Dispatcher;

use crate::permission_profile::resolve_requested_profile;

/// Split `lane` (the block record's own CLI shape, `--lane <repo>/<lane>`) on the FIRST `/`
/// into `(repo, lane_name)`. `Err` naming the literal `lane` string given when it contains no
/// `/`, or more than one — a malformed value is refused before any dispatch happens, never
/// silently truncated.
fn parse_lane(lane: &str) -> Result<(&str, &str)> {
    let parts: Vec<&str> = lane.split('/').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        anyhow::bail!(
            "--lane \"{lane}\" is not a valid `<repo>/<lane>` value — expected exactly one `/` \
             separating a non-empty repo and a non-empty lane name"
        );
    }
    Ok((parts[0], parts[1]))
}

/// `bastion drain --once --lane <repo>/<lane> [--profile <name>]` — resolve `brain_root` exactly
/// as every other bastion write verb does (`engine_core::brain_root::resolve_brain_root()`) and
/// dispatch to [`run_drain_once_at`].
pub async fn run_drain_once(lane: &str, profile: Option<&str>) -> Result<()> {
    let brain_root =
        engine_core::brain_root::resolve_brain_root().context("cannot resolve brain root")?;
    run_drain_once_at(&brain_root, lane, profile).await
}

/// `run_drain_once` against an already-resolved `root` — the pure(ish) core [`run_drain_once`]
/// wraps, mirroring `sweep_cli.rs`'s `run_sweep_once`/`run_sweep_once_at` split so a test can
/// drive this against a `tempdir()` fixture. Prints [`drain_summary_at`]'s human summary.
pub async fn run_drain_once_at(root: &Path, lane: &str, profile: Option<&str>) -> Result<()> {
    println!("{}", drain_summary_at(root, lane, profile).await?);
    Ok(())
}

/// The core of [`run_drain_once_at`], returning the rendered summary rather than printing it —
/// what every fixture test in this module asserts against directly, so a test can check the
/// EXACT printed text (e.g. "emit status: refused") rather than only that the call succeeded.
///
/// Resolves `lane` and the permission profile FIRST — either failing refuses before any
/// dispatch happens. Then builds a fresh [`Dispatcher`], registers `COMMANDER`
/// (`engine_serve::workflows::register_commander`), and dispatches the event
/// `CommanderDrainNode::process` reads: `{"root", "repo", "agent", "profile"}` — `lock_dir`,
/// `dir`, `roadmap`, `drain_log_path`, `heartbeat_name`, and `now` are left at their documented
/// defaults (this task does not need to override them).
async fn drain_summary_at(root: &Path, lane: &str, profile: Option<&str>) -> Result<String> {
    let (repo, agent) = parse_lane(lane)?;

    let brain_toml_path = root.join("brain.toml");
    let config = mev::brain::config::load_brain_config(&brain_toml_path)
        .with_context(|| format!("failed to load brain.toml at {}", brain_toml_path.display()))?;
    let resolved_profile = resolve_requested_profile(&config.permission_profiles, profile)
        .map_err(|message| anyhow::anyhow!(message))?;

    let event = serde_json::json!({
        "root": root.display().to_string(),
        "repo": repo,
        "agent": agent,
        "profile": resolved_profile,
    });

    let mut dispatcher = Dispatcher::new();
    engine_serve::workflows::register_commander(&mut dispatcher);
    let workflow = dispatcher
        .dispatch_with_event("COMMANDER", &event)
        .map_err(|err| anyhow::anyhow!("failed to dispatch COMMANDER: {err}"))?;

    let ctx = workflow
        .run(event, Box::new(|_ctx| {}))
        .await
        .map_err(|err| anyhow::anyhow!("COMMANDER run failed: {err}"))?;

    Ok(drain_summary(&ctx, repo, agent))
}

/// Render a human summary of `ctx.nodes["CommanderDrainNode"]`'s output — queues discovered,
/// messages drained/completed, and the emit outcome's `status` verbatim. Pure and independently
/// testable from the dispatch/run above.
fn drain_summary(ctx: &engine_contract::TaskContext, repo: &str, agent: &str) -> String {
    let Some(node_output) = ctx.nodes.get("CommanderDrainNode") else {
        return format!(
            "bastion drain: no CommanderDrainNode output found for {repo}/{agent} — the \
             COMMANDER workflow may not have reached its start node"
        );
    };

    let queues_discovered = node_output
        .get("queues_discovered")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let drained = node_output
        .get("drained")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let completed = node_output
        .get("completed")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let emit_status = node_output
        .get("emit")
        .and_then(|emit| emit.get("status"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let emit_reason = node_output
        .get("emit")
        .and_then(|emit| emit.get("reason"))
        .and_then(serde_json::Value::as_str);

    let mut lines = vec![
        format!("bastion drain: {repo}/{agent}"),
        format!("bastion drain: queues discovered: {queues_discovered}"),
        format!("bastion drain: drained: {drained}, completed: {completed}"),
        format!("bastion drain: emit status: {emit_status}"),
    ];
    if let Some(reason) = emit_reason {
        lines.push(format!("bastion drain: emit reason: {reason}"));
    }
    lines.join("\n")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_lane ────────────────────────────────────────────────────────────

    #[test]
    fn parse_lane_splits_repo_and_lane_on_first_slash() {
        let (repo, lane) = parse_lane("bastion/console").expect("valid lane");
        assert_eq!(repo, "bastion");
        assert_eq!(lane, "console");
    }

    /// AC: an unparsable `--lane` value (no `/`) returns an `Err` naming the literal value
    /// given, before any dispatch happens.
    #[test]
    fn parse_lane_with_no_slash_is_refused() {
        let err = parse_lane("bastion-console").expect_err("no slash must be refused");
        assert!(err.to_string().contains("bastion-console"));
    }

    /// AC: more than one `/` is also refused, naming the literal value.
    #[test]
    fn parse_lane_with_more_than_one_slash_is_refused() {
        let err = parse_lane("bastion/console/extra").expect_err("extra slash must be refused");
        assert!(err.to_string().contains("bastion/console/extra"));
    }

    #[test]
    fn parse_lane_with_empty_repo_or_lane_is_refused() {
        assert!(parse_lane("/console").is_err());
        assert!(parse_lane("bastion/").is_err());
    }

    // ── fixture plumbing shared by the run_drain_once_at tests ──────────────────────

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

[[repos]]
slug = "bastion"
tier = "core"
repo_path = "bastion"
status_file = "bastion/planning/status.md"
cache_doc = "docs/projects/bastion.md"

[[repos]]
slug = "hq"
repo_path = "."
status_file = "planning/status.md"
cache_doc = "docs/projects/hq.md"
"#;

    /// Write `brain.toml` plus the minimal `<root>/bastion/planning/state.json`
    /// `BrainConfig::scope_dependencies("bastion")` needs to resolve — mirroring
    /// `engine_core::workflows::commander::emit_commit`'s own `brain_fixture` helper, so the
    /// scoped emit this task's `run_drain_once_at` triggers gets far enough to actually reach
    /// `mev::emit_state_as` (and, in the foreign-lease test, its quiesce check) rather than
    /// failing earlier on `E_EMIT_UNKNOWN_SCOPE`.
    fn write_valid_brain_toml(root: &Path) {
        std::fs::write(root.join("brain.toml"), VALID_BRAIN_TOML).expect("write brain.toml");
        let planning_dir = root.join("bastion").join("planning");
        std::fs::create_dir_all(&planning_dir).expect("mkdir planning");
        std::fs::write(
            planning_dir.join("state.json"),
            r#"{ "repo": "bastion", "kind": "project", "updated": "2026-08-20",
  "focus": { "now": [], "next": [], "blocked": [] },
  "tracks": [{ "title": "P1", "blocks": [] }] }"#,
        )
        .expect("write state.json");
    }

    /// Write one live, `exclusive`, `scope: repo` lease file under
    /// `<root>/.fleet-locks/leases/` — the on-disk shape `mev::brain::lease::check_quiesce`
    /// reads, mirroring `engine_core::workflows::commander::emit_commit`'s own `write_lease`
    /// test helper. `agent` is a DIFFERENT identity than the one this task's drain runs under,
    /// so the emit step is refused by a genuinely FOREIGN lease — the block record's own
    /// known-bad-input rule (D68): copy an actual lease shape from `.fleet-locks/leases/`
    /// rather than inventing one.
    fn write_foreign_lease(root: &Path, repo: &str, foreign_agent: &str) {
        let leases_dir = root.join(".fleet-locks").join("leases");
        std::fs::create_dir_all(&leases_dir).expect("mkdir .fleet-locks/leases");
        let now = chrono::Utc::now().to_rfc3339();
        let raw = serde_json::json!({
            "acquired_at": now,
            "agent": foreign_agent,
            "heartbeat": now,
            "kind": "exclusive",
            "lane": "console",
            "repo": repo,
        });
        std::fs::write(
            leases_dir.join(format!("lease-{repo}.json")),
            serde_json::to_string_pretty(&raw).expect("serialize lease fixture"),
        )
        .expect("write lease fixture");
    }

    fn init_git_repo(dir: &Path) {
        std::fs::create_dir_all(dir).expect("create repo dir");
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(dir)
                .output()
                .expect("run git");
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "Test"]);
        std::fs::write(dir.join(".gitkeep"), "").expect("write .gitkeep");
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "init"]);
    }

    /// AC: `run_drain_once_at` against a fixture `.fleet-locks` tree carrying one of the
    /// repo's existing agent-written lease shapes (a DIFFERENT agent than this drain's own)
    /// reports the emit outcome's status as `refused` in its printed output, and the process
    /// exits `Ok` rather than erroring.
    #[tokio::test]
    async fn run_drain_once_at_reports_a_foreign_lease_refusal_without_erroring() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_valid_brain_toml(tmp.path());
        let repo_dir = tmp.path().join("bastion");
        init_git_repo(&repo_dir);
        write_foreign_lease(tmp.path(), "bastion", "some-other-lane-holder");

        let summary = drain_summary_at(tmp.path(), "bastion/console", None)
            .await
            .expect("a refused emit must be reported, not turned into a process error");
        assert!(
            summary.contains("emit status: refused"),
            "expected the refusal to be reported in the printed summary, got: {summary}"
        );
    }

    /// AC: `run_drain_once_at` against a `<repo>/<lane>` fixture with no queue messages at all
    /// still succeeds (prints a summary with zero counts) rather than failing — mirrors
    /// `CommanderDrainNode`'s own "re-derives, never detects" contract.
    #[tokio::test]
    async fn run_drain_once_at_with_no_messages_still_succeeds() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_valid_brain_toml(tmp.path());
        let repo_dir = tmp.path().join("bastion");
        init_git_repo(&repo_dir);

        let summary = drain_summary_at(tmp.path(), "bastion/console", None)
            .await
            .expect("an empty queue tree must not fail");
        assert!(summary.contains("queues discovered: 0"));
        assert!(summary.contains("drained: 0, completed: 0"));
    }

    /// An unparsable `--lane` value is refused before `run_drain_once_at` does any I/O at all
    /// (no `brain.toml` fixture is even written for this tempdir).
    #[tokio::test]
    async fn run_drain_once_at_refuses_a_malformed_lane_before_any_dispatch() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let err = run_drain_once_at(tmp.path(), "no-slash-here", None)
            .await
            .expect_err("a malformed --lane must be refused");
        assert!(err.to_string().contains("no-slash-here"));
    }

    // ── drain_summary ────────────────────────────────────────────────────────

    #[test]
    fn drain_summary_reports_refused_emit_status() {
        let ctx = engine_contract::TaskContext {
            event: serde_json::json!({}),
            nodes: [(
                "CommanderDrainNode".to_string(),
                serde_json::json!({
                    "queues_discovered": 1,
                    "drained": 0,
                    "completed": 0,
                    "emit": {"status": "refused", "reason": "foreign lease held by X"},
                }),
            )]
            .into_iter()
            .collect(),
            metadata: serde_json::json!({}),
            node_runs: std::collections::HashMap::new(),
        };
        let summary = drain_summary(&ctx, "bastion", "console");
        assert!(summary.contains("emit status: refused"));
        assert!(summary.contains("foreign lease held by X"));
    }

    #[test]
    fn drain_summary_reports_zero_counts_for_an_empty_drain() {
        let ctx = engine_contract::TaskContext {
            event: serde_json::json!({}),
            nodes: [(
                "CommanderDrainNode".to_string(),
                serde_json::json!({
                    "queues_discovered": 0,
                    "drained": 0,
                    "completed": 0,
                    "emit": {"status": "noop"},
                }),
            )]
            .into_iter()
            .collect(),
            metadata: serde_json::json!({}),
            node_runs: std::collections::HashMap::new(),
        };
        let summary = drain_summary(&ctx, "bastion", "console");
        assert!(summary.contains("queues discovered: 0"));
        assert!(summary.contains("drained: 0, completed: 0"));
        assert!(summary.contains("emit status: noop"));
    }
}
