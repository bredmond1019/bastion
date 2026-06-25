# Task Spec — Phase 6, Block A

**Status:** Not started · **Last run:** never

## Goal
Vendor the `knowledge_graph` algorithms (Dgraph-free) into bastion and expose a `bastion brain` subcommand that answers structural questions — dependents, blast-radius, lineage — over the graph derived from the OKF `[[link]]` corpus.

## Context Pointers
- **Plan:** `planning/master-plan.md` → *Phase 6 — Brain & code retrieval (program Wave 1)* → *Block A* (the source block definition; carry its **Files** and **Out of scope** through verbatim).
- **Source crate (read-only):** `../workflow-engine-rs/services/knowledge_graph/` — algorithms live in `src/algorithms/{shortest_path,topological_sort,traversal,ranking}.rs` and the graph model in `src/graph.rs`. They are coupled to a `Concept`/`Uuid` domain model and a Dgraph client; **vendor the algorithm shapes decoupled from Dgraph**, reusing `petgraph`'s built-ins where they suffice (`petgraph = "0.8.3"` is already a dependency — A\*, Dijkstra, topological sort, DFS/BFS are all built in). PageRank + community detection are *available for Phase 8* but **out of scope here**.
- **Existing reuse:** `src/validate/frontmatter.rs` (`Frontmatter` struct + `validate_frontmatter`) shows the OKF frontmatter-parsing approach to mirror. `src/validate/links.rs::extract_links` extracts markdown `[text](path)` links — the OKF `[[link]]` wiki-style edge is **different**; the brain reader needs its own `[[...]]` extractor.
- **CLI pattern:** `src/cli.rs` `Commands` enum + `src/main.rs` dispatch — follow the established subcommand shape (see `Validate`, `Costs`).
- **Standing rules:** `CLAUDE.md` Rule 1 (tests ship with every block), Rule 6 (separate pure logic from I/O; exhaustively unit-test the pure core incl. error/degrade paths; smoke-test the thin I/O shell and record in `## Notes`). Governing principle 4 (read-only observer — the brain corpus is read, never written).

## Step-by-Step Tasks

### 1. Scaffold `src/brain/` + pure OKF reader (`okf.rs`) + fixtures
- Create `src/brain/mod.rs` as a module skeleton: declare `pub mod okf;` (and `graph`/`query` as `todo!()`-stubbed phase-labeled placeholders for Tasks 2–3), plus a `run()` entry stub labeled `// Phase 6 Block A — Task 4`.
- Add `mod brain;` to `src/main.rs` (declaration only; dispatch wiring is Task 4).
- Create `src/brain/okf.rs` — **pure, no I/O**:
  - Define the shared node/edge model: `BrainNode { id, title, path }` (id = the stable slug, e.g. frontmatter `name`/`title` or filename stem) and `BrainEdge { from, to }`.
  - `parse_okf_node(content, path) -> Option<BrainNode>` — reuse the `frontmatter.rs` parsing approach to pull the OKF id/title.
  - `extract_okf_links(content) -> Vec<String>` — a `[[link]]`-only extractor (distinct from `links.rs::extract_links`); handles `[[slug]]` and `[[slug|alias]]` forms.
  - `build_node_edge_lists(docs: &[(PathBuf, String)]) -> (Vec<BrainNode>, Vec<BrainEdge>)` — pure mapper from in-memory `(path, content)` pairs to node + edge lists; unresolved `[[link]]` targets handled deterministically (skipped or recorded — pick one and test it).
- Create `src/brain/fixtures/` — a small interlinked OKF corpus (≈4–6 `.md` files) mirroring the decision-graph shape, e.g. `d3.md`, `d20.md`, `d21.md` where `d20` is referenced by `d3` and `d21` via `[[d20]]`, with at least one chain for lineage and one unresolved link.
- **Tests (Rule 6):** exhaustively unit-test `parse_okf_node`, `extract_okf_links`, and `build_node_edge_lists` against the fixture corpus — frontmatter present/absent, single/multiple/aliased/unresolved links, empty doc.
- **Files:** *New* `src/brain/mod.rs`, `src/brain/okf.rs`, `src/brain/fixtures/*.md`; *Modified* `src/main.rs`.

### 2. Vendor the `knowledge_graph` algorithms → `graph.rs` (Dgraph-free)
- Create `src/brain/graph.rs` — a `BrainGraph` wrapper over `petgraph::graph::DiGraph<BrainNode, ()>`:
  - `BrainGraph::build(nodes, edges) -> BrainGraph` — the `(nodes, edges) → graph` build signature that is the **shared surface** Blocks B/C and Phase 8 consume; maintains an id→`NodeIndex` map.
  - Vendor/adapt the algorithm surface from the source crate decoupled from Dgraph/`Concept`/`Uuid`, reusing `petgraph` built-ins where they suffice: shortest path (`petgraph::algo::dijkstra` / `astar`), topological sort (`petgraph::algo::toposort`), and directed traversal (DFS/BFS reachability, forward and reverse).
  - Expose typed errors (e.g. unknown node id, cycle on toposort) rather than panicking.
- **Tests (Rule 6):** build graphs from fixture-derived node/edge lists and assert traversal / shortest-path / toposort on known shapes — linear chain, diamond DAG, cycle (toposort error path), unknown-node error path.
- **Files:** *New* `src/brain/graph.rs`; *Modified* `src/brain/mod.rs` (flip the `graph` stub to a real `pub mod graph;`).
- **Depends on:** Task 1 (node/edge model).

### 3. Structural queries → `query.rs` (dependents / blast-radius / lineage)
- Create `src/brain/query.rs` — **pure** functions over `BrainGraph`:
  - `dependents(graph, id) -> Vec<BrainNode>` — direct incoming-edge sources ("what depends on D20").
  - `blast_radius(graph, id) -> Vec<BrainNode>` — transitive reverse reachability (everything that breaks if `id` changes).
  - `lineage(graph, id) -> Vec<BrainNode>` — outgoing ancestry/path trace of a node's references.
  - Unknown-id returns a typed error (reuse Task 2's error type), not an empty success.
- **Tests (Rule 6):** against the fixture — `dependents("d20")` matches its known referrers; `blast_radius` includes transitive dependents; `lineage` traces the known chain; unknown-id error path.
- **Files:** *New* `src/brain/query.rs`; *Modified* `src/brain/mod.rs` (flip the `query` stub to `pub mod query;`).
- **Depends on:** Task 2 (`BrainGraph` + algorithm API).

### 4. Wire the `bastion brain` subcommand (thin I/O shell + CLI + dispatch)
- `src/brain/mod.rs`: implement `run()` — the thin I/O shell that walks a corpus root directory (default = the brain repo; overridable via `--root`), reads each `.md`/`.mdx` into `(path, content)` pairs, calls `okf::build_node_edge_lists` → `graph::BrainGraph::build` → the requested `query::*` function, and renders a **greppable** report (one result per line, e.g. `dependent: <id>\t<path>`). Keep the I/O shell thin over the already-tested pure core; graceful degradation (unreadable root, unknown node) prints a clear message and exits non-zero.
- `src/cli.rs`: add a `Brain` variant to `Commands` with mutually-exclusive query flags (`--dependents <id>`, `--blast-radius <id>`, `--lineage <id>`) and `--root <path>` (default to the brain corpus).
- `src/main.rs`: dispatch `Commands::Brain { .. }` to `brain::run`.
- **Tests:** unit-test any pure helpers added here (flag→query selection, report-line formatting). **Smoke test (Rule 6):** run `bastion brain --dependents <known-node>` against the live brain repo corpus, confirm output, and record the result in `## Notes`.
- **Files:** *Modified* `src/brain/mod.rs`, `src/cli.rs`, `src/main.rs`.
- **Depends on:** Tasks 1–3.

### 5. Validate
- Run the Validation Commands listed below and confirm all pass.
- Confirm no new Dgraph dependency was added to `Cargo.toml` and the crate builds with `petgraph` only (add a vendored dep only if `petgraph` genuinely does not suffice — justify in `## Notes`).

## Acceptance Criteria
- `bastion brain` returns the correct **dependents** and **lineage** for a known OKF node (e.g. a decision node's dependents match its stated `[[link]]` relations) when run against the live brain repo corpus.
- The graph is built from the file-derived OKF `[[link]]` corpus — markdown docs as nodes, `[[link]]` references as directed edges — with **no Dgraph dependency** in `Cargo.toml`.
- The pure OKF→graph builder (`okf.rs`), the `BrainGraph` algorithms (`graph.rs`), and the query functions (`query.rs`) are exhaustively unit-tested against the fixture corpus, including error/degrade paths (unresolved links, unknown node id, toposort cycle).
- The thin file-walk I/O shell in `brain::run` is smoke-tested against the live corpus and the result is recorded in `## Notes`.
- All gated checks pass: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
<filled in as work happens>

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
