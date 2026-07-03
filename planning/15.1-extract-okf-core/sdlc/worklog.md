# Worklog — 15.1-extract-okf-core

## Task 1 — PASSED (1 attempt)
What: Scaffolded the new okf-core workspace crate (empty lib.rs, serde dep) and wired it into the root workspace members and as a path dependency of crates/bastion, confirmed by a clean cargo build.
Validated: gating checks (fast tripwire)

## Task 2 — PASSED (1 attempt)
What: Moved the OKF frontmatter parser (Frontmatter, ParseResult, extract_frontmatter, parse_frontmatter) into okf-core as pub items with their tests, repointed brain/okf.rs to call okf_core::parse_frontmatter directly, and made bastion's validate/frontmatter.rs re-export the parser from okf-core while keeping validate_frontmatter and its tests unchanged.
Decisions: Added #[allow(unused_imports)] on the pub use re-export in validate/frontmatter.rs since Frontmatter is not named directly in that file (only used via type inference), avoiding a clippy -D warnings failure while still satisfying the spec's required re-export surface.
Validated: gating checks (fast tripwire)
