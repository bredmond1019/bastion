---
type: Handoff
created: 2026-06-22
---

# Handoff — phase2-blockA done; next is phase2-blockB (`bastion costs`)

> **For the next agent:** Read this immediately after `/prime`. Delete this file once consumed.

## What we're doing and why
Phase 2 Block A (`bastion inspect` — the static post-mortem graph view) **shipped and passed
review** this session. The monitor/observability track is fully unblocked (orchestrator D28 landed;
bastion D2 gate lifted). The next sequenced work is **phase2-blockB — `bastion costs`**
(`master-plan.md:119–121`): implement `db::costs` aggregation queries and a `bastion costs --last 7d`
command that prints a formatted table of workflow names, run counts, token totals, and estimated USD
cost, supporting `7d` / `30d` / `all` windows. The data is already in reach — `db::workflows`'s
`parse_task_context` extracts per-node `tokens_in` / `tokens_out` / `model` from
`events.task_context.node_runs[*].usage`; `costs` aggregates those across runs. The CLI is already
wired: `Commands::Costs { last }` (`src/cli.rs:36`) and `main.rs:36` already call `costs::run(last)`;
`src/costs/mod.rs` and `src/db/costs.rs` are `todo!()` stubs awaiting this block.

## Completed this session
- **Shipped phase2-blockA (`bastion inspect`)** via `/sdlc-run phase2-blockA --from implement` →
  **PASS in 2 review attempts**, 272 tests (net +7 over the 265 baseline).
  - `src/monitor/events.rs`: widened `setup_terminal` / `restore_terminal` / `handle_key` to
    `pub(crate)` (visibility-only, no behavior change) so inspect reuses them.
  - `src/inspect/mod.rs`: replaced the `todo!()` with `build_inspect_app` (pure seam, 9 unit cases),
    `run_static_loop` (one-shot draw + blocking `crossterm::event::read()`, **no poll / no
    `tokio::select!`**), and a graceful-degrade async `run` (handles missing `DATABASE_URL`, unknown
    run id, unreachable graph API). Commits `ae89be6` (impl), `6883cec` (fix pass 2 — moved the
    deferred smoke-test record into `tasks.md § Notes` per Rule 6).
  - Docs: created `docs/inspect.md` (operator reference), cross-linked from `docs/monitor.md`
    (`392bc27`); wrap-up `3661f2a`.
- **Closed the `docs/index.md` NEEDS_REVIEW follow-up** (`f09cf1f`): added the `inspect.md` row to
  the navigation table, placed right after the `monitor.md` row.

## Remaining work
- **Next block: phase2-blockB (`bastion costs`).** Start with `/generate-tasks phase2-blockB`, then
  run the SDLC pipeline. Scope: `db::costs` aggregation (sum `node_runs[*].usage` tokens, group by
  `workflow_type`, count runs) + `costs::run(last)` window parsing (`7d`/`30d`/`all`) + a formatted
  stdout table with an estimated USD column. Acceptance (master-plan.md:121): output matches manual
  SQL against the same data; all three windows handled. Keep the Rule 6 split — window parsing,
  aggregation math, and table formatting are **pure** and unit-tested element-by-element; the
  Postgres query is the thin I/O shell.
- **Deferred smoke tests (need the orchestrator stack up — `./scripts/dev.sh` in
  `../python-orchestration-system`):** (1) `bastion inspect <run-id>` live render/nav/exit
  (phase2-blockA `tasks.md § Notes`); (2) `bastion monitor` live render/poll-cycle (phase1-blockB
  `tasks.md § Notes`). One bring-up clears both — and `costs` will want the same stack up to verify
  its output against manual SQL, so fold all three into that session.

## Open questions / choices
- **USD cost estimation source.** `bastion costs` needs a per-model price table to turn token totals
  into an estimated USD column. Decide where that lives (a hardcoded `HashMap` in `costs`, a small
  config file, or env) and which models/prices to seed — pick this at `/generate-tasks` time. No
  prior decision constrains it. Everything else is settled; clear to proceed.

## Context the next agent needs
- **`costs` is on the gated Postgres (observability) track** — async/tokio is allowed (D5's
  synchronous-no-tokio rule applies only to `sessions/`). It's a read-only observer of the
  orchestrator's Postgres (D2; gate lifted).
- **Token data is already parsed** — reuse, don't re-parse: `db::workflows::parse_task_context`
  (`src/db/workflows.rs:105`) already pulls `tokens_in` / `tokens_out` / `model` per node. The costs
  query can either call into that path or run its own aggregation SQL over the same `events` table —
  decide at task-gen time, but don't duplicate the JSON-parsing logic.
- **Validation gate** (`planning/harness.json`): `cargo fmt --check`, `cargo clippy -- -D warnings`,
  `cargo test`, `cargo build --release`. Test baseline is now **272** (2 ignored = pre-existing DB
  integration tests, not a regression).
- **Working tree is clean** — all phase2-blockA work is committed (`f09cf1f` is HEAD).

## First command after `/prime`
`/generate-tasks phase2-blockB`
