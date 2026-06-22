---
type: DocumentReport
title: Phase 3 Block A — bastion run
---

# Documentation Report — phase3-blockA

**Date:** 2026-06-22
**Spec:** planning/phase3-blockA/tasks.md
**Verdict gate:** PASS (confirmed)

## Docs Patched

| Doc File | Section Updated | Change Summary |
|---|---|---|
| docs/run.md | (new file) | Created operator reference for `bastion run` covering usage, flags, output format, degrade paths, and key internals (`parse_args`, `format_trigger_success`, `trigger_body`, `trigger_url`, `trigger_workflow`). Follows the established per-command doc pattern (see `costs.md`, `monitor.md`). |

## Docs Flagged NEEDS_REVIEW

- **docs/index.md** — top-level navigation table. Needs a new row added for `run.md`:
  `| [run.md](run.md) | Workflow trigger — \`bastion run <workflow> [--args '{}'] [--monitor]\`: POST to orchestrator, print task_id, optional monitor hand-off |`
  Not edited directly per documentation agent instructions.

## Docs Clean (checked, no changes needed)

- **docs/data-contract.md** — the `### Trigger → api::client::trigger_workflow` section (lines 83–85) already correctly describes `POST /` with `{ "workflow_type": str, "data": object }` → `202 { "task_id": str, "message": str }`. The implementation fulfils this contract exactly; no field mapping changes needed.
- **docs/monitor.md** — not referenced by the changed files; no updates needed.
- **docs/inspect.md** — not referenced by the changed files; no updates needed.
- **docs/costs.md** — not referenced by the changed files; no updates needed.
- **docs/sessions.md** — not referenced by the changed files; no updates needed.
- **docs/claude-code-workflow.md** — not referenced by the changed files; no updates needed.
