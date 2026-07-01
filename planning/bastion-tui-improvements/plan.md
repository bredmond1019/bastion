---
type: Plan
title: Unified Console Follow-up Cleanup Plan
description: Mini-roadmap for the BA.12 unified console follow-up cleanup — Kanban rows, Mission Control theming, Spaces/session redesign, and a shared file browser reused from bella.
---

# Unified Console Follow-up Cleanup — Plan

*Mini-roadmap. Created 2026-07-01. Ad-hoc: not in `planning/master-plan.md` (Phase 12 — the
unified console — was built ad-hoc under `BA.12.A`/`BA.12.B`; this plan continues that phase as
`BA.12.C`–`BA.12.G`). See `planning/decisions/D34-adhoc-planning-seam.md` and
`planning/bastion-tui-improvements/initial-plan.md` (the original BA.12.A vision this plan closes
gaps against).*

## The Goal, Stated Plainly

BA.12.A/B shipped the unified TUI console (Kanban board, Mission Control DAG view, Space
Overview, tmux sidebar), but it landed short of the original `initial-plan.md` vision in five
ways: the Kanban board's 3 columns are hard to read side-by-side, Mission Control never picked up
the console's color theme, the sidebar conflates "Spaces" (the brain/project hierarchy the
original plan called for) with the flat tmux session list, Mission Control never absorbed session
control the way the original "Session Drop-In" design intended, and Space Overview can only
render one hardcoded file with no scrolling or file browsing. This plan closes all five gaps in
one continuous pass, landing as five ordered, independently reviewable blocks.

## The Destination

- The Kanban board's three lanes (`In Progress`/`Up Next`/`Blocked`) render as full-width
  horizontal rows, not cramped side-by-side columns.
- Mission Control uses the same `ui_theme` palette as the rest of the console instead of
  hardcoded ANSI colors.
- Mission Control's left pane is a single unified list of active work — tmux sessions *and*
  orchestrator workflow runs — with a right-hand detail pane that shows session or run detail
  depending on what's selected. Session keybindings (attach/new/send/kill) keep working from
  there.
- The left sidebar shows **Spaces**: the `brain.toml` tier hierarchy (`HQ`/`core`/`side`/`client`/
  `portfolio`) with each tier's projects nested underneath, replacing the flat session list that
  moved into Mission Control.
- Space Overview is scrollable and has a file-browser panel — reused from bella's existing
  directory browser (promoted into `bella-engine` so both apps share it, rather than rebuilt from
  scratch) — scoped to whichever space is selected. Selecting a file loads it into the pane;
  opening it as a new tab uses the already-defined `TabState::MarkdownDocument` variant.

## Architecture / Design Overview

- **Blocks C and D are independent, low-risk theming/layout fixes** — no state model changes,
  land first.
- **Block E (session/Mission-Control merge)** introduces a `MissionItem` enum
  (`Session`/`Run`) so the left pane's selection model treats both kinds uniformly, replacing two
  parallel selection indices with one.
- **Block F (Spaces tree)** adds a new `src/brain/spaces.rs` module that parses `brain.toml`
  (already a `toml`-crate dependency) into a tier-grouped tree, and repurposes the sidebar that
  Block E just freed up from session duty.
- **Block G (file browser)** is a two-repo block: it first promotes bella's existing
  `crates/bella/src/browser.rs` (`Browser`/`BrowserEntry`/`BrowserEntryKind` + `ignore`-crate
  walk + `move_cursor`/`descend`/`ascend_target`) out of bella's private binary crate into the
  already-shared `bella-engine` library (bastion already path-depends on `bella-engine` for
  markdown rendering), decoupling it from bella's own `App` and adding an optional root-boundary
  so a caller can prevent navigating above a given directory. bella's own binary is updated to
  consume the promoted module (removing the duplicate). Bastion then instantiates it rooted at
  the selected space's directory, scoped by that same root boundary — matching the request to
  reuse bella's existing feature instead of rebuilding a new file tree.
- Depends on Block F's `space_tree`/`selected_space` state to know which directory to root the
  browser in, so **G must land after F**.

---

## The Block Contract

`/generate-tasks` reads **only the target block's section** below. Every block is self-sufficient
and uses the same skeleton:

- **What** — the scope, in implementation terms.
- **Why** — the motivation (keeps the generator from over- or under-scoping).
- **Files** — *new* vs *modified*, named by path. Load-bearing: tasks sharing a file must be
  serialized (`dependsOn`) or append-only; tasks owning distinct files may run in parallel.
- **Interfaces / shared surface** *(optional)* — shared exports/APIs consumed or added.
- **Out of scope** — explicit boundaries; what belongs to a later block or a different effort.
- **Acceptance criteria** — true/false conditions checkable against the diff, ending with the
  project's gating checks passing.

---

## Phase 12 — Unified Console Cleanup (continuation)

### Block BA.12.C — Kanban board: rows instead of columns
- **What:** Swap the Kanban board's 3-column side-by-side layout to 3 horizontal rows so task
  text stops wrapping awkwardly.
- **Why:** The user reported the current side-by-side columns make task titles hard to read.
- **Files:**
  - *Modified* `src/overview/mod.rs` — change the `columns` `Layout` (~line 104-111) from
    `Direction::Horizontal` to `Direction::Vertical`, keeping the existing `Percentage(33/33/34)`
    constraints (now row heights). `List` widget construction and `ListItem` building
    (id span + title span + blank separator) are unchanged.
- **Out of scope:** No changes to task data, `StateJson`/`Focus`/`BlockTask` parsing, or column
  content/styling.
- **Acceptance criteria:**
  - The three lanes render as stacked horizontal rows, not side-by-side columns, confirmed via
    manual `tmux capture-pane` smoke test.
  - Project's gating checks pass (see `planning/harness.json`).

---

### Block BA.12.D — Mission Control: apply the console theme
- **What:** Replace Mission Control's hardcoded `ratatui::Color` values with the shared
  `ui_theme` palette already used by Kanban and the tab bar.
- **Why:** Mission Control is the only console tab still rendering in plain gray/hardcoded ANSI
  colors, inconsistent with the rest of the console.
- **Files:**
  - *Modified* `src/monitor/ui.rs` — `status_color()` maps `RunStatus::Running` →
    `ui_theme::cyan()`, `Success` → `ui_theme::sage()`, `Failed` → `ui_theme::rose()`, `Pending` →
    `ui_theme::muted()`; error spans and the banner span use `ui_theme::rose()`; borders use
    `ui_theme::border_dim_style()` / `border_active_style()` matching Kanban's border treatment.
- **Out of scope:** No layout changes to `render_graph_pane`/`render_detail_pane` (that's Block
  BA.12.E); no new colors added to `ui_theme.rs` — reuse existing palette functions.
- **Acceptance criteria:**
  - `status_color()` has a unit test asserting each `RunStatus` variant maps to the expected
    `ui_theme` color.
  - No `Color::Yellow`/`Color::Green`/`Color::Red`/`Color::DarkGray` literals remain in
    `src/monitor/ui.rs`.
  - Manual `tmux capture-pane` smoke test confirms Mission Control now matches the console's
    color scheme.
  - Project's gating checks pass.

---

### Block BA.12.E — Merge sessions into Mission Control
- **What:** Mission Control's left pane becomes a single list combining tmux sessions and
  orchestrator workflow runs (instead of only the DAG, with its "No active runs" empty state).
  Selecting an item shows session or run detail in the right pane accordingly. This is the
  "Session Drop-In" concept from `initial-plan.md` §3, confirmed by the user as the direction
  (merged, not a separate tab).
- **Why:** Sessions currently only live in the sidebar as a flat list; the original console
  design always intended Mission Control to be the single place to see and act on all active
  work, sessions included.
- **Files:**
  - *Modified* `src/monitor/app.rs` — add a `MissionItem` enum (`Session(Session) |
    Run(WorkflowRun)`) and a merge/order function `pub fn build_mission_items(sessions:
    &[Session], runs: &[WorkflowRun]) -> Vec<MissionItem>` (needs-action sessions first, then
    running, then idle/success); `App` gains `pub items: Vec<MissionItem>` and a single
    `selected: usize` replacing the separate `selected_run`/`selected_node` indices where they
    served pane selection (keep `selected_node` if still needed for in-run node drill-down).
  - *Modified* `src/monitor/ui.rs` — `render_graph_pane` renders `app.items` as a themed list
    (sessions section, then runs section, or interleaved by the merge order — keep it simple);
    `render_detail_pane` branches on the selected `MissionItem` variant, reusing existing
    node/timing/token rendering for `Run` and adding `name`/`agent_state`/`foreground_cmd`/
    `last_line` rendering for `Session` (reuse `ui_theme::state_working_style()` /
    `state_blocked_style()` / `state_idle_style()` dots already used in the old sidebar).
  - *Modified* `src/sessions/app.rs` — the poll/refresh loop that already updates
    `monitor_app.runs` also feeds `sessions` into `monitor_app.items` via
    `build_mission_items`; existing `on_key` handlers for attach/new/send/kill route through when
    the selected `MissionItem` is `Session`.
- **Interfaces / shared surface:** `src/sessions/model.rs`'s `Session` struct is consumed as-is,
  no changes needed there.
- **Out of scope:** No changes to `Session`/`WorkflowRun` data model; no new tmux capabilities;
  Spaces sidebar redesign is Block BA.12.F.
- **Acceptance criteria:**
  - `build_mission_items()` has unit tests covering: empty inputs, sessions-only, runs-only, and
    a mixed case asserting the expected ordering.
  - Selecting a session in Mission Control's list shows session detail (name, agent state,
    foreground command, last output line) in the right pane; selecting a run shows the existing
    run/node detail.
  - `a`/`n`/`s`/`k` keybindings still work when a session item is selected in Mission Control.
  - Manual `tmux capture-pane` smoke test confirms the merged list and detail pane render
    correctly with at least one live session and one live/mock run.
  - Project's gating checks pass.

---

### Block BA.12.F — Spaces: brain.toml-driven hierarchy tree
- **What:** Replace the (now-freed, per Block BA.12.E) flat session sidebar with a **Spaces**
  tree sourced from `/Users/brandon/Dev/agentic-portfolio/brain.toml`: tier groups (`_root` →
  "HQ", `core`, `side`, `client`, `portfolio`) as parent rows, each tier's repos as indented
  child leaves.
- **Why:** The original console design (`initial-plan.md` §2) always treated Spaces (context
  switchers across the brain hierarchy) as distinct from the session/agent list; the shipped
  build conflated the two. This block builds the piece that was never built.
- **Files:**
  - *New* `src/brain/spaces.rs` — `SpaceEntry { slug, tier, repo_path: PathBuf, heading }`,
    `SpaceTree { tiers: Vec<(String, Vec<SpaceEntry>)> }` (fixed display order `_root`, `core`,
    `side`, `client`, `portfolio`), `pub fn load_space_tree(brain_toml_path: &Path) ->
    Result<SpaceTree, ...>` deserializing `[[repos]]` via the existing `toml` dependency. The
    tier-grouping logic is pure and unit-tested against a fixture TOML string; only the
    file-read in `load_space_tree` is I/O.
  - *Modified* `src/brain/mod.rs` — register the new `spaces` submodule.
  - *Modified* `src/sessions/app.rs` — `AppState` gains `pub space_tree: SpaceTree` and `pub
    selected_space: usize`; locate `brain.toml` using the same root-discovery convention as the
    existing `BASTION_PLANNING_ROOT` resolution (check `src/config.rs` / wherever
    `planning_root` is resolved today), adding a `BASTION_BRAIN_TOML` env override following
    that same pattern rather than hardcoding a relative path.
  - *Modified* `src/sessions/ui.rs` — sidebar rendering swaps `build_sidebar_items(&sessions)`
    for a tree render: tier headers styled/unselectable (`ui_theme::muted()`), repo leaves
    selectable and themed consistently with the rest of the console.
- **Out of scope:** Space Overview file browsing (Block BA.12.G) — this block only builds the
  tree and selection state; it does not yet drive what Space Overview displays.
- **Acceptance criteria:**
  - `load_space_tree()` correctly groups a fixture `brain.toml`-shaped TOML string into tiers in
    the fixed display order, unit-tested without filesystem access for the grouping logic.
  - The sidebar renders the real `brain.toml`'s tiers and repos as an indented tree, confirmed
    via manual `tmux capture-pane` smoke test.
  - Project's gating checks pass.

---

### Block BA.12.G — Space Overview: scrollable content + shared file browser
- **What:** Two-repo block. First, promote bella's existing file-browser
  (`core/bella/crates/bella/src/browser.rs`) into `bella-engine` as a public, app-agnostic
  module so it can be reused rather than rebuilt. Then wire it into bastion's Space Overview tab,
  scoped to the space selected in Block BA.12.F, with scrollable content.
- **Why:** The user explicitly asked to reuse bella's existing file-tree feature instead of
  building a new one; bella-engine is already the shared library both apps depend on (bastion
  already path-depends on it for markdown rendering via `bella_engine::render_with_edit`).
- **Files:**
  - *Modified* `core/bella/crates/bella-engine/Cargo.toml` — add the `ignore` crate dependency
    (already used in bella's binary crate for the same walk).
  - *New* `core/bella/crates/bella-engine/src/browser.rs` — promoted `Browser`, `BrowserEntry`,
    `BrowserEntryKind`, `build_entries` (via `ignore::WalkBuilder`, `max_depth(1)`, `.md`/`.mdx`
    filter, `.gitignore`-respecting — same as today), `move_cursor`, `descend`,
    `ascend_target`. Decoupled from bella's `App`: selecting a file returns the path as data
    (e.g. `Option<PathBuf>`) rather than mutating an app struct directly. Adds an optional
    root-boundary field so `ascend_target()` cannot navigate above a given root path (`None`
    preserves bella's current unrestricted behavior).
  - *Modified* `core/bella/crates/bella-engine/src/lib.rs` — export the new `browser` module.
  - *Modified* `core/bella/crates/bella/src/app.rs`, `core/bella/crates/bella/src/events.rs`,
    `core/bella/crates/bella/src/ui.rs` — consume `bella_engine::browser::Browser` instead of
    the local copy; delete `core/bella/crates/bella/src/browser.rs` and its `mod browser;`
    declaration.
  - *Modified* `src/sessions/app.rs` (bastion) — `AppState` gains `pub file_browser:
    bella_engine::browser::Browser` (re-instantiated when `selected_space` changes, rooted at
    that space's `repo_path` with the root-boundary set to the same path) and `pub
    space_overview_scroll: u16`.
  - *Modified* `src/sessions/ui.rs` (bastion) — Space Overview tab renders the file-browser panel
    alongside the content pane; content pane `Paragraph` gains `.scroll((space_overview_scroll,
    0))` (ratatui's native scroll support — no manual line-slicing of `bella_engine::Rendered`
    needed); `PageUp`/`PageDown`/`j`/`k` (when Space Overview is focused and the content pane has
    focus) adjust and clamp the scroll offset to the rendered line count.
  - *Modified* `src/sessions/app.rs` (bastion) — selecting a markdown entry in the browser loads
    it into the content pane by default (replacing the currently-hardcoded
    `planning/status.md` path); a distinct key opens it as a new tab via the already-defined
    `TabState::MarkdownDocument(PathBuf)` variant (push onto `AppState.tabs`, switch
    `active_tab_index` — same pattern as the shipped `next_tab`/`prev_tab` cycling).
- **Interfaces / shared surface:** `bella_engine::browser::{Browser, BrowserEntry,
  BrowserEntryKind}` becomes new shared public API consumed by both `bella` and `bastion`.
- **Out of scope:** No eager/recursive multi-level tree — browsing stays a single-directory
  drill-down (descend/ascend) matching bella's existing UX, one level at a time; this still
  covers arbitrarily deep `docs/`/`planning/` subdirectories interactively. No mutation of
  markdown files from the browser (read-only navigation and viewing only).
- **Acceptance criteria:**
  - bella's own `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` / `cargo build
    --release` stay green after the extraction, and a manual smoke test confirms bella's own
    file-browser UX is unchanged for its own users.
  - A unit test in `bella-engine` covers the new root-boundary behavior: `ascend_target()`
    returns `None` (or is otherwise blocked) at the root boundary, and normal ascend/descend
    behavior is unchanged when no boundary is set.
  - In bastion: selecting a space in the Block BA.12.F sidebar roots the file browser at that
    space's directory and the browser cannot navigate above it.
  - Space Overview's content pane scrolls via `PageUp`/`PageDown`/`j`/`k`, confirmed manually
    against a markdown file longer than the viewport.
  - Selecting a file loads it into the content pane in place; a distinct key opens it as a new
    tab, confirmed manually via `tmux capture-pane`.
  - Both bastion's and bella's gating checks pass.

---

## Quick Reference Sequence Table

| Phase | Block | What | Why | Role in destination |
|---|---|---|---|---|
| 12 | C | Kanban rows instead of columns | Cramped columns hard to read | Legible Kanban lanes |
| 12 | D | Mission Control theme | Only tab still hardcoded/gray | Consistent console palette |
| 12 | E | Merge sessions into Mission Control | Original "Session Drop-In" design never landed | Unified active-work view |
| 12 | F | Spaces tree from brain.toml | Sidebar conflated Spaces with sessions | Real brain/project hierarchy nav |
| 12 | G | Shared file browser (bella-engine) + scroll | Reuse bella's existing browser instead of rebuilding | Browsable, scrollable Space Overview |

---

*Ad-hoc mini-roadmap — run one block or the full train (see Report below).*
