---
type: TaskSpec
title: "Task Spec — Phase 2, Block B (bastion costs)"
description: LLM spend summary — db::costs aggregation + `bastion costs --last <window>` formatted table.
---

# Task Spec — Phase 2, Block B (`bastion costs`)

## Goal
Implement `db::costs` aggregation queries so `bastion costs --last 7d` prints a formatted table of workflow names, run counts, token totals, and estimated USD cost — handling the `7d`, `30d`, and `all` windows.

## Context Pointers
- **Plan:** `planning/master-plan.md` §"Phase 2 — Block B — `bastion costs`" (lines 119–121); architecture note that all run state is JSON in the `events` table (no relational tables).
- **Already wired:** `src/cli.rs` `Commands::Costs { last }` (default `"7d"`); `src/main.rs:36` calls `costs::run(last).await`. Stubs to fill: `src/costs/mod.rs` (`run` is `todo!()`) and `src/db/costs.rs` (empty).
- **Reuse, don't re-parse:** `src/db/workflows.rs` exposes `pub(crate) fn parse_task_context(&Value) -> Result<Vec<NodeState>>` (line 105) which already pulls per-node `tokens_in` / `tokens_out` / `model` from `node_runs[*].usage`, plus the public `WorkflowRun` / `NodeState` / `RunStatus` types. `started_at` per node is RFC3339 (`"2026-06-20T09:00:09Z"`). The costs query is a new `SELECT … FROM events` over **all** runs (active + completed) feeding the same parse path.
- **Track / invariants:** `costs` is on the gated Postgres observability track — `async`/`tokio` is allowed here (D5's synchronous rule applies only to `sessions/`). Read-only observer of the orchestrator's Postgres (D2; gate lifted). Never writes.
- **CLAUDE.md standing rules:** Rule 1 (tests ship with the block), Rule 6 (Coverage bar — pure logic exhaustively unit-tested without I/O; the thin Postgres shell smoke-tested and recorded in `## Notes`). Validation gate from `planning/harness.json`.
- **Resolved open question (USD price source):** a hardcoded per-model price table lives in a new `src/costs/pricing.rs` (no config file, no env). Seeded with the models present in fixtures plus current Claude models; unknown models estimate `$0.00` and are surfaced as unpriced. This settles the handoff's open choice — no prior decision constrains it.

## Step-by-Step Tasks

### 1. Pricing table + USD estimation (pure)
- Create `src/costs/pricing.rs`. Define a `ModelPrice { input_per_mtok: f64, output_per_mtok: f64 }` and a pure `price_for(model: &str) -> Option<ModelPrice>` backed by a hardcoded table. Seed it with: `claude-3-5-haiku-20241022`, `text-embedding-3-small` (zero output cost), and current Claude models (`claude-opus-4`, `claude-sonnet-4`, `claude-3-5-sonnet`, `claude-3-opus`) — values per million tokens.
- Add a pure `estimate_usd(model: &str, tokens_in: u64, tokens_out: u64) -> f64` = `tokens_in/1e6 * input_per_mtok + tokens_out/1e6 * output_per_mtok`; **unknown model → `0.0`**.
- Register `mod pricing;` in `src/costs/mod.rs` (only the `mod` line — keep the rest of mod.rs for later tasks).
- **Tests (pure, element-by-element):** a known model returns exact USD for known token counts; an embedding model contributes only input cost; an unknown model → `0.0`; zero tokens → `0.0`.
- **Files:** `src/costs/pricing.rs` (new), `src/costs/mod.rs` (mod decl).

### 2. Window parsing + cutoff filter (pure)
- Add `chrono = { version = "0.4", features = ["clock"] }` to `Cargo.toml` (RFC3339 parse + duration math — `started_at` is an ISO string and the window needs `now − N days`).
- In `src/costs/mod.rs` add a `Window` enum (`Days(i64)`, `All`) and `parse_window(s: &str) -> anyhow::Result<Window>` accepting `"7d"`, `"30d"`, `"all"` (case-insensitive); reject anything else with a clear error.
- Add a pure `within_window(window: Window, now: DateTime<Utc>, started_at: Option<&str>) -> bool`: `All` → always `true`; `Days(n)` → `true` iff `started_at` parses and is `>= now − n days`. A missing/unparseable `started_at` is excluded from a `Days` window (included under `All`). Inject `now` as a parameter so the function stays pure; the I/O shell passes `Utc::now()`.
- **Tests:** `7d` / `30d` / `all` parse; garbage string errors; in-window vs out-of-window boundary against a fixed `now`; `None` started_at excluded under `Days`, included under `All`.
- **Files:** `Cargo.toml`, `src/costs/mod.rs`.

### 3. Aggregation + table rendering (pure)
- In `src/costs/mod.rs` define `WorkflowCost { workflow_name, runs: u64, tokens_in: u64, tokens_out: u64, usd: f64 }` and a `CostSummary` (rows + a totals line + a list of any unpriced models seen).
- `aggregate(runs: &[WorkflowRun], window: Window, now: DateTime<Utc>) -> CostSummary`: keep only runs where `within_window(window, now, started_at)`; group by `workflow_name`; per group count runs, sum `tokens_in`/`tokens_out` across nodes (treating `None` as 0), and sum per-node `estimate_usd(model, …)` for nodes that carry a `model`. Record any node whose `model` has no `price_for` entry as unpriced. Sort rows by `usd` descending.
- `render_table(summary: &CostSummary) -> String`: fixed-width columns (Workflow · Runs · Tokens In · Tokens Out · Est. USD), a totals row, and a trailing note listing unpriced models when any exist. Return a `String` (don't print) so it is unit-testable.
- **Tests:** aggregation over a hand-built `Vec<WorkflowRun>` (two workflow types, mixed null/non-null usage) asserts run counts, token sums, and USD per group element-by-element; window filtering drops an out-of-window run; unpriced model is recorded; `render_table` output contains the expected header, a known row, and the totals line.
- **Files:** `src/costs/mod.rs`.

### 4. DB query — `db::costs` (thin I/O shell)
- In `src/db/costs.rs` implement `pub async fn fetch_all_runs(db_url: &str) -> anyhow::Result<Vec<WorkflowRun>>`: open a 1-connection `PgPoolOptions` pool (mirror `db::workflows`), `SELECT id, workflow_type, task_context FROM events`, and assemble each row into a `WorkflowRun` via the existing parse path. To avoid duplicating the row→run assembly, expose `db::workflows`'s `parse_event_row` (and `EventRow` as needed) as `pub(crate)`, or add a small `pub(crate)` assembler there and call it — **do not** re-implement `parse_task_context`. Read-only; never writes (D2).
- Add an `#[ignore]`d integration test stub (guarded by `BASTION_INTEGRATION_TEST`, matching the pattern in `db/workflows.rs`) documenting the call shape; the real verification is the smoke test in Task 5.
- **Files:** `src/db/costs.rs`; minimal visibility-only widening in `src/db/workflows.rs` (declare the widened item names in the task so the dependency analysis sees the overlap).

### 5. Wire `costs::run` + graceful degradation + smoke test
- Implement `costs::run(window: String)` in `src/costs/mod.rs`: `parse_window(&window)` → load config (`DATABASE_URL`) → `db::costs::fetch_all_runs` → `aggregate(&runs, window, Utc::now())` → `print!` the `render_table` output. Degrade gracefully (clear message, non-panic) when `DATABASE_URL` is unset or Postgres is unreachable — mirror the inspect/monitor degrade posture.
- **Tests:** the degrade branches (missing `DATABASE_URL`, unreachable DB) return/print the expected typed outcome without panicking; `parse_window` rejection surfaces a clear error.
- **Smoke test (Rule 6):** with the orchestrator stack up (`./scripts/dev.sh` in `../python-orchestration-system`), run `bastion costs --last 7d`, `--last 30d`, and `--last all`; confirm the table matches a manual SQL aggregation over the same `events` rows. Record the result (or an explicit deferral if the stack can't be brought up this session) in `## Notes`.
- **Files:** `src/costs/mod.rs`.

### 6. Validate
- Run the Validation Commands listed below and confirm all pass.

## Acceptance Criteria
- `bastion costs --last 7d` prints a formatted table with one row per workflow type: run count, total input tokens, total output tokens, and an estimated USD column, plus a totals row.
- All three windows (`7d`, `30d`, `all`) are handled; `parse_window` rejects unknown window strings with a clear error.
- Token totals and USD figures match a manual SQL aggregation over the same `events` data (verified in the Task 5 smoke test, or deferral recorded per Rule 6).
- Pure logic (pricing/USD, window parse + filter, aggregation, table render) is exhaustively unit-tested without I/O; the Postgres query is a thin shell that reuses `parse_task_context` (no duplicated JSON parsing).
- Missing `DATABASE_URL` or an unreachable DB degrades gracefully (no panic).
- All gated checks pass; net test count increases over the 272 baseline.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
<!-- filled in as work happens -->
