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
