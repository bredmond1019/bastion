---
type: TaskSpec
title: "Task Spec — Phase 11, Block B: Session REST + named-key helper"
description: "Extend bastion serve with session REST endpoints (list/pane/send/key/create/delete) wrapping sessions::tmux via web::block, add a tmux named-key helper for Escape/arrows/bare-Enter, and bump the serve-api contract to v0.1."
status: not-started
phase: 11
block: B
---

# Task Spec — Phase 11, Block B: Session REST + named-key helper

**Status:** Not started · **Last run:** never

## Goal
Project the existing tmux session-control surface onto `bastion serve` as a REST API — list sessions, read a pane, send keystrokes, send named keys (Escape/arrows/bare-Enter), create and delete sessions — wrapping the synchronous `sessions::tmux` functions via `web::block`, and bump `docs/serve-api.md` to v0.1.

## Context Pointers
- **Plan:** `planning/master-plan.md` → *Phase 11 — BastionUI Console API* → *Block B — Session REST + named-key helper (prog. D)*. This is the first real data surface on `bastion serve`; it builds directly on the Block A scaffold (PR #5, merged) and is the daily-friction win for the Flutter `bastion-ui` Surface.
- **Cross-repo governance:** brain **D28** (BastionUI program), upholding **D21** (reuse the `pub` `sessions::tmux`/`model` substrate — do not duplicate tmux logic) and **D25** (read-only state / triggered mutations: session create/delete/send are legitimate operator mutations of tmux, *not* of the orchestrator DB). The `docs/serve-api.md` contract is **D20-style** — this repo produces + versions it; `bastion-ui` pins it.
- **Existing substrate (verified):**
  - `src/sessions/tmux.rs` — `pub` blocking fns: `list_sessions_raw()`, `capture_pane_raw(name)`, `new_session(name, dir)`, `kill_session(name)`, `send_keys(name, keys)` (literal `-l`, **cannot** send named keys), plus pure `*_args` builders and `classify_no_server(stderr)`. The `send_keys_args`/`send_enter_args` pattern (literal `-l --` vs named `Enter`) is the template for the new named-key helper.
  - `src/sessions/model.rs` — `parse_sessions(raw) -> Vec<Session>`, `Session`/`SessionState`/`Pane` derive only `Debug, Clone` (no serde → DTOs required), `Pane::new`, `Pane::last_lines(Option<usize>)`, `classify_state(cmd)`.
  - `src/serve/mod.rs` — `run_server` builds an actix `App` with a protected `web::scope("/api").wrap(BearerAuthMiddleware…)` placeholder (currently empty) plus a separate `/ws` scope. **Session routes mount under the existing `/api` protected scope** (final paths `/api/sessions…`), inheriting bearer auth — follow the established convention; the master-plan's `/sessions` shorthand resolves to `/api/sessions`.
  - `src/serve/dto.rs` — independent serde DTOs (`HealthResponse`, `WsFrame`, `ErrorPayload`); new session DTOs live here.
  - `src/observ/errors.rs` — `ConsoleError` / `ErrorCode` (C001–C014) for mapping tmux degradation to typed codes; `ErrorPayload.code` is the string carrier.
- **Standing rules (`CLAUDE.md`):** Rule 1 (tests ship with every block), Rule 2 (OKF frontmatter on every `.md`), Rule 6 (coverage bar — pure logic exhaustively unit-tested without I/O incl. error/degradation paths; the thin I/O shell smoke-tested + recorded in `## Notes`), Rule 7 (adding a file to a directory updates that directory's `index.md` — `docs/index.md` already rows `serve-api.md`; no new doc file added).

## Step-by-Step Tasks

### 1. Named-key tmux helpers (`src/sessions/tmux.rs`)
- Add `send_named_key_args(session_name, key) -> Vec<String>` building `tmux send-keys -t <name> <KeyName>` (**no `-l`, no `--`**) so tmux resolves the key name (`Escape`, `Enter`, `Up`/`Down`/`Left`/`Right`, `C-c`, etc.) — the verified gap that literal `send_keys` cannot fill.
- Add `send_named_keys_args(session_name, keys: &[String]) -> Vec<String>` appending each key name as a separate argv element (one `send-keys` call delivering a sequence).
- Add the thin execution shells `send_named_key(name, key)` / `send_named_keys(name, keys)` over `run_tmux`, mirroring the existing `send_keys` shell.
- Pure element-wise `*_args` unit tests mirroring the existing `send_keys_args_*` tests: assert exact argv ordering, assert **no** `-l`/`--` present, cover a single key, a multi-key sequence, and a key with a hyphen-style modifier (`C-c`).
- **Owns:** `src/sessions/tmux.rs` only. No dependencies.

### 2. Session + Pane request/response DTOs (`src/serve/dto.rs`)
- Add serde structs: `SessionDto` (name, state-as-string, last-line) with a `From<&Session>`/constructor mapping from the domain `Session` (+ `SessionState::as_str`); `PaneDto` (session name, `lines: Vec<String>`) built from `Pane::last_lines`.
- Add request-body DTOs: `SendBody { keys: String }`, `KeyBody { key: String }` (or `keys: Vec<String>` for sequences — pick one and document), `NewSessionBody { name: String, dir: Option<String> }`.
- Per Rule 6, unit-test each DTO: serialize shape (field names/values), round-trip, and a rejects-missing-required-field case — matching the existing `dto.rs` test style.
- **Owns:** `src/serve/dto.rs` only (append-only additions). No dependencies.

### 3. Session REST handlers (`src/serve/handlers/sessions.rs` + `src/serve/handlers/mod.rs`)
- Create the new `handlers/` submodule (`src/serve/handlers/mod.rs` declaring `pub mod sessions;`).
- Implement async handlers, each wrapping the synchronous tmux fns in `web::block` and mapping results to JSON DTOs / HTTP statuses:
  - `GET /sessions` → `list_sessions_raw` + `parse_sessions` → `Vec<SessionDto>`.
  - `GET /sessions/{name}/pane?lines=N` → `capture_pane_raw` + `Pane::last_lines` → `PaneDto`.
  - `POST /sessions/{name}/send` (`SendBody`) → `send_keys` then `send_enter` (literal + Enter), 204/200.
  - `POST /sessions/{name}/key` (`KeyBody`) → `send_named_key`/`send_named_keys` (Task 1), 204/200.
  - `POST /sessions` (`NewSessionBody`) → `new_session`, 201.
  - `DELETE /sessions/{name}` → `kill_session`, 204.
- Map tmux degradation to clean statuses via a pure helper (e.g. `tmux_error_to_status(&anyhow::Error) -> (StatusCode, ErrorPayload)`): tmux-not-installed/no-server → `503`; unknown session (kill/send/capture on a missing target) → `404`; other → `500`. Reuse `observ` `ErrorCode` strings for `ErrorPayload.code`. Unit-test this pure mapping helper across each branch.
- **Owns:** `src/serve/handlers/` (new dir). **Depends on:** Task 1 (named-key fns), Task 2 (DTOs).

### 4. Route wiring + integration tests (`src/serve/mod.rs`)
- Add `pub mod handlers;` to `src/serve/mod.rs`.
- Mount the six routes inside the existing protected `web::scope("/api")` so they inherit `BearerAuthMiddleware` (paths `/api/sessions…`); register the same routes in the `build_app()` test helper so test routing mirrors production exactly.
- Integration tests via `actix_web::test`: each route rejects a missing/wrong bearer token with `401`; `GET /api/sessions` returns `200` with a JSON array shape; method/path mapping is correct (unknown method → `405`). (Live tmux behaviour is smoke-tested, not asserted in-process — Rule 6.)
- **Owns:** `src/serve/mod.rs` only. **Depends on:** Task 2, Task 3.

### 5. Extend the serve-api contract to v0.1 (`docs/serve-api.md`)
- Bump **Version** to v0.1; add a `## Sessions` section documenting all six routes: method, path (`/api/sessions…`), `lines` query param, request bodies (`SendBody`/`KeyBody`/`NewSessionBody`), response DTOs (`SessionDto`/`PaneDto`), and the named-key endpoint with the set of accepted key names (Escape, Enter, arrows, `C-c`).
- Document the degradation → HTTP status mapping (503/404/500) and the `ErrorPayload` body shape.
- Append a dated line to the doc's Amendment Log recording the v0 → v0.1 delta.
- **Owns:** `docs/serve-api.md` only. **Depends on:** the surface settled in Tasks 1–4.

### 6. Validate
- Run the Validation Commands listed below and confirm all pass.
- Smoke-test the live I/O shell against a real tmux server (`curl` the running `bastion serve`): list sessions, read a pane, send keys, send `Escape`, create and kill a session; record the result in `## Notes` per Rule 6.

## Acceptance Criteria
- `src/sessions/tmux.rs` exposes `send_named_key`/`send_named_keys` (+ `*_args` builders) that emit `send-keys <KeyName>` **without** `-l`/`--`, proven by element-wise `*_args` unit tests.
- `bastion serve` serves all six session routes under the bearer-protected `/api` scope: `GET /api/sessions`, `GET /api/sessions/{name}/pane?lines=N`, `POST /api/sessions/{name}/send`, `POST /api/sessions/{name}/key`, `POST /api/sessions`, `DELETE /api/sessions/{name}`.
- A live `curl` smoke test lists sessions, reads a pane, sends keys, sends `Escape`, and creates + kills a session against a running server (recorded in `## Notes`).
- Every route rejects a missing/wrong bearer token with `401` (asserted in-process).
- tmux degradation maps to documented HTTP statuses (tmux/no-server → 503, unknown session → 404, other → 500) via a unit-tested pure helper.
- `SessionDto`/`PaneDto` and the request-body DTOs have serde round-trip + missing-field unit tests.
- `docs/serve-api.md` is at v0.1 documenting the session routes, named-key endpoint, DTO shapes, and status mapping, with an Amendment Log entry.
- All gated checks pass (`cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`).

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes

### Task 6 — Validation smoke test (2026-06-26)

**Validation commands:** all four pass on the 11.B-session-rest-flow branch.
- `cargo fmt --check` — clean
- `cargo clippy -- -D warnings` — no warnings
- `cargo test` — 775 tests passed, 0 failed, 3 ignored
- `cargo build --release` — clean build

**Live smoke test against `bastion serve` on `127.0.0.1:18080` with `BASTION_SERVE_TOKEN=smoke-test-token`:**

1. `GET /api/sessions` — returned `[{"name":"test-bastion","state":"idle","last_line":""}]` (HTTP 200)
2. `GET /api/sessions/test-bastion/pane?lines=5` — returned `{"session_name":"test-bastion","lines":["~/Dev/agentic-portfolio/bastion main > ..."]}` (HTTP 200)
3. `POST /api/sessions/test-bastion/send` with `{"keys":"echo hello from bastion"}` — HTTP 204
4. `POST /api/sessions/test-bastion/key` with `{"key":"Escape"}` — HTTP 204
5. `POST /api/sessions` with `{"name":"smoke-test-new"}` — HTTP 201; session visible in subsequent list
6. `DELETE /api/sessions/smoke-test-new` — HTTP 204; session gone
7. `GET /api/sessions` with no `Authorization` header — HTTP 401 `{"code":"unauthorized","error":"unauthorized"}`

All six routes respond correctly; bearer auth enforced on every session route; 401 on missing token confirmed.

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
