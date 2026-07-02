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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_status_reachable_equality() {
        assert_eq!(DbStatus::Reachable, DbStatus::Reachable);
    }

    #[test]
    fn db_status_unreachable_equality() {
        let a = DbStatus::Unreachable("connection refused".to_string());
        let b = DbStatus::Unreachable("connection refused".to_string());
        assert_eq!(a, b);
    }

    #[test]
    fn db_status_reachable_ne_unreachable() {
        assert_ne!(
            DbStatus::Reachable,
            DbStatus::Unreachable("error".to_string())
        );
    }

    #[test]
    fn db_status_unreachable_stores_message() {
        let msg = "could not connect to server";
        let status = DbStatus::Unreachable(msg.to_string());
        match status {
            DbStatus::Unreachable(s) => assert_eq!(s, msg),
            DbStatus::Reachable => panic!("expected Unreachable"),
        }
    }

    #[test]
    fn db_status_debug_contains_variant_name() {
        let s = format!("{:?}", DbStatus::Reachable);
        assert!(s.contains("Reachable"));

        let u = format!("{:?}", DbStatus::Unreachable("timeout".to_string()));
        assert!(u.contains("Unreachable"));
        assert!(u.contains("timeout"));
    }
}
