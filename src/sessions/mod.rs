// sessions/ — tmux session control surface.
// Decision D4: this surface is DB-free. It shells out to tmux via
// std::process::Command and never opens a Postgres pool or calls Config::load().

pub mod agent_panel;
pub mod app;
pub mod ask;
pub mod ask_question;
pub mod commands;

// Ported to `term-core` (BA.18.F) — re-exported here so every existing
// `crate::sessions::{tmux, model, claude_state}` path keeps resolving
// unchanged. The attach path (`attach_session` / `suspend_and_attach`)
// lives in the separate `term-attach` crate (bastion CLI only — never
// reachable from the engine/server side; see that crate's docs) and is
// folded into this `tmux` shim alongside term-core's own tmux module.
pub use term_core::{claude_state, model};

pub mod tmux {
    pub use term_attach::{attach_session, suspend_and_attach};
    pub use term_core::tmux::*;
}
pub mod ui;

#[cfg(test)]
mod tui_tests;

pub use commands::run;
