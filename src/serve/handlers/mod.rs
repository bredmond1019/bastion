//! Route handler submodules for `bastion serve`.
//!
//! Each submodule owns the handler functions for one API surface.
//! Handlers are registered in `src/serve/mod.rs` inside the protected
//! `/api` scope so they inherit `BearerAuthMiddleware`.

pub mod sessions;
pub mod status;
