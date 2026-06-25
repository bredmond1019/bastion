---
type: TaskSpec
title: Phase 3 Block A — bastion run
description: Trigger a workflow via the FastAPI generic dispatcher and optionally drop into the live monitor.
---

# Task Spec — Phase 3, Block A

## Goal
Implement `bastion run <workflow> [--args '{}'] [--monitor]` so it issues `POST /` with `{workflow_type, data}` to the orchestrator's generic dispatcher, prints the returned `task_id`, and optionally drops into `bastion monitor` for that run.

## Context Pointers
- **Plan:** `planning/master-plan.md` §"Phase 3 — Run + Validate" → Block A (`master-plan.md:127–129`).
- **Authoritative contract:** `docs/data-contract.md` §"Trigger → `api::client::trigger_workflow`" (line 85): `POST /` with `{ "workflow_type": str, "data": object }` → `202 { "task_id": str, "message": str }`. **Triple-confirmed against the live orchestrator** (`../python-orchestration-system/app/api/endpoint.py:30`, `app/api/models.py:6–13`): unknown `workflow_type` → `422` with a valid-types list; invalid `data` → `422` with Pydantic errors. There is **no** `/workflows/{name}/run` route — only `POST /` and `GET /workflows/{type}/graph`.
- **Stubs to fill:**
  - `src/api/client.rs:68` — `ApiClient::trigger_workflow(&self, workflow_type: &str, data: Option<serde_json::Value>) -> Result<String>` (signature already correct; comment already matches the contract).
  - `src/run/mod.rs:10` — `run::trigger(workflow: String, args: Option<String>, monitor: bool) -> Result<()>`.
- **Already wired (do not touch):** `Commands::Run { workflow, args, monitor }` (`src/cli.rs:42`); dispatch in `src/main.rs:37–41`.
- **Reuse:** follow the existing `ApiClient::workflow_graph()` reqwest pattern (`src/api/client.rs:50`) — 2s timeout, `.error_for_status()`, `.context(...)`. `Config::load()` supplies `api_base_url` (see `run::status`, `src/run/mod.rs:14`). On `--monitor`, hand off to `monitor::run(Some(task_id))` (`src/monitor/mod.rs:23`, takes `Option<String>`).
- **Posture:** `run` is on the observability track — `async`/`tokio` is allowed (D5's synchronous rule applies only to `sessions/`). This POST is a *trigger*, not a DB write; the D2 read-only-Postgres observer posture is unchanged.
- **Standing rules:** `CLAUDE.md` Rule 1 (tests ship with the block) and Rule 6 (separate pure logic from I/O; test pure logic exhaustively element-by-element including error/degrade paths; smoke-test the thin I/O shell and record it in `## Notes`).

## Step-by-Step Tasks

### 1. Implement `trigger_workflow` in `src/api/client.rs`
- **Owns:** `src/api/client.rs`.
- Add a private request-body type (`#[derive(Serialize)] struct TriggerRequest { workflow_type: String, data: serde_json::Value }`) and a private response type (`#[derive(Deserialize)] struct TaskAccepted { task_id: String, message: String }` — `message` parsed but unused for return).
- Decide the `data` default in a **pure helper** (e.g. `fn trigger_body(workflow_type, data: Option<Value>) -> TriggerRequest`) so a `None` argument serializes as `"data": {}` (empty object), matching the orchestrator's `data: dict` field. Unit-test the serialized JSON shape element-by-element for both `Some(obj)` and `None`.
- Fill `trigger_workflow`: POST to `format!("{}/", base_url.trim_end_matches('/'))` with `.json(&body)`, 2s timeout, `.error_for_status()`, decode `TaskAccepted`, return `task_id`. Mirror `workflow_graph`'s `.context(...)` messages so a `422`/unknown-workflow surfaces a clear error.
- Tests: assert the pure `trigger_body` output (JSON serialization) for `Some`/`None`; assert the trigger URL is `base + "/"` with trailing slashes normalized (reuse the `health_url` trailing-slash test pattern).

### 2. Implement `run::trigger` in `src/run/mod.rs`
- **Owns:** `src/run/mod.rs`. `dependsOn`: Task 1 (calls `trigger_workflow`; runs in a later wave).
- Add a **pure** `fn parse_args(args: Option<String>) -> Result<Option<serde_json::Value>>` that returns `Ok(None)` for `None`, parses the string as JSON otherwise, and returns a clear typed error on malformed JSON (and, if non-object, a message — the orchestrator expects an object). Unit-test: `None` → `None`; valid `'{"k":1}'` → parsed object; malformed `'{'` → `Err` with a useful message; non-object `'5'` → handled per chosen rule.
- Fill `trigger`: `Config::load()` → `ApiClient::new(&config.api_base_url)` → `parse_args(args)?` → `client.trigger_workflow(&workflow, data).await`. On success print the `task_id` (stable, greppable line, e.g. `task_id: <id>`). If `monitor` is true, hand off to `monitor::run(Some(task_id)).await` after printing.
- Graceful degradation: an unreachable API or a `422` returns a clear `Err`/message (non-panicking), consistent with how `run::status` treats unreachable services. Add a pure render/format helper if it keeps the success/error output testable without I/O.
- Tests: `parse_args` cases above; any output-formatting helper asserted directly.

### 3. Validate
- Run the Validation Commands below and confirm all pass.
- Record in `## Notes`: the deferred live smoke test (needs the orchestrator stack — `./scripts/dev.sh` in `../python-orchestration-system`): trigger a real workflow, confirm the printed `task_id` matches the orchestrator's `202` body, confirm `--monitor` drops into the live graph for that run, and confirm a bad `workflow_type` surfaces the `422` cleanly. Per the handoff, fold this in with the three other deferred smoke tests (costs / inspect / monitor) on the same bring-up.

## Acceptance Criteria
- `bastion run <workflow>` issues `POST /` with `{ "workflow_type": <workflow>, "data": {...} }` and prints the returned `task_id`.
- `--args '{"k":1}'` is forwarded as the `data` object; omitting `--args` sends `data: {}`. Malformed `--args` JSON fails fast with a clear error and no panic.
- `--monitor` drops into `bastion monitor` filtered to the triggered run after printing the `task_id`.
- An unreachable orchestrator or a `422` (unknown workflow / invalid data) produces a clear error message, not a panic.
- Pure logic (`trigger_body`, `parse_args`, any output formatter) is unit-tested element-by-element, including the malformed-args and default-`data` paths. All gated checks pass; the test baseline grows from 302.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes

**Deferred smoke test (Rule 6 — thin I/O shell):** Needs the orchestrator stack running
(`./scripts/dev.sh` in `../python-orchestration-system`). Fold into the same session as the
other deferred smoke tests (costs / inspect / monitor):

1. `bastion run <real_workflow_type>` — confirm `task_id: <uuid>` printed and matches the
   orchestrator's `202` response body.
2. `bastion run <workflow> --args '{"k":1}'` — confirm `data` forwarded correctly (check
   orchestrator logs or task output).
3. `bastion run <workflow> --monitor` — confirm drops into `bastion monitor` filtered to that
   `task_id` after printing it.
4. `bastion run unknown_type` — confirm the `422` surfaces a clear error message (not a panic)
   listing valid workflow types.
5. `bastion run <workflow> --args 'not-json'` — confirm fast-fail before any network call with
   parse error message.
