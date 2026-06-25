---
type: Reference
title: brain — OKF Knowledge Graph Queries
description: Reference for `bastion brain`: OKF corpus discovery, graph construction, structural query modes (--dependents / --blast-radius / --lineage), output format, and degradation paths.
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
```

Exactly one of `--dependents`, `--blast-radius`, or `--lineage` is required.
`--root` defaults to the current directory when omitted.

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

`run()` in `src/brain/mod.rs`:

1. **Discovers** all `.md` and `.mdx` files under `--root` using
   `validate::find_markdown_files` (recursive, skips hidden dirs and `target/`).
2. **Reads** each file; individual unreadable files are skipped with a warning on
   stderr — the corpus continues to be built from the remaining files.
3. **Parses** each file into a `BrainNode` and its outgoing `BrainEdge` list via
   `okf::build_node_edge_lists`.
4. **Builds** the directed graph via `BrainGraph::build`.
5. **Runs** the requested query and prints the greppable report.

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
| `brain` (entry) | `src/brain/mod.rs` | `BrainQuery` enum, `query_label`, `query_node_id`, `format_result_line`, `run()` I/O shell |
| `brain::okf` | `src/brain/okf.rs` | Pure OKF parser: `BrainNode`, `BrainEdge`, `build_node_edge_lists` |
| `brain::graph` | `src/brain/graph.rs` | `BrainGraph` petgraph wrapper: `build`, `predecessors`, `reachable_forward`, `reachable_reverse`, `shortest_path`, `toposort`, `has_path` |
| `brain::query` | `src/brain/query.rs` | Semantic query wrappers: `dependents`, `blast_radius`, `lineage` |

## Public API Summary

### `src/brain/mod.rs`

| Item | Kind | Description |
|---|---|---|
| `BrainQuery` | enum | `Dependents(String)` / `BlastRadius(String)` / `Lineage(String)` — the structural query to run. |
| `query_label(query)` | `fn` | Returns the stable output prefix label for a query variant (`"dependent"`, `"blast-radius"`, `"lineage"`). |
| `query_node_id(query)` | `fn` | Extracts the target node id string from any `BrainQuery` variant. |
| `format_result_line(label, node)` | `fn` | Formats one result as `"<label>: <id>\t<path>"`. |
| `run(query, root)` | `fn` | Thin I/O shell — discovers corpus, builds graph, runs query, prints report. Returns `Err` on empty corpus or unknown node id. |

### `src/brain/okf.rs`

| Item | Kind | Description |
|---|---|---|
| `BrainNode` | struct | `id: String`, `title: String`, `path: PathBuf` — a corpus node. |
| `BrainEdge` | struct | `from: String`, `to: String` — a directed `[[link]]` edge. |
| `build_node_edge_lists` | `fn` | Pure: parses `(path, content)` pairs into `(Vec<BrainNode>, Vec<BrainEdge>)`. |

### `src/brain/graph.rs`

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

### `src/brain/query.rs`

| Function | Description |
|---|---|
| `dependents(g, id)` | Thin wrapper over `BrainGraph::predecessors`. |
| `blast_radius(g, id)` | Thin wrapper over `BrainGraph::reachable_reverse`. |
| `lineage(g, id)` | Thin wrapper over `BrainGraph::reachable_forward`. |

## Degradation Paths

| Condition | Behaviour |
|---|---|
| No `.md`/`.mdx` files found under `--root` | Prints message on stderr; exits non-zero. |
| Individual file unreadable | Warning on stderr; file skipped; corpus continues. |
| `[[link]]` target not in corpus | Edge silently dropped at graph-build time. |
| Node id not found in graph | Prints message on stderr; exits non-zero. |

## Test Fixtures (`src/brain/fixtures/`)

Five fixture files mirror a realistic decisions corpus for unit and integration tests:

| Fixture | Role |
|---|---|
| `index.md` | Hub — links to `d3`, `d20`, `d21`, `d4` |
| `d3.md` | Links to `d20` and `d21` |
| `d20.md` | Links to `d21` and `d4` |
| `d21.md` | Links to `d4` |
| `d4.md` | Leaf — no outgoing links |
| `unlinked.md` | Isolated node — no links in or out |

## Notes

Node ids in the corpus default to filename stems when frontmatter `title` is
absent. The real brain repo uses filename-stem `[[links]]` throughout, so
`--root` pointed at that repo resolves correctly without frontmatter. If
frontmatter titles are present, their slugified form must match the `[[link]]`
targets in source files for edges to be created.

### Smoke-Test Results (Task 4)

```
$ cargo run -- brain --lineage d3 --root src/brain/fixtures
lineage: d20    src/brain/fixtures/d20.md
lineage: d21    src/brain/fixtures/d21.md
lineage: d4     src/brain/fixtures/d4.md

$ cargo run -- brain --dependents d4 --root src/brain/fixtures
dependent: d20  src/brain/fixtures/d20.md
dependent: d21  src/brain/fixtures/d21.md

$ cargo run -- brain --blast-radius d21 --root src/brain/fixtures
blast-radius: d3    src/brain/fixtures/d3.md
blast-radius: d20   src/brain/fixtures/d20.md
blast-radius: index src/brain/fixtures/index.md
```
