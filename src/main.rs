// Dead code is expected during incremental scaffold build-out.
// Remove this attribute once all phases are wired up.
#![allow(dead_code)]

mod api;
mod brain;
mod cli;
mod config;
mod costs;
mod db;
mod detect;
mod inspect;
mod man;
mod monitor;
mod observ;
mod overview;
mod run;
mod serve;
mod sessions;
mod ui_theme;
mod validate;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Commands};
use observ::errors::{ConsoleError, ErrorCode};

// ── Pure helpers (unit-tested below) ─────────────────────────────────────────

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
        Commands::Status => "status",
        Commands::Sessions => "sessions",
        Commands::Attach { .. } => "attach",
        Commands::New { .. } => "new",
        Commands::Kill { .. } => "kill",
        Commands::Send { .. } => "send",
        Commands::Capture { .. } => "capture",
        Commands::Ask { .. } => "ask",
        Commands::Man { .. } => "man",
        Commands::Brain { .. } => "brain",
        Commands::Code { .. } => "code",
        Commands::Serve { .. } => "serve",
    }
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
            Commands::Monitor { workflow_id } => monitor::run(workflow_id).await,
            Commands::Inspect { run_id } => inspect::run(run_id).await,
            Commands::Overview => overview::run(),
            Commands::Validate { path } => validate::run(path).await,
            Commands::Costs { last } => costs::run(last).await,
            Commands::Run {
                workflow,
                args,
                monitor,
            } => run::trigger(workflow, args, monitor).await,
            Commands::Status => run::status().await,
            // Sessions path is DB-free (D4): no Config::load(), no Postgres pool.
            // All session verbs are sync blocking (D5): no async/tokio coupling.
            Commands::Sessions => sessions::run(),
            Commands::Attach { session } => sessions::commands::attach(&session),
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
                brain::run(query, root, workspace, &registry)
            }
            // Serve is DB-free — does NOT call Config::load() or require DATABASE_URL.
            // The actix System runs on a dedicated OS thread (runtime-spike outcome, Task 1).
            Commands::Serve { addr, token } => {
                let serve_cfg =
                    config::load_serve_config(addr, token).map_err(|e| anyhow::anyhow!("{e}"))?;
                tokio::task::spawn_blocking(move || serve::run(serve_cfg.addr, serve_cfg.token))
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
            } => {
                let query = if let Some(name) = def {
                    brain::code_graph::CodeQuery::Def(name)
                } else if let Some(name) = refs {
                    brain::code_graph::CodeQuery::Refs(name)
                } else if let Some(name) = dependents {
                    brain::code_graph::CodeQuery::Dependents(name)
                } else {
                    // Unreachable: clap ArgGroup enforces exactly one of the three flags.
                    unreachable!("clap ArgGroup guarantees exactly one code query flag is set")
                };
                let registry = config::load_workspace_registry(
                    std::env::var("XDG_CONFIG_HOME").ok(),
                    std::env::var("HOME").ok(),
                )?;
                brain::code_graph::run_code(query, root, workspace, &registry)
            }
        },
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

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

    // ── command_name resolver — every variant ─────────────────────────────────

    #[test]
    fn command_name_tui() {
        assert_eq!(command_name(&Commands::Tui), "tui");
    }

    #[test]
    fn command_name_monitor() {
        assert_eq!(
            command_name(&Commands::Monitor { workflow_id: None }),
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
            command_name(&Commands::Costs { last: "7d".into() }),
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
            }),
            "run"
        );
    }

    #[test]
    fn command_name_status() {
        assert_eq!(command_name(&Commands::Status), "status");
    }

    #[test]
    fn command_name_sessions() {
        assert_eq!(command_name(&Commands::Sessions), "sessions");
    }

    #[test]
    fn command_name_attach() {
        assert_eq!(
            command_name(&Commands::Attach {
                session: "s".into()
            }),
            "attach"
        );
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
            }),
            "brain"
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
            }),
            "code"
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
