---
type: Index
title: Brain Fixtures Index
description: Small interlinked OKF corpus used as test fixtures for the brain module.
---

# Brain Fixtures

This directory contains a small decision-graph corpus used as test fixtures for
`src/brain/okf.rs` and `src/brain/graph.rs`.

| File | Title | References |
|---|---|---|
| `d3.md` | D3 — Use petgraph | [[d20]] |
| `d20.md` | D20 — Shared data contract | [[d21]], [[d3]] |
| `d21.md` | D21 — DB-free session surface | [[d20]], [[d4]] |
| `d4.md` | D4 — Synchronous session commands | (leaf) |
| `unlinked.md` | Unlinked node | (leaf, no incoming) |

Graph shape:
- d3 → d20 → d21 → d4 (chain for lineage)
- d20 → d3 (back-ref, creates a cycle in the undirected sense but DAG in directed)
- d21 → d20 (back-ref completing the d20↔d21 mutual ref)
- unlinked: isolated, no references to or from main cluster
