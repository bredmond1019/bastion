---
type: Handoff
created: 2026-06-22
---

# Handoff — phase2-blockB done; next is phase3-blockA (`bastion run`)

> **For the next agent:** Read this immediately after `/prime`. Delete this file once consumed.

## What we're doing and why
Phase 2 is **complete** — `bastion costs` (Block B) shipped and passed review this session, closing
out the post-mortem/spend half of the observability track. The next sequenced work is
**phase3-blockA — `bastion run`** (`master-plan.md:127–129`): implement
`api::client::trigger_workflow` and `run::trigger(workflow, args, monitor)` so
`bastion run <workflow> [--args '{}'] [--monitor]` issues a POST to the orchestrator's generic
dispatcher, prints the returned `task_id`, and (with `--monitor`) drops into `bastion monitor` for
that run. The CLI is already wired: `Commands::Run { workflow, args, monitor }` (`src/cli.rs:42`) and
`main.rs:37` already call `run::trigger(workflow, args, monitor)`. Both
`run::trigger` (`src/run/mod.rs:10`) and `ApiClient::trigger_workflow` (`src/api/client.rs:68`) are
`todo!()` stubs awaiting this block. This is the first **write** path to the orchestrator (POST) —
but it's a trigger, not a DB write, so the D2 read-only-Postgres observer posture is unchanged.

## Completed this session
- **Shipped phase2-blockB (`bastion costs`)** via `/sdlc-run phase2-blockB` → **PASS in 1 review
  attempt**, 302 tests (net **+30** over the 272 baseline). Commits `b71418d` (spec), `b83124d`
  (impl), `7aed418` (docs), `1a282a5` (wrap-up).
  - `src/costs/pricing.rs` (new): hardcoded per-model price table + pure `price_for` / `estimate_usd`
    (unknown model → `$0.00`, surfaced as unpriced). **Resolved the prior open question** — pricing
    lives in a hardcoded table, not config/env; settled inline in the spec (no formal decision record
    needed).
  - `src/costs/mod.rs`: `chrono`-backed `Window` + `parse_window` (`7d`/`30d`/`all`), pure
    `within_window` cutoff filter (injected `now`), `aggregate`, `render_table`, and the wired
    `costs::run` with graceful degradation (missing `DATABASE_URL` / unreachable DB).
  - `src/db/costs.rs`: `fetch_all_runs` thin Postgres shell reusing `db::workflows::parse_event_row`
    (widened to `pub(crate)`) — no duplicated JSON parsing. `chrono` added to `Cargo.toml`.
  - Docs: created `docs/costs.md` (operator reference); added its row to `docs/index.md`; patched
    `docs/data-contract.md` to document the costs DB-only read path.
- **Authored + committed the phase2-blockB spec** earlier this session via `/generate-tasks`.

## Remaining work
- **Next block: phase3-blockA (`bastion run`).** Start with `/generate-tasks phase3-blockA`, then run
  the SDLC pipeline. Scope: fill `ApiClient::trigger_workflow` (reqwest POST, parse `task_id` from the
  202 body) + `run::trigger` (parse optional `--args` JSON, call the client, print `task_id`, and on
  `--monitor` hand off to `monitor::run`). Keep the Rule 6 split — JSON-arg parsing, request-body
  construction, and the response/`task_id` extraction are **pure** and unit-tested element-by-element;
  the reqwest call is the thin I/O shell, smoke-tested and recorded in `## Notes`.
- **Deferred smoke tests (need the orchestrator stack up — `./scripts/dev.sh` in
  `../python-orchestration-system`):** (1) `bastion costs --last 7d/30d/all` vs manual SQL
  (phase2-blockB; an `#[ignore]` stub `db::costs::tests::integration_fetch_all_runs_returns_vec` is in
  place — run `BASTION_INTEGRATION_TEST=1 cargo test -- --ignored`); (2) `bastion inspect <run-id>`
  live render/nav/exit (phase2-blockA); (3) `bastion monitor` live render/poll-cycle (phase1-blockB).
  One bring-up clears all three — and `bastion run` will want the same stack to verify a real trigger
  end-to-end, so fold all four into that session.

## Open questions / choices
- **Trigger endpoint shape — verify against the data contract before coding.** There is a
  discrepancy in the stubs: `master-plan.md:128` and `api/client.rs:73` say the generic dispatcher is
  `POST /` with body `{workflow_type, data}` → `202 {task_id, message}` (data contract §7), but the
  `run::trigger` stub comment (`src/run/mod.rs:11`) says `POST /workflows/{name}/run`. Confirm the
  real route against `docs/data-contract.md` §7 (and the live orchestrator's FastAPI routes if the
  stack is up) at `/generate-tasks` time and make the stubs consistent. Treat the **data contract** as
  authoritative.
- Everything else is settled — clear to proceed once the endpoint is confirmed.

## Context the next agent needs
- **`run` is on the observability track** — `async`/`tokio` is allowed (D5's synchronous rule applies
  only to `sessions/`). `bastion run` is a *trigger* (POST), the first non-read call to the
  orchestrator; it does not touch Postgres, so the D2 read-only-DB posture is untouched.
- **Reuse the existing client + monitor:** `ApiClient::new(base_url)` already exists with a working
  `health()` / `workflow_graph()`; follow their reqwest + 2s-timeout + `.error_for_status()` pattern
  for `trigger_workflow`. `--monitor` should hand off to `monitor::run(workflow_id)` (`main.rs:33`).
  `Config::load()` supplies `api_base_url` (see `run::status` at `src/run/mod.rs:14` for the pattern).
- **Validation gate** (`planning/harness.json`): `cargo fmt --check`, `cargo clippy -- -D warnings`,
  `cargo test`, `cargo build --release`. Test baseline is now **302** (2 ignored = pre-existing +
  new DB integration stubs, not a regression).
- **Working tree is clean** — all phase2-blockB work is committed (`1a282a5` is HEAD).

## First command after `/prime`
`/generate-tasks phase3-blockA`
