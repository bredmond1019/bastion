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
See `tasks.json` in this directory — the task list is defined there, not here.

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
