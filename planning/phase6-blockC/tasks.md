# Task Spec — Phase 6, Block C

**Status:** Complete · **Last run:** 2026-06-25

## Goal
Add a Console-side, deterministic **code-as-graph** surface — exact symbol definition / reference / dependents lookup over source — alongside the docs-as-graph, reusing Block A's `BrainGraph` algorithms and Block B's workspace-root resolver.

## Context Pointers
- **Plan:** `planning/master-plan.md` → *Phase 6 — Block C* (program Block Q). Carry its **Out of scope** verbatim: no semantic "how does X work" search (that's program Block P, Engine/Python); no cross-repo refactoring or edits; no whole-repo call-graph completeness for every language — **scope extraction to the project's primary language (Rust) and note the coverage**; source repos are read-only.
- **What Block A already ships (reuse, do not re-build):** `src/brain/graph.rs` — `BrainGraph::build(nodes: Vec<BrainNode>, edges: Vec<BrainEdge>)` plus `predecessors` / `successors` / `reachable_reverse` / `reachable_forward` / `shortest_path` / `get_node`, all returning typed `BrainGraphError`. `BrainNode { id, title, path }` and `BrainEdge { from, to }` live in `src/brain/okf.rs`. The code-graph maps **symbols → `BrainNode`** (`id` = symbol name, `path` = defining file) and **call/import sites → `BrainEdge`** (`from` = referencing symbol or file, `to` = referenced symbol), then feeds `BrainGraph::build` — so "what calls X / what breaks if X changes" is just `predecessors` / `reachable_reverse`, exactly as the OKF brain does.
- **What Block B already ships (reuse):** `crate::config::resolve_workspace_root(explicit_root, workspace, registry)` (pure) and `config::load_workspace_registry(...)` (DB-free file read). The code surface resolves its scan root **the same way `brain::run` does** (`src/brain/mod.rs:67-76`) — DB-free, no `DATABASE_URL`.
- **The I/O-shell pattern to mirror:** `src/brain/mod.rs::run` — resolve root → discover files → read into `(PathBuf, String)` pairs (skip unreadable with stderr warning) → pure build → pure query → greppable `println!`. Replicate this shape for code, but discover **`.rs`** files (markdown walker `validate::find_markdown_files` is markdown-only; add a local `.rs` walker in `code_graph.rs` reusing its skip rules: skip hidden dirs, skip `target/`, deterministic sort).
- **Determinism / model-free (load-bearing):** extraction is `tree-sitter` (+ `tree-sitter-rust` grammar) over source text — no LLM, no network. tree-sitter parsing is deterministic over its input string, so `extract_*` functions are pure over `(source, path)` and unit-testable directly.
- **Greppable output convention (mirror Block A):** one line per result, `<relation>: <id>\t<path>` (see `brain::format_result_line`). Use relations `def` / `ref` / `dependent`.
- **Standing rules:** `CLAUDE.md` Rule 6 — pure extraction + graph-build + query logic exhaustively unit-tested (incl. boundary cases: nested fns, methods in `impl`, an unresolved/extern symbol, an isolated symbol); the thin file-walk I/O shell smoke-tested and recorded in `## Notes`. Rule 1 (tests ship with the block). Governing principle 4 (read-only over the source tree).

## Step-by-Step Tasks

### 1. Add tree-sitter, code fixtures, and the extraction module (`src/brain/code.rs`)
- **`Cargo.toml`:** add `tree-sitter` and `tree-sitter-rust` (compatible versions) under `[dependencies]`.
- **`src/brain/fixtures/code/`** (new): a small **multi-file Rust source fixture** that is NOT part of the crate module tree (no `mod` declaration references it, so it is never compiled or formatted) — e.g. `lib.rs` defining `fn alpha()`, `fn beta()`, `struct Widget`, an `impl Widget { fn render() }`; `consumer.rs` that `use`s and calls `alpha`/`beta`/`Widget::render`; `util.rs` with one isolated helper that nothing calls. Embed them in tests via `include_str!`. Keep the topology small and documented in a comment so edge/def counts are assertable.
  - Note in `## Notes` whether `cargo fmt --check` touches these `.rs` fixtures; if it does, rename them to a non-`.rs` suffix (e.g. `.rs.fixture`) and adjust `include_str!`.
- **`src/brain/code.rs`** (new): define `SymbolKind` (`Fn`, `Struct`, `Enum`, `Trait`, `Mod`, `Impl` — cover the kinds present in the fixture), `CodeSymbol { name: String, kind: SymbolKind, path: PathBuf, line: usize }`, and `CodeRef { name: String, path: PathBuf, line: usize }` (call sites + `use` import paths). Implement pure functions `extract_symbols(source: &str, path: &Path) -> Vec<CodeSymbol>` and `extract_refs(source: &str, path: &Path) -> Vec<CodeRef>` using tree-sitter queries against the Rust grammar. Respect function/class/symbol boundaries (a method inside `impl` is one symbol; a nested fn is its own symbol).
- **`src/brain/mod.rs`:** append `pub mod code;` (append-only).
- Unit-test `extract_symbols` / `extract_refs` against the fixture sources: assert each expected def is found with the right kind + line, and each expected call/import shows up as a ref. Include a malformed/partial-source case (tree-sitter recovers; extraction returns what it can, no panic).

### 2. Build the code-as-graph and structural queries (`src/brain/code_graph.rs`)
- **`src/brain/code_graph.rs`** (new), depends on Task 1's types. Pure layer:
  - `build_code_node_edge_lists(symbols: &[CodeSymbol], refs: &[CodeRef]) -> (Vec<BrainNode>, Vec<BrainEdge>)` — one `BrainNode` per symbol (`id` = symbol name, `path` = defining file); one `BrainEdge` per ref that resolves to a known symbol (`from` = the referencing file's or enclosing symbol's id, `to` = referenced symbol); drop refs to unknown symbols (extern/std) silently, mirroring `okf::build_node_edge_lists`. Document the from-id rule chosen.
  - Query helpers over the built graph: `find_definition(symbols, name) -> Vec<&CodeSymbol>` (def lookup), `find_references(refs, name) -> Vec<&CodeRef>`, and dependents via the reused `BrainGraph::predecessors` (direct callers) / `reachable_reverse` (blast radius).
- **Thin I/O shell** in this module — `run_code(query, explicit_root, workspace, registry)` analogous to `brain::run`: resolve root via `config::resolve_workspace_root`, walk `.rs` files with a local `find_rust_files(root)` (skip hidden + `target/`, sorted), read into `(PathBuf, String)` pairs skipping unreadable with a stderr warning, run extraction over all files, build the graph, dispatch the query, print greppable `<relation>: <id>\t<path>` lines (empty-result and unknown-symbol messages mirror `brain::run`).
- **`src/brain/mod.rs`:** append `pub mod code_graph;` (append-only).
- Unit-test the pure layer: build node/edge lists from the fixture symbols/refs and assert the topology (def lookup returns the right file+line; references return all call sites; `predecessors`/`reachable_reverse` of a known symbol return the expected callers; an isolated symbol has no dependents; an unresolved ref produces no edge).

### 3. Wire the CLI surface and dispatch (`src/cli.rs`, `src/main.rs`)
- **`src/cli.rs`:** add a top-level `Code` subcommand (mirrors the `Brain` variant's shape) with an `ArgGroup` requiring exactly one of `--def <SYMBOL>` / `--refs <SYMBOL>` / `--dependents <SYMBOL>`, plus `--root <PATH>` and `--workspace <NAME>` (with `--knowledge-dir` visible alias, matching `Brain`). Add a doc comment describing the def/refs/dependents surface and the greppable output. Add parse unit tests for each flag (mirror the existing `brain_*_parses` tests).
- **`src/main.rs`:** dispatch `Commands::Code { .. }` DB-free — build the code query enum, load the workspace registry via `config::load_workspace_registry` (same DB-free path the `Brain` arm uses, `src/main.rs:81-106`), and call `brain::code_graph::run_code(...)`. Keep the `?`-propagation / single-print error posture established by the Block B review fixes.
- Confirm `bastion code --help` renders and the three modes are mutually exclusive (ArgGroup) via the parse tests.

### 4. Validate
- Run the Validation Commands listed below and confirm all pass.
- Manually smoke-test the file-walk I/O shell against a real source tree (e.g. `bastion code --def <symbol> --root src`, `--refs`, `--dependents`) and record the result in `## Notes` per Rule 6.

## Acceptance Criteria
- `bastion code --def <SYMBOL>` returns the correct defining file (+ line) for a known symbol in the fixture/target tree; `--refs <SYMBOL>` returns its call/import sites.
- `bastion code --dependents <SYMBOL>` answers a code-dependents query over the fixture (direct callers of a known symbol), reusing `BrainGraph`.
- Extraction respects function/class/symbol boundaries (method in `impl`, nested fn, struct/enum/trait each extracted as the correct `SymbolKind`); an unresolved/extern reference produces no edge; an isolated symbol has no dependents.
- Extraction, graph-build, and query logic are pure and exhaustively unit-tested (happy + boundary + unresolved/isolated cases); the file-walk shell is smoke-tested and recorded in `## Notes` (Rule 6).
- Exactly one of `--def` / `--refs` / `--dependents` is required (ArgGroup); `--root` / `--workspace` resolve the scan root DB-free (no `DATABASE_URL`).
- Coverage scope (Rust-only) is stated in `## Notes`.
- All gated checks pass.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes

**Task 1 — fixture formatting:** The `.rs` fixture files in `src/brain/fixtures/code/` are touched by `cargo fmt --check`. They were renamed to `.rs.fixture` suffix and loaded via `include_str!` to avoid formatter interference.

**Coverage scope:** Extraction covers Rust (`.rs`) files only. Files in other languages are skipped silently. The tree-sitter Rust grammar is used for deterministic, LLM-free parsing.

**Task 4 — Validation (2026-06-25):**

All gated checks pass:
- `cargo fmt --check` — PASS
- `cargo clippy -- -D warnings` — PASS
- `cargo test` — PASS (577 tests, 0 failures)
- `cargo build --release` — PASS

Manual smoke-test of the file-walk I/O shell against `src/` (the crate's own source tree):

```
$ bastion code --def run_code --root src
def: run_code	src/brain/code_graph.rs:224

$ bastion code --refs build_code_node_edge_lists --root src
ref: build_code_node_edge_lists	src/brain/code_graph.rs:266
ref: build_code_node_edge_lists	src/brain/code_graph.rs:376
... (16 call/use sites total)

$ bastion code --dependents build_code_node_edge_lists --root src
dependent: reachable_reverse_render_includes_main_consumer	src/brain/code_graph.rs
dependent: run_code	src/brain/code_graph.rs
... (16 dependent symbols total, all test fns + run_code in code_graph.rs)
```

`bastion code --help` renders correctly and ArgGroup enforces mutual exclusivity of `--def`/`--refs`/`--dependents`. Root resolution is DB-free (no `DATABASE_URL` needed).

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
- 2026-06-25 [task 3] CLI wiring (Commands::Code ArgGroup in cli.rs, dispatch arm in main.rs) was implemented in the same commit as task 2 (6ad32ea) rather than as a separate step. Both tasks passed independently; no scope was dropped.
