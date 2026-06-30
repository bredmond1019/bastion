---
type: ProjectStatus
title: Bastion Status
description: Rollup status for Bastion sub-brain.
doc_id: bastion-status
layer: [meta]
status: active
now: BA.11.C0 agent-state detection manifest engine complete — 812 tests pass, PASS verdict
next: Start BA.11.C (WebSocket hub + live pane streaming); BA.11.C0 detect() is the seam BA.11.C needs
blocked: []
---

# Status — Bastion

## Momentum

> Working board — keep all five queues live. **Never end a meaningful session with every queue
> empty.** The headlines of **now / next / blocked** mirror the frontmatter scalars above.

- **now** — Phase 11: BA.11.C0 done — pure config-driven agent-state detection engine (TOML manifests, gate matcher, Claude + Pi seeds); 812 tests pass, PASS verdict
- **next** — Start BA.11.C (WebSocket hub + live pane streaming, BastionUI/D28 priority); BA.11.C0 `detect()` is the seam BA.11.C needs; BA.7.B (exact `bastion costs` tiktoken counter) as a lower-priority interleave
- **blocked** — nothing blocked
- **improve** — `blank_code_spans` handles single-backtick inline spans only (fenced triple-backtick blocks out of scope); confirm `bastion validate` skips `trees/` if worktrees accumulate `.md` files; `status` config-file API URL not loaded when `DATABASE_URL` absent
- **recurring** — none yet

## Metrics

> Cheap, hand-maintained signals (leading + lagging). Do **not** push these into frontmatter —
> they are multi-valued and volatile.

- tasks completed / verified this period; intervention rate; retry rate; regression rate
- reusable assets created since last milestone
- days since last eval improvement; days since last new skill/workflow
- % of runs ending with an explicit next action
