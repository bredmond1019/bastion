---
type: Reference
title: brain — OKF Knowledge Graph Queries
description: "Reference for `bastion brain`: OKF corpus discovery, graph construction, structural query modes (--dependents / --blast-radius / --lineage), output format, and degradation paths."
doc_id: brain
layer: [console, brain]
project: bastion
status: active
keywords: [OKF graph, structural queries, dependents, blast-radius, lineage, knowledge graph]
related: [validate, code, config]
---

# brain — OKF Knowledge Graph Queries

`bastion brain` builds a directed graph from an OKF markdown corpus and answers
structural questions about how documents relate to one another via `[[link]]`
references. Output is one greppable line per result node.

## Usage

```
bastion brain [--dependents <NODE_ID>
              | --blast-radius <NODE_ID>
              | --lineage <NODE_ID>]
              [--root <DIR>]
              [--workspace <NAME> | --knowledge-dir <NAME>]
```

Exactly one of `--dependents`, `--blast-radius`, or `--lineage` is required.

The corpus root is resolved with the following precedence (highest to lowest):

1. `--root <DIR>` — explicit override; always wins.
2. `--workspace <NAME>` (alias: `--knowledge-dir`) — looks up `NAME` in the `[workspaces]`
   table in `~/.config/bastion/config.toml`.
3. `default_workspace` in the config file — resolved from the same registry.
4. Built-in default: current directory (`.`).

An unknown workspace name (step 2 or 3) is a fatal error with a clear message.

## Query Modes

| Flag | Relation name | What it returns |
|---|---|---|
| `--dependents <NODE_ID>` | `dependent` | Nodes that directly reference `NODE_ID` via a `[[link]]` (incoming edges only; not transitive). |
| `--blast-radius <NODE_ID>` | `blast-radius` | All nodes transitively affected by a change to `NODE_ID` — reverse BFS over the whole subgraph. |
| `--lineage <NODE_ID>` | `lineage` | All nodes that `NODE_ID` transitively references — forward DFS over the whole subgraph. |

## Output Format

Each result is printed as a single tab-separated line:

```
<relation>: <node-id>\t<path>
```

- `<relation>` is the stable label for the query mode (see table above).
- `<node-id>` is the stable slug for the result node.
- `<path>` is the filesystem path to the source file.

When no results match, a single comment line is printed:

```
# no <relation> results for '<NODE_ID>'
```

Lines are independently greppable by relation (`grep "^blast-radius:"`) or by
node id (`grep "\td20"`).

## Corpus Discovery and Graph Construction

`run()` in `crates/bastion/src/brain/mod.rs`:

1. **Resolves** the effective corpus root via `config::resolve_workspace_root` —
   pure, DB-free, using the workspace registry loaded from the config file.
2. **Discovers** all `.md` and `.mdx` files under the resolved root using
   `validate::find_markdown_files` (recursive, skips hidden dirs and `target/`).
3. **Reads** each file; individual unreadable files are skipped with a warning on
   stderr — the corpus continues to be built from the remaining files.
4. **Parses** each file into a `BrainNode` and its outgoing `BrainEdge` list via
   `okf::build_node_edge_lists`.
5. **Builds** the directed graph via `BrainGraph::build`.
6. **Runs** the requested query and prints the greppable report.

## Node Identity

Node ids are stable slugs derived by `okf::build_node_edge_lists`:

- If OKF frontmatter is present and contains a non-empty `title` field, the slug
  is the slugified title (lowercase, spaces to hyphens, non-`[a-z0-9\-\_]` chars
  dropped).
- Otherwise the node id falls back to the filename stem (no extension).

`[[link]]` targets inside documents are matched against these ids without
extension. An edge is silently dropped if its target id does not correspond to
any known node.

## Module Layout

| Module | File | Responsibility |
|---|---|---|
| `brain` (entry) | `crates/bastion/src/brain/mod.rs` | `BrainQuery` enum, `query_label`, `query_node_id`, `format_result_line`, `run()` I/O shell |
| `brain::okf` | `crates/bastion/src/brain/okf.rs` | Pure OKF parser: `BrainNode`, `BrainEdge`, `build_node_edge_lists` |
| `brain::graph` | `crates/bastion/src/brain/graph.rs` | `BrainGraph` petgraph wrapper: `build`, `predecessors`, `reachable_forward`, `reachable_reverse`, `shortest_path`, `toposort`, `has_path` |
| `brain::query` | `crates/bastion/src/brain/query.rs` | Semantic query wrappers: `dependents`, `blast_radius`, `lineage` |
| `brain::code` | `crates/bastion/src/brain/code.rs` | Pure tree-sitter extraction: `SymbolKind`, `CodeSymbol`, `CodeRef`, `extract_symbols`, `extract_refs` (Rust grammar only; no I/O) |
| `brain::code_graph` | `crates/bastion/src/brain/code_graph.rs` | Code-as-graph layer: `CodeQuery`, `build_code_node_edge_lists`, `find_definition`, `find_references`, format helpers, `find_rust_files`, `run_code` I/O shell |

## Public API Summary

### `crates/bastion/src/brain/mod.rs`

| Item | Kind | Description |
|---|---|---|
| `BrainQuery` | enum | `Dependents(String)` / `BlastRadius(String)` / `Lineage(String)` — the structural query to run. |
| `query_label(query)` | `fn` | Returns the stable output prefix label for a query variant (`"dependent"`, `"blast-radius"`, `"lineage"`). |
| `query_node_id(query)` | `fn` | Extracts the target node id string from any `BrainQuery` variant. |
| `format_result_line(label, node)` | `fn` | Formats one result as `"<label>: <id>\t<path>"`. |
| `run(query, explicit_root, workspace, registry)` | `fn` | Thin I/O shell — resolves corpus root via workspace registry, discovers corpus, builds graph, runs query, prints report. Returns `Err` on unknown workspace name, empty corpus, or unknown node id. |

### `crates/bastion/src/brain/okf.rs`

| Item | Kind | Description |
|---|---|---|
| `BrainNode` | struct | `id: String`, `title: String`, `path: PathBuf` — a corpus node. |
| `BrainEdge` | struct | `from: String`, `to: String` — a directed `[[link]]` edge. |
| `build_node_edge_lists` | `fn` | Pure: parses `(path, content)` pairs into `(Vec<BrainNode>, Vec<BrainEdge>)`. |

### `crates/bastion/src/brain/graph.rs`

| Item | Kind | Description |
|---|---|---|
| `BrainGraphError` | enum | `UnknownNode(String)` / `CycleDetected` |
| `BrainGraph` | struct | petgraph `DiGraph` wrapper. |
| `BrainGraph::build` | `fn` | Constructs the graph from node/edge lists. Unknown edge endpoints silently dropped. |
| `BrainGraph::predecessors` | `fn` | Direct incoming neighbours of a node. |
| `BrainGraph::reachable_forward` | `fn` | DFS from a node following outgoing edges (lineage). Start node excluded. |
| `BrainGraph::reachable_reverse` | `fn` | BFS on the reversed graph following incoming edges (blast radius). Start node excluded. |
| `BrainGraph::shortest_path` | `fn` | A* (unit costs) from one node to another; returns ordered node list or `None`. |
| `BrainGraph::toposort` | `fn` | Topological ordering; errors on cycles. |
| `BrainGraph::has_path` | `fn` | Boolean reachability check. |

### `crates/bastion/src/brain/query.rs`

| Function | Description |
|---|---|
| `dependents(g, id)` | Thin wrapper over `BrainGraph::predecessors`. |
| `blast_radius(g, id)` | Thin wrapper over `BrainGraph::reachable_reverse`. |
| `lineage(g, id)` | Thin wrapper over `BrainGraph::reachable_forward`. |

### `crates/bastion/src/brain/code.rs`

| Item | Kind | Description |
|---|---|---|
| `SymbolKind` | enum | `Fn` / `Struct` / `Enum` / `Trait` / `Mod` / `Impl` — category of a Rust symbol definition. |
| `CodeSymbol` | struct | `name: String`, `kind: SymbolKind`, `path: PathBuf`, `line: usize` — a definition found in source. |
| `CodeRef` | struct | `name: String`, `path: PathBuf`, `line: usize` — a call site or `use` import reference. |
| `extract_symbols(source, path)` | `fn` | Pure: returns all symbol definitions in `source` via tree-sitter-rust. Recovers from partial/malformed source without panicking. |
| `extract_refs(source, path)` | `fn` | Pure: returns all call sites and `use` import references in `source`. Refs to extern/std symbols are included; the graph layer drops unresolved refs. |

### `crates/bastion/src/brain/code_graph.rs`

| Item | Kind | Description |
|---|---|---|
| `CodeQuery` | enum | `Def(String)` / `Refs(String)` / `Dependents(String)` — the structural query to run against the code graph. |
| `build_code_node_edge_lists(symbols, refs)` | `fn` | Pure: maps `CodeSymbol`/`CodeRef` pairs into `(Vec<BrainNode>, Vec<BrainEdge>)` consumable by `BrainGraph::build`. One node per symbol (`id` = symbol name); one edge per ref that resolves both `from` (enclosing symbol) and `to` (known symbol name). Deduplicates `(from, to)` pairs. |
| `find_definition<'a>(symbols, name)` | `fn` | Pure: returns all `CodeSymbol`s whose `name` matches `name` (definition lookup). |
| `find_references<'a>(refs, name)` | `fn` | Pure: returns all `CodeRef`s whose `name` matches `name` (call sites + use imports). |
| `format_def_line(sym)` | `fn` | Formats a definition result as `"def: <name>\t<path>:<line>"`. |
| `format_ref_line(r)` | `fn` | Formats a reference result as `"ref: <name>\t<path>:<line>"`. |
| `format_dependent_line(node)` | `fn` | Formats a dependent result as `"dependent: <id>\t<path>"`. |
| `find_rust_files(root)` | `fn` | Walks `root` recursively, returns all `.rs` files in sorted order. Skips hidden directories and `target/`. |
| `run_code(query, explicit_root, workspace, registry)` | `fn` | Thin I/O shell: resolves scan root, discovers `.rs` files, reads them (skipping unreadable with stderr warnings), runs extraction, builds the graph, dispatches the query, prints greppable output. Returns `Err` on unknown workspace name, empty source tree, or unknown symbol. |

## Degradation Paths

| Condition | Behaviour |
|---|---|
| Unknown `--workspace` / `default_workspace` name | Prints clear error on stderr; exits non-zero. |
| No `.md`/`.mdx` files found under resolved root | Prints message on stderr; exits non-zero. |
| Individual file unreadable | Warning on stderr; file skipped; corpus continues. |
| `[[link]]` target not in corpus | Edge silently dropped at graph-build time. |
| Node id not found in graph | Prints message on stderr; exits non-zero. |

## Test Fixtures (`crates/bastion/src/brain/fixtures/`)

Two fixture corpora are embedded at compile time for unit and integration tests.

### Decision-graph corpus (`crates/bastion/src/brain/fixtures/`)

Five files mirror a realistic decisions corpus:

| Fixture | Role |
|---|---|
| `index.md` | Hub — links to `d3`, `d20`, `d21`, `d4` |
| `d3.md` | Links to `d20` and `d21` |
| `d20.md` | Links to `d21` and `d4` |
| `d21.md` | Links to `d4` |
| `d4.md` | Leaf — no outgoing links |
| `unlinked.md` | Isolated node — no links in or out |

### Portable corpus (`crates/bastion/src/brain/fixtures/portable/`)

A second, domain-independent corpus (client/project knowledge) used to verify
that `build_node_edge_lists` is corpus-agnostic and not hardwired to the
decision-graph domain:

| Fixture | Role |
|---|---|
| `index.md` | Registry — links to all corpus members |
| `proj-overview.md` | Project overview — links to `team-roster` and `req-doc` |
| `team-roster.md` | Team roster — links to `proj-overview` |
| `req-doc.md` | Requirements doc — links to `tech-spec` |
| `tech-spec.md` | Technical spec — leaf |
| `stale-note.md` | Isolated node — no links in or out |

## Notes

Node ids in the corpus default to filename stems when frontmatter `title` is
absent. The real brain repo uses filename-stem `[[links]]` throughout, so
`--root` pointed at that repo resolves correctly without frontmatter. If
frontmatter titles are present, their slugified form must match the `[[link]]`
targets in source files for edges to be created.

### Smoke-Test Results (Task 4)

```
$ cargo run -- brain --lineage d3 --root crates/bastion/src/brain/fixtures
lineage: d20    crates/bastion/src/brain/fixtures/d20.md
lineage: d21    crates/bastion/src/brain/fixtures/d21.md
lineage: d4     crates/bastion/src/brain/fixtures/d4.md

$ cargo run -- brain --dependents d4 --root crates/bastion/src/brain/fixtures
dependent: d20  crates/bastion/src/brain/fixtures/d20.md
dependent: d21  crates/bastion/src/brain/fixtures/d21.md

$ cargo run -- brain --blast-radius d21 --root crates/bastion/src/brain/fixtures
blast-radius: d3    crates/bastion/src/brain/fixtures/d3.md
blast-radius: d20   crates/bastion/src/brain/fixtures/d20.md
blast-radius: index crates/bastion/src/brain/fixtures/index.md
```
