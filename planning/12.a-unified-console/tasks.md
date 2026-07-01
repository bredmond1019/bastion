# Task Spec — Phase 12, Block A (Unified Operator Console)

**Status:** Not started · **Last run:** never

## Goal
Evolve the Bastion TUI into a dynamic, markdown-native IDE workspace featuring a collapsible sidebar, dynamic tab engine, and a hierarchical DAG execution tree.

## Context Pointers
- `planning/bastion-tui-improvements/initial-plan.md`
- `planning/bastion-mission/notes.md`
- `CLAUDE.md` (Standing rules, especially pure-logic vs I/O separation)

## Step-by-Step Tasks

### [done] 1. State: Dynamic Tab Engine & Layout Math
- **Target Files:** `src/sessions/app.rs`
- Refactor `AppState` to drop the static tab enum and introduce a dynamic `Vec<TabState>` and `active_tab_index: usize`.
- Define `TabState` enum with `SpaceOverview`, `MissionControl`, and `MarkdownDocument(PathBuf)`.
- Implement a pure `compute_view(&self, area: Rect)` function that calculates and returns the structural `Rect` boundaries for the sidebar and main content area.
- Write unit tests to assert that pushing/closing tabs updates the index correctly and that `compute_view` yields mathematically correct constraints.

### 2. View: TUI Scaffolding & Mouse Events
- **Target Files:** `src/sessions/ui.rs`, `src/sessions/events.rs`
- *(Depends on Task 1)*
- Update `src/sessions/ui.rs` to consume the `Rect` boundaries from `compute_view()` and draw the structural `Block` borders (Collapsible Sidebar, Tab Bar, Main Area).
- Enable `crossterm::event::EnableMouseCapture` in the terminal setup.
- Map mouse click events (x, y) to a new `Action::SelectTab(usize)` state mutation to allow clicking tabs.

### 3. Mission Control: Hierarchical DAG Tree
- **Target Files:** `src/monitor/app.rs`, `src/monitor/ui.rs`
- Port the existing text-jumble DAG rendering into a structured, indented tree format using Ratatui list primitives and box-drawing characters (`├─`, `└─`).
- Apply color-coding based on node execution status (Green for success, Red for failure).
- Ensure this view is correctly nested within the `TabState::MissionControl` render loop.

### 4. Session Control: Drop-In Suspend UX
- **Target Files:** `src/sessions/tmux.rs`
- Implement a `suspend_and_attach(session_name: &str)` execution shell.
- Before issuing `tmux attach -t <name>`, clear the screen and print a styled instruction banner: `[ BASTION ] Attaching to Agent. Press Ctrl-b d to detach and return.`
- Ensure the terminal state (raw mode, alt screen) is safely suspended and flawlessly restored upon detachment.

### 5. View: Bella-Engine Integration (Native AST)
- **Target Files:** `Cargo.toml`, `src/sessions/ui.rs`
- *(Depends on Task 2)*
- Add `bella-engine` as a path dependency in `Cargo.toml`.
- When `TabState::SpaceOverview` is active, instantiate the Bella markdown parser against `planning/status.md` and render its output to the main content `Rect`.

### 6. Logic: Agent State Manifest Engine
- **Target Files:** `src/detect/manifest.rs` (New), `src/sessions/app.rs`
- Create a pure, unit-testable parser that takes raw string output (from `tmux capture-pane`) and matches it against TOML/Regex signatures to yield an `AgentState` enum (`Idle`, `Working`, `Blocked`).
- Update `AppState`'s sidebar list to fetch and display these live states.
- Write unit tests feeding mock terminal output to the parser to ensure 100% classification accuracy.

### 7. Validate
- Run the Validation Commands listed below and confirm all pass.

## Acceptance Criteria
- `cargo test` passes, proving pure layout math, tab state management, and TOML regex parsing operate correctly without I/O.
- The TUI renders a multi-tab layout with a mouse-clickable tab bar.
- `bastion monitor`'s graph is rendered as an indented tree with proper box-drawing characters.
- Suspending to a tmux session successfully restores the Bastion TUI upon detachment.

## Validation Commands
```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
cargo run -- --help
```

## Notes
**2026-07-01**: Renamed SessionApp to AppState and introduced TabState and layout boundaries logic.

## Amendment Log
_No amendments yet._
