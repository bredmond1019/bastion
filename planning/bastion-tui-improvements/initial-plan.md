---
type: Plan
title: BA.12.A Unified Operator Console — Initial Plan
description: Formal execution plan for evolving the Bastion TUI into a unified, markdown-native operator console. Incorporates Herdr-style layout concepts while maintaining strict alignment with the Bastion Mission (Agent Pager, Dual-Graph, Native AST).
doc_id: initial-plan
layer: [console, surface]
project: bastion
status: active
keywords: [tui, plan, layout, herdr, bella, mission-control, BA.12.A]
related: [notes, bastion-mission-notes, herdr-bella-console-research]
---

# BA.12.A Unified Operator Console — Initial Plan

## 1. Strategic Objective & Differentiation
The goal of BA.12.A is to evolve Bastion from a collection of isolated CLI commands into a unified, IDE-like operator console. While we draw layout inspiration from tools like Herdr, Bastion differentiates itself fundamentally:
1. **Desktop Control Plane for Mobile:** The state and event loop powering this console is exactly what will be broadcast over WebSockets to the BastionUI Android app (The Agent Pager).
2. **Markdown as Native AST:** We do not just stream raw PTY ANSI logs. We use `bella-engine` to treat markdown as the underlying abstract syntax tree, enabling interactive elements (links, checkboxes) directly in the console.
3. **Execution Trace vs. Window Manager:** We are visualizing the Python Orchestrator's DAG logic, not just arranging terminal windows.

## 2. Core Layout Architecture

The console will utilize a `compute_view() -> render()` separation pattern. This computes layout constraints (sidebar widths, tab boundaries) prior to rendering, ensuring butter-smooth performance and precise mouse hit-testing.

*   **Collapsible Left Sidebar:**
    *   **Spaces:** Context-switchers (HQ, Core, Bastion, Orchestrator).
    *   **Agents:** Live agent list (Claude, Pi) populated by the TOML-manifest state engine (`idle`, `working`, `blocked`).
*   **Middle Sidebar (Optional/Contextual):**
    *   A directory tree or block list powered by `bella-engine` for navigating markdown structures within the active Space.
*   **Main Content Area (Dynamic IDE-Style Tabs):**
    *   Unlike a static dashboard, tabs are managed dynamically as a collection of views.
    *   **Pinned Tab 1: Space Overview (The Bella Workspace):** Rich markdown rendering of `status.md` and `master-plan.md`. Architected for future interactivity (e.g., clicking a checkbox mutates the actual markdown file).
    *   **Pinned Tab 2: Mission Control Center:** The primary observability and control pane.
    *   **Dynamic Tabs:** Operators can open custom tabs (e.g., clicking a file in the Directory Tree opens a new `bella-engine` markdown tab, or spawning a "Costs Dashboard" tab). Dynamic tabs can be closed via a `[x]` indicator or `Ctrl+w`.

## 3. Mission Control Center (Tab 2)

This tab unifies the workflow DAG observability (`bastion monitor`) and tmux process control (`bastion sessions`).

### The Hierarchical Execution Tree
Instead of a tangled 2D map, the workflow DAG is represented as a clean, expanding execution trace (a hierarchical tree) using Unicode box-drawing characters.
*   **Visuals:** Color-coded status icons (Green `[✔]`, Red `[✘]`, Yellow `[⚠]`).
*   **Interactivity:** Mouse-collapsible loop iterations and nodes.
*   **Inspection:** Clicking a node populates a side-panel with the exact payload inputs, outputs, and token telemetry for that specific block.

### The Session "Drop-In" UX (Option A)
When an operator needs to intervene in a live agent session:
*   **Interaction:** Double-clicking an agent session in the sidebar suspends the Bastion TUI and executes a native `tmux attach`.
*   **UX Enhancements:** Before dropping in, Bastion clears the screen and prints a high-visibility instruction banner. Furthermore, we will inject a custom, temporary `tmux` status bar that prominently displays the detachment keybinding (e.g., `[ BASTION: Press Ctrl-b d to Return ]`).
*   **Return:** Upon `Ctrl-b d`, the operator detaches from tmux, and Bastion instantly redraws the Mission Control state.

## 4. Execution Sequence

### Step 1: Layout Scaffolding & Dynamic Tab Engine
*   Refactor the entry point to support the `compute_view() -> render()` pipeline.
*   Scaffold the Collapsible Sidebar and Main Area.
*   Implement the Dynamic Tab Engine (managing state as a list of open tabs rather than a static enum), allowing tabs to be spawned, focused, and closed. Initialize it with Pinned Tab 1 and Tab 2 placeholders.
*   Ensure the event loop can simultaneously poll the orchestrator database and read local session states without blocking.

### Step 2: Mouse Enablement
*   Enable `crossterm::event::EnableMouseCapture`.
*   Implement the `map_mouse() -> Action -> apply()` pure-state pattern.
*   Add basic hit-testing: click-to-select tabs, click-to-expand sidebar items.

### Step 3: Tab 2 — Mission Control & Hierarchical Tree
*   Port the existing DAG rendering logic into a strict, indented hierarchical execution tree using Ratatui list/tree primitives.
*   Implement the "Drop-In" `tmux attach` suspension logic with the custom UX banners/status-bar overrides.

### Step 4: Tab 1 — Bella Integration (Native AST)
*   Integrate `bella-engine` as a path dependency.
*   Wire the Space Overview tab to load and render `status.md` (and eventually `master-plan.md` waves).
*   Ensure the architecture supports sending `Action` events back to the file system (e.g., for future checkbox mutability).

### Step 5: Agent State Manifest Engine
*   Build the TOML-based PTY parsing engine.
*   Update the Sidebar "Agents" list to display accurate `idle`/`working`/`blocked` states, replacing naive connection checks.
