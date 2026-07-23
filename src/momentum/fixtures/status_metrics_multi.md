---
type: ProjectStatus
title: Fixture Status — Multi-Bullet Metrics
description: status.md fixture with a multi-bullet Metrics section, used by parse_metrics tests.
doc_id: fixture-status-metrics-multi
layer: [meta]
status: active
updated: 2026-07-23T00:00:00Z
now: "BA.7.D in progress — momentum module"
next: "Wire the CLI subcommand"
blocked: "[]"
---

# Status — Fixture (Multi-Bullet Metrics)

## Momentum

- **now** — BA.7.D in progress — momentum module
- **next** — Wire the CLI subcommand
- **blocked** — nothing blocked
- **improve** — tighten metrics parsing
- **recurring** — none yet

## Metrics

> Snapshot as of the last `mev emit-state` run.

- blocks shipped this week: 3
- open backlog tickets: 12
- days since last handoff: 2

## Notes

This section must never be captured by `parse_metrics` — it exists to prove
that bullet capture stops at the next `## ` heading.

- this bullet must not appear in the parsed Metrics vec
