---
type: Handoff
created: 2026-06-26
---

# Handoff — phase6-blockC code-review deferred fixes

> **For the next agent:** Read this immediately after `/prime`. Delete this file once consumed.

## What we're doing and why

phase6-blockC (structural code navigation / code-as-graph) shipped via `/sdlc-flow` and landed
as PR #3 (branch `phase6-blockC-flow`). A `/code-review` was run post-flow against PR #3; 8
findings were confirmed. Four quick fixes were applied inline and committed to the PR branch
(`78d9fc2`). Four harder findings were deferred — they require rethinking the query layer and
the ID scheme, not just adding a pattern. Those four are the next agent's primary work before
PR #3 is merged.

## Completed this session

- Ran `/sdlc-flow phase6-blockC` — all 4 tasks passed, review verdict PASS, PR #3 opened
- Ran `/code-review 3` — 8 angles × parallel verify; 8 confirmed findings
- Applied 4 quick fixes to `trees/phase6-blockC-flow/` and committed (`78d9fc2`):
  - **Generic impl extraction** (`src/brain/code.rs:113`): added query pattern
    `(impl_item type: (generic_type type: (type_identifier) @name))` so `impl<T> Container<T>`
    is correctly captured as an `Impl` symbol (was silently dropped)
  - **Turbofish call refs** (`src/brain/code.rs:167`): added query pattern
    `(call_expression function: (generic_function function: (identifier) @name))` so
    `foo::<u32>()` call sites produce a `CodeRef` (was silently dropped)
  - **Silent zero-exit on all-files-unreadable** (`src/brain/code_graph.rs:257`): added guard
    after the read loop to `bail!` with non-zero exit if `sources` is empty when `files` was not
  - **Silent dir errors in `collect_rs_files`** (`src/brain/code_graph.rs:189`): changed bare
    `Err(_) => return` and `filter_map(|e| e.ok())` to `eprintln!` warnings, consistent with
    `run_code`'s per-file warning pattern
- Added tests for generic impl and turbofish extraction; 579 tests pass

## Remaining work

Four deferred findings from the code review — all in `trees/phase6-blockC-flow/`:

1. **BrainNode ID collision** (`src/brain/code_graph.rs:72`) — HIGHEST PRIORITY
   `build_code_node_edge_lists` sets `id = symbol.name` (bare name, no file qualification).
   `BrainGraph::build` inserts both into the petgraph but `index.insert(id, idx)` overwrites on
   collision: when `struct Widget` and `impl Widget` both exist, the struct node is orphaned in
   petgraph — it exists but is unreachable through the index. `--dependents Widget` silently
   resolves only to the last-inserted Widget. Fix requires either a qualified ID
   (`file_stem::name` or `path:line:name`) with a separate name→nodes multimap for callers who
   query by bare name, OR deduplication into a canonical node when multiple definitions share a
   name (with a decision on which wins).

2. **Grouped `use` imports not captured** (`src/brain/code.rs:165`) — CORRECTNESS
   `use std::{io, fmt}` parses as `(use_declaration argument: (use_list ...))` — no current
   pattern matches this. The fix requires either a recursive tree-walk (since `use_list` can
   nest: `use std::{io::{self, Write}, fmt}`) or an additional query pattern that descends into
   `use_list` children. Idiomatic Rust uses grouped imports everywhere; this is a significant
   graph coverage gap.

3. **`OnceLock` query hoisting** (`src/brain/code.rs:119`) — EFFICIENCY
   `extract_symbols` compiles 7 queries and `extract_refs` compiles 5 per file call.
   `run_code` calls both once per `.rs` file → O(files × 12) compilations of the same immutable
   s-expression patterns. Fix: hoist compiled `Query` objects into `OnceLock<[Query; N]>` statics
   so they compile exactly once per process. (After this fix, also consider fix #4 below.)

4. **File parsed twice per source** (`src/brain/code.rs:85`) — EFFICIENCY
   `run_code` calls `extract_symbols(content, path)` then `extract_refs(content, path)` for
   every file; both construct a `Parser` and call `parser.parse(source, None)` independently.
   Fix: combine into a single extraction function that parses once and runs all queries against
   one `Tree`, or pass a pre-built `&Tree` to both. This is a follow-on to fix #3 above (do
   after hoisting queries, since the combined function would use the shared statics too).

## Open questions / choices

- **ID scheme for fix #1:** Two viable approaches — (a) qualified IDs (`file_stem::name`) stored
  in the graph, with a bare-name lookup multimap for CLI queries; (b) keep bare-name IDs but
  deduplicate into a single canonical node per name (last-wins or first-wins). Approach (a)
  preserves all definition sites; approach (b) is simpler but loses the distinction. The CLI
  already uses bare names (`--def Widget`, `--dependents Widget`), so approach (b) fits the
  current API but silently merges distinct definitions. Recommend (a) — record the choice as a
  decision before implementing.

- **`use_list` recursion strategy for fix #2:** A single additional tree-sitter query pattern
  can capture the immediate children of a `use_list`, but nested use groups require either
  multiple patterns or a recursive tree-walk. Determine the depth of nesting you want to
  support before implementing.

## Context the next agent needs

- PR #3 is on branch `phase6-blockC-flow`, worktree at
  `trees/phase6-blockC-flow/`. All work should happen in the worktree; merge to `main` after
  the deferred fixes are applied and tests pass.
- The CLAUDE.md coverage bar (rule 6) requires: pure-logic functions exhaustively unit-tested,
  error paths explicitly tested, thin I/O shell smoke-tested with result recorded in `## Notes`
  of the task spec. The quick fixes added unit tests for their new paths; the deferred fixes
  must do the same.
- After the deferred fixes: push the branch, verify PR #3 CI passes, then merge per the normal
  flow (`/clean-worktree` or manual merge).
- 579 tests currently passing on the branch. Any change must keep all tests green.

## First command after `/prime`

`cd trees/phase6-blockC-flow && cargo test`
