---
type: TaskSpec
title: "Task Spec — Phase 13, Block BA.13.2: Mouse interactivity"
description: Decompose BA.13.2 into disjoint-ownership tasks rebuilding a pure on_mouse dispatcher over the current panes (spine, browser, content, agent panel); sub-tab routing deferred to BA.13.4.
doc_id: 13-2-mouse-interactivity-tasks
layer: [console]
project: bastion
status: active
keywords: [mouse, on-mouse, point-in, viewport-rect, sessions-tui, unified-console]
related: [bastion-master-plan, 13-0-spine-primary-navigation-tasks, 13-1-persistent-agent-panel-tasks]
---

# Task Spec — Phase 13, Block BA.13.2: Mouse interactivity

**Status:** Not started · **Last run:** never

## Goal
Rebuild mouse handling as a pure `on_mouse` dispatcher that stores per-pane viewport `Rect`s during draw and routes clicks/scroll via `bella_engine::geometry::point_in` across the current unified-console panes.

## Context Pointers
- **Plan section:** `planning/master-plan.md` → Phase 13 → *Block BA.13.2 — Mouse interactivity* (lines ~1210–1230). Mouse capture is already enabled (`src/sessions/ui.rs:520`, `:564`); BA.13.0 **deleted** the old `on_mouse` (see the comment at `src/sessions/app.rs:459`), so this block re-introduces the dispatcher from scratch against the post-BA.13.0 layout.
- **Repo files (per block's *Files* list):** `src/sessions/app.rs` (viewport `Rect` fields on `AppState` + pure `on_mouse` dispatcher), `src/sessions/ui.rs` (store the `Rect`s during draw; extend the Mouse event arm for scroll).
- **Interfaces / shared surface:** reuses `bella_engine::geometry::{point_in, body_pos, select_word_at}` (confirmed exported at `../bella/crates/bella-engine/src/geometry.rs`; bastion already depends on `bella_engine`). Panes available today: `selected_spine` (BA.13.0), `file_browser: bella_engine::browser::Browser` (`app.rs:58`), the content pane, and the agent panel (BA.13.1).
- **CLAUDE.md standing rules:** Rule 1 (tests ship with every behaviour change), Rule 6 (`on_mouse` is a **pure** dispatcher — unit-tested per pane with synthetic `Rect`s + coords yielding the asserted `Action`/state, no terminal; keep the `ui.rs` event arm a thin shell). Track is DB-free (D4), read-only vs the orchestrator (D2).
- **Scope decision (2026-07-02, session):** the sub-tab bar (`SubTab`/`sub_tab`/`subtab_area`) does **not** exist yet — it's introduced by BA.13.4 (gated behind BA.13.3). Per the block's real dependency (BA.13.0 only), **this spec ships mouse support for the panes that exist now.** Sub-tab **click routing** and the agent-panel `SubTab::Sessions` jump are **deferred to BA.13.4**; the agent-panel row here jumps to that session's **space** (`selected_spine`) only.
- **Out of scope (hard boundary, from the block + scope decision):** the standalone `monitor` subcommand's own event loop (`src/monitor/events.rs`); word/link selection in the content pane (optional/best-effort — `select_word_at` may be wired but is not required); sub-tab click routing and `SubTab::Sessions` (BA.13.4).

## Step-by-Step Tasks

### BA.13.2.1 Viewport `Rect` fields + pure `on_mouse` dispatcher in `src/sessions/app.rs`
- **Owns:** `src/sessions/app.rs` (only file touched by this task).
- Add per-pane viewport `Rect` fields to `AppState`, defaulting to empty/zero `Rect`s: `spine_area`, `browser_area`, `content_area`, `agent_panel_area`. (Do **not** add `subtab_area` — deferred to BA.13.4.)
- Add a pure `on_mouse(&mut self, kind, col, row) -> Option<Action>` (match the repo's existing action/return convention) that routes via `bella_engine::geometry::point_in(rect, col, row)`:
  - `spine_area` → set `selected_spine` to the clicked row (respecting the wrapping/selectable-row model from BA.13.0).
  - `browser_area` → set `file_browser.selected` to the clicked entry.
  - `content_area` → scroll / cursor position via `bella_engine::geometry::body_pos`.
  - `agent_panel_area` → jump to that session's **space** by setting `selected_spine` (no `SubTab` — deferred).
  - `ScrollUp`/`ScrollDown` anywhere over the content pane → scroll the content pane.
  - Clicks outside all known Rects → no-op (`None`).
- **Tests (Rule 6):** unit-test `on_mouse` per pane with synthetic `Rect`s + coords (no terminal), asserting: a spine click selects the right row; a browser click selects the right entry; a content-pane scroll adjusts the scroll offset; an agent-panel click sets `selected_spine` to that session's space; an out-of-bounds click is a no-op. Cover `ScrollUp` and `ScrollDown`.

### BA.13.2.2 Store pane `Rect`s during draw + extend the Mouse event arm in `src/sessions/ui.rs`
- **Owns:** `src/sessions/ui.rs` (only file touched by this task). **Depends on:** BA.13.2.1 (the `Rect` fields + `on_mouse`).
- During draw, write each pane's computed viewport `Rect` back onto `AppState` (`spine_area`, `browser_area`, `content_area`, `agent_panel_area`) so `on_mouse` has live geometry.
- Extend the Mouse event arm (replacing the "out of scope" stub around `ui.rs:498`) to forward `Down`/`ScrollUp`/`ScrollDown` events into `AppState::on_mouse` and apply the returned `Action`/state, mirroring how key events are dispatched.
- Keep this arm a thin shell over the pure `on_mouse` (Rule 6): translate crossterm `MouseEvent` → `(kind, col, row)`, call `on_mouse`, apply the result.
- **Tests:** a `draw_for_test` (existing `tui_tests.rs` pattern) asserting the pane `Rect`s are populated after a draw (non-zero for the visible panes) so `on_mouse` routing has real geometry to match against.

### BA.13.2.3 Validate
- Run the Validation Commands listed below and confirm all pass.
- Manually smoke-test the TUI via tmux (mouse events can't be scripted through `send-keys`, so drive interactively where possible): click a spine row, a browser entry, and an agent-panel row; wheel-scroll the content pane; confirm each selects/scrolls the expected target and clicks outside panes are inert. Record the result (including any interactive-only limitation) in `## Notes`.

## Acceptance Criteria
- Clicking a spine row selects it (`selected_spine`); clicking a browser entry selects it (`file_browser.selected`); clicking an agent-panel row jumps to that session's space.
- Wheel `ScrollUp`/`ScrollDown` scrolls the content pane.
- `on_mouse` is unit-tested per pane with synthetic `Rect`s + coords yielding the asserted `Action`/state (no terminal), including an out-of-bounds no-op.
- Pane viewport `Rect`s are populated during draw (asserted via `draw_for_test`).
- Sub-tab click routing and `SubTab::Sessions` are **not** implemented here (deferred to BA.13.4); no `subtab_area` field is added.
- All gated checks pass.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
<filled in as work happens>

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
