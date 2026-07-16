---
type: Reference
title: Run Abort Switch
description: Operator reference for `bastion abort <run>` — call the Engine's authenticated abort endpoint to stop a running workflow, per BA.7.C / D25.
doc_id: abort
layer: [console, engine]
project: bastion
status: active
keywords: [abort, kill switch, cancellation, X-API-Key, engine-serve, run.md, D25]
related: [run, costs, data-contract, serve-api]
---

# Run Abort Switch

`bastion abort <run>` stops a running workflow by calling the Engine's authenticated abort
endpoint, `POST /events/{run_id}/abort` (contract v1.1.0 §5). Per brain decision D25 — "bastion
triggers, the Engine executes" — bastion only *triggers* the abort: it never cancels a run itself,
writes the `events` row, or touches Celery/Redis. The Engine (`engine-serve`, embedded in
`bastion serve` per D48) flips the run's `CancellationToken` and stamps the terminal state.

> **Naming:** the master plan originally worded this switch as `bastion kill <run>`. That name was
> taken — `bastion kill <session>` already ships as the tmux session-kill subcommand (Phase 5) —
> so this block ships it as **`bastion abort <run>`** instead. `kill` remains tmux-only.

> **Needs `bastion serve` running.** The abort endpoint is served by `engine-serve`'s route table,
> which `bastion serve` mounts (see [serve-api.md](serve-api.md) §15) when both `DATABASE_URL` and
> `BASTION_ENGINE_API_KEY` are configured. It is never served by the Python orchestrator.

## Usage

```bash
bastion abort <run>          # prompts for confirmation before sending
bastion abort <run> --yes    # skip the confirmation prompt (scripted use)
```

| Argument / Flag | Type | Required | Meaning |
|---|---|---|---|
| `<run>` | string | yes | The `run_id` to abort. |
| `--yes` | flag | no | Skip the interactive confirmation prompt. The operator/script invoking the command is still the one choosing to abort — `--yes` only removes the prompt, it does not change what gets sent. |

Without `--yes`, `abort` prints:

```
About to abort run 'run-123'. This cannot be undone. Continue? [y/N]
```

Only `y`/`yes` (case-insensitive, whitespace-trimmed) confirms; anything else — including empty
input — cancels with `abort cancelled (not confirmed)` and no network call.

## Outcomes

`abort` calls `POST /events/{run_id}/abort` with the `X-API-Key` header
(`BASTION_ENGINE_API_KEY` / config file's `engine_api_key`) and reports the outcome distinctly for
each pinned response, plus a connection failure:

| Outcome | Message | `C0xx` code |
|---|---|---|
| `202 { run_id, status }` | `abort accepted: run <id> is now '<status>'` | none — a successful abort is not an error/degradation signal |
| `404` (unknown or already-finished run) | `abort failed: run '<id>' not found or already finished` | `C002` |
| `401` (bad/missing `X-API-Key`) | `abort failed: engine rejected the request (bad or missing X-API-Key)` | `C012` |
| Connection failure (engine unreachable) | `abort failed: could not reach the engine for run '<id>'` — points at `bastion serve` | `C009` |
| `engine_api_key` not configured | Same "could not reach the engine" framing — points at `bastion serve` | `C005` |

All four outcome messages are guaranteed distinct (asserted by `render_outcome_all_four_outcomes_
are_distinct_messages` in `src/run/abort.rs`), so scripted callers can `grep` for the specific
failure mode.

## Key internals

| Symbol | Role |
|---|---|
| `run::abort::confirm_prompt(run_id)` | Pure: builds the confirmation prompt text. |
| `run::abort::parse_confirmation(input)` | Pure: `y`/`yes` (case-insensitive, trimmed) → confirmed; everything else → declined. |
| `run::abort::render_outcome(run_id, result)` | Pure: one branch per `AbortOutcome`/`Err`, formatting the operator-facing message. |
| `api::client::ApiClient::abort_run(run_id)` | Async I/O: `POST /events/{run_id}/abort` with `X-API-Key`, 5s timeout; classifies the response via `classify_abort_response`. |
| `api::client::classify_abort_response(status, body)` | Pure: maps `202`/`404`/`401` to a typed `AbortOutcome`; any other status is a decode/contract-mismatch `Err`. |
| `run::abort::run(run_id, yes)` | Async entry point: confirm (unless `--yes`) → load config → call the API → print outcome → emit `observ` event. |

## Testing

The pure confirmation/rendering logic is unit-tested element-by-element in `src/run/abort.rs`
(Rule 6), including every outcome branch and boundary case (empty input, mixed case, trailing
newlines). `classify_abort_response` is unit-tested against fixtures in `src/api/client.rs`. The
end-to-end path — a real HTTP client against a real `engine-serve` `App` — is covered by the
in-process integration test `tests/abort_contract.rs`, which asserts the 401 / 404 / 202 paths
against the actual engine (no orchestrator, no external stack, no manual step).

## Related

- [run.md](run.md) — the pre-dispatch budget gate that can prevent a run from starting in the
  first place; `abort` is the switch for a run already in flight.
- [costs.md](costs.md) — `--watch`'s budget alerts, the usual trigger for reaching for `abort`.
- [serve-api.md](serve-api.md) — how `bastion serve` mounts the engine route table this surface
  calls.
- [data-contract.md](data-contract.md) — the pinned abort endpoint wire shape and its
  `api::client::abort_run` mapping.
