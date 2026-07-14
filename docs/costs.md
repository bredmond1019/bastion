---
type: Reference
title: LLM Spend Summary Surface
description: Operator reference for `bastion costs --last <window>` — formatted table of per-workflow LLM token usage (exact tiktoken counts) and USD cost.
doc_id: costs
layer: [console]
project: bastion
status: active
keywords: [LLM costs, token usage, spend summary, pricing, tiktoken, exact token count]
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
Workflow                           Runs  Tokens In   Tokens Out   USD
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
| Tokens In | Exact `tiktoken` count of each node's input text (via `costs::tokens::count`); falls back to the orchestrator-reported `node_runs[*].usage.input_tokens` when a node has no countable text or no model id, summed across all nodes in all runs |
| Tokens Out | Exact `tiktoken` count of each node's output text (same rule), summed across all nodes in all runs |
| USD | Dollar figure computed from the exact token counts above, using the hardcoded per-model price table in `src/costs/pricing.rs` |

Rows are sorted by `USD` descending. A TOTAL row appears after the separator. If any
node references a model not in the price table, a trailing notice lists the unpriced model count.

## Token counting

Token counts are **exact**, not estimated: `costs::tokens::count(text, model)` runs the real
`tiktoken` encoder (`cl100k_base` or `o200k_base`, selected by model id — `o200k_base` for the
GPT-4o/4.1/o-series family, `cl100k_base` for everything else including Claude models, with no
panic on an unknown model id). When a node carries no countable input/output text, or no model id,
`costs::exact_or_reported_tokens` falls back to the orchestrator-reported `tokens_in`/`tokens_out`
for that node.

## Pricing

Prices are hardcoded in `src/costs/pricing.rs` (`price_for(model) -> Option<ModelPrice>`).
Unknown models contribute `$0.00` to the cost and are reported as unpriced. No config file
or environment variable is needed. The price table itself is unchanged by exact token counting —
only the token counts feeding it became exact.

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
| `costs::tokens::count(text, model)` | Pure: exact `tiktoken` token count for `text` under the encoder selected for `model` (`cl100k_base`/`o200k_base`); `0` for empty text; no panic on unknown model id. |
| `costs::extract_text(&Option<Value>)` | Pure: pulls countable text out of a `NodeState.input`/`.output` JSON value (string used directly; other shapes serialized to text; `None` passes through). |
| `costs::exact_or_reported_tokens(&NodeState)` | Pure: `(u64, u64)` — exact `tokens::count` for input/output when both text and model are present, else the orchestrator-reported `tokens_in`/`tokens_out`. |
| `costs::pricing::price_for(model)` | Pure: returns `ModelPrice` for known models, `None` for unknown. |
| `costs::pricing::cost_usd(model, in, out)` | Pure: returns `f64` dollar figure from exact token counts (0.0 if unknown model). |
| `parse_window(s)` | Pure: parses `"7d"` / `"30d"` / `"all"` into `Window`; rejects anything else. |
| `within_window(window, now, started_at)` | Pure: returns `bool`; `now: DateTime<Utc>` is injected (no I/O). |
| `aggregate(runs, window, now)` | Pure: filters, groups, sums exact/fallback token counts, sorts; returns `CostSummary`. |
| `render_table(summary)` | Pure: formats `CostSummary` into a fixed-width string ready for `print!`. |
| `db::costs::fetch_all_runs(db_url)` | I/O shell: 1-connection pool, `SELECT id, workflow_type, task_context FROM events`, assembles rows via the shared `parse_event_row` from `db::workflows`. |
| `costs::run(window)` | Async entry point. Parses window, loads config, fetches runs, aggregates, prints. |

## Related

- [monitor.md](monitor.md) — live polling view of active workflow runs.
- [inspect.md](inspect.md) — static post-mortem graph view of a single run.
- [data-contract.md](data-contract.md) — the orchestrator field mappings this surface reads.
