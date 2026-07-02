pub mod app;
pub mod events;
pub mod graph;
pub mod ui;

use anyhow::Result;

use crate::api::client::ApiClient;
use crate::config::Config;
use crate::db::workflows;
use crate::monitor::{app::App, graph::build_layout};

/// Entry point for `bastion monitor [--workflow-id <id>]`.
///
/// Loads config, fetches active workflow runs from the orchestrator's
/// PostgreSQL, fetches the static DAG from the API, builds an initial
/// `GraphLayout`, then hands off to the TUI event loop.
///
/// Degrades with a clear message (never panics) when:
/// - config is missing (DATABASE_URL not set)
/// - no active runs are found
/// - the DB or API is unreachable
pub async fn run(workflow_id: Option<String>) -> Result<()> {
    // ── Config ────────────────────────────────────────────────────────────────
    let config = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("bastion monitor: configuration error — {e}");
            eprintln!(
                "  Set DATABASE_URL (and optionally BASTION_API_URL) in your environment or .env file."
            );
            return Ok(());
        }
    };

    let api_client = ApiClient::new(&config.api_base_url);

    // ── Fetch initial runs ────────────────────────────────────────────────────
    let runs = match &workflow_id {
        Some(id) => match workflows::get_run_state(&config.database_url, id).await {
            Ok(run) => vec![run],
            Err(e) => {
                eprintln!("bastion monitor: failed to fetch run '{id}' — {e}");
                return Ok(());
            }
        },
        None => match workflows::list_active_runs(&config.database_url).await {
            Ok(runs) => runs,
            Err(e) => {
                eprintln!("bastion monitor: failed to query active runs — {e:#}");
                eprintln!("  Is the Python orchestrator stack running? (./scripts/dev.sh)");
                return Ok(());
            }
        },
    };

    // Re-fetch sessions
    let sessions = match crate::sessions::tmux::list_sessions_raw() {
        Ok(raw) => {
            let mut s = crate::sessions::model::parse_sessions(&raw);
            for session in s.iter_mut() {
                if let Ok(output) = crate::sessions::tmux::capture_pane_raw(&session.name) {
                    let pane = crate::sessions::model::Pane::new(&session.name, output);
                    session.last_line = pane.last_line().to_string();
                }
            }
            s
        }
        Err(_) => vec![],
    };

    let items = app::build_mission_items(&sessions, &runs);
    if items.is_empty() {
        eprintln!("bastion monitor: no active workflow runs or sessions found.");
        eprintln!("  Trigger a workflow or session first, then re-run `bastion monitor`.");
        return Ok(());
    }

    // ── Build initial App state ───────────────────────────────────────────────
    let mut app = App::new();
    app.replace_items(items);

    // ── Build initial layout for the first selected run (if any) ──────────────
    if let Some(app::MissionItem::Run(run)) = app.selected_item() {
        match api_client.workflow_graph(&run.workflow_name).await {
            Ok(graph) => {
                let layout = build_layout(&graph, &run.nodes);
                app.layout = Some(layout);
            }
            Err(e) => {
                eprintln!("bastion monitor: could not fetch workflow graph — {e}");
                eprintln!("  Entering TUI without initial layout; will retry on first poll tick.");
            }
        }
    }

    // ── Enter TUI event loop ──────────────────────────────────────────────────
    events::run_event_loop(
        &mut app,
        &config.database_url,
        &api_client,
        config.poll_interval_secs,
        workflow_id.as_deref(),
    )
    .await
}
