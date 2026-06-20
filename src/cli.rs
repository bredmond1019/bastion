use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "bastion",
    about = "Personal ops CLI for the agentic engineering stack"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Quick stack health check (non-TUI)
    Status,
    /// Live TUI graph inspector (Phase 1)
    Monitor,
    /// Static post-mortem graph view (Phase 2)
    Inspect {
        /// Workflow ID to inspect
        workflow_id: String,
    },
    /// LLM spend summary (Phase 2)
    Costs,
    /// Trigger a workflow run (Phase 3)
    Run {
        /// Workflow ID to trigger
        workflow_id: String,
    },
    /// Validate markdown/MDX content (Phase 4)
    Validate {
        /// Path to validate
        path: String,
    },
}
