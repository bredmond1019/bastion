//! Agent-state detection adapter for the `bastion serve` WebSocket hub.
//!
//! Wraps the Block C₀ `detect` engine (in `crate::detect`) with the embedded
//! Claude manifest, exposing the two surface-level functions used by the hub:
//!
//! - [`detect::needs_input`] — boolean "permission prompt visible" signal.
//! - [`detect::detect_state`] — raw [`AgentState`] for debounce logic.

pub mod detect;
pub mod repo;
