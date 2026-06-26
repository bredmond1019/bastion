// Dead code is expected during incremental scaffold build-out.
// Remove this attribute once all phases are wired up.
#![allow(dead_code)]

mod api;
mod brain;
mod cli;
mod config;
mod costs;
mod db;
mod inspect;
mod man;
mod monitor;
mod run;
mod sessions;
mod validate;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Commands};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        // No subcommand or explicit `tui` → interactive session dashboard.
        // Synchronous call, consistent with the other session verbs (D5).
        None | Some(Commands::Tui) => sessions::ui::run(),

        Some(cmd) => match cmd {
            // Tui handled above — included to keep the match exhaustive.
            Commands::Tui => unreachable!(),
            Commands::Monitor { workflow_id } => monitor::run(workflow_id).await,
            Commands::Inspect { run_id } => inspect::run(run_id).await,
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
                sessions::ask::ask(args).map_err(|e| {
                    eprintln!("bastion ask: {e}");
                    anyhow::anyhow!("{e}")
                })
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
