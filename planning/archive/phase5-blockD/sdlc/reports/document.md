---
type: DocumentReport
title: Phase 5 Block D — bastion capture
---

# Documentation Report — phase5-blockD

**Date:** 2026-06-21
**Spec:** planning/phase5-blockD/tasks.md
**Verdict gate:** PASS (confirmed)

## Docs Patched

| Doc File | Section Updated | Change Summary |
|---|---|---|
| docs/sessions.md | frontmatter description | Added `capture` to the verb list in the description field |
| docs/sessions.md | Verb reference | Added `bastion capture <session> [--lines N]` verb section with usage examples and explanation of trailing-blank stripping |
| docs/sessions.md | Error behavior table | Extended unknown-session row to include `capture` alongside `attach` / `kill` / `send` |
| docs/sessions.md | Footer note | Updated "Blocks A–C surface / Block D planned" to "Blocks A–D surface / Block E planned" |
| docs/index.md | Doc table | Added `capture` to the sessions.md row's verb list |

## Docs Flagged NEEDS_REVIEW

None. The changes are confined to the session-control surface; no top-level architecture or core wiring docs are affected.

## Docs Clean (checked, no changes needed)

- docs/data-contract.md — unrelated to session-control verbs; no reference to `capture`, `Pane`, or `last_lines`.
