---
type: TaskSpec
title: "Task Spec — Phase 13, Block BA.13.0: Spine model + primary navigation"
description: Decompose BA.13.0 into disjoint-ownership tasks that replace the three-tab layout with a spine-only navigator.
doc_id: 13-0-spine-primary-navigation-tasks
layer: [console]
project: bastion
status: active
keywords: [spine, navigation, unified-console, sessions-tui, spine-row, selected-node]
related: [bastion-master-plan]
---

# Task Spec — Phase 13, Block BA.13.0: Spine model + primary navigation

**Status:** Done · **Last run:** 2026-07-02

## Goal
Replace the three-tab layout with a spine-only navigator: `SpineRow` model (`MissionControl` pinned first, `Hq`, selectable `Tier`, `Space`), wrapping selection, and main-area routing on the selected node.

## Context Pointers
- **Plan section:** `planning/master-plan.md` → Phase 13 → *Block BA.13.0 — Spine model + primary navigation* (lines ~1157–1184). Phase preamble (lines ~1141–1156) explains why the spine must become the single primary navigator: Mission Control's session list is built globally (`build_mission_items`, `src/monitor/app.rs`) and top tabs are the wrong abstraction.
- **Repo files (per block's *Files* list):** `src/brain/spaces.rs` (model), `src/sessions/app.rs` (state + selection), `src/sessions/ui.rs` (render + routing).
- **CLAUDE.md standing rules:** Rule 1 (tests ship with every behaviour change), Rule 6 (coverage bar — pure logic exhaustively unit-tested without I/O; the `tmux.rs` construction-vs-execution split is the model). Track is DB-free (D4) and read-only vs the orchestrator (D2).
- **Out of scope (hard boundary, from the block):** the sub-tab bar (BA.13.4), the agent panel (BA.13.1), the theming refactor (BA.14.0), mouse (BA.13.2). Markdown "open" (`t`) becomes a transient full-screen overlay flag here only insofar as needed to remove the tab dependency; overlay polish is deferred.

## Step-by-Step Tasks

### 1. BA.13.0.1 Spine model in `src/brain/spaces.rs`
- **Owns:** `src/brain/spaces.rs` (only file touched by this task).
- Add a `SpineRow` model as a presentation layer over the unchanged `parse_space_tree` output — do **not** modify `parse_space_tree` itself. Row kinds: `MissionControl` (pinned first), `Hq`, `Tier(name)`, `Space(...)`.
- Add a `SelectedNode` enum (the routing target consumed by `app.rs` + `ui.rs`): `MissionControl`, `Hq`, `Tier(name)`, `Space(...)`. Define it here so both downstream tasks share one type.
- Add `spine_rows(&SpaceTree) -> Vec<SpineRow>` producing the ordered, flattened row list: `◆ Mission Control` first, then `HQ` and its children (`learn-ai`/`base-template`), then the `core`/`side`/`client`/`portfolio` tiers with their spaces.
- Rename the `_root` tier → `HQ`; collapse the redundant `brain` leaf into the `HQ` row (data source = brain root `.`); keep `learn-ai`/`base-template` as `HQ` children.
- **Tests (Rule 6):** unit-test `spine_rows()` against a `parse_space_tree` fixture asserting order (Mission Control first), the `HQ` rename, the collapsed-`brain` invariant (no standalone `brain` leaf), and that `learn-ai`/`base-template` appear under `HQ`. Cover an empty/degenerate tree.

### 2. BA.13.0.2 Selection + node routing in `src/sessions/app.rs`
- **Owns:** `src/sessions/app.rs` (only file touched by this task). **Depends on:** BA.13.0.1 (`SpineRow`/`SelectedNode`).
- Replace `selected_space` with `selected_spine` (index into `spine_rows()`); add a derived `selected_node() -> SelectedNode`.
- Rewrite `select_next`/`select_prev` to **wrap over all rows** (drop the header-skip logic) — Mission Control and tier headers are now selectable.
- Update the derived accessors to key off the selected spine node: `reinit_browser`, `current_space_planning_root`, `selected_session`, `selected_space_slug`.
- Remove the tab machinery entirely: `tabs`, `active_tab_index`, `next_tab`, `prev_tab`, `push_tab`, `close_tab`, and the `Tab`/`BackTab` key arms. Where the markdown "open" (`t`) path depended on pushing a tab, replace it with a transient full-screen overlay flag sufficient to drop the tab dependency (overlay polish deferred).
- **Tests (Rule 6):** unit-test `select_next`/`select_prev` wrap-around across the full spine (including wrap at both ends), and `selected_node()` returning the correct `SelectedNode` for Mission Control, `HQ`, a tier header, and a space row.

### 3. BA.13.0.3 Sidebar render + main-area routing in `src/sessions/ui.rs`
- **Owns:** `src/sessions/ui.rs` (only file touched by this task). **Depends on:** BA.13.0.2 (`selected_spine`/`selected_node()`).
- Rewrite `build_sidebar_items` to render the pinned `◆ Mission Control` row plus selectable tier/`HQ` headers and space rows from `spine_rows()`.
- Delete the top tab-bar rendering block.
- Route the main area on `selected_node()` (`SelectedNode`): Mission Control → existing `monitor::ui::render`; Space/Hq → existing Space Overview; Tier → tier overview rooted at `<tier>/planning/status.md` with a graceful empty-state degrade when the file/tier is absent.
- **Tests (Rule 6):** a `draw_for_test` (following the existing sessions TUI test pattern) asserting the new sidebar contains `◆ Mission Control` first, shows selectable `HQ`/`core`/`side`/`client`/`portfolio` headers, no longer renders a top tab bar, and no longer shows a standalone `brain` leaf. Assert the tier empty-state degrade renders without panicking.

### 4. BA.13.0.4 Validate
- Run the Validation Commands listed below and confirm all pass.
- Manually smoke-test the TUI (`cargo run -- sessions` or the unified-console entry) via tmux `capture-pane`: confirm no top tab bar, `◆ Mission Control` selectable at the top of the spine, tier headers selectable, selecting a tier shows its status or empty-state, and the `brain` leaf is gone. Record the result in `## Notes`.

## Acceptance Criteria
- No top tab bar renders anywhere in the unified console.
- `◆ Mission Control` is the first spine row and is selectable.
- Tier headers (`HQ`/`core`/`side`/`client`/`portfolio`) are selectable.
- Selecting a tier shows `<tier>/planning/status.md` or a graceful empty-state (no panic when absent).
- The `brain` leaf no longer appears; `HQ` is its replacement with `learn-ai`/`base-template` beneath it.
- `spine_rows()`, `select_next`/`select_prev` wrap, and `selected_node()` are unit-tested; a `draw_for_test` asserts the new sidebar.
- All gated checks pass.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes

**Task 4 — Validate (2026-07-02).** Ran the full validation suite from the worktree root:
`cargo fmt --check` clean, `cargo clippy -- -D warnings` clean, `cargo test` — 1022 passed / 0
failed / 3 ignored, `cargo build --release` clean.

Manual TUI smoke test via a detached tmux session (`tmux new-session -d ... "cargo run -- tui"`,
driven with `tmux send-keys` + `tmux capture-pane -p`):
- No top tab bar renders anywhere in the frame.
- `◆ Mission Control` is the first spine row, pinned, and selectable — selecting it renders the
  existing Mission Control session list in the main area.
- Tier headers (`HQ`, `core`, `side`, `client`, `portfolio`) are present and selectable in the
  sidebar.
- Selecting the `core` tier header routes the main area to a tier overview panel titled `core`
  rendering `core/planning/status.md` (rollup table + Momentum/Metrics), confirming the
  tier-overview routing path works without panicking.
- No standalone `brain` leaf appears; `HQ` is present with `learn-ai` and `base-template`
  nested beneath it.

All acceptance criteria for this block are satisfied; no regressions observed.

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
2026-07-02 [task 2] Task 2's implementation of `src/sessions/app.rs` was already correct, but the crate-wide validation gate failed because `src/sessions/ui.rs` and `src/sessions/tui_tests.rs` (owned by Task 3, not yet rewritten) still referenced the removed tab API. To unblock the gate, made minimal compile-fixing adaptations to those two Task-3-owned files (dropped the tab-bar render block, routed the main area on `selected_node()`, dropped the dead Kanban path and the out-of-scope mouse handler, and rebuilt the TUI test fixtures on the new `selected_spine`/`selected_node()` API) rather than leaving them broken until Task 3 ran.
