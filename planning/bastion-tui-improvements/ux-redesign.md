---
type: Plan
title: Bastion TUI UX Redesign & Fixes (Phase 13)
description: Plan to address mouse interactivity, space tree selection semantics, tier-level visibility, and overall tab coupling.
doc_id: bastion-tui-ux-redesign
layer: [console]
project: bastion
status: draft
related: [bastion-tui-improvements, initial-plan]
---

# Bastion TUI UX Redesign & Fixes (Phase 13)

Based on recent feedback, the unified console's UX needs structural adjustments to better align with the operator's mental model, as well as several functional enhancements.

## 1. UX Disconnect: Tab Coupling
**Problem:** `Space Overview`, `Kanban Board`, and `Mission Control` currently sit as three equal sibling tabs. However, `Space Overview` and `Kanban Board` are contextually bound to the selected space in the left sidebar, while `Mission Control` is global (showing all tmux sessions and the orchestrator DAG). This creates cognitive friction.
**Solution:**
- Consolidate the top-level tabs to just **Space** and **Mission Control**.
- Within the **Space** tab, introduce sub-navigation (e.g., a segmented control or sub-tabs) to toggle between the **Overview** (file browser + markdown preview) and the **Kanban Board** for that specific space. 
- The sidebar remains visible across both sub-views of the Space tab.

## 2. Space Tree: Selectable Tiers & "HQ"
**Problem:** Tier headers (`_root`, `core`, `side`) are currently unselectable labels. Furthermore, the `brain` repo is technically just the `_root` directory.
**Solution:**
- Rename the `_root` tier to **`HQ`** in the UI.
- Make tier headers **selectable**. Selecting a tier header (like `core`) should point the Space Overview to `core/planning/status.md` and load the tier-level `core/planning/state.json` Kanban data.
- Remove the redundant `brain` entry. Selecting `HQ` itself will act as selecting the root of the project (`.`), effectively replacing `brain`.

## 3. File Browser: Filtering redundant sub-brains
**Problem:** When selecting the root (HQ), the file browser lists directories like `core`, `side`, and `client`. Since these are already explicitly listed in the spaces sidebar, showing them in the file browser is redundant noise.
**Solution:**
- Add an ignore/filter rule to the `bella_engine::browser` (or the Bastion integration layer) to hide directories that correspond to tier headers when browsing the root space.

## 4. Mouse Interactivity
**Problem:** The TUI lost the extensive mouse support that was present in `bella` (clicking files, scrolling lists). Currently, only the top tabs respond to clicks.
**Solution:**
- Elevate this to a high-priority block.
- Wire `crossterm` mouse events down the widget tree:
  - Sidebar: Click to select a space.
  - File Browser: Click to select/descend; scroll wheel to scroll the list.
  - Content Pane: Scroll wheel to scroll the markdown.
  - Kanban Board: Scroll wheel to scroll columns.

## Tasks / Next Steps
We will stage these as the next phase of TUI improvements (e.g., Phase 13). 
1. **[BA.13.A]** Refactor tab routing (Space vs Mission Control) and embed Kanban as a sub-view.
2. **[BA.13.B]** Refactor `SpaceTree` to allow tier selection, rename `_root` to `HQ`, and drop `brain`.
3. **[BA.13.C]** Implement directory filtering in the file browser for tier directories.
4. **[BA.13.D]** Wire extensive mouse support across all panes.
