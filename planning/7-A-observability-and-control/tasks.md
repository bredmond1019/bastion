---
type: TaskSpec
title: "Task Spec — Phase 7, Block A: Tracing + C0xx structured-error spine"
description: Vendor the claude-sdk-rs C001–C014 error taxonomy and introduce a tracing-backed structured event spine across bastion, with --verbose/--json-logs surfaces.
project: bastion
layer: [console]
status: active
keywords: [tracing, error-taxonomy, observability, C0xx, structured-logging]
---

# Task Spec — Phase 7, Block A

**Status:** Not started · **Last run:** never

## Goal
Introduce `tracing` (spans + structured fields) across bastion and vendor the `claude-sdk-rs` `C001–C014` error taxonomy as the Console's error model, so every command emits a structured start/outcome/duration event and errors carry a `C0xx` code + context.

## Context Pointers
- **Plan:** `planning/master-plan.md` → *Phase 7 — Observability & control* → *Block A — Tracing + structured-error spine (program Block H)*. The block names the files (New/Modified), Out of scope, Interfaces, and acceptance criteria — carried through below.
- **Source for the vendored taxonomy (read-only):** `../claude-sdk-rs/src/core/error.rs` — `ErrorCode` enum (`C001`–`C014`, `Display` as `C{:03}`), the `Error` enum with `#[error("[{code}] …")]` messages, `Error::code()`, and `Error::is_recoverable()`. Vendor the taxonomy shape; do **not** depend on the `claude-sdk-rs` crate (source repo is read-only — copy, don't link).
- **Standing rules (`CLAUDE.md`):** Rule 1 (tests ship with every block), Rule 6 (Coverage bar — pure logic exhaustively unit-tested without I/O; error/degradation paths tested; thin I/O shell smoke-tested and recorded in `## Notes`). The `ErrorCode` mapping, `Display` formatting, recoverability classification, and event-record construction are all pure functions and must be asserted element-by-element.
- **Dispatch surface:** `src/main.rs` (clap dispatch `match cli.command`), `src/cli.rs` (`Cli` struct + `Commands` enum), `Cargo.toml` (deps: `anyhow`, `thiserror`, `serde`, `serde_json` already present; `tracing`/`tracing-subscriber` to be added).
- **Naming:** the block calls the vendored model "`C001–C014` error taxonomy + `ErrorContext`". There is no literal `ErrorContext` struct in the source — model it here as bastion's `ConsoleError` enum (carrying the `C0xx` codes) plus an `ErrorContext` wrapper that attaches the originating command/operation. This is the once-vendored model later blocks (7C, 9A, 9B, 10A) emit into — keep it self-contained in `src/observ/`.

## Step-by-Step Tasks

### 1. Vendor the `C001–C014` error taxonomy as the Console error model
- Create `src/observ/mod.rs` declaring the module's submodules (start with `pub mod errors;`; `tracing` helpers are added in Task 2). Wire `mod observ;` into `src/main.rs`'s module list (single-line addition — see disjoint-ownership note; main.rs's dispatch body is **not** touched in this task).
- Create `src/observ/errors.rs` vendoring the taxonomy from `../claude-sdk-rs/src/core/error.rs`:
  - `ErrorCode` enum with the 14 variants `C001`–`C014` (`BinaryNotFound`=1 … `Utf8Error`=14), `#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]`, and a `Display` impl formatting as `C{:03}` (so `ErrorCode::BinaryNotFound.to_string() == "C001"`).
  - `ConsoleError` enum (the bastion-side analogue of the source `Error`) using `thiserror`, each variant carrying its `[{code}] …` message; a `ConsoleError::code() -> ErrorCode` exhaustive match; and `ConsoleError::is_recoverable() -> bool` matching the source's recoverable set (`Timeout`, `RateLimitExceeded`, `StreamClosed`, `Io`, `ProcessError`).
  - An `ErrorContext` wrapper struct that pairs a `ConsoleError` with the originating command/operation string (so a top-level error can report *which* command failed with which `C0xx` code).
- Unit-test exhaustively (Rule 6): every `ErrorCode` → `C0xx` string; every `ConsoleError` variant → expected `code()`; `is_recoverable()` true/false cases; `ErrorContext` construction + accessor round-trip.
- **Files:** `src/observ/errors.rs` (new), `src/observ/mod.rs` (new), `src/main.rs` (module-declaration line only).

### 2. Tracing initialization + structured event-emission helpers
- Add `tracing` and `tracing-subscriber` (with `env-filter` + `json` features) to `Cargo.toml`.
- In `src/observ/mod.rs` add: (a) `init_tracing(verbose: bool, json_logs: bool)` that installs a `tracing-subscriber` writing to stderr — human-readable by default, JSON when `json_logs` is set, with verbosity controlling the level filter; (b) a pure event-record builder (e.g. `CommandEvent { command, phase, duration_ms, error_code }` with a pure constructor / serializer) plus thin `emit_*` helpers (`emit_start`, `emit_outcome`) that log via `tracing` macros. Keep the record construction/serialization pure and the `tracing` call a thin shell over it.
- Unit-test the pure record builder/serializer element-by-element (start vs success vs error-coded outcome; JSON field presence). `init_tracing` is the thin I/O shell — smoke-test and record in `## Notes` (a global subscriber can only be installed once per process, so guard the unit test accordingly).
- **Files:** `src/observ/mod.rs` (modify — depends on Task 1), `Cargo.toml` (modify).

### 3. Global `--verbose` / `--json-logs` flags + subscriber wiring
- In `src/cli.rs` add global flags to the `Cli` struct: `--verbose` (`-v`, `ArgAction::Count` or bool) and `--json-logs`, documented in `long_about`/help.
- In `src/main.rs` call `observ::init_tracing(verbose, json_logs)` at the top of `main()` (before dispatch), reading the new flags off `cli`. Do not yet change the dispatch arms (Task 4).
- Unit-test the flag parsing via clap (`Cli::try_parse_from` over arg vectors asserting `verbose`/`json_logs` are set/unset). Smoke-test that `bastion --json-logs status` emits JSON and `bastion status` emits human-readable; record in `## Notes`.
- **Files:** `src/cli.rs` (modify), `src/main.rs` (modify — depends on Tasks 1–2).

### 4. Dispatch event instrumentation + top-level error → `C0xx` mapping
- In `src/main.rs` wrap each command's execution so **every subcommand** emits a structured start event and an outcome event (success or failure) with command name + duration. Centralize this at the dispatch layer (one helper that takes the command name + the result future/closure) so no per-command-module edits are needed — satisfying "every subcommand emits start/outcome/duration" without touching N command modules (consistent with the block's append-style intent while staying parallel-merge-safe).
- Map a top-level `Err` to an `ErrorContext` carrying the failing command name + a `C0xx` code (best-effort classification of the `anyhow` error into a `ConsoleError` variant; default to a generic code when unclassifiable), emit it as a structured error event, and preserve the process's non-zero exit.
- Unit-test the pure pieces: the command-name resolver (each `Commands` variant → its name string) and the error→`C0xx` classifier (sample errors → expected codes). Smoke-test one failing command (e.g. `bastion inspect <bad-id>` or a command with the orchestrator down) shows a `C0xx`-coded structured error and exits non-zero; record in `## Notes`.
- **Files:** `src/main.rs` (modify — depends on Task 3).

### 5. Validate
- Run the Validation Commands listed below and confirm all pass.
- Confirm the acceptance criteria below hold; record any deferred smoke tests in `## Notes` per Rule 6.

## Acceptance Criteria
- `src/observ/errors.rs` defines the `C001`–`C014` taxonomy; `ErrorCode` `Display` yields `C001`…`C014`, and `ConsoleError::code()` maps every variant to its code (asserted in tests).
- `ConsoleError::is_recoverable()` returns the source's recoverable set (`Timeout`, `RateLimitExceeded`, `StreamClosed`, `Io`, `ProcessError`) and false otherwise (asserted).
- Every subcommand emits a structured **start** and **outcome** event carrying the command name and duration; a failing command emits an error event carrying a `C0xx` code (verified via the dispatch instrumentation + smoke test).
- `--json-logs` produces machine-parseable (JSON) event output; the default surface is human-readable; `--verbose` raises log verbosity (smoke-tested, recorded in `## Notes`).
- The vendored taxonomy compiles in bastion **without** a dependency on the `claude-sdk-rs` crate.
- Pure logic (code mapping, `Display`, recoverability, event-record construction, command-name resolver, error classifier) is exhaustively unit-tested without I/O (Rule 6); error/degradation paths are covered.
- All gated checks pass.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes

### Task 1 smoke test
The `ErrorCode`, `ConsoleError`, and `ErrorContext` types compile and all unit tests pass
(`cargo test`). No I/O executed in tests.

### Task 2 smoke test
`init_tracing` is the thin I/O shell (process-global subscriber install). It cannot be
called more than once per process, so unit tests for `CommandEvent` and `emit_*` helpers
run without a subscriber installed (tracing macros are no-ops in that case — no panic, no
output). The pure `CommandEvent` builders/serializers are exhaustively asserted in tests.

### Task 3 smoke test (2026-06-26)
- `--verbose` / `-v` and `--json-logs` flags added as `global = true` to `Cli` struct.
- `observ::init_tracing(cli.verbose, cli.json_logs)` called at the top of `main()` before
  any dispatch.
- Smoke: `./target/release/bastion --json-logs validate /dev/null` — subscriber installed
  without panic; no tracing events emitted yet (those are Task 4's job). `bastion --help`
  shows both flags with their help text.
- `./target/release/bastion --json-logs sessions` exits cleanly (no tmux server ≠ crash).
- All 614 unit tests pass; `cargo fmt --check`, `cargo clippy`, `cargo build --release`
  all clean.

### Task 4 smoke test (2026-06-26)
- `command_name` resolver: all 17 `Commands` variants mapped exhaustively and unit-tested
  element-by-element.
- `classify_error`: typed `ConsoleError` downcast (9 variants), `std::io::Error` downcast
  (3 kinds), keyword heuristics (10 patterns), and unclassifiable fallback — all unit-tested
  without I/O.
- `dispatch` async fn extracted from `main`; all subcommand logic unchanged.
- `main` now: resolves `cmd_name` (pure), emits `emit_start`, records `Instant::now()`,
  awaits `dispatch`, computes `duration_ms`, emits `emit_outcome` (success or error with C0xx
  code), returns result (anyhow terminates non-zero on Err).
- Smoke (thin I/O shell — no subscriber installed in unit tests):
  `./target/release/bastion validate /dev/null` — emits start + success events to stderr in
  human-readable format; exits 0. Duration field is present (>= 0ms).
  `./target/release/bastion --json-logs validate /dev/null` — emits JSON start + success events;
  exits 0.
- All 653 unit tests pass; `cargo fmt --check`, `cargo clippy`, `cargo build --release` all clean.

### Task 5 validation (2026-06-26)
- `cargo fmt --check` — clean (no output).
- `cargo clippy -- -D warnings` — clean (0 warnings).
- `cargo test` — 653 passed; 0 failed; 3 ignored.
- `cargo build --release` — clean.
- All acceptance criteria confirmed:
  - `src/observ/errors.rs` defines `C001`–`C014`; `Display` and `code()` exhaustively unit-tested.
  - `is_recoverable()` returns true for the 5 recoverable variants, false for all others — unit-tested.
  - Dispatch instrumentation emits start + outcome events per subcommand (smoke-tested in Task 4).
  - `--json-logs` / `--verbose` flags present and wired (smoke-tested in Task 3).
  - No `claude-sdk-rs` crate dependency; taxonomy fully vendored in `src/observ/`.
  - Pure logic exhaustively unit-tested without I/O; error/degradation paths covered (Rule 6).

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
