// `bastion run <workflow>` — trigger a workflow via FastAPI.
// `bastion status`         — quick stack health check (non-TUI).

use anyhow::Result;

pub async fn trigger(_workflow: String, _args: Option<String>, _monitor: bool) -> Result<()> {
    todo!("Phase 3: POST to FastAPI /workflows/{{name}}/run, optionally enter monitor")
}

pub async fn status() -> Result<()> {
    todo!("Phase 0: check DB reachable, API reachable, Celery workers up, Redis connected")
}
