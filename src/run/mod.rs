// `bastion run <workflow>` — trigger a workflow via FastAPI.
// `bastion status`         — quick stack health check (non-TUI).

use anyhow::{Context, Result, anyhow};

use crate::api::client::{ApiClient, ApiStatus};
use crate::config::Config;
use crate::db::health::{self, DbStatus};
use crate::monitor;

/// Parse an optional JSON string argument into a `serde_json::Value`.
///
/// - `None` → `Ok(None)` (caller sends `data: {}` via `trigger_body`)
/// - Valid JSON object string → `Ok(Some(value))`
/// - Malformed JSON → `Err` with a clear message
/// - Valid JSON but not an object (e.g. `"5"`, `"[1,2]"`) → `Err` asking for an object
///
/// Pure function — no I/O — so it is unit-testable without network or filesystem.
pub fn parse_args(args: Option<String>) -> Result<Option<serde_json::Value>> {
    match args {
        None => Ok(None),
        Some(s) => {
            let value: serde_json::Value = serde_json::from_str(&s)
                .with_context(|| format!("--args is not valid JSON: {s}"))?;
            if !value.is_object() {
                return Err(anyhow!(
                    "--args must be a JSON object (got {}), e.g. '{{\"key\": \"value\"}}'",
                    value_type_name(&value)
                ));
            }
            Ok(Some(value))
        }
    }
}

/// Return a human-readable type label for a JSON value (used in error messages).
/// Pure function — no I/O.
fn value_type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Format the success output for a triggered workflow.
/// Pure function — no I/O — so it is unit-testable.
pub fn format_trigger_success(workflow: &str, task_id: &str) -> String {
    format!("workflow: {workflow}\ntask_id: {task_id}\n")
}

pub async fn trigger(workflow: String, args: Option<String>, monitor: bool) -> Result<()> {
    let data = parse_args(args)?;
    let config = Config::load()?;
    let client = ApiClient::new(&config.api_base_url);
    let task_id = client
        .trigger_workflow(&workflow, data)
        .await
        .with_context(|| {
            format!("failed to trigger workflow '{workflow}' — is the orchestrator running?")
        })?;
    print!("{}", format_trigger_success(&workflow, &task_id));
    if monitor {
        monitor::run(Some(task_id)).await?;
    }
    Ok(())
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

    // ── parse_args ────────────────────────────────────────────────────────────

    #[test]
    fn parse_args_none_returns_none() {
        assert!(parse_args(None).unwrap().is_none());
    }

    #[test]
    fn parse_args_valid_object_returns_some() {
        let result = parse_args(Some(r#"{"k": 1, "flag": true}"#.to_string())).unwrap();
        let val = result.unwrap();
        assert!(val.is_object());
        assert_eq!(val["k"], 1);
        assert_eq!(val["flag"], true);
    }

    #[test]
    fn parse_args_malformed_json_returns_err() {
        let err = parse_args(Some("{".to_string())).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("--args is not valid JSON"), "got: {msg}");
    }

    #[test]
    fn parse_args_number_returns_err_with_type_hint() {
        let err = parse_args(Some("5".to_string())).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("--args must be a JSON object"), "got: {msg}");
        assert!(msg.contains("number"), "got: {msg}");
    }

    #[test]
    fn parse_args_array_returns_err_with_type_hint() {
        let err = parse_args(Some("[1,2,3]".to_string())).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("--args must be a JSON object"), "got: {msg}");
        assert!(msg.contains("array"), "got: {msg}");
    }

    #[test]
    fn parse_args_string_value_returns_err() {
        let err = parse_args(Some(r#""hello""#.to_string())).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("--args must be a JSON object"), "got: {msg}");
        assert!(msg.contains("string"), "got: {msg}");
    }

    // ── format_trigger_success ────────────────────────────────────────────────

    #[test]
    fn format_trigger_success_contains_workflow_and_task_id() {
        let out = format_trigger_success("research_workflow", "abc-123");
        assert!(out.contains("workflow: research_workflow"), "got: {out}");
        assert!(out.contains("task_id: abc-123"), "got: {out}");
    }

    #[test]
    fn format_trigger_success_task_id_on_own_line() {
        let out = format_trigger_success("wf", "tid-999");
        // Each key should be on its own line
        let lines: Vec<&str> = out.lines().collect();
        assert!(
            lines.iter().any(|l| *l == "workflow: wf"),
            "lines: {lines:?}"
        );
        assert!(
            lines.iter().any(|l| *l == "task_id: tid-999"),
            "lines: {lines:?}"
        );
    }

    // ── value_type_name ───────────────────────────────────────────────────────

    #[test]
    fn value_type_name_variants() {
        use serde_json::Value;
        assert_eq!(value_type_name(&Value::Null), "null");
        assert_eq!(value_type_name(&Value::Bool(true)), "boolean");
        assert_eq!(value_type_name(&Value::Number(42.into())), "number");
        assert_eq!(value_type_name(&Value::String("s".into())), "string");
        assert_eq!(value_type_name(&Value::Array(vec![])), "array");
        assert_eq!(
            value_type_name(&Value::Object(serde_json::Map::new())),
            "object"
        );
    }

    // ── render_status ─────────────────────────────────────────────────────────

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
