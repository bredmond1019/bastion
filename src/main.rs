mod api;
mod cli;
mod config;
mod costs;
mod db;
mod inspect;
mod monitor;
mod run;
mod validate;

use clap::Parser;
use cli::{Cli, Commands};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Status => run::status().await,
        Commands::Monitor => {
            todo!("Phase 1: live TUI monitor")
        }
        Commands::Inspect { workflow_id: _ } => {
            todo!("Phase 2: static inspect view")
        }
        Commands::Costs => {
            todo!("Phase 2: LLM cost summary")
        }
        Commands::Run { workflow_id: _ } => {
            todo!("Phase 3: trigger workflow")
        }
        Commands::Validate { path: _ } => {
            todo!("Phase 4: validate markdown/MDX")
        }
    }
}
