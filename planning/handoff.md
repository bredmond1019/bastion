---
type: Handoff
title: Handoff — BA.11.C complete; WebSocket hub shipped; next block or BA.7.B
description: Session handoff after shipping the full WebSocket hub (BA.11.C) — topic subscriptions, live pane diff-push, needs-input detection, 908 tests, PASS.
doc_id: handoff
layer: [console]
project: bastion
status: active
keywords: [handoff, websocket, hub, phase 11, BA.11.C, BA.7.B]
related: [bastion-status, master-plan]
created: 2026-06-30
---

# Handoff — BA.11.C complete; WebSocket hub shipped; next block or BA.7.B

> **For the next agent:** Read this immediately after `/prime`. Delete this file once consumed.

## What we're doing and why

We are building Bastion's Phase 11 — the live pane streaming + agent-state layer that
BastionUI (D28) needs to show agents as Idle / Working / Blocked in real time.

BA.11.C (the WebSocket hub + live pane streaming) shipped this session with a clean PASS
and is now on `main` (merged from branch `11.C-websocket-hub-flow`, PR #8). The hub wires
together the detection engine (BA.11.C0, shipped last session) with live WebSocket push to
BastionUI clients. Phase 11's real-time streaming core is now complete.

## Completed this session

- **BA.11.C fully implemented and merged** — WebSocket hub + live pane streaming:
  - `src/serve/dto.rs` — 7 new `WsFrameKind` variants, 6 payload structs (`SubscribePayload`, `SendPayload`, `SendKeyPayload`, `SessionsPayload`, `PanePayload`, `EventPayload`), `Topic` enum, `parse_topic()` parser
  - `src/serve/status/mod.rs` + `detect.rs` — `OnceLock`-compiled Claude manifest adapter; `needs_input(pane) -> bool` + `detect_state()` seam for the hub's debounce
  - `src/serve/poll.rs` — pure pane-diff logic: `diff_pane`, `PaneCursor::observe` (seq-bumping diff cursor), `sessions_snapshot`; fully unit-tested without I/O
  - `src/serve/ws/server.rs` — `Hub` actix actor: ref-counted per-pane poll tasks, topic subscription tracking, `PaneCursor` diff fan-out, rising-edge needs-input debounce; 38 pure unit tests; `ConnId` uses `AtomicU64` (no uuid dep)
  - `src/serve/ws/session.rs` — `WsConn` actix actor: per-connection lifecycle (Connect/Disconnect/Subscribe/Unsubscribe/Send/SendKey frame dispatch)
  - `/ws` route upgraded from echo actor to hub-backed handler in `src/serve/mod.rs`
  - `docs/serve-api.md` bumped to v0.2 — full topic/frame/event/disconnect docs
  - **908 tests pass, PASS verdict, zero review findings**
- **Light code review (low)** ran post-flow — clean, no findings
- **Worktree merged and cleaned** — `11.C-websocket-hub-flow` merged FF into `main`, worktree removed, branch deleted
- **PR #8** opened on GitHub

## Remaining work

1. **Identify and start the next Phase 11 block** — check `planning/master-plan.md` Phase 11
   for the next un-started block after BA.11.C (likely BA.11.D or later).
2. **BA.7.B** (vendor tiktoken counter → exact `bastion costs`) remains as a lower-priority
   interleave if the next Phase 11 block hits a blocker.
3. **Push local main to remote** — local `main` is 34+ commits ahead of `origin/main`.
4. **Minor improvement backlog** (non-blocking, from status.md):
   - Confirm `bastion validate` skips `trees/` if worktrees accumulate `.md` files
   - `status` config-file API URL not loaded when `DATABASE_URL` absent
   - `blank_code_spans` handles single-backtick inline spans only (by design)

## Open questions / choices

None — clear to proceed. Check the master-plan for the next Phase 11 block and spin it up
with `/sdlc-flow <spec-slug>`.

## Context the next agent needs

- **908 tests** is the current baseline after BA.11.C.
- **Hub actor pattern:** `Hub` is a process-singleton actix actor started in `run_server()` (`src/serve/mod.rs`). It owns all subscription state and all poll `SpawnHandle`s. Per-connection `WsConn` actors hold an `Addr<Hub>`. Runtime model unchanged from Block A: `actix_web::rt::System::new().block_on(...)` on a dedicated OS thread via `tokio::task::spawn_blocking`.
- **`detect()` seam:** `src/serve/status/detect.rs` — `needs_input(pane: &str) -> bool` wraps the detection engine. To change what counts as "blocked", edit `src/detect/manifests/claude.toml`, not the hub.
- **`serve-api.md` at v0.2.** Any new frame kinds, routes, or event types must be documented there before the next block ships.
- **AGENT.md and GEMINI.md** are untracked files at the repo root — not part of this project; ignore them.
- **PR #8** is open on GitHub; local `main` is the source of truth (already has all the work merged).

## First command after `/prime`

`grep -A 5 "11\.C\|11\.D\|Phase 11" planning/master-plan.md | head -40`
