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
//! outcomes it returns; `brain`, for `tests/code_index_contract.rs`'s direct
//! use of `bastion::brain::code_index::*`) are exposed here directly. `brain`
//! itself, though, reaches — directly or transitively through `sessions`'s
//! TUI surface — into most of the crate's other modules
//! (`crate::config::FileConfig`, `crate::sessions::model::Session`,
//! `crate::runs::*`, `crate::db::*`, `crate::ui_theme::*`,
//! `crate::serve::status::detect::detect`, `crate::openwork::*`, and the
//! `term-core`-ported `crate::detect` re-export). Those are therefore all
//! declared here too, purely so the `[lib]` target's module graph resolves
//! the same way `main.rs`'s binary module tree already does. This is a
//! declarations-only mirror of `main.rs`'s `mod` list (the
//! `#[cfg(test)]`-only `testsupport` excluded — nothing under `brain`
//! reaches it), not a behavior change: no logic here, just the same
//! source files compiled a second time under the `bastion::` crate name so
//! `tests/*.rs` can reach them. Grow this list only when a future
//! `tests/*.rs` needs another module's public surface, or when a module
//! gains a new `crate::`-qualified dependency.
#![allow(dead_code)]

pub mod api;
pub mod brain;
pub mod cli;
pub mod config;
pub mod costs;
pub mod db;
pub mod monitor;
pub mod notify;
pub mod observ;
pub mod openwork;
pub mod run;
pub mod runs;
pub mod serve;
pub mod sessions;
pub mod ui_theme;
pub mod validate;

// Detect engine moved to term-core (BA.18.F Phase 0b extraction); re-exported
// here (mirroring main.rs) so every existing `crate::detect::*` path in
// `serve/` and `sessions/` keeps resolving unchanged from this `[lib]`
// target too.
pub use term_core::detect;

// `testsupport` is `#[cfg(test)]`-only in `main.rs` too — several modules'
// unit tests (`crate::testsupport::EnvVarGuard`, `unique_temp_dir`, ...)
// reach it under that same cfg, so it must be declared here identically for
// `cargo test`/`cargo clippy --all-targets` on this `[lib]` target to find
// those symbols.
#[cfg(test)]
pub mod testsupport;
