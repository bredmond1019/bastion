// sessions/ — tmux session control surface.
// Decision D4: this surface is DB-free. It shells out to tmux via
// std::process::Command and never opens a Postgres pool or calls Config::load().

pub mod app;
pub mod commands;
pub mod model;
pub mod tmux;
pub mod ui;

pub use commands::run;
