// Read-only PostgreSQL reachability probe for `bastion status`.
// Observer only (D2): opens a pool and runs `SELECT 1`. No writes.

use sqlx::postgres::PgPoolOptions;
use std::time::Duration;

/// Outcome of probing PostgreSQL. Unreachable is a normal outcome, not an `Err`.
#[derive(Debug, Clone, PartialEq)]
pub enum DbStatus {
    Reachable,
    Unreachable(String),
}

pub async fn probe(db_url: &str) -> DbStatus {
    let pool = match PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(2))
        .connect(db_url)
        .await
    {
        Ok(p) => p,
        Err(e) => return DbStatus::Unreachable(e.to_string()),
    };

    match sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&pool)
        .await
    {
        Ok(_) => DbStatus::Reachable,
        Err(e) => DbStatus::Unreachable(e.to_string()),
    }
}
