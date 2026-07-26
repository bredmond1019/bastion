//! Headless `bastion monitor --watch` — the no-TTY sibling of the TUI event
//! loop, mirroring `costs::watch`'s construction exactly (Rule 6, the
//! established thin-shell/pure-core split): a pure per-tick decision
//! ([`tick`]) reused by both this module and the TUI wiring in
//! `monitor::events`, a pure plain-text renderer ([`render_plain_summary`]),
//! and a thin fetch→tick→print→sleep I/O shell ([`run`]).
//!
//! `--watch` exists for scripted/CI/no-TTY use: it never touches crossterm,
//! ratatui, or the alternate screen, and it exits cleanly on Ctrl-C like any
//! other polling loop in this crate.

use std::time::Duration;

use anyhow::Result;

use super::alerts::{self, AlertEvent, AlertState};
use crate::config::Config;
use crate::db::workflows::{self, WorkflowRun};

// ── Pure: per-tick decision ─────────────────────────────────────────────────

/// The pure decision for a single headless watch tick: what to render, and
/// which alerts (if any) this tick's runs produced against `state`.
///
/// Built by [`tick`] from the freshly-fetched `runs`; consumed by [`run`]'s
/// thin I/O shell, which does the actual printing / notification dispatch.
#[derive(Debug, Clone, PartialEq)]
pub struct WatchTickOutcome {
    /// The rendered plain-text summary for this tick (always re-rendered).
    pub rendered: String,
    /// Alerts detected this tick via [`alerts::detect_alerts`] — the same
    /// alerting core the TUI wiring in `monitor::events` uses, so headless
    /// and TUI modes never diverge on what counts as alert-worthy.
    pub alerts: Vec<AlertEvent>,
}

/// Evaluate one `--watch` tick against `state` and `runs`. Pure — no I/O, no
/// sleep, no fetch.
pub fn tick(state: &mut AlertState, runs: &[WorkflowRun]) -> WatchTickOutcome {
    let rendered = render_plain_summary(runs);
    let alerts = alerts::detect_alerts(state, runs);
    WatchTickOutcome { rendered, alerts }
}

/// Render a plain-text, no-color, no-TTY-required summary: one line per run,
/// followed by one indented line per node. Used by `--watch` in place of the
/// ratatui two-pane graph view.
///
/// Pure function — exhaustively unit-tested below.
pub fn render_plain_summary(runs: &[WorkflowRun]) -> String {
    if runs.is_empty() {
        return "no active workflow runs\n".to_string();
    }

    let mut out = String::new();
    for run in runs {
        out.push_str(&format!(
            "{} [{}] {:?}\n",
            run.workflow_name, run.id, run.status
        ));
        for node in &run.nodes {
            out.push_str(&format!("  - {} {:?}\n", node.name, node.status));
        }
    }
    out
}

// ── Thin I/O shell ───────────────────────────────────────────────────────────

/// Run `bastion monitor --watch [--workflow-id <id>]`.
///
/// Polls on `Config::poll_interval_secs`, re-running `workflows::get_run_state`
/// (when `workflow_id` is given) or `workflows::list_active_runs` (otherwise),
/// re-rendering [`render_plain_summary`] and dispatching any [`AlertEvent`]s
/// through `notify::send_macos_notification` (skipped entirely when
/// `Config::notify_enabled` is `false` — but [`AlertState`] is still updated
/// either way, since state tracking must not depend on whether notifications
/// are enabled) each tick until interrupted (Ctrl-C — no in-process stop
/// condition).
///
/// Degrades rather than panics on a DB failure: prints a message and keeps
/// polling — the same posture as `costs::watch::run`.
///
/// Manually smoke-tested (Rule 6) — see the task spec's Notes for the
/// recorded pass/fail (correct plain-text output against a live run, and a
/// clean Ctrl-C exit).
pub async fn run(workflow_id: Option<String>) -> Result<()> {
    let config = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "bastion monitor --watch: {e}\n\
                 Set DATABASE_URL in your .env file or environment:\n\
                 DATABASE_URL=postgres://user:pass@localhost:5432/dbname"
            );
            return Ok(());
        }
    };

    let mut state = AlertState::default();
    let poll_interval = Duration::from_secs(config.poll_interval_secs.max(1));

    loop {
        let fetch = match &workflow_id {
            Some(id) => workflows::get_run_state(&config.database_url, id)
                .await
                .map(|run| vec![run]),
            None => workflows::list_active_runs(&config.database_url).await,
        };

        match fetch {
            Ok(runs) => {
                let outcome = tick(&mut state, &runs);
                print!("{}", outcome.rendered);
                alerts::dispatch_alerts(&outcome.alerts, config.notify_enabled);
            }
            Err(e) => {
                eprintln!("bastion monitor --watch: could not fetch run state: {e}");
                // Transient failure — keep the loop alive; do not reset
                // `state`, so a real transition detected on the next
                // successful tick is still evaluated against the last known
                // statuses.
            }
        }

        tokio::time::sleep(poll_interval).await;
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::workflows::{NodeState, RunStatus};

    fn node(id: &str, name: &str, status: RunStatus) -> NodeState {
        NodeState {
            id: id.to_string(),
            name: name.to_string(),
            status,
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

    fn run_fixture(
        id: &str,
        workflow_name: &str,
        status: RunStatus,
        nodes: Vec<NodeState>,
    ) -> WorkflowRun {
        WorkflowRun {
            id: id.to_string(),
            workflow_name: workflow_name.to_string(),
            status,
            budget_halt: None,
            nodes,
            started_at: None,
            elapsed_secs: None,
        }
    }

    // ── render_plain_summary ─────────────────────────────────────────────────

    #[test]
    fn render_empty_runs() {
        assert_eq!(render_plain_summary(&[]), "no active workflow runs\n");
    }

    #[test]
    fn render_single_run_no_nodes() {
        let runs = vec![run_fixture("r1", "pipeline", RunStatus::Running, vec![])];
        assert_eq!(render_plain_summary(&runs), "pipeline [r1] Running\n");
    }

    #[test]
    fn render_single_run_multiple_nodes_mixed_statuses() {
        let runs = vec![run_fixture(
            "r1",
            "pipeline",
            RunStatus::Running,
            vec![
                node("n1", "IngestNode", RunStatus::Success),
                node("n2", "SummaryNode", RunStatus::Running),
                node("n3", "ReviewNode", RunStatus::Pending),
            ],
        )];
        assert_eq!(
            render_plain_summary(&runs),
            "pipeline [r1] Running\n\
             \x20\x20- IngestNode Success\n\
             \x20\x20- SummaryNode Running\n\
             \x20\x20- ReviewNode Pending\n"
        );
    }

    #[test]
    fn render_multiple_runs_each_with_nodes() {
        let runs = vec![
            run_fixture(
                "r1",
                "pipeline-a",
                RunStatus::Failed,
                vec![node("n1", "Step1", RunStatus::Failed)],
            ),
            run_fixture(
                "r2",
                "pipeline-b",
                RunStatus::Success,
                vec![node("n1", "Step1", RunStatus::Success)],
            ),
        ];
        assert_eq!(
            render_plain_summary(&runs),
            "pipeline-a [r1] Failed\n\
             \x20\x20- Step1 Failed\n\
             pipeline-b [r2] Success\n\
             \x20\x20- Step1 Success\n"
        );
    }

    // ── tick: rendering always happens, alerts reuse detect_alerts ──────────

    #[test]
    fn tick_always_renders() {
        let mut state = AlertState::default();
        let runs = vec![run_fixture("r1", "pipeline", RunStatus::Running, vec![])];
        let outcome = tick(&mut state, &runs);
        assert_eq!(outcome.rendered, "pipeline [r1] Running\n");
        assert!(outcome.alerts.is_empty());
    }

    #[test]
    fn tick_surfaces_fresh_alert() {
        let mut state = AlertState::default();
        let runs = vec![run_fixture("r1", "pipeline", RunStatus::Failed, vec![])];
        let outcome = tick(&mut state, &runs);
        assert_eq!(outcome.alerts.len(), 1);
        assert!(matches!(outcome.alerts[0], AlertEvent::RunFailed { .. }));
    }

    #[test]
    fn tick_does_not_realert_on_sustained_status() {
        let mut state = AlertState::default();
        let runs = vec![run_fixture("r1", "pipeline", RunStatus::Failed, vec![])];
        let first = tick(&mut state, &runs);
        assert_eq!(first.alerts.len(), 1);

        let second = tick(&mut state, &runs);
        assert!(second.alerts.is_empty());
        // Rendering still happens even with no new alerts.
        assert_eq!(second.rendered, "pipeline [r1] Failed\n");
    }
}
