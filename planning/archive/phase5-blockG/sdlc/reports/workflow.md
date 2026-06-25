---
type: sdlc/workflow-report
phase: phase5-blockG
date: 2026-06-21
---

# SDLC Workflow Report — phase5-blockG

**Date:** 2026-06-21
**Spec:** phase5-blockG
**Task scope:** All tasks
**Pipeline started from:** implement
**Review attempts:** 2 of 3 max

## Final Verdict

PASS — all 6 acceptance criteria MET and all 4 gating checks green after a targeted fix to the readiness-check string match in `src/sessions/ask.rs`.

## Stage Results

| Stage | Status | Report | Commit | Notes |
|---|---|---|---|---|
| implement | completed | planning/phase5-blockG/sdlc/reports/implement.md | 76c980b | Implemented bastion ask subcommand: CLI surface, pure helpers (done_path, trigger_text, poll_plan, has_session_args), AskError enum, thin I/O shell; 206 tests pass |
| test (attempt 1) | completed | planning/phase5-blockG/sdlc/reports/test.md | — | All 5 checks passed: fmt, clippy, test suite (206 passed; 0 failed), build --release |
| review (attempt 1) | PARTIAL | planning/phase5-blockG/sdlc/reports/review.md | — | All four gating checks pass and 5/6 criteria are MET; only gap: smoke-test notes placeholder not filled in tasks.md (Criterion 4) |
| fix (attempt 2) | completed | planning/phase5-blockG/sdlc/reports/implement.md | bd7190d | Fixed readiness check bug (exact 'claude' string failed because Claude Code v2.1.185 sets ucomm to its version string); replaced with classify_state==Running; recorded all 5 smoke-test scenarios in tasks.md ## Notes |
| test (attempt 2) | completed | planning/phase5-blockG/sdlc/reports/test.md | — | All 5 gating checks passed: fmt, clippy, test (206 passed, 2 ignored), build --release |
| review (attempt 2) | PASS | planning/phase5-blockG/sdlc/reports/review.md | — | All 6 acceptance criteria MET; all 4 gating checks pass (fmt, clippy, test, build); smoke-test notes complete |
| ui-test | SKIPPED | — | — | uiTest disabled in harness.json |
| document | completed | planning/phase5-blockG/sdlc/reports/document.md | d177e65 | Added bastion ask verb docs to sessions.md (full flag table, protocol steps, exit semantics, trust pre-flight, D4/D5 guarantees); updated docs/index.md sessions row |

## Key Findings

- **`bastion ask` implements the cross-repo brain contract v0.1.0** from `agentic-portfolio/docs/integrations/claude-code-llm-provider.md` §2 verbatim: flag names, trigger wording, `<out>.done` marker, and exit semantics (0 only on success; non-zero with stderr on timeout/failure).
- **Process-name rename bug (discovered during smoke test):** Claude Code v2.1.185 sets its `ucomm` via `pthread_setname_np` to its version string `"2.1.185"`. tmux's `#{pane_current_command}` reports `ucomm`, so the original `foreground.trim() == "claude"` check never passed. Fixed by using `classify_state(&foreground) == SessionState::Running` (any non-idle-shell foreground counts as "Claude is up"), making the check robust to future version renames.
- **Cold-start timing gap (noted, not fixed):** On the first cold-start smoke run, the trigger was sent the moment `classify_state` returned Running — before Claude Code's TUI fully initialized — causing a timeout. The second attempt (warm session) succeeded. A future improvement could add a short fixed delay (1–2s) after readiness detection. Out of scope for this block.
- **D4 (DB-free) and D5 (synchronous) preserved:** `src/sessions/ask.rs` imports no `Config`, `Pool`, or `DATABASE_URL`; no `async`/`await`/`tokio` on this path. Verified by unit test `pure_helpers_require_no_database_url` and code inspection.

## Files Modified

| File | Action |
|---|---|
| `src/cli.rs` | Modified — added `Ask` subcommand with all required flags and defaults |
| `src/main.rs` | Modified — dispatch `Commands::Ask` → `sessions::ask::ask(...)` |
| `src/sessions/mod.rs` | Modified — registered `pub mod ask;` |
| `src/sessions/ask.rs` | Created — pure helpers + AskError enum + thin I/O `ask()` fn; bug fix in pass 2 |
| `planning/phase5-blockG/tasks.md` | Created — spec file with smoke-test notes added in fix pass |

## Docs Updated

| Doc File | Change |
|---|---|
| `docs/sessions.md` | Added full `bastion ask` section: flags table, protocol steps, exit semantics, trust pre-flight, D4/D5 guarantees; updated block-completion footer |
| `docs/index.md` | Added `ask` to the verb list in the sessions.md description row |
| `agentic-portfolio/docs/integrations/claude-code-llm-provider.md` | **NEEDS_REVIEW** — §3 should be updated in the parent repo to mark Block G as done and unblock the orchestrator's `CLAUDE_CODE_SESSION` provider implementation. Out of bastion's doc surface; not edited here. |

## Commits (this pipeline run)

```
d177e65 docs: update docs for phase5-blockG
bd7190d fix: fix pass 2 for phase5-blockG — readiness check + smoke test notes
76c980b feat: implement phase5-blockG — bastion ask subcommand
```
