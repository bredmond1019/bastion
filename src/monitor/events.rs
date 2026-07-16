// tokio event loop: keyboard navigation + DB poll every N seconds + redraw on change.
//
// This is the thin I/O shell. It owns:
//   - crossterm raw mode + alternate screen lifecycle
//   - ratatui Terminal setup/teardown
//   - tokio::select! over keyboard events (background thread + channel) and
//     a DB-poll interval
//
// All navigation logic lives in App (pure); all render logic lives in ui (pure).
// Those are the testable surfaces. This file is smoke-tested manually per Rule 6.

use std::time::Duration;

use anyhow::Result;
use crossterm::{
    event::{Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::time;

use crate::api::client::ApiClient;
use crate::db::workflows;
use crate::monitor::{app::App, graph, ui};

/// Enter the live-TUI event loop.
///
/// Sets up crossterm raw mode and the alternate screen, then loops with
/// `tokio::select!` over:
/// - Keyboard events from a background thread via a channel
/// - A `tokio::time::interval(poll_secs)` tick that re-fetches DB state
///
/// On exit (q / Esc / Ctrl-C, or the inner loop returning an error) the
/// terminal is always restored before returning. The `workflow_id` filter
/// is forwarded to the DB query on each tick.
pub async fn run_event_loop(
    app: &mut App,
    db_url: &str,
    api_client: &ApiClient,
    poll_secs: u64,
    workflow_id: Option<&str>,
) -> Result<()> {
    // ── Terminal setup ────────────────────────────────────────────────────────
    let mut terminal = setup_terminal()?;

    // ── Keyboard event channel ────────────────────────────────────────────────
    // A background OS thread polls crossterm events and forwards them over a
    // tokio mpsc channel so that tokio::select! can interleave them with the
    // DB-poll tick without blocking the async runtime.
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<Event>(32);
    std::thread::spawn(move || {
        loop {
            match crossterm::event::poll(Duration::from_millis(100)) {
                Ok(true) => {
                    if let Ok(ev) = crossterm::event::read()
                        && event_tx.blocking_send(ev).is_err()
                    {
                        break; // receiver dropped → loop exiting, stop thread
                    }
                }
                Ok(false) => {} // timeout, keep polling
                Err(_) => break,
            }
        }
    });

    // ── Poll interval ─────────────────────────────────────────────────────────
    let mut interval = time::interval(Duration::from_secs(poll_secs.max(1)));

    // ── Event loop ────────────────────────────────────────────────────────────
    let result = async {
        loop {
            terminal.draw(|f| ui::render(f, app, f.area()))?;

            tokio::select! {
                biased; // prioritise keyboard events over timer ticks

                event = event_rx.recv() => {
                    match event {
                        Some(Event::Key(key)) => handle_key(app, key),
                        None => break, // channel closed
                        _ => {}
                    }
                }

                _ = interval.tick() => {
                    poll_and_update(app, db_url, api_client, workflow_id).await;
                }
            }

            if app.should_quit {
                break;
            }
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;

    // ── Always restore the terminal ───────────────────────────────────────────
    restore_terminal(&mut terminal).ok(); // best-effort; don't shadow the real error
    result
}

// ── Terminal lifecycle helpers ─────────────────────────────────────────────────

pub(crate) fn setup_terminal() -> Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).map_err(Into::into)
}

pub(crate) fn restore_terminal(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

// ── Key handling ───────────────────────────────────────────────────────────────

/// Handle a single `KeyEvent`, mutating `App` navigation state.
/// Pure from the perspective of I/O: no terminal or DB access.
pub(crate) fn handle_key(app: &mut App, key: KeyEvent) {
    match key.code {
        // Quit
        KeyCode::Char('q') | KeyCode::Esc => app.quit(),
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => app.quit(),

        // Node navigation (arrows + vim keys)
        KeyCode::Down | KeyCode::Char('j') => app.next_node(),
        KeyCode::Up | KeyCode::Char('k') => app.prev_node(),

        // Run navigation
        KeyCode::Right | KeyCode::Char('n') => app.next_item(),
        KeyCode::Left | KeyCode::Char('p') => app.prev_item(),

        _ => {}
    }
}

// ── DB poll ────────────────────────────────────────────────────────────────────

/// Re-fetch run state from the DB and rebuild the graph layout.
/// Errors are surfaced as a banner on the App rather than propagated, so
/// a transient DB hiccup does not kill the TUI.
async fn poll_and_update(
    app: &mut App,
    db_url: &str,
    api_client: &ApiClient,
    workflow_id: Option<&str>,
) {
    // Re-fetch runs
    let new_runs = match workflow_id {
        Some(id) => match workflows::get_run_state(db_url, id).await {
            Ok(run) => vec![run],
            Err(e) => {
                app.banner = Some(format!("DB: {e}"));
                return;
            }
        },
        None => match workflows::list_active_runs(db_url).await {
            Ok(runs) => runs,
            Err(e) => {
                app.banner = Some(format!("DB: {e}"));
                return;
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

    let items = crate::monitor::app::build_mission_items(&sessions, &new_runs);

    if items.is_empty() {
        app.banner = Some("No active workflow runs or sessions found".to_string());
        app.replace_items(items);
        return;
    }

    app.replace_items(items);

    // Rebuild the layout for the currently (or about-to-be) selected run.
    if let Some(crate::monitor::app::MissionItem::Run(run)) = app.selected_item() {
        match api_client.workflow_graph(&run.workflow_name).await {
            Ok(graph) => {
                let layout = graph::build_layout(&graph, &run.nodes);
                app.layout = Some(layout);
                app.banner = None; // clear any previous error
            }
            Err(e) => {
                app.banner = Some(format!("API: {e}"));
                // Keep the old layout — stale is better than no layout.
            }
        }
    } else {
        app.layout = None; // No layout if session selected
        app.banner = None;
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────
//
// The I/O shell (terminal setup/teardown, live render, live DB poll) is tested
// manually — see the smoke-test notes in tasks.md § Notes.
//
// The pure key-handling logic IS unit-tested here.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::workflows::{NodeState, RunStatus, WorkflowRun};

    // ── Helper ────────────────────────────────────────────────────────────────

    fn make_run(id: &str, node_count: usize) -> WorkflowRun {
        let nodes = (0..node_count)
            .map(|i| NodeState {
                id: format!("node{i}"),
                name: format!("Node{i}"),
                status: RunStatus::Pending,
                depends_on: vec![],
                input: None,
                output: None,
                error: None,
                tokens_in: None,
                tokens_out: None,
                model: None,
                started_at: None,
                elapsed_secs: None,
            })
            .collect();
        WorkflowRun {
            id: id.to_string(),
            workflow_name: "wf".to_string(),
            status: RunStatus::Running,
            budget_halt: None,
            nodes,
            started_at: None,
            elapsed_secs: None,
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    fn app_with_runs(run_node_counts: &[usize]) -> App {
        let mut app = App::new();
        let runs: Vec<_> = run_node_counts
            .iter()
            .enumerate()
            .map(|(i, &n)| make_run(&format!("r{i}"), n))
            .collect();
        app.items = crate::monitor::app::build_mission_items(&[], &runs);
        app
    }

    // ── quit keys ─────────────────────────────────────────────────────────────

    #[test]
    fn key_q_sets_quit() {
        let mut app = App::new();
        handle_key(&mut app, key(KeyCode::Char('q')));
        assert!(app.should_quit);
    }

    #[test]
    fn key_esc_sets_quit() {
        let mut app = App::new();
        handle_key(&mut app, key(KeyCode::Esc));
        assert!(app.should_quit);
    }

    #[test]
    fn ctrl_c_sets_quit() {
        let mut app = App::new();
        handle_key(&mut app, ctrl_key(KeyCode::Char('c')));
        assert!(app.should_quit);
    }

    // ── node navigation ───────────────────────────────────────────────────────

    #[test]
    fn down_arrow_advances_node() {
        let mut app = app_with_runs(&[3]);
        handle_key(&mut app, key(KeyCode::Down));
        assert_eq!(app.selected_node, 1);
    }

    #[test]
    fn j_key_advances_node() {
        let mut app = app_with_runs(&[3]);
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.selected_node, 1);
    }

    #[test]
    fn up_arrow_retreats_node() {
        let mut app = app_with_runs(&[3]);
        app.selected_node = 2;
        handle_key(&mut app, key(KeyCode::Up));
        assert_eq!(app.selected_node, 1);
    }

    #[test]
    fn k_key_retreats_node() {
        let mut app = app_with_runs(&[3]);
        app.selected_node = 2;
        handle_key(&mut app, key(KeyCode::Char('k')));
        assert_eq!(app.selected_node, 1);
    }

    #[test]
    fn down_clamps_at_last_node() {
        let mut app = app_with_runs(&[2]);
        app.selected_node = 1;
        handle_key(&mut app, key(KeyCode::Down));
        assert_eq!(app.selected_node, 1, "must not advance past last node");
    }

    #[test]
    fn up_clamps_at_zero() {
        let mut app = app_with_runs(&[3]);
        app.selected_node = 0;
        handle_key(&mut app, key(KeyCode::Up));
        assert_eq!(app.selected_node, 0, "must not go below zero");
    }

    // ── run navigation ────────────────────────────────────────────────────────

    #[test]
    fn right_arrow_advances_run() {
        let mut app = app_with_runs(&[2, 2]);
        handle_key(&mut app, key(KeyCode::Right));
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn n_key_advances_run() {
        let mut app = app_with_runs(&[2, 2]);
        handle_key(&mut app, key(KeyCode::Char('n')));
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn left_arrow_retreats_run() {
        let mut app = app_with_runs(&[2, 2]);
        app.selected = 1;
        handle_key(&mut app, key(KeyCode::Left));
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn p_key_retreats_run() {
        let mut app = app_with_runs(&[2, 2]);
        app.selected = 1;
        handle_key(&mut app, key(KeyCode::Char('p')));
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn right_clamps_at_last_run() {
        let mut app = app_with_runs(&[2, 2]);
        app.selected = 1;
        handle_key(&mut app, key(KeyCode::Right));
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn left_clamps_at_first_run() {
        let mut app = app_with_runs(&[2, 2]);
        app.selected = 0;
        handle_key(&mut app, key(KeyCode::Left));
        assert_eq!(app.selected, 0);
    }

    // ── unknown key is ignored ────────────────────────────────────────────────

    #[test]
    fn unknown_key_does_not_change_state() {
        let mut app = app_with_runs(&[3]);
        app.selected_node = 1;
        handle_key(&mut app, key(KeyCode::F(5)));
        assert!(!app.should_quit);
        assert_eq!(app.selected_node, 1);
        assert_eq!(app.selected, 0);
    }

    // ── error-path: degrade on empty runs ────────────────────────────────────
    // `poll_and_update` is async/I/O and can't be unit-tested here; its
    // degrade branch is exercised in the smoke test (recorded in tasks.md §Notes).
}
