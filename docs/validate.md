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
| `frontmatter` | `src/validate/frontmatter.rs` | `validate_frontmatter(content: &str, file: &Path) -> Vec<ValidationError>` | Implemented (Task 2) |
| `links` | `src/validate/links.rs` | `validate_links(content: &str, file: &Path) -> Vec<ValidationError>` | Stub (Task 3) |
| `report` | `src/validate/report.rs` | `render_report(errors: &[ValidationError], files_scanned: usize) -> String` | Stub (Task 4) |

## Exit Behaviour

- Exits **0** when all scanned files pass validation.
- Exits **non-zero** when one or more `ValidationError` values are produced.
- Report is printed to stdout; each line is greppable by `label()`.

## Notes

Smoke-test recording is deferred to Task 5 per spec. The `run()` shell is `async` to match the dispatch site in `main.rs`, but all I/O inside is synchronous `std::fs`.
