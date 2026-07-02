---
type: Reference
title: LLM Spend Summary Surface
description: Operator reference for `bastion costs --last <window>` — formatted table of per-workflow LLM token usage and estimated USD cost.
doc_id: costs
layer: [console]
project: bastion
status: active
keywords: [LLM costs, token usage, spend summary, pricing, cost estimation]
related: [monitor, inspect, data-contract]
---

# LLM Spend Summary

`bastion costs --last <window>` queries the orchestrator's PostgreSQL `events` table over all
workflow runs in the requested time window, aggregates token usage per workflow type, and prints
a formatted spend summary. Use it to understand which workflows are driving LLM cost.

> **Needs the orchestrator stack up.** `costs` reads the PostgreSQL `events` table, so
> `DATABASE_URL` must be set and the orchestrator's Postgres must be reachable. Bring the stack
> up from the `python-orchestration-system/` repo:
> `./scripts/dev.sh` (START) / `./scripts/dev.sh stop` (STOP).

## Usage

```bash
bastion costs --last 7d    # last 7 days (default)
bastion costs --last 30d   # last 30 days
bastion costs --last all   # all time
```

| Flag | Values | Default | Meaning |
|---|---|---|---|
| `--last` | `7d`, `30d`, `all` | `7d` | Time window to aggregate over. Case-insensitive. |

## Output format

```
Workflow                           Runs  Tokens In   Tokens Out   Est. USD
-----------------------------  ------  ----------  ----------  ----------
ContentGenerationWorkflow           3      12 450       4 200       $0.53
SummaryWorkflow                     1       3 100         900       $0.11
-----------------------------  ------  ----------  ----------  ----------
TOTAL                               4      15 550       5 100       $0.64

Note: 1 unpriced model(s) encountered — cost may be understated.
```

Columns:

| Column | Contents |
|---|---|
| Workflow | `workflow_type` from the `events` row |
| Runs | Distinct `events.id` count in the window |
| Tokens In | Sum of `node_runs[*].usage.input_tokens` across all nodes in all runs |
| Tokens Out | Sum of `node_runs[*].usage.output_tokens` across all nodes in all runs |
| Est. USD | Dollar estimate using the hardcoded per-model price table in `crates/bastion/src/costs/pricing.rs` |

Rows are sorted by `Est. USD` descending. A TOTAL row appears after the separator. If any
node references a model not in the price table, a trailing notice lists the unpriced model count.

## Pricing

Prices are hardcoded in `crates/bastion/src/costs/pricing.rs` (`price_for(model) -> Option<ModelPrice>`).
Unknown models contribute `$0.00` to the estimate and are reported as unpriced. No config file
or environment variable is needed.

Seeded models include current Claude models (`claude-opus-4-8`, `claude-sonnet-4-6`,
`claude-haiku-4-5`, and variants), retired Claude models that appear in existing fixtures, and
OpenAI embedding models (`text-embedding-3-small`, `text-embedding-3-large`,
`text-embedding-ada-002`). To add a new model, extend the match arm in `price_for`.

## Degrade paths

`costs` never panics; it prints a clear message and exits instead:

| Situation | Behavior |
|---|---|
| Unknown `--last` value | Prints `unknown window '<value>'` + accepted values and exits. |
| `DATABASE_URL` misconfigured / missing | Prints configuration error + hint to set the env var and exits. |
| DB unreachable | Prints connection error + orchestrator-stack hint and exits. |

## Key internals

| Symbol | Role |
|---|---|
| `costs::pricing::price_for(model)` | Pure: returns `ModelPrice` for known models, `None` for unknown. |
| `costs::pricing::estimate_usd(model, in, out)` | Pure: returns `f64` dollar estimate (0.0 if unknown). |
| `parse_window(s)` | Pure: parses `"7d"` / `"30d"` / `"all"` into `Window`; rejects anything else. |
| `within_window(window, now, started_at)` | Pure: returns `bool`; `now: DateTime<Utc>` is injected (no I/O). |
| `aggregate(runs, window, now)` | Pure: filters, groups, sums, sorts; returns `CostSummary`. |
| `render_table(summary)` | Pure: formats `CostSummary` into a fixed-width string ready for `print!`. |
| `db::costs::fetch_all_runs(db_url)` | I/O shell: 1-connection pool, `SELECT id, workflow_type, task_context FROM events`, assembles rows via the shared `parse_event_row` from `db::workflows`. |
| `costs::run(window)` | Async entry point. Parses window, loads config, fetches runs, aggregates, prints. |

## Related

- [monitor.md](monitor.md) — live polling view of active workflow runs.
- [inspect.md](inspect.md) — static post-mortem graph view of a single run.
- [data-contract.md](data-contract.md) — the orchestrator field mappings this surface reads.
