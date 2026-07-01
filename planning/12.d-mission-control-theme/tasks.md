# Task Spec — Phase 12, Block D (Mission Control: apply the console theme)

**Status:** Not started · **Last run:** never

## Goal
Replace Mission Control's hardcoded `ratatui::Color` values with the shared `ui_theme` palette already used by Kanban and the tab bar.

## Context Pointers
- `planning/bastion-tui-improvements/plan.md` → Block BA.12.D
- `src/monitor/ui.rs` (`status_color()`, error/banner spans, borders)
- `src/ui_theme.rs` (existing palette: `cyan()`, `sage()`, `rose()`, `muted()`, `border_dim_style()`, `border_active_style()` — reuse, do not add new colors unless nothing fits)
- `CLAUDE.md` (pure-logic-is-tested rule — `status_color()` is a small pure function)

## Step-by-Step Tasks

### BA.12.D.1 Theme `status_color()` and error/banner spans
- **Target Files:** `src/monitor/ui.rs`
- Replace `RunStatus::Running` → `Color::Yellow` with `ui_theme::cyan()`, `Success` → `Color::Green` with `ui_theme::sage()`, `Failed` → `Color::Red` with `ui_theme::rose()`, `Pending` → `Color::DarkGray` with `ui_theme::muted()`.
- Replace error span styling and the banner span's `Color::Red`/`Color::Yellow` usage with `ui_theme::rose()` (reuse; do not introduce a new `warn()` helper unless no existing color fits).
- Remove all remaining `Color::Yellow`/`Color::Green`/`Color::Red`/`Color::DarkGray` literals from this file.
- Add or extend a unit test for `status_color()` asserting each `RunStatus` variant maps to the expected `ui_theme` color function's output.

### BA.12.D.2 Theme Mission Control borders
- **Target Files:** `src/monitor/ui.rs`
- Update the graph pane and detail pane block borders to use `ui_theme::border_dim_style()` (inactive) / `ui_theme::border_active_style()` (active), matching the border treatment already used by the Kanban board.
- Manually smoke test via `tmux capture-pane` that Mission Control now matches the console's color scheme and record the result in this spec's `## Notes`.

### BA.12.D.3 Validate
- Run the Validation Commands listed below and confirm all pass.

## Acceptance Criteria
- `status_color()` has a unit test asserting each `RunStatus` variant maps to the expected `ui_theme` color.
- No `Color::Yellow`/`Color::Green`/`Color::Red`/`Color::DarkGray` literals remain in `src/monitor/ui.rs`.
- Mission Control's borders use the same `ui_theme::border_dim_style()`/`border_active_style()` treatment as Kanban.
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
