---
type: Reference
title: Workflow Trigger Surface
description: Operator reference for `bastion run <workflow> [--args '{}'] [--monitor]` — trigger a workflow via the FastAPI generic dispatcher and optionally drop into the live monitor.
---

# Workflow Trigger

`bastion run <workflow>` issues `POST /` to the orchestrator's generic dispatcher with
`{ "workflow_type": <workflow>, "data": <args> }`, prints the returned `task_id`, and
optionally drops into the live monitor filtered to that run.

> **Needs the orchestrator stack up.** `run` posts to the FastAPI endpoint, so
> `BASTION_API_URL` must be set and the orchestrator's API must be reachable. Bring the stack
> up from the `python-orchestration-system/` repo:
> `./scripts/dev.sh` (START) / `./scripts/dev.sh stop` (STOP).

## Usage

```bash
bastion run <workflow>                      # trigger with empty data payload
bastion run <workflow> --args '{"k":1}'    # trigger with a data object
bastion run <workflow> --monitor           # trigger then open live monitor for this run
bastion run <workflow> --args '{"k":1}' --monitor
```

| Argument / Flag | Type | Required | Meaning |
|---|---|---|---|
| `<workflow>` | string | yes | `workflow_type` sent to the orchestrator. An unknown type returns a `422` with a list of valid types. |
| `--args` | JSON object string | no | Forwarded as the `data` field. Omit to send `data: {}`. Must be a JSON object (not a number, array, or bare string). Malformed JSON exits with a clear error before any network call. |
| `--monitor` | flag | no | After printing `task_id`, hand off to `bastion monitor` filtered to that run. |

## Output

On success, prints a stable greppable line then exits (or enters the monitor):

```
task_id: 3fa85f64-5717-4562-b3fc-2c963f66afa6
```

On error, prints a human-readable message and exits non-zero. Examples:

| Situation | Message style |
|---|---|
| Orchestrator unreachable | `error: orchestrator unreachable — is the stack up?` |
| Unknown workflow type (`422`) | HTTP error message from the orchestrator (lists valid types) |
| Malformed `--args` JSON | `invalid JSON in --args: ...` |
| Non-object `--args` value | `--args must be a JSON object, got <type>` |

## Key internals

| Symbol | Role |
|---|---|
| `run::parse_args(args)` | Pure: `None` → `Ok(None)`; parses string as JSON; rejects non-objects with a typed message. |
| `run::format_trigger_success(task_id)` | Pure: formats the `task_id: <id>` output line. |
| `api::client::trigger_body(workflow_type, data)` | Pure: builds `TriggerRequest`; maps `None` data → `{}`. |
| `api::client::trigger_url()` | Pure: returns `<base_url>/` with trailing-slash normalisation. |
| `api::client::trigger_workflow(workflow_type, data)` | Async I/O: POST, 2s timeout, `.error_for_status()`, decode `TaskAccepted`, return `task_id`. |
| `run::trigger(workflow, args, monitor)` | Async entry point: load config → parse args → call API → print → optional monitor hand-off. |

## Degrade paths

`run` never panics; it returns a clear `Err` in all error paths:

| Situation | Behavior |
|---|---|
| Malformed `--args` | Validates before any network call; exits with parse error. |
| Non-object `--args` value | Exits with `got <type>` message. |
| Network / timeout | Propagates reqwest error with orchestrator-unreachable hint. |
| `422` from orchestrator | Propagates HTTP error (unknown workflow, invalid data). |

## Related

- [monitor.md](monitor.md) — live polling view of active workflow runs (the `--monitor` hand-off target).
- [data-contract.md](data-contract.md) — the orchestrator field mappings including the trigger endpoint contract.
