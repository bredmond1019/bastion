# Worklog — 11.C-websocket-hub

## Task 1 — PASSED (1 attempt)
What: Extended WsFrameKind with 7 v0.2 variants, added 6 payload structs, and added Topic enum + parse_topic() pure parser with exhaustive unit tests (848 tests pass, fmt/clippy/build clean).
Decisions: Topic enum is not serde-derived since it is an internal parsed form, not a wire type — the raw topic string in SubscribePayload carries the wire form; parse_topic rejects empty pane names (pane: with no suffix) returning None, matching the spec requirement
Validated: gating checks (fast tripwire)
