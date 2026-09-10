//! The live run view (BA.26.D task 4, AC-2/AC-4): a mirror of engine-rs's
//! `StreamFrame` wire type, and the five-state classification that answers
//! "why is this pane empty" instead of rendering one blank screen.
//!
//! Two things live here, deliberately kept in one module because they are
//! two halves of the same feature: a pane that cannot RENDER a frame it
//! cannot trust the shape of, and a pane that cannot tell the operator why
//! there is nothing to render yet.

use std::time::Duration;

use engine_contract::task_context::TaskContext;
use serde::Deserialize;
use uuid::Uuid;

use crate::api::client::ApiClient;
use crate::db::workflows::WorkflowRun;

// ── AC-4: the StreamFrame mirror ─────────────────────────────────────────

/// Mirror of `engine_serve::stream::StreamFrame`
/// (`../engine-rs/crates/engine-serve/src/stream.rs`) — that struct is
/// `Serialize`-only by engine-rs's own choice (its doc comment: bastion
/// consumes it over the wire, engine-rs never derives `Deserialize` for a
/// consumer it doesn't own). bastion MIRRORS the shape here rather than
/// asking engine-rs to change; this type owns the deserialize side and the
/// pinning test that catches drift.
///
/// `#[serde(deny_unknown_fields)]` is what makes this a MIRROR rather than a
/// tolerant subset: without it, `engine_serve::stream::StreamFrame` gaining
/// a field is invisible here forever (serde silently ignores unknown keys
/// by default), which is exactly the direction this type will actually
/// drift as the wire contract grows. See the round-trip test below for the
/// shown-failing proof.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamFrame {
    pub event_id: Uuid,
    pub status: String,
    pub task_context: TaskContext,
    pub terminal: bool,
}

// ── AC-2: five-state unreachability classification ──────────────────────

/// The five states a run pane can honestly report, in place of one blank
/// screen — AC-2. States [`RunViewState::EngineRoutesUnmounted`] and
/// [`RunViewState::GenuinelyIdle`] are NOT distinguishable on the route the
/// pane already reads (`GET /api/runs` simply stays empty either way —
/// `src/serve/mod.rs:551`), and `/health` is no help because bastion
/// deliberately shadows engine-serve's own `/health` with an
/// always-answering one (first-registration-wins, `src/serve/mod.rs`). The
/// workable probe is a SECOND call, on a SECOND credential, against an
/// engine route such as `GET /workflows` — see [`classify_run_view`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunViewState {
    /// No client bearer token is configured at all — nothing has been
    /// attempted yet. Produced by [`probe_not_configured`].
    NotConfigured,
    /// `bastion serve` itself could not be reached (connection refused,
    /// DNS failure, timeout — the raw reason is kept, never discarded, per
    /// this block's diagnostics criterion). Produced by
    /// [`probe_bastion_serve`]'s `Err` arm.
    ServeUnreachable(String),
    /// A reachable server rejected the credential presented to it — either
    /// `bastion serve`'s own `/api/*` gate (bad client bearer token) or the
    /// engine's own gate (bad engine API key). Produced by
    /// [`probe_unauthorized`].
    Unauthorized,
    /// The engine probe route itself is not mounted (`404`) — `bastion
    /// serve` is up, but nothing routes engine requests at all. Produced by
    /// [`probe_engine_routes_unmounted`].
    EngineRoutesUnmounted,
    /// Both probes succeeded and the engine route answered normally: the
    /// pane really is empty because there is genuinely nothing running, not
    /// because anything is broken. Produced by [`probe_genuinely_idle`].
    GenuinelyIdle,
}

/// Probe 1 (state: [`RunViewState::NotConfigured`]) — pure, no I/O. `true`
/// when no client bearer token is configured at all, which means every
/// later probe would fail identically and uninformatively; short-circuit
/// before making any network call.
pub fn probe_not_configured(client_bearer_token: Option<&str>) -> bool {
    client_bearer_token.is_none()
}

/// Probe 2 (state: [`RunViewState::ServeUnreachable`]) — calls `path`
/// against `bastion serve`'s own protected `/api/*` surface via
/// [`ApiClient::get_api`] (task 3's client bearer token transport). Returns
/// the raw HTTP status on any completed exchange (classification of THAT
/// status is [`probe_unauthorized`]'s job, not this function's); returns
/// `Err` with the underlying reason only when the connection itself could
/// not be completed — the state-2 case.
pub async fn probe_bastion_serve(client: &ApiClient, path: &str) -> Result<u16, String> {
    client
        .get_api(path)
        .await
        .map(|resp| resp.status().as_u16())
        .map_err(|e| e.to_string())
}

/// Probe 3 (state: [`RunViewState::Unauthorized`]) — pure classification
/// over a status code already fetched by [`probe_bastion_serve`] or
/// [`probe_engine_route`]. `true` for `401` from either probe.
pub fn probe_unauthorized(status: u16) -> bool {
    status == 401
}

/// The SECOND call, on the SECOND credential (AC-2): probes an engine route
/// directly — bypassing `ApiClient::get_api`, which only knows how to send
/// the client bearer token (task 3) — using the engine's own `X-API-Key`
/// header (`engine_api_key`, the same secret [`ApiClient::abort_run`] and
/// [`ApiClient::resume_run`] already send). Returns the raw status on any
/// completed exchange; `Err` only on a connection failure.
pub async fn probe_engine_route(
    http: &reqwest::Client,
    engine_base_url: &str,
    engine_api_key: Option<&str>,
    path: &str,
) -> Result<u16, String> {
    let url = format!("{}{path}", engine_base_url.trim_end_matches('/'));
    let mut req = http.get(&url).timeout(Duration::from_secs(5));
    if let Some(key) = engine_api_key {
        req = req.header("X-API-Key", key);
    }
    req.send()
        .await
        .map(|resp| resp.status().as_u16())
        .map_err(|e| e.to_string())
}

/// Probe 4 (state: [`RunViewState::EngineRoutesUnmounted`]) — pure
/// classification: a `404` on the engine probe means nothing at that path
/// is registered at all, distinct from a `200` with an empty body.
pub fn probe_engine_routes_unmounted(engine_probe_status: u16) -> bool {
    engine_probe_status == 404
}

/// Probe 5 (state: [`RunViewState::GenuinelyIdle`]) — pure classification:
/// a `200` on the engine probe means the route IS mounted and answering, so
/// an empty `/api/runs` really does mean no live runs, not a missing
/// engine.
pub fn probe_genuinely_idle(engine_probe_status: u16) -> bool {
    engine_probe_status == 200
}

/// Compose the five probes above into the pane's actual classification.
/// `console_path` is the `bastion serve` `/api/*` route to probe first
/// (e.g. `/api/runs`); `engine_path` is the second, engine-side route
/// (e.g. `/workflows`) probed only once the first call is confirmed
/// reachable and authorized.
///
/// Any engine-probe status outside `{401, 404, 200}` is folded into
/// [`RunViewState::ServeUnreachable`] carrying the unexpected status —
/// there is no sixth state to put it in, and silently mapping an unknown
/// status to "idle" would defeat this block's entire premise.
pub async fn classify_run_view(
    console_client: &ApiClient,
    http: &reqwest::Client,
    engine_base_url: &str,
    engine_api_key: Option<&str>,
    client_bearer_token: Option<&str>,
    console_path: &str,
    engine_path: &str,
) -> RunViewState {
    if probe_not_configured(client_bearer_token) {
        return RunViewState::NotConfigured;
    }

    let console_status = match probe_bastion_serve(console_client, console_path).await {
        Err(reason) => return RunViewState::ServeUnreachable(reason),
        Ok(status) => status,
    };
    if probe_unauthorized(console_status) {
        return RunViewState::Unauthorized;
    }

    let engine_status =
        match probe_engine_route(http, engine_base_url, engine_api_key, engine_path).await {
            Err(reason) => return RunViewState::ServeUnreachable(reason),
            Ok(status) => status,
        };
    if probe_unauthorized(engine_status) {
        return RunViewState::Unauthorized;
    }
    if probe_engine_routes_unmounted(engine_status) {
        return RunViewState::EngineRoutesUnmounted;
    }
    if probe_genuinely_idle(engine_status) {
        return RunViewState::GenuinelyIdle;
    }
    RunViewState::ServeUnreachable(format!("unexpected engine probe status {engine_status}"))
}

// ── BA.26.E: the finished-runs discovery list ────────────────────────────
//
// `bastion inspect <run_id>` already renders any run at any status — the
// gap this block closes is DISCOVERY: nothing lists finished runs, so a run
// can only be inspected if you already know its UUID. `list_finished_runs`
// (task 1, `src/db/workflows.rs`) is the read-only query; the two items
// below are the console-side data type and row formatter that wire it into
// Mission Control.

/// Progress/result state for loading the finished-runs discovery list
/// (BA.26.E task 2). Mirrors [`crate::sessions::app::RunViewStatus`]'s
/// Idle/Probing(here: Loading)/Done shape, plus an explicit `Failed` arm
/// for a DB error: unlike the run-view probe (which only ever resolves to a
/// classified [`RunViewState`], never an `Err`), a real Postgres query can
/// fail outright (bad/missing `DATABASE_URL`, connection refused), and that
/// failure must reach the operator rather than silently rendering an empty
/// list. Purely data — no DB connection/handle lives on `AppState`; the
/// ui.rs event loop owns the background thread that runs the query and
/// writes the result back here.
#[derive(Debug, Clone, Default)]
pub enum FinishedRunsStatus {
    /// No load has run yet this session.
    #[default]
    Idle,
    /// A load is in flight — set the moment the key is pressed, before the
    /// event loop has spawned the background thread, so the pane reflects
    /// "loading" starting on the very next frame (mirrors `RunViewStatus::Probing`).
    Loading,
    /// The most recently completed load's result, rendered until the next
    /// load starts.
    Done(Vec<WorkflowRun>),
    /// The most recently completed load errored. The raw reason is
    /// preserved, never discarded — same diagnostics discipline as
    /// [`RunViewState::ServeUnreachable`].
    Failed(String),
}

/// Render one finished-runs list row from a [`WorkflowRun`] (BA.26.E task
/// 2). The ONE formatter the list rendering calls — a reviewer can point at
/// this single call site rather than finding a second formatter (the block
/// record's "renders through the SAME NodeState the live view uses"
/// constraint: this reads `WorkflowRun`'s own already-derived fields
/// directly rather than re-deriving or re-parsing anything). Column-aligned
/// the same way `sessions::ui::session_row` formats its own list rows.
pub fn finished_run_row(run: &WorkflowRun) -> String {
    format!(
        "{:<38} {:<24} {:<14} {}",
        run.id,
        run.workflow_name,
        format!("{:?}", run.status),
        run.started_at.as_deref().unwrap_or("—"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    /// Spin up a real, minimal TCP listener on an ephemeral port that
    /// answers the first request with `status_line` and drops the
    /// connection — same technique as `src/api/client.rs`'s
    /// `get_api_sends_authorization_bearer_header_when_configured` (no
    /// mock-HTTP crate is a dev-dependency of this crate).
    fn fake_server(status_line: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let addr = listener.local_addr().expect("local addr");
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(status_line.as_bytes());
            }
        });
        format!("http://{addr}")
    }

    /// An address nothing is listening on — bind then immediately drop, so
    /// the port is free but every connection attempt is refused.
    fn unreachable_addr() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let addr = listener.local_addr().expect("local addr");
        drop(listener);
        format!("http://{addr}")
    }

    // ── probe_not_configured ─────────────────────────────────────────────

    #[test]
    fn probe_not_configured_true_when_bearer_absent() {
        assert!(probe_not_configured(None));
    }

    #[test]
    fn probe_not_configured_false_when_bearer_present() {
        assert!(!probe_not_configured(Some("client-bearer-token")));
    }

    // ── probe_bastion_serve ──────────────────────────────────────────────

    #[tokio::test]
    async fn probe_bastion_serve_returns_status_from_a_reachable_server() {
        let base_url = fake_server("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        let client = ApiClient::new(&base_url);
        let status = probe_bastion_serve(&client, "/api/runs")
            .await
            .expect("server was reachable");
        assert_eq!(status, 200);
    }

    #[tokio::test]
    async fn probe_bastion_serve_returns_401_from_a_reachable_but_rejecting_server() {
        let base_url = fake_server("HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n");
        let client = ApiClient::new(&base_url);
        let status = probe_bastion_serve(&client, "/api/runs")
            .await
            .expect("server was reachable");
        assert_eq!(status, 401);
    }

    #[tokio::test]
    async fn probe_bastion_serve_errs_when_connection_is_refused() {
        let client = ApiClient::new(&unreachable_addr());
        let result = probe_bastion_serve(&client, "/api/runs").await;
        assert!(
            result.is_err(),
            "expected a connection error, got {result:?}"
        );
    }

    // ── probe_unauthorized ───────────────────────────────────────────────

    #[test]
    fn probe_unauthorized_true_for_401() {
        assert!(probe_unauthorized(401));
    }

    #[test]
    fn probe_unauthorized_false_for_other_statuses() {
        assert!(!probe_unauthorized(200));
        assert!(!probe_unauthorized(404));
    }

    // ── probe_engine_route ───────────────────────────────────────────────

    #[tokio::test]
    async fn probe_engine_route_returns_404_when_route_is_not_mounted() {
        let base_url = fake_server("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
        let http = reqwest::Client::new();
        let status = probe_engine_route(&http, &base_url, None, "/workflows")
            .await
            .expect("server was reachable");
        assert_eq!(status, 404);
    }

    #[tokio::test]
    async fn probe_engine_route_returns_200_when_mounted_and_reachable() {
        let base_url = fake_server("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        let http = reqwest::Client::new();
        let status = probe_engine_route(&http, &base_url, Some("engine-key"), "/workflows")
            .await
            .expect("server was reachable");
        assert_eq!(status, 200);
    }

    #[tokio::test]
    async fn probe_engine_route_errs_when_connection_is_refused() {
        let http = reqwest::Client::new();
        let result = probe_engine_route(&http, &unreachable_addr(), None, "/workflows").await;
        assert!(
            result.is_err(),
            "expected a connection error, got {result:?}"
        );
    }

    #[tokio::test]
    async fn probe_engine_route_sends_x_api_key_header_when_configured() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let addr = listener.local_addr().expect("local addr");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
                let _ = tx.send(request);
            }
        });
        let http = reqwest::Client::new();
        let _ = probe_engine_route(
            &http,
            &format!("http://{addr}"),
            Some("engine-secret"),
            "/workflows",
        )
        .await;
        let request = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("server thread should have received a request");
        assert!(
            request.to_lowercase().contains("x-api-key: engine-secret"),
            "request did not carry the expected X-API-Key header: {request}"
        );
    }

    #[tokio::test]
    async fn probe_engine_route_sends_no_x_api_key_header_when_unconfigured() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let addr = listener.local_addr().expect("local addr");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
                let _ = tx.send(request);
            }
        });
        let http = reqwest::Client::new();
        let _ = probe_engine_route(&http, &format!("http://{addr}"), None, "/workflows").await;
        let request = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("server thread should have received a request");
        assert!(
            !request.to_lowercase().contains("x-api-key"),
            "request should carry no X-API-Key header when unconfigured: {request}"
        );
    }

    // ── probe_engine_routes_unmounted ────────────────────────────────────

    #[test]
    fn probe_engine_routes_unmounted_true_for_404() {
        assert!(probe_engine_routes_unmounted(404));
    }

    #[test]
    fn probe_engine_routes_unmounted_false_for_200() {
        assert!(!probe_engine_routes_unmounted(200));
    }

    // ── probe_genuinely_idle ─────────────────────────────────────────────

    #[test]
    fn probe_genuinely_idle_true_for_200() {
        assert!(probe_genuinely_idle(200));
    }

    #[test]
    fn probe_genuinely_idle_false_for_404() {
        assert!(!probe_genuinely_idle(404));
    }

    // ── classify_run_view — one test per resulting state ────────────────

    #[tokio::test]
    async fn classify_run_view_not_configured_when_bearer_token_absent() {
        let client = ApiClient::new("http://127.0.0.1:1");
        let http = reqwest::Client::new();
        let state = classify_run_view(
            &client,
            &http,
            "http://127.0.0.1:1",
            None,
            None,
            "/api/runs",
            "/workflows",
        )
        .await;
        assert_eq!(state, RunViewState::NotConfigured);
    }

    #[tokio::test]
    async fn classify_run_view_serve_unreachable_when_bastion_serve_is_down() {
        let client =
            ApiClient::new(&unreachable_addr()).with_bearer_token(Some("client-token".into()));
        let http = reqwest::Client::new();
        let state = classify_run_view(
            &client,
            &http,
            "http://127.0.0.1:1",
            None,
            Some("client-token"),
            "/api/runs",
            "/workflows",
        )
        .await;
        assert!(
            matches!(state, RunViewState::ServeUnreachable(_)),
            "expected ServeUnreachable, got {state:?}"
        );
    }

    #[tokio::test]
    async fn classify_run_view_unauthorized_when_bastion_serve_rejects_the_bearer_token() {
        let base_url = fake_server("HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n");
        let client = ApiClient::new(&base_url).with_bearer_token(Some("bad-token".into()));
        let http = reqwest::Client::new();
        let state = classify_run_view(
            &client,
            &http,
            "http://127.0.0.1:1",
            None,
            Some("bad-token"),
            "/api/runs",
            "/workflows",
        )
        .await;
        assert_eq!(state, RunViewState::Unauthorized);
    }

    #[tokio::test]
    async fn classify_run_view_engine_routes_unmounted_when_the_engine_probe_404s() {
        let console_url = fake_server("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        let engine_url = fake_server("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
        let client = ApiClient::new(&console_url).with_bearer_token(Some("client-token".into()));
        let http = reqwest::Client::new();
        let state = classify_run_view(
            &client,
            &http,
            &engine_url,
            Some("engine-key"),
            Some("client-token"),
            "/api/runs",
            "/workflows",
        )
        .await;
        assert_eq!(state, RunViewState::EngineRoutesUnmounted);
    }

    #[tokio::test]
    async fn classify_run_view_genuinely_idle_when_both_probes_succeed() {
        let console_url = fake_server("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        let engine_url = fake_server("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        let client = ApiClient::new(&console_url).with_bearer_token(Some("client-token".into()));
        let http = reqwest::Client::new();
        let state = classify_run_view(
            &client,
            &http,
            &engine_url,
            Some("engine-key"),
            Some("client-token"),
            "/api/runs",
            "/workflows",
        )
        .await;
        assert_eq!(state, RunViewState::GenuinelyIdle);
    }

    // ── AC-4: StreamFrame mirror round-trip ─────────────────────────────

    fn a_real_task_context() -> TaskContext {
        TaskContext {
            event: serde_json::json!({"type": "test-event"}),
            nodes: std::collections::HashMap::new(),
            metadata: serde_json::json!({}),
            node_runs: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn stream_frame_mirror_round_trips_a_real_engine_serve_stream_frame() {
        let real = engine_serve::stream::StreamFrame {
            event_id: Uuid::new_v4(),
            status: "running".to_string(),
            task_context: a_real_task_context(),
            terminal: false,
        };
        let json = serde_json::to_string(&real).expect("serialize a real StreamFrame");

        let mirrored: StreamFrame =
            serde_json::from_str(&json).expect("mirror should deserialize a real StreamFrame");

        assert_eq!(mirrored.event_id, real.event_id);
        assert_eq!(mirrored.status, real.status);
        assert_eq!(mirrored.terminal, real.terminal);
        assert_eq!(mirrored.task_context, real.task_context);
    }

    #[test]
    fn stream_frame_mirror_rejects_an_unrecognized_field() {
        // Simulates engine-rs's StreamFrame gaining a field this mirror
        // doesn't know about, without editing the sibling repo in an
        // automated test: hand-build the JSON shape a future StreamFrame
        // with one extra field would serialize to, and confirm
        // deny_unknown_fields rejects it. The live version of this proof
        // (editing engine-rs's actual struct, confirming red, reverting) is
        // recorded as a manual step in this task's ## Notes per AC-4 — this
        // test is the permanent, automatable half of that same proof.
        let json = serde_json::json!({
            "event_id": Uuid::new_v4().to_string(),
            "status": "running",
            "task_context": {
                "event": {},
                "nodes": {},
                "metadata": {},
                "node_runs": {},
            },
            "terminal": false,
            "a_field_this_mirror_does_not_know_about": "drift",
        })
        .to_string();

        let result: Result<StreamFrame, _> = serde_json::from_str(&json);
        assert!(
            result.is_err(),
            "deny_unknown_fields should reject an unrecognized field, got {result:?}"
        );
    }

    // ── finished_run_row (BA.26.E task 2) ───────────────────────────────

    fn a_workflow_run(status: crate::db::workflows::RunStatus) -> WorkflowRun {
        WorkflowRun {
            id: "11111111-1111-1111-1111-111111111111".to_string(),
            workflow_name: "some-workflow".to_string(),
            status,
            budget_halt: None,
            nodes: vec![],
            started_at: Some("2026-09-01T00:00:00Z".to_string()),
            elapsed_secs: None,
        }
    }

    #[test]
    fn finished_run_row_success_and_failed_differ_only_in_the_status_field() {
        use crate::db::workflows::RunStatus;

        let success_row = finished_run_row(&a_workflow_run(RunStatus::Success));
        let failed_row = finished_run_row(&a_workflow_run(RunStatus::Failed));

        assert_ne!(
            success_row, failed_row,
            "rows for differing statuses must render differently"
        );
        // Every other field (id, workflow_name, started_at) is identical
        // between the two fixtures above, so splitting each row on
        // whitespace and comparing every column EXCEPT the status one
        // (index 2 — id, workflow_name, status, started_at) must leave
        // identical tokens — proving one formatter handles both, not two
        // parallel code paths. (Column-width PADDING differs between
        // "Success" and "Failed" — a raw substring-replace-then-compare
        // would spuriously fail on that padding, not on a real content
        // difference.)
        let success_fields: Vec<&str> = success_row.split_whitespace().collect();
        let failed_fields: Vec<&str> = failed_row.split_whitespace().collect();
        assert_eq!(success_fields.len(), failed_fields.len());
        for (i, (a, b)) in success_fields.iter().zip(failed_fields.iter()).enumerate() {
            if i == 2 {
                continue; // the status column — expected to differ.
            }
            assert_eq!(
                a, b,
                "column {i} should be identical between the two rows: {success_row:?} vs {failed_row:?}"
            );
        }
    }

    #[test]
    fn finished_run_row_renders_dash_when_started_at_is_absent() {
        use crate::db::workflows::RunStatus;

        let mut run = a_workflow_run(RunStatus::Success);
        run.started_at = None;
        let row = finished_run_row(&run);
        assert!(
            row.contains('—'),
            "a run with no started_at should render the placeholder dash: {row:?}"
        );
    }

    /// The discovery-gap property this whole block exists to close (block
    /// record AC-1, task 2's own AC-1): a run present in the finished-runs
    /// list must be ABSENT from an active-runs-equivalent set built with the
    /// SAME terminal/non-terminal predicate `list_active_runs` uses
    /// (pending|running == active). A bare row-count assertion would pass
    /// even if the two sets overlapped entirely; this proves they don't.
    #[test]
    fn finished_runs_discovery_gap_terminal_run_absent_from_active_equivalent_set() {
        use crate::db::workflows::{RunStatus, select_finished};

        fn is_active_equivalent(run: &WorkflowRun) -> bool {
            matches!(run.status, RunStatus::Running | RunStatus::Pending)
        }

        let mut terminal_run = a_workflow_run(RunStatus::Success);
        terminal_run.id = "terminal-run-1".to_string();
        let mut running_run = a_workflow_run(RunStatus::Running);
        running_run.id = "running-run-1".to_string();

        let all_runs = vec![terminal_run.clone(), running_run.clone()];
        let finished = select_finished(all_runs.clone(), 10);
        let active_equivalent: Vec<&WorkflowRun> = all_runs
            .iter()
            .filter(|r| is_active_equivalent(r))
            .collect();

        assert!(
            finished.iter().any(|r| r.id == terminal_run.id),
            "the terminal run must appear in the finished-runs list"
        );
        assert!(
            !active_equivalent.iter().any(|r| r.id == terminal_run.id),
            "the terminal run must be ABSENT from the active-runs-equivalent \
             set — that gap is the discovery problem this block closes"
        );
    }

    #[test]
    fn finished_run_row_contains_id_and_workflow_name() {
        use crate::db::workflows::RunStatus;

        let run = a_workflow_run(RunStatus::Cancelled);
        let row = finished_run_row(&run);
        assert!(row.contains(&run.id), "row must show the run id: {row:?}");
        assert!(
            row.contains(&run.workflow_name),
            "row must show the workflow name: {row:?}"
        );
    }
}
