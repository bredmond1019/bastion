---
type: TaskSpec
title: "Task Spec — Phase 13, Block BA.13.1: Persistent global agent panel"
description: Decompose BA.13.1 into disjoint-ownership tasks adding an always-visible cross-space agents·priority strip under every SelectedNode.
doc_id: 13-1-persistent-agent-panel-tasks
layer: [console]
project: bastion
status: active
keywords: [agent-panel, session-urgency, fleet-status, unified-console, agentstate, sessions-tui]
related: [bastion-master-plan, 13-0-spine-primary-navigation-tasks, 14-0-config-driven-theme-tasks]
---

# Task Spec — Phase 13, Block BA.13.1: Persistent global agent panel

**Status:** Not started · **Last run:** never

## Goal
Add an always-visible bottom "agents · priority" strip listing every tmux session across all spaces with its detected `AgentState`, sorted by urgency (Blocked/needs-input first), rendered under every `SelectedNode`.

## Context Pointers
- **Plan section:** `planning/master-plan.md` → Phase 13 → *Block BA.13.1 — Persistent global agent panel* (lines ~1186–1208). Delivers the user's hard requirement: full cross-space transparency of session state while focused on one space (Herdr-grade fleet-status), reusing the existing `detect/` engine.
- **Repo files (per block's *Files* list):** `src/monitor/app.rs` (extract pure `session_urgency`; reuse in `build_mission_items` at line ~14), new `src/sessions/agent_panel.rs` builder (+ `mod` declaration in `src/sessions/mod.rs`), `src/sessions/ui.rs` (reserve the always-on bottom strip + render).
- **Interfaces / shared surface:** consumes `detect_state` output already applied in `poll_sessions`; consumes the runtime theme from BA.14.0 (`ui_theme::current_theme()`) for its colors — **not** literals. `build_mission_items` signature is shared by `monitor/events.rs` and must be preserved.
- **CLAUDE.md standing rules:** Rule 1 (tests ship with every behaviour change), Rule 6 (pure logic — `session_urgency`, `agent_panel_rows` — exhaustively unit-tested without I/O; keep the render shell thin). Track is DB-free (D4) and read-only vs the orchestrator (D2). Both dependencies (BA.13.0 spine, BA.14.0 theme) are already merged.
- **Out of scope (hard boundary, from the block):** click-to-jump on panel rows (BA.13.2 mouse); grouping rows by space (uses `session_urgency` ordering now; cwd-based grouping refines after BA.13.3).

## Step-by-Step Tasks

### 1. BA.13.1.1 Extract pure `session_urgency` in `src/monitor/app.rs` (in progress)
- **Owns:** `src/monitor/app.rs` (only file touched by this task).
- Extract the urgency ordering currently inline in `build_mission_items` into a pure `session_urgency(&Session) -> u8` (lower value = higher urgency, Blocked/needs-input first), and reuse it inside `build_mission_items`.
- **Preserve the `build_mission_items` signature** (`build_mission_items(sessions: &[Session], runs: &[WorkflowRun]) -> Vec<MissionItem>`) — it is shared by `monitor/events.rs`. This is a refactor with no behaviour change to Mission Control's output.
- **Tests (Rule 6):** unit-test `session_urgency` for all four `AgentState` values (Working/Blocked/Idle/Unknown) **plus Running**, asserting Blocked sorts above Working above Idle; add/keep a regression assertion that `build_mission_items` ordering is unchanged.

### 2. BA.13.1.2 Pure `agent_panel_rows` builder in `src/sessions/agent_panel.rs`
- **Owns:** new `src/sessions/agent_panel.rs` + `mod` declaration line in `src/sessions/mod.rs`. **Depends on:** BA.13.1.1 (`session_urgency`).
- Add an `AgentPanelRow` model (session label + detected `AgentState` + whatever the render needs) and a pure `agent_panel_rows(&[Session]) -> Vec<AgentPanelRow>` that maps every session and sorts by `session_urgency` (Blocked/needs-input first). No I/O, no theme access in this pure builder — rows carry state, colors are applied at render time.
- Register the module with `pub mod agent_panel;` in `src/sessions/mod.rs`.
- **Tests (Rule 6):** unit-test `agent_panel_rows` produces one row per session, sorted so a Blocked session precedes a Working precedes an Idle; cover the empty-slice case.

### 3. BA.13.1.3 Reserve + render the bottom strip in `src/sessions/ui.rs`
- **Owns:** `src/sessions/ui.rs` (only file touched by this task). **Depends on:** BA.13.1.2 (`agent_panel_rows`/`AgentPanelRow`) and BA.13.1.1 (`session_urgency`).
- Reserve an always-on bottom strip in the top-level vertical split so it renders under **every** `SelectedNode` (Mission Control / HQ / Tier / Space), with a min-height fallback when vertical space is tight.
- Render `agent_panel_rows` with themed state dots/colors sourced from the runtime theme (`ui_theme::current_theme()` — BA.14.0), never literal colors.
- **Tests:** a `draw_for_test` (existing `tui_tests.rs` pattern) asserting the strip renders under at least two different `SelectedNode` selections, that rows appear in urgency order, and that the min-height fallback renders without panic when the frame is short.

### 4. BA.13.1.4 Validate
- Run the Validation Commands listed below and confirm all pass.
- Manually smoke-test the TUI via tmux `capture-pane`: with multiple tmux sessions in differing states, confirm the agents·priority strip is visible under Mission Control, a tier, and a space; Blocked sorts first; colors track the active theme. Record the result in `## Notes`.

## Acceptance Criteria
- The panel renders under every `SelectedNode` (unit + `draw_for_test`).
- A Blocked session sorts above a Working one above an Idle one; `session_urgency` is unit-tested for all four `AgentState` values plus Running.
- `build_mission_items` signature is preserved (shared by `monitor/events.rs`) and its ordering is unchanged.
- Panel colors come from the runtime theme, not literals.
- A min-height fallback keeps the strip rendering without panic when vertical space is tight.
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
