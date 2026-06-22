---
type: ImplementReport
title: Implementation Report — phase3-blockB-task2
description: Frontmatter validation (OKF fields) implementation report for bastion validate.
---

# Implementation Report — phase3-blockB-task2

**Date:** 2026-06-22
**Plan:** planning/phase3-blockB/tasks.md
**Scope:** Task 2

## What Was Built or Changed

- Replaced the stub body of `src/validate/frontmatter.rs` with a full implementation of OKF frontmatter validation.
- Added internal `Frontmatter` struct capturing parsed `key -> (value, 1-based line)` map plus the block's open/close line span.
- Added internal `ParseResult` enum covering all structural outcomes: `Ok(Frontmatter)`, `UnterminatedFence`, `MalformedLine`, `NoFrontmatter`.
- Implemented pure `extract_frontmatter(content: &str) -> ParseResult` — line-based parser with no external YAML dependency; first colon splits key from value; tracks 1-based line numbers throughout.
- Implemented pure `validate_frontmatter(content, file) -> Vec<ValidationError>` — dispatches on `ParseResult` and emits the typed `ErrorKind` variants (`MissingFrontmatter`, `MalformedFrontmatter`, `MissingField`, `EmptyField`) at the correct source lines.
- Added 24 exhaustive unit tests directly in the module covering: valid full frontmatter, each parse-level structural error, each missing field individually, all fields missing, each empty/whitespace field, line-number assertions, and file-path preservation.

## Files Created or Modified

| File | Action |
|---|---|
| src/validate/frontmatter.rs | modified (stub replaced with full implementation) |

## Validation Output

**Commands run:**
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

**Result:** PASSED

## Decisions and Trade-offs

- `extract_frontmatter` is module-private (`fn`, not `pub`) because its return type `ParseResult` is an internal enum. The public contract is only `validate_frontmatter`. Tests inside the module can still exercise `extract_frontmatter` directly.
- A blank line inside the frontmatter block (no colon) is treated as `MalformedLine`, consistent with strict OKF expectations (flat key-value only, no multi-line or empty separators).
- When a required field is absent, the error line is set to `close_line` (the closing `---`), which places it within the frontmatter span — a useful anchor for editors.
- No `serde_yaml` or any YAML library added. The line-based parser covers the flat scalar subset that OKF uses, keeping `Cargo.toml`/`Cargo.lock` untouched per the spec constraint.

## Follow-up Work

- Task 3 fills in `src/validate/links.rs` (relative link existence check).
- Task 4 fills in `src/validate/report.rs` and adds fixtures + integration tests.

## git diff --stat

```
 src/validate/frontmatter.rs | 419 +++++++++++++++++++++++++++++++++++++++++++-
 1 file changed, 413 insertions(+), 6 deletions(-)
```
