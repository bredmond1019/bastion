---
type: TaskSpec
title: "Task Spec — Phase 13, Block 2: Mouse interactivity"
description: Full pure mouse dispatcher for the unified console — per-pane viewport Rects on AppState, click-to-select across spine/browser/agent-panel, wheel scroll, routed via bella_engine geometry.
doc_id: 13-2-mouse-interactivity
layer: [console]
project: bastion
status: active
keywords: [mouse, on_mouse, point_in, pane areas, click-to-select, wheel scroll, unified console]
related: [master-plan, planning-index]
---

# Task Spec — Phase 13, Block 2 (BA.13.2)

**Status:** Not started · **Last run:** never

## Goal

Extend the already-enabled mouse capture into a full pure dispatcher: store per-pane viewport `Rect`s on `AppState` during draw, rewrite `on_mouse` to route clicks via `bella_engine::geometry::point_in` (spine → `selected_spine`, browser list → `file_browser.selected`, agent panel → jump to that session's space, content → scroll), and handle `ScrollUp`/`ScrollDown`.

## Context Pointers

- **Block definition:** `planning/master-plan.md` → Phase 13 → `### Block BA.13.2`. Depends on BA.13.0 (spine) — **done**. BA.13.1 (agent panel) and BA.14.0 (theme) are also already landed.
- **Repo-state adaptations (grounded deviations from the block text, which was authored before 13.0/13.1 landed):**
  - The block mentions `subtab_area` → `sub_tab` routing, but **the sub-tab bar is BA.13.4, which is blocked (depends on BA.13.3) and not in the tree**. Sub-tab click routing is **out of scope here**; the dispatcher must be structured (single `match`-per-pane over stored Rects) so BA.13.4 adds a `subtab_area` arm without a rewrite. Do NOT invent a `SubTab` type.
  - The block says agent-panel click jumps to "that session's space + `SubTab::Sessions`". `SubTab` doesn't exist and session→space mapping is BA.13.3 (not landed) — today's only link is **slug-name equality** (see `selected_space_slug` usage in `src/sessions/app.rs`). v1 behavior: clicking an agent-panel row selects the spine row whose Space slug equals the session name; no-op when no space matches. Refines after 13.3/13.4.
- **Current mouse state:** capture is already enabled (`src/sessions/ui.rs:520` and `:564` — `event::EnableMouseCapture`), but the event loop only matches `Event::Key` (`ui.rs:497–501`) and the old `on_mouse` was deliberately removed in 13.0 (`src/sessions/app.rs:459` comment). This block adds the `Event::Mouse` arm and a fresh dispatcher.
- **Layout to mirror:** `draw_with_root` (`ui.rs:232`) — outer vertical split (`areas`, `ui.rs:247`: main + agent-panel strip via `agent_panel_strip_height`), `main_chunks` (`ui.rs:279`: sidebar/spine + main area), `overview_chunks` (`ui.rs:323`: browser + content). `draw` (`ui.rs:428`) currently takes `&AppState` — storing Rects during draw means threading `&mut AppState` (or storing computed areas from a pure function; see task 1).
- **Geometry helpers (already exported, bastion already depends on `bella_engine`):** `point_in` (`../bella/crates/bella-engine/src/geometry.rs:152`), `body_pos` (`geometry.rs:26`), `select_word_at` (`geometry.rs:68`, optional/best-effort only).
- **Selection state to drive:** `selected_spine` + `spine_rows()`/`select_next`/`select_prev` (`src/sessions/app.rs:56–171`), `file_browser: bella_engine::browser::Browser` (`.selected`, `.move_cursor`, invariant `scroll <= selected < scroll + viewport_h`), `space_overview_scroll` (`app.rs:59`), `overview_pane: OverviewPane` (`app.rs:60`).
- **Standing rules that bite:** CLAUDE.md rule 1 (ships with tests) and rule 6 (the dispatcher and pane-area math are pure and exhaustively unit-tested without a terminal; the event-loop wiring is the thin I/O shell, manually smoke-tested with the result recorded in `## Notes`).
- **Out of scope (hard boundary):** the standalone `monitor` subcommand's own event loop (`src/monitor/events.rs`); word/link selection in the content pane (optional/best-effort — skip unless free); the sub-tab bar (BA.13.4); session→space cwd mapping (BA.13.3).

## Step-by-Step Tasks

See `tasks.json` in this directory — the task list is defined there, not here.

## Acceptance Criteria

- Per-pane viewport `Rect`s (`spine_area`, `browser_area`, `content_area`, `agent_panel_area`) are computed by a pure function mirroring the draw layout, stored on `AppState` during draw, and unit-tested against known frame sizes.
- Clicking a spine row selects it (accounting for the list block's border and scroll offset, clamped to `spine_rows()` length).
- Clicking a file-browser entry sets `file_browser.selected` (accounting for border + `Browser.scroll`, clamped to entries length) and moves focus to the Browser pane.
- Clicking an agent-panel row selects the spine row whose Space slug equals that session's name; a session with no matching space is a no-op.
- `ScrollUp`/`ScrollDown` route by hover pane: content → `space_overview_scroll` ± (saturating), browser → `move_cursor`, spine → `select_prev`/`select_next`.
- `on_mouse` is a pure method unit-tested per pane with synthetic `Rect`s + coordinates yielding the asserted state/`Action` — no terminal involved. Clicks outside every stored pane and clicks before the first draw (unset/default areas) are no-ops.
- The event-loop `Event::Mouse` arm feeds `on_mouse` and handles its returned `Action` through the same path as key events; manual smoke test of real click/scroll recorded in `## Notes`.
- No sub-tab routing is introduced; the dispatcher shape leaves an obvious seam for BA.13.4.
- All gated checks pass (`cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`).

## Validation Commands

```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes

- **Task 3 — event-loop wiring:** `run_inner`'s event read (`src/sessions/ui.rs`) now matches
  `Event::Key`/`Event::Mouse`/other, feeding both `app.on_key`/`app.on_mouse` results through the
  same `Action` handling path (the `Attach` suspend/resume branch, then `execute_action`) — no
  duplicated action plumbing. Mouse capture enable/disable remains symmetric: enabled at
  `run()` entry (ui.rs `EnterAlternateScreen, event::EnableMouseCapture`) and the `Attach`
  suspend/resume pair, disabled at the `Attach` suspend point and at `run()`'s always-run teardown
  — unchanged by this task, verified by re-reading both call sites.
- **Manual smoke test (tmux SGR mouse injection):** launched `./target/release/bastion tui` (release
  build) inside a scratch tmux session (`bastion-mouse-smoke`, ~100x30) alongside four other live
  tmux sessions (`orchestration`, `Core`, `smoke-a`, `smoke-b`, and later `mev`). Sent raw SGR mouse
  escape sequences (`\x1b[<Cb;Cx;CyM`/`m`) via `tmux load-buffer`/`paste-buffer` and inspected the
  result with `tmux capture-pane`:
  - **Spine click:** clicked the "core" tier row — file browser and content pane updated to the
    "core" tier's files/status (spine selection changed).
  - **File-browser click:** clicked a specific entry ("docs") — the `>>` cursor moved to that row.
  - **Browser wheel scroll:** `ScrollDown` over the browser pane moved the `>>` cursor down one
    entry (from `..` to `_planning`), confirming `move_cursor` routing.
  - **Agent-panel click, no match:** clicked the "bastion-mouse-smoke" row (no space with that
    slug) — correctly a no-op for spine selection (only the legacy in-view session detail panel,
    driven by a separate pre-existing field, showed that session — unrelated to this block's
    dispatcher).
  - **Agent-panel click, match:** clicked the "mev" row (a session name matching the `mev` project
    slug in the spine tree) — spine navigated to the `mev` space and content pane rendered `mev`'s
    `status.md`, confirming the slug-equality jump.
  - **Content wheel scroll:** sent `ScrollDown` over the content pane; no visible change because
    the rendered content was shorter than the viewport (saturating scroll had nothing to reveal) —
    inconclusive by observation alone, but the underlying `space_overview_scroll` arithmetic is
    exhaustively unit-tested in task 2.
  - Key events (`q` to quit) continued to work interleaved with mouse events, confirming the merged
    match arm didn't regress key handling.
  All scratch tmux sessions were torn down after the test.

## Amendment Log

<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
