# Task Spec — Phase 0, Block A

## Goal
Verify the Rust toolchain compiles the scaffold and implement `bastion status` end-to-end — probe PostgreSQL and the FastAPI health endpoint, print a reachability summary, and ship a `.env.example`.

## Context Pointers
- `planning/master-plan.md` → **Phase 0 → Block A — Foundation setup** (the authoritative scope: `config.rs` reads `DATABASE_URL`/`BASTION_API_URL`; `run::status()` calls `api::client::ApiClient::health()` + a test PostgreSQL query; prints a formatted table `DB ✓ / API ✓ / worker count / queue depth`, or `unreachable` per service).
- `planning/decisions/D2-observability-consumer-contract.md` — bastion is a **read-only** observer; Phase 0 `status` is explicitly unblocked (only Phase 1 `monitor` is gated on the orchestrator).
- Repo files in scope: `src/config.rs`, `src/db/` (connection probe), `src/api/client.rs`, `src/run/` (`status()`), `src/cli.rs` (the `status` subcommand), `src/main.rs` (dispatch).
- `CLAUDE.md` — bastion standing rules (OKF frontmatter on docs, atomic decisions, no emoji in source/docs — note the master-plan's `✓`/table glyphs are illustrative, not literal-required output).
- `planning/harness.json` — the gated checks every task must leave green: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`.

> **Harness honesty / live-infra note.** The master-plan acceptance line "prints real health data against the running Python orchestrator" requires live PostgreSQL + FastAPI and is a **manual acceptance step, out of scope for the gated checks**. The gating checks (`cargo …`) must pass **offline**. Therefore the unreachable-service path is the unit-tested behavior: with no DB/API reachable, `bastion status` must print `unreachable` per service and exit cleanly (never panic). Live ✓/✓ output is confirmed manually when the stack is up.

## Step-by-Step Tasks

### 1. Toolchain + config plumbing
- Confirm `cargo build` and `cargo test` run on the scaffold before changes (record the baseline in Notes).
- Implement `config.rs`: read `DATABASE_URL` and `BASTION_API_URL` from the environment into a `Config` struct, with a typed error (or `Option`) when a var is missing — do not `panic!`/`unwrap` on missing env.
- Add a `.env.example` at the repo root documenting both vars with placeholder values and a one-line comment each.

### 2. Service health probes
- `api/client.rs`: implement `ApiClient::health()` using `reqwest` against `BASTION_API_URL` — return a typed result (reachable + parsed health body, or an unreachable/error variant). Use a short timeout so an absent service fails fast rather than hanging.
- `db/` (e.g. `db/workflows.rs` or a small `db/health.rs`): implement a read-only connection probe — open the pool and run a trivial test query (`SELECT 1`, plus worker-count / queue-depth reads if the schema is available), returning reachable-with-stats or an unreachable/error variant. Read-only only (honor D2).

### 3. `bastion status` command + output
- `cli.rs`: define the `status` subcommand (no required args).
- `run/`: implement `status()` to call both probes and render a formatted summary table to stdout — one row per service showing reachable state and (when reachable) worker count / queue depth, or `unreachable` when not. The function must return cleanly for every combination of reachable/unreachable services.
- `main.rs`: wire `status` into clap dispatch.

### 4. Unit tests (offline-passing)
- Config: parsing succeeds with both vars set; returns the typed missing-var error (no panic) when a var is absent.
- Status rendering: given constructed/mocked health results (reachable-with-stats, and unreachable), the table renderer produces the expected strings — including the `unreachable` path — without needing a live service.
- Keep all tests hermetic (no real network/DB) so `cargo test` passes offline.

### 5. Validate
- Run the Validation Commands below and confirm all pass.

## Acceptance Criteria
- `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, and `cargo build --release` all pass.
- `config.rs` reads `DATABASE_URL` + `BASTION_API_URL` from env and surfaces a missing var as a typed error (no panic).
- `.env.example` exists at the repo root documenting both vars.
- `bastion status` runs against unreachable services and prints `unreachable` per service, exiting cleanly (no panic) — covered by a unit test.
- Health-probe and status-render logic are unit-tested with hermetic tests.
- (Manual, non-gating) With the Python orchestrator + DB live, `bastion status` prints real reachable health data.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
<filled in as work happens>
