//! End-to-end coverage for the three defects BA.18.A exists to fix (Task 5).
//!
//! `poller.rs`'s own `#[cfg(test)] mod tests` already exercises
//! [`super::poller::tick_decision`] and [`super::poller::BlockedEdgePoller::tick`]
//! in isolation (no [`Hub`](crate::serve::ws::server::Hub), no actix system).
//! This module proves the same three scenarios hold through the *wired*
//! path production actually runs: a real `Hub` actor, started the same way
//! the serve app factory starts it (`Hub::new(...).start()` — see
//! `src/serve/mod.rs`'s `build_app_with_live_store` test helper), with the
//! poller's `.with_hub(hub)` fan-out attached, and **zero** WebSocket clients
//! ever `Connect`ed or `Subscribe`d to it. Nothing here spawns a real tmux
//! server — every capture is an injected closure, exactly like `poller.rs`'s
//! own seam.
//!
//! Three scenarios, each pinned to the Goal/Acceptance-Criteria language:
//! - [`zero_subscribers_still_produces_a_durable_sink_record`] — "with zero
//!   WebSocket clients ever connected, blocking a session produces a
//!   durable sink record within one poll interval."
//! - [`restart_with_already_blocked_sessions_emits_nothing`] — "restarting
//!   the process with N already-blocked sessions emits zero edges for
//!   sessions that were already blocked before the restart."
//! - [`re_block_after_resolution_produces_two_records`] — "a session that
//!   blocks, is answered, and blocks again produces two records, not one."
//!
//! Each test also re-opens the sink from a *second*, independently
//! constructed [`BlockedEdgeSink`] handle to assert the record is readable
//! from another process's perspective (Task 2's "readable by a separate
//! process" criterion), not just from the handle that wrote it.

use std::collections::HashMap;

use actix::Actor;
use engine_serve::live_state::LiveStateStore;

use crate::config::FileConfig;
use crate::detect::AgentState;
use crate::serve::ws::server::Hub;
use crate::testsupport::unique_temp_dir;

use super::poller::BlockedEdgePoller;
use super::sink::BlockedEdgeSink;

/// Start a `Hub` the same way the serve app factory does
/// (`src/serve/mod.rs`'s `build_app_with_live_store`) — a fresh registry, a
/// fresh empty live-state store, and no engine mounted. No WebSocket
/// connection is ever made against it in any test in this module: the point
/// is that the poller's sink write does not depend on one existing.
fn start_test_hub() -> actix::Addr<Hub> {
    Hub::new(
        2,
        FileConfig::default(),
        LiveStateStore::new(),
        (false, None),
    )
    .start()
}

fn state(name: &str, state: AgentState) -> (String, AgentState) {
    (name.to_string(), state)
}

fn temp_sink_path(name: &str) -> std::path::PathBuf {
    unique_temp_dir(&format!("bastion-blocked-edge-e2e-{name}"))
}

// ── Scenario 1: zero subscribers ────────────────────────────────────────────

#[actix_web::test]
async fn zero_subscribers_still_produces_a_durable_sink_record() {
    let path = temp_sink_path("no-subscriber");
    let sink = BlockedEdgeSink::new(&path);
    let hub = start_test_hub();

    let mut ticks: Vec<Vec<(String, AgentState)>> = vec![
        vec![state("sess-a", AgentState::Working)], // seed tick
        vec![state("sess-a", AgentState::Blocked)], // rising edge, no subscribers
    ];
    ticks.reverse();
    let capture = Box::new(move || Ok(ticks.pop().unwrap_or_default()));
    let mut poller = BlockedEdgePoller::with_capture(sink, "test-host", capture).with_hub(hub);

    poller.tick(); // seed — no WS client has ever connected
    poller.tick(); // sess-a crosses into Blocked, still no WS client

    // Reopen from an independent handle — the "readable by a separate
    // process" criterion, exercised here as "readable by a separate handle".
    let reader = BlockedEdgeSink::new(&path);
    let records = reader.read_all().expect("read_all should succeed");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].session, "sess-a");
    assert_eq!(records[0].to, AgentState::Blocked);

    let _ = std::fs::remove_file(&path);
}

// ── Scenario 2: restart storm suppression ───────────────────────────────────

#[actix_web::test]
async fn restart_with_already_blocked_sessions_emits_nothing() {
    let path = temp_sink_path("restart-storm");
    let sink = BlockedEdgeSink::new(&path);
    let hub = start_test_hub();

    // Simulates a restart landing on a box with three sessions that were
    // already Blocked before this process started — `prev` starts empty
    // (the in-process map does not survive a restart), so this is exactly
    // the storm scenario the Goal describes.
    let capture = Box::new(|| {
        Ok(vec![
            state("sess-a", AgentState::Blocked),
            state("sess-b", AgentState::Blocked),
            state("sess-c", AgentState::Blocked),
        ])
    });
    let mut poller = BlockedEdgePoller::with_capture(sink, "test-host", capture).with_hub(hub);

    poller.tick(); // the first tick after "restart" — seed only, must emit nothing

    let reader = BlockedEdgeSink::new(&path);
    let records = reader.read_all().expect("read_all should succeed");
    assert!(
        records.is_empty(),
        "expected zero records on the seed tick, got {records:?}"
    );

    let _ = std::fs::remove_file(&path);
}

/// A second tick after the seed tick, with sessions still Blocked, must
/// likewise stay silent — the seed pass is what suppresses the storm, not
/// merely the first observation ever made of any one session.
#[actix_web::test]
async fn restart_then_steady_state_blocked_emits_nothing_either() {
    let path = temp_sink_path("restart-storm-steady-state");
    let sink = BlockedEdgeSink::new(&path);
    let hub = start_test_hub();

    let capture = Box::new(|| {
        Ok(vec![
            state("sess-a", AgentState::Blocked),
            state("sess-b", AgentState::Blocked),
        ])
    });
    let mut poller = BlockedEdgePoller::with_capture(sink, "test-host", capture).with_hub(hub);

    poller.tick(); // seed
    poller.tick(); // still blocked, no new crossing

    let reader = BlockedEdgeSink::new(&path);
    let records = reader.read_all().expect("read_all should succeed");
    assert!(records.is_empty());

    let _ = std::fs::remove_file(&path);
}

// ── Scenario 3: blocked, answered, re-blocked ───────────────────────────────

#[actix_web::test]
async fn re_block_after_resolution_produces_two_records() {
    let path = temp_sink_path("re-block");
    let sink = BlockedEdgeSink::new(&path);
    let hub = start_test_hub();

    let mut ticks: Vec<Vec<(String, AgentState)>> = vec![
        vec![state("sess-a", AgentState::Working)], // seed
        vec![state("sess-a", AgentState::Blocked)], // first block (record #1)
        vec![state("sess-a", AgentState::Working)], // answered
        vec![state("sess-a", AgentState::Blocked)], // second block (record #2)
    ];
    ticks.reverse();
    let capture = Box::new(move || Ok(ticks.pop().unwrap_or_default()));
    let mut poller = BlockedEdgePoller::with_capture(sink, "test-host", capture).with_hub(hub);

    for _ in 0..4 {
        poller.tick();
    }

    let reader = BlockedEdgeSink::new(&path);
    let records = reader.read_all().expect("read_all should succeed");
    assert_eq!(
        records.len(),
        2,
        "expected two distinct block events, got {records:?}"
    );
    assert!(records.iter().all(|r| r.session == "sess-a"));
    // Two distinct timestamps — the two events are independently
    // observable, not a single coalesced record.
    assert_ne!(records[0].observed_at, records[1].observed_at);

    let _ = std::fs::remove_file(&path);
}

// ── Scenario 4: edge_tx seam alongside the wired hub (BA.20.C task 3) ──────

/// The `with_edge_tx` seam (task 3) is additive on top of the existing
/// `with_hub` fan-out — both a live `Hub` (with zero WS subscribers, exactly
/// as every scenario above) and a wired edge-notification receiver observe
/// the same crossing the durable sink records, from the same tick.
#[actix_web::test]
async fn edge_tx_and_hub_both_observe_the_same_crossing_the_sink_recorded() {
    let path = temp_sink_path("edge-tx-with-hub");
    let sink = BlockedEdgeSink::new(&path);
    let hub = start_test_hub();

    let mut ticks: Vec<Vec<(String, AgentState)>> = vec![
        vec![state("sess-a", AgentState::Working)], // seed
        vec![state("sess-a", AgentState::Blocked)], // rising edge
    ];
    ticks.reverse();
    let capture = Box::new(move || Ok(ticks.pop().unwrap_or_default()));
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let mut poller = BlockedEdgePoller::with_capture(sink, "test-host", capture)
        .with_hub(hub)
        .with_edge_tx(tx);

    poller.tick(); // seed
    poller.tick(); // sess-a crosses into Blocked

    let reader = BlockedEdgeSink::new(&path);
    let records = reader.read_all().expect("read_all should succeed");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].session, "sess-a");

    let received = rx.try_recv().expect("edge_tx should have the crossing");
    assert_eq!(received.session, "sess-a");
    assert_eq!(received.to, AgentState::Blocked);
    assert!(
        rx.try_recv().is_err(),
        "exactly one crossing was emitted this run"
    );

    let _ = std::fs::remove_file(&path);
}

// ── No real tmux server anywhere in this module ─────────────────────────────

/// Documents, rather than mechanically enforces, the "no test spawns a real
/// tmux server" acceptance criterion: every capture closure in this module
/// is a plain in-memory `Vec` built from `state(...)` literals — none of
/// them calls `crate::sessions::tmux::list_sessions_raw` /
/// `capture_pane_raw`, and `BlockedEdgePoller::with_capture` (never `::new`,
/// which does call into real tmux) is the only constructor used above.
#[test]
fn tick_decision_seam_used_here_never_touches_prev_map_directly() {
    // A pure sanity check that this module's helper composes with the
    // shared `tick_decision` seam the same way `poller.rs`'s own tests do,
    // so a future refactor of either can't silently diverge in meaning.
    let prev: HashMap<String, AgentState> = HashMap::new();
    let current = vec![state("sess-a", AgentState::Blocked)];
    let (crossing, _next) = super::poller::tick_decision(false, &prev, &current);
    assert!(crossing.is_empty());
}
