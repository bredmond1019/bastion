//! Route handler submodules for `bastion serve`.
//!
//! Each submodule owns the handler functions for one API surface.
//! Handlers are registered in `src/serve/mod.rs` inside the protected
//! `/api` scope so they inherit `BearerAuthMiddleware`.

pub mod actions;
pub mod attention;
pub mod block_graph;
pub mod board;
pub mod concurrency;
pub mod costs;
pub mod docs;
pub mod epics;
pub mod lanes;
pub mod notify;
pub mod pipeline;
pub mod runs;
pub mod sessions;
pub mod status;
