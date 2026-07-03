---
type: Note
title: Bastion TUI Improvements — Consolidated Plan
description: Consolidated notes and initial plan for BA.12.A Unified Operator Console, merging concepts from Bastion Workspace Kanban, Herdr-style layout research, and Bella engine integration.
doc_id: bastion-tui-improvements
layer: [console, surface]
project: bastion
status: draft
keywords: [tui, layout, herdr, bella, kanban, workspace, BA.12]
related: [context]
---

# Bastion TUI Improvements — Consolidated Plan

## 1. Vision & Core Layout Concept
The evolution of Bastion involves moving from disparate, keyboard-only commands (`monitor`, `sessions`, `costs`) into a unified, mouse-enabled, IDE-like operator console. This relies on the **Herdr** aesthetic, mapping it to the Agentic Engineering Stack.

### The Unified Layout (Left to Right)
1. **Left Sidebar: Spaces**
   - Represents structural spaces (HQ, Core, Bastion, Orchestrator, etc.).
   - Primary context switcher.
2. **Middle Sidebar: Directory Tree**
   - Powered by `bella-engine` capabilities to drill down into markdown files within the selected Space.
3. **Bottom Left: Agents / Auxiliary View**
   - Live state chips (`idle`, `working`, `blocked`) for active Agents (e.g., Claude, Pi).
   - Driven by TOML-manifest agent state detection (rather than naive tmux checks).
4. **Main Content Area (Tabbed)**
   - **Tab 1: Space Overview** (Workspace Overview + Kanban)
     - Reads D30 scalars (`now`, `next`, `blocked`) from `planning/status.md` and wave tables from `master-plan.md` using `mev` as the data-extraction layer.
     - Automatically refreshes via `notify` watchers (SSE file watcher pattern).
     - Renders rich markdown using `bella-engine`.
   - **Tab 2: Mission Control Center**
     - Unifies `bastion monitor` and `bastion sessions`.
     - Interact with live `tmux` agent sessions, trigger workflows, and view live DAG execution states.
   - **Tab 3: Metrics / Costs / Knowledge**
     - Momentum rollups, budget tracking, and potentially MEV knowledge graph integration.

## 2. Technical Strategy & Integration

### A. The `bella-engine` Integration
- Bring in `bella-engine` as a path dependency. It matches Bastion's `ratatui` (0.30) and `crossterm` (0.29) versions exactly and has zero terminal I/O logic, making it a clean drop-in for rendering markdown in the TUI.

### B. Compute-then-Render Pipeline
- Adopt the `compute_view() -> render()` separation pattern. This prevents UI stuttering when calculating complex 3-column + main area geometry and allows for precise mouse hit-testing.

### C. Mouse Support & Event Routing
- Enable `crossterm`'s `EnableMouseCapture`.
- Use the **Bella mouse pattern**: `map_mouse() -> Action -> apply()`. This extends Bastion's existing pure-state `on_key() -> Action` paradigm safely.

### D. Agent State Detection (Manifest-driven)
- Adopt TOML-based manifests (region, contains, regex, priority) to accurately detect agent states (e.g., Claude explicitly waiting for input vs generating). This replaces hardcoded/naive detection.

### E. Workspace Overview & Kanban (Powered by `mev`)
- The TUI doesn't need to parse markdown manually; it invokes `mev` to scan `status.md` and `master-plan.md` to feed the TUI views (Kanban columns, progress bars, dependency badges).

## 3. Initial Plan for BA.12.A (Unified Operator Console)

Since **BA.12.B** (Standalone Kanban pane) has already shipped, our next phase is **BA.12.A** (Scaffolding the multi-pane grid and Bella integration). 

### Phase 1: Layout Scaffolding & Compute-then-Render
- Restructure `src/sessions/ui.rs` (or a new unified entry point like `src/console/ui.rs`) into the Herdr-style multi-pane grid.
- Implement the `compute_view() -> render()` split. Use placeholder text for Spaces, Directory Tree, and the Main Tab area.

### Phase 2: Bella Integration (Tab 1 & Directory Tree)
- Add `bella-engine` as a path dependency.
- Wire up the Middle Sidebar (Directory Tree) to list markdown files for the selected Space.
- Wire up Tab 1 to render the loaded markdown files using `bella-engine`.

### Phase 3: Mouse Enablement
- Add `EnableMouseCapture` to terminal initialization.
- Implement Bella's `map_mouse() -> Action -> apply()` for the console.
- Add basic click-to-focus for panes, clicking to select spaces/files, and clicking to switch tabs.

### Phase 4: Mission Control Port (Tab 2)
- Port the existing `monitor` live graph and `sessions` views into Tab 2, placing them side-by-side or stacked as designed.
- Ensure the event loop handles data updates for both tracks correctly.

### Phase 5: Agent State Manifests (Sidebar Chips)
- Build the TOML manifest parser for agent state detection (Claude, Pi).
- Update the Bottom Left (Agents) sidebar to reflect accurate states based on PTY analysis.
