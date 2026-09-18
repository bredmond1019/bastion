// Dead code is expected during incremental scaffold build-out.
// Remove this attribute once all phases are wired up.
#![allow(dead_code)]

mod api;
mod assess;
mod brain;
mod brainval;
mod buildstamp;
mod cli;
mod config;
mod coord_cli;
mod costs;
mod db;
mod docview;
mod drain_cli;
mod inspect;
mod man;
mod momentum;
mod monitor;
mod notify;
mod notify_cli;
mod observ;
mod openwork;
mod overview;
mod permission_profile;
mod roadmap_status_cli;
mod run;
mod runs;
mod serve;
mod sessions;
mod sweep_cli;
#[cfg(test)]
mod testsupport;
mod ui_theme;
mod validate;

// Detect engine moved to term-core (BA.18.F Phase 0b extraction); re-exported
// here so every existing `crate::detect::*` path in serve/ and sessions/
// keeps resolving unchanged.
pub use term_core::detect;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Commands, CoordMode, NotifyMode};
use observ::errors::{ConsoleError, ErrorCode};

// ── Pure helpers (unit-tested below) ─────────────────────────────────────────

/// Default index db path when no config override is in play (config override
/// itself lands in a later task): `$(git rev-parse --git-common-dir)/bastion-code/index.sqlite`,
/// resolved relative to `repo_root`.
fn default_code_index_db_path(repo_root: &std::path::Path) -> Result<std::path::PathBuf> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["rev-parse", "--git-common-dir"])
        .output()
        .map_err(|e| anyhow::anyhow!("failed to spawn git rev-parse --git-common-dir: {e}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "git rev-parse --git-common-dir failed (exit {:?}): {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let git_common_dir = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let git_common_dir_path = std::path::PathBuf::from(git_common_dir);
    let git_common_dir_abs = if git_common_dir_path.is_absolute() {
        git_common_dir_path
    } else {
        repo_root.join(git_common_dir_path)
    };
    Ok(git_common_dir_abs.join("bastion-code").join("index.sqlite"))
}

/// Every blob OID reachable from any ref, via `git rev-list --objects --all` —
/// the "cheaper" of task 2's two suggested strategies for computing the
/// `--prune` live set. Deliberately over-inclusive (also collects commit/tree
/// OIDs alongside blob OIDs from the same line stream) rather than
/// under-inclusive: `prune_unreachable` only ever deletes rows whose blob_oid
/// is ABSENT from this set, so an over-inclusive set can only under-prune,
/// never wrongly delete a still-reachable blob's cache row.
fn live_oids_from_all_refs(
    repo_root: &std::path::Path,
) -> Result<std::collections::HashSet<String>> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["rev-list", "--objects", "--all"])
        .output()
        .map_err(|e| anyhow::anyhow!("failed to spawn git rev-list --objects --all: {e}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "git rev-list --objects --all failed (exit {:?}): {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(|s| s.to_string())
        .collect())
}

/// One JSON-batch line's parsed query — `{"def": "<name>"}` / `{"refs": "<name>"}` /
/// `{"dependents": "<name>"}`, mirroring the existing bare `--def`/`--refs`/
/// `--dependents` flags' shape for `bastion code query --json`'s stdin protocol.
#[derive(serde::Deserialize)]
struct StdinCodeQuery {
    def: Option<String>,
    refs: Option<String>,
    dependents: Option<String>,
}

impl TryFrom<StdinCodeQuery> for brain::code_graph::CodeQuery {
    type Error = anyhow::Error;

    fn try_from(q: StdinCodeQuery) -> Result<Self> {
        match (q.def, q.refs, q.dependents) {
            (Some(name), None, None) => Ok(brain::code_graph::CodeQuery::Def(name)),
            (None, Some(name), None) => Ok(brain::code_graph::CodeQuery::Refs(name)),
            (None, None, Some(name)) => Ok(brain::code_graph::CodeQuery::Dependents(name)),
            _ => anyhow::bail!(
                "each stdin query line must set exactly one of \"def\"/\"refs\"/\"dependents\""
            ),
        }
    }
}

#[cfg(test)]
mod stdin_code_query_tests {
    use super::StdinCodeQuery;
    use crate::brain::code_graph::CodeQuery;

    /// `StdinCodeQuery`'s `TryFrom` (task 4's `query --json` batch path) must
    /// enforce exactly one of `def`/`refs`/`dependents`, mirroring the bare
    /// `--def`/`--refs`/`--dependents` flags' `ArgGroup` contract — this impl
    /// shipped in task 4 with no direct unit test of its own until now.
    #[test]
    fn stdin_code_query_accepts_exactly_one_field() {
        let def = StdinCodeQuery {
            def: Some("Foo".into()),
            refs: None,
            dependents: None,
        };
        assert!(matches!(
            CodeQuery::try_from(def).expect("def-only should parse"),
            CodeQuery::Def(name) if name == "Foo"
        ));

        let refs = StdinCodeQuery {
            def: None,
            refs: Some("Bar".into()),
            dependents: None,
        };
        assert!(matches!(
            CodeQuery::try_from(refs).expect("refs-only should parse"),
            CodeQuery::Refs(name) if name == "Bar"
        ));

        let dependents = StdinCodeQuery {
            def: None,
            refs: None,
            dependents: Some("Baz".into()),
        };
        assert!(matches!(
            CodeQuery::try_from(dependents).expect("dependents-only should parse"),
            CodeQuery::Dependents(name) if name == "Baz"
        ));
    }

    #[test]
    fn stdin_code_query_rejects_none_or_many_fields() {
        let none = StdinCodeQuery {
            def: None,
            refs: None,
            dependents: None,
        };
        assert!(CodeQuery::try_from(none).is_err());

        let both = StdinCodeQuery {
            def: Some("Foo".into()),
            refs: Some("Foo".into()),
            dependents: None,
        };
        assert!(CodeQuery::try_from(both).is_err());
    }
}

#[cfg(test)]
mod run_code_action_registry_tests {
    use crate::testsupport;

    /// `run_code_action` (task 4) loads the workspace registry through
    /// `config::load_workspace_registry(XDG_CONFIG_HOME, HOME)` exactly like
    /// every other DB-free command on this CLI — the same call that task 4's
    /// refactor relocated (not removed) from the bare `--def`/`--refs`/
    /// `--dependents` arm into this dedicated dispatch function. Proves that
    /// exact call still resolves a `[workspaces]` entry from a fixture config
    /// pointed at by `XDG_CONFIG_HOME`, so the relocation changed nothing
    /// observable about config loading for the new index/query/status verbs.
    #[test]
    fn run_code_action_registry_load_honors_xdg_config_home() {
        let env_lock = testsupport::lock_env();
        let dir = tempfile::tempdir().expect("tempdir");
        let config_dir = dir.path().join("bastion");
        std::fs::create_dir_all(&config_dir).expect("create config dir");
        std::fs::write(
            config_dir.join("config.toml"),
            "[workspaces]\nfoo = \"foo\"\n",
        )
        .expect("write fixture config");

        let _xdg = testsupport::EnvVarGuard::set(
            &env_lock,
            "XDG_CONFIG_HOME",
            &dir.path().to_string_lossy(),
        );
        let _home = testsupport::EnvVarGuard::unset(&env_lock, "HOME");

        let registry = crate::config::load_workspace_registry(
            std::env::var("XDG_CONFIG_HOME").ok(),
            std::env::var("HOME").ok(),
        )
        .expect("run_code_action's registry-loading call must still succeed");
        assert!(
            registry
                .workspaces
                .as_ref()
                .is_some_and(|ws| ws.contains_key("foo")),
            "expected the fixture's [workspaces] entry to load, got {registry:?}"
        );
    }
}

/// Combined report for `bastion code status`: this run's cache hit/miss/reparse
/// counters against the current blob set (warming the cache as a side effect,
/// same as any other query would) alongside the index's total row/blob counts.
#[derive(Debug, serde::Serialize)]
struct CodeStatusReport {
    hits: usize,
    misses: usize,
    reparses: usize,
    total_rows: usize,
    distinct_blobs: usize,
}

/// Dispatch for `bastion code <index|query|status>` (task 4) — the index-backed
/// verbs alongside the pre-existing bare `--def`/`--refs`/`--dependents` flags
/// on `Commands::Code` (handled by the caller when `action` is `None`).
fn run_code_action(action: cli::CodeAction) -> Result<()> {
    let registry = config::load_workspace_registry(
        std::env::var("XDG_CONFIG_HOME").ok(),
        std::env::var("HOME").ok(),
    )?;

    match action {
        cli::CodeAction::Index {
            rev,
            staged,
            prune,
            root,
            workspace,
        } => {
            let (resolved_root, root_source) =
                config::resolve_cli_root_from_cwd(root, workspace.as_deref(), &registry)
                    .map_err(anyhow::Error::from)?;
            eprintln!(
                "code index: root {} ({})",
                resolved_root.display(),
                root_source
            );
            let db_path = default_code_index_db_path(&resolved_root)?;
            let conn = brain::code_index::open_or_create_index(&db_path)?;
            let blob_list =
                brain::code_index::assemble_blob_list(&resolved_root, rev.as_deref(), staged)?;
            let (_symbols, _refs, counters) =
                brain::code_graph::load_symbols_refs_via_index(&conn, &resolved_root, &blob_list);
            println!(
                "code index: {} blobs — {} hits, {} misses, {} reparses",
                blob_list.len(),
                counters.hits,
                counters.misses,
                counters.reparses
            );
            if prune {
                let live_oids = live_oids_from_all_refs(&resolved_root)?;
                let removed = brain::code_index::prune_unreachable(&conn, &live_oids)?;
                println!("code index: pruned {removed} unreachable row(s)");
            }
            Ok(())
        }
        cli::CodeAction::Query {
            json: _json,
            rev,
            staged,
            root,
            workspace,
        } => {
            let (resolved_root, root_source) =
                config::resolve_cli_root_from_cwd(root, workspace.as_deref(), &registry)
                    .map_err(anyhow::Error::from)?;
            eprintln!(
                "code query: root {} ({})",
                resolved_root.display(),
                root_source
            );
            let db_path = default_code_index_db_path(&resolved_root)?;
            let conn = brain::code_index::open_or_create_index(&db_path)?;
            let blob_list =
                brain::code_index::assemble_blob_list(&resolved_root, rev.as_deref(), staged)?;
            let (all_symbols, all_refs, _counters) =
                brain::code_graph::load_symbols_refs_via_index(&conn, &resolved_root, &blob_list);
            let (nodes, edges) =
                brain::code_graph::build_code_node_edge_lists(&all_symbols, &all_refs);
            let graph = brain::graph::BrainGraph::build(nodes, edges);
            let root_str = resolved_root.display().to_string();

            use std::io::BufRead;
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                let line = line?;
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let parsed: StdinCodeQuery = serde_json::from_str(trimmed)
                    .map_err(|e| anyhow::anyhow!("invalid query line {trimmed:?}: {e}"))?;
                let query: brain::code_graph::CodeQuery = parsed.try_into()?;
                let line_out = match &query {
                    brain::code_graph::CodeQuery::Def(name) => {
                        let defs = brain::code_graph::find_definition(&all_symbols, name);
                        let envelope =
                            brain::code_graph::build_def_json_envelope(&root_str, name, &defs);
                        serde_json::to_string(&envelope)?
                    }
                    brain::code_graph::CodeQuery::Refs(name) => {
                        let references = brain::code_graph::find_references(&all_refs, name);
                        let envelope = brain::code_graph::build_refs_json_envelope(
                            &root_str,
                            name,
                            &references,
                        );
                        serde_json::to_string(&envelope)?
                    }
                    brain::code_graph::CodeQuery::Dependents(name) => {
                        let callers = graph.predecessors_by_name(name);
                        let envelope = brain::code_graph::build_dependents_json_envelope(
                            &root_str, name, &callers,
                        );
                        serde_json::to_string(&envelope)?
                    }
                };
                println!("{line_out}");
            }
            Ok(())
        }
        cli::CodeAction::Status {
            json,
            root,
            workspace,
        } => {
            let (resolved_root, root_source) =
                config::resolve_cli_root_from_cwd(root, workspace.as_deref(), &registry)
                    .map_err(anyhow::Error::from)?;
            if !json {
                eprintln!(
                    "code status: root {} ({})",
                    resolved_root.display(),
                    root_source
                );
            }
            let db_path = default_code_index_db_path(&resolved_root)?;
            let conn = brain::code_index::open_or_create_index(&db_path)?;
            let blob_list = brain::code_index::assemble_blob_list(&resolved_root, None, false)?;
            let (_symbols, _refs, counters) =
                brain::code_graph::load_symbols_refs_via_index(&conn, &resolved_root, &blob_list);
            let stats = brain::code_index::index_stats(&conn)?;
            let report = CodeStatusReport {
                hits: counters.hits,
                misses: counters.misses,
                reparses: counters.reparses,
                total_rows: stats.total_rows,
                distinct_blobs: stats.distinct_blobs,
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "code status: {} hits, {} misses, {} reparses — {} total rows, {} distinct blobs",
                    report.hits,
                    report.misses,
                    report.reparses,
                    report.total_rows,
                    report.distinct_blobs
                );
            }
            Ok(())
        }
    }
}

/// Split `lane` (`bastion attach`'s own CLI shape, `<repo>/<lane>`) on the FIRST `/` into
/// `(repo, lane_name)` — mirroring `drain_cli`'s own `parse_lane` convention for the sibling
/// `bastion drain --lane <repo>/<lane>` shape (`BA.25.D`). `Err` naming the literal `lane`
/// string given when it contains no `/`, or more than one — refused before any file I/O,
/// never silently truncated.
fn parse_attach_lane(lane: &str) -> Result<(&str, &str)> {
    let parts: Vec<&str> = lane.split('/').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        anyhow::bail!(
            "attach \"{lane}\" is not a valid `<repo>/<lane>` value — expected exactly one `/` \
             separating a non-empty repo and a non-empty lane name"
        );
    }
    Ok((parts[0], parts[1]))
}

/// Resolve the canonical name string for a subcommand variant (pure).
///
/// Returns a `&'static str` so the value can be captured before `cli` is
/// consumed by `dispatch`.
fn command_name(cmd: &Commands) -> &'static str {
    match cmd {
        Commands::Tui => "tui",
        Commands::Monitor { .. } => "monitor",
        Commands::Inspect { .. } => "inspect",
        Commands::Overview => "overview",
        Commands::Validate { .. } => "validate",
        Commands::Costs { .. } => "costs",
        Commands::Run { .. } => "run",
        Commands::Abort { .. } => "abort",
        Commands::Status => "status",
        Commands::Momentum => "momentum",
        Commands::Sessions => "sessions",
        Commands::Attach { .. } => "attach",
        Commands::New { .. } => "new",
        Commands::Kill { .. } => "kill",
        Commands::Send { .. } => "send",
        Commands::Capture { .. } => "capture",
        Commands::Ask { .. } => "ask",
        Commands::Man { .. } => "man",
        Commands::Brain { .. } => "brain",
        Commands::ValidateBrain { .. } => "validate-brain",
        Commands::Manifest { .. } => "manifest",
        Commands::Graph { .. } => "graph",
        Commands::EmitState { .. } => "emit-state",
        Commands::Code { .. } => "code",
        Commands::Serve { .. } => "serve",
        Commands::View { .. } => "view",
        Commands::Edit { .. } => "edit",
        Commands::Assess { .. } => "assess",
        Commands::Notify { .. } => "notify",
        Commands::Coord { .. } => "coord",
        Commands::RoadmapStatus { .. } => "roadmap-status",
        Commands::Sweep { .. } => "sweep",
        Commands::Drain { .. } => "drain",
    }
}

/// Resolve the function `dispatch` calls for `bastion overview` (pure).
///
/// Returned as a bare `fn() -> Result<()>` pointer so a test can assert
/// exactly what `dispatch`'s `Commands::Overview` arm invokes — the new
/// open-work renderer (`overview::run_sections_ui`, BA.26.G task 3) — without
/// running the binary or driving a real terminal. The parked Kanban entry
/// point, `overview::run`, stays reachable as code (BA.26.I's decision D20)
/// but this function must never return it; that is the concrete meaning of
/// "re-pointed".
fn overview_dispatch_target() -> fn() -> Result<()> {
    overview::run_sections_ui
}

/// Best-effort classification of an `anyhow` error into a `C0xx` code (pure).
///
/// First attempts a typed downcast to `ConsoleError`; falls back to
/// inspecting the error message chain for known keywords. Returns
/// `ErrorCode::InvalidInput` when the error is unclassifiable.
fn classify_error(err: &anyhow::Error) -> ErrorCode {
    // Typed downcast — exact classification when the error is already a ConsoleError.
    if let Some(ce) = err.downcast_ref::<ConsoleError>() {
        return ce.code();
    }

    // Downcast to std::io::Error for OS-level I/O errors.
    if let Some(io_err) = err.downcast_ref::<std::io::Error>() {
        return match io_err.kind() {
            std::io::ErrorKind::PermissionDenied => ErrorCode::PermissionDenied,
            std::io::ErrorKind::TimedOut => ErrorCode::Timeout,
            std::io::ErrorKind::InvalidData => ErrorCode::Utf8Error,
            _ => ErrorCode::IoError,
        };
    }

    // Keyword heuristics over the full error chain.
    let msg = format!("{err:#}").to_lowercase();

    if msg.contains("not found in path") || msg.contains("binary not found") {
        return ErrorCode::BinaryNotFound;
    }
    if msg.contains("permission denied") {
        return ErrorCode::PermissionDenied;
    }
    if msg.contains("timed out") || msg.contains("timeout") {
        return ErrorCode::Timeout;
    }
    if msg.contains("rate limit") {
        return ErrorCode::RateLimitExceeded;
    }
    if msg.contains("utf-8") || msg.contains("utf8") || msg.contains("invalid utf") {
        return ErrorCode::Utf8Error;
    }
    if msg.contains("not authenticated") || msg.contains("run 'claude auth'") {
        return ErrorCode::NotAuthenticated;
    }
    if msg.contains("mcp server") {
        return ErrorCode::McpError;
    }
    if msg.contains("io error") || msg.contains("no such file") || msg.contains("broken pipe") {
        return ErrorCode::IoError;
    }
    if msg.contains("process error")
        || msg.contains("spawn")
        || msg.contains("tmux")
        || msg.contains("exit status")
    {
        return ErrorCode::ProcessError;
    }
    if msg.contains("configuration") || msg.contains("config error") {
        return ErrorCode::ConfigError;
    }
    if msg.contains("stream closed") {
        return ErrorCode::StreamClosed;
    }
    if msg.contains("serializ") || msg.contains("deserializ") || msg.contains("json") {
        return ErrorCode::SerializationError;
    }

    // Unclassifiable: use C006 (InvalidInput) as the generic fallback.
    ErrorCode::InvalidInput
}

// ── Command dispatch (all subcommand logic lives here) ───────────────────────

/// Execute the selected subcommand and return its result.
///
/// All I/O, async work, and error propagation happen here. `main` wraps this
/// with timing + structured event emission without touching the dispatch logic.
async fn dispatch(cli: Cli) -> Result<()> {
    match cli.command {
        // No subcommand or explicit `tui` → interactive session dashboard.
        // Synchronous call, consistent with the other session verbs (D5).
        None | Some(Commands::Tui) => sessions::ui::run(),

        Some(cmd) => match cmd {
            // Tui handled above — included to keep the match exhaustive.
            Commands::Tui => unreachable!(),
            Commands::Monitor { workflow_id, watch } => {
                if watch {
                    monitor::watch::run(workflow_id).await
                } else {
                    monitor::run(workflow_id).await
                }
            }
            Commands::Inspect { run_id } => inspect::run(run_id).await,
            Commands::Overview => overview_dispatch_target()(),
            Commands::Validate { path } => validate::run(path).await,
            Commands::Costs { last, watch } => costs::run(last, watch).await,
            Commands::Run {
                workflow,
                args,
                monitor,
                force,
            } => run::trigger(workflow, args, monitor, force).await,
            Commands::Abort { run, yes } => run::abort::run(run, yes).await,
            Commands::Status => run::status().await,
            // Momentum rollup is DB-free (D25 read-only) and synchronous — reads
            // `[workspaces]` status.md files directly, no orchestrator/Postgres coupling.
            Commands::Momentum => momentum::run(),
            // Sessions path is DB-free (D4): no Config::load(), no Postgres pool.
            // All session verbs are sync blocking (D5): no async/tokio coupling.
            Commands::Sessions => sessions::run(),
            Commands::Attach { lane } => {
                let (repo, lane_name) = parse_attach_lane(&lane)?;
                sessions::commands::attach_lane(repo, lane_name)
            }
            Commands::New { session, dir } => {
                sessions::commands::new(&session, dir.as_deref().and_then(|p| p.to_str()))
            }
            Commands::Kill { session } => sessions::commands::kill(&session),
            Commands::Send { session, cmd } => {
                let keys = cmd.join(" ");
                sessions::commands::send(&session, &keys)
            }
            Commands::Capture { session, lines } => sessions::commands::capture(&session, lines),
            // `ask` is DB-free (D4) and synchronous (D5) — lives on the sessions surface.
            Commands::Ask {
                session,
                prompt_file,
                out,
                dir,
                timeout,
                launch_cmd,
            } => {
                let args = sessions::ask::AskArgs {
                    session,
                    prompt_file,
                    out,
                    dir,
                    timeout_secs: timeout,
                    launch_cmd,
                };
                sessions::ask::ask(args).map_err(|e| anyhow::anyhow!("{e}"))
            }
            Commands::Man { out } => man::run(out),
            // Brain is DB-free (D4) and synchronous — lives on the knowledge-graph surface.
            // Load only the workspace registry (no DATABASE_URL required).
            Commands::Brain {
                dependents,
                blast_radius,
                lineage,
                root,
                workspace,
                json,
            } => {
                let query = if let Some(id) = dependents {
                    brain::BrainQuery::Dependents(id)
                } else if let Some(id) = blast_radius {
                    brain::BrainQuery::BlastRadius(id)
                } else if let Some(id) = lineage {
                    brain::BrainQuery::Lineage(id)
                } else {
                    // Unreachable: clap ArgGroup enforces exactly one of the three flags.
                    unreachable!("clap ArgGroup guarantees exactly one query flag is set")
                };
                // Load workspace registry DB-free: absent/unreadable → empty registry;
                // malformed TOML → propagated error (non-zero exit with diagnostic).
                let registry = config::load_workspace_registry(
                    std::env::var("XDG_CONFIG_HOME").ok(),
                    std::env::var("HOME").ok(),
                )?;
                brain::run(query, root, workspace, &registry, json)
            }
            // ValidateBrain is DB-free (D4) and synchronous — thin pass-through to the `mev`
            // path-dep library (D15 / BA.15.2). No mev/bella source is touched.
            Commands::ValidateBrain {
                path,
                sync,
                graph,
                state,
                links,
                structure,
                json,
            } => brainval::run(path, sync, graph, state, links, structure, json),
            // Manifest / Graph / EmitState are DB-free (D4) and synchronous — thin
            // pass-throughs to the `mev` path-dep library (D15 / BA.15.2).
            Commands::Manifest { path, pretty } => brainval::run_manifest(path, pretty),
            Commands::Graph { path } => brainval::run_graph(path),
            Commands::EmitState {
                path,
                write,
                fail_on_drift,
                agent,
                scope,
            } => brainval::run_emit_state(path, write, fail_on_drift, agent, scope),
            // Serve is DB-free — does NOT call Config::load() or require DATABASE_URL.
            // The actix System runs on a dedicated OS thread (runtime-spike outcome, Task 1).
            Commands::Serve { addr, token } => {
                let serve_cfg =
                    config::load_serve_config(addr, token).map_err(|e| anyhow::anyhow!("{e}"))?;
                tokio::task::spawn_blocking(move || {
                    serve::run(
                        serve_cfg.addr,
                        serve_cfg.token,
                        serve_cfg.signing_key,
                        serve_cfg.clock_skew_secs,
                    )
                })
                .await
                .map_err(|e| anyhow::anyhow!("serve thread panicked: {e}"))?
            }
            // Code is DB-free and synchronous — lives on the knowledge-graph surface.
            // Resolves the scan root from the workspace registry (no DATABASE_URL required).
            Commands::Code {
                def,
                refs,
                dependents,
                root,
                workspace,
                json,
                action,
            } => {
                if let Some(action) = action {
                    run_code_action(action)
                } else {
                    let query = if let Some(name) = def {
                        brain::code_graph::CodeQuery::Def(name)
                    } else if let Some(name) = refs {
                        brain::code_graph::CodeQuery::Refs(name)
                    } else if let Some(name) = dependents {
                        brain::code_graph::CodeQuery::Dependents(name)
                    } else {
                        // Enforced here (not by a clap ArgGroup) because the group can no
                        // longer be `required(true)` once `action`'s nested subcommand
                        // shares the same `Code` variant — `bastion code index` must be
                        // able to parse with none of --def/--refs/--dependents set.
                        anyhow::bail!(
                            "one of --def, --refs, or --dependents is required when no \
                             `bastion code <action>` subcommand (index/query/status) is given"
                        );
                    };
                    let registry = config::load_workspace_registry(
                        std::env::var("XDG_CONFIG_HOME").ok(),
                        std::env::var("HOME").ok(),
                    )?;
                    brain::code_graph::run_code(query, root, workspace, &registry, json)
                }
            }
            // View/Edit are DB-free (D4) and synchronous — thin pass-throughs to the
            // `bella` terminal markdown viewer/editor over bella-engine (D14/BA.15.2).
            Commands::View { path } => docview::view(path),
            Commands::Edit { path } => docview::edit(path),
            // Assess is DB-free and synchronous (D5-style) and performs zero filesystem
            // writes end to end — a read-only repo diagnostic (Phase 15, Block BA.15.9).
            Commands::Assess { path, json } => assess::run::run(path, json),
            // Notify is DB-free (D4) — a thin I/O shell over the operator transport
            // (`BA.ticket.notify-operator-cli` task 5). `ask` terminates the process
            // directly for its own outcome contract (exit 0/2/3/4); `send` and any
            // config/validation/permanent-transport failure flow through this `Result`
            // as usual (exit 1 on `Err`).
            Commands::Notify { mode } => match mode {
                NotifyMode::Send { text, bot } => notify_cli::run_send(&bot, &text).await,
                NotifyMode::Ask {
                    gate_id,
                    summary,
                    option,
                    timeout_secs,
                    bot,
                    lock_dir,
                } => {
                    notify_cli::run_ask(
                        &bot,
                        gate_id,
                        &summary,
                        &option,
                        timeout_secs,
                        lock_dir.as_deref(),
                    )
                    .await
                }
            },
            // Coord is DB-free and synchronous — a thin CLI shell over engine-core's
            // read-only coordination reader (BA.25.A). No reader logic lives here.
            Commands::Coord { mode } => match mode {
                CoordMode::Status { json } => coord_cli::run_status(json),
                CoordMode::Register {
                    agent_name,
                    repo,
                    lane,
                    roadmap,
                    category,
                    lock_dir,
                } => coord_cli::run_register(
                    &agent_name,
                    &repo,
                    &lane,
                    &roadmap,
                    category.as_deref(),
                    lock_dir.as_deref(),
                ),
                CoordMode::Heartbeat {
                    agent_name,
                    current_block,
                    block_started_at,
                    lock_dir,
                } => coord_cli::run_heartbeat(
                    &agent_name,
                    current_block.as_deref(),
                    block_started_at.as_deref(),
                    lock_dir.as_deref(),
                ),
                CoordMode::Release {
                    agent_name,
                    lock_dir,
                } => coord_cli::run_release(&agent_name, lock_dir.as_deref()),
                CoordMode::Lease {
                    repo,
                    lane,
                    agent_name,
                    kind,
                    scope,
                    window,
                    lane_block,
                    lock_dir,
                } => coord_cli::run_lease(
                    &repo,
                    &lane,
                    &agent_name,
                    &kind,
                    scope.as_deref(),
                    &window,
                    &lane_block,
                    lock_dir.as_deref(),
                ),
                CoordMode::Unlease { repo, lock_dir } => {
                    coord_cli::run_unlease(&repo, lock_dir.as_deref())
                }
                CoordMode::Drain {
                    repo,
                    lane,
                    lock_dir,
                } => coord_cli::run_drain(&repo, &lane, lock_dir.as_deref()),
                CoordMode::Send {
                    repo,
                    lane,
                    file,
                    lock_dir,
                } => coord_cli::run_send(&repo, &lane, &file, lock_dir.as_deref()),
                CoordMode::Complete {
                    repo,
                    lane,
                    message_id,
                    lock_dir,
                } => coord_cli::run_complete(&repo, &lane, &message_id, lock_dir.as_deref()),
                CoordMode::Restore { lock_dir } => coord_cli::run_restore(lock_dir.as_deref()),
            },
            // Faces engine-core's typed roadmap-status join, kept beside `/roadmap-status`'s
            // Python path (BA.25.E). No discovery logic lives here; see `roadmap_status_cli`.
            Commands::RoadmapStatus { roadmap, json } => {
                roadmap_status_cli::run_roadmap_status(&roadmap, json)
            }
            // Sweep/Drain are the only two hand-woken faces for the Rust SWEEP and
            // COMMANDER workflows in this cut (BA.25.D, Fork 4 — no schedule). No
            // pipeline logic lives here; see `sweep_cli`/`drain_cli`.
            Commands::Sweep {
                roadmap,
                dry_run,
                profile,
            } => sweep_cli::run_sweep_once(&roadmap, dry_run, profile.as_deref()).await,
            Commands::Drain { lane, profile } => {
                drain_cli::run_drain_once(&lane, profile.as_deref()).await
            }
        },
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Handle --build-stamp before any subcommand dispatch or tracing init: it is a
    // top-level flag that must work with no subcommand present, and its output is a
    // single JSON line on stdout meant for machine consumers (mev's toolchain-freshness
    // check), not something to interleave with tracing.
    if cli.build_stamp {
        println!("{}", buildstamp::stamp_json());
        return Ok(());
    }

    // Install the global tracing subscriber before any dispatch.
    // verbose/json_logs are global flags parsed by clap before the subcommand.
    observ::init_tracing(cli.verbose, cli.json_logs);

    // Resolve the command name before cli is consumed by dispatch (pure call).
    let cmd_name: &'static str = cli.command.as_ref().map_or("tui", command_name);

    // Emit structured start event.
    observ::emit_start(cmd_name);
    let t0 = std::time::Instant::now();

    // Execute the subcommand.
    let result = dispatch(cli).await;

    // Compute wall-clock duration and emit outcome event.
    let duration_ms = t0.elapsed().as_millis() as u64;
    match &result {
        Ok(()) => {
            observ::emit_outcome(cmd_name, duration_ms, None);
        }
        Err(err) => {
            let code = classify_error(err);
            observ::emit_outcome(cmd_name, duration_ms, Some(&code.to_string()));
            // The Err is returned below; anyhow's termination handler prints it and
            // exits non-zero — no duplicate eprintln! needed.
        }
    }

    result
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // ── `bastion overview` dispatch target (BA.26.G task 3, AC-2) ─────────────

    /// `dispatch`'s `Commands::Overview` arm must call the new open-work
    /// renderer, not the parked Kanban entry point — asserted over the
    /// dispatch itself (function-pointer identity), never by invoking the
    /// binary. Fails at compile time already if either symbol is renamed or
    /// removed; fails at runtime if `overview_dispatch_target` is ever
    /// re-pointed back at the parked path.
    #[test]
    fn overview_dispatches_to_the_new_renderer_not_the_parked_kanban_path() {
        let target = overview_dispatch_target() as *const () as usize;
        assert_eq!(
            target,
            overview::run_sections_ui as *const () as usize,
            "bastion overview must dispatch to overview::run_sections_ui"
        );
        assert_ne!(
            target,
            overview::run as *const () as usize,
            "bastion overview must no longer dispatch to the parked Kanban entry point overview::run"
        );
    }

    // ── command_name resolver — every variant ─────────────────────────────────

    #[test]
    fn command_name_tui() {
        assert_eq!(command_name(&Commands::Tui), "tui");
    }

    #[test]
    fn command_name_monitor() {
        assert_eq!(
            command_name(&Commands::Monitor {
                workflow_id: None,
                watch: false,
            }),
            "monitor"
        );
    }

    #[test]
    fn command_name_inspect() {
        assert_eq!(
            command_name(&Commands::Inspect {
                run_id: "r1".into()
            }),
            "inspect"
        );
    }

    #[test]
    fn command_name_validate() {
        assert_eq!(
            command_name(&Commands::Validate {
                path: PathBuf::from(".")
            }),
            "validate"
        );
    }

    #[test]
    fn command_name_costs() {
        assert_eq!(
            command_name(&Commands::Costs {
                last: "7d".into(),
                watch: false,
            }),
            "costs"
        );
    }

    #[test]
    fn command_name_run() {
        assert_eq!(
            command_name(&Commands::Run {
                workflow: "wf".into(),
                args: None,
                monitor: false,
                force: false,
            }),
            "run"
        );
    }

    #[test]
    fn command_name_abort() {
        assert_eq!(
            command_name(&Commands::Abort {
                run: "run-1".into(),
                yes: false,
            }),
            "abort"
        );
    }

    #[test]
    fn command_name_status() {
        assert_eq!(command_name(&Commands::Status), "status");
    }

    #[test]
    fn command_name_momentum() {
        assert_eq!(command_name(&Commands::Momentum), "momentum");
    }

    #[test]
    fn command_name_sessions() {
        assert_eq!(command_name(&Commands::Sessions), "sessions");
    }

    #[test]
    fn command_name_attach() {
        assert_eq!(
            command_name(&Commands::Attach { lane: "r/l".into() }),
            "attach"
        );
    }

    #[test]
    fn command_name_roadmap_status() {
        assert_eq!(
            command_name(&Commands::RoadmapStatus {
                roadmap: "r".into(),
                json: false,
            }),
            "roadmap-status"
        );
    }

    #[test]
    fn parse_attach_lane_splits_repo_and_lane_on_first_slash() {
        assert_eq!(parse_attach_lane("bastion/b1").unwrap(), ("bastion", "b1"));
    }

    #[test]
    fn parse_attach_lane_refuses_a_value_with_no_slash_naming_it() {
        let err = parse_attach_lane("not-a-repo-slash-lane").unwrap_err();
        assert!(err.to_string().contains("not-a-repo-slash-lane"));
    }

    #[test]
    fn parse_attach_lane_refuses_a_value_with_more_than_one_slash() {
        let err = parse_attach_lane("a/b/c").unwrap_err();
        assert!(err.to_string().contains("a/b/c"));
    }

    #[test]
    fn command_name_new() {
        assert_eq!(
            command_name(&Commands::New {
                session: "s".into(),
                dir: None,
            }),
            "new"
        );
    }

    #[test]
    fn command_name_kill() {
        assert_eq!(
            command_name(&Commands::Kill {
                session: "s".into()
            }),
            "kill"
        );
    }

    #[test]
    fn command_name_send() {
        assert_eq!(
            command_name(&Commands::Send {
                session: "s".into(),
                cmd: vec!["echo".into()],
            }),
            "send"
        );
    }

    #[test]
    fn command_name_capture() {
        assert_eq!(
            command_name(&Commands::Capture {
                session: "s".into(),
                lines: None,
            }),
            "capture"
        );
    }

    #[test]
    fn command_name_ask() {
        assert_eq!(
            command_name(&Commands::Ask {
                session: "s".into(),
                prompt_file: PathBuf::from("/p"),
                out: PathBuf::from("/o"),
                dir: None,
                timeout: 180,
                launch_cmd: "claude".into(),
            }),
            "ask"
        );
    }

    #[test]
    fn command_name_man() {
        assert_eq!(command_name(&Commands::Man { out: None }), "man");
    }

    #[test]
    fn command_name_brain() {
        assert_eq!(
            command_name(&Commands::Brain {
                dependents: Some("doc-id".into()),
                blast_radius: None,
                lineage: None,
                root: None,
                workspace: None,
                json: false,
            }),
            "brain"
        );
    }

    #[test]
    fn command_name_validate_brain() {
        assert_eq!(
            command_name(&Commands::ValidateBrain {
                path: PathBuf::from("."),
                sync: false,
                graph: false,
                state: false,
                links: false,
                structure: false,
                json: false,
            }),
            "validate-brain"
        );
    }

    #[test]
    fn command_name_manifest() {
        assert_eq!(
            command_name(&Commands::Manifest {
                path: PathBuf::from("."),
                pretty: false,
            }),
            "manifest"
        );
    }

    #[test]
    fn command_name_graph() {
        assert_eq!(
            command_name(&Commands::Graph {
                path: PathBuf::from("."),
            }),
            "graph"
        );
    }

    #[test]
    fn command_name_emit_state() {
        assert_eq!(
            command_name(&Commands::EmitState {
                path: PathBuf::from("."),
                write: false,
                fail_on_drift: false,
                agent: None,
                scope: None,
            }),
            "emit-state"
        );
    }

    #[test]
    fn command_name_serve() {
        assert_eq!(
            command_name(&Commands::Serve {
                addr: None,
                token: None,
            }),
            "serve"
        );
    }

    #[test]
    fn command_name_code() {
        assert_eq!(
            command_name(&Commands::Code {
                def: Some("MyFn".into()),
                refs: None,
                dependents: None,
                root: None,
                workspace: None,
                json: false,
                action: None,
            }),
            "code"
        );
    }

    #[test]
    fn command_name_view() {
        assert_eq!(
            command_name(&Commands::View {
                path: PathBuf::from("doc.md"),
            }),
            "view"
        );
    }

    #[test]
    fn command_name_edit() {
        assert_eq!(
            command_name(&Commands::Edit {
                path: PathBuf::from("doc.md"),
            }),
            "edit"
        );
    }

    #[test]
    fn command_name_assess() {
        assert_eq!(
            command_name(&Commands::Assess {
                path: PathBuf::from("."),
                json: false,
            }),
            "assess"
        );
    }

    #[test]
    fn command_name_notify() {
        assert_eq!(
            command_name(&Commands::Notify {
                mode: NotifyMode::Send {
                    text: "hi".to_string(),
                    bot: "lane".to_string(),
                },
            }),
            "notify"
        );
    }

    // ── classify_error — typed ConsoleError downcasts ─────────────────────────

    #[test]
    fn classify_typed_binary_not_found() {
        let err = anyhow::Error::new(ConsoleError::BinaryNotFound);
        assert_eq!(classify_error(&err), ErrorCode::BinaryNotFound);
    }

    #[test]
    fn classify_typed_timeout() {
        let err = anyhow::Error::new(ConsoleError::Timeout(30));
        assert_eq!(classify_error(&err), ErrorCode::Timeout);
    }

    #[test]
    fn classify_typed_config_error() {
        let err = anyhow::Error::new(ConsoleError::ConfigError("bad".into()));
        assert_eq!(classify_error(&err), ErrorCode::ConfigError);
    }

    #[test]
    fn classify_typed_process_error() {
        let err = anyhow::Error::new(ConsoleError::ProcessError("crash".into()));
        assert_eq!(classify_error(&err), ErrorCode::ProcessError);
    }

    #[test]
    fn classify_typed_not_authenticated() {
        let err = anyhow::Error::new(ConsoleError::NotAuthenticated);
        assert_eq!(classify_error(&err), ErrorCode::NotAuthenticated);
    }

    #[test]
    fn classify_typed_rate_limit_exceeded() {
        let err = anyhow::Error::new(ConsoleError::RateLimitExceeded);
        assert_eq!(classify_error(&err), ErrorCode::RateLimitExceeded);
    }

    #[test]
    fn classify_typed_io_error() {
        let err = anyhow::Error::new(ConsoleError::Io("disk".into()));
        assert_eq!(classify_error(&err), ErrorCode::IoError);
    }

    #[test]
    fn classify_typed_stream_closed() {
        let err = anyhow::Error::new(ConsoleError::StreamClosed);
        assert_eq!(classify_error(&err), ErrorCode::StreamClosed);
    }

    #[test]
    fn classify_typed_utf8_error() {
        let err = anyhow::Error::new(ConsoleError::Utf8Error("bad".into()));
        assert_eq!(classify_error(&err), ErrorCode::Utf8Error);
    }

    // ── classify_error — std::io::Error downcasts ─────────────────────────────

    #[test]
    fn classify_std_io_permission_denied() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "permission denied");
        let err = anyhow::Error::new(io_err);
        assert_eq!(classify_error(&err), ErrorCode::PermissionDenied);
    }

    #[test]
    fn classify_std_io_timed_out() {
        let io_err = std::io::Error::new(std::io::ErrorKind::TimedOut, "timed out");
        let err = anyhow::Error::new(io_err);
        assert_eq!(classify_error(&err), ErrorCode::Timeout);
    }

    #[test]
    fn classify_std_io_generic() {
        let io_err =
            std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "connection refused");
        let err = anyhow::Error::new(io_err);
        assert_eq!(classify_error(&err), ErrorCode::IoError);
    }

    // ── classify_error — keyword heuristics ──────────────────────────────────

    #[test]
    fn classify_keyword_permission_denied() {
        let err = anyhow::anyhow!("permission denied: /etc/shadow");
        assert_eq!(classify_error(&err), ErrorCode::PermissionDenied);
    }

    #[test]
    fn classify_keyword_timed_out() {
        let err = anyhow::anyhow!("operation timed out after 30s");
        assert_eq!(classify_error(&err), ErrorCode::Timeout);
    }

    #[test]
    fn classify_keyword_rate_limit() {
        let err = anyhow::anyhow!("rate limit exceeded: retry after 60s");
        assert_eq!(classify_error(&err), ErrorCode::RateLimitExceeded);
    }

    #[test]
    fn classify_keyword_utf8() {
        let err = anyhow::anyhow!("invalid utf-8 sequence in output");
        assert_eq!(classify_error(&err), ErrorCode::Utf8Error);
    }

    #[test]
    fn classify_keyword_not_authenticated() {
        let err = anyhow::anyhow!("not authenticated. run 'claude auth' to authenticate");
        assert_eq!(classify_error(&err), ErrorCode::NotAuthenticated);
    }

    #[test]
    fn classify_keyword_process_tmux() {
        let err = anyhow::anyhow!("tmux: no server running on /tmp/tmux-1000/default");
        assert_eq!(classify_error(&err), ErrorCode::ProcessError);
    }

    #[test]
    fn classify_keyword_stream_closed() {
        let err = anyhow::anyhow!("stream closed unexpectedly");
        assert_eq!(classify_error(&err), ErrorCode::StreamClosed);
    }

    #[test]
    fn classify_keyword_serialization() {
        let err = anyhow::anyhow!("failed to deserialize json response");
        assert_eq!(classify_error(&err), ErrorCode::SerializationError);
    }

    #[test]
    fn classify_keyword_io_error() {
        let err = anyhow::anyhow!("io error: broken pipe");
        assert_eq!(classify_error(&err), ErrorCode::IoError);
    }

    #[test]
    fn classify_keyword_binary_not_found() {
        let err = anyhow::anyhow!("binary not found: claude is not in PATH");
        assert_eq!(classify_error(&err), ErrorCode::BinaryNotFound);
    }

    #[test]
    fn classify_keyword_binary_not_found_in_path() {
        let err = anyhow::anyhow!("not found in path: /usr/local/bin");
        assert_eq!(classify_error(&err), ErrorCode::BinaryNotFound);
    }

    #[test]
    fn classify_keyword_mcp_server() {
        let err = anyhow::anyhow!("mcp server error: connection refused");
        assert_eq!(classify_error(&err), ErrorCode::McpError);
    }

    #[test]
    fn classify_keyword_config_error() {
        let err = anyhow::anyhow!("invalid configuration: missing required field");
        assert_eq!(classify_error(&err), ErrorCode::ConfigError);
    }

    #[test]
    fn classify_unclassifiable_defaults_to_invalid_input() {
        let err = anyhow::anyhow!("something completely unexpected happened");
        assert_eq!(classify_error(&err), ErrorCode::InvalidInput);
    }
}
