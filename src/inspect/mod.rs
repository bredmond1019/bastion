// `bastion inspect <run-id>` — static post-mortem graph view.
// Reuses monitor graph/ui code with polling disabled.

use anyhow::Result;

pub async fn run(_run_id: String) -> Result<()> {
    todo!("Phase 2: load completed run from DB, render static graph, allow navigation")
}
