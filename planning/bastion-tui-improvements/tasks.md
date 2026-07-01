# Task Spec — Phase 12, Block F

**Status:** Not started · **Last run:** never

## Goal
Replace the flat session sidebar with a Spaces tree sourced from `brain.toml`: tier groups as parent rows, each tier's repos as indented child leaves.

## Context Pointers
- `planning/bastion-tui-improvements/plan.md` (Block BA.12.F)
- `CLAUDE.md` (Standing rules, testing requirements)

## Step-by-Step Tasks

### BA.12.F.1 Create SpaceTree model and parser
- Create `src/brain/spaces.rs` defining `SpaceEntry { slug, tier, repo_path, heading }` and `SpaceTree { tiers }`.
- Implement `pub fn load_space_tree(brain_toml_path: &Path) -> Result<SpaceTree, ...>` to deserialize `[[repos]]` via `toml` and group them by tier in the fixed display order (`_root`, `core`, `side`, `client`, `portfolio`).
- Write unit tests for `load_space_tree` using a fixture TOML string to verify tier grouping logic without filesystem access.
- Register `pub mod spaces;` in `src/brain/mod.rs`.

### BA.12.F.2 Integrate SpaceTree into AppState
- Modify `src/sessions/app.rs` to add `pub space_tree: SpaceTree` and `pub selected_space: usize` to `AppState`.
- Add resolution logic for `brain.toml` using a new `BASTION_BRAIN_TOML` env override, mirroring existing `BASTION_PLANNING_ROOT` logic (checking `src/config.rs` for precedence).
- Initialize the `space_tree` during `App::new()` or the appropriate setup phase.

### BA.12.F.3 Render Spaces tree in UI
- Modify `src/sessions/ui.rs` to replace `build_sidebar_items(&sessions)` with a tree render for the sidebar.
- Render tier headers as styled but unselectable items using `ui_theme::muted()`.
- Render repo leaves as selectable items, styled consistently with the rest of the console.
- Ensure keyboard navigation (`j`/`k` or Up/Down) properly navigates the selectable leaves, skipping headers if necessary.

### BA.12.F.4 Validate
- Run the Validation Commands listed below and confirm all pass.

## Acceptance Criteria
- `load_space_tree()` correctly groups a fixture `brain.toml`-shaped TOML string into tiers in the fixed display order, unit-tested without filesystem access for the grouping logic.
- The sidebar renders the real `brain.toml`'s tiers and repos as an indented tree, confirmed via manual `tmux capture-pane` smoke test.
- Project's gating checks pass.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes

## Amendment Log
_No amendments yet._
