---
type: Reference
title: code — Symbol-Level Code Graph Queries
description: Reference for `bastion code`: deterministic tree-sitter extraction over Rust source, symbol definition / reference / dependents lookup, output format, root resolution, and degradation paths.
---

# code — Symbol-Level Code Graph Queries

`bastion code` builds a directed symbol graph from `.rs` source files using
tree-sitter (deterministic, no LLM, no network) and answers structural
questions about how symbols relate to one another. Output is one greppable
line per result.

Coverage: **Rust (`.rs`) files only**. Source files in other languages under
the scan root are silently skipped.

## Usage

```
bastion code (--def <SYMBOL>
             | --refs <SYMBOL>
             | --dependents <SYMBOL>)
             [--root <DIR>]
             [--workspace <NAME> | --knowledge-dir <NAME>]
```

Exactly one of `--def`, `--refs`, or `--dependents` is required.

The scan root is resolved with the following precedence (highest to lowest):

1. `--root <DIR>` — explicit override; always wins.
2. `--workspace <NAME>` (alias: `--knowledge-dir`) — looks up `NAME` in the
   `[workspaces]` table in `~/.config/bastion/config.toml`.
3. `default_workspace` in the config file — resolved from the same registry.
4. Built-in default: current directory (`.`).

An unknown workspace name (step 2 or 3) is a fatal error with a clear message.
Root resolution is DB-free — no `DATABASE_URL` is required.

## Query Modes

| Flag | Relation label | What it returns |
|---|---|---|
| `--def <SYMBOL>` | `def` | File(s) and line(s) where `SYMBOL` is defined. Multiple results occur when the same name appears in more than one file (e.g. trait impls). |
| `--refs <SYMBOL>` | `ref` | All call sites and `use` import statements that reference `SYMBOL`. |
| `--dependents <SYMBOL>` | `dependent` | Symbols that directly call or import `SYMBOL` (direct predecessors in the code graph; not transitive). |

## Output Format

### `--def`

```
def: <name>\t<path>:<line>
```

### `--refs`

```
ref: <name>\t<path>:<line>
```

### `--dependents`

```
dependent: <name>\t<path>
```

When no results match, a single comment line is printed:

```
# no <mode> results for '<SYMBOL>'
```

Lines are independently greppable by relation (`grep "^ref:"`) or by
symbol name (`grep "\trun_code"`).

## Extraction and Graph Construction

`run_code()` in `src/brain/code_graph.rs`:

1. **Resolves** the effective scan root via `config::resolve_workspace_root` —
   pure, DB-free, using the workspace registry loaded from the config file.
2. **Discovers** all `.rs` files under the resolved root via `find_rust_files`
   (recursive, sorted, skips hidden dirs and `target/`).
3. **Reads** each file; individual unreadable files are skipped with a warning
   on stderr — extraction continues from the remaining files.
4. **Extracts** symbols and references from each file via
   `code::extract_symbols` / `code::extract_refs` (tree-sitter-rust grammar;
   deterministic over source text; recovers from partial/malformed source).
5. **Builds** the symbol graph via `build_code_node_edge_lists` + `BrainGraph::build`.
6. **Runs** the requested query and prints the greppable report.

## Symbol Coverage

The following Rust item kinds are extracted as symbol definitions:

| `SymbolKind` | Rust construct |
|---|---|
| `Fn` | `fn` items — top-level, method inside `impl`, or nested |
| `Struct` | `struct` items |
| `Enum` | `enum` items |
| `Trait` | `trait` items |
| `Mod` | `mod` items (inline or file-module declarations) |
| `Impl` | `impl` blocks — keyed by the implementing type name |

Reference extraction covers: direct function calls, method calls, and `use`
import paths (last path segment).

## From-id Rule (Edge Construction)

Each edge in the code graph has a `from` id equal to the **enclosing symbol**
for a given reference. The enclosing symbol for a reference at line L in file F
is the last symbol defined in F at or before line L (located via binary search
over the per-file symbol index).

References that precede all symbols in their file (e.g. module-level `use`
statements appearing before the first `fn`) have no enclosing symbol and are
silently dropped — their `from` cannot resolve to a known node.

References to unknown symbols (extern, std, or out-of-scope) are also silently
dropped. Duplicate `(from, to)` pairs are deduplicated.

## Module Layout

| Module | File | Responsibility |
|---|---|---|
| `brain::code` | `src/brain/code.rs` | Pure tree-sitter extraction: `SymbolKind`, `CodeSymbol`, `CodeRef`, `extract_symbols`, `extract_refs` |
| `brain::code_graph` | `src/brain/code_graph.rs` | Graph layer and I/O shell: `CodeQuery`, `build_code_node_edge_lists`, query helpers, `find_rust_files`, `run_code` |

The code graph reuses `BrainNode` / `BrainEdge` (from `brain::okf`) and
`BrainGraph` (from `brain::graph`) — the same types and algorithms that power
`bastion brain`.

## Degradation Paths

| Condition | Behaviour |
|---|---|
| Unknown `--workspace` / `default_workspace` name | Prints clear error on stderr; exits non-zero. |
| No `.rs` files found under resolved root | Prints message on stderr; exits non-zero. |
| Individual file unreadable | Warning on stderr; file skipped; extraction continues. |
| Reference to unknown / extern symbol | Edge silently dropped at graph-build time. |
| Reference with no enclosing symbol in its file | Edge silently dropped (from-id cannot resolve). |
| Symbol name not found in graph | Prints `# no <mode> results for '<SYMBOL>'`; exits zero. |

## Test Fixtures (`src/brain/fixtures/code/`)

Three `.rs.fixture` files (renamed from `.rs` to prevent `cargo fmt` from
touching them) define a small multi-file Rust fixture used in unit tests:

| Fixture | Role |
|---|---|
| `lib.rs.fixture` | Defines `fn alpha`, `fn beta`, `struct Widget`, `impl Widget { fn render }` |
| `consumer.rs.fixture` | Imports and calls `alpha`, `beta`, and `Widget::render` |
| `util.rs.fixture` | Isolated helper — no callers, no imports from other fixture files |

Tests assert exact symbol counts, kinds, line numbers, and edge topology
(including the isolated-symbol and unresolved-extern cases).

## Notes

### Smoke-Test Results (Task 4, 2026-06-25)

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
... (16 dependent symbols total)
```

`bastion code --help` renders correctly. `--def` / `--refs` / `--dependents`
are mutually exclusive via the ArgGroup. Root resolution requires no
`DATABASE_URL`.
