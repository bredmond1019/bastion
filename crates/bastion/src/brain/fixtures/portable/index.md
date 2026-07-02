---
type: Index
title: Portable Fixtures Index
description: Small interlinked OKF corpus in a client/project knowledge domain — used to prove the brain reader is not hardcoded to the bastion decision graph.
doc_id: portable-index
status: active
keywords: [fixture, portability, test, okf, knowledge-base]
---

# Portable Fixtures

A second OKF corpus used as portability test fixtures for `src/brain/okf.rs`.
Uses a **client/project knowledge** domain, distinct from the decision-graph domain
of the sibling `fixtures/` directory.

| File | doc_id | Title | References |
|---|---|---|---|
| `proj-overview.md` | `proj-overview` | Project Overview | [[req-doc]], [[team-roster]] |
| `team-roster.md` | `team-roster` | Team Roster | [[proj-overview]] |
| `req-doc.md` | `req-doc` | Requirements Document | [[tech-spec]], [[team-roster]] |
| `tech-spec.md` | `tech-spec` | Technical Specification | [[req-doc]] |
| `stale-note.md` | `stale-note` | Stale Note | [[missing-page]] (unresolved) |

Graph shape:
- proj-overview → req-doc → tech-spec (lineage chain)
- req-doc → team-roster (cross-reference)
- proj-overview → team-roster (cross-reference)
- team-roster → proj-overview (back-ref, mutual ref with proj-overview)
- tech-spec → req-doc (back-ref)
- stale-note → missing-page (unresolved — target not in corpus)
