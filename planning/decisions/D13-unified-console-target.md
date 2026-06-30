---
type: Decision
title: "D13: The three keyboard-only TUIs converge into one unified operator console"
description: monitor, sessions, and costs/momentum merge into a single ratatui shell (BA.12.A) — sidebar of live-state entities + tabbed/paned main + mouse + compute-then-render over one event loop and the serve transport.
doc_id: D13-unified-console-target
status: active
keywords: [console, TUI, ratatui, unified, mouse, compute-then-render]
related: [D11-herdr-reference-only, D12-toml-manifest-detection, D4-session-management-surface, D2-observability-consumer-contract]
---

# D13 — The three keyboard-only TUIs converge into one unified operator console

**Date:** 2026-06-29
**Supersedes:** none (refines the implicit "three separate TUIs" assumption of Phases 1–7)
**Builds on:** D4 (session surface), D2 (observability consumer), D11, D12

## Context

bastion has grown three **independent, keyboard-only** TUI surfaces, each with its own event loop:
`monitor` (live workflow graph), `sessions` (tmux dashboard), and the `costs`/momentum reads. They share
no entity state, duplicate render/loop scaffolding, and can't show a unified "what is live right now"
view. The herdr-bella console research (2026-06-29) settled the target shape.

## Decision

**The three TUIs converge into one operator console — `BA.12.A` — built on a single event loop and the
`serve` transport.** Its shape:

- **Sidebar of live-state entities** — runs + sessions as rows, each with a working / blocked / idle /
  done chip driven by the `src/detect/` engine (D12).
- **Tabbed / paned main area** — run graph · costs · momentum · (Kanban block tracker) as tabs.
- **Compute-then-render split** — a `compute_view()` pass mutates view geometry *before* render, each
  frame (the Herdr `src/ui.rs` pattern, reimplemented clean per D11).
- **Workspace Overview pane** (`BA.12.A`) — a `notify` watcher on each repo's `planning/status.md` that
  parses the D30 `now`/`next`/`blocked` scalars (via `mev`) and renders them as a structured pane.
- **Kanban Block Tracker tab** (`BA.12.B`) — parses `master-plan.md` wave tables (via an extended `mev`
  parser — a cross-repo seam) and groups blocks by status into Ratatui columns with `Gauge` progress
  bars and cross-repo BLOCKED/BLOCKING badges.
- **Mouse** (`BA.12.C`) — `EnableMouseCapture` + an `on_mouse() -> Action` branch following **Bella's**
  `map_mouse` pattern (same ratatui 0.30 / crossterm 0.29 stack), **not** Herdr's (D11). `bella-engine`
  is added as a path dependency only when a markdown pane is needed.

## Alternatives considered

- **Keep three separate TUIs.** Rejected: triplicated event loops, no shared live-entity state, no single
  "what's live" view, and three places to add mouse/streaming.
- **Adopt Herdr's layout/runtime wholesale.** Rejected: AGPL + transport mismatch (D11). Patterns are
  reimplemented, not inherited.
- **Mouse via Herdr's selection model.** Rejected: Bella already implements the same screen→content
  coordinate conversion on the identical ratatui/crossterm versions and is defensibly ours.

## Consequences

- `BA.12.A` is the convergence block; the Workspace Overview pane and Kanban tab become `BA.12.A`/`BA.12.B`
  features rather than standalone tools.
- A **cross-repo seam** is created: the Kanban tab depends on a `mev` master-plan / wave-table parser
  (coordinated in the consolidated core master-plan).
- The momentum/metrics read (program "Block V") is folded in as a console tab rather than a separate surface.
- Existing single-purpose entry points (`bastion monitor`, `bastion sessions`) may remain as thin launchers
  into the unified shell or be retired during `BA.12.A` — decided when that block is specced.
