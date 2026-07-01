# Task Spec — Phase 12, Block E (Merge sessions into Mission Control)

**Status:** Not started · **Last run:** never

## Goal
Merge tmux sessions and orchestrator workflow runs into a single Mission Control list, so selecting either shows the right detail in the right-hand pane — implementing the "Session Drop-In" concept from `initial-plan.md` §3 that the shipped console never absorbed.

## Context Pointers
- `planning/bastion-tui-improvements/plan.md` → Block BA.12.E
- `planning/bastion-tui-improvements/initial-plan.md` §3 (the original "Session Drop-In" design)
- `src/monitor/app.rs` (`App` struct, currently `runs: Vec<WorkflowRun>` + `selected_run`/`selected_node`)
- `src/monitor/ui.rs` (`render_graph_pane`, `render_detail_pane`)
- `src/sessions/model.rs` (`Session` struct — reuse as-is, no changes needed)
- `src/sessions/app.rs` (`AppState.sessions: Vec<Session>`, poll/refresh loop, `on_key` attach/new/send/kill handlers)
- `src/ui_theme.rs` (`state_working_style()`, `state_blocked_style()`, `state_idle_style()` — reuse for session dots)
- `CLAUDE.md` (pure-logic-is-tested rule — the merge/ordering function must be a standalone, unit-tested pure function)

## Step-by-Step Tasks

### BA.12.E.1 `MissionItem` enum + pure merge/ordering function
- **Target Files:** `src/monitor/app.rs`
- Add `pub enum MissionItem { Session(crate::sessions::model::Session), Run(WorkflowRun) }`.
- Add `pub fn build_mission_items(sessions: &[Session], runs: &[WorkflowRun]) -> Vec<MissionItem>` as a standalone pure function: order needs-action sessions first, then running items, then idle/success items.
- Add `pub items: Vec<MissionItem>` to `App`, replacing `selected_run`/`selected_node` with a single `pub selected: usize` for top-level list selection (retain a separate node-drill-down index only if still needed for in-run detail).
- Unit test `build_mission_items()` covering: empty inputs, sessions-only, runs-only, and a mixed case asserting the expected ordering.

### BA.12.E.2 Unified list + detail-pane rendering
- **Target Files:** `src/monitor/ui.rs`
- *(Depends on BA.12.E.1)*
- Update `render_graph_pane` to render `app.items` as a themed list (sessions section then runs section, or interleaved per the merge order — keep the grouping simple and legible).
- Update `render_detail_pane` to branch on the selected `MissionItem`: for `Session`, render `name`, `agent_state` (styled dot via `ui_theme::state_working_style()`/`state_blocked_style()`/`state_idle_style()`), `foreground_cmd`, `last_line`; for `Run`, keep the existing node/timing/token detail rendering unchanged.
- Manually smoke test via `tmux capture-pane` with at least one live session and one live/mock run, and record the result in this spec's `## Notes`.

### BA.12.E.3 Wire sessions into Mission Control + keep keybindings working
- **Target Files:** `src/sessions/app.rs`
- *(Depends on BA.12.E.1)*
- In the poll/refresh loop that already updates `monitor_app.runs`, also call `build_mission_items(&self.sessions, &self.monitor_app.runs)` and assign the result to `monitor_app.items`.
- Update the existing `a`/`n`/`s`/`k` (attach/new/send/kill) key handlers so they act on the currently-selected `MissionItem::Session` in `monitor_app.items` when Mission Control is the active tab, instead of only working against the old flat sidebar selection.

### BA.12.E.4 Validate
- Run the Validation Commands listed below and confirm all pass.

## Acceptance Criteria
- `build_mission_items()` has unit tests covering empty, sessions-only, runs-only, and mixed-ordering cases.
- Selecting a session in Mission Control's list shows session detail (name, agent state, foreground command, last output line) in the right pane; selecting a run shows the existing run/node detail.
- `a`/`n`/`s`/`k` keybindings work correctly when a session item is selected in Mission Control.
- Manual `tmux capture-pane` smoke test result recorded in `## Notes`.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
_None yet._

## Amendment Log
_No amendments yet._
