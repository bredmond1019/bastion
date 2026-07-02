# Worklog — 13.1-persistent-agent-panel

## Task 1 — PASSED (1 attempt)
What: Extracted pure session_urgency(&Session) -> u8 out of build_mission_items in src/monitor/app.rs, reused it in build_mission_items, and added exhaustive unit tests covering all four AgentState values plus SessionState::Running, plus a build_mission_items ordering regression test.
Decisions: Kept build_mission_items's existing logic for WorkflowRun urgency untouched (only the Session branch was factored into session_urgency), since the task scoped only the session ordering extraction.; Added a make_session test helper in monitor/app.rs's tests module rather than reusing any existing fixture, since none existed for Session construction in this file.
Validated: gating checks (fast tripwire)

## Task 2 — PASSED (1 attempt)
What: Added a pure agent_panel_rows builder + AgentPanelRow model in src/sessions/agent_panel.rs that maps sessions to rows sorted by session_urgency (Blocked first), registered as a new sessions module, with 5 unit tests covering empty-slice, one-row-per-session, sort ordering, row content, and Running-state parity.
Decisions: AgentPanelRow carries only label + AgentState per spec (no theme/color fields) to keep the builder pure; theme mapping is deferred to Task 3's render shell.; Reused monitor::app::session_urgency directly rather than duplicating urgency logic, since it was already extracted pure in Task 1.
Validated: gating checks (fast tripwire)

## Task 3 — PASSED (1 attempt)
What: src/sessions/ui.rs now reserves and renders an always-on bottom "agents · priority" strip (themed, urgency-sorted, min-height fallback) under every SelectedNode in the session dashboard.
Decisions: Strip height is computed by a new pure fn agent_panel_strip_height(row_count, frame_height): grows from 3 to a 7-line cap with session count, shrinks toward 0 (never underflows/panics) when the frame can't spare room beyond the 1-line main area + 1-line footer — this is the 'min-height fallback'.; Dot glyph/color mapping for AgentState lives in a small agent_state_dot() helper mirroring the existing build_space_item pattern (reads ui_theme::state_*_style(), never literal colors); not unit-tested in isolation (consistent with the existing build_space_item precedent) but covered via the render-based tui_tests.rs assertions.; Left planning/13.1-persistent-agent-panel/tasks.md's pre-existing '(in progress)' marker diff untouched/unstaged since it predates this task's work and is pipeline-owned state.
Validated: gating checks (fast tripwire)

## Task 4 — PASSED (1 attempt)
What: Validated BA.13.1 (fmt/clippy/test/release build all green after fixing a clippy collapsible-if lint) and manually smoke-tested the agents-priority strip via tmux capture-pane across Mission Control, a tier, and a space; results recorded in tasks.md Notes.
Decisions: Fixed a clippy::collapsible_if warning surfaced by --all-targets in the task-3 test code (nested if-let/if in build_space_item_working_dot_tracks_runtime_theme) by collapsing to a let-chain — no behavior change, required for the clippy gate to pass.; Used cargo clippy --all-targets rather than plain cargo clippy since the spec's harness/CLAUDE.md gate must cover test code too; the plain command alone would have missed the test-only warning.
Validated: gating checks (fast tripwire)

## Docs
Patched: docs/sessions.md

## Wrap-up — PASS
Next: Pick the next Phase 13/14 block per state.json's focus.next ordering: BA.13.2 / BA.13.3 / BA.13.5 (Phase 13), or BA.14.1 / BA.14.2 / BA.14.3 (color pass, unblocked by BA.14.0), or resume Phase 15 (bastion-product packaging plan).

## PR
https://github.com/bredmond1019/bastion/pull/12
