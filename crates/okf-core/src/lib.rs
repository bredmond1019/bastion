//! `okf-core` — the single-source OKF frontmatter contract.
//!
//! This crate owns the OKF (Open Knowledge Format) frontmatter model, its YAML
//! serializer, and its parser. Consumers (e.g. `bastion`) depend on this crate
//! rather than maintaining their own copies of the model/serializer/parser, so
//! the frontmatter contract has exactly one source of truth across the
//! workspace.

mod frontmatter;
mod parse;
mod state;

pub use frontmatter::{OkfFrontmatter, serialize_frontmatter};
pub use parse::{Frontmatter, ParseResult, extract_frontmatter, parse_frontmatter};
pub use state::{
    Backlog, Block, BlockedBy, Carryover, CarryoverScope, CrossRepoEdge, Endpoint, Focus, Origin,
    RepoRollup, StateEdge, StateEdgeKind, StateFile, StateGraph, StateLoadError, StateNode,
    StateSource, TierEntry, Track, TrackBlock, build_state_graph, load_state,
};
