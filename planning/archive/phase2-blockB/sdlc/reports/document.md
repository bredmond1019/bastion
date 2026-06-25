---
type: DocumentReport
title: "Documentation Report — Phase 2, Block B (bastion costs)"
block: phase2-blockB
status: complete
---

# Documentation Report — phase2-blockB

**Date:** 2026-06-22
**Spec:** planning/phase2-blockB/tasks.md
**Verdict gate:** PASS (confirmed)

## Docs Patched

| Doc File | Section Updated | Change Summary |
|---|---|---|
| `docs/costs.md` | (new file) | Created full operator reference for `bastion costs --last <window>`: usage, output format, pricing model, degrade paths, and key internals table |
| `docs/index.md` | Navigation table | Added `costs.md` row between `inspect.md` and `claude-code-workflow.md` |
| `docs/data-contract.md` | "Read path" section | Split into "Monitor / Inspect" and new "Costs" subsections; added description of `db::costs::fetch_all_runs` read path (full-table SELECT + shared `parse_event_row`); added `db::costs` to the re-pin checklist |

## Docs Flagged NEEDS_REVIEW

None. The new `costs` command is self-contained (no changes to core routing, CLI wiring, or
cross-repo contracts beyond what is already documented above).

## Docs Clean (checked, no changes needed)

- `docs/inspect.md` — no overlap with costs implementation
- `docs/monitor.md` — no overlap with costs implementation
- `docs/sessions.md` — no overlap with costs implementation
- `docs/claude-code-workflow.md` — no overlap with costs implementation
