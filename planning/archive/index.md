---
type: Index
title: bastion Planning Archive
description: Retired bastion spec/task folders, kept verbatim for provenance after their durable content was distilled into knowledge.md/memory.md/decisions.
doc_id: bastion-planning-archive-index
layer: [factory]
project: bastion
status: active
keywords: [archive, retired, completed blocks, provenance, distillation]
related: [knowledge, memory, master-plan, planning-index]
---

# bastion Planning — Archive

Retired spec/task folders. Each was completed (or, where noted, intentionally shelved) and its
durable residue was folded forward before retirement (D35 "never archive empty-handed"). Files
here are kept **verbatim for provenance** — do not edit.

| Folder | What it was | Status |
|---|---|---|
| `11.B-session-rest/` | BA.11.B — Session REST API (list/pane/send/key/create/delete) + tmux named-key helper on `bastion serve`, serve-api contract bumped to v0.1 | Complete — shipped PR #6; residue distilled 2026-07-02 |
| `11.C-websocket-hub/` | BA.11.C — WebSocket hub (topic subscriptions, ref-counted pane polls, diff-and-push fan-out) + needs-input detection wired to Block C₀'s manifest engine, serve-api bumped to v0.2 | Complete — shipped PR #8; residue distilled 2026-07-02 |
| `11.C0-agent-state-detection/` | BA.11.C0 — Agent-state detection manifest engine: config-driven TOML manifests (region + gate matchers + priority) compiled into rules, clean-room reimplementation of Herdr's pattern (D11/D12), seeded with Claude + Pi manifests | Complete — shipped PR #7; residue distilled 2026-07-02 |
| `12.a-unified-console/` | Phase 12 Block A (ad-hoc/umbrella) — dynamic tab engine, TUI mouse events, hierarchical DAG tree, tmux suspend/attach UX; Tasks 1–4 shipped, Tasks 5–6 (bella-engine integration / manifest engine) superseded by later dedicated blocks | Closed — Tasks 1–4 complete, 5–6 superseded; residue distilled 2026-07-02 |
| `12.c-kanban-rows/` | BA.12.C — Kanban board layout swapped from 3 side-by-side columns to 3 stacked horizontal rows (`src/overview/mod.rs`) | Closed — verified shipped in code though spec's own Notes/status were never updated; residue distilled 2026-07-02 |
| `12.d-mission-control-theme/` | BA.12.D — Retheme Mission Control off hardcoded `ratatui::Color` onto the shared `ui_theme` palette (borders + `status_color()`/error spans) | Closed per state.json — but verified only Task 2 (borders) shipped; Task 1 (`status_color()`/error-span retheme) was never implemented; gap flagged in memory.md; residue distilled 2026-07-02 |
| `12.e-mission-control-sessions/` | BA.12.E — Merge tmux sessions + orchestrator workflow runs into one Mission Control list via `MissionItem` + `build_mission_items()`, unified detail-pane rendering, session keybindings rewired | Complete — verified shipped in `src/monitor/app.rs`; residue distilled 2026-07-02 |
| `13.0-spine-primary-navigation/` | BA.13.0 — Replaced the three-tab layout with a spine-only navigator (`SpineRow`/`SelectedNode`, `Hq` tier replacing `_root`/`brain`, wrap-around selection, tier-overview routing) | Complete — shipped, 1022 tests passing; residue distilled 2026-07-02 |
| `13.1-persistent-agent-panel/` | BA.13.1 — Always-visible bottom "agents · priority" strip under every `SelectedNode`: `session_urgency`, `agent_panel_rows`, themed state dots, min-height fallback | Complete — shipped PR #12, 1056 tests passing; residue distilled 2026-07-02 |
| `13.2-mouse-interactivity/` | BA.13.2 — Authored spec (not built) for a pure `on_mouse` dispatcher over spine/browser/content/agent-panel viewport `Rect`s via `bella_engine::geometry` | **Shelved, never started** — no `sdlc/` run, Notes never filled in; operator paused Phase 13/14 before this block began; residue is the design itself (sparse — see memory.md) 2026-07-02 |
| `14.0-config-driven-theme/` | BA.14.0 — Config-driven runtime `Theme` system: `OnceLock`-backed active theme, `theme_by_name`/`to_bella_theme` mapping, `[theme]` config section, shared chrome+markdown theming | Complete — shipped, 1037 tests passing; residue distilled 2026-07-02 |
| `phase11-blockD/` | BA.11.D — Repo/workflow status REST surface (`GET /repos`, `/status`, `/handoff`, `/workflows`) + `FlowWatcher` transition detector, serve-api bumped to v0.3 | Complete — shipped PR #9; note: `workflow_done` WS push was deferred (documented but not wired to the live Hub — still true, see memory.md); residue distilled 2026-07-02 |
