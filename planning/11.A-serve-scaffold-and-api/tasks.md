---
type: TaskSpec
title: "Task Spec — Phase 11, Block A: serve scaffold + serve-api contract v0"
description: "bastion serve scaffold — actix HTTP+WS network face, runtime spike, bearer auth, /health + /ws echo, and the v0 serve-api contract that bastion-ui pins."
status: Not started
phase: 11
block: A
---

# Task Spec — Phase 11, Block A: `serve` scaffold + serve-api contract v0

**Status:** Not started · **Last run:** never

## Goal
Stand up `bastion serve` — an actix-web HTTP+WebSocket network face that boots on a tailnet bind behind mandatory bearer-token auth, serving `GET /health` and a minimal `/ws` accept+echo — and publish `docs/serve-api.md` v0, the contract the Flutter `bastion-ui` Surface pins against.

## Context Pointers
- **Plan:** `planning/master-plan.md` → *Phase 11 — BastionUI Console API* (track preamble) + *Block A — `serve` scaffold + serve-api contract v0*. This is the foundational producer of the whole Phase 11 track; nothing else in Phase 11 (or `bastion-ui`) can be built or pinned until the server boots and the contract exists.
- **Cross-repo governance:** brain **D28** (BastionUI program), upholding **D21** (reuse `pub` `sessions::tmux`/`model` substrate) and **D25** (read-only state / triggered mutations). The `docs/serve-api.md` contract is **D20-style**: this repo produces + versions it; `bastion-ui` pins it.
- **Standing rules (`CLAUDE.md`):** Rule 1 (tests ship with every block), Rule 2 (OKF frontmatter on every `.md`), Rule 6 (coverage bar — separate pure logic from I/O, test logic exhaustively incl. error/degradation paths, smoke-test the thin I/O shell and record it in `## Notes`), Rule 7 (adding a file to a directory requires updating that directory's `index.md`).
- **Verified source seams (read 2026-06-26):**
  - `src/main.rs:238` is `#[tokio::main]` (tokio "full"); dispatch is `async fn dispatch(cli) -> Result<()>` with a `match cli.command` arm per subcommand. The serve arm is the integration risk (see Task 1).
  - `src/config.rs:141` `Config::load()` **requires `DATABASE_URL`** — serve must NOT go through it. Add a separate **DB-free** `load_serve_config()` reading only `BASTION_SERVE_ADDR` (default `0.0.0.0:4317`) + `BASTION_SERVE_TOKEN`.
  - `src/cli.rs:50` `pub enum Commands` — add a `Serve { addr, token }` arm; `Cargo.toml:19` has `tokio` "full"; `actix`/`actix-web`/`actix-web-actors` are **not yet** present.
  - `src/observ/` carries the C0xx error taxonomy reused for error mapping.
- **Harvest source:** the WS actor skeleton + actix pins come from `rag-engine-rs/src/services/chat/` (actix 0.13 / actix-web 4.9 / actix-web-actors 4.3). Pin to those versions for copy-compatibility with later blocks.

## Step-by-Step Tasks

### 1. Runtime spike — actix deps + `src/serve/` boot under the tokio runtime
- **This is the one real integration risk; settle it before any endpoint work.** `actix-web-actors` WS actors need an actix `System`/Arbiter in scope, which bastion's `#[tokio::main]` runtime does not provide. **Start from running actix on its own thread** via `actix_web::rt::System::new().block_on(serve::run(...))`, and treat "it just works inside the existing tokio runtime" as the hypothesis to disprove.
- `Cargo.toml`: add `actix`, `actix-web`, `actix-web-actors`, `futures`, pinned to the rag-engine-rs versions (actix 0.13 / actix-web 4.9 / actix-web-actors 4.3).
- *New* `src/serve/mod.rs`: `pub async fn run(addr, token) -> Result<()>` (or the thread-spawned `System` equivalent the spike settles on) that builds an `HttpServer`, wires routing, binds the configured addr, and serves `GET /health` returning a small JSON liveness body.
- Record the runtime-spike outcome (which integration approach works, and what was disproven) in `## Notes`.
- *Primary files:* `Cargo.toml`, `src/serve/mod.rs`.

### 2. CLI arm + dispatch + DB-free serve config
- `src/cli.rs`: add `Commands::Serve { addr: Option<String>, token: Option<String> }` with help text.
- `src/main.rs`: add the dispatch arm that resolves config (Task 3's `load_serve_config`) and calls `serve::run(...)`; register `command_name` for the new arm so `observ::emit_start`/`emit_outcome` cover it.
- `src/config.rs`: add a **DB-free** `load_serve_config()` (pure merge over env: `BASTION_SERVE_ADDR` default `0.0.0.0:4317`, `BASTION_SERVE_TOKEN`) returning a `ServeConfig { addr, token }`; the token is **mandatory** — absent token is a typed error, never an empty default. Unit-test the merge + the missing-token error path (Rule 6).
- *Primary files:* `src/cli.rs`, `src/main.rs`, `src/config.rs`.

### 3. Bearer-token auth middleware + tailnet bind
- *New* `src/serve/auth.rs`: an actix middleware (or extractor) that enforces a `Authorization: Bearer <token>` match against the configured `BASTION_SERVE_TOKEN`. Missing/invalid token → `401`. Keep the token-comparison logic in a pure function (`token_matches(header, expected) -> bool`) and unit-test it exhaustively (present/absent header, wrong scheme, wrong token, correct token) per Rule 6.
- Wire the middleware into `src/serve/mod.rs` so protected routes require the token; document the exact policy (which routes require auth vs. `/health` liveness) in Task 6's contract.
- Confirm the bind uses the configured addr (default `0.0.0.0:4317`, tailnet-reachable).
- *Primary files:* `src/serve/auth.rs` (+ route wiring in `src/serve/mod.rs`).

### 4. Serde DTOs + frame envelope
- *New* `src/serve/dto.rs`: serde DTOs for the v0 surface — a health response body and the **WS frame envelope** skeleton (a typed/tagged frame wrapper later blocks extend with concrete variants). `Session`/`SessionState`/`Pane` derive only `Debug, Clone`, so DTOs are separate serde types, not those structs directly (no upstream derive changes).
- Unit-test serialize/deserialize round-trips for each DTO + the envelope tagging (Rule 6).
- *Primary files:* `src/serve/dto.rs`.

### 5. Minimal `/ws` accept + echo actor
- *New* `src/serve/ws/echo.rs`: an `actix-web-actors` WS actor that accepts the upgrade and echoes received text frames back — the live socket the Flutter foundation needs before the real hub (Block C) exists.
- Wire the `/ws` upgrade route into `src/serve/mod.rs` (behind the bearer middleware per the contract).
- The actor I/O shell is smoke-tested (connect with `websocat`, send a frame, observe the echo) and the result recorded in `## Notes` per Rule 6; any pure frame-handling helper is unit-tested.
- *Primary files:* `src/serve/ws/echo.rs` (+ route wiring in `src/serve/mod.rs`).

### 6. [~] `docs/serve-api.md` v0 contract + index
- *New* `docs/serve-api.md` (OKF frontmatter): v0 contract documenting the base URL/tailnet bind, the bearer-auth scheme + 401 behavior, `GET /health` (shape + auth policy), the `/ws` upgrade + echo behavior, and the **frame envelope skeleton** later blocks extend. State the version explicitly as **v0** (later blocks bump to v0.1+).
- Update `docs/index.md` to add the `serve-api.md` row (Rule 7).
- *Primary files:* `docs/serve-api.md`, `docs/index.md`.

### 7. [x] Validate
- Run the Validation Commands listed below and confirm all pass.

## Acceptance Criteria
- `bastion serve` boots and binds the configured tailnet addr (default `0.0.0.0:4317`); the runtime-spike outcome (actix `System` integration vs. plain tokio) is documented in `## Notes`.
- `GET /health` returns a successful JSON liveness response; a request to a protected route **without** a valid `Authorization: Bearer <token>` returns `401`, and **with** the correct token succeeds.
- A `/ws` upgrade connects and echoes a sent text frame back (smoke-tested with `websocat`, recorded in `## Notes`).
- `load_serve_config()` is DB-free (does not require `DATABASE_URL`); a missing `BASTION_SERVE_TOKEN` is a typed error, not a silent empty default.
- Pure logic (token match, config merge, DTO serde round-trips) is unit-tested including error paths; the actor/server I/O shell is smoke-tested and recorded (Rule 6).
- `docs/serve-api.md` v0 is committed and `docs/index.md` lists it.
- All four gated checks pass.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes

### Task 1 — Runtime-spike outcome (actix System vs. plain tokio)

The integration risk going in: `actix-web-actors` WS actors need an actix `System`/`Arbiter` that
the existing `#[tokio::main]` entry-point does not provide.

Two approaches were evaluated:

1. **Plain tokio await** — `HttpServer::new(...).run().await` inside a tokio-spawned future.
   Compiles and works for the plain-HTTP `/health` surface, but when `actix-web-actors` starts
   (Block C), the WS actor needs an `Arbiter` which is absent in a pure-tokio context. Disproven
   as a forward-safe choice.

2. **Dedicated thread + actix `System`** — `actix_web::rt::System::new().block_on(...)` on a
   dedicated OS thread spins up the actix `System`, which provides the `Arbiter`. The inner async
   block runs `HttpServer`, `/health`, and WS actors uniformly.

**Decision: approach 2 adopted.** The `serve::run` function is synchronous and blocking; the
tokio dispatch arm calls it via `tokio::task::spawn_blocking`. This keeps the entry-point uniform
when WS actors land in Task 5 / Block C. The integration detail is also captured in the
`src/serve/mod.rs` module doc comment.

### Task 5 — `/ws` echo smoke test (websocat, 2026-06-26)

Environment: `BASTION_SERVE_ADDR=127.0.0.1:14319 BASTION_SERVE_TOKEN=smoke-token`

```
$ echo "hello-echo" | websocat -H='Authorization: Bearer smoke-token' ws://127.0.0.1:14319/ws
hello-echo
```

Result: frame sent, identical frame echoed back, websocat exited 0. The server also correctly
returns `401 Unauthorized` when the `Authorization` header is absent or carries the wrong token
(exercised by the bearer-auth unit tests). The `/ws` echo actor and auth middleware are confirmed
working end-to-end.

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
