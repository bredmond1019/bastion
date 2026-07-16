//! Library surface for bastion (BA.7.C task 4).
//!
//! bastion is otherwise a binary-only crate (`src/main.rs` owns the module
//! tree via `mod api; mod observ; ...`). `tests/abort_contract.rs` needs to
//! call `api::client::ApiClient::abort_run` against a real in-process
//! `engine-serve` App, and a plain `tests/*.rs` integration test can only
//! reach a crate's items through a `[lib]` target — a binary crate exposes
//! nothing to `tests/`. This lib target recompiles the same source files
//! as a second, separate crate (`bastion::`) so integration tests can
//! `use bastion::api::client::ApiClient;`; `main.rs` is unaffected and keeps
//! declaring its own module tree for the binary.
//!
//! Kept deliberately minimal: only the modules an integration test needs
//! today (`api`, for the abort client; `observ`, for the `ConsoleError`
//! outcomes it returns) are exposed here. Grow this list only when a future
//! `tests/*.rs` needs another module's public surface.

pub mod api;
pub mod observ;
