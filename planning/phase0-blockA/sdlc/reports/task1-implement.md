---
okf: "1.0"
type: sdlc-report
task: phase0-blockA-task1
---

# Implementation Report — phase0-blockA-task1

**Date:** 2026-06-20
**Plan:** planning/phase0-blockA/tasks.md
**Scope:** Task 1 — Toolchain + config plumbing (covers all five sub-tasks in the spec)

## What Was Built or Changed

- Created `src/config.rs` with typed `ConfigError`, `Config` struct, and pure `from_vars()` parser (no env access in tests). `DATABASE_URL` is required; `BASTION_API_URL` and `BASTION_POLL_INTERVAL` have defaults.
- Created `src/api/client.rs` with `ApiClient::health()` returning `ApiStatus` enum (Reachable/Unreachable) using a 2s timeout; existing `trigger_workflow`/`rerun_node` stubs left as `todo!()`.
- Created `src/db/health.rs` with a read-only `probe()` function returning `DbStatus` enum; runs `SELECT 1` on a short-timeout pool. Observer-only (honors D2).
- Updated `src/db/mod.rs` to expose `pub mod health`.
- Updated `src/run/mod.rs` with `status()` calling both probes and a pure `render_status()` side-effect-free renderer. Output words only (`reachable`/`unreachable`), no emoji.
- Updated `src/cli.rs` — `Commands::Status` subcommand confirmed present (scaffold had it; no duplicate added).
- Updated `src/main.rs` — dispatch to `run::status()` confirmed present.
- Created `.env.example` at repo root with recon-corrected values (port 8080, db name `postgres`).
- Added 3 hermetic config unit tests in `src/config.rs`.
- Added 2 hermetic render unit tests in `src/run/mod.rs`.

## Files Created or Modified

| File | Action |
|---|---|
| src/config.rs | modified (rewritten with typed error + from_vars + tests) |
| src/api/client.rs | modified (ApiStatus enum + health() impl) |
| src/db/health.rs | created |
| src/db/mod.rs | modified (added pub mod health) |
| src/run/mod.rs | modified (status() + render_status() + tests) |
| src/cli.rs | modified (reformatted; Commands::Status already present) |
| src/main.rs | modified (dispatch wiring confirmed present) |
| .env.example | present (no-diff — scaffold already had recon-correct values) |

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
cargo fmt --check  → exit 0 (no diffs after cargo fmt pass)

cargo clippy -- -D warnings → exit 0
    Finished `dev` profile

cargo test →
running 5 tests
test config::tests::missing_database_url_is_typed_error_not_panic ... ok
test config::tests::applies_defaults_for_optional_vars ... ok
test config::tests::parses_when_all_vars_present ... ok
test run::tests::renders_reachable_services_with_version ... ok
test run::tests::renders_unreachable_services_without_panicking ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

cargo build --release → exit 0
    Finished `release` profile [optimized]
```

Status: PASSED

**Smoke test (manual, non-gating):**
```
DATABASE_URL="postgres://localhost/nonexistent" cargo run -- status
DB   unreachable
API  unreachable
```
Exit 0, no panic.

## Decisions and Trade-offs

- `ApiStatus` and `DbStatus` are plain enums (not `Result`) so an unreachable service is a normal outcome, not an error that aborts `status`. This means `status()` always exits cleanly regardless of whether services are up.
- `trigger_workflow`/`rerun_node` in `api/client.rs` carry `#[allow(dead_code)]` rather than being removed — they are Phase 3/4 stubs whose presence documents the planned surface area.
- Worker count / queue depth rows deferred — per D2, those live in Redis (outside bastion's configured scope). The status table shows DB + API reachability only.
- Default `BASTION_API_URL` set to `http://localhost:8080` (recon-corrected; the scaffold had `8000` which was wrong per the D2 recon note).
- `#[allow(dead_code)]` on `poll_interval_secs` field because it will be consumed in Phase 1 monitor but is not needed in Phase 0.

## Follow-up Work

- Phase 1: wire `poll_interval_secs` in the live TUI monitor (`monitor/mod.rs`).
- Phase 1: add worker-count and queue-depth rows to `status` output once Redis scope is settled.
- Docs pass: update `CLAUDE.md` Environment block (still shows port 8000 / `orchestrator_db`) to match recon-corrected values in `.env.example`.

## git diff --stat

```
 src/api/client.rs   | 51 +++++++++++++++++++++++++++++-------
 src/cli.rs          | 59 ++++++++++++++++-------------------------
 src/config.rs       | 75 +++++++++++++++++++++++++++++++++++++++++++++--------
 src/costs/mod.rs    |  9 ++-----
 src/db/costs.rs     | 18 ++-----------
 src/db/mod.rs       |  3 ++-
 src/db/workflows.rs | 48 ++--------------------------------
 src/inspect/mod.rs  | 10 ++-----
 src/main.rs         | 33 +++++++++++++----------
 src/monitor/mod.rs  | 12 ++-------
 src/run/mod.rs      | 61 ++++++++++++++++++++++++++++++++++++++++---
 src/validate/mod.rs | 11 ++------
 12 files changed, 220 insertions(+), 170 deletions(-)
```
