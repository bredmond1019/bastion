---
type: TaskSpec
title: Task Spec — Phase 3, Block B (bastion validate)
description: Markdown/MDX content validator — walk a path, validate OKF frontmatter, check links, report errors with file + line.
---

# Task Spec — Phase 3, Block B (`bastion validate`)

## Goal
Port `markdown-engine-validator` logic into `bastion validate`: scan a content directory, validate frontmatter, check links, and report errors with file + line.

## Context Pointers
- **Plan:** `planning/master-plan.md` → Phase 3 Block B (`bastion validate`). What: "Scan a content directory, validate frontmatter, check links, report errors with file + line." Acceptance: "Detects known-bad frontmatter and broken links in test fixtures."
- **Stub to fill:** `src/validate/mod.rs` (currently `todo!()`), already wired in `src/cli.rs:29` (`Validate { path }`, defaults to `.`) and `src/main.rs:35`. **Do not edit `cli.rs` or `main.rs`** — keep `run`'s signature `pub async fn run(path: PathBuf) -> Result<()>` so the existing dispatch stays valid; the body does synchronous `std::fs` work (no `.await`).
- **Architecture (handoff + D5):** `validate` is **not** on the observability track. It reads the filesystem, not Postgres/the API. Synchronous `std::fs`, no tokio coupling inside the module — same camp as `sessions/`.
- **OKF field set:** Required frontmatter fields are `type`, `title`, `description` (CLAUDE.md Rule 6 / project OKF convention). No locale-parity or content-layout rule applies here.
- **CLAUDE.md Rule 6 (coverage bar):** Separate pure logic from I/O. All parsing/extraction/classification/formatting live in pure functions asserted directly against fixtures; the thin `run` I/O shell is manually smoke-tested and recorded in `## Notes`.
- **No new dependency:** OKF frontmatter is a flat block of `key: value` scalars — parse it with a minimal line-based parser rather than adding `serde_yaml` (keeps `Cargo.toml`/`Cargo.lock` untouched and the logic exhaustively unit-testable). Record this choice in `## Notes`.

## Step-by-Step Tasks

### 1. Module skeleton, shared types, and file discovery
- **Owns:** `src/validate/mod.rs`; creates stub files `src/validate/frontmatter.rs`, `src/validate/links.rs`, `src/validate/report.rs`.
- In `mod.rs`: declare `mod frontmatter; mod links; mod report;` and define the shared types:
  - `ValidationError { file: PathBuf, line: usize, kind: ErrorKind, message: String }`.
  - `ErrorKind` enum covering the cases the later tasks emit: `MissingFrontmatter`, `MalformedFrontmatter`, `MissingField`, `EmptyField`, `BrokenLink`. Give it a stable lowercase `&str` label method (e.g. `missing-field`) for greppable output.
- Implement the pure file-discovery walker: `find_markdown_files(root: &Path) -> Vec<PathBuf>` — recursive, collects `.md` and `.mdx` files, skips hidden dirs/files (leading `.`) and `target/`, returns a deterministically sorted list. Handle a single-file path argument (return just that file if it is `.md`/`.mdx`).
- Write `run` as the synchronous I/O shell: resolve the path, call `find_markdown_files`, read each file, call `frontmatter::validate_frontmatter` + `links::validate_links`, collect all `ValidationError`s, print `report::render_report(...)`, and return `Err` (or set a non-zero outcome via `anyhow::bail!`) when any errors were found so the command signals failure.
- In the three stub files, define the function signatures the shell calls, returning empty results so the crate compiles:
  - `frontmatter.rs`: `pub fn validate_frontmatter(content: &str, file: &Path) -> Vec<ValidationError>`
  - `links.rs`: `pub fn validate_links(content: &str, file: &Path) -> Vec<ValidationError>`
  - `report.rs`: `pub fn render_report(errors: &[ValidationError], files_scanned: usize) -> String`
- **Tests:** exhaustive unit tests for `find_markdown_files` (recursion, extension filtering, hidden/`target` skip, single-file arg, deterministic ordering) using a `tempfile`-style temp tree or a committed fixture dir. `ErrorKind` label mapping asserted per variant.

### 2. Frontmatter validation (OKF fields)
- **Owns:** `src/validate/frontmatter.rs`. **dependsOn: 1.**
- Pure `extract_frontmatter(content: &str) -> Option<Frontmatter>` where `Frontmatter` captures the parsed `key -> (value, line)` map plus the block's line span. A frontmatter block is the leading `---` fence, lines until the closing `---`. Track 1-based line numbers for each key.
- Pure `validate_frontmatter(content, file)`:
  - No leading frontmatter block → one `MissingFrontmatter` error at line 1.
  - Malformed block (opening `---` with no closing fence, or a non-`key: value` line inside) → `MalformedFrontmatter` at the offending line.
  - Each required field (`type`, `title`, `description`) absent → `MissingField` (message names the field); present but empty/whitespace value → `EmptyField`.
  - Line numbers point at the real source line (the field's line for present-but-empty; line 1 / fence line for structural problems).
- **Tests:** exhaustive — valid full frontmatter (no errors), each missing field individually, all missing, empty value per field, no frontmatter, unterminated fence, malformed inner line. Assert error `kind`, `line`, and field name in message element-by-element.

### 3. Link checking
- **Owns:** `src/validate/links.rs`. **dependsOn: 1.**
- Pure `extract_links(content: &str) -> Vec<(usize, String)>` — find inline markdown links `[text](target)` with 1-based line numbers. Ignore image-only vs link distinction is fine; capture the target inside the parens (strip any `"title"` suffix and surrounding whitespace).
- Pure classification helper(s): external (`http://`, `https://`, `mailto:`) and pure in-page anchors (`#...`) are **skipped** (not checked); a target with a `#fragment` is checked against the file portion only.
- Pure-ish `validate_links(content, file)`: for each relative file link, resolve against the containing file's directory and emit a `BrokenLink` error (with the link's line + the unresolved target in the message) when the target does not exist. Resolution touches the filesystem — keep the path-resolution logic (split fragment, join relative) in a pure function tested directly; the existence check is the thin I/O part.
- **Tests:** exhaustive — `extract_links` against multiple links per line, links across lines, no links, links with titles, anchors, external URLs. Classification asserted per scheme. `validate_links` against a fixture file with a mix of valid relative links (a sibling that exists) and broken ones; assert only the broken targets are reported with correct line + message.

### 4. Report rendering, fixtures, and integration
- **Owns:** `src/validate/report.rs`, `src/validate/fixtures/**`. **dependsOn: 1, 2, 3.**
- Pure `render_report(errors, files_scanned) -> String`: a greppable per-error line `<file>:<line>: <kind-label>: <message>`, errors grouped/sorted by file then line, followed by a summary line (`N error(s) across M file(s)` / a clean "no issues" line when empty). No ANSI required.
- Add committed fixtures under `src/validate/fixtures/`:
  - `good.md` — valid OKF frontmatter + a valid relative link to a sibling fixture.
  - `bad-frontmatter.md` — missing a required field and/or empty value (known-bad).
  - `broken-links.md` — valid frontmatter but a broken relative link, plus one valid link and one external URL (to prove those are not flagged).
- Integration test(s): run `validate_frontmatter` + `validate_links` over the fixtures and assert the exact error set per file (satisfies the plan's acceptance: "Detects known-bad frontmatter and broken links in test fixtures"). Assert `render_report` output shape for a representative error set.
- **Tests:** `render_report` — empty set (clean summary), single error, multi-file sorted ordering, label/format of each line. Fixture-driven integration assertions as above.

### 5. Validate
- Run the Validation Commands listed below and confirm all pass.
- Manually smoke-test the `run` I/O shell: `cargo run -- validate src/validate/fixtures` (expect non-zero exit + reported bad-frontmatter/broken-link errors) and `cargo run -- validate <a-clean-dir>` (expect clean summary, exit 0). Record the result in `## Notes` per CLAUDE.md Rule 6.

## Acceptance Criteria
- `bastion validate <path>` recursively discovers `.md`/`.mdx` files (skipping hidden dirs and `target/`) and accepts both a directory and a single-file path.
- Missing/empty required OKF fields (`type`, `title`, `description`) and malformed/absent frontmatter are reported with the correct file + line and a typed `ErrorKind`.
- Broken relative links are reported with file + line; external URLs and pure anchors are not flagged.
- The fixtures prove the acceptance: `good.md` yields no errors; `bad-frontmatter.md` and `broken-links.md` yield exactly the expected errors.
- The command prints a greppable report and exits non-zero when any errors are found, zero when clean.
- All pure functions (`find_markdown_files`, `extract_frontmatter`, `validate_frontmatter`, `extract_links`, link classification/resolution, `validate_links`, `render_report`) are exhaustively unit-tested including error/degradation paths; the `run` shell is smoke-tested and recorded in `## Notes`.
- `src/cli.rs` and `src/main.rs` are unchanged; no new crate dependency added (`Cargo.toml`/`Cargo.lock` untouched).
- All gated validation checks pass.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
<filled in as work happens>
