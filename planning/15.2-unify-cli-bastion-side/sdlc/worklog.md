# Worklog — 15.2-unify-cli-bastion-side

## Task 1 — PASSED (1 attempt)
What: Added mev = { path = "../../../mev" } cross-repo dep and bastion validate-brain (6-way flag dispatch, --json) as a thin pass-through over mev's validate_brain* functions, with byte-identical --json parity verified against mev on the brain corpus.
Decisions: Used mev = { path = "../../../mev" } (3 ups, same shape as bella-engine) rather than the spec's literal shorthand "../mev" — matches the established cross-repo path-dep convention (bella-engine, workflow-engine-*) and works cleanly from the main (non-worktree) repo.; Discovered and worked around a Cargo workspace-detection bug specific to SDLC worktrees: mev has no [workspace] table of its own (unlike bella), so Cargo's ancestor walk from the shared gitignored trees/mev shim doesn't stop early and instead misattributes mev to the main (non-worktree) checkout's workspace, breaking workflow-engine-core's edition inheritance. Fixed by making the gitignored core/bastion/trees/mev/ a real directory with a wrapper Cargo.toml (mev's own [package]/[dependencies] copied, [lib]/[bin] path overridden to the real ../mev/src/*.rs, plus an empty [workspace] table as a stopper) — a local dev-environment fixup, not part of the tracked diff, analogous to the existing trees/bella and core/bastion/portfolio shims. Verified two other in-flight sibling worktrees have an identical, pre-existing, unrelated workspace error before and after this fixup (not a regression).; Put --json as a per-subcommand flag on ValidateBrain rather than a new global Cli flag, since the existing --json-logs global flag is for structured logging, not command output; scoping --json to the subcommand avoids confusion and matches how future validate-brain-family commands (Task 2) will likely also need their own --json.; Chose anyhow::bail! after printing the report (matching the existing validate::run pattern) to produce exit code 1 on failure, rather than std::process::exit, to stay consistent with the binary's existing error-propagation style.
Validated: gating checks (fast tripwire)

## Task 2 — PASSED (1 attempt)
What: bastion now exposes manifest/graph/emit-state as thin mev pass-throughs alongside validate-brain, with byte-identical output to the equivalent mev subcommands.
Decisions: Commands::Graph takes no --pretty flag (per the task's own cli.rs spec text), so bastion graph mirrors only mev emit-graph's default compact output, not its --pretty mode; run_emit_state resolves the root via find_brain_root (like the other handlers) before calling mev::emit_state, so a missing brain.toml surfaces as a hard anyhow error rather than exercising mev's internal E_CONFIG_NOT_FOUND diagnostic path — consistent with manifest/graph/validate-brain handlers already in this module
Validated: gating checks (fast tripwire)

## Task 3 — PASSED (1 attempt)
What: Added bastion view/edit subcommands as thin subprocess pass-throughs to the bella binary (bella-engine's app loop is private/binary-only), with pure validate_path/view_args/edit_args unit-tested and the spawn shell smoke-tested.
Decisions: bella-engine exposes only a one-shot Rendered layout (render_with_edit), not an interactive loop, and the bella app crate builds a binary only (no [lib] target) so its Reader/Browser event loop can't be imported without touching bella's Cargo.toml; resolved by shelling out to the `bella` binary as a subprocess (mirrors sessions/tmux.rs's construction-vs-execution split) instead of reimplementing bella's TUI inside bastion.; bella itself currently has no distinct edit-mode CLI flag/keybinding (only Reader/Browser modes), so `bastion edit` invokes the identical `bella <path>` command as `bastion view` today; kept as a separate cli.rs/docview entrypoint so a future bella edit-mode flag has a home without another CLI shape change.; validate_path runs before any process spawn so a missing file or a directory path degrades cleanly (typed DocViewError) without ever touching bella; a bella-not-on-PATH failure is mapped to a C001-style message and a non-zero bella exit to C010, reusing the existing observ error-code conventions.
Validated: gating checks (fast tripwire)

## Task 4 — PASSED (1 attempt)
What: Task 4 (validation-only): confirmed cargo fmt/clippy/test/build all pass at workspace root (1111 combined tests, no regressions) and re-verified byte-identical parity between bastion's validate-brain/manifest/graph/emit-state subcommands and the equivalent mev binary invocations on the brain corpus; recorded results in tasks.md Notes.
Decisions: No source changes needed for Task 4 — it is purely a validation/parity-smoke-test task per its description; only tasks.md Notes were updated.; Left the tasks.md top-of-file Status/Last-run line untouched since no prior task (1-3) touched it either; assumed that's owned by a later documentation/review pipeline stage.
Validated: gating checks (fast tripwire)

## Docs
Patched: docs/index.md | Created: docs/brainval.md, docs/docview.md

## Wrap-up — PASS
Next: Pick up BA.15.12 (mev-side dedup: drop mev's OKF/state.json dupes for okf-core, deferred out of 15.2 per D15) or resume Phase 13/14 blocks per state.json's regenerated focus.next.

## PR
https://github.com/bredmond1019/bastion/pull/15
