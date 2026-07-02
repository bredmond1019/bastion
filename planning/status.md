---
type: ProjectStatus
title: Bastion Status
description: Rollup status for Bastion sub-brain.
doc_id: bastion-status
layer: [meta]
status: active
updated: 2026-07-02T13:38:57Z
now: "BA.14.0 (spec 14.0-config-driven-theme) done — 4/4 tasks passed, review PASS. Status: Done."
next: "Decide among BA.13.2 / BA.13.3 / BA.13.5 / BA.14.1 / BA.14.2 (state.json focus.next) or resume Phase 15 — see planning/handoff.md."
blocked: []
---

# Status — Bastion

## Momentum

> Working board — keep all five queues live. **Never end a meaningful session with every queue
> empty.** The headlines of **now / next / blocked** mirror the frontmatter scalars above.

- **now** — Full spec **14.0-config-driven-theme** (BA.14.0) done — 4/4 tasks passed, review verdict PASS. `src/ui_theme.rs` now defines a runtime `Theme` (`bastion` preset) behind a process-wide `OnceLock` accessor, with a pure `theme_by_name` lookup and a `to_bella_theme` mapping to `bella_engine::Theme`; `src/config.rs` gained an optional `[theme]` section with a pure `resolve_theme()` (default fallback when absent/unknown, existing configs still parse unchanged); `src/sessions/ui.rs` initializes the runtime theme from resolved config at TUI startup and both `render_with_edit` call sites now pass the mapped theme, so chrome and the markdown view share one palette. Full validation suite green (fmt/clippy/test — 1037 passed/build --release) and the TUI was smoke-tested live via tmux across named/unknown/absent `[theme]` config states with no panic. No `../bella` files touched (Rule 7 caveat not triggered — existing `bella_engine::Theme` covered the mapping). Docs updated: `docs/config.md`, `docs/sessions.md`.
- **next** — Decide among BA.13.2 / BA.13.3 / BA.13.5 / BA.14.1 / BA.14.2 per `state.json`'s `focus.next` ordering, or resume Phase 15 (`bastion-product` packaging plan). See `planning/handoff.md` for the open question.
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
