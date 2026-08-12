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
//! This module only defines the record shape and the sink itself. Wiring an
//! always-on poller that calls it (seed-before-emit, restart-storm
//! suppression) is Task 3; making the WS hub a consumer of the same
//! computation is Task 4.

pub mod sink;

// Wired into an always-on poller in Task 3; unused until then.
#[allow(unused_imports)]
pub use sink::{BlockedEdgeRecord, BlockedEdgeSink, SinkError, default_sink_path};
