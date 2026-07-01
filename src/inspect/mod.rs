// `bastion inspect <run-id>` — static post-mortem graph view.
//
// Reuses monitor graph/ui code with polling disabled. The flow is:
//   1. Load config + DB connection
//   2. Fetch the single run by ID (any status — post-mortem or still-active snapshot)
//   3. Fetch the static DAG from the API (non-fatal if unreachable)
//   4. Build an App via `build_inspect_app` (pure, unit-tested)
//   5. Enter the static render loop (draw + wait for key; no poll interval)
//
// The thin I/O shell (`run` + `run_static_loop`) is smoke-tested manually per Rule 6.

use anyhow::Result;
use crossterm::event::Event;

use crate::api::client::{ApiClient, WorkflowGraph};
use crate::config::Config;
use crate::db::workflows::{WorkflowRun, get_run_state};
use crate::monitor::{
    app::{App, MissionItem},
    events::{handle_key, restore_terminal, setup_terminal},
    graph::build_layout,
    ui,
};

// ── Pure seam ─────────────────────────────────────────────────────────────────

/// Build an `App` loaded with a single completed (or in-progress snapshot) run.
///
/// - If `graph` is `Some`, computes the `GraphLayout` via `build_layout` and
///   stores it in `app.layout`.
/// - If `graph` is `None` (API unreachable), `app.layout` stays `None`; the TUI
///   renders nodes without edges.
/// - `app.items` is set to `vec![run]` via `replace_runs`.
///
/// Pure: no I/O, no tokio. Unit-tested exhaustively.
pub fn build_inspect_app(run: WorkflowRun, graph: Option<&WorkflowGraph>) -> App {
    let mut app = App::new();
    app.layout = graph.map(|g| build_layout(g, &run.nodes));
    app.replace_items(vec![MissionItem::Run(run)]);
    app
}

// ── Static render loop ────────────────────────────────────────────────────────

/// Static event loop: draw once per iteration, block on keyboard, no DB re-fetch.
///
/// Restores the terminal on exit (best-effort), even on error.
pub fn run_static_loop(app: &mut App) -> Result<()> {
    let mut terminal = setup_terminal()?;

    let result = (|| {
        loop {
            terminal.draw(|f| ui::render(f, app, f.area()))?;

            // Block until a key event arrives (no timeout needed — static view).
            if let Event::Key(key) = crossterm::event::read()? {
                handle_key(app, key);
            }

            if app.should_quit {
                break;
            }
        }
        Ok::<(), anyhow::Error>(())
    })();

    // Always restore the terminal — best-effort, don't shadow the real error.
    restore_terminal(&mut terminal).ok();
    result
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Entry point for `bastion inspect <run-id>`.
///
/// Degrades with a clear message (never panics) when:
/// - config is missing (DATABASE_URL not set)
/// - the run ID is unknown or the DB is unreachable
/// - the graph API is unreachable (renders nodes without edges)
pub async fn run(run_id: String) -> Result<()> {
    // ── Config ────────────────────────────────────────────────────────────────
    let config = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("bastion inspect: configuration error — {e}");
            eprintln!(
                "  Set DATABASE_URL (and optionally BASTION_API_URL) in your environment or .env file."
            );
            return Ok(());
        }
    };

    let api_client = ApiClient::new(&config.api_base_url);

    // ── Fetch run ─────────────────────────────────────────────────────────────
    let run = match get_run_state(&config.database_url, &run_id).await {
        Ok(r) => r,
        Err(_) => {
            eprintln!("bastion inspect: no run found for '{run_id}'");
            eprintln!("  Is the Python orchestrator stack running? (./scripts/dev.sh)");
            return Ok(());
        }
    };

    // ── Fetch static DAG (non-fatal) ──────────────────────────────────────────
    let graph = match api_client.workflow_graph(&run.workflow_name).await {
        Ok(g) => Some(g),
        Err(e) => {
            eprintln!("bastion inspect: could not fetch workflow graph — {e}");
            eprintln!("  Rendering nodes without edges.");
            None
        }
    };

    // ── Build App + enter static loop ─────────────────────────────────────────
    let mut app = build_inspect_app(run, graph.as_ref());
    run_static_loop(&mut app)
}

// ── Tests ─────────────────────────────────────────────────────────────────────
//
// `build_inspect_app` is pure and exhaustively unit-tested here.
// The I/O shell (`run` + `run_static_loop`) is smoke-tested manually — see ## Notes
// in the task spec.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::client::WorkflowGraph;
    use crate::db::workflows::{NodeState, RunStatus, WorkflowRun};

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn make_node(name: &str) -> NodeState {
        NodeState {
            id: name.to_string(),
            name: name.to_string(),
            status: RunStatus::Success,
            depends_on: vec![],
            input: None,
            output: None,
            error: None,
            tokens_in: None,
            tokens_out: None,
            model: None,
            started_at: None,
            elapsed_secs: None,
        }
    }

    fn make_run(id: &str, node_names: &[&str]) -> WorkflowRun {
        WorkflowRun {
            id: id.to_string(),
            workflow_name: "test_wf".to_string(),
            status: RunStatus::Success,
            nodes: node_names.iter().map(|n| make_node(n)).collect(),
            started_at: None,
            elapsed_secs: None,
        }
    }

    fn make_graph(nodes: &[&str], edges: &[(&str, &str)]) -> WorkflowGraph {
        WorkflowGraph {
            nodes: nodes.iter().map(|s| s.to_string()).collect(),
            edges: edges
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
        }
    }

    // ── build_inspect_app: single run installed ───────────────────────────────

    #[test]
    fn single_run_is_installed() {
        let run = make_run("r1", &["A", "B"]);
        let app = build_inspect_app(run, None);
        assert_eq!(app.items.len(), 1);
        assert_eq!(app.selected_run().unwrap().id, "r1");
    }

    // ── build_inspect_app: node count preserved ───────────────────────────────

    #[test]
    fn node_count_preserved() {
        let run = make_run("r1", &["A", "B", "C"]);
        let app = build_inspect_app(run, None);
        assert_eq!(app.selected_run().unwrap().nodes.len(), 3);
    }

    // ── build_inspect_app: layout present when graph supplied ─────────────────

    #[test]
    fn layout_present_when_graph_supplied() {
        let run = make_run("r1", &["A", "B"]);
        let graph = make_graph(&["A", "B"], &[("A", "B")]);
        let app = build_inspect_app(run, Some(&graph));
        assert!(
            app.layout.is_some(),
            "layout must be Some when a graph is supplied"
        );
    }

    // ── build_inspect_app: layout absent when no graph ────────────────────────

    #[test]
    fn layout_absent_when_no_graph() {
        let run = make_run("r1", &["A"]);
        let app = build_inspect_app(run, None);
        assert!(
            app.layout.is_none(),
            "layout must be None when no graph is supplied"
        );
    }

    // ── build_inspect_app: layout nodes match run nodes ───────────────────────

    #[test]
    fn layout_node_count_matches_graph_nodes() {
        let run = make_run("r1", &["A", "B", "C"]);
        let graph = make_graph(&["A", "B", "C"], &[("A", "B"), ("B", "C")]);
        let app = build_inspect_app(run, Some(&graph));
        let layout = app.layout.unwrap();
        // petgraph DiGraph has node count equal to the number of distinct nodes.
        assert_eq!(layout.graph.node_count(), 3);
    }

    // ── build_inspect_app: cursors start at zero ──────────────────────────────

    #[test]
    fn cursors_start_at_zero() {
        let run = make_run("r1", &["A", "B"]);
        let app = build_inspect_app(run, None);
        assert_eq!(app.selected, 0);
        assert_eq!(app.selected_node, 0);
    }

    // ── build_inspect_app: should_quit starts false ───────────────────────────

    #[test]
    fn should_quit_starts_false() {
        let run = make_run("r1", &["A"]);
        let app = build_inspect_app(run, None);
        assert!(!app.should_quit);
    }

    // ── build_inspect_app: empty run (no nodes) ───────────────────────────────

    #[test]
    fn empty_run_no_nodes() {
        let run = make_run("r1", &[]);
        let app = build_inspect_app(run, None);
        assert_eq!(app.selected_run().unwrap().nodes.len(), 0);
        assert!(app.layout.is_none());
    }

    // ── build_inspect_app: graph with no edges still builds layout ────────────

    #[test]
    fn graph_with_no_edges_builds_layout() {
        let run = make_run("r1", &["A", "B"]);
        let graph = make_graph(&["A", "B"], &[]);
        let app = build_inspect_app(run, Some(&graph));
        let layout = app.layout.unwrap();
        assert_eq!(layout.graph.node_count(), 2);
        assert_eq!(layout.graph.edge_count(), 0);
    }
}
