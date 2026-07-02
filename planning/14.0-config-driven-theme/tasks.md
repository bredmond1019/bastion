---
type: TaskSpec
title: "Task Spec — Phase 14, Block BA.14.0: Config-driven theme system"
description: Decompose BA.14.0 into disjoint-ownership tasks that make all UI color config-driven off a runtime Theme shared by chrome and the markdown view.
doc_id: 14-0-config-driven-theme-tasks
layer: [console]
project: bastion
status: active
keywords: [theme, config-driven, ui-theme, bella-engine, runtime-theme, toml-config]
related: [bastion-master-plan, 13-0-spine-primary-navigation-tasks]
---

# Task Spec — Phase 14, Block BA.14.0: Config-driven theme system

**Status:** Not started · **Last run:** never

## Goal
Make all color config-oriented: `ui_theme` functions read a runtime `Theme`, an optional `[theme]` config section selects a named preset (default `bastion`, default fallback), and the same theme maps to `bella_engine::Theme` for `render_with_edit` so chrome and the markdown view share one theme.

## Context Pointers
- **Plan section:** `planning/master-plan.md` → Phase 14 → *Block BA.14.0 — Config-driven theme system* (lines ~1304–1328). Phase preamble (lines ~1293–1302): colors are already centralized in `src/ui_theme.rs` (named fns via `bella_engine::palette::rgb()` with xterm-256 downgrade), a configurable `Theme` struct already exists in `bella_engine` (`dark`/`light`/`bastion`), config is already TOML (`~/.config/bastion/config.toml`, `FileConfig`) — so this is refactor-and-extend, not build-from-scratch. **Sequenced right after BA.13.0 so every new pane is built against the runtime theme.**
- **Repo files (per block's *Files* list):** `src/ui_theme.rs` (runtime `Theme` + accessor + bastion→bella mapping), `src/config.rs` (`FileConfig` `[theme]` section + resolution), `src/sessions/ui.rs` (pass mapped `bella_engine::Theme` to `render_with_edit`). Optionally `core/bella/crates/bella-engine/src/theme.rs` **only if** a new constructor/preset is required.
- **CLAUDE.md standing rules:** Rule 1 (tests ship with every behaviour change), Rule 6 (pure logic — name→theme resolution and the bastion→bella mapping — exhaustively unit-tested without I/O), Rule 7 (`bella-engine` is an unpinned cross-repo path dep in a *separate* repo/worktree — **prefer consuming the existing `Theme`; do not edit `../bella` unless a mapping genuinely needs a new constructor**, and if so coordinate the break; do not add `default-features = false`). Track is DB-free (D4) and read-only vs the orchestrator (D2).
- **Out of scope (hard boundary, from the block):** full custom-palette-in-TOML / per-role overrides (later extension); the actual color-value retune (BA.14.3).

## Step-by-Step Tasks

### 1. [~] BA.14.0.1 Runtime `Theme` refactor + accessor + bella mapping in `src/ui_theme.rs`
- **Owns:** `src/ui_theme.rs` (only file touched by this task).
- Refactor the `ui_theme` functions so they read a runtime `Theme` (process-wide `OnceCell`/`OnceLock` set at startup, with a `bastion` default when unset) instead of returning baked `rgb()` constants. No fixed `rgb(...)`/`Color::` literals should remain outside the theme definition itself.
- Provide named presets keyed by name (default `bastion`, with room for more, e.g. `dark`/`light`), and a pure `theme_by_name(&str) -> Theme` (or equivalent) that falls back to the default for an absent/unknown name.
- Add the bastion→`bella_engine::Theme` mapping (a pure function) so the selected theme can be handed to `render_with_edit`. **Consume the existing `bella_engine::Theme` presets/constructors** — only add a bella-side constructor as a last resort (see BA.14.0.3 note; Rule 7).
- Expose the runtime-`Theme` accessor + an init/setter that all other UI blocks (Phase 13/14) consume.
- **Tests (Rule 6):** unit-test `theme_by_name` for a known preset, the default when the name is absent, and the fallback for an unknown name; unit-test the bastion→bella mapping (asserting mapped colors/roles); assert the accessor returns the `bastion` default before any init.

### 2. [~] BA.14.0.2 `[theme]` config section + resolution in `src/config.rs`
- **Owns:** `src/config.rs` (only file touched by this task). **Depends on:** BA.14.0.1 (preset names / `theme_by_name`).
- Extend `FileConfig` with an optional `[theme]` section carrying (at minimum) a theme *name* selection; keep it fully optional so existing configs parse unchanged.
- Add resolution: config `[theme].name` (or absent) → resolved `Theme` via the BA.14.0.1 lookup, with a default fallback when the section is absent or the name is unknown.
- **Tests (Rule 6):** unit-test parsing a `config.toml` fixture *with* a `[theme]` name, *without* a `[theme]` section (→ default), and with an *unknown* name (→ default fallback); confirm a pre-existing config with no `[theme]` still deserializes.

### 3. [~] BA.14.0.3 Apply runtime theme at the sessions entry in `src/sessions/ui.rs`
- **Owns:** `src/sessions/ui.rs` (only file touched by this task). **Depends on:** BA.14.0.1 (accessor/init + bella mapping) and BA.14.0.2 (resolved `Theme` from `FileConfig`).
- At the sessions/TUI entry, initialize the runtime-`Theme` accessor from the resolved `FileConfig` theme (so chrome reads the active theme).
- Replace the fixed `Theme::bastion()` passed to `render_with_edit` with the mapped `bella_engine::Theme` from the active theme, so the markdown view and TUI chrome share one theme.
- **Cross-repo caveat (Rule 7):** if and only if the mapping cannot be expressed against the existing `bella_engine::Theme` API, add the minimal constructor in `../bella/crates/bella-engine/src/theme.rs` and record the coordination in `## Notes` + the Amendment Log; otherwise touch no bella files.
- **Tests:** a `draw_for_test` (existing sessions TUI test pattern) asserting a non-default theme selection changes the rendered chrome color for at least one element, and that `render_with_edit` receives the mapped theme (assert via the pure mapping seam rather than pixel colors where the render is opaque).

### 4. BA.14.0.4 Validate
- Run the Validation Commands listed below and confirm all pass.
- Manually smoke-test the TUI via tmux `capture-pane`: set a `[theme]` name in a scratch config (and unset it), confirm the chrome + markdown view visibly share the theme and that an absent/unknown name falls back to `bastion` without panic. Record the result in `## Notes`.

## Acceptance Criteria
- A theme resolves by name from config with a default fallback when `[theme]` is absent/unknown (unit-tested).
- `ui_theme` functions return the active theme's colors (no fixed `rgb`/`Color::` literals outside the theme definition).
- The bastion→`bella_engine::Theme` mapping is unit-tested.
- The markdown view and chrome visibly share the theme.
- `../bella` is edited only if a new bella constructor was unavoidable; if so the coordination is recorded (Rule 7).
- All gated checks pass.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
- **Task 3 (BA.14.0.3):** `src/sessions/ui.rs` now initializes the process-wide runtime theme
  at the top of `run()` via `init_theme_from_config()` — a thin wrapper over already-tested pure
  functions (`config::load_workspace_registry`, `config::resolve_theme`, `ui_theme::init_theme`)
  reading `XDG_CONFIG_HOME`/`HOME`. Both `render_with_edit` call sites now pass
  `ui_theme::to_bella_theme(ui_theme::current_theme())` instead of the fixed
  `bella_engine::Theme::mission_control()`, so chrome and the markdown view read the same
  runtime theme. No `../bella` files were touched — the existing `bella_engine::Theme` struct
  covers the mapping (Rule 7 caveat not triggered).
  - Per Rule 6, `init_theme_from_config`'s I/O shell is a trivial wrapper and is not
    independently unit-tested; its constituent pure functions already carry unit tests in
    `src/config.rs` (BA.14.0.2) and `src/ui_theme.rs` (BA.14.0.1). Manual smoke-test of the
    live `run()` path (setting/unsetting `[theme]` in a scratch config and confirming no panic +
    the fallback to `bastion`) is deferred to Task 4's Validate step, which owns that check for
    the whole spec.
  - Added two tests in `src/sessions/ui.rs`'s own test module (no other files touched):
    `build_space_item_working_dot_tracks_runtime_theme` renders a working-state session via
    `draw_for_test`/`TestBackend` and asserts the sidebar dot's `Cell::fg` equals
    `ui_theme::current_theme().sage` (proving chrome reads live from the runtime theme, not a
    baked literal); `render_with_edit_receives_theme_mapped_from_current_theme` asserts the
    `to_bella_theme(current_theme())` seam that both `render_with_edit` call sites use stays in
    lock-step with the live theme's fields (mapping asserted directly rather than via opaque
    rendered pixel colors, per the task's test guidance).
  - Not exercised: mutating the global runtime theme mid-test-binary via `ui_theme::init_theme`
    was deliberately avoided in these tests — `ACTIVE_THEME` is a process-wide `OnceLock` shared
    across the whole `cargo test` binary, and only one preset (`bastion`) currently exists via
    `theme_by_name`, so there is no second named preset to switch to for a true "non-default
    theme" comparison; tests instead assert against whatever `current_theme()` resolves to at
    call time, which is deterministic and safe under parallel test execution.

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
