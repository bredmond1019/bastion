---
type: Reference
title: validate — Markdown/MDX Content Validation
description: "Reference for `bastion validate <path>`: file discovery, shared error types, and subcommand wiring."
doc_id: validate
layer: [console]
project: bastion
status: active
keywords: [validation, frontmatter, OKF, markdown, link checking, ValidationError]
related: [brain]
---

# validate — Markdown/MDX Content Validation

`bastion validate <path>` recursively discovers `.md` and `.mdx` files under `<path>` (or validates a single file directly), runs frontmatter and link checks, prints a greppable report, and exits non-zero if any errors are found.

## Usage

```
bastion validate <PATH>
```

`PATH` may be a directory (recursive) or a single `.md`/`.mdx` file.

## File Discovery

`find_markdown_files(root: &Path) -> Vec<PathBuf>` (in `src/validate/mod.rs`):

- Accepts a directory or a single file path.
- Recursively collects files with `.md` or `.mdx` extensions.
- Skips hidden directories and files (names starting with `.`).
- Skips the `target/` build-artifact directory.
- Returns a deterministically sorted (lexicographic) list.

## Shared Error Types

Defined in `src/validate/mod.rs` and used by all submodules.

### `ValidationError`

```rust
pub struct ValidationError {
    pub file: PathBuf,   // absolute or relative path to the file
    pub line: usize,     // 1-based line number of the offending content
    pub kind: ErrorKind, // typed error category
    pub message: String, // human-readable description
}
```

### `ErrorKind`

| Variant | `label()` | Description |
|---|---|---|
| `MissingFrontmatter` | `missing-frontmatter` | File has no YAML frontmatter block at all |
| `MalformedFrontmatter` | `malformed-frontmatter` | YAML present but cannot be parsed |
| `MissingField` | `missing-field` | A required OKF field (`type`, `title`, `description`) is absent |
| `EmptyField` | `empty-field` | A required OKF field is present but blank |
| `BrokenLink` | `broken-link` | A relative link target does not exist on disk |

`label()` returns a stable lowercase string suitable for grep-based filtering of report output.

## Submodule Contracts

| Module | File | Public function | Status |
|---|---|---|---|
| `frontmatter` | `src/validate/frontmatter.rs` | `validate_frontmatter(content: &str, file: &Path) -> Vec<ValidationError>` | Implemented (Task 2) |
| `links` | `src/validate/links.rs` | `validate_links(content: &str, file: &Path) -> Vec<ValidationError>` | Implemented (Task 3) |
| `report` | `src/validate/report.rs` | `render_report(errors: &[ValidationError], files_scanned: usize) -> String` | Implemented (Task 4) |

## Report Rendering (`src/validate/report.rs`)

Implemented in Task 4. Produces a greppable, human-readable report string from a slice of `ValidationError` values.

### Public API

| Function | Signature | Description |
|---|---|---|
| `render_report` | `fn render_report(errors: &[ValidationError], files_scanned: usize) -> String` | Formats all errors as greppable lines followed by a summary line. |

### Output Format

Each error is rendered as:

```
<file>:<line>: <kind-label>: <message>
```

- Errors are grouped and sorted by file path (lexicographic), then by line number within each file.
- The final line of the output is a summary: `N error(s) across M file(s)` when errors exist, or `no issues found across M file(s)` when clean.
- `<kind-label>` is the stable string returned by `ErrorKind::label()` (e.g. `broken-link`, `missing-field`).

### Example

```
docs/guide.md:3: missing-field: required field 'description' is missing
docs/guide.md:7: broken-link: relative link target 'nonexistent.md' does not exist
src/validate/fixtures/bad-frontmatter.md:4: empty-field: required field 'description' is empty
2 error(s) across 2 file(s)
```

## Test Fixtures (`src/validate/fixtures/`)

Three fixture files are used by the integration tests in `src/validate/report.rs`:

| Fixture | Purpose |
|---|---|
| `good.md` | Valid OKF frontmatter and a working relative link; yields zero errors. |
| `bad-frontmatter.md` | Valid `type`/`title` but an empty `description` value; yields one `EmptyField` error. |
| `broken-links.md` | Valid frontmatter, one valid relative link, one external URL, one pure anchor, and one broken relative link (`nonexistent-file.md`); yields exactly one `BrokenLink` error and no frontmatter errors. |

External URLs and pure `#`-anchors in `broken-links.md` confirm that `is_skipped_target` is respected end-to-end.

## Link Checking (`src/validate/links.rs`)

Implemented in Task 3. All link-logic lives in pure functions; only `validate_links` touches the filesystem.

### Public API

| Function | Signature | Description |
|---|---|---|
| `extract_links` | `fn extract_links(content: &str) -> Vec<(String, usize)>` | Parses all `[text](target)` inline links in `content`; returns `(target, line_number)` pairs (1-based). Strips optional `"title"` / `'title'` suffixes from the target. |
| `is_skipped_target` | `fn is_skipped_target(target: &str) -> bool` | Returns `true` for `http://`, `https://`, `mailto:` prefixes and pure `#`-anchors. All other targets (relative paths) return `false` and are checked. |
| `split_fragment` | `fn split_fragment(target: &str) -> (&str, Option<&str>)` | Splits a link target into `(file_portion, fragment)`. Returns `("", Some(fragment))` for pure anchors. |
| `resolve_link_path` | `fn resolve_link_path(target: &str, containing_file: &Path) -> PathBuf` | Resolves the file portion of a relative link against the containing file's parent directory. Fragment is discarded before resolution. |
| `validate_links` | `fn validate_links(content: &str, file: &Path) -> Vec<ValidationError>` | Thin I/O shell: calls the pure helpers and calls `.exists()` on each resolved relative path; emits `ValidationError { kind: BrokenLink }` for each missing target. |

### Behaviour Notes

- External URLs (`http://`, `https://`), `mailto:` links, and pure `#`-anchors are never flagged — they are skipped by `is_skipped_target`.
- Fragment-qualified relative links (e.g. `guide.md#section`) check only the file portion; anchor resolution within the target file is out of scope.
- Line numbers in `BrokenLink` errors are 1-based and match the source line of the `[text](target)` syntax.

## Exit Behaviour

- Exits **0** when all scanned files pass validation.
- Exits **non-zero** when one or more `ValidationError` values are produced.
- Report is printed to stdout; each line is greppable by `label()`.

## Notes

The `run()` shell is `async` to match the dispatch site in `main.rs`, but all I/O inside is synchronous `std::fs`.

### Smoke-Test Results (Task 5)

**Dirty path — fixture directory (expect exit 1):**
```
$ cargo run -- validate src/validate/fixtures
src/validate/fixtures/bad-frontmatter.md:4: empty-field: required field `description` is present but empty
src/validate/fixtures/broken-links.md:14: broken-link: broken link: nonexistent-file.md
2 error(s) across 2 file(s)
Error: 2 error(s) found
EXIT CODE: 1
```

**Clean path — single good file (expect exit 0):**
```
$ cargo run -- validate src/validate/fixtures/good.md
no issues found across 1 file(s)
EXIT CODE: 0
```

Both paths exercised the full `run()` I/O shell end-to-end; exit codes match spec.
