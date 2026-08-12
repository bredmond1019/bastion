//! Durable, cross-process sink for the Blocked rising edge (BA.18.A).
//!
//! `src/serve/poll.rs`'s [`crate::serve::poll::should_emit_needs_input`] computes
//! *when* a session's state crosses into [`crate::detect::AgentState::Blocked`],
//! but until this module existed that edge was observable only by a live
//! WebSocket subscriber, in-process, for the lifetime of one `bastion serve`
//! run. This module gives the edge somewhere durable to land:
//!
//! - [`sink::BlockedEdgeRecord`] — one edge event: session, host/instance
//!   identity, the `(from, to)` state transition, and the observation
//!   timestamp.
//! - [`sink::BlockedEdgeSink`] — an append-only JSONL file sink. The console
//!   process (`:8080`) is the only writer; the engine process (`:8090`) reads
//!   the same path with no WebSocket and no direct RPC between the two.
//!
//! Task 2 defines the record shape and the sink itself. [`poller`] (Task 3)
//! wires an always-on poller that calls it — spawned once at server boot
//! (`src/serve/mod.rs::run_server`), independent of any WebSocket
//! subscription, with a seed-before-emit first tick so a restart with
//! already-blocked sessions produces zero records for them. Making the WS
//! hub a consumer of the same computation is Task 4.

pub mod poller;
pub mod sink;

pub use poller::BlockedEdgePoller;
// `BlockedEdgeRecord`/`SinkError` have no in-crate caller yet — they are the
// cross-process contract's public shape for the not-yet-built `engine-rs:EN.8.B`
// consumer (and Task 5's tests). Keep them exported rather than pruning them
// just because nothing here calls them.
#[allow(unused_imports)]
pub use sink::{BlockedEdgeRecord, BlockedEdgeSink, SinkError, default_sink_path};
