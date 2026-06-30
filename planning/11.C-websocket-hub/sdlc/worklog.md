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

## Task 4 — PASSED (1 attempt)
What: Implement Hub + WsConn actors with topic subscriptions, ref-counted per-pane polls, PaneCursor diff, needs-input rising-edge debounce, and pure classify_inbound dispatch seam — 38 new unit tests, all 907 pass
Decisions: Used ConnId(u64) with AtomicU64 counter instead of Uuid (uuid is not a dependency — per breakdown note); For pane poll FnMut closure, cloned pane_name into separate name_for_block and name_for_then per tick to satisfy borrow checker without using Arc; send_keys already appends Enter internally (two tmux invocations: literal + Enter); WS Send handler just calls tmux::send_keys which handles the Enter press — no double-Enter issue; Used let-chain syntax (Rust 2024 edition) for clippy::collapsible_if fixes: `if cond && let Some(h) = opt.take() { ... }`; fan_out helper method on Hub is unused directly (handlers inline the iteration for clarity); kept it for future use — will be pruned if clippy flags it in Task 5; Task 4 does NOT wire the /ws route to the hub (that is spec Task 5 — the only non-additive serve/mod.rs edit)
Validated: gating checks (fast tripwire)
