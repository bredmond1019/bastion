// `bastion run <workflow>` — trigger a workflow via FastAPI.
// `bastion status`         — quick stack health check (non-TUI).

use anyhow::Result;

use crate::api::client::{ApiClient, ApiStatus};
use crate::config::Config;
use crate::db::health::{self, DbStatus};

pub async fn status() -> Result<()> {
    let config = Config::load()?;
    let db = health::probe(&config.database_url).await;
    let api = ApiClient::new(&config.api_base_url).health().await;
    println!("{}", render_status(&db, &api));
    Ok(())
}

#[allow(dead_code)]
pub async fn trigger(_workflow_id: &str) -> Result<()> {
    todo!("Phase 3: trigger workflow via FastAPI")
}

/// Pure renderer — one row per service. Kept side-effect-free so it is unit-testable
/// without a live DB/API. No emoji (words only) per the project's source/docs rule.
fn render_status(db: &DbStatus, api: &ApiStatus) -> String {
    let db_line = match db {
        DbStatus::Reachable => "DB   reachable".to_string(),
        DbStatus::Unreachable(_) => "DB   unreachable".to_string(),
    };
    let api_line = match api {
        ApiStatus::Reachable { version, .. } => {
            format!("API  reachable (version {version})")
        }
        ApiStatus::Unreachable(_) => "API  unreachable".to_string(),
    };
    format!("{db_line}\n{api_line}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_reachable_services_with_version() {
        let out = render_status(
            &DbStatus::Reachable,
            &ApiStatus::Reachable {
                status: "ok".into(),
                version: "1.2.3".into(),
            },
        );
        assert!(out.contains("DB   reachable"), "got: {out}");
        assert!(out.contains("API  reachable (version 1.2.3)"), "got: {out}");
    }

    #[test]
    fn renders_unreachable_services_without_panicking() {
        let out = render_status(
            &DbStatus::Unreachable("connection refused".into()),
            &ApiStatus::Unreachable("connection refused".into()),
        );
        assert!(out.contains("DB   unreachable"), "got: {out}");
        assert!(out.contains("API  unreachable"), "got: {out}");
    }
}
