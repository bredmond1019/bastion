//! `okf-core` — the single-source OKF frontmatter contract.
//!
//! This crate owns the OKF (Open Knowledge Format) frontmatter model, its YAML
//! serializer, and its parser. Consumers (e.g. `bastion`) depend on this crate
//! rather than maintaining their own copies of the model/serializer/parser, so
//! the frontmatter contract has exactly one source of truth across the
//! workspace.
