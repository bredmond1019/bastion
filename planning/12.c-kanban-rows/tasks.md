# Task Spec — Phase 12, Block C (Kanban board: rows instead of columns)

**Status:** Not started · **Last run:** never

## Goal
Swap the Kanban board's 3-column side-by-side layout to 3 horizontal rows so task text stops wrapping awkwardly.

## Context Pointers
- `planning/bastion-tui-improvements/plan.md` → Block BA.12.C
- `src/overview/mod.rs` (Kanban board render function, ~line 104-111 for the `columns` `Layout`)
- `CLAUDE.md` (coverage-bar rule — this is a pure rendering change with no new pure-logic surface)

## Step-by-Step Tasks

### BA.12.C.1 Swap Kanban layout direction to rows
- **Target Files:** `src/overview/mod.rs`
- Change the `columns` `Layout` split from `Direction::Horizontal` to `Direction::Vertical`, keeping the existing `Constraint::Percentage(33)/Percentage(33)/Percentage(34)` (now row heights instead of column widths).
- Leave the `List` widget construction and `ListItem` building (id span + title span + blank separator) for `In Progress` / `Up Next` / `Blocked` unchanged.
- Manually smoke test via `tmux capture-pane` that the three lanes now render as stacked horizontal rows and record the result in this spec's `## Notes`.

### BA.12.C.2 Validate
- Run the Validation Commands listed below and confirm all pass.

## Acceptance Criteria
- The Kanban board's `In Progress` / `Up Next` / `Blocked` lanes render as three stacked horizontal rows, not side-by-side columns.
- No other rendering logic in `src/overview/mod.rs` (task data parsing, list item construction) changed.
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
