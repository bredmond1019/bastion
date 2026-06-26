# Worklog — 11.A-serve-scaffold-and-api

## Task 1 — PASSED (1 attempt)
What: Task 1: actix runtime spike — added actix-web 4.9/actix-web-actors 4.3/actix 0.13/futures 0.3 deps, created src/serve/mod.rs with pub fn run() using thread+System integration, GET /health handler returning JSON liveness body, 5 unit tests (200/body/405/404), all 662 tests pass.
Decisions: Used actix_web::rt::System::new().block_on() on a dedicated thread (not plain tokio await) — future-proofs for WS actors (Task 5) which need an Arbiter that tokio-only context lacks.; Used web::resource('/health') instead of web::route() so POST returns 405 Method Not Allowed rather than 404.; Extracted run_server() as a separate async fn so it can be tested independently via actix_web::test utilities without binding a port.
Validated: gating checks (fast tripwire)

## Task 2 — PASSED (1 attempt)
What: Added Commands::Serve CLI arm, dispatch, and DB-free ServeConfig with full unit tests
Decisions: build_serve_config is a pure function taking four Option<String> parameters (addr_flag, token_flag, addr_env, token_env) so it is fully unit-testable without I/O; MissingServeToken is a new typed ConfigError variant rather than reusing MissingVar so the error message is serve-specific; dispatch arm uses spawn_blocking + single ? to propagate both JoinError (converted) and inner serve::run errors
Validated: gating checks (fast tripwire)

## Task 3 — PASSED (1 attempt)
What: Bearer-token auth middleware wired into serve: pure token_matches() exhaustively tested, BearerAuthMiddleware enforces Authorization: Bearer on /api scope, /health stays public, 18 new tests added (693 total pass)
Decisions: Bearer scheme matching is case-sensitive (Bearer not bearer) — matches RFC 7617 and rejects common typos; Both Transform and Service impl bounds require B: MessageBody + 'static to allow map_into_boxed_body() on the success path, unifying both branches to ServiceResponse<BoxBody>; Protected routes placed under /api scope (not top-level) so Task 5 /ws can be wired in as /api/ws or a peer scope without restructuring; Rc used (not Arc) for token and service inside the middleware since actix runs single-threaded workers
Validated: gating checks (fast tripwire)

## Task 4 — PASSED (1 attempt)
What: Added src/serve/dto.rs with serde DTOs (HealthResponse, WsFrame, WsFrameKind, ErrorPayload) and exhaustive round-trip tests; exposed dto module from serve/mod.rs
Decisions: WsFrame uses a flat struct with kind+payload fields (not adjacently-tagged enum) so the Flutter client can dispatch on kind before parsing payload; WsFrameKind serializes with serde rename_all = snake_case matching the serve-api contract wire format; ErrorPayload is a separate named struct (not an inline serde_json::Value) to give the error surface a typed contract from day one
Validated: gating checks (fast tripwire)

## Task 5 — PASSED (1 attempt)
What: Add minimal /ws accept+echo actor: EchoActor (actix-web-actors) echoes text frames back, wired behind BearerAuthMiddleware at /ws scope in serve/mod.rs, with pure echo_text helper exhaustively unit-tested (6 cases).
Decisions: The /ws route is mounted as a separate scope (not under /api) to keep WS upgrade semantics distinct from REST; both scopes are protected by BearerAuthMiddleware.
Validated: gating checks (fast tripwire)

## Task 6 — PASSED (1 attempt)
What: Published docs/serve-api.md v0 contract (base URL, bearer-auth, GET /health, /ws echo, frame envelope) and added its row to docs/index.md
Validated: gating checks (fast tripwire)
