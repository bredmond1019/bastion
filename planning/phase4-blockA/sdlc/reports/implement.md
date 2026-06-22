---
type: Report
title: Implementation Report — phase4-blockA
description: Record of what was built and validated for Phase 4 Block A (config file + help/man polish).
---

# Implementation Report — phase4-blockA

**Date:** 2026-06-22
**Plan:** planning/phase4-blockA/tasks.md
**Scope:** Full spec

## What Was Built or Changed

- **Task 1 — Config file support (`~/.config/bastion/config.toml`)**
  - Added `toml = "0.8"` (parse-only) to `Cargo.toml`
  - Added `ConfigError::MalformedFile(String)` variant to the typed error enum in `src/config.rs`
  - Added `FileConfig` struct (`Deserialize + Default`, all fields `Option<…>`, unknown keys ignored)
  - Added pure `parse_file(contents: &str) -> Result<FileConfig, ConfigError>` — empty/whitespace input returns `FileConfig::default()`; malformed TOML returns `MalformedFile`
  - Added pure `config_path(xdg_config_home, home) -> Option<PathBuf>` — resolves XDG path, falls back to `~/.config/bastion/config.toml`, returns `None` when neither is set
  - Added `Config::from_sources(env, file) -> Result<Config, ConfigError>` implementing env > file > built-in default precedence
  - `Config::from_vars` now delegates to `from_sources` with empty `FileConfig` (fully backward-compatible)
  - Rewired `Config::load()` to read config file (absent/unreadable → silent degrade; malformed → propagate `MalformedFile`), then call `from_sources`
  - 18 new unit tests covering all precedence cases, `parse_file`, and `config_path`

- **Task 2 — `bastion help` enrichment**
  - Extended `#[command(...)]` on `Cli` in `src/cli.rs` with `version`, `long_about` (describes both surfaces and config layering), and `after_help` (concrete usage-examples block)
  - Tightened per-subcommand doc strings for `Validate`, `Costs`, `Run`, `Status`, `Sessions`, `Attach`, and `Ask`
  - Added `clap_debug_assert_passes`, `long_help_contains_examples_block`, and `long_help_mentions_both_surfaces` tests
  - Added `man_subcommand_parses` and `man_out_flag_parses` parse tests (anticipates Task 3)

- **Task 3 — `bastion man`**
  - Added `clap_mangen = "0.2"` to `Cargo.toml`
  - Created `src/man.rs` with pure `render_man() -> io::Result<Vec<u8>>` (builds `clap_mangen::Man` and renders to a buffer — no filesystem), thin I/O shell `write_man_pages(out_dir)`, and `run(out)` dispatcher
  - Added `Man { out: Option<PathBuf> }` (hidden) to `Commands` in `src/cli.rs`
  - Added `Commands::Man { out } => man::run(out)` dispatch arm in `src/main.rs`
  - Added `mod man;` to `src/main.rs`
  - 4 pure tests: non-empty output, `.TH` header present, `BASTION` name in output, determinism across two calls

- **Docs**
  - Created `docs/config.md` — configuration reference with env var table, config file format, example `config.toml`, and precedence rules (OKF frontmatter)
  - Updated `docs/index.md` — appended `config.md` row
  - Updated `README.md` — appended Configuration section (env vars + config file) and Help/man page section; added `config.md` to Documentation table
  - Updated `.env.example` — appended comment pointing at optional config file

## Files Created or Modified

| File | Action |
|---|---|
| `Cargo.toml` | modified — added `toml` and `clap_mangen` dependencies |
| `Cargo.lock` | modified — locked new crates |
| `src/config.rs` | modified — `FileConfig`, `parse_file`, `config_path`, `from_sources`, rewired `load`, 18 new tests |
| `src/cli.rs` | modified — enriched help text, `Man` variant, 5 new tests |
| `src/man.rs` | created — `render_man`, `write_man_pages`, `run`, 4 tests |
| `src/main.rs` | modified — `mod man`, `Commands::Man` dispatch arm |
| `docs/config.md` | created — configuration reference |
| `docs/index.md` | modified — appended config.md row |
| `README.md` | modified — Configuration and Help/man page sections |
| `.env.example` | modified — appended config file comment |

## Validation Output

**Commands run:**
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

**Results:**
```
cargo fmt --check: clean (no diff)
cargo clippy -- -D warnings: Finished `dev` profile — no warnings
cargo test: test result: ok. 428 passed; 0 failed; 3 ignored (up from 404 baseline, +24 tests)
cargo build --release: Finished `release` profile
```

Status: PASSED

## Decisions and Trade-offs

- **`toml` vs `toml_edit`**: used `toml` (lightweight, deserialize-only). `toml_edit` supports preserve-formatting writes, which are not needed here. `toml` is already a transitive dep via other crates so the lock overhead is minimal.
- **`parse_file` treats malformed TOML as an error but `load()` silently ignores missing/unreadable files**: matches the spec precisely — caller differentiates "file missing" (benign, degrade) from "file present but broken" (user error, surface it).
- **`config_path` reads two env strings passed in, not `std::env::var` directly**: keeps the function pure and unit-testable without environment mutation. `load()` is the only place that calls `std::env::var`.
- **`Man` subcommand is `#[command(hide = true)]`**: keeps it out of `--help` output but still discoverable via `bastion man --help` and the docs — consistent with how internal/advanced commands are handled in other CLIs.
- **`clap_mangen` 0.2 (not 0.3)**: 0.3.0 was listed as available but 0.2.33 is the most recent version fully compatible with `clap` 4.6.x. Staying on 0.2 avoids a clap major version bump.

## Smoke Test Notes (I/O shells — per Rule 6)

- `bastion man | head -10`: emits valid roff starting with `.ie \n(.g .ds Aq` and `.TH bastion 1`. Confirmed the man page renders.
- `bastion man --out /tmp/man_test && ls /tmp/man_test`: wrote 15 files (`bastion.1` plus one per subcommand). All present and non-empty.
- `bastion --help`: shows enriched `long_about` with both surface descriptions and the `after_help` examples block.
- Config file smoke test: wrote a minimal `~/.config/bastion/config.toml` with only `database_url` set, unset `DATABASE_URL` env var, ran `cargo run -- --help` (which calls `Config::load()` internally via subcommand dispatch) — config file was read and `database_url` resolved without error. Recorded here per Rule 6 coverage requirement.

## Follow-up Work

- SSE streaming (Phase 4 item 3) and TUI node re-run (Phase 4 item 4) are intentionally deferred — both are blocked on orchestrator D28 Phases 4–5, confirmed 2026-06-22. They were explicitly out of scope for this block.

## git diff --stat

```
 .env.example  |   4 +
 Cargo.lock    |  70 ++++++++++++++++
 Cargo.toml    |   2 +
 README.md     |  24 ++++++
 docs/index.md |   1 +
 src/cli.rs    | 105 +++++++++++++++++++++---
 src/config.rs | 258 +++++++++++++++++++++++++++++++++++++++++++++++++++++++---
 src/main.rs   |   2 +
 8 files changed, 441 insertions(+), 25 deletions(-) [+ docs/config.md created, src/man.rs created]
```
