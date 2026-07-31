// Pure alert-decision core behind `bastion monitor`'s desktop notifications
// (Part B). Mirrors `costs::watch`'s tick shape — a previous-tick-state param
// threaded through a pure decision fn — generalized from one scalar
// `GateVerdict` to per-run and per-node status maps, since a monitor tick can
// carry many independently-transitioning runs/nodes instead of one budget
// verdict.
//
// Shared by both consumers of a poll tick: the TUI wiring in
// `monitor::events::run_event_loop` and the headless `monitor::watch::run`
// loop. Neither duplicates the alerting decision — both call
// [`detect_alerts`] against their own [`AlertState`] instance.
//
// Deliberately NOT unified with `costs::budget`'s `detect_crossing`/`Crossing`
// (scope boundary, see the Part B plan) — that module tracks one scalar gate
// verdict; this one tracks a growing set of runs/nodes with independent
// lifecycles. Forcing a shared abstraction across the two domains would cost
// more in indirection than it would save in duplicated code.

use std::collections::HashMap;

use crate::db::workflows::{RunStatus, WorkflowRun};

/// Carries the previous tick's run/node statuses so [`detect_alerts`] can
/// distinguish a fresh transition from a sustained state. One instance lives
/// for the lifetime of a `bastion monitor` invocation (TUI or `--watch`);
/// `Default` gives every caller an empty starting state.
///
/// Node keys are `(run_id, node_id)` rather than bare node id: node ids are
/// only unique *within* a run (they're the node's class name — see
/// `db::workflows::parse_task_context`), so two concurrently-active runs of
/// the same workflow type would otherwise collide on the same node id.
///
/// Design choice — entries are never pruned: once a run or node is observed,
/// its last-known status stays in the map even if a later fetch omits that
/// run/node entirely (e.g. `list_active_runs` drops a run the moment none of
/// its nodes are non-terminal — see the note on [`detect_alerts`]). Pruning
/// would risk a stale-but-real id reappearing later (e.g. after a gap in
/// polling) being treated as brand new and re-alerting on a state it already
/// alerted on; keeping the entry costs a few bytes of memory for the life of
/// the process and is the safer default.
#[derive(Debug, Clone, Default)]
pub struct AlertState {
    node_status: HashMap<(String, String), RunStatus>,
    run_status: HashMap<String, RunStatus>,
}

/// One alert-worthy transition detected by [`detect_alerts`].
#[derive(Debug, Clone, PartialEq)]
pub enum AlertEvent {
    /// A node transitioned into `RunStatus::Failed`. Node-level alerting is
    /// deliberately narrow: `Pending`/`Running`/`Success` never alert at the
    /// node level (only the run-level `RunDone` covers the success case).
    NodeFailed {
        run_id: String,
        node_id: String,
        node_name: String,
    },
    /// A run transitioned into `RunStatus::Failed`.
    RunFailed {
        run_id: String,
        workflow_name: String,
    },
    /// A run transitioned into `RunStatus::Cancelled` (v1.1.0
    /// `metadata.cancellation`).
    RunCancelled {
        run_id: String,
        workflow_name: String,
    },
    /// A run transitioned into `RunStatus::BudgetHalted` (v1.1.0
    /// `metadata.budget`).
    RunBudgetHalted {
        run_id: String,
        workflow_name: String,
    },
    /// A run transitioned into `RunStatus::Suspended` (a real, resumable
    /// pause — `metadata.suspension.suspended == true`). Not terminal: a
    /// later resume transitions the run away from `Suspended` like any other
    /// status change, and re-alerts if it is suspended again later.
    RunSuspended {
        run_id: String,
        workflow_name: String,
    },
    /// A run transitioned into `RunStatus::Success`. Run-level only — there
    /// is no equivalent "node done" event (a node reaching `Success` is the
    /// unremarkable common case, not alert-worthy on its own).
    RunDone {
        run_id: String,
        workflow_name: String,
    },
}

/// Detect alert-worthy transitions in `runs` against `state`'s previously
/// recorded statuses, then update `state` in place. Pure — no I/O.
///
/// **Fresh-transition semantics:** a run/node is "fresh" into an alertable
/// status whenever its current status differs from what `state` last
/// recorded for that id — including the very first time that id is ever
/// seen. This mirrors `costs::watch::tick`'s own
/// `tick_first_tick_breach_is_fresh_alert` precedent: a run that is already
/// `Failed` (or a node already `Failed`) the very first time `bastion
/// monitor` observes it is exactly as alert-worthy as one that transitions
/// while being watched — the operator wasn't there to see it happen either
/// way, so suppressing the first-tick case would silently swallow real
/// failures that occurred between polls (or before the monitor was started).
///
/// **Sustained state does not re-alert:** once a run/node's current status
/// has been recorded, repeated ticks with the same status produce no further
/// events — matching `costs::watch`'s "sustained breach does not re-alert"
/// rule. A later transition away and back (e.g. `Cancelled` is not a real
/// recovery path today, but the general rule holds for any state graph) is
/// fresh again and alerts again.
///
/// **Known interaction with `list_active_runs`:** when called with the "all
/// active runs" query (as opposed to a `--workflow-id`-scoped single run),
/// the DB layer only returns runs with at least one non-terminal node — so a
/// run's transition to a terminal status (`Success`/`Failed`) may drop it
/// from `runs` in the very same tick that status is reached, before this
/// function ever observes the transition. This is a property of the query
/// this function is fed, not a bug in the detection logic itself; the
/// `--workflow-id`-scoped path (`get_run_state`) has no such gap.
pub fn detect_alerts(state: &mut AlertState, runs: &[WorkflowRun]) -> Vec<AlertEvent> {
    let mut events = Vec::new();

    for run in runs {
        let previous_run_status = state.run_status.get(&run.id).cloned();
        if previous_run_status.as_ref() != Some(&run.status) {
            match run.status {
                RunStatus::Failed => events.push(AlertEvent::RunFailed {
                    run_id: run.id.clone(),
                    workflow_name: run.workflow_name.clone(),
                }),
                RunStatus::Cancelled => events.push(AlertEvent::RunCancelled {
                    run_id: run.id.clone(),
                    workflow_name: run.workflow_name.clone(),
                }),
                RunStatus::BudgetHalted => events.push(AlertEvent::RunBudgetHalted {
                    run_id: run.id.clone(),
                    workflow_name: run.workflow_name.clone(),
                }),
                RunStatus::Success => events.push(AlertEvent::RunDone {
                    run_id: run.id.clone(),
                    workflow_name: run.workflow_name.clone(),
                }),
                RunStatus::Suspended => events.push(AlertEvent::RunSuspended {
                    run_id: run.id.clone(),
                    workflow_name: run.workflow_name.clone(),
                }),
                RunStatus::Running | RunStatus::Pending => {}
            }
        }
        state.run_status.insert(run.id.clone(), run.status.clone());

        for node in &run.nodes {
            let key = (run.id.clone(), node.id.clone());
            let previous_node_status = state.node_status.get(&key).cloned();
            if previous_node_status.as_ref() != Some(&node.status)
                && node.status == RunStatus::Failed
            {
                events.push(AlertEvent::NodeFailed {
                    run_id: run.id.clone(),
                    node_id: node.id.clone(),
                    node_name: node.name.clone(),
                });
            }
            state.node_status.insert(key, node.status.clone());
        }
    }

    events
}

/// Format the desktop-notification `(title, message, sound)` for `event`.
/// Pure function — no I/O.
///
/// Sound choice (bastion's own convention — no existing Rust-side precedent
/// to match; consistent with the shell-side `notify_mac` helper this block's
/// bash sibling uses): `Basso` for failures (node or run), `Sosumi` for
/// operator-initiated-or-adjacent terminal states that aren't outright
/// failures (`Cancelled`, `BudgetHalted` — both stop a run short of success
/// but neither is "something broke"), `Pop` for `Suspended` — a mild,
/// non-alarming cue since a pause is expected/benign and, unlike the other
/// two, not terminal — `Glass` for a clean success.
pub fn alert_notification(event: &AlertEvent) -> (String, String, &'static str) {
    match event {
        AlertEvent::NodeFailed {
            run_id,
            node_id,
            node_name,
        } => (
            "bastion: node failed".to_string(),
            format!("{node_name} ({node_id}) failed in run {run_id}"),
            "Basso",
        ),
        AlertEvent::RunFailed {
            run_id,
            workflow_name,
        } => (
            "bastion: run failed".to_string(),
            format!("{workflow_name} (run {run_id}) failed"),
            "Basso",
        ),
        AlertEvent::RunCancelled {
            run_id,
            workflow_name,
        } => (
            "bastion: run cancelled".to_string(),
            format!("{workflow_name} (run {run_id}) was cancelled"),
            "Sosumi",
        ),
        AlertEvent::RunBudgetHalted {
            run_id,
            workflow_name,
        } => (
            "bastion: run budget-halted".to_string(),
            format!("{workflow_name} (run {run_id}) halted by a budget cap"),
            "Sosumi",
        ),
        AlertEvent::RunSuspended {
            run_id,
            workflow_name,
        } => (
            "bastion: run suspended".to_string(),
            format!("{workflow_name} (run {run_id}) is suspended, awaiting resume"),
            "Pop",
        ),
        AlertEvent::RunDone {
            run_id,
            workflow_name,
        } => (
            "bastion: run done".to_string(),
            format!("{workflow_name} (run {run_id}) completed successfully"),
            "Glass",
        ),
    }
}

/// Dispatch desktop notifications for `events`, shared by both consumers of a
/// poll tick (`monitor::events::run_event_loop`'s TUI wiring and the headless
/// `monitor::watch::run` loop) so the dispatch policy — log every event
/// unconditionally, notify only when `notify_enabled` — can never drift
/// between the two call sites (previously each hand-rolled an identical
/// `for` loop independently).
///
/// Notification delivery is fire-and-forget via
/// [`crate::notify::dispatch_macos_notification`], which never blocks the
/// caller — safe to call from inside a `tokio::select!` loop that also owns
/// keyboard-event handling.
pub fn dispatch_alerts(events: &[AlertEvent], notify_enabled: bool) {
    for event in events {
        tracing::warn!(event = ?event, "monitor alert");
        if notify_enabled {
            let (title, message, sound) = alert_notification(event);
            crate::notify::dispatch_macos_notification(title, message, sound);
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::workflows::NodeState;

    fn node(id: &str, status: RunStatus) -> NodeState {
        NodeState {
            id: id.to_string(),
            name: format!("Node-{id}"),
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

    fn run(id: &str, status: RunStatus, nodes: Vec<NodeState>) -> WorkflowRun {
        WorkflowRun {
            id: id.to_string(),
            workflow_name: "pipeline".to_string(),
            status,
            budget_halt: None,
            nodes,
            started_at: None,
            elapsed_secs: None,
        }
    }

    // ── run-level: first-tick-already-terminal alerts ───────────────────────

    #[test]
    fn run_first_tick_already_failed_alerts() {
        let mut state = AlertState::default();
        let runs = vec![run("r1", RunStatus::Failed, vec![])];
        let events = detect_alerts(&mut state, &runs);
        assert_eq!(
            events,
            vec![AlertEvent::RunFailed {
                run_id: "r1".to_string(),
                workflow_name: "pipeline".to_string(),
            }]
        );
    }

    #[test]
    fn run_first_tick_running_does_not_alert() {
        let mut state = AlertState::default();
        let runs = vec![run("r1", RunStatus::Running, vec![])];
        let events = detect_alerts(&mut state, &runs);
        assert!(events.is_empty());
    }

    // ── run-level: no re-alert while status unchanged ───────────────────────

    #[test]
    fn run_no_realert_while_sustained_failed() {
        let mut state = AlertState::default();
        let first = detect_alerts(&mut state, &[run("r1", RunStatus::Failed, vec![])]);
        assert_eq!(first.len(), 1);

        let second = detect_alerts(&mut state, &[run("r1", RunStatus::Failed, vec![])]);
        assert!(second.is_empty(), "sustained failure must not re-alert");
    }

    // ── run-level: fresh transition alerts exactly once ─────────────────────

    #[test]
    fn run_fresh_transition_running_to_failed_alerts() {
        let mut state = AlertState::default();
        let first = detect_alerts(&mut state, &[run("r1", RunStatus::Running, vec![])]);
        assert!(first.is_empty());

        let second = detect_alerts(&mut state, &[run("r1", RunStatus::Failed, vec![])]);
        assert_eq!(
            second,
            vec![AlertEvent::RunFailed {
                run_id: "r1".to_string(),
                workflow_name: "pipeline".to_string(),
            }]
        );
    }

    // ── run-level: recovery then refresh re-alerts ──────────────────────────

    #[test]
    fn run_recovery_then_refresh_realerts() {
        let mut state = AlertState::default();
        let t1 = detect_alerts(&mut state, &[run("r1", RunStatus::Cancelled, vec![])]);
        assert_eq!(t1.len(), 1);

        // "Recovery" here means the run is re-observed running again (e.g. a
        // fresh trigger reusing state, or an id collision across a very long
        // watch session) — the general transition rule still applies.
        let t2 = detect_alerts(&mut state, &[run("r1", RunStatus::Running, vec![])]);
        assert!(t2.is_empty());

        let t3 = detect_alerts(&mut state, &[run("r1", RunStatus::Cancelled, vec![])]);
        assert_eq!(
            t3,
            vec![AlertEvent::RunCancelled {
                run_id: "r1".to_string(),
                workflow_name: "pipeline".to_string(),
            }]
        );
    }

    #[test]
    fn run_budget_halted_and_done_alert() {
        let mut state = AlertState::default();
        let events = detect_alerts(
            &mut state,
            &[
                run("r1", RunStatus::BudgetHalted, vec![]),
                run("r2", RunStatus::Success, vec![]),
            ],
        );
        assert_eq!(events.len(), 2);
        assert!(events.contains(&AlertEvent::RunBudgetHalted {
            run_id: "r1".to_string(),
            workflow_name: "pipeline".to_string(),
        }));
        assert!(events.contains(&AlertEvent::RunDone {
            run_id: "r2".to_string(),
            workflow_name: "pipeline".to_string(),
        }));
    }

    #[test]
    fn run_suspended_alerts_once_then_resume_realerts_on_re_suspend() {
        let mut state = AlertState::default();
        let t1 = detect_alerts(&mut state, &[run("r1", RunStatus::Suspended, vec![])]);
        assert_eq!(
            t1,
            vec![AlertEvent::RunSuspended {
                run_id: "r1".to_string(),
                workflow_name: "pipeline".to_string(),
            }]
        );

        // Sustained suspension does not re-alert (same rule as every other status).
        let t2 = detect_alerts(&mut state, &[run("r1", RunStatus::Suspended, vec![])]);
        assert!(t2.is_empty());

        // Resume, then a later re-suspend, alerts fresh again.
        let t3 = detect_alerts(&mut state, &[run("r1", RunStatus::Running, vec![])]);
        assert!(t3.is_empty());
        let t4 = detect_alerts(&mut state, &[run("r1", RunStatus::Suspended, vec![])]);
        assert_eq!(
            t4,
            vec![AlertEvent::RunSuspended {
                run_id: "r1".to_string(),
                workflow_name: "pipeline".to_string(),
            }]
        );
    }

    // ── node-level: first-tick-already-terminal alerts ──────────────────────

    #[test]
    fn node_first_tick_already_failed_alerts() {
        let mut state = AlertState::default();
        let runs = vec![run(
            "r1",
            RunStatus::Failed,
            vec![node("n1", RunStatus::Failed)],
        )];
        let events = detect_alerts(&mut state, &runs);
        assert!(events.contains(&AlertEvent::NodeFailed {
            run_id: "r1".to_string(),
            node_id: "n1".to_string(),
            node_name: "Node-n1".to_string(),
        }));
    }

    #[test]
    fn node_first_tick_non_failed_statuses_do_not_alert() {
        let mut state = AlertState::default();
        let runs = vec![run(
            "r1",
            RunStatus::Running,
            vec![
                node("n1", RunStatus::Pending),
                node("n2", RunStatus::Running),
                node("n3", RunStatus::Success),
            ],
        )];
        let events = detect_alerts(&mut state, &runs);
        assert!(
            events.is_empty(),
            "pending/running/success nodes never alert"
        );
    }

    // ── node-level: no re-alert while sustained ──────────────────────────────

    #[test]
    fn node_no_realert_while_sustained_failed() {
        let mut state = AlertState::default();
        let nodes = vec![node("n1", RunStatus::Failed)];
        let first = detect_alerts(&mut state, &[run("r1", RunStatus::Failed, nodes.clone())]);
        assert_eq!(first.len(), 2); // run-level + node-level, both fresh

        let second = detect_alerts(&mut state, &[run("r1", RunStatus::Failed, nodes)]);
        assert!(
            second.is_empty(),
            "sustained node failure must not re-alert"
        );
    }

    // ── node-level: fresh transition alerts exactly once ────────────────────

    #[test]
    fn node_fresh_transition_running_to_failed_alerts() {
        let mut state = AlertState::default();
        let first = detect_alerts(
            &mut state,
            &[run(
                "r1",
                RunStatus::Running,
                vec![node("n1", RunStatus::Running)],
            )],
        );
        assert!(first.is_empty());

        let second = detect_alerts(
            &mut state,
            &[run(
                "r1",
                RunStatus::Failed,
                vec![node("n1", RunStatus::Failed)],
            )],
        );
        assert!(second.contains(&AlertEvent::NodeFailed {
            run_id: "r1".to_string(),
            node_id: "n1".to_string(),
            node_name: "Node-n1".to_string(),
        }));
    }

    // ── node-level: recovery then refresh re-alerts ─────────────────────────

    #[test]
    fn node_recovery_then_refresh_realerts() {
        let mut state = AlertState::default();
        let t1 = detect_alerts(
            &mut state,
            &[run(
                "r1",
                RunStatus::Failed,
                vec![node("n1", RunStatus::Failed)],
            )],
        );
        assert!(
            t1.iter()
                .any(|e| matches!(e, AlertEvent::NodeFailed { .. }))
        );

        // A retried node (e.g. a future rerun-node feature) moves back to
        // running — no re-alert while running.
        let t2 = detect_alerts(
            &mut state,
            &[run(
                "r1",
                RunStatus::Running,
                vec![node("n1", RunStatus::Running)],
            )],
        );
        assert!(
            !t2.iter()
                .any(|e| matches!(e, AlertEvent::NodeFailed { .. }))
        );

        // Fails again — must be reported fresh, not swallowed.
        let t3 = detect_alerts(
            &mut state,
            &[run(
                "r1",
                RunStatus::Failed,
                vec![node("n1", RunStatus::Failed)],
            )],
        );
        assert!(
            t3.iter()
                .any(|e| matches!(e, AlertEvent::NodeFailed { .. }))
        );
    }

    // ── node ids are scoped per-run, not global ─────────────────────────────

    #[test]
    fn node_ids_are_scoped_per_run_not_global() {
        // Two different runs of the same workflow type both have a node "n1".
        // r1's n1 failing must not suppress r2's n1 failing (or vice versa) —
        // confirms the (run_id, node_id) composite key, not a bare node id.
        let mut state = AlertState::default();
        let events = detect_alerts(
            &mut state,
            &[
                run("r1", RunStatus::Failed, vec![node("n1", RunStatus::Failed)]),
                run(
                    "r2",
                    RunStatus::Running,
                    vec![node("n1", RunStatus::Running)],
                ),
            ],
        );
        assert!(events.contains(&AlertEvent::NodeFailed {
            run_id: "r1".to_string(),
            node_id: "n1".to_string(),
            node_name: "Node-n1".to_string(),
        }));

        // Now r2's n1 fails too — must alert independently of r1's history.
        let events2 = detect_alerts(
            &mut state,
            &[
                run("r1", RunStatus::Failed, vec![node("n1", RunStatus::Failed)]),
                run("r2", RunStatus::Failed, vec![node("n1", RunStatus::Failed)]),
            ],
        );
        assert_eq!(
            events2,
            vec![
                AlertEvent::RunFailed {
                    run_id: "r2".to_string(),
                    workflow_name: "pipeline".to_string(),
                },
                AlertEvent::NodeFailed {
                    run_id: "r2".to_string(),
                    node_id: "n1".to_string(),
                    node_name: "Node-n1".to_string(),
                },
            ]
        );
    }

    // ── alert_notification — one assertion per variant ──────────────────────

    #[test]
    fn notification_node_failed() {
        let (title, message, sound) = alert_notification(&AlertEvent::NodeFailed {
            run_id: "r1".to_string(),
            node_id: "n1".to_string(),
            node_name: "ValidationNode".to_string(),
        });
        assert!(title.contains("node failed"), "got: {title}");
        assert!(message.contains("ValidationNode"), "got: {message}");
        assert!(message.contains("n1"), "got: {message}");
        assert!(message.contains("r1"), "got: {message}");
        assert_eq!(sound, "Basso");
    }

    #[test]
    fn notification_run_failed() {
        let (title, message, sound) = alert_notification(&AlertEvent::RunFailed {
            run_id: "r1".to_string(),
            workflow_name: "pipeline".to_string(),
        });
        assert!(title.contains("run failed"), "got: {title}");
        assert!(message.contains("pipeline"), "got: {message}");
        assert!(message.contains("r1"), "got: {message}");
        assert_eq!(sound, "Basso");
    }

    #[test]
    fn notification_run_cancelled() {
        let (title, message, sound) = alert_notification(&AlertEvent::RunCancelled {
            run_id: "r1".to_string(),
            workflow_name: "pipeline".to_string(),
        });
        assert!(title.contains("cancelled"), "got: {title}");
        assert!(message.contains("cancelled"), "got: {message}");
        assert_eq!(sound, "Sosumi");
    }

    #[test]
    fn notification_run_budget_halted() {
        let (title, message, sound) = alert_notification(&AlertEvent::RunBudgetHalted {
            run_id: "r1".to_string(),
            workflow_name: "pipeline".to_string(),
        });
        assert!(title.contains("budget-halted"), "got: {title}");
        assert!(message.contains("budget"), "got: {message}");
        assert_eq!(sound, "Sosumi");
    }

    #[test]
    fn notification_run_suspended() {
        let (title, message, sound) = alert_notification(&AlertEvent::RunSuspended {
            run_id: "r1".to_string(),
            workflow_name: "pipeline".to_string(),
        });
        assert!(title.contains("suspended"), "got: {title}");
        assert!(message.contains("awaiting resume"), "got: {message}");
        assert_eq!(sound, "Pop");
    }

    #[test]
    fn notification_run_done() {
        let (title, message, sound) = alert_notification(&AlertEvent::RunDone {
            run_id: "r1".to_string(),
            workflow_name: "pipeline".to_string(),
        });
        assert!(title.contains("done"), "got: {title}");
        assert!(message.contains("completed successfully"), "got: {message}");
        assert_eq!(sound, "Glass");
    }
}
