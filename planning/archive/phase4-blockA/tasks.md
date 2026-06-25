---
type: TaskSpec
title: Phase 4 Block A — Config File + Help/Man Polish
description: Add ~/.config/bastion/config.toml support and improve bastion help with a generated man page.
---

# Task Spec — Phase 4, Block A

## Goal
Ship the two unblocked Phase 4 polish items: `~/.config/bastion/config.toml` support so the DB URL
isn't always an env var, and `bastion help` improvements plus a generated man page.

## Context Pointers

- **Plan:** `planning/master-plan.md` → Phase 4 (Polish). This block covers exactly two of the four
  listed items — **config-file support** and **`bastion help` improvements / man page**. The other
  two (SSE streaming, TUI node re-run) are **out of scope**: both are blocked on orchestrator
  capability that is still deferred (orchestrator D28 Phases 4–5 not shipped), confirmed 2026-06-22.
- **Repo files:**
  - `src/config.rs` — current env-only loader; pure `from_vars` parser + `load()`. The config-file
    work extends this (Task 1).
  - `src/cli.rs` — clap `Cli`/`Commands` definitions; help text lives here (Task 2, Task 3 add the
    `Man` variant).
  - `src/main.rs` — clap dispatch (Task 3 adds the `man` arm).
  - `Cargo.toml` — adds `toml` (Task 1) and `clap_mangen` (Task 3).
- **CLAUDE.md standing rules:**
  - **Rule 1 / Rule 6 (Coverage bar):** separate pure logic from I/O; unit-test parsing, merge
    precedence, path resolution, and man-page rendering exhaustively; smoke-test the thin I/O shells
    and record results in `## Notes`.
  - **Rule 2 (OKF frontmatter)** on every new markdown file; **Rule 7** — update the directory's
    `index.md` when adding a doc.
- **Parallel-merge safety:** Task 1 owns `src/config.rs`; Task 2 owns `src/cli.rs` help text. Both
  touch `Cargo.toml` and the shared docs (`README.md`, `docs/index.md`) only via **append-only**
  edits — declare those as additive so the block engine union-merges them. Task 3 **dependsOn Task 1
  and Task 2** because it edits `Cargo.toml`, `src/cli.rs`, and `src/main.rs` after them — it runs in
  a later wave, never in parallel with 1 or 2.

## Step-by-Step Tasks

### 1. Config file support (`~/.config/bastion/config.toml`)
- **Primary files:** `src/config.rs`, `Cargo.toml` (append `toml` to `[dependencies]`),
  `.env.example` (append a comment pointing at the optional config file). New doc `docs/config.md`;
  append a row to `docs/index.md` and a config section to `README.md` (both append-only).
- Add a `toml` dependency (lightweight, deserialize-only).
- Add a `FileConfig` struct (`#[derive(serde::Deserialize, Default)]`) with **optional** fields
  mirroring the env vars: `database_url`, `api_base_url`, `poll_interval` (all `Option<…>`). Unknown
  keys ignored.
- Add a **pure** `parse_file(contents: &str) -> Result<FileConfig, ConfigError>` — parses TOML,
  mapping a TOML error to a new typed `ConfigError::MalformedFile(String)` variant. Empty string →
  `FileConfig::default()`.
- Add a **pure** `config_path(xdg_config_home: Option<String>, home: Option<String>) -> Option<PathBuf>`
  resolving `$XDG_CONFIG_HOME/bastion/config.toml`, falling back to `$HOME/.config/bastion/config.toml`,
  `None` when neither is set. **No new crate** for the home dir — read the two env values.
- Add a **pure** merge with explicit precedence **env var > config file > built-in default**: extend
  or wrap `from_vars` so it takes the file values as the fallback layer (e.g.
  `from_sources(env: EnvVars, file: FileConfig)`). `DATABASE_URL` still required, but may now be
  satisfied by the file; only error `MissingVar` when absent from **both** env and file.
- Rewire `load()` to: `dotenvy::dotenv().ok()` → resolve `config_path` → read file if present
  (absent/unreadable file degrades to `FileConfig::default()`, never an error) → merge with env.
- **Tests (pure, exhaustive):** precedence — env wins over file, file fills a gap the env omits,
  built-in default applies when both omit (`api_base_url` → 8080, `poll_interval` → 2); `database_url`
  satisfied by file alone; missing from both → `MissingVar`; malformed TOML → `MalformedFile`; empty
  file → defaults; `config_path` for each of (xdg set), (only home set), (neither). Keep existing
  `from_vars` tests green.
- **Smoke test (I/O shell):** write a real `config.toml`, run a command that calls `load()` with the
  env var unset, confirm it reads the file; record in `## Notes` per Rule 6.

### 2. `bastion help` enrichment
- **Primary file:** `src/cli.rs` (help strings only — do **not** add or change subcommand variants
  here; Task 3 owns the `Man` variant). Append-only: a help/examples section in `README.md` and a row
  in `docs/index.md` if a new doc is added.
- Expand the top-level `#[command(...)]`: add a `long_about` describing the two surfaces (workflow
  observability vs. session control) and a `version`. Add `after_help` / `after_long_help` with a
  concrete usage-examples block (e.g. `bastion sessions`, `bastion monitor`, `bastion costs --last 7d`,
  `bastion validate ./docs`).
- Tighten per-subcommand doc comments where terse, so `bastion <cmd> --help` reads well. Group related
  verbs visually if clap supports it cleanly (optional).
- **Tests:** `Cli::command().debug_assert()` (clap's built-in config validator) in a unit test; assert
  the rendered long help contains the examples block and both surface names
  (`Cli::command().render_long_help().to_string()`). Keep all existing `cli.rs` parse tests green.

### 3. `bastion man` — generated man page
- **dependsOn: Task 1, Task 2.** **Primary files:** new `src/man.rs`; append `clap_mangen` to
  `Cargo.toml` `[dependencies]`; add a `Man` variant to `src/cli.rs` `Commands`; add the dispatch arm
  in `src/main.rs`. New doc note in `docs/index.md` (append-only) + a man section in `README.md`.
- Add `clap_mangen` dependency.
- Add a hidden-ish `Man { /// optional output dir #[arg(long)] out: Option<PathBuf> }` subcommand:
  `bastion man` prints the roff man page to stdout; `--out <dir>` writes `bastion.1` (and one page per
  subcommand) into the directory.
- **Pure core:** `render_man() -> std::io::Result<Vec<u8>>` builds `clap_mangen::Man::new(Cli::command())`
  and renders to a buffer — no filesystem. Keep the directory-write path a thin I/O shell over it.
- **Tests (pure):** `render_man()` output (as UTF-8) contains `.TH` / the `BASTION` title and the
  command name; non-empty; deterministic across two calls. Add a parse test that `bastion man` and
  `bastion man --out /tmp/x` parse.
- **Smoke test (I/O shell):** `bastion man | head`, and `bastion man --out <tmpdir>` then `man -l
  <tmpdir>/bastion.1` renders; record in `## Notes` per Rule 6.

### 4. Validate
- Run the Validation Commands below and confirm all pass.
- Confirm the full test count moved up from the 404 baseline (no regressions; pre-existing 3 ignored
  remain ignored).
- Confirm `bastion --help`, `bastion help`, and `bastion man` all produce output, and that
  `cargo run -- status` still works with the env var unset but a `config.toml` present.

## Acceptance Criteria
- `~/.config/bastion/config.toml` (and `$XDG_CONFIG_HOME` variant) supplies `database_url` /
  `api_base_url` / `poll_interval` when the corresponding env var is unset; an env var **always wins**
  over the file; built-in defaults apply when both are absent. A missing or unreadable file degrades
  silently (never an error); malformed TOML produces a typed `ConfigError::MalformedFile`.
- Pure config logic (parse, merge precedence, path resolution) is unit-tested element-by-element,
  including every error/degradation branch.
- `bastion --help` / `bastion <cmd> --help` show enriched descriptions and a usage-examples block;
  `Cli::command().debug_assert()` passes.
- `bastion man` prints a valid roff man page; `bastion man --out <dir>` writes `bastion.1`. Rendering
  is unit-tested without touching the filesystem.
- New docs (`docs/config.md`, README sections) carry OKF frontmatter where applicable and `docs/index.md`
  is updated.
- All four gated checks pass; no clippy warnings; release build succeeds.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
<!-- filled in as work happens -->
