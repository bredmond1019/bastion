# Worklog — 11.C-websocket-hub

## Task 1 — PASSED (1 attempt)
What: Extended WsFrameKind with 7 v0.2 variants, added 6 payload structs, and added Topic enum + parse_topic() pure parser with exhaustive unit tests (848 tests pass, fmt/clippy/build clean).
Decisions: Topic enum is not serde-derived since it is an internal parsed form, not a wire type — the raw topic string in SubscribePayload carries the wire form; parse_topic rejects empty pane names (pane: with no suffix) returning None, matching the spec requirement
Validated: gating checks (fast tripwire)

## Task 2 — PASSED (1 attempt)
What: Add src/serve/status/ needs-input detection adapter: OnceLock-compiled Claude manifest, needs_input() and detect_state() pure fns, fixtures, and 6 unit tests covering all required paths.
Decisions: detect_state() passthrough included proactively per the breakdown note (Task 4 will need it for debounce; shipping it now avoids a cross-task edit later); no_input.txt fixture uses the 'esc to interrupt' working pattern from claude_working.txt to ensure AgentState::Working is returned, satisfying the working-pane test
Validated: gating checks (fast tripwire)
