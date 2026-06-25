---
type: ImplementationReport
title: Implementation Report — phase3-blockB-task3
description: Link checking implementation for bastion validate (src/validate/links.rs).
---

# Implementation Report — phase3-blockB-task3

**Date:** 2026-06-22
**Plan:** planning/phase3-blockB/tasks.md
**Scope:** Task 3 — Link checking

## What Was Built or Changed

- Implemented `src/validate/links.rs` in full — replacing the Task 1 stub with the complete link-checking module.
- `extract_links(content)` — pure function; parses `[text](target)` inline links with 1-based line numbers; strips optional `"title"` / `'title'` suffix from the target.
- `is_skipped_target(target)` — pure classification; returns `true` for `http://`, `https://`, `mailto:` prefixes and pure `#`-anchors.
- `split_fragment(target)` — pure; splits a target into `(file_portion, Option<fragment>)`.
- `resolve_link_path(target, containing_file)` — pure; resolves the file portion of a relative link against the containing file's directory (fragment discarded).
- `validate_links(content, file)` — thin I/O shell; calls the pure helpers, checks resolved paths with `.exists()`, emits `ValidationError { kind: BrokenLink }` for each missing target.
- 25 unit tests covering `extract_links` (single link, multiple per line, links across lines, title stripping, anchors, external URLs, mailto, empty target), `is_skipped_target` (all schemes + relative paths), `split_fragment` (no fragment, with fragment, pure anchor, subpath), `resolve_link_path` (sibling, subdirectory, parent, fragment stripping), and `validate_links` (valid sibling, broken link, external URL, HTTP, mailto, pure anchor, fragment to existing file, fragment to missing file, mixed content with correct line numbers, link with title).

## Files Created or Modified

| File | Action |
|---|---|
| src/validate/links.rs | modified (stub replaced with full implementation + 25 tests) |
| planning/phase3-blockB/sdlc/reports/task3-implement.md | created |

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

- **No new dependencies** — link parsing is done with a manual byte-walk rather than a regex crate; this satisfies the Cargo.toml/Cargo.lock freeze requirement and keeps all logic testable as pure functions.
- **First `)` closes the link target** — the parser finds the first `)` as the link target boundary, which handles all well-formed markdown links. Pathological cases (e.g. unescaped `)` inside a title string) are out of scope for this validator.
- **Fragment-only links skipped, file+fragment checked at file level** — a target like `guide.md#section` passes the file existence check; whether `#section` actually resolves is not checked (anchor resolution would require parsing the target file, which is beyond the spec's scope).
- **`validate_links` is the only I/O-touching function** — the `.exists()` call is the sole filesystem interaction; all path-computation logic lives in the pure `resolve_link_path`/`split_fragment` helpers and is unit-tested without I/O.

## Notes

**Smoke-test (Rule 6 — thin I/O shell):** The `run` I/O shell in `src/validate/mod.rs` (owned by Task 1) calls `validate_links`. It will be smoke-tested end-to-end in Task 5 once fixtures (Task 4) are committed.  The `validate_links` function itself was exercised via the temp-file tests in `links::tests` (see `validate_links` test group above) — all paths including broken links, external URLs, pure anchors, and fragment-qualified links were asserted against real temp files.

## Follow-up Work

- Task 4 will add committed fixtures (`good.md`, `bad-frontmatter.md`, `broken-links.md`) and the `render_report` implementation; the full integration test against those fixtures will run there.
- Task 5 smoke-test will record end-to-end `cargo run -- validate` output.

## git diff --stat

```
 src/validate/links.rs | 548 +++++++++++++++++++++++++++++++++++++++++++++++++-
 1 file changed, 541 insertions(+), 7 deletions(--)
```
