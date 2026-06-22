// `bastion run <workflow>` — trigger a workflow via FastAPI.
// `bastion status`         — quick stack health check (non-TUI).

use anyhow::Result;

use crate::api::client::{ApiClient, ApiStatus};
use crate::config::Config;
use crate::db::health::{self, DbStatus};

pub async fn trigger(_workflow: String, _args: Option<String>, _monitor: bool) -> Result<()> {
    todo!("Phase 3: POST / with {{workflow_type, data}} → 202 {{task_id}}, optionally enter monitor")
}

pub async fn status() -> Result<()> {
    let config = Config::load()?;

    let db = health::probe(&config.database_url).await;
    let api = ApiClient::new(&config.api_base_url).health().await;

    print!("{}", render_status(&db, &api));
    Ok(())
}

/// Render a plain-text summary table for the given probe outcomes.
/// Pure function (no I/O) so it can be unit-tested without live services.
fn render_status(db: &DbStatus, api: &ApiStatus) -> String {
    let db_row = match db {
        DbStatus::Reachable => "DB    reachable".to_string(),
        DbStatus::Unreachable(msg) => format!("DB    unreachable ({msg})"),
    };

    let api_row = match api {
        ApiStatus::Reachable { status, version } => {
            format!("API   reachable (status={status}, version={version})")
        }
        ApiStatus::Unreachable(msg) => format!("API   unreachable ({msg})"),
    };

    format!("Stack health\n{db_row}\n{api_row}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_both_unreachable() {
        let out = render_status(
            &DbStatus::Unreachable("connection refused".to_string()),
            &ApiStatus::Unreachable("timeout".to_string()),
        );
        assert!(out.contains("DB    unreachable (connection refused)"));
        assert!(out.contains("API   unreachable (timeout)"));
    }

    #[test]
    fn render_both_reachable() {
        let out = render_status(
            &DbStatus::Reachable,
            &ApiStatus::Reachable {
                status: "ok".to_string(),
                version: "0.1.0".to_string(),
            },
        );
        assert!(out.contains("DB    reachable"));
        assert!(out.contains("API   reachable (status=ok, version=0.1.0)"));
    }

    #[test]
    fn render_mixed_db_up_api_down() {
        let out = render_status(
            &DbStatus::Reachable,
            &ApiStatus::Unreachable("HTTP 503".to_string()),
        );
        assert!(out.contains("DB    reachable"));
        assert!(out.contains("API   unreachable (HTTP 503)"));
    }
}
