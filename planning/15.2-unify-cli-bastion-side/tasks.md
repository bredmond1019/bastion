---
type: TaskSpec
title: "Task Spec — Phase 15, Block BA.15.2: Unify the CLI (bastion-side)"
description: Add bastion subcommands (validate-brain/emit-state/manifest/graph over mev; view/edit over bella-engine) as thin pass-throughs to cross-repo path-dep libraries, with zero mev/bella source changes.
doc_id: 15-2-unify-cli-bastion-side-tasks
layer: [console]
project: bastion
status: active
keywords: [unified cli, mev, bella-engine, path dependency, validate-brain, emit-state, pass-through]
related: [master-plan, bastion-product-plan, D15-mev-integration-cross-repo-path-dep]
---

# Task Spec — Phase 15, Block BA.15.2: Unify the CLI (bastion-side)

**Status:** Not started · **Last run:** never

## Goal
Fold mev's brain-ops commands and bella's document viewer into the `bastion` binary as thin
pass-throughs — `bastion validate-brain` / `emit-state` / `manifest` / `graph` (over `mev`) and
`bastion view` / `edit` (over `bella-engine`) — with **no changes to mev or bella source**.

## Scope decision (per D15 — operator-confirmed 2026-07-03)
This is the **bastion-side half** of the original BA.15.2, split per
[`D15`](../decisions/D15-mev-integration-cross-repo-path-dep.md):
- **mev is consumed as a cross-repo Cargo path dep** (`mev = { path = "../mev" }`), exactly like
  `bella-engine`. mev stays its own repo; **its internals are not touched**. mev is already a library
  exporting the functions we need (`validate_brain`, `validate_brain_sync`/`_graph`/`_state`/`_links`/
  `_structure`, `emit_state`, `manifest_brain`, `graph_brain`, `visualize_brain`, and the pub
  `JsonReport` envelope).
- **The mev-side dedup is OUT OF SCOPE** — dropping mev's OKF/`state.json` dupes for `okf-core` (and the
  okf-core state-schema + OKF-model reconciliation it needs) is the deferred **BA.15.12**.
- **No `bin-shims`** — `mev` and `bella` keep their own standalone binaries in their own repos, so no
  re-dispatch shims are needed.

## Context Pointers
- **Plan:** `planning/master-plan.md` → "Block BA.15.2 — Unify the CLI (bastion-side)";
  `planning/bastion-product/plan.md` → Wave 1 BA.15.2 + Critical-files row; `D15`.
- **mev public API to call** (`../mev/src/lib.rs`, crate name `mev`): each `validate_brain*` /
  `emit_state` returns `mev::Report`; `manifest_brain` → `mev::Manifest`; `graph_brain` →
  `mev::GraphExport`; `mev::JsonReport::new(validator, root, &report).to_json()` is the machine envelope.
  All take a `root: &Path` and resolve `brain.toml` by walking up (returning an `E_CONFIG_NOT_FOUND`
  diagnostic, never a panic).
- **mev's CLI shape to mirror** (`../mev/src/main.rs`): `ValidateBrain { path, sync, graph, state, links,
  structure }` with **dispatch precedence** `--links > --structure > --state > --graph > --sync > (base
  OKF pass)`; `Manifest { path, pretty }`; `EmitState { path, write }` (dry-run default). A global
  `--json` flag emits the `JsonReport` envelope; exit code is 1 when `report.is_failure()`.
- **bella-engine** (`../../../bella/crates/bella-engine`, already a bastion dep — D14): `markdown::render`
  and `markdown::render_with_edit` exist; bastion already renders markdown via `bella_engine::render_with_edit`
  in `crates/bastion/src/sessions/ui.rs:307`. `bastion view`/`edit` are thin standalone wrappers over
  bella's document open — **confirm the exact interactive entrypoint against bella's own binary**
  (`../../../bella` crate `bella`), since `render*` produce a `Rendered` buffer rather than running an
  interactive loop; if bella-engine exposes no one-call interactive open, mirror bella's `main.rs` app
  loop. (This is the one implementation-uncertainty in the block — see Notes.)
- **Dispatch pattern to follow** (`crates/bastion/src/cli.rs` `enum Commands`, `main.rs` name-mapper +
  dispatch `match`): declare→name→dispatch, DB-free (D4) — these commands never open the Postgres pool.
- **CLAUDE.md rules:** Rule 1 (tests ship with every change); Rule 6 (separate pure logic from I/O — the
  flag→function selection, exit-code-from-`Report`, and output rendering are pure and unit-tested; the
  mev/bella calls are the thin I/O shell, smoke-tested and recorded in Notes); Rule 7/§D14/§D15
  (mev + bella are unpinned cross-repo path-dep contracts — consume, never fork).

## Design (thin pass-through)
- New module `crates/bastion/src/brainval/` holds the mev-backed subcommand handlers + **pure**
  render/exit helpers (`report_to_exit_code(&Report) -> u8`, `render_human(&Report) -> String`,
  `--json` via `mev::JsonReport`). Handlers call the matching `mev::*` fn and print.
- New module `crates/bastion/src/docview/` holds `view`/`edit` handlers over `bella-engine`.
- `cli.rs` declares the new `Commands` variants; `main.rs` adds the name-mapper entries + dispatch arms.
- Parity target: the `--json` envelope is **byte-identical** to mev's (same `mev::JsonReport`), exit
  codes match, and the diagnostics set matches; the human summary is *equivalent* (bastion renders its
  own summary from the `Report` — it does not import mev's private `main.rs` formatter, since we don't
  change mev).

## Step-by-Step Tasks
See `tasks.json` in this directory — the task list is defined there, not here.

## Acceptance Criteria
- `crates/bastion/Cargo.toml` declares `mev = { path = "../mev" }`; the workspace builds with mev pulled
  in; **no** file under `../mev` or `../../../bella` is modified.
- `bastion validate-brain [--sync|--graph|--state|--links|--structure] [--json]` calls the matching
  `mev::validate_brain*` function with mev's documented flag precedence, prints results, and exits 1 on
  failure / 0 otherwise — its `--json` output is byte-identical to `mev validate-brain … --json` on the
  brain corpus.
- `bastion manifest [--pretty]`, `bastion graph`, and `bastion emit-state [--write]` produce output
  matching the equivalent `mev` subcommand (manifest/graph JSON identical; `emit-state` dry-run reports
  the same planned actions).
- `bastion view <file>` and `bastion edit <file>` open a document via `bella-engine` (viewer / editor).
- Pure logic (flag→fn selection, exit-code-from-`Report`, `--json` rendering) is unit-tested per Rule 6;
  the mev/bella I/O shells are smoke-tested and recorded in `## Notes`.
- Combined test count is not lower; gated checks pass at the workspace root.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```
<!-- Plus the parity smoke-tests in Task 4 (recorded in Notes), run from the brain root. -->

## Notes
<filled in as work happens — record the mev/bella I/O smoke tests and the resolved bella view/edit entrypoint here>

### Task 1 — mev path dep + `validate-brain`

- `crates/bastion/Cargo.toml` adds `mev = { path = "../../../mev" }` (same 3-up shape as the
  existing `bella-engine` dep). `crates/bastion/src/brainval/mod.rs` holds the pure
  `select_validate_brain_mode` (flag precedence), `report_to_exit_code`, `render_human`, and
  `render_json` helpers plus the thin `run()` I/O shell; `cli.rs` gained
  `Commands::ValidateBrain`; `main.rs` registered `mod brainval;` + the name-mapper/dispatch arm.
- **Worktree-only build fixup (environment, not code):** building `mev` as a cross-repo path dep
  from inside an SDLC worktree (`core/bastion/trees/<name>/...`) hits a Cargo workspace-detection
  bug: `mev`'s own `Cargo.toml` has no `[workspace]` table (unlike `bella`'s, which does), so
  Cargo's ancestor walk from the worktree-shared `trees/mev` shim symlink doesn't stop there and
  instead climbs to `core/bastion/Cargo.toml` (the *main*, non-worktree checkout's own workspace),
  misattributing `mev` to the wrong workspace and breaking `workflow-engine-core`'s
  `edition.workspace = true` inheritance. Fixed **without touching `../mev`** by making the
  gitignored `core/bastion/trees/mev/` a real directory containing a small wrapper
  `Cargo.toml` (mev's own `[package]`/`[dependencies]` copied verbatim, `[lib]`/`[bin]` `path`
  overridden to the real `../mev/src/{lib,main}.rs`, plus an empty `[workspace]` table to stop the
  ancestor walk) — this is a local, machine-specific dev-environment fixup (like the existing
  `trees/bella` and `core/bastion/portfolio` shims), not part of this commit's tracked diff.
  Verified the two other in-flight sibling worktrees (`13.2-mouse-interactivity-flow-2`,
  `phase3-blockb-task3`) have an unrelated, pre-existing "believes it's in a workspace when it's
  not" error identical before and after this fixup — confirmed not a regression I introduced.
- **Parity smoke test** (brain root `/Users/brandon/Dev/agentic-portfolio`):
  `cargo run -- validate-brain <root> --json` vs `mev validate-brain <root> --json` — `diff`
  confirms byte-identical output (0 errors, 1 pre-existing keywords warning). Human-mode
  (`bastion validate-brain <root>`) prints one line per diagnostic + a summary line and exits 0
  for warnings-only, matching mev's own shape.

### Task 2 — `manifest` / `graph` / `emit-state`

- `crates/bastion/src/cli.rs` gains `Commands::Manifest { path, pretty }`,
  `Commands::Graph { path }` (no `--pretty`, per the task's own description — mev's `emit-graph`
  defaults to compact and bastion's `graph` mirrors only the default, compact shape), and
  `Commands::EmitState { path, write }`. `crates/bastion/src/brainval/mod.rs` gains the pure
  `render_manifest_json(&Manifest, pretty) -> Result<String>` and
  `render_graph_json(&GraphExport) -> Result<String>` helpers plus the thin I/O shells
  `run_manifest`, `run_graph`, `run_emit_state` (the latter reuses the same
  diagnostic-line + summary-line human rendering shape as mev's own `EmitState` command).
  `main.rs` registered the three name-mapper entries + dispatch arms.
- **Parity smoke tests** (brain root `/Users/brandon/Dev/agentic-portfolio`, both binaries built
  in debug):
  - `bastion manifest .` vs `mev manifest .` — `diff` confirms byte-identical (311,611 bytes).
  - `bastion graph .` vs `mev emit-graph .` — `diff` confirms byte-identical.
  - `bastion emit-state .` (dry-run default) vs `mev emit-state .` (dry-run default) — `diff`
    confirms byte-identical (same planned-action lines + summary line), both exit 0.

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
