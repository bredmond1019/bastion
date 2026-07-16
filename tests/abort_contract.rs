//! BA.7.C task 4: in-process `engine-serve` contract test.
//!
//! Drives bastion's `api::client::ApiClient::abort_run` against a REAL,
//! network-bound `engine-serve` `App` — not a mock, and never the
//! orchestrator (D48: `OR.I` superseded; the Python orchestrator has no
//! abort endpoint). The server is spawned in-process on an OS-assigned
//! loopback port for the duration of each test, so this passes under a bare
//! `cargo test` with no external stack, no Python, and no running
//! orchestrator.
//!
//! The fixture workflow (`WaitNode` blocking on a `tokio::sync::Notify`,
//! then `SuccessNode`) mirrors the worked reference,
//! `../engine-rs/crates/engine-serve/tests/abort_integration.rs`: it gives
//! this test a window to grab the freshly-minted `run_id` from the live
//! state store and abort it while the run is still live, so the `202`
//! (accepted) path is exercised against a real in-flight run rather than a
//! synthetic id.

use std::collections::HashMap as StdHashMap;
use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

use actix_web::{App, HttpServer, web};
use bastion::api::client::AbortOutcome;
use bastion::api::client::ApiClient;
use engine_contract::TaskContext;
use engine_core::{Node, NodeConfig, NodeError, NodeRegistry, Workflow, WorkflowSchema};
use engine_serve::abort::RunRegistry;
use engine_serve::dispatch::Dispatcher;
use engine_serve::durable::spawn_durable_writer;
use engine_serve::http::{AppState, configure};
use engine_serve::live_state::LiveStateStore;
use tokio::sync::Notify;

const FIXTURE_WORKFLOW_TYPE: &str = "abort-contract-fixture";
const API_KEY: &str = "abort-contract-test-key";

/// First node in the fixture graph: blocks in `process` until `release` is
/// notified, giving this test a window to grab the live `run_id` and abort
/// it mid-run.
struct WaitNode {
    release: Arc<Notify>,
}

#[async_trait::async_trait]
impl Node for WaitNode {
    async fn process(&self, mut ctx: TaskContext) -> Result<TaskContext, NodeError> {
        self.release.notified().await;
        ctx.nodes
            .insert(self.name().to_string(), serde_json::json!({ "ran": true }));
        Ok(ctx)
    }

    fn name(&self) -> &str {
        "WaitNode"
    }
}

/// Second node in the fixture graph. Cancellation is only observed at the
/// next node boundary, so this node should stay `Pending` once the run is
/// aborted mid-`WaitNode`.
struct SuccessNode;

#[async_trait::async_trait]
impl Node for SuccessNode {
    async fn process(&self, mut ctx: TaskContext) -> Result<TaskContext, NodeError> {
        ctx.nodes
            .insert(self.name().to_string(), serde_json::json!({ "ran": true }));
        Ok(ctx)
    }

    fn name(&self) -> &str {
        "SuccessNode"
    }
}

fn fixture_schema() -> WorkflowSchema {
    let mut nodes = StdHashMap::new();
    nodes.insert(
        "WaitNode".to_string(),
        NodeConfig::new("WaitNode", vec!["SuccessNode".to_string()]),
    );
    nodes.insert(
        "SuccessNode".to_string(),
        NodeConfig::new("SuccessNode", vec![]),
    );
    WorkflowSchema::new(FIXTURE_WORKFLOW_TYPE, "WaitNode", nodes)
}

fn test_app_state(release: Arc<Notify>) -> AppState {
    let mut dispatcher = Dispatcher::new();
    dispatcher.register(
        fixture_schema(),
        Box::new(move || {
            let mut registry = NodeRegistry::new();
            registry.register(Box::new(WaitNode {
                release: release.clone(),
            }));
            registry.register(Box::new(SuccessNode));
            Workflow::new(registry, fixture_schema())
        }),
    );

    AppState {
        dispatcher: Arc::new(dispatcher),
        live: LiveStateStore::new(),
        durable: spawn_durable_writer(None),
        runs: RunRegistry::new(),
        api_key: API_KEY.to_string(),
    }
}

/// Spawn a real `engine-serve` HTTP server on an OS-assigned loopback port
/// and return its base URL plus the `LiveStateStore` handle (used to poll
/// for the freshly-minted `run_id`, the same technique as
/// `abort_integration.rs`).
async fn spawn_engine(release: Arc<Notify>) -> (String, LiveStateStore) {
    let state = test_app_state(release);
    let live = state.live.clone();
    let data = web::Data::new(state);

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral loopback port");
    let addr = listener.local_addr().expect("read bound local addr");

    let server = HttpServer::new(move || App::new().app_data(data.clone()).configure(configure))
        .listen(listener)
        .expect("attach listener to HttpServer")
        .run();

    actix_web::rt::spawn(server);

    (format!("http://{addr}"), live)
}

#[actix_web::test]
async fn abort_without_api_key_configured_is_a_typed_config_error() {
    let (base_url, _live) = spawn_engine(Arc::new(Notify::new())).await;

    // No `.with_engine_api_key(..)` — the client itself refuses to send an
    // unauthenticated request rather than letting the server 401 it.
    let client = ApiClient::new(&base_url);
    let err = client
        .abort_run("00000000-0000-0000-0000-000000000000")
        .await
        .expect_err("missing engine_api_key must be a typed client-side error");

    // ConsoleError::ConfigError — asserted via Display since the error type
    // is only re-exported through the observ module, not this crate root.
    assert!(format!("{err}").contains("engine_api_key"));
}

#[actix_web::test]
async fn abort_with_wrong_api_key_returns_401() {
    let (base_url, _live) = spawn_engine(Arc::new(Notify::new())).await;

    let client = ApiClient::new(&base_url).with_engine_api_key(Some("wrong-key".to_string()));
    let outcome = client
        .abort_run("00000000-0000-0000-0000-000000000000")
        .await
        .expect("a 401 response classifies as Ok(Unauthorized), not a transport error");

    assert!(
        matches!(outcome, AbortOutcome::Unauthorized(_)),
        "expected Unauthorized, got {outcome:?}"
    );
}

#[actix_web::test]
async fn abort_unknown_run_id_returns_404() {
    let (base_url, _live) = spawn_engine(Arc::new(Notify::new())).await;

    let client = ApiClient::new(&base_url).with_engine_api_key(Some(API_KEY.to_string()));
    let outcome = client
        .abort_run("00000000-0000-0000-0000-000000000000")
        .await
        .expect("a 404 response classifies as Ok(NotFound)");

    assert!(
        matches!(outcome, AbortOutcome::NotFound(_)),
        "expected NotFound, got {outcome:?}"
    );
}

/// Full round trip against the real engine: trigger the fixture workflow,
/// abort it mid-run through bastion's `abort_run`, assert the `202`
/// (accepted) path, then assert a repeat abort against the now-finished run
/// reads as `404`.
#[actix_web::test]
async fn aborting_a_live_run_returns_202_then_repeat_abort_returns_404() {
    let release = Arc::new(Notify::new());
    let (base_url, live) = spawn_engine(release.clone()).await;

    // Trigger the fixture workflow directly (bastion's `ApiClient` has no
    // `/events/` trigger method today — `trigger_workflow` posts to `/`,
    // the orchestrator's dispatcher route, not the engine's). A raw
    // `reqwest` client is the same shape `abort_integration.rs` uses.
    let raw = reqwest::Client::new();
    let trigger_base_url = base_url.clone();
    let trigger_handle = tokio::spawn(async move {
        raw.post(format!("{trigger_base_url}/events/"))
            .header("X-API-Key", API_KEY)
            .json(&serde_json::json!({
                "workflow_type": FIXTURE_WORKFLOW_TYPE,
                "data": {},
            }))
            .send()
            .await
    });

    // Poll the live-state store until the freshly-minted run_id shows up —
    // `WaitNode`'s RUNNING transition is recorded via `on_progress` before
    // its `process()` call blocks on `release`.
    let run_id = loop {
        let active = live.list_active();
        if let Some(run_id) = active.into_iter().next() {
            break run_id;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    };

    let client = ApiClient::new(&base_url).with_engine_api_key(Some(API_KEY.to_string()));
    let outcome = client
        .abort_run(&run_id.to_string())
        .await
        .expect("aborting a live run should succeed");

    match &outcome {
        AbortOutcome::Accepted {
            run_id: rid,
            status,
        } => {
            assert_eq!(rid, &run_id.to_string());
            assert_eq!(status, "aborting");
        }
        other => panic!("expected Accepted, got {other:?}"),
    }

    // Let WaitNode finish now that the token is triggered; the run loop
    // observes the cancellation at the next boundary, before dispatching
    // SuccessNode.
    release.notify_one();

    let trigger_resp = trigger_handle
        .await
        .expect("trigger task panicked")
        .expect("trigger request should complete");
    assert_eq!(trigger_resp.status(), 202);

    // Aborting again now that the run has ended (and been deregistered)
    // reads as unknown, not as a stale success.
    let repeat_outcome = client
        .abort_run(&run_id.to_string())
        .await
        .expect("repeat abort should still classify, not transport-fail");
    assert!(
        matches!(repeat_outcome, AbortOutcome::NotFound(_)),
        "expected NotFound on repeat abort, got {repeat_outcome:?}"
    );
}
