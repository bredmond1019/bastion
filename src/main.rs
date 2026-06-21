// Dead code is expected during incremental scaffold build-out.
// Remove this attribute once all phases are wired up.
#![allow(dead_code)]

mod api;
mod cli;
mod config;
mod costs;
mod db;
mod inspect;
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
    }
}
