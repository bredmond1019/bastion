---
type: Reference
title: LLM Spend Summary Surface
description: Operator reference for `bastion costs --last <window>` — formatted table of per-workflow LLM token usage (exact tiktoken counts) and USD cost.
doc_id: costs
layer: [console]
project: bastion
status: active
keywords: [LLM costs, token usage, spend summary, pricing, tiktoken, exact token count, watch, budget, alerts]
related: [monitor, inspect, data-contract, run, abort]
---

# LLM Spend Summary

`bastion costs --last <window>` queries the PostgreSQL `events` table over all workflow runs in
the requested time window, aggregates token usage per workflow type, and prints a formatted spend
summary. Use it to understand which workflows are driving LLM cost. `--watch` (BA.7.C) turns this
into a live-updating view with budget-threshold alerts — see [Watch mode](#watch-mode) below.

> **Needs a Postgres holding the `events` contract.** `costs` reads the PostgreSQL `events` table,
> so `DATABASE_URL` must be set and that Postgres must be reachable. Which stack populates that
> table depends on how the run was triggered — see CLAUDE.md's Environment section for the
> current guidance (D48: the Python orchestrator's `./scripts/dev.sh` stack is not the only
> writer as of `BA.7.C` — runs triggered through the embedded `engine-serve` write via
> `engine-store` instead).

## Usage

```bash
bastion costs --last 7d    # last 7 days (default)
bastion costs --last 30d   # last 30 days
bastion costs --last all   # all time
bastion costs --watch      # live-updating view, --last defaults to 7d
bastion costs --last 30d --watch
```

| Flag | Values | Default | Meaning |
|---|---|---|---|
| `--last` | `7d`, `30d`, `all` | `7d` | Time window to aggregate over. Case-insensitive. |
| `--watch` | flag | off | Re-run the aggregation on a poll interval and re-render, until interrupted (Ctrl-C). See [Watch mode](#watch-mode). |

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

`costs` never panics; it prints a clear message and exits (or, in `--watch` mode, keeps polling)
instead:

| Situation | Behavior |
|---|---|
| Unknown `--last` value | Prints `unknown window '<value>'` + accepted values and exits. |
| `DATABASE_URL` misconfigured / missing | Prints configuration error + hint to set the env var and exits. |
| DB unreachable (one-shot) | Prints connection error + hint and exits. |
| DB unreachable (mid-`--watch`) | Prints a `C0xx`-coded connection error to stderr and keeps polling — a transient failure does not kill the watch loop, and the last known budget verdict is preserved so a crossing on the next successful tick is still evaluated correctly. |

## Watch mode

`bastion costs --watch` (BA.7.C) re-runs the same `fetch_all_runs` → `aggregate` → `render_table`
pipeline on `Config::poll_interval_secs` (`BASTION_POLL_INTERVAL`, default 2s), re-printing the
table each tick until interrupted (Ctrl-C — there is no in-process stop condition). It does not
fork the one-shot aggregation path; it reuses it.

### Budget thresholds and alerts

When a budget cap is configured (`BASTION_MAX_TOTAL_TOKENS` / `BASTION_MAX_COST_USD`, or the
config file's `max_total_tokens` / `max_cost_usd` — see [config.md](config.md)), each tick's
totals are evaluated against the cap by the pure `costs::budget::evaluate`. Crossing a cap prints
a structured alert to stderr and emits a `tracing::warn!` `observ` event
(`event = "budget_alert"`) carrying the cap name, the spent value, and the limit:

```
BUDGET ALERT: cap 'max_total_tokens' breached — spent 105000, limit 100000
```

The alert fires **once per crossing, not once per poll tick** — `costs::budget::detect_crossing`
distinguishes a fresh breach (`Crossing::FreshBreach`, alerts) from an ongoing one
(`Crossing::SustainedBreach`, silent) by comparing each tick's verdict against the previous tick's.
Recovering below the cap re-arms the alert: a later re-crossing is reported as fresh again.

No budget configured is a valid, unchanged configuration — `--watch` behaves exactly as it did
before `BA.7.C`, with no alerts and no extra evaluation cost.

This surface pairs with [run.md](run.md)'s pre-dispatch budget gate (same `costs::budget::evaluate`
core, checked once before a workflow is triggered rather than on a poll loop) and
[abort.md](abort.md) (the switch to stop a run once a breach is noticed).

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
| `costs::budget::evaluate(spend, budget)` | Pure: compares a `Spend` reading against a `Budget`'s optional caps; `>=` the limit counts as breached. |
| `costs::budget::detect_crossing(previous, current)` | Pure: classifies a tick's verdict against the prior tick's as `FreshBreach` / `SustainedBreach` / `Within`. |
| `costs::watch::tick(runs, window, now, budget, previous)` | Pure: one watch tick's full decision — rendered table, verdict, and `Option<BreachReason>` to alert on. |
| `costs::watch::alert_message(reason)` | Pure: formats the stderr alert line from a `BreachReason`. |
| `costs::watch::run(window)` | Async entry point for `--watch`: parses window, loads config, loops fetch → tick → print/alert → sleep until interrupted. |

## Related

- [monitor.md](monitor.md) — live polling view of active workflow runs.
- [inspect.md](inspect.md) — static post-mortem graph view of a single run.
- [run.md](run.md) — the pre-dispatch budget gate that shares this surface's `costs::budget` core.
- [abort.md](abort.md) — the run-abort switch for stopping a run once a budget alert fires.
- [data-contract.md](data-contract.md) — the orchestrator field mappings this surface reads.
