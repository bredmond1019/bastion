# Task Spec — Phase 12, Block G

**Status:** Not started · **Last run:** never

## Goal
Promote bella's existing file browser into `bella-engine` as a shared module, then wire it into bastion's Space Overview tab scoped to the selected space with native Ratatui scrolling support.

## Context Pointers
- `planning/bastion-tui-improvements/plan.md` (Block BA.12.G)
- `CLAUDE.md` (Standing rules, testing requirements)

## Step-by-Step Tasks

### BA.12.G.1 Promote file browser into bella-engine
- Add `ignore` crate dependency to `core/bella/crates/bella-engine/Cargo.toml`.
- Move `core/bella/crates/bella/src/browser.rs` to `core/bella/crates/bella-engine/src/browser.rs` and make types public (`Browser`, `BrowserEntry`, `BrowserEntryKind`).
- Decouple `Browser` from `App` so it returns `Option<PathBuf>` on selection.
- Add an optional `root_boundary: Option<PathBuf>` field to `Browser` to prevent `ascend_target()` from navigating above a given path.
- Export `pub mod browser;` in `core/bella/crates/bella-engine/src/lib.rs`.
- Add a unit test in `bella-engine` confirming `ascend_target()` returns `None` (or is blocked) at the root boundary.

### BA.12.G.2 Refactor bella to consume shared browser
- Delete `core/bella/crates/bella/src/browser.rs` and its `mod browser;` declaration.
- Update `core/bella/crates/bella/src/app.rs`, `events.rs`, and `ui.rs` to consume `bella_engine::browser::Browser`.
- Ensure bella's file-browser UX remains unchanged.

### BA.12.G.3 Integrate file browser into bastion AppState
- Modify `src/sessions/app.rs` to add `pub file_browser: bella_engine::browser::Browser` and `pub space_overview_scroll: u16` to `AppState`.
- Re-instantiate `file_browser` when `selected_space` changes, rooted at that space's `repo_path` with the `root_boundary` set to the same path.
- Map browser navigation keys (e.g. up, down, enter, back) to `file_browser` methods when Space Overview is focused.
- Modify `src/sessions/app.rs` to load the selected markdown file into the content pane (replacing `planning/status.md`); add a distinct key to open it as a new tab via `TabState::MarkdownDocument(PathBuf)`.

### BA.12.G.4 Render file browser and scrollable content in bastion UI
- Modify `src/sessions/ui.rs` to render the file-browser panel alongside the content pane in the Space Overview tab.
- Add native Ratatui scrolling to the content pane `Paragraph` using `.scroll((space_overview_scroll, 0))`.
- Handle `PageUp`/`PageDown`/`j`/`k` in `app.rs` to adjust and clamp `space_overview_scroll` to the rendered line count.

### BA.12.G.5 Validate
- Run the Validation Commands listed below and confirm all pass for both `bella` and `bastion`.

## Acceptance Criteria
- bella's own gating checks stay green and manual smoke test confirms its file-browser UX is unchanged.
- A unit test in `bella-engine` confirms `ascend_target()` is blocked at the root boundary.
- In bastion: selecting a space roots the file browser at that space's directory and it cannot navigate above it.
- Space Overview's content pane scrolls via `PageUp`/`PageDown`/`j`/`k`, confirmed manually against a long file.
- Selecting a file loads it into the content pane in place; a distinct key opens it as a new tab.
- Both bastion's and bella's gating checks pass.

## Validation Commands
```
cd ../../bella && cargo fmt --check && cargo clippy -- -D warnings && cargo test && cargo build --release
cd ../bastion && cargo fmt --check && cargo clippy -- -D warnings && cargo test && cargo build --release
```

## Notes

## Amendment Log
_No amendments yet._
