---
type: ProjectStatus
title: Bastion Status
description: Rollup status for Bastion sub-brain.
doc_id: bastion-status
layer: [meta]
status: active
updated: 2026-07-02T15:01:31Z
now: "BA.13.1 (spec 13.1-persistent-agent-panel) done — /sdlc-flow ran all 4 tasks to PASS, review PASS, docs patched. Status: Done."
next: "Pick the next Phase 13/14 block per focus.next — BA.13.2 / BA.13.3 / BA.13.5 (Phase 13) or BA.14.1 / BA.14.2 / BA.14.3 (color pass) are unblocked. See planning/handoff.md."
blocked: []
---

# Status — Bastion

## Momentum

> Working board — keep all five queues live. **Never end a meaningful session with every queue
> empty.** The headlines of **now / next / blocked** mirror the frontmatter scalars above.

- **now** — Full spec **13.1-persistent-agent-panel** (BA.13.1) done. `/sdlc-flow` ran all 4 tasks
  to PASS in one pass each: Task 1 extracted a pure `session_urgency(&Session) -> u8` out of
  `build_mission_items` (`src/monitor/app.rs`); Task 2 added a pure `agent_panel_rows` builder +
  `AgentPanelRow` model in a new `src/sessions/agent_panel.rs`; Task 3 wired an always-on themed
  bottom "agents · priority" strip into `src/sessions/ui.rs`, rendered under every `SelectedNode`
  with a min-height fallback; Task 4 validated (fmt/clippy --all-targets/test/release build all
  green) and manually smoke-tested via tmux `capture-pane` across Mission Control, a tier, and a
  space. End review verdict: PASS (0 findings). Docs patched: `docs/sessions.md`.
- **next** — Pick the next Phase 13/14 block per `state.json`'s regenerated `focus.next` ordering:
  `BA.13.2` / `BA.13.3` / `BA.13.5` (Phase 13), or `BA.14.1` / `BA.14.2` / `BA.14.3` (color pass,
  unblocked by BA.14.0), or resume Phase 15 (`bastion-product` packaging plan). See
  `planning/handoff.md`.
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
