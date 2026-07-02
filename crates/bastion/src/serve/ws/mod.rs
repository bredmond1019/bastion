//! WebSocket handlers for `bastion serve`.
//!
//! At v0 this exposes only the [`echo`] actor; Block C replaces or extends it
//! with the full session-hub router.

pub mod echo;
pub mod server;
pub mod session;
