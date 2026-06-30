# Worklog — 11.C-websocket-hub

## Task 1 — PASSED (1 attempt)
What: Extended WsFrameKind with 7 v0.2 variants, added 6 payload structs, and added Topic enum + parse_topic() pure parser with exhaustive unit tests (848 tests pass, fmt/clippy/build clean).
Decisions: Topic enum is not serde-derived since it is an internal parsed form, not a wire type — the raw topic string in SubscribePayload carries the wire form; parse_topic rejects empty pane names (pane: with no suffix) returning None, matching the spec requirement
Validated: gating checks (fast tripwire)

## Task 2 — PASSED (1 attempt)
What: Add src/serve/status/ needs-input detection adapter: OnceLock-compiled Claude manifest, needs_input() and detect_state() pure fns, fixtures, and 6 unit tests covering all required paths.
Decisions: detect_state() passthrough included proactively per the breakdown note (Task 4 will need it for debounce; shipping it now avoids a cross-task edit later); no_input.txt fixture uses the 'esc to interrupt' working pattern from claude_working.txt to ensure AgentState::Working is returned, satisfying the working-pane test
Validated: gating checks (fast tripwire)

## Task 3 — PASSED (1 attempt)
What: Created src/serve/poll.rs with pure pane-diff logic (diff_pane, PaneCursor::observe, sessions_snapshot) and added pub mod poll to src/serve/mod.rs; 875 tests pass.
Decisions: extract_lines() is a private helper that mirrors Pane::last_lines(None) semantics rather than calling it directly, since poll.rs operates on raw &str captures not Pane structs; PaneCursor uses Default derive so seq starts at 0 and first push yields seq=1 naturally
Validated: gating checks (fast tripwire)
