---
type: Reference
title: validate — Markdown/MDX Content Validation
description: Reference for `bastion validate <path>`: file discovery, shared error types, and subcommand wiring.
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
| `frontmatter` | `src/validate/frontmatter.rs` | `validate_frontmatter(content: &str, file: &Path) -> Vec<ValidationError>` | Stub (Task 2) |
| `links` | `src/validate/links.rs` | `validate_links(content: &str, file: &Path) -> Vec<ValidationError>` | Implemented (Task 3) |
| `report` | `src/validate/report.rs` | `render_report(errors: &[ValidationError], files_scanned: usize) -> String` | Stub (Task 4) |

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

Smoke-test recording is deferred to Task 5 per spec. The `run()` shell is `async` to match the dispatch site in `main.rs`, but all I/O inside is synchronous `std::fs`.
