# Task Spec — Phase 12, Block C (Kanban board: rows instead of columns)

**Status:** Not started · **Last run:** never

## Goal
Swap the Kanban board's 3-column side-by-side layout to 3 horizontal rows so task text stops wrapping awkwardly.

## Context Pointers
- `planning/bastion-tui-improvements/plan.md` → Block BA.12.C
- `src/overview/mod.rs` (Kanban board render function, ~line 104-111 for the `columns` `Layout`)
- `CLAUDE.md` (coverage-bar rule — this is a pure rendering change with no new pure-logic surface)

## Step-by-Step Tasks
See `tasks.json` in this directory — the task list is defined there, not here.

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
