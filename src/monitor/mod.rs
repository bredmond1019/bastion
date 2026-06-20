pub mod app;
pub mod events;
pub mod graph;
pub mod ui;

use anyhow::Result;

pub async fn run(_workflow_id: Option<String>) -> Result<()> {
    todo!("Phase 1: enter ratatui TUI loop for live workflow monitoring")
}
