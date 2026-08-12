//! Always-on Blocked rising-edge poller (BA.18.A task 3).
//!
//! `src/serve/ws/server.rs`'s sessions-topic poller only runs while at least
//! one WebSocket client is subscribed to the `sessions` topic — its capture
//! interval is installed on [`Subscribe`](crate::serve::ws::server::Subscribe)
//! and torn down on last unsubscribe/disconnect. That means the rising edge
//! `should_emit_needs_input` computes is only ever observed when somebody is
//! already watching, and (per the block's Goal) `sessions_last_state` is an
//! in-process actor field, so every restart re-observes every already-blocked
//! session as a fresh rising edge — a storm straight onto the notification
//! transport.
//!
//! [`BlockedEdgePoller`] fixes both: it is spawned once at server boot
//! (`src/serve/mod.rs::run_server`), independent of any WebSocket
//! subscription, and its very first tick only *seeds* its previous-state map
//! — it never emits from that tick, so a restart with N already-blocked
//! sessions produces zero records for them. Only transitions observed on a
//! tick *after* the seed tick can produce a [`super::sink::BlockedEdgeRecord`].
//!
//! Its capture composition (`list-sessions` once, then one `capture-pane` per
//! session) deliberately mirrors `ws/server.rs`'s sessions-topic poller
//! exactly, rather than inventing a second shape — see [`capture_states`].
//! Folding the two into one physical sweep (so a subscribed client and this
//! poller never double the tmux subprocess count) is Task 4's job
//! ("make the WS hub a consumer, not the owner").

use std::collections::HashMap;
use std::time::Duration;

use actix::Addr;
use chrono::Utc;

use crate::detect::AgentState;
use crate::serve::poll::{sessions_needing_input, sessions_snapshot};
use crate::serve::status::detect as status_detect;
use crate::serve::ws::server::{BlockedEdgeCrossed, Hub};
use crate::sessions::tmux;

use super::sink::{BlockedEdgeRecord, BlockedEdgeSink};

// ── Capture ──────────────────────────────────────────────────────────────────

/// One capture pass over every tmux session: list sessions, capture each
/// session's pane once, and classify the capture into an [`AgentState`].
///
/// Exactly the same composition `ws/server.rs`'s sessions-topic poller uses —
/// one `tmux::list_sessions_raw` call, then one `tmux::capture_pane_raw` call
/// per session (the "1 + S" shape the Context Pointers describe) — so this
/// poller does not introduce a second, differently-shaped sweep. A session
/// whose pane capture fails is silently dropped from this tick's result
/// (best-effort, matching the WS hub's own "ignore tmux errors" tick
/// handling) rather than failing the whole tick.
fn capture_states() -> anyhow::Result<Vec<(String, AgentState)>> {
    let raw = tmux::list_sessions_raw()?;
    let sessions = sessions_snapshot(&raw);
    let states = sessions
        .into_iter()
        .filter_map(|s| {
            tmux::capture_pane_raw(&s.name)
                .ok()
                .map(|capture| (s.name, status_detect::detect_state(&capture)))
        })
        .collect();
    Ok(states)
}

// ── Pure decision ─────────────────────────────────────────────────────────────

/// Pure per-tick decision.
///
/// Given whether the poller has ever completed a tick before (`seeded`), the
/// previous tick's state map, and this tick's captured `(name, state)` pairs,
/// return the session names that just crossed into `Blocked` — always empty
/// on the seed tick, which is the entire seed-before-emit fix — plus the
/// state map to carry into the next tick.
///
/// [`sessions_needing_input`] (and the `should_emit_needs_input` predicate it
/// wraps) is called unchanged in meaning; only the "has a prior tick ever
/// happened" gate is layered on top, so the predicate itself stays exactly
/// what Task 1 left it.
///
/// No I/O — directly unit-testable (Rule 6).
pub fn tick_decision(
    seeded: bool,
    prev: &HashMap<String, AgentState>,
    current: &[(String, AgentState)],
) -> (Vec<String>, HashMap<String, AgentState>) {
    let next_prev: HashMap<String, AgentState> = current.iter().cloned().collect();
    let crossing = if seeded {
        sessions_needing_input(prev, current)
    } else {
        Vec::new()
    };
    (crossing, next_prev)
}

// ── I/O shell ─────────────────────────────────────────────────────────────────

/// The capture function's shape, boxed so tests can inject a fake without
/// spawning a real tmux server (`Send` so the box can cross into
/// [`tokio::task::spawn_blocking`]).
type CaptureFn = Box<dyn FnMut() -> anyhow::Result<Vec<(String, AgentState)>> + Send>;

/// The always-on Blocked rising-edge poller.
///
/// Owns its own previous-state map — independent of both the WS hub's
/// subscription-gated sessions poller and `NeedsInputTracker` (Task 1's
/// off-actor home for the WS hub's own use, wired in Task 4) — so it can run
/// from server boot, before any WebSocket ever connects, and keep running
/// with zero subscribers for the rest of the process's life.
pub struct BlockedEdgePoller {
    sink: BlockedEdgeSink,
    host: String,
    prev: HashMap<String, AgentState>,
    seeded: bool,
    capture: CaptureFn,
    /// The WS hub, if any (task 4) — the poller is the sole owner of the
    /// needs-input rising-edge decision; the hub is only a *consumer* of its
    /// output, fanning [`BlockedEdgeCrossed`] out to whichever `sessions`
    /// subscribers are connected right now. `None` in every unit test in
    /// this module (no actix system running there) and whenever the caller
    /// chooses not to wire WS delivery — the durable sink write happens
    /// either way.
    hub: Option<Addr<Hub>>,
}

impl BlockedEdgePoller {
    /// Construct a poller against the real tmux capture sweep.
    pub fn new(sink: BlockedEdgeSink, host: impl Into<String>) -> Self {
        Self::with_capture(sink, host, Box::new(capture_states))
    }

    /// Construct a poller with an injected capture function — the seam tests
    /// use to exercise seed-before-emit, cross-tick emission, and re-block
    /// behavior without a real tmux server (Task 5 uses the same seam).
    pub fn with_capture(
        sink: BlockedEdgeSink,
        host: impl Into<String>,
        capture: CaptureFn,
    ) -> Self {
        Self {
            sink,
            host: host.into(),
            prev: HashMap::new(),
            seeded: false,
            capture,
            hub: None,
        }
    }

    /// Wire this poller to fan `BlockedEdgeCrossed` out to the WS hub in
    /// addition to writing the durable sink record (task 4). Optional: a
    /// poller with no hub still writes every record — WS delivery is a
    /// consumer of the edge, never a precondition for recording it.
    pub fn with_hub(mut self, hub: Addr<Hub>) -> Self {
        self.hub = Some(hub);
        self
    }

    /// Run one poll tick synchronously: capture, apply [`tick_decision`],
    /// append any resulting records to the sink.
    ///
    /// A capture failure (no tmux server yet, transient error) is swallowed
    /// — best-effort, matching `ws/server.rs`'s own "ignore tmux errors" tick
    /// handling — so one bad tick never poisons `seeded` or panics the
    /// poller loop. A sink-write failure is likewise swallowed per-record so
    /// one unwritable record doesn't block the rest of the tick.
    pub fn tick(&mut self) {
        let current = match (self.capture)() {
            Ok(current) => current,
            Err(_) => return,
        };

        let (crossing, next_prev) = tick_decision(self.seeded, &self.prev, &current);
        for name in &crossing {
            let from = self.prev.get(name).copied();
            let record = BlockedEdgeRecord::new(
                name.clone(),
                self.host.clone(),
                from,
                AgentState::Blocked,
                Utc::now(),
            );
            let _ = self.sink.append(&record);
        }
        if let Some(hub) = &self.hub
            && !crossing.is_empty()
        {
            hub.do_send(BlockedEdgeCrossed { sessions: crossing });
        }
        self.prev = next_prev;
        self.seeded = true;
    }

    /// Drive [`Self::tick`] forever on `poll_secs` cadence. Never returns
    /// under normal operation; intended to be spawned once at server boot
    /// (`src/serve/mod.rs::run_server`) via `actix_web::rt::spawn`,
    /// independent of any WebSocket subscription.
    ///
    /// Each tick's capture is offloaded to `tokio::task::spawn_blocking`
    /// (tmux capture is blocking I/O) — mirroring the WS hub's own
    /// `web::block` usage for the identical capture composition.
    pub async fn run(mut self, poll_secs: u64) {
        let mut interval = tokio::time::interval(Duration::from_secs(poll_secs.max(1)));
        loop {
            interval.tick().await;
            self = match tokio::task::spawn_blocking(move || {
                self.tick();
                self
            })
            .await
            {
                Ok(restored) => restored,
                // The blocking task panicked — stop the poller rather than
                // spin on a poisoned closure forever.
                Err(_) => return,
            };
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_sink_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "bastion-blocked-edge-poller-test-{}-{}-{}",
            std::process::id(),
            name,
            uuid::Uuid::new_v4()
        ))
    }

    fn state(name: &str, state: AgentState) -> (String, AgentState) {
        (name.to_string(), state)
    }

    // ── tick_decision (pure) ─────────────────────────────────────────────────

    #[test]
    fn seed_tick_never_emits_even_when_already_blocked() {
        let prev = HashMap::new();
        let current = vec![state("sess-a", AgentState::Blocked)];

        let (crossing, next_prev) = tick_decision(false, &prev, &current);

        assert!(crossing.is_empty());
        assert_eq!(next_prev.get("sess-a"), Some(&AgentState::Blocked));
    }

    #[test]
    fn seed_tick_seeds_multiple_already_blocked_sessions_with_zero_emissions() {
        let prev = HashMap::new();
        let current = vec![
            state("sess-a", AgentState::Blocked),
            state("sess-b", AgentState::Blocked),
            state("sess-c", AgentState::Working),
        ];

        let (crossing, next_prev) = tick_decision(false, &prev, &current);

        assert!(crossing.is_empty());
        assert_eq!(next_prev.len(), 3);
    }

    #[test]
    fn post_seed_tick_emits_on_new_rising_edge() {
        let mut prev = HashMap::new();
        prev.insert("sess-a".to_string(), AgentState::Working);
        let current = vec![state("sess-a", AgentState::Blocked)];

        let (crossing, _next_prev) = tick_decision(true, &prev, &current);

        assert_eq!(crossing, vec!["sess-a".to_string()]);
    }

    #[test]
    fn post_seed_tick_does_not_re_emit_while_still_blocked() {
        let mut prev = HashMap::new();
        prev.insert("sess-a".to_string(), AgentState::Blocked);
        let current = vec![state("sess-a", AgentState::Blocked)];

        let (crossing, _next_prev) = tick_decision(true, &prev, &current);

        assert!(crossing.is_empty());
    }

    #[test]
    fn post_seed_tick_emits_again_after_re_block() {
        let mut prev = HashMap::new();
        prev.insert("sess-a".to_string(), AgentState::Working);
        let current = vec![state("sess-a", AgentState::Blocked)];

        let (crossing, _next_prev) = tick_decision(true, &prev, &current);

        assert_eq!(crossing, vec!["sess-a".to_string()]);
    }

    // ── BlockedEdgePoller::tick (I/O shell, injected capture) ───────────────

    #[test]
    fn tick_with_zero_subscribers_writes_a_record_after_the_seed_tick() {
        let path = temp_sink_path("no-subscriber");
        let sink = BlockedEdgeSink::new(&path);
        let mut ticks: Vec<Vec<(String, AgentState)>> = vec![
            vec![state("sess-a", AgentState::Working)],
            vec![state("sess-a", AgentState::Blocked)],
        ];
        ticks.reverse();
        let capture: CaptureFn = Box::new(move || Ok(ticks.pop().unwrap_or_default()));
        let mut poller = BlockedEdgePoller::with_capture(sink.clone(), "test-host", capture);

        poller.tick(); // seed
        poller.tick(); // rising edge

        let records = sink.read_all().expect("read_all should succeed");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].session, "sess-a");
        assert_eq!(records[0].host, "test-host");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tick_seed_pass_emits_nothing_for_already_blocked_sessions() {
        let path = temp_sink_path("restart-storm");
        let sink = BlockedEdgeSink::new(&path);
        let capture: CaptureFn = Box::new(|| {
            Ok(vec![
                state("sess-a", AgentState::Blocked),
                state("sess-b", AgentState::Blocked),
                state("sess-c", AgentState::Blocked),
            ])
        });
        let mut poller = BlockedEdgePoller::with_capture(sink.clone(), "test-host", capture);

        poller.tick(); // seed — simulates a restart with 3 already-blocked sessions

        let records = sink.read_all().expect("read_all should succeed");
        assert!(records.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tick_re_block_after_resolution_produces_two_records() {
        let path = temp_sink_path("re-block");
        let sink = BlockedEdgeSink::new(&path);
        let mut ticks: Vec<Vec<(String, AgentState)>> = vec![
            vec![state("sess-a", AgentState::Working)], // seed
            vec![state("sess-a", AgentState::Blocked)], // first block
            vec![state("sess-a", AgentState::Working)], // answered
            vec![state("sess-a", AgentState::Blocked)], // second block
        ];
        ticks.reverse();
        let capture: CaptureFn = Box::new(move || Ok(ticks.pop().unwrap_or_default()));
        let mut poller = BlockedEdgePoller::with_capture(sink.clone(), "test-host", capture);

        for _ in 0..4 {
            poller.tick();
        }

        let records = sink.read_all().expect("read_all should succeed");
        assert_eq!(records.len(), 2);
        assert!(records.iter().all(|r| r.session == "sess-a"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tick_swallows_capture_errors_without_marking_seeded() {
        let path = temp_sink_path("capture-error");
        let sink = BlockedEdgeSink::new(&path);
        let capture: CaptureFn = Box::new(|| anyhow::bail!("no tmux server"));
        let mut poller = BlockedEdgePoller::with_capture(sink.clone(), "test-host", capture);

        poller.tick();

        assert!(!poller.seeded);
        assert!(sink.read_all().expect("read_all should succeed").is_empty());
        let _ = std::fs::remove_file(&path);
    }
}
