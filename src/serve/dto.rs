//! Serde DTOs for the `bastion serve` v0 surface.
//!
//! All types here are independent serde structs/enums — they do **not** derive
//! directly from the domain types (`Session`, `SessionState`, `Pane`) which only
//! implement `Debug, Clone`.  This keeps the DTO layer free to evolve independently
//! of the domain model.
//!
//! # Types
//! - [`HealthResponse`] — JSON body for `GET /health`.
//! - [`WsFrame`] — tagged envelope for all WebSocket messages (v0 skeleton).
//! - [`WsFrameKind`] — discriminant enum extended by later blocks.
//! - [`CommandRequest`] / [`CommandResponse`] — `POST /actions/command` quick-action
//!   inject/spawn request and response (BA.11.E).
//! - [`BoardDto`] / [`BoardScope`] — `GET /api/board` cross-brain now/next/blocked/finished
//!   rollup response and its `scope` query param (BA.11.K).
//! - [`AttentionDto`] / [`AttentionLanesDto`] / [`AttentionCarryoverDto`] /
//!   [`AttentionBacklogDto`] / [`AttentionThresholdsDto`] — `GET /api/attention` stale-carryover /
//!   aging-backlog / orphaned-capture projection response (BA.11.P). Reuses [`BoardScope`] for its
//!   `scope` query param — the semantics are identical to `GET /api/board`.
//! - [`DocTreeDto`] / [`DocEntryDto`] — `GET /api/docs/{repo}/tree` allowlisted markdown tree
//!   response (BA.11.Q).
//! - [`DocFileDto`] — `GET /api/docs/{repo}/file` raw markdown read response (BA.11.Q).
//! - [`PipelineDto`] / [`OpportunitySummaryDto`] / [`OpportunityDetailDto`] /
//!   [`ContactDto`] / [`OpportunityActionDto`] / [`ResearchBriefDto`] /
//!   [`ProspectLeadDto`] — `GET /api/pipeline` + `GET /api/pipeline/{slug}` read
//!   projection over the business sub-brain's opportunity markdown files (BW.3.A).
//! - [`BlockGraphDto`] / [`BlockGraphNodeDto`] / [`BlockGraphEdgeDto`] / [`BlockLaneDto`] /
//!   [`BlockEdgeKindDto`] — `GET /api/blocks/graph` mechanical projection of
//!   `mev::brain::block_graph::BlockGraphExport` (BA.17.A). Pure data only — no
//!   conversion logic lives here (see `handlers/block_graph.rs::block_graph_dto`).
//! - [`CostSummaryDto`] / [`WorkflowCostDto`] / [`BudgetStateDto`] / [`BudgetBreachDto`] —
//!   `GET /api/costs` read-only projection of `costs::CostSummary` + the BA.7.C budget
//!   state (BA.11.J). Caps are reported as configured; nothing here mutates them (mutation
//!   stays CLI/D48). Conversion logic lives in `handlers/costs.rs`, not here.
//! - [`RunSummaryDto`] — `GET /api/runs` widened live-runs summary projection (BA.11.T),
//!   alongside [`RunStateDto`]/[`NodeTransitionDto`]/[`RunUsageDto`] (BA.11.M). Conversion
//!   logic lives in `handlers/runs.rs`, not here.
//! - [`RunTransitionPayload`] / [`RunStreamStatusPayload`] — server→client
//!   `event{run_transition}` / `event{run_stream_status}` WS pushes for the
//!   subscribable `runs` topic (BA.11.N). Diff/derivation logic lives in
//!   `serve/poll.rs`, not here.

use crate::sessions::model::{Pane, Session};
use mev::TriageLane;
use serde::{Deserialize, Serialize};
use typeshare::typeshare;

// ── Health ─────────────────────────────────────────────────────────────────────

/// JSON body returned by `GET /health`.
///
/// Matches the shape documented in `docs/serve-api.md` v0:
/// ```json
/// { "status": "ok", "service": "bastion" }
/// ```
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HealthResponse {
    /// Liveness status; always `"ok"` when the server is healthy.
    pub status: String,
    /// Service identifier; always `"bastion"`.
    pub service: String,
}

impl HealthResponse {
    /// Construct the canonical liveness response.
    pub fn ok() -> Self {
        Self {
            status: "ok".to_owned(),
            service: "bastion".to_owned(),
        }
    }
}

// ── WebSocket frame envelope ───────────────────────────────────────────────────

/// Tagged WebSocket frame envelope.
///
/// Every WS message sent or received by `bastion serve` is wrapped in this
/// envelope so the Flutter client can dispatch on `kind` before parsing
/// `payload`.  This is the v0 skeleton; later blocks add concrete `kind`
/// variants and payload types.
///
/// Wire format (JSON):
/// ```json
/// { "kind": "echo", "payload": <any JSON value> }
/// ```
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WsFrame {
    /// Frame type discriminant.  The Flutter client switches on this field.
    pub kind: WsFrameKind,
    /// Arbitrary JSON payload.  Shape is defined per-kind in the serve-api contract.
    pub payload: serde_json::Value,
}

/// Discriminant for [`WsFrame::kind`].
///
/// v0 defined `Echo` and `Error`.  v0.2 adds client→server kinds (`Subscribe`,
/// `Unsubscribe`, `Send`, `SendKey`) and server→client kinds (`Sessions`, `Pane`,
/// `Event`).
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WsFrameKind {
    /// Echo — the `/ws` actor reflects the received frame back unchanged (v0).
    Echo,
    /// Error — server-side error notification pushed to the client.
    Error,
    // ── client → server (v0.2) ────────────────────────────────────────────
    /// Subscribe to a topic (`sessions` or `pane:<name>`).
    Subscribe,
    /// Unsubscribe from a topic.
    Unsubscribe,
    /// Send literal keystrokes to a tmux session (followed by Enter).
    Send,
    /// Send a single named tmux key to a session (e.g. `"Escape"`, `"C-c"`).
    SendKey,
    // ── server → client (v0.2) ────────────────────────────────────────────
    /// Session list snapshot pushed to `sessions` subscribers.
    Sessions,
    /// Pane diff pushed to `pane:<name>` subscribers.
    Pane,
    /// Async event pushed to all subscribed connections (e.g. `needs_input`).
    Event,
}

// ── Error payload ──────────────────────────────────────────────────────────────

/// Payload shape for `WsFrameKind::Error` frames.
///
/// Allows the server to surface typed error information over the WS channel.
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorPayload {
    /// Short machine-readable error code (e.g. `"C001"`).
    pub code: String,
    /// Human-readable error message.
    pub message: String,
}

// ── v0.2 WebSocket payload structs ────────────────────────────────────────────

/// Payload for client→server `subscribe` / `unsubscribe` frames.
///
/// Wire format: `{ "topic": "sessions" }` or `{ "topic": "pane:work" }`
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubscribePayload {
    /// Topic string: `"sessions"` or `"pane:<name>"`.
    pub topic: String,
}

/// Payload for client→server `send` frames (literal keystrokes + Enter).
///
/// Wire format: `{ "session": "main", "keys": "cargo test" }`
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SendPayload {
    /// Target tmux session name.
    pub session: String,
    /// Literal text to send (forwarded with `-l`), followed by Enter.
    pub keys: String,
}

/// Payload for client→server `send_key` frames (single named tmux key).
///
/// Wire format: `{ "session": "main", "key": "Escape" }`
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SendKeyPayload {
    /// Target tmux session name.
    pub session: String,
    /// Symbolic tmux key name (e.g. `"Escape"`, `"Up"`, `"C-c"`).
    pub key: String,
}

/// Payload for server→client `sessions` frames (session list snapshot).
///
/// Wire format: `{ "sessions": [ … ] }`
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionsPayload {
    /// Current snapshot of all tmux sessions.
    pub sessions: Vec<SessionDto>,
}

/// Payload for server→client `pane` frames (pane diff push).
///
/// Wire format: `{ "session": "main", "seq": 42, "lines": ["line1", "line2"] }`
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PanePayload {
    /// tmux session name whose pane was captured.
    pub session: String,
    /// Monotonically increasing sequence number; bumped on every diff push.
    #[typeshare(serialized_as = "number")]
    pub seq: u64,
    /// Non-blank trailing lines from the captured pane output.
    pub lines: Vec<String>,
}

/// Payload for server→client `event` frames (e.g. `needs_input`).
///
/// Wire format: `{ "session": "main", "event": "needs_input" }`
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventPayload {
    /// Session that triggered the event.
    pub session: String,
    /// Event name; currently only `"needs_input"` is defined.
    pub event: String,
}

// ── Topic enum + parser ────────────────────────────────────────────────────────

/// Parsed representation of a WebSocket subscription topic string.
///
/// Valid topic strings:
/// - `"sessions"` → [`Topic::Sessions`]
/// - `"pane:<name>"` → [`Topic::Pane`] where `<name>` is non-empty
/// - `"runs"` → [`Topic::Runs`] (BA.11.N)
///
/// Any other string (including `"pane:"` with an empty name) is invalid and
/// causes [`parse_topic`] to return `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Topic {
    /// The global sessions list topic (`"sessions"`).
    Sessions,
    /// A named pane topic (`"pane:<name>"`).
    Pane(String),
    /// The run-transition topic (`"runs"`, BA.11.N) — subscribers receive
    /// `run_transition` events on aggregate run-status change and a
    /// `run_stream_status` frame immediately on subscribe.
    Runs,
}

/// Parse a topic string into a [`Topic`] variant.
///
/// Returns `None` for any unrecognised or malformed string.
///
/// # Examples
/// ```
/// use crate::serve::dto::{parse_topic, Topic};
/// assert_eq!(parse_topic("sessions"), Some(Topic::Sessions));
/// assert_eq!(parse_topic("pane:work"), Some(Topic::Pane("work".into())));
/// assert_eq!(parse_topic("pane:"),     None);  // empty name
/// assert_eq!(parse_topic("runs"),      Some(Topic::Runs));
/// assert_eq!(parse_topic("unknown"),   None);
/// ```
pub fn parse_topic(s: &str) -> Option<Topic> {
    if s == "sessions" {
        return Some(Topic::Sessions);
    }
    if s == "runs" {
        return Some(Topic::Runs);
    }
    if let Some(name) = s.strip_prefix("pane:") {
        if name.is_empty() {
            return None;
        }
        return Some(Topic::Pane(name.to_owned()));
    }
    None
}

// ── Session response DTOs ─────────────────────────────────────────────────────

/// JSON response for a single tmux session (one element of `GET /api/sessions`).
///
/// Wire format:
/// ```json
/// { "name": "main", "state": "running", "last_line": "$ cargo test" }
/// ```
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionDto {
    /// tmux session name.
    pub name: String,
    /// Session state as a string: `"running"` or `"idle"`.
    pub state: String,
    /// Last non-blank line from the session's pane, or empty string when unavailable.
    pub last_line: String,
}

impl From<&Session> for SessionDto {
    fn from(s: &Session) -> Self {
        Self {
            name: s.name.clone(),
            state: s.state.as_str().to_owned(),
            last_line: s.last_line.clone(),
        }
    }
}

/// JSON response for `GET /api/sessions/{name}/pane`.
///
/// Wire format:
/// ```json
/// { "session_name": "main", "lines": ["line1", "line2"] }
/// ```
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaneDto {
    /// tmux session name this pane belongs to.
    pub session_name: String,
    /// Lines of captured pane output (trailing blank padding stripped).
    pub lines: Vec<String>,
}

impl PaneDto {
    /// Build a `PaneDto` from a [`Pane`] capture, returning at most `n` trailing lines.
    ///
    /// Pass `None` to include all non-padding lines.
    pub fn from_pane(pane: &Pane, n: Option<usize>) -> Self {
        Self {
            session_name: pane.session_name.clone(),
            lines: pane.last_lines(n),
        }
    }
}

// ── Session request-body DTOs ─────────────────────────────────────────────────

/// Request body for `POST /api/sessions/{name}/send`.
///
/// Sends a literal string of keystrokes to the session followed by `Enter`.
/// Wire format: `{ "keys": "cargo test" }`
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SendBody {
    /// Literal text to send to the session (forwarded with `-l`).
    pub keys: String,
}

/// Request body for `POST /api/sessions/{name}/key`.
///
/// Sends a single named tmux key (e.g. `"Escape"`, `"Up"`, `"C-c"`) without
/// the `-l` flag so tmux resolves the symbolic key name.
///
/// Wire format: `{ "key": "Escape" }`
///
/// Accepted key names include: `Escape`, `Enter`, `Up`, `Down`, `Left`,
/// `Right`, and modifier combinations such as `C-c`.
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyBody {
    /// Symbolic tmux key name to send (e.g. `"Escape"`, `"Up"`, `"C-c"`).
    pub key: String,
}

/// Request body for `POST /api/sessions` (create a new tmux session).
///
/// Wire format: `{ "name": "mysession", "dir": "/optional/start/dir" }`
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewSessionBody {
    /// Name of the new tmux session to create.
    pub name: String,
    /// Optional starting directory for the session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
}

// ── Quick-action command DTOs (BA.11.E) ────────────────────────────────────────

/// Dispatch mode for `POST /actions/command`.
///
/// Serializes/deserializes as the lowercase wire string (`"inject"` / `"spawn"`).
#[typeshare]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandMode {
    /// Send the command into an existing tmux session.
    Inject,
    /// Create a new session, launch `claude`, wait for readiness, then send the command.
    Spawn,
}

/// `model` values accepted for `mode:"spawn"` requests (BA.11.E).
pub const ALLOWED_COMMAND_MODELS: &[&str] = &["opus", "sonnet"];

/// Request body for `POST /actions/command`.
///
/// Wire format (inject):
/// ```json
/// { "mode": "inject", "session": "main", "command": "/status" }
/// ```
///
/// Wire format (spawn):
/// ```json
/// { "mode": "spawn", "name": "work", "dir": "/repo", "model": "sonnet", "command": "/status" }
/// ```
///
/// Field requirements are mode-dependent and enforced by [`CommandRequest::validate`],
/// not by serde: `session` is required for `inject`; `name` is required for `spawn`;
/// `model`, when present, must be one of [`ALLOWED_COMMAND_MODELS`].
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandRequest {
    /// Dispatch mode: `"inject"` or `"spawn"`.
    pub mode: CommandMode,
    /// Target tmux session name. Required when `mode:"inject"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// Name for the new tmux session. Required when `mode:"spawn"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Optional starting directory for a spawned session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
    /// Optional Claude model for a spawned session; one of [`ALLOWED_COMMAND_MODELS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The slash command (or literal text) to send once the target session is ready.
    pub command: String,
}

/// Validation failure for a [`CommandRequest`], returned by [`CommandRequest::validate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandValidationError {
    /// `mode:"inject"` was given without a (non-empty) `session`.
    InjectMissingSession,
    /// `mode:"spawn"` was given without a (non-empty) `name`.
    SpawnMissingName,
    /// `model` was present but not one of [`ALLOWED_COMMAND_MODELS`].
    UnknownModel(String),
}

impl std::fmt::Display for CommandValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InjectMissingSession => {
                write!(f, "mode:\"inject\" requires a non-empty \"session\" field")
            }
            Self::SpawnMissingName => {
                write!(f, "mode:\"spawn\" requires a non-empty \"name\" field")
            }
            Self::UnknownModel(m) => write!(
                f,
                "unknown model {m:?}; expected one of {ALLOWED_COMMAND_MODELS:?}"
            ),
        }
    }
}

impl std::error::Error for CommandValidationError {}

impl CommandRequest {
    /// Validate mode-dependent field requirements.
    ///
    /// Pure — performs no I/O. Checked in order: mode-specific required field first
    /// (empty string counts as missing), then `model` (if present) against the
    /// allow-list, regardless of mode.
    pub fn validate(&self) -> Result<(), CommandValidationError> {
        match self.mode {
            CommandMode::Inject => {
                if self.session.as_deref().unwrap_or("").is_empty() {
                    return Err(CommandValidationError::InjectMissingSession);
                }
            }
            CommandMode::Spawn => {
                if self.name.as_deref().unwrap_or("").is_empty() {
                    return Err(CommandValidationError::SpawnMissingName);
                }
            }
        }
        if let Some(model) = &self.model
            && !ALLOWED_COMMAND_MODELS.contains(&model.as_str())
        {
            return Err(CommandValidationError::UnknownModel(model.clone()));
        }
        Ok(())
    }
}

/// Response body for `POST /actions/command`.
///
/// Wire format: `{ "session": "work" }`
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandResponse {
    /// The target tmux session id (existing for inject, newly created for spawn).
    pub session: String,
}

// ── Repo / workflow status DTOs (BA.11.D) ──────────────────────────────────────

/// JSON response element for `GET /repos` (one per workspace registry entry).
///
/// Wire format: `{ "name": "bastion", "now": "BA.11.D in progress", "has_handoff": false }`
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoSummaryDto {
    /// Workspace registry name.
    pub name: String,
    /// Frontmatter `now:` scalar from the repo's `planning/status.md`.
    pub now: String,
    /// Whether `planning/handoff.md` exists for this workspace.
    pub has_handoff: bool,
}

/// JSON response for `GET /repos/{name}/status`.
///
/// Mirrors [`crate::serve::status::repo::RepoStatus`] field-for-field — kept
/// as an independent DTO (per this module's doc comment) rather than reusing
/// the domain type directly.
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoStatusDto {
    /// Workspace registry name.
    pub name: String,
    /// Frontmatter `now:` scalar.
    pub now: String,
    /// Frontmatter `next:` scalar.
    pub next: String,
    /// Frontmatter `blocked:` scalar.
    pub blocked: String,
    /// Whether `planning/handoff.md` exists.
    pub has_handoff: bool,
    /// Body `## Momentum` → `now` queue line text.
    pub momentum_now: String,
    /// Body `## Momentum` → `next` queue line text.
    pub momentum_next: String,
    /// Body `## Momentum` → `blocked` queue line text.
    pub momentum_blocked: String,
    /// Body `## Momentum` → `improve` queue line text.
    pub momentum_improve: String,
    /// Body `## Momentum` → `recurring` queue line text.
    pub momentum_recurring: String,
}

impl From<crate::serve::status::repo::RepoStatus> for RepoStatusDto {
    fn from(s: crate::serve::status::repo::RepoStatus) -> Self {
        Self {
            name: s.name,
            now: s.now,
            next: s.next,
            blocked: s.blocked,
            has_handoff: s.has_handoff,
            momentum_now: s.momentum_now,
            momentum_next: s.momentum_next,
            momentum_blocked: s.momentum_blocked,
            momentum_improve: s.momentum_improve,
            momentum_recurring: s.momentum_recurring,
        }
    }
}

/// JSON response element for `GET /repos/{name}/workflows`.
///
/// Serializable projection of [`crate::serve::status::flow::FlowState`].
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowStateDto {
    pub spec_slug: String,
    pub branch: String,
    /// Raw status string, e.g. `"running"`, `"done"`, `"blocked"`.
    pub status: String,
    pub current_task: u32,
    pub started_at: String,
    pub updated_at: String,
    /// The engine's `events.id` run UUID that produced this write, stamped by
    /// engine-rs `EN.6.J` into the top-level `run_id` key of
    /// `sdlc-flow-state.json`. `None` for states written before that fix and
    /// for any state written by base-template's JS `sdlc-flow.js` engine,
    /// which never sets it. Deliberately carries `skip_serializing_if` (the
    /// `BoardBlockDto.last_touched` precedent) so `None` serialises as an
    /// **absent key** rather than `null` — a consumer must be able to
    /// distinguish "this run predates the stamp" from "field not understood".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}

impl From<crate::serve::status::flow::FlowState> for WorkflowStateDto {
    fn from(f: crate::serve::status::flow::FlowState) -> Self {
        Self {
            spec_slug: f.spec_slug,
            branch: f.branch,
            status: f.status,
            current_task: f.current_task,
            started_at: f.started_at,
            updated_at: f.updated_at,
            run_id: f.run_id,
        }
    }
}

/// JSON response element for `GET /api/workflows` — one [`WorkflowStateDto`]
/// tagged with the repo it belongs to.
///
/// `WorkflowStateDto` carries `spec_slug` but no repo dimension: in the
/// per-repo route (`GET /repos/{name}/workflows`) the repo is implied by the
/// path, but flattened across every registered repo it is not — two repos can
/// legitimately hold the same `spec_slug`, so a bare list of `WorkflowStateDto`
/// would be ambiguous. This wrapper makes the repo dimension explicit.
///
/// **Shape note:** the natural encoding here would be
/// `{ repo: String, #[serde(flatten)] state: WorkflowStateDto }`, but
/// `typeshare` 1.13 does not support `#[serde(flatten)]` — it is a hard parse
/// error ("The serde flatten attribute is not currently supported"), verified
/// empirically against this exact shape before choosing the fallback below.
/// So this struct mirrors `WorkflowStateDto`'s fields directly rather than
/// composing it; the duplication is forced by the typeshare toolchain, not a
/// design choice. If `WorkflowStateDto` gains or changes a field, mirror the
/// change here too — the `From<(String, WorkflowStateDto)>` impl below will
/// fail to compile if the two drift, since it destructures `WorkflowStateDto`
/// by name.
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoWorkflowStateDto {
    /// The registered workspace name this flow state belongs to.
    pub repo: String,
    pub spec_slug: String,
    pub branch: String,
    /// Raw status string, e.g. `"running"`, `"done"`, `"blocked"`.
    pub status: String,
    pub current_task: u32,
    pub started_at: String,
    pub updated_at: String,
    /// See [`WorkflowStateDto::run_id`] — same semantics, same
    /// absent-key-when-`None` serialization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}

impl From<(String, WorkflowStateDto)> for RepoWorkflowStateDto {
    fn from((repo, state): (String, WorkflowStateDto)) -> Self {
        let WorkflowStateDto {
            spec_slug,
            branch,
            status,
            current_task,
            started_at,
            updated_at,
            run_id,
        } = state;
        Self {
            repo,
            spec_slug,
            branch,
            status,
            current_task,
            started_at,
            updated_at,
            run_id,
        }
    }
}

#[cfg(test)]
mod repo_workflow_state_dto_tests {
    use super::*;

    fn sample_state(run_id: Option<String>) -> WorkflowStateDto {
        WorkflowStateDto {
            spec_slug: "phase11-blockD".to_string(),
            branch: "feat/phase11-blockD".to_string(),
            status: "running".to_string(),
            current_task: 3,
            started_at: "2026-08-01T00:00:00Z".to_string(),
            updated_at: "2026-08-01T01:00:00Z".to_string(),
            run_id,
        }
    }

    #[test]
    fn from_tuple_carries_repo_and_all_state_fields() {
        let state = sample_state(Some("9c6c6f1e-6d1a-4b3a-9b1a-1e2f3a4b5c6d".to_string()));
        let dto = RepoWorkflowStateDto::from(("bastion".to_string(), state.clone()));
        assert_eq!(dto.repo, "bastion");
        assert_eq!(dto.spec_slug, state.spec_slug);
        assert_eq!(dto.branch, state.branch);
        assert_eq!(dto.status, state.status);
        assert_eq!(dto.current_task, state.current_task);
        assert_eq!(dto.started_at, state.started_at);
        assert_eq!(dto.updated_at, state.updated_at);
        assert_eq!(dto.run_id, state.run_id);
    }

    #[test]
    fn round_trip_with_run_id() {
        let state = sample_state(Some("abc-123".to_string()));
        let dto = RepoWorkflowStateDto::from(("orchestrator".to_string(), state));
        let json = serde_json::to_string(&dto).expect("serialize");
        let round_tripped: RepoWorkflowStateDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(round_tripped, dto);
        assert_eq!(round_tripped.run_id.as_deref(), Some("abc-123"));
    }

    #[test]
    fn none_run_id_is_absent_key() {
        let state = sample_state(None);
        let dto = RepoWorkflowStateDto::from(("bastion".to_string(), state));
        let value = serde_json::to_value(&dto).expect("serialize to value");
        let obj = value.as_object().expect("object");
        assert!(
            !obj.contains_key("run_id"),
            "run_id should be an absent key when None, got: {value}"
        );
    }

    #[test]
    fn repo_field_is_present_in_serialized_json() {
        let state = sample_state(None);
        let dto = RepoWorkflowStateDto::from(("bella".to_string(), state));
        let value = serde_json::to_value(&dto).expect("serialize to value");
        assert_eq!(value["repo"], "bella");
    }
}

/// One registered workspace that `GET /api/workflows?with_skipped=1`
/// could not fully report on.
///
/// Returned only inside [`WorkflowsAggregateDto::skipped`] — the default,
/// no-query-param response of `GET /api/workflows` never includes this type.
/// `reason` is a plain `String` on the wire (one of `"unreadable_root"` |
/// `"no_planning_dir"` | `"malformed_flow_state"`), matching how
/// [`RepoWorkflowStateDto::status`] carries a raw status string rather than
/// an enum. See serve-api §11.6 for the full vocabulary, the
/// first-match-wins precedence order, and the "empty is not skipped" rule.
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkippedWorkspaceDto {
    /// The registered workspace name whose report is incomplete.
    pub repo: String,
    /// One of `"unreadable_root"`, `"no_planning_dir"`, `"malformed_flow_state"`.
    pub reason: String,
}

/// The `?with_skipped=1` envelope for `GET /api/workflows`.
///
/// Returned ONLY when the request carries `?with_skipped=1` — the bare-array
/// default response of `GET /api/workflows` is unchanged from v0.23 and does
/// not use this type. `entries` carries the same [`RepoWorkflowStateDto`]
/// list the default response returns, in the same order; `skipped` names
/// every registered workspace whose flow-state report is incomplete, and
/// why. Both fields always serialize, including when empty (`[]`, never an
/// absent key), so a consumer never has to distinguish absent-vs-empty.
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowsAggregateDto {
    pub entries: Vec<RepoWorkflowStateDto>,
    pub skipped: Vec<SkippedWorkspaceDto>,
}

#[cfg(test)]
mod skipped_workspace_dto_tests {
    use super::*;

    #[test]
    fn round_trip() {
        let dto = SkippedWorkspaceDto {
            repo: "bastion".to_string(),
            reason: "unreadable_root".to_string(),
        };
        let json = serde_json::to_string(&dto).expect("serialize");
        let round_tripped: SkippedWorkspaceDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(round_tripped, dto);
    }

    #[test]
    fn carries_repo_and_reason_verbatim() {
        let dto = SkippedWorkspaceDto {
            repo: "orchestrator".to_string(),
            reason: "no_planning_dir".to_string(),
        };
        let value = serde_json::to_value(&dto).expect("serialize to value");
        assert_eq!(value["repo"], "orchestrator");
        assert_eq!(value["reason"], "no_planning_dir");
    }
}

#[cfg(test)]
mod workflows_aggregate_dto_tests {
    use super::*;

    #[test]
    fn round_trip_with_entries_and_skipped() {
        let dto = WorkflowsAggregateDto {
            entries: vec![RepoWorkflowStateDto {
                repo: "bastion".to_string(),
                spec_slug: "phase11-blockD".to_string(),
                branch: "feat/phase11-blockD".to_string(),
                status: "running".to_string(),
                current_task: 3,
                started_at: "2026-08-01T00:00:00Z".to_string(),
                updated_at: "2026-08-01T01:00:00Z".to_string(),
                run_id: None,
            }],
            skipped: vec![SkippedWorkspaceDto {
                repo: "orchestrator".to_string(),
                reason: "malformed_flow_state".to_string(),
            }],
        };
        let json = serde_json::to_string(&dto).expect("serialize");
        let round_tripped: WorkflowsAggregateDto =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(round_tripped, dto);
    }

    #[test]
    fn empty_entries_and_skipped_serialize_as_empty_arrays_not_absent_keys() {
        let dto = WorkflowsAggregateDto {
            entries: Vec::new(),
            skipped: Vec::new(),
        };
        let value = serde_json::to_value(&dto).expect("serialize to value");
        let obj = value.as_object().expect("object");
        assert!(
            obj.contains_key("entries"),
            "entries should be a present key even when empty, got: {value}"
        );
        assert!(
            obj.contains_key("skipped"),
            "skipped should be a present key even when empty, got: {value}"
        );
        assert_eq!(value["entries"], serde_json::json!([]));
        assert_eq!(value["skipped"], serde_json::json!([]));
    }
}

#[cfg(test)]
mod workflow_state_dto_tests {
    use super::*;
    use crate::serve::status::flow::FlowState;

    fn sample_flow_state(run_id: Option<String>) -> FlowState {
        FlowState {
            spec_slug: "phase11-blockD".to_string(),
            branch: "feat/phase11-blockD".to_string(),
            status: "running".to_string(),
            current_task: 3,
            started_at: "2026-08-01T00:00:00Z".to_string(),
            updated_at: "2026-08-01T01:00:00Z".to_string(),
            run_id,
        }
    }

    #[test]
    fn workflow_state_dto_round_trip_with_run_id() {
        let dto = WorkflowStateDto::from(sample_flow_state(Some(
            "9c6c6f1e-6d1a-4b3a-9b1a-1e2f3a4b5c6d".to_string(),
        )));
        let json = serde_json::to_string(&dto).expect("serialize");
        let round_tripped: WorkflowStateDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(round_tripped, dto);
        assert_eq!(
            round_tripped.run_id.as_deref(),
            Some("9c6c6f1e-6d1a-4b3a-9b1a-1e2f3a4b5c6d")
        );
    }

    #[test]
    fn workflow_state_dto_none_run_id_is_absent_key() {
        let dto = WorkflowStateDto::from(sample_flow_state(None));
        let value = serde_json::to_value(&dto).expect("serialize to value");
        let obj = value.as_object().expect("object");
        assert!(
            !obj.contains_key("run_id"),
            "run_id should be an absent key when None, got: {value}"
        );
    }

    #[test]
    fn from_flow_state_carries_run_id_verbatim_some() {
        let flow = sample_flow_state(Some("abc-123".to_string()));
        let dto = WorkflowStateDto::from(flow.clone());
        assert_eq!(dto.run_id, flow.run_id);
    }

    #[test]
    fn from_flow_state_carries_run_id_verbatim_none() {
        let flow = sample_flow_state(None);
        let dto = WorkflowStateDto::from(flow.clone());
        assert_eq!(dto.run_id, flow.run_id);
        assert_eq!(dto.run_id, None);
    }
}

/// JSON response for `GET /repos/{name}/handoff`.
///
/// Mirrors [`crate::serve::status::handoff::HandoffInfo`] field-for-field — kept
/// as an independent DTO (per this module's doc comment) rather than reusing
/// the domain type directly.
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HandoffInfoDto {
    /// Title — frontmatter `title:` scalar if present, else the text after
    /// the first `# Handoff —`/`# Handoff -` heading, else an empty string.
    pub title: String,
    /// The full raw markdown content (including frontmatter, if any).
    pub body: String,
}

impl From<crate::serve::status::handoff::HandoffInfo> for HandoffInfoDto {
    fn from(h: crate::serve::status::handoff::HandoffInfo) -> Self {
        Self {
            title: h.title,
            body: h.body,
        }
    }
}

#[cfg(test)]
mod handoff_info_dto_tests {
    use super::*;
    use crate::serve::status::handoff::HandoffInfo;

    fn sample_handoff_info() -> HandoffInfo {
        HandoffInfo {
            title: "Handoff — sample".to_string(),
            body: "# Handoff — sample\n\nbody text".to_string(),
        }
    }

    #[test]
    fn handoff_info_dto_round_trip() {
        let dto = HandoffInfoDto::from(sample_handoff_info());
        let json = serde_json::to_string(&dto).expect("serialize");
        let round_tripped: HandoffInfoDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(round_tripped, dto);
        assert_eq!(round_tripped.title, "Handoff — sample");
        assert_eq!(round_tripped.body, "# Handoff — sample\n\nbody text");
    }

    #[test]
    fn from_handoff_info_carries_fields_verbatim() {
        let info = sample_handoff_info();
        let dto = HandoffInfoDto::from(info.clone());
        assert_eq!(dto.title, info.title);
        assert_eq!(dto.body, info.body);
    }
}

/// Payload for the server→client `event{workflow_done}` WS push.
///
/// Sent inside an [`EventPayload`]-shaped frame: the `event` field is fixed
/// to `"workflow_done"` and the extra repo/spec_slug/status fields are
/// flattened into the same JSON object by the caller (Task 4 WS wiring).
///
/// Wire format: `{ "repo": "bastion", "spec_slug": "phase11-blockD", "status": "done" }`
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowDonePayload {
    /// Workspace registry name the workflow belongs to.
    pub repo: String,
    /// `sdlc-flow-state.json` spec slug.
    pub spec_slug: String,
    /// The terminal status that triggered the event (`"done"` or `"blocked"`).
    pub status: String,
}

// ── Run transitions (BA.11.N) ────────────────────────────────────────────────

/// Payload for the server→client `event{run_transition}` WS push.
///
/// Sent inside an [`EventPayload`]-shaped frame: the `event` field is fixed
/// to `"run_transition"` and the extra `run_id`/`status`/`terminal`/`spec_slug`
/// fields are flattened into the same JSON object by the caller (`runs`-topic
/// WS wiring, `src/serve/ws/server.rs`).
///
/// Wire format: `{ "run_id": "…", "status": "running", "terminal": false }`
/// (`spec_slug` present only when known).
///
/// `terminal` means **lifecycle**-terminal — the run left `LiveStateStore`'s
/// live map (`list_active()` no longer returns its id) — not "wire-terminal"
/// in the engine's own `publish_suspended` sense. A suspended run is still in
/// the live map, so it is reported as `status: "suspended"` paired with
/// `terminal: false`; only a run's genuine disappearance from the live set
/// (completed, failed, or otherwise retired into the completed ring) emits
/// `terminal: true` (D17 constraint 1).
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunTransitionPayload {
    /// The run's UUID, as a string.
    pub run_id: String,
    /// Aggregate run status string, from `db::workflows::derive_run_status`
    /// via `run_status_str` — the same derivation `GET /api/runs` uses.
    pub status: String,
    /// `true` only when the run has left `LiveStateStore`'s live map
    /// (lifecycle-terminal); `false` for every live-map status, including
    /// `"suspended"`.
    pub terminal: bool,
    /// `sdlc-flow-state.json` spec slug, when known. Absent (not `null`)
    /// when unknown, matching the repo's established DTO convention.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spec_slug: Option<String>,
}

/// Payload for the server→client `event{run_stream_status}` WS push.
///
/// Sent inside an [`EventPayload`]-shaped frame: the `event` field is fixed
/// to `"run_stream_status"` and the extra `available`/`reason` fields are
/// flattened into the same JSON object by the caller. Pushed immediately to
/// a connection when it subscribes to the `runs` topic, before any
/// `run_transition` frame (D17 constraint 2).
///
/// Wire format: `{ "available": true }` or
/// `{ "available": false, "reason": "DATABASE_URL not set" }`.
///
/// `available: false` means the engine was not mounted for this `bastion
/// serve` process, so `LiveStateStore` can never be written and no
/// `run_transition` frame will ever arrive on this connection — the client
/// must fall back to polling `GET /api/runs` / `GET /api/runs/{id}`, which
/// remain the source of truth regardless of `runs`-topic availability (D17
/// constraints 2 and 3).
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunStreamStatusPayload {
    /// Whether the `runs` topic can ever emit a `run_transition` frame on
    /// this connection (i.e. whether the engine is mounted).
    pub available: bool,
    /// Human-readable reason when `available` is `false`. Absent (not
    /// `null`) when `available` is `true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

// ── Board (BA.11.K) ──────────────────────────────────────────────────────────

/// `scope` query param for `GET /api/board`.
///
/// Deserializes from the lowercase wire values (`"hq"`, `"tier"`, `"project"`,
/// `"business"`, `"epic"`); missing/absent `scope` defaults to
/// [`BoardScope::Hq`]. An unknown scope string fails to deserialize (surfaced
/// as a 400 via the existing malformed-request `ErrorPayload` path).
#[typeshare]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BoardScope {
    /// Whole-brain aggregate (`mev::brain::state::TierScope::All`).
    #[default]
    Hq,
    /// Single tier's aggregate board (`TierScope::Tier(<tier>)`, default `"core"`).
    Tier,
    /// Single tier resolved per-project (client renders each project's board
    /// from `repos[]`); same underlying `TierScope::Tier(<tier>)`.
    Project,
    /// Shortcut for `tier=business` (`TierScope::Tier("business")`).
    Business,
    /// Cross-repo initiative projection — every block tagged with the
    /// `&epic=<slug>` query param's epic, across every repo (`TierScope::All`).
    /// `epic` is **required** for this scope; absent or unknown → 404/`C005`.
    Epic,
}

/// One lane entry (a single now/next/blocked/finished block) in a board response.
///
/// Wire format:
/// `{ "id": "BA.11.K", "title": "Cross-brain board read endpoint", "repo": "bastion", "status": "in_progress", "blocked_by": [], "epics": ["bastion-surfaces"], "wave": 3, "priority": 1, "due": "2026-07-15", "track": "Phase 11" }`
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoardBlockDto {
    /// Canonical block ID (e.g. `BA.11.K`).
    pub id: String,
    /// Brief human description, looked up from the owning repo's `tracks[].blocks[]`.
    pub title: String,
    /// Owning repo slug.
    pub repo: String,
    /// Lifecycle status, when known (`"open"`/`"in_progress"`/`"closed"`).
    #[serde(default)]
    pub status: Option<String>,
    /// What this block is waiting on (populated for `blocked` lane entries).
    #[serde(default)]
    pub blocked_by: Vec<okf_core::BlockedBy>,
    /// Cross-repo epic membership — slugs into the HQ `epics[]` registry.
    ///
    /// Joined back from the owning repo's `tracks[].blocks[]`: the rollup
    /// entries `derive_rollup` produces hard-code this empty (see
    /// `../mev/src/brain/state.rs` ~2212–2257). Unlike `okf_core`, this DTO does
    /// **not** carry `skip_serializing_if` — the wire always shows `[]` so TS
    /// clients get a stable array rather than an absent key.
    #[serde(default)]
    pub epics: Vec<String>,
    /// Execution-order rank for "what's next", from the authoring `TrackBlock`.
    ///
    /// Typed `i64` to mirror `okf_core::TrackBlock.wave` exactly (the master plan
    /// wrote `u32`; casting would mangle out-of-range authored values).
    #[serde(default)]
    #[typeshare(serialized_as = "number")]
    pub wave: Option<i64>,
    /// Execution priority (e.g. 1, 2, 3), from the authoring `TrackBlock`.
    #[serde(default)]
    pub priority: Option<u8>,
    /// Target due date or timing string (e.g. `"2026-07-15"`), from the
    /// authoring `TrackBlock`.
    #[serde(default)]
    pub due: Option<String>,
    /// Title of the enclosing `tracks[]` phase/wave entry (`okf_core::Track.title`).
    #[serde(default)]
    pub track: Option<String>,
    /// mev's derived per-block SDLC recency (`MV.10.D`), carried verbatim from
    /// `mev::brain::last_touched::derive_last_touched` — `serve` derives nothing
    /// itself. **Absence means "never worked", not "worked long ago"**: a block
    /// with no resolvable SDLC run has no entry in mev's map and this field is
    /// `None`. Unlike the v0.11 sibling fields above (`wave`/`priority`/`due`/
    /// `track`, which serialise as `null`), this field deliberately carries
    /// `skip_serializing_if` so `None` serialises as an **absent key** rather
    /// than `null` — the block's stated backward-compatibility contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_touched: Option<String>,
    /// Corpus-wide count of in-corpus blocks whose `BlockedBy` edges point at this
    /// block, carried **verbatim** from mev's `BlockGraphNode::dependent_count`
    /// (`../mev/src/brain/block_graph.rs:204`) — `serve` derives nothing itself.
    /// Computed over the full corpus before any scope filtering, so it is
    /// **identical for a given block whether the board is fetched at `hq` scope or
    /// a narrower tier/project scope** (see `../mev/src/brain/block_graph.rs:110-113`)
    /// — this is the property bastion-web's in-scope reverse-dep count
    /// (`lib/board-view.ts:669-676`) structurally cannot have, and the entire
    /// justification for shipping this field. `None` means the block was **absent
    /// from the graph export** (truncated by `max_nodes`, or filtered out of scope
    /// entirely) — never a fabricated zero; mev's own `dependent_count` is `0` for a
    /// block nothing depends on, so `Some(0)` and `None` are deliberately distinct.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependent_count: Option<u32>,
    /// Membership in mev's `ready_order` set, carried verbatim from
    /// `BlockGraphNode::ready`. **This is the readiness signal consumers should
    /// use** — not `unmet_count == 0` (see that field's doc comment). `None` means
    /// the block was absent from the graph export, never a fabricated `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready: Option<bool>,
    /// Count of unmet dependencies, carried verbatim from
    /// `BlockGraphNode::unmet_count`, but populated **only for blocked-lane
    /// entries**. mev defines `unmet_count` as `0` for every non-blocked lane
    /// (`../mev/src/brain/block_graph.rs:104-106`) — that is a structural zero, not
    /// a measurement, so projecting it unqualified onto `now`/`next`/`deferred`/
    /// `finished` blocks would let a consumer read "0 unmet ⇒ ready" and reproduce,
    /// server-blessed, the exact false-ready bug this enrichment exists to kill.
    /// **Consumers must use `ready`, never `unmet_count == 0`, as the readiness
    /// check.** `None` on the blocked lane means the block was absent from the
    /// graph export; `None` on every other lane is the field's normal, permanent
    /// state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unmet_count: Option<u32>,
}

/// The four now/next/blocked/finished lanes for one board (aggregate or per-repo).
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BoardLaneDto {
    /// Blocks currently in progress.
    #[serde(default)]
    pub now: Vec<BoardBlockDto>,
    /// Blocks queued for next (ordered).
    #[serde(default)]
    pub next: Vec<BoardBlockDto>,
    /// Blocks waiting on something.
    #[serde(default)]
    pub blocked: Vec<BoardBlockDto>,
    /// Blocks deliberately parked on the back burner (authored `status == "deferred"`).
    ///
    /// Real roadmap work that is not being surfaced as next. Never overlaps
    /// `next` — a deferred block is structurally excluded from ready-order — and
    /// never overlaps `blocked`, even when it carries unmet deps.
    #[serde(default)]
    pub deferred: Vec<BoardBlockDto>,
    /// Blocks whose `status == "closed"`.
    #[serde(default)]
    pub finished: Vec<BoardBlockDto>,
}

/// One repo's lane breakdown within a `scope=project` board response.
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoBoardDto {
    /// Repo slug.
    pub repo: String,
    /// Tier classification, when known (e.g. `"core"`, `"business"`).
    #[serde(default)]
    pub tier: Option<String>,
    /// This repo's own four lanes.
    pub lanes: BoardLaneDto,
}

/// JSON response for `GET /api/board?scope=hq|tier|project|business[&tier=<name>]`.
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoardDto {
    /// The resolved scope this response was projected under.
    #[serde(default)]
    pub scope: BoardScope,
    /// The resolved tier name, when `scope` is tier-scoped (`None` for `hq`).
    #[serde(default)]
    pub tier: Option<String>,
    /// Aggregate lanes across all in-scope repos.
    pub lanes: BoardLaneDto,
    /// Per-project lane breakdown (populated for `scope=project`; empty otherwise).
    #[serde(default)]
    pub repos: Vec<RepoBoardDto>,
    /// Freshness flag: `true` when any in-scope repo's `status.md` cache lags
    /// its `state.json` (derived from `mev::brain::sync::check_sync`).
    pub stale: bool,
}

// ── Epics (BA.11.R) ──────────────────────────────────────────────────────────

/// One entry in the HQ brain's `epics[]` cross-repo initiative registry —
/// the wire projection of `okf_core::Epic`.
///
/// The registry is HQ-only (D2 precedent, same as `backlog[]`): it is the closed
/// vocabulary a block's `epics[]` membership is validated against by
/// `mev validate-brain`. Membership itself is authored on the blocks, not here —
/// `repos` is a human hint, not the source of truth.
///
/// Wire format:
/// `{ "slug": "bastion-surfaces", "title": "Bastion Surfaces", "description": "…", "status": "active", "plan": "core/planning/master-plan.md", "repos": ["bastion", "bastion-web"] }`
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpicDto {
    /// Stable kebab-case key — the value blocks reference in their `epics[]`.
    pub slug: String,
    /// Human-readable name (e.g. `"Bastion OS"`).
    pub title: String,
    /// One-line description of what the initiative covers.
    #[serde(default)]
    pub description: Option<String>,
    /// Lifecycle: `"active"` · `"paused"` · `"complete"`.
    #[serde(default)]
    pub status: Option<String>,
    /// Authored initiative weight, carried verbatim from `okf_core::Epic.weight`.
    ///
    /// Range policy (`0..=100`) is mev's, enforced by its `check_epics`
    /// (`E_STATE_EPIC_BAD_WEIGHT`) — bastion never clamps, defaults, or
    /// range-checks it, so an out-of-policy authored value reaches the wire
    /// unchanged rather than being silently corrected here.
    ///
    /// `null` on the wire means unauthored (a consumer default applies —
    /// bastion-web currently falls back to 60), which stays distinguishable
    /// from an authored `0`.
    #[serde(default)]
    pub weight: Option<u8>,
    /// Repo-relative path to the owning master-plan / plan doc, when one exists.
    #[serde(default)]
    pub plan: Option<String>,
    /// Repos the initiative is expected to touch — an authored hint for readers.
    #[serde(default)]
    pub repos: Vec<String>,
    /// Member blocks with authored `status == "closed"`.
    #[serde(default)]
    pub closed: u32,
    /// Member blocks with authored `status == "in_progress"`.
    #[serde(default)]
    pub in_progress: u32,
    /// Member blocks that are open (authored `open`, or status absent).
    #[serde(default)]
    pub open: u32,
    /// Member blocks with authored `status == "deferred"`.
    #[serde(default)]
    pub deferred: u32,
    /// Every member block, in any state. `0` means the epic has no members yet.
    #[serde(default)]
    pub total: u32,
    /// Is this epic's remaining work entirely parked?
    ///
    /// True iff it has at least one deferred member and no unfinished
    /// non-deferred work. An epic whose members are all `closed` is *complete*,
    /// not deferred, so this stays false for it.
    ///
    /// Lets a Surface render "this whole initiative is parked" on load, instead
    /// of drawing four empty lane columns and leaving the reader to infer it.
    #[serde(default)]
    pub fully_deferred: bool,
}

// ── Attention (BA.11.P) ──────────────────────────────────────────────────────

/// One `carryover[]` entry that has crossed its per-`kind` staleness threshold.
///
/// Wire format:
/// ```json
/// { "repo": "bastion", "slug": "engine-mount-env", "kind": "env",
///   "text": "engine routes need DATABASE_URL + BASTION_ENGINE_API_KEY set",
///   "clears_when": "the engine mount is documented in .env.example",
///   "created": "2026-07-01", "reviewed": null,
///   "age_days": 23, "threshold_days": 3 }
/// ```
/// Render a typed `okf_core::ClearsWhen` to the display string that crosses the
/// serve boundary on [`AttentionCarryoverDto::clears_when`].
///
/// Pure, no I/O — the typed enum never crosses the wire (BA.ticket.carryover-triage-dto
/// task 1). Handles every `ClearsWhen` / `ClearsWhenPredicate` variant, and appends the
/// predicate's `note` gloss in a consistent `" (note)"` suffix when present.
pub fn render_clears_when(cw: &okf_core::ClearsWhen) -> String {
    use okf_core::{ClearsWhen, ClearsWhenPredicate};

    fn with_note(base: String, note: &Option<String>) -> String {
        match note {
            Some(n) => format!("{base} ({n})"),
            None => base,
        }
    }

    match cw {
        ClearsWhen::Prose(s) => s.clone(),
        ClearsWhen::Predicate(ClearsWhenPredicate::BlockClosed { repo, id, note }) => {
            with_note(format!("block {repo}/{id} is closed"), note)
        }
        ClearsWhen::Predicate(ClearsWhenPredicate::FileExists { path, note }) => {
            with_note(format!("{path} exists"), note)
        }
        ClearsWhen::Predicate(ClearsWhenPredicate::FileContains {
            path,
            pattern,
            note,
        }) => with_note(format!("{path} contains \"{pattern}\""), note),
        ClearsWhen::Predicate(ClearsWhenPredicate::CommandExitsZero { command, note }) => {
            with_note(format!("`{command}` exits zero"), note)
        }
    }
}

/// Projects `mev::CarryoverRanking` verbatim (repo/slug/kind/lane/priority/
/// effective_priority/unmet_blocks/finding_id/clears_when_satisfied) alongside the
/// original entry's display fields (text/clears_when/created/reviewed/threshold_days).
/// Every `Option` serializes as an absent key when `None` — never `null` — matching
/// this repo's established absent-is-never-neutral convention (BA.ticket
/// .carryover-triage-dto task 2).
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttentionCarryoverDto {
    /// Owning repo slug.
    pub repo: String,
    /// Stable item slug.
    pub slug: String,
    /// Carryover kind (`"env"`, `"deferred"`, `"known_issue"`, `"constraint"`, or other).
    pub kind: String,
    /// The carryover text itself, untruncated.
    pub text: String,
    /// What clears this item, when recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clears_when: Option<String>,
    /// Creation date (`YYYY-MM-DD`), when recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    /// Last-reviewed date (`YYYY-MM-DD`), when recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed: Option<String>,
    /// Days past the anchor date (`max(created, reviewed)`), as of `as_of`. `None`
    /// when the entry is currently snoozed or has no parseable anchor date — such
    /// entries still reach the board (contract §2; they no longer have to be
    /// stale-with-an-age to be included).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[typeshare(serialized_as = "number")]
    pub age_days: Option<i64>,
    /// The per-`kind` threshold this item tripped.
    #[typeshare(serialized_as = "number")]
    pub threshold_days: i64,
    /// The triage lane `mev::rank_carryover` assigned (`blocking|hot|aging|standing`).
    /// Never re-derived here — see contract §6 rule 1.
    pub lane: TriageLane,
    /// Authored `priority`, verbatim from `mev::CarryoverRanking`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<u8>,
    /// Effective priority (reverse-topo min-propagation over `blocks[]` edges),
    /// verbatim from `mev::CarryoverRanking`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_priority: Option<u8>,
    /// Unmet `blocks[]` edges. Non-empty iff this entry is in the BLOCKING lane.
    /// There is deliberately no `blocking: bool` field (contract §6 rule 2) —
    /// consumers derive it from `!unmet_blocks.is_empty()`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unmet_blocks: Vec<String>,
    /// Free-form cross-repo finding identity, verbatim from `mev::CarryoverRanking`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finding_id: Option<String>,
    /// Whether every reference extracted from `clears_when` is currently satisfied
    /// (`mev::CarryoverLane::Cleared`), verbatim from `mev::CarryoverRanking`.
    pub clears_when_satisfied: bool,
}

/// One `backlog[]` node that has crossed the backlog staleness threshold — used for both the
/// `aging_backlog` lane (non-capture nodes) and the `orphaned_captures` lane (`origin.type ==
/// "capture"` nodes, never both).
///
/// Wire format:
/// ```json
/// { "repo": "bastion", "slug": "serve-attention-endpoint", "title": "Attention projection",
///   "kind": "feature", "status": "ready", "notes": null,
///   "created": "2026-07-10", "reviewed": null,
///   "age_days": 14, "threshold_days": 7 }
/// ```
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttentionBacklogDto {
    /// Owning repo slug.
    pub repo: String,
    /// Stable item slug.
    pub slug: String,
    /// Human-readable title.
    pub title: String,
    /// Backlog kind (serde-renamed from `type` on the domain type).
    pub kind: String,
    /// Lifecycle status (`"idea"` / `"ready"`, the only statuses that can age).
    pub status: String,
    /// Notes; for `orphaned_captures` this is `origin.notes` falling back to the node's own
    /// `notes`.
    #[serde(default)]
    pub notes: Option<String>,
    /// Creation date (`YYYY-MM-DD`), when recorded.
    #[serde(default)]
    pub created: Option<String>,
    /// Last-reviewed date (`YYYY-MM-DD`), when recorded.
    #[serde(default)]
    pub reviewed: Option<String>,
    /// Days past the anchor date (`max(created, reviewed)`), as of `as_of`.
    #[typeshare(serialized_as = "number")]
    pub age_days: i64,
    /// The `backlog_days` threshold this item tripped.
    #[typeshare(serialized_as = "number")]
    pub threshold_days: i64,
}

/// The three Attention board lanes.
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AttentionLanesDto {
    /// `carryover[]` entries past their per-`kind` threshold, oldest-first.
    #[serde(default)]
    pub stale_carryover: Vec<AttentionCarryoverDto>,
    /// Non-capture `backlog[]` nodes past `backlog_days`, oldest-first.
    #[serde(default)]
    pub aging_backlog: Vec<AttentionBacklogDto>,
    /// `backlog[]` nodes with `origin.type == "capture"` past `backlog_days`, oldest-first.
    #[serde(default)]
    pub orphaned_captures: Vec<AttentionBacklogDto>,
}

/// The resolved `brain.toml` `[attention]` thresholds, mirroring
/// `mev::brain::config::AttentionThresholds`.
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttentionThresholdsDto {
    /// Threshold (days) for `kind == "env"` carryover.
    #[typeshare(serialized_as = "number")]
    pub env_days: i64,
    /// Threshold (days) for `kind == "deferred"` carryover.
    #[typeshare(serialized_as = "number")]
    pub deferred_days: i64,
    /// Threshold (days) for `kind == "known_issue"` carryover.
    #[typeshare(serialized_as = "number")]
    pub known_issue_days: i64,
    /// Threshold (days) for `kind == "constraint"` carryover.
    #[typeshare(serialized_as = "number")]
    pub constraint_days: i64,
    /// Threshold (days) for aging/orphaned `backlog[]` nodes.
    #[typeshare(serialized_as = "number")]
    pub backlog_days: i64,
}

/// JSON response for `GET /api/attention?scope=hq|tier|project|business[&tier=<name>]`.
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttentionDto {
    /// The resolved scope this response was projected under. Reuses [`BoardScope`] — the
    /// `/api/attention` query semantics are identical to `/api/board`.
    #[serde(default)]
    pub scope: BoardScope,
    /// The resolved tier name, when `scope` is tier-scoped (`None` for `hq`).
    #[serde(default)]
    pub tier: Option<String>,
    /// `YYYY-MM-DD` the ages were computed against.
    pub as_of: String,
    /// The three Attention lanes.
    pub lanes: AttentionLanesDto,
    /// The resolved thresholds used to compute every `age_days` / `threshold_days` pair above.
    pub thresholds: AttentionThresholdsDto,
}

// ── Live run read DTOs (BA.11.M) ───────────────────────────────────────────────

/// Token/model usage for an LLM node, projected from `engine_contract::task_context::Usage`.
///
/// Wire format:
/// ```json
/// { "input_tokens": 512, "output_tokens": 128, "model": "claude-sonnet-5" }
/// ```
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunUsageDto {
    /// Prompt token count, when reported by the provider.
    #[serde(default)]
    #[typeshare(serialized_as = "number")]
    pub input_tokens: Option<u64>,
    /// Completion token count, when reported by the provider.
    #[serde(default)]
    #[typeshare(serialized_as = "number")]
    pub output_tokens: Option<u64>,
    /// Model identifier used for this node's LLM call.
    pub model: String,
}

/// One node's projected run state — the join of `TaskContext::node_runs[class]`
/// (status/timing/error/input/usage) with `TaskContext::nodes[class]` (output),
/// keyed by the node's class name (BA.11.M).
///
/// Wire format:
/// ```json
/// {
///   "node": "DataIngestionNode",
///   "status": "success",
///   "started_at": "2026-07-24T12:00:00Z",
///   "completed_at": "2026-07-24T12:00:01Z",
///   "error": null,
///   "input": null,
///   "output": { "documents_loaded": 3 },
///   "usage": null
/// }
/// ```
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeTransitionDto {
    /// Node identity — the Python class name (contract §1), used as the map key
    /// in both `TaskContext::nodes` and `TaskContext::node_runs`.
    pub node: String,
    /// Lifecycle status as the lowercase wire string: `pending`/`running`/`success`/`failed`.
    pub status: String,
    /// ISO-8601 UTC timestamp set on entry, `null` while pending.
    #[serde(default)]
    pub started_at: Option<String>,
    /// ISO-8601 UTC timestamp set on success or failure, `null` before completion.
    #[serde(default)]
    pub completed_at: Option<String>,
    /// Error message, present only for a `failed` node.
    #[serde(default)]
    pub error: Option<String>,
    /// The node's recorded input, present only for a `failed` node.
    #[serde(default)]
    pub input: Option<serde_json::Value>,
    /// The node's output value from `TaskContext::nodes`, `null` when not yet produced.
    #[serde(default)]
    pub output: Option<serde_json::Value>,
    /// Token/model usage, present only for LLM nodes.
    #[serde(default)]
    pub usage: Option<RunUsageDto>,
}

/// JSON response for `GET /api/runs/{id}` — the projected `LiveStateStore` snapshot
/// for one run (BA.11.M).
///
/// Wire format:
/// ```json
/// {
///   "run_id": "b6a1...",
///   "event": { "ticket_id": "T-1" },
///   "metadata": { "workflow": "sdlc-flow" },
///   "nodes": [ { "node": "DataIngestionNode", "status": "success", ... } ]
/// }
/// ```
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunStateDto {
    /// The run's UUID as a string.
    pub run_id: String,
    /// The triggering event payload, carried through from `TaskContext::event`.
    pub event: serde_json::Value,
    /// Workflow-level metadata, carried through from `TaskContext::metadata`.
    pub metadata: serde_json::Value,
    /// Per-node projected states, one entry per class name in `TaskContext::node_runs`.
    pub nodes: Vec<NodeTransitionDto>,
}

/// One live run's summary projection — `GET /api/runs` (BA.11.T).
///
/// A widened successor to the bare-UUID `Vec<String>` `GET /api/runs` used to return: each
/// entry now carries enough to render a live-runs band (spec slug, derived status, timing)
/// without an N+1 `GET /api/runs/{id}` fetch per run.
///
/// Wire format (both variants — `spec_slug` present vs. omitted):
/// ```json
/// {
///   "run_id": "b6a1c1e0-0000-4000-8000-000000000000",
///   "status": "running",
///   "spec_slug": "11.T-run-summary-projection",
///   "started_at": "2026-07-24T12:00:00Z",
///   "updated_at": "2026-07-24T12:00:01Z"
/// }
/// ```
/// ```json
/// {
///   "run_id": "c7b2d2f1-0000-4000-8000-000000000000",
///   "status": "pending",
///   "started_at": null,
///   "updated_at": null
/// }
/// ```
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunSummaryDto {
    /// The run's UUID as a string.
    pub run_id: String,
    /// Workflow identity (e.g. `"sdlc-flow"`). **Always absent today** — no production code
    /// stamps a workflow-identity key anywhere `bastion` can read it from a live `TaskContext`;
    /// `engine-serve` only tracks it in a process-local, `pub(crate)`-scoped side table. Tracked
    /// by the engine-rs follow-up ticket `EN.ticket.expose-live-run-workflow-type`
    /// (`core/engine-rs/planning/ticket-expose-live-run-workflow-type/`); this DTO does not
    /// fabricate a value in the meantime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_type: Option<String>,
    /// Lifecycle status as the lowercase wire string, derived via
    /// `db::workflows::derive_run_status`: `pending`/`running`/`success`/`failed`/
    /// `cancelled`/`budget_halted`/`suspended` (v0.17). `suspended` is not
    /// terminal — a resumed run falls back through to the other rules.
    pub status: String,
    /// The triggering event's `spec_slug` field, when present. Omitted (not `null`) when the
    /// run's event carries no `spec_slug` key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec_slug: Option<String>,
    /// Earliest non-null `node_runs[*].started_at` across all tracked nodes, as RFC3339.
    /// `null` when the run has no recorded node transitions yet.
    #[serde(default)]
    pub started_at: Option<String>,
    /// Latest non-null `node_runs[*].started_at` **or** `completed_at` across all tracked
    /// nodes, as RFC3339. `null` when the run has no recorded node transitions yet.
    #[serde(default)]
    pub updated_at: Option<String>,
    /// The repo that owns this run, resolved by an **exact `run_id` match** against the
    /// registry's flow state (`RepoWorkflowStateDto` from `collect_all_workflows`, A2). Absent
    /// (never `null`) when no flow state carries this run's `run_id` — the run could not be
    /// attributed to a repo. A wrong label would be strictly worse than an absent one, so this
    /// field is never guessed via substring, prefix, or spec-slug similarity matching (A7).
    /// Only populated when the request opts in via `?with_repo=1`; otherwise always absent, to
    /// keep the unopted poll path free of the registry walk that resolving `repo` requires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
}

// ── Docs read API (BA.11.Q) ─────────────────────────────────────────────────────

/// One entry in a `GET /api/docs/{repo}/tree` listing.
///
/// Wire format:
/// ```json
/// { "path": "planning/status.md", "name": "status.md", "is_dir": false }
/// ```
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocEntryDto {
    /// Repo-root-relative path — exactly what `GET /api/docs/{repo}/file`'s `?path=` accepts.
    pub path: String,
    /// The entry's base name (final path component).
    pub name: String,
    /// `true` when the entry is a directory, `false` for a file.
    pub is_dir: bool,
}

/// JSON response for `GET /api/docs/{repo}/tree?path=<rel-dir>`.
///
/// Wire format:
/// ```json
/// {
///   "repo": "bastion",
///   "root": "planning",
///   "entries": [
///     { "path": "planning/status.md", "name": "status.md", "is_dir": false },
///     { "path": "planning/decisions", "name": "decisions", "is_dir": true }
///   ]
/// }
/// ```
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocTreeDto {
    /// Echoes the resolved workspace registry name.
    pub repo: String,
    /// The allowlisted root (or subtree) the listing is relative to; `""` when the whole
    /// allowlist was walked.
    pub root: String,
    /// Sorted directories-first, then by `name`. Only markdown-bearing entries appear.
    #[serde(default)]
    pub entries: Vec<DocEntryDto>,
}

/// JSON response for `GET /api/docs/{repo}/file?path=<rel-file>`.
///
/// Wire format:
/// ```json
/// {
///   "repo": "bastion",
///   "path": "planning/status.md",
///   "content": "---\ntype: Status\n---\n\n# bastion — Status\n…",
///   "bytes": 8421,
///   "modified": "2026-07-24T18:03:11Z"
/// }
/// ```
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocFileDto {
    /// Echoes the resolved workspace registry name.
    pub repo: String,
    /// The validated, repo-root-relative path that was read.
    pub path: String,
    /// The file's raw markdown, byte-for-byte — no rendering, no frontmatter stripping, no
    /// sentinel removal.
    pub content: String,
    /// Content length in bytes.
    #[typeshare(serialized_as = "number")]
    pub bytes: u64,
    /// Filesystem mtime, RFC 3339 UTC; `None` when unavailable.
    #[serde(default)]
    pub modified: Option<String>,
}

// ── Pipeline / opportunities API (BW.3.A) ────────────────────────────────────
//
// Read projection over the business sub-brain's opportunity markdown files
// (`business/docs/opportunities/*.md` + `business/docs/leads/*.md`). Read-only
// (D25) — the API never writes. Backing handlers live in
// `handlers/pipeline.rs`.

/// JSON response for `GET /api/pipeline`.
///
/// `stages` is the canonical stage vocabulary/order parsed from
/// `business/docs/pipeline.md`'s `## Stages` line; `opportunities` are the
/// per-file summaries, sorted by stage order (index in `stages`, unknown/none
/// last) then title.
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipelineDto {
    /// Canonical stage vocabulary in pipeline order.
    #[serde(default)]
    pub stages: Vec<String>,
    /// One summary per opportunity file (opportunities + leads).
    #[serde(default)]
    pub opportunities: Vec<OpportunitySummaryDto>,
}

/// A single opportunity, projected for the list view (`GET /api/pipeline`).
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpportunitySummaryDto {
    /// File stem (e.g. `anthropic`) — the `{slug}` for the detail route.
    pub slug: String,
    /// `kind` frontmatter (`company` | `prospecting-sweep` | `job-posting`);
    /// defaults to `company` when absent.
    pub kind: String,
    /// `title` frontmatter (falls back to `slug` when absent).
    pub title: String,
    /// `source` frontmatter, when present.
    #[serde(default)]
    pub source: Option<String>,
    /// `stage` frontmatter, when present.
    #[serde(default)]
    pub stage: Option<String>,
    /// `last_contact` frontmatter, when present.
    #[serde(default)]
    pub last_contact: Option<String>,
    /// `next_action` frontmatter, when present.
    #[serde(default)]
    pub next_action: Option<String>,
    /// Whether the body carries a parseable research brief (```json fence).
    pub has_findings: bool,
}

/// The full projection for one opportunity (`GET /api/pipeline/{slug}`).
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpportunityDetailDto {
    /// File stem (e.g. `anthropic`).
    pub slug: String,
    /// `kind` frontmatter; defaults to `company` when absent.
    pub kind: String,
    /// `title` frontmatter (falls back to `slug` when absent).
    pub title: String,
    /// `source` frontmatter, when present.
    #[serde(default)]
    pub source: Option<String>,
    /// `stage` frontmatter, when present.
    #[serde(default)]
    pub stage: Option<String>,
    /// `last_contact` frontmatter, when present.
    #[serde(default)]
    pub last_contact: Option<String>,
    /// `next_action` frontmatter, when present.
    #[serde(default)]
    pub next_action: Option<String>,
    /// `url` frontmatter, when present.
    #[serde(default)]
    pub url: Option<String>,
    /// `links` frontmatter list.
    #[serde(default)]
    pub links: Vec<String>,
    /// `research_ref` frontmatter, when present.
    #[serde(default)]
    pub research_ref: Option<String>,
    /// `contacts` frontmatter list.
    #[serde(default)]
    pub contacts: Vec<ContactDto>,
    /// The research brief parsed from the body's first ```json fence, when present.
    #[serde(default)]
    pub findings: Option<ResearchBriefDto>,
    /// `actions` frontmatter list (activity log).
    #[serde(default)]
    pub actions: Vec<OpportunityActionDto>,
    /// The markdown body (everything after the frontmatter block), when non-empty.
    #[serde(default)]
    pub body_markdown: Option<String>,
}

/// A single contact channel bundle for an opportunity.
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContactDto {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub emails: Vec<String>,
    #[serde(default)]
    pub whatsapp: Vec<String>,
    #[serde(default)]
    pub phones: Vec<String>,
    #[serde(default)]
    pub links: Vec<String>,
    #[serde(default)]
    pub note: Option<String>,
}

/// One entry in an opportunity's `actions` activity log.
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpportunityActionDto {
    /// Date/timestamp of the action (e.g. `2026-07-25`).
    pub at: String,
    /// Action kind (e.g. `research`, `outreach`).
    pub kind: String,
    /// Free-text note.
    pub note: String,
}

/// The research brief embedded in an opportunity body's first ```json fence.
///
/// `kind` is `"company"` (a CompanyBrief — has `company_name`) or
/// `"prospecting"` (a ProspectingResult — has `prospects`/`vertical`); the two
/// families' fields are unioned so a single DTO covers both, with the unused
/// family's fields left empty/`None`.
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResearchBriefDto {
    /// `"company"` or `"prospecting"`.
    pub kind: String,
    // ── CompanyBrief fields ──
    #[serde(default)]
    pub company_name: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub recent_developments: Vec<String>,
    #[serde(default)]
    pub pain_points: Vec<String>,
    #[serde(default)]
    pub outreach_hooks: Vec<String>,
    #[serde(default)]
    pub sources: Vec<String>,
    // ── ProspectingResult fields ──
    #[serde(default)]
    pub vertical: Option<String>,
    #[serde(default)]
    pub common_pain_points: Vec<String>,
    #[serde(default)]
    pub prospects: Vec<ProspectLeadDto>,
}

/// One prospect archetype inside a ProspectingResult research brief.
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProspectLeadDto {
    pub name: String,
    pub pillar: String,
    #[serde(default)]
    pub pain_points: Vec<String>,
    #[serde(default)]
    pub outreach_hook: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

// ── Block graph (BA.17.A) ──────────────────────────────────────────────────────
//
// Mechanical projection of `mev::brain::block_graph::BlockGraphExport` for
// `GET /api/blocks/graph`. Bastion performs zero derivation of its own — every
// field here is copied straight across from the upstream mev type by
// `handlers/block_graph.rs::block_graph_dto`. `BlockEdgeKindDto` is declared
// locally rather than reusing `okf_core::StateEdgeKind` because typeshare only
// scans `src/serve` — an okf-core type would never reach `types/serve.ts`.

/// Mirrors `mev::brain::block_graph::BlockLane`'s six variants. Serialises
/// `snake_case` to match upstream (`"now"`, `"next"`, `"blocked"`, `"deferred"`,
/// `"closed"`, `"other"`).
#[typeshare]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockLaneDto {
    /// Authored `status == "in_progress"`.
    Now,
    /// Ready — open, no external deps, all block deps closed.
    Next,
    /// Open with at least one unmet dependency.
    Blocked,
    /// Authored `status == "deferred"`.
    Deferred,
    /// Authored `status == "closed"`.
    Closed,
    /// An authored status that matches none of the above.
    Other,
}

/// Mirrors `okf_core::state::StateEdgeKind`'s two variants. Serialises
/// `snake_case` to match upstream (`"blocked_by"`, `"cross_repo"`). Declared
/// locally rather than reusing `okf_core::StateEdgeKind` — see the module note
/// above.
#[typeshare]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockEdgeKindDto {
    /// A `blocked_by` dependency (a block is waiting on another block).
    BlockedBy,
    /// An explicit cross-repo dependency declared in a brain file's `cross_repo[]`.
    CrossRepo,
}

/// Mirrors `mev::brain::block_graph::BlockGraphNode` field-for-field.
///
/// Wire format:
/// ```json
/// {
///   "key": "bastion:BA.17.A", "repo": "bastion", "id": "BA.17.A",
///   "title": "GET /api/blocks/graph endpoint", "status": "in_progress",
///   "lane": "now", "track": "Phase 17", "wave": 1, "priority": 1,
///   "effective_priority": 1, "due": null, "epics": ["bastion-surfaces"],
///   "layer": 0, "topo_index": 4, "ready": true, "in_cycle": false,
///   "in_scope": true, "external_deps": [], "unmet_count": 0, "dependent_count": 0
/// }
/// ```
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockGraphNodeDto {
    /// Canonical `"repo:id"` key.
    pub key: String,
    /// Owning repo slug.
    pub repo: String,
    /// Canonical block ID.
    pub id: String,
    /// Brief human description.
    pub title: String,
    /// Authored lifecycle status (`open`/`in_progress`/`deferred`/`closed`), if any.
    #[serde(default)]
    pub status: Option<String>,
    /// Derived attention lane.
    pub lane: BlockLaneDto,
    /// Title of the containing `tracks[]` phase/wave, if resolvable.
    #[serde(default)]
    pub track: Option<String>,
    /// Authored execution-order rank.
    #[serde(default)]
    #[typeshare(serialized_as = "number")]
    pub wave: Option<i64>,
    /// Authored own priority.
    #[serde(default)]
    pub priority: Option<u8>,
    /// Effective priority — absent when it never lands in the real `0..=3` range.
    #[serde(default)]
    pub effective_priority: Option<u8>,
    /// Authored due date/timing string.
    #[serde(default)]
    pub due: Option<String>,
    /// Cross-repo epic membership.
    #[serde(default)]
    pub epics: Vec<String>,
    /// Longest path over resolved `depends_on` edges (`0` = no resolved prerequisites).
    #[typeshare(serialized_as = "number")]
    pub layer: u32,
    /// Position in the full-corpus topological order.
    #[typeshare(serialized_as = "number")]
    pub topo_index: u32,
    /// Membership in the ready-order set.
    pub ready: bool,
    /// Whether this node participates in a `depends_on` cycle.
    pub in_cycle: bool,
    /// Whether this node survives the scope pipeline's tier/repo/epic/closed stages.
    pub in_scope: bool,
    /// `what` strings from this block's `{type:"external"}` `depends_on` entries.
    #[serde(default)]
    pub external_deps: Vec<String>,
    /// Count of unmet dependencies for a `Blocked` node — `0` for every other lane.
    #[typeshare(serialized_as = "number")]
    pub unmet_count: u32,
    /// Corpus-wide count of in-corpus blocks whose `BlockedBy` edges point at this node.
    #[typeshare(serialized_as = "number")]
    pub dependent_count: u32,
}

/// Mirrors `mev::brain::block_graph::BlockGraphEdge` field-for-field.
///
/// Wire format:
/// ```json
/// { "from": "bastion:BA.17.A", "to_ref": "mev:MV.10.B", "kind": "blocked_by",
///   "target_node_id": "mev:MV.10.B", "blocking": false }
/// ```
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockGraphEdgeDto {
    /// `"repo:id"` key of the source (dependent) block.
    pub from: String,
    /// Raw, as-authored `"repo:id"` reference.
    pub to_ref: String,
    /// Edge discriminant.
    pub kind: BlockEdgeKindDto,
    /// `Some(to_ref)` when it resolves to a node in this export; `None` when dangling.
    #[serde(default)]
    pub target_node_id: Option<String>,
    /// `false` when either endpoint is `closed`.
    pub blocking: bool,
}

/// JSON response for `GET /api/blocks/graph?scope=...&tier=...&epic=...&repo=...
/// &include_closed=...&include_boundary=...&max_nodes=...`.
///
/// A mechanical projection of `mev::brain::block_graph::BlockGraphExport` plus the
/// response-level `scope` echo (reusing [`BoardScope`]) and `stale` flag, matching
/// `BoardDto`'s convention.
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockGraphDto {
    /// Schema version — currently `"1"`.
    pub version: String,
    /// Display path of the brain root used for the build.
    pub root: String,
    /// The resolved scope this response was projected under.
    #[serde(default)]
    pub scope: BoardScope,
    /// The resolved tier name, when `scope` is tier-scoped (`None` for `hq`).
    #[serde(default)]
    pub tier: Option<String>,
    /// The resolved `&epic=` slug, when `scope=epic`.
    #[serde(default)]
    pub epic: Option<String>,
    /// The resolved `&repo=` restriction, when present.
    #[serde(default)]
    pub repo: Option<String>,
    /// Whether `Closed`-lane nodes were retained.
    pub include_closed: bool,
    /// Whether direct neighbours of the in-scope set were re-added as boundary nodes.
    pub include_boundary: bool,
    /// Nodes, emitted in `topo_index` order.
    #[serde(default)]
    pub nodes: Vec<BlockGraphNodeDto>,
    /// Edges — one per surviving scoped edge.
    #[serde(default)]
    pub edges: Vec<BlockGraphEdgeDto>,
    /// Cycles found over the full corpus (never the scoped subgraph).
    #[serde(default)]
    pub cycles: Vec<Vec<String>>,
    /// Node count before any `max_nodes` truncation.
    #[typeshare(serialized_as = "number")]
    pub total_nodes: u32,
    /// Whether `max_nodes` truncated the node list.
    pub truncated: bool,
    /// Freshness flag: `true` when any in-scope repo's `status.md` cache lags its
    /// `state.json` (same posture as `BoardDto::stale`).
    pub stale: bool,
}

// ── Cost read API (BA.11.J) ─────────────────────────────────────────────────────

/// One per-workflow aggregated cost row — mirrors `costs::WorkflowCost` exactly.
///
/// Wire format:
/// `{ "workflow_name": "content-pipeline", "runs": 12, "tokens_in": 48000, "tokens_out": 9000, "usd": 1.32 }`
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowCostDto {
    /// Distinct workflow name this row aggregates.
    pub workflow_name: String,
    /// Number of runs contributing to this row.
    #[typeshare(serialized_as = "number")]
    pub runs: u64,
    /// Total input tokens across the contributing runs.
    #[typeshare(serialized_as = "number")]
    pub tokens_in: u64,
    /// Total output tokens across the contributing runs.
    #[typeshare(serialized_as = "number")]
    pub tokens_out: u64,
    /// Total USD cost across the contributing runs.
    pub usd: f64,
}

/// Detail of a breached budget cap — mirrors `costs::budget::BreachReason`.
///
/// `cap` carries `Cap::as_str`'s exact strings (`"max_total_tokens"` /
/// `"max_cost_usd"`), matching what the Engine stamps into
/// `metadata.budget.reason.cap` (contract v1.1.0 §5).
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetBreachDto {
    /// Which cap was breached: `"max_total_tokens"` or `"max_cost_usd"`.
    pub cap: String,
    /// The spend value that tripped the cap.
    pub spent: f64,
    /// The configured limit that was reached.
    pub limit: f64,
}

/// Budget configuration + current gate state for the aggregated window.
///
/// Read-only projection: caps are reported as configured (from `Config`), not
/// mutated over HTTP — budget mutation stays CLI/D48. A run with no caps
/// configured (`Budget::default()`) always reports `breached: false`.
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetStateDto {
    /// Configured token cap, when set.
    #[serde(default)]
    #[typeshare(serialized_as = "number")]
    pub max_total_tokens: Option<u64>,
    /// Configured USD cost cap, when set.
    #[serde(default)]
    pub max_cost_usd: Option<f64>,
    /// Current total tokens spent (`tokens_in + tokens_out`) for the window.
    #[typeshare(serialized_as = "number")]
    pub total_tokens: u64,
    /// Current total USD cost for the window.
    pub total_cost_usd: f64,
    /// Whether any configured cap has been reached (`>=`, per `evaluate`'s
    /// documented boundary).
    pub breached: bool,
    /// Which cap was breached and by how much, when `breached` is `true`.
    #[serde(default)]
    pub breach: Option<BudgetBreachDto>,
}

/// `GET /api/costs` response body — read-only projection of `costs::CostSummary`
/// plus the BA.7.C budget state for the resolved window.
///
/// Wire format:
/// ```json
/// {
///   "window": "7d",
///   "rows": [{ "workflow_name": "content-pipeline", "runs": 12, "tokens_in": 48000, "tokens_out": 9000, "usd": 1.32 }],
///   "totals": { "workflow_name": "TOTAL", "runs": 12, "tokens_in": 48000, "tokens_out": 9000, "usd": 1.32 },
///   "unpriced_models": [],
///   "budget": { "max_total_tokens": null, "max_cost_usd": null, "total_tokens": 57000, "total_cost_usd": 1.32, "breached": false, "breach": null }
/// }
/// ```
#[typeshare]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostSummaryDto {
    /// The resolved window echoed back to the client (`"7d"` / `"30d"` / `"all"`).
    pub window: String,
    /// One row per distinct `workflow_name`, sorted by `usd` descending.
    #[serde(default)]
    pub rows: Vec<WorkflowCostDto>,
    /// Totals across all rows.
    pub totals: WorkflowCostDto,
    /// Model IDs that appeared in the data but have no price entry — spend is
    /// under-reported for these rather than silently omitted.
    #[serde(default)]
    pub unpriced_models: Vec<String>,
    /// Budget configuration + current gate state for this window.
    pub budget: BudgetStateDto,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── HealthResponse ─────────────────────────────────────────────────────

    #[test]
    fn health_response_ok_constructor() {
        let h = HealthResponse::ok();
        assert_eq!(h.status, "ok");
        assert_eq!(h.service, "bastion");
    }

    #[test]
    fn health_response_serializes_to_expected_json() {
        let h = HealthResponse::ok();
        let v = serde_json::to_value(&h).expect("serialize HealthResponse");
        assert_eq!(v["status"], "ok");
        assert_eq!(v["service"], "bastion");
    }

    #[test]
    fn health_response_round_trip() {
        let original = HealthResponse::ok();
        let json = serde_json::to_string(&original).expect("serialize");
        let decoded: HealthResponse = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, decoded, "round-trip must preserve all fields");
    }

    #[test]
    fn health_response_deserializes_from_json() {
        let raw = r#"{"status":"ok","service":"bastion"}"#;
        let h: HealthResponse = serde_json::from_str(raw).expect("deserialize HealthResponse");
        assert_eq!(h.status, "ok");
        assert_eq!(h.service, "bastion");
    }

    #[test]
    fn health_response_rejects_missing_status_field() {
        let raw = r#"{"service":"bastion"}"#;
        let result: Result<HealthResponse, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "deserialize must fail when 'status' field is missing"
        );
    }

    // ── WsFrameKind ────────────────────────────────────────────────────────

    #[test]
    fn ws_frame_kind_echo_serializes_snake_case() {
        let kind = WsFrameKind::Echo;
        let v = serde_json::to_value(&kind).expect("serialize WsFrameKind::Echo");
        assert_eq!(v, json!("echo"), "Echo must serialize to snake_case 'echo'");
    }

    #[test]
    fn ws_frame_kind_error_serializes_snake_case() {
        let kind = WsFrameKind::Error;
        let v = serde_json::to_value(&kind).expect("serialize WsFrameKind::Error");
        assert_eq!(
            v,
            json!("error"),
            "Error must serialize to snake_case 'error'"
        );
    }

    #[test]
    fn ws_frame_kind_echo_deserializes() {
        let kind: WsFrameKind =
            serde_json::from_str(r#""echo""#).expect("deserialize 'echo' variant");
        assert_eq!(kind, WsFrameKind::Echo);
    }

    #[test]
    fn ws_frame_kind_error_deserializes() {
        let kind: WsFrameKind =
            serde_json::from_str(r#""error""#).expect("deserialize 'error' variant");
        assert_eq!(kind, WsFrameKind::Error);
    }

    #[test]
    fn ws_frame_kind_unknown_variant_fails() {
        let result: Result<WsFrameKind, _> = serde_json::from_str(r#""unknown_kind""#);
        assert!(
            result.is_err(),
            "unknown kind variant must fail to deserialize"
        );
    }

    // ── WsFrame envelope ───────────────────────────────────────────────────

    #[test]
    fn ws_frame_echo_round_trip() {
        let frame = WsFrame {
            kind: WsFrameKind::Echo,
            payload: json!({"text": "hello"}),
        };
        let json = serde_json::to_string(&frame).expect("serialize WsFrame");
        let decoded: WsFrame = serde_json::from_str(&json).expect("deserialize WsFrame");
        assert_eq!(
            frame, decoded,
            "WsFrame round-trip must preserve kind + payload"
        );
    }

    #[test]
    fn ws_frame_serializes_kind_as_snake_case_tag() {
        let frame = WsFrame {
            kind: WsFrameKind::Echo,
            payload: json!(null),
        };
        let v = serde_json::to_value(&frame).expect("serialize WsFrame");
        assert_eq!(
            v["kind"], "echo",
            "WsFrame.kind must be the snake_case discriminant string; got {v}"
        );
    }

    #[test]
    fn ws_frame_payload_preserved_unchanged() {
        let payload = json!({"session_id": "abc123", "count": 42, "active": true});
        let frame = WsFrame {
            kind: WsFrameKind::Echo,
            payload: payload.clone(),
        };
        let v = serde_json::to_value(&frame).expect("serialize WsFrame");
        assert_eq!(
            v["payload"], payload,
            "WsFrame.payload must be preserved exactly"
        );
    }

    #[test]
    fn ws_frame_deserializes_from_json_object() {
        let raw = r#"{"kind":"echo","payload":{"text":"hello world"}}"#;
        let frame: WsFrame = serde_json::from_str(raw).expect("deserialize WsFrame from JSON");
        assert_eq!(frame.kind, WsFrameKind::Echo);
        assert_eq!(frame.payload["text"], "hello world");
    }

    #[test]
    fn ws_frame_error_kind_round_trip() {
        let frame = WsFrame {
            kind: WsFrameKind::Error,
            payload: json!({"code": "C001", "message": "internal error"}),
        };
        let json = serde_json::to_string(&frame).expect("serialize error frame");
        let decoded: WsFrame = serde_json::from_str(&json).expect("deserialize error frame");
        assert_eq!(frame, decoded);
    }

    #[test]
    fn ws_frame_accepts_null_payload() {
        let frame = WsFrame {
            kind: WsFrameKind::Echo,
            payload: json!(null),
        };
        let json = serde_json::to_string(&frame).expect("serialize null payload frame");
        let decoded: WsFrame = serde_json::from_str(&json).expect("deserialize null payload frame");
        assert_eq!(frame, decoded);
    }

    // ── ErrorPayload ───────────────────────────────────────────────────────

    #[test]
    fn error_payload_round_trip() {
        let ep = ErrorPayload {
            code: "C001".to_owned(),
            message: "connection refused".to_owned(),
        };
        let json = serde_json::to_string(&ep).expect("serialize ErrorPayload");
        let decoded: ErrorPayload = serde_json::from_str(&json).expect("deserialize ErrorPayload");
        assert_eq!(ep, decoded);
    }

    #[test]
    fn error_payload_serializes_expected_fields() {
        let ep = ErrorPayload {
            code: "C014".to_owned(),
            message: "unknown error".to_owned(),
        };
        let v = serde_json::to_value(&ep).expect("serialize ErrorPayload to value");
        assert_eq!(v["code"], "C014");
        assert_eq!(v["message"], "unknown error");
    }

    #[test]
    fn error_payload_rejects_missing_code() {
        let raw = r#"{"message":"oops"}"#;
        let result: Result<ErrorPayload, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "ErrorPayload must fail to deserialize when 'code' is missing"
        );
    }

    #[test]
    fn error_payload_rejects_missing_message() {
        let raw = r#"{"code":"C001"}"#;
        let result: Result<ErrorPayload, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "ErrorPayload must fail to deserialize when 'message' is missing"
        );
    }

    // ── SessionDto ────────────────────────────────────────────────────────

    fn make_session(
        name: &str,
        state: crate::sessions::model::SessionState,
        last_line: &str,
    ) -> crate::sessions::model::Session {
        crate::sessions::model::Session {
            name: name.to_owned(),
            state,
            window_count: 1,
            foreground_cmd: "zsh".to_owned(),
            last_line: last_line.to_owned(),
            agent_state: crate::detect::AgentState::Unknown,
            cwd: String::new(),
        }
    }

    #[test]
    fn session_dto_from_running_session() {
        use crate::sessions::model::SessionState;
        let s = make_session("main", SessionState::Running, "$ cargo test");
        let dto = SessionDto::from(&s);
        assert_eq!(dto.name, "main");
        assert_eq!(dto.state, "running");
        assert_eq!(dto.last_line, "$ cargo test");
    }

    #[test]
    fn session_dto_from_idle_session() {
        use crate::sessions::model::SessionState;
        let s = make_session("scratch", SessionState::Idle, "");
        let dto = SessionDto::from(&s);
        assert_eq!(dto.name, "scratch");
        assert_eq!(dto.state, "idle");
        assert_eq!(dto.last_line, "");
    }

    #[test]
    fn session_dto_serializes_expected_fields() {
        use crate::sessions::model::SessionState;
        let s = make_session("work", SessionState::Running, "hello");
        let dto = SessionDto::from(&s);
        let v = serde_json::to_value(&dto).expect("serialize SessionDto");
        assert_eq!(v["name"], "work");
        assert_eq!(v["state"], "running");
        assert_eq!(v["last_line"], "hello");
    }

    #[test]
    fn session_dto_round_trip() {
        use crate::sessions::model::SessionState;
        let s = make_session("loop", SessionState::Idle, "done");
        let dto = SessionDto::from(&s);
        let json = serde_json::to_string(&dto).expect("serialize");
        let decoded: SessionDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(dto, decoded);
    }

    #[test]
    fn session_dto_rejects_missing_name() {
        let raw = r#"{"state":"idle","last_line":""}"#;
        let result: Result<SessionDto, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "SessionDto must fail when 'name' is missing"
        );
    }

    #[test]
    fn session_dto_rejects_missing_state() {
        let raw = r#"{"name":"s","last_line":""}"#;
        let result: Result<SessionDto, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "SessionDto must fail when 'state' is missing"
        );
    }

    #[test]
    fn session_dto_rejects_missing_last_line() {
        let raw = r#"{"name":"s","state":"idle"}"#;
        let result: Result<SessionDto, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "SessionDto must fail when 'last_line' is missing"
        );
    }

    // ── PaneDto ───────────────────────────────────────────────────────────

    #[test]
    fn pane_dto_from_pane_with_n_lines() {
        let pane = Pane::new("work", "line1\nline2\nline3\n\n");
        let dto = PaneDto::from_pane(&pane, Some(2));
        assert_eq!(dto.session_name, "work");
        assert_eq!(dto.lines, vec!["line2", "line3"]);
    }

    #[test]
    fn pane_dto_from_pane_none_returns_all() {
        let pane = Pane::new("work", "a\nb\nc\n\n");
        let dto = PaneDto::from_pane(&pane, None);
        assert_eq!(dto.lines, vec!["a", "b", "c"]);
    }

    #[test]
    fn pane_dto_serializes_expected_fields() {
        let pane = Pane::new("main", "out1\nout2\n");
        let dto = PaneDto::from_pane(&pane, None);
        let v = serde_json::to_value(&dto).expect("serialize PaneDto");
        assert_eq!(v["session_name"], "main");
        assert_eq!(v["lines"][0], "out1");
        assert_eq!(v["lines"][1], "out2");
    }

    #[test]
    fn pane_dto_round_trip() {
        let pane = Pane::new("s", "x\ny\n");
        let dto = PaneDto::from_pane(&pane, None);
        let json = serde_json::to_string(&dto).expect("serialize");
        let decoded: PaneDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(dto, decoded);
    }

    #[test]
    fn pane_dto_rejects_missing_session_name() {
        let raw = r#"{"lines":["x"]}"#;
        let result: Result<PaneDto, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "PaneDto must fail when 'session_name' is missing"
        );
    }

    #[test]
    fn pane_dto_rejects_missing_lines() {
        let raw = r#"{"session_name":"s"}"#;
        let result: Result<PaneDto, _> = serde_json::from_str(raw);
        assert!(result.is_err(), "PaneDto must fail when 'lines' is missing");
    }

    // ── SendBody ──────────────────────────────────────────────────────────

    #[test]
    fn send_body_serializes_keys_field() {
        let b = SendBody {
            keys: "cargo test".to_owned(),
        };
        let v = serde_json::to_value(&b).expect("serialize SendBody");
        assert_eq!(v["keys"], "cargo test");
    }

    #[test]
    fn send_body_round_trip() {
        let b = SendBody {
            keys: "hello world".to_owned(),
        };
        let json = serde_json::to_string(&b).expect("serialize");
        let decoded: SendBody = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(b, decoded);
    }

    #[test]
    fn send_body_rejects_missing_keys() {
        let raw = r#"{}"#;
        let result: Result<SendBody, _> = serde_json::from_str(raw);
        assert!(result.is_err(), "SendBody must fail when 'keys' is missing");
    }

    // ── KeyBody ───────────────────────────────────────────────────────────

    #[test]
    fn key_body_serializes_key_field() {
        let b = KeyBody {
            key: "Escape".to_owned(),
        };
        let v = serde_json::to_value(&b).expect("serialize KeyBody");
        assert_eq!(v["key"], "Escape");
    }

    #[test]
    fn key_body_round_trip() {
        let b = KeyBody {
            key: "C-c".to_owned(),
        };
        let json = serde_json::to_string(&b).expect("serialize");
        let decoded: KeyBody = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(b, decoded);
    }

    #[test]
    fn key_body_rejects_missing_key() {
        let raw = r#"{}"#;
        let result: Result<KeyBody, _> = serde_json::from_str(raw);
        assert!(result.is_err(), "KeyBody must fail when 'key' is missing");
    }

    // ── NewSessionBody ────────────────────────────────────────────────────

    #[test]
    fn new_session_body_with_dir_serializes() {
        let b = NewSessionBody {
            name: "work".to_owned(),
            dir: Some("/home/user".to_owned()),
        };
        let v = serde_json::to_value(&b).expect("serialize NewSessionBody");
        assert_eq!(v["name"], "work");
        assert_eq!(v["dir"], "/home/user");
    }

    #[test]
    fn new_session_body_without_dir_omits_field() {
        let b = NewSessionBody {
            name: "scratch".to_owned(),
            dir: None,
        };
        let v = serde_json::to_value(&b).expect("serialize NewSessionBody");
        assert_eq!(v["name"], "scratch");
        // dir field must be absent when None (skip_serializing_if = "Option::is_none")
        assert!(v.get("dir").is_none(), "dir must be omitted when None");
    }

    #[test]
    fn new_session_body_round_trip_with_dir() {
        let b = NewSessionBody {
            name: "work".to_owned(),
            dir: Some("/tmp".to_owned()),
        };
        let json = serde_json::to_string(&b).expect("serialize");
        let decoded: NewSessionBody = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(b, decoded);
    }

    #[test]
    fn new_session_body_round_trip_without_dir() {
        let b = NewSessionBody {
            name: "empty".to_owned(),
            dir: None,
        };
        let json = serde_json::to_string(&b).expect("serialize");
        let decoded: NewSessionBody = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(b, decoded);
    }

    #[test]
    fn new_session_body_rejects_missing_name() {
        let raw = r#"{"dir":"/tmp"}"#;
        let result: Result<NewSessionBody, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "NewSessionBody must fail when 'name' is missing"
        );
    }

    #[test]
    fn new_session_body_accepts_missing_dir_as_none() {
        // dir is optional — missing in JSON means None
        let raw = r#"{"name":"test"}"#;
        let b: NewSessionBody = serde_json::from_str(raw).expect("deserialize");
        assert_eq!(b.name, "test");
        assert!(b.dir.is_none());
    }

    // ── v0.2 WsFrameKind variants ──────────────────────────────────────────

    #[test]
    fn ws_frame_kind_subscribe_serializes_snake_case() {
        let kind = WsFrameKind::Subscribe;
        let v = serde_json::to_value(&kind).expect("serialize WsFrameKind::Subscribe");
        assert_eq!(v, json!("subscribe"));
    }

    #[test]
    fn ws_frame_kind_unsubscribe_serializes_snake_case() {
        let kind = WsFrameKind::Unsubscribe;
        let v = serde_json::to_value(&kind).expect("serialize WsFrameKind::Unsubscribe");
        assert_eq!(v, json!("unsubscribe"));
    }

    #[test]
    fn ws_frame_kind_send_serializes_snake_case() {
        let kind = WsFrameKind::Send;
        let v = serde_json::to_value(&kind).expect("serialize WsFrameKind::Send");
        assert_eq!(v, json!("send"));
    }

    #[test]
    fn ws_frame_kind_send_key_serializes_snake_case() {
        let kind = WsFrameKind::SendKey;
        let v = serde_json::to_value(&kind).expect("serialize WsFrameKind::SendKey");
        assert_eq!(v, json!("send_key"));
    }

    #[test]
    fn ws_frame_kind_sessions_serializes_snake_case() {
        let kind = WsFrameKind::Sessions;
        let v = serde_json::to_value(&kind).expect("serialize WsFrameKind::Sessions");
        assert_eq!(v, json!("sessions"));
    }

    #[test]
    fn ws_frame_kind_pane_serializes_snake_case() {
        let kind = WsFrameKind::Pane;
        let v = serde_json::to_value(&kind).expect("serialize WsFrameKind::Pane");
        assert_eq!(v, json!("pane"));
    }

    #[test]
    fn ws_frame_kind_event_serializes_snake_case() {
        let kind = WsFrameKind::Event;
        let v = serde_json::to_value(&kind).expect("serialize WsFrameKind::Event");
        assert_eq!(v, json!("event"));
    }

    #[test]
    fn ws_frame_kind_v02_round_trips() {
        for kind in [
            WsFrameKind::Subscribe,
            WsFrameKind::Unsubscribe,
            WsFrameKind::Send,
            WsFrameKind::SendKey,
            WsFrameKind::Sessions,
            WsFrameKind::Pane,
            WsFrameKind::Event,
        ] {
            let json = serde_json::to_string(&kind).expect("serialize");
            let decoded: WsFrameKind = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(kind, decoded, "round-trip failed for {json}");
        }
    }

    // ── SubscribePayload ──────────────────────────────────────────────────

    #[test]
    fn subscribe_payload_round_trip() {
        let p = SubscribePayload {
            topic: "sessions".to_owned(),
        };
        let json = serde_json::to_string(&p).expect("serialize");
        let decoded: SubscribePayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(p, decoded);
    }

    #[test]
    fn subscribe_payload_serializes_topic_field() {
        let p = SubscribePayload {
            topic: "pane:work".to_owned(),
        };
        let v = serde_json::to_value(&p).expect("serialize");
        assert_eq!(v["topic"], "pane:work");
    }

    #[test]
    fn subscribe_payload_rejects_missing_topic() {
        let raw = r#"{}"#;
        let result: Result<SubscribePayload, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "SubscribePayload must fail when topic is missing"
        );
    }

    // ── SendPayload ───────────────────────────────────────────────────────

    #[test]
    fn send_payload_round_trip() {
        let p = SendPayload {
            session: "main".to_owned(),
            keys: "cargo test".to_owned(),
        };
        let json = serde_json::to_string(&p).expect("serialize");
        let decoded: SendPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(p, decoded);
    }

    #[test]
    fn send_payload_serializes_expected_fields() {
        let p = SendPayload {
            session: "work".to_owned(),
            keys: "ls -la".to_owned(),
        };
        let v = serde_json::to_value(&p).expect("serialize");
        assert_eq!(v["session"], "work");
        assert_eq!(v["keys"], "ls -la");
    }

    #[test]
    fn send_payload_rejects_missing_session() {
        let raw = r#"{"keys":"hello"}"#;
        let result: Result<SendPayload, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "SendPayload must fail when session is missing"
        );
    }

    #[test]
    fn send_payload_rejects_missing_keys() {
        let raw = r#"{"session":"main"}"#;
        let result: Result<SendPayload, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "SendPayload must fail when keys is missing"
        );
    }

    // ── SendKeyPayload ────────────────────────────────────────────────────

    #[test]
    fn send_key_payload_round_trip() {
        let p = SendKeyPayload {
            session: "main".to_owned(),
            key: "Escape".to_owned(),
        };
        let json = serde_json::to_string(&p).expect("serialize");
        let decoded: SendKeyPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(p, decoded);
    }

    #[test]
    fn send_key_payload_serializes_expected_fields() {
        let p = SendKeyPayload {
            session: "work".to_owned(),
            key: "C-c".to_owned(),
        };
        let v = serde_json::to_value(&p).expect("serialize");
        assert_eq!(v["session"], "work");
        assert_eq!(v["key"], "C-c");
    }

    #[test]
    fn send_key_payload_rejects_missing_session() {
        let raw = r#"{"key":"Escape"}"#;
        let result: Result<SendKeyPayload, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "SendKeyPayload must fail when session is missing"
        );
    }

    #[test]
    fn send_key_payload_rejects_missing_key() {
        let raw = r#"{"session":"main"}"#;
        let result: Result<SendKeyPayload, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "SendKeyPayload must fail when key is missing"
        );
    }

    // ── SessionsPayload ───────────────────────────────────────────────────

    #[test]
    fn sessions_payload_round_trip() {
        let p = SessionsPayload {
            sessions: vec![SessionDto {
                name: "main".to_owned(),
                state: "running".to_owned(),
                last_line: "$ cargo test".to_owned(),
            }],
        };
        let json = serde_json::to_string(&p).expect("serialize");
        let decoded: SessionsPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(p, decoded);
    }

    #[test]
    fn sessions_payload_serializes_sessions_field() {
        let p = SessionsPayload { sessions: vec![] };
        let v = serde_json::to_value(&p).expect("serialize");
        assert!(v["sessions"].is_array(), "sessions must be an array");
        assert_eq!(v["sessions"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn sessions_payload_rejects_missing_sessions() {
        let raw = r#"{}"#;
        let result: Result<SessionsPayload, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "SessionsPayload must fail when sessions is missing"
        );
    }

    // ── PanePayload ───────────────────────────────────────────────────────

    #[test]
    fn pane_payload_round_trip() {
        let p = PanePayload {
            session: "main".to_owned(),
            seq: 7,
            lines: vec!["line1".to_owned(), "line2".to_owned()],
        };
        let json = serde_json::to_string(&p).expect("serialize");
        let decoded: PanePayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(p, decoded);
    }

    #[test]
    fn pane_payload_serializes_expected_fields() {
        let p = PanePayload {
            session: "work".to_owned(),
            seq: 42,
            lines: vec!["hello".to_owned()],
        };
        let v = serde_json::to_value(&p).expect("serialize");
        assert_eq!(v["session"], "work");
        assert_eq!(v["seq"], 42);
        assert_eq!(v["lines"][0], "hello");
    }

    #[test]
    fn pane_payload_rejects_missing_session() {
        let raw = r#"{"seq":1,"lines":[]}"#;
        let result: Result<PanePayload, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "PanePayload must fail when session is missing"
        );
    }

    #[test]
    fn pane_payload_rejects_missing_seq() {
        let raw = r#"{"session":"main","lines":[]}"#;
        let result: Result<PanePayload, _> = serde_json::from_str(raw);
        assert!(result.is_err(), "PanePayload must fail when seq is missing");
    }

    #[test]
    fn pane_payload_rejects_missing_lines() {
        let raw = r#"{"session":"main","seq":1}"#;
        let result: Result<PanePayload, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "PanePayload must fail when lines is missing"
        );
    }

    // ── EventPayload ──────────────────────────────────────────────────────

    #[test]
    fn event_payload_round_trip() {
        let p = EventPayload {
            session: "main".to_owned(),
            event: "needs_input".to_owned(),
        };
        let json = serde_json::to_string(&p).expect("serialize");
        let decoded: EventPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(p, decoded);
    }

    #[test]
    fn event_payload_serializes_expected_fields() {
        let p = EventPayload {
            session: "work".to_owned(),
            event: "needs_input".to_owned(),
        };
        let v = serde_json::to_value(&p).expect("serialize");
        assert_eq!(v["session"], "work");
        assert_eq!(v["event"], "needs_input");
    }

    #[test]
    fn event_payload_rejects_missing_session() {
        let raw = r#"{"event":"needs_input"}"#;
        let result: Result<EventPayload, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "EventPayload must fail when session is missing"
        );
    }

    #[test]
    fn event_payload_rejects_missing_event() {
        let raw = r#"{"session":"main"}"#;
        let result: Result<EventPayload, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "EventPayload must fail when event is missing"
        );
    }

    // ── parse_topic ───────────────────────────────────────────────────────

    #[test]
    fn parse_topic_sessions() {
        assert_eq!(parse_topic("sessions"), Some(Topic::Sessions));
    }

    #[test]
    fn parse_topic_pane_with_name() {
        assert_eq!(
            parse_topic("pane:work"),
            Some(Topic::Pane("work".to_owned()))
        );
    }

    #[test]
    fn parse_topic_pane_empty_name_is_none() {
        assert_eq!(
            parse_topic("pane:"),
            None,
            "empty pane name must be rejected"
        );
    }

    #[test]
    fn parse_topic_unknown_is_none() {
        assert_eq!(parse_topic("unknown"), None);
        assert_eq!(parse_topic(""), None);
        assert_eq!(parse_topic("SESSIONS"), None);
        assert_eq!(parse_topic("Pane:work"), None);
    }

    #[test]
    fn parse_topic_runs() {
        assert_eq!(parse_topic("runs"), Some(Topic::Runs));
    }

    #[test]
    fn parse_topic_runs_near_misses_are_none() {
        assert_eq!(parse_topic("run"), None);
        assert_eq!(parse_topic("runs:"), None);
        assert_eq!(parse_topic("RUNS"), None);
        assert_eq!(parse_topic("runs "), None);
    }

    #[test]
    fn parse_topic_pane_name_with_hyphens_and_underscores() {
        // names like "claude-work" or "my_session" are valid
        assert_eq!(
            parse_topic("pane:claude-work"),
            Some(Topic::Pane("claude-work".to_owned()))
        );
        assert_eq!(
            parse_topic("pane:my_session"),
            Some(Topic::Pane("my_session".to_owned()))
        );
    }

    // ── RepoSummaryDto ────────────────────────────────────────────────────

    #[test]
    fn repo_summary_dto_round_trips() {
        let dto = RepoSummaryDto {
            name: "bastion".to_owned(),
            now: "BA.11.D in progress".to_owned(),
            has_handoff: true,
        };
        let json = serde_json::to_string(&dto).expect("serialize");
        let back: RepoSummaryDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(dto, back);
    }

    #[test]
    fn repo_summary_dto_rejects_missing_fields() {
        let raw = r#"{"name":"bastion","now":"x"}"#;
        let result: Result<RepoSummaryDto, _> = serde_json::from_str(raw);
        assert!(result.is_err(), "missing has_handoff must fail to parse");
    }

    // ── RepoStatusDto ─────────────────────────────────────────────────────

    fn sample_repo_status() -> crate::serve::status::repo::RepoStatus {
        crate::serve::status::repo::parse_status(
            "---\nnow: \"focus\"\nnext: \"next thing\"\nblocked: \"[]\"\n---\n\n## Momentum\n- **now** — focus\n- **next** — next thing\n- **blocked** — nothing\n- **improve** — tighten\n- **recurring** — none\n",
        )
        .expect("fixture status content must parse")
    }

    #[test]
    fn repo_status_dto_from_repo_status() {
        let status = sample_repo_status();
        let dto: RepoStatusDto = status.clone().into();
        assert_eq!(dto.now, status.now);
        assert_eq!(dto.next, status.next);
        assert_eq!(dto.blocked, status.blocked);
        assert_eq!(dto.momentum_now, status.momentum_now);
    }

    #[test]
    fn repo_status_dto_round_trips() {
        let status = sample_repo_status();
        let dto: RepoStatusDto = status.into();
        let json = serde_json::to_string(&dto).expect("serialize");
        let back: RepoStatusDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(dto, back);
    }

    // ── WorkflowStateDto ──────────────────────────────────────────────────

    fn sample_flow_state() -> crate::serve::status::flow::FlowState {
        crate::serve::status::flow::parse_flow_state(
            r#"{"spec_slug":"phase11-blockD","branch":"phase11-blockD-flow","status":"running","current_task":3,"started_at":"2026-06-30T00:00:00Z","updated_at":"2026-06-30T01:00:00Z"}"#,
        )
        .expect("fixture flow state must parse")
    }

    #[test]
    fn workflow_state_dto_from_flow_state() {
        let flow = sample_flow_state();
        let dto: WorkflowStateDto = flow.clone().into();
        assert_eq!(dto.spec_slug, flow.spec_slug);
        assert_eq!(dto.branch, flow.branch);
        assert_eq!(dto.status, flow.status);
        assert_eq!(dto.current_task, flow.current_task);
    }

    #[test]
    fn workflow_state_dto_round_trips() {
        let flow = sample_flow_state();
        let dto: WorkflowStateDto = flow.into();
        let json = serde_json::to_string(&dto).expect("serialize");
        let back: WorkflowStateDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(dto, back);
    }

    #[test]
    fn workflow_state_dto_rejects_missing_fields() {
        let raw = r#"{"spec_slug":"x","branch":"y","status":"running"}"#;
        let result: Result<WorkflowStateDto, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "missing current_task/started_at/updated_at must fail to parse"
        );
    }

    // ── WorkflowDonePayload ───────────────────────────────────────────────

    #[test]
    fn workflow_done_payload_round_trips() {
        let payload = WorkflowDonePayload {
            repo: "bastion".to_owned(),
            spec_slug: "phase11-blockD".to_owned(),
            status: "done".to_owned(),
        };
        let json = serde_json::to_string(&payload).expect("serialize");
        let back: WorkflowDonePayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(payload, back);
    }

    #[test]
    fn workflow_done_payload_serializes_expected_shape() {
        let payload = WorkflowDonePayload {
            repo: "bastion".to_owned(),
            spec_slug: "phase11-blockD".to_owned(),
            status: "blocked".to_owned(),
        };
        let v = serde_json::to_value(&payload).expect("serialize");
        assert_eq!(v["repo"], "bastion");
        assert_eq!(v["spec_slug"], "phase11-blockD");
        assert_eq!(v["status"], "blocked");
    }

    #[test]
    fn workflow_done_payload_rejects_missing_fields() {
        let raw = r#"{"repo":"bastion","spec_slug":"phase11-blockD"}"#;
        let result: Result<WorkflowDonePayload, _> = serde_json::from_str(raw);
        assert!(result.is_err(), "missing status must fail to parse");
    }

    // ── RunTransitionPayload ─────────────────────────────────────────────

    #[test]
    fn run_transition_payload_round_trips_with_spec_slug() {
        let payload = RunTransitionPayload {
            run_id: "9a2b1c3d-0000-0000-0000-000000000001".to_owned(),
            status: "running".to_owned(),
            terminal: false,
            spec_slug: Some("phase11-blockN".to_owned()),
        };
        let json = serde_json::to_string(&payload).expect("serialize");
        let back: RunTransitionPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(payload, back);
    }

    #[test]
    fn run_transition_payload_round_trips_without_spec_slug() {
        let payload = RunTransitionPayload {
            run_id: "9a2b1c3d-0000-0000-0000-000000000002".to_owned(),
            status: "done".to_owned(),
            terminal: true,
            spec_slug: None,
        };
        let json = serde_json::to_string(&payload).expect("serialize");
        let back: RunTransitionPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(payload, back);
    }

    #[test]
    fn run_transition_payload_omits_absent_spec_slug_key() {
        let payload = RunTransitionPayload {
            run_id: "9a2b1c3d-0000-0000-0000-000000000003".to_owned(),
            status: "suspended".to_owned(),
            terminal: false,
            spec_slug: None,
        };
        let v = serde_json::to_value(&payload).expect("serialize");
        assert!(
            v.get("spec_slug").is_none(),
            "spec_slug must be an absent key, not null, when None"
        );
        assert_eq!(v["run_id"], "9a2b1c3d-0000-0000-0000-000000000003");
        assert_eq!(v["status"], "suspended");
        assert_eq!(v["terminal"], false);
    }

    #[test]
    fn run_transition_payload_includes_present_spec_slug_key() {
        let payload = RunTransitionPayload {
            run_id: "9a2b1c3d-0000-0000-0000-000000000004".to_owned(),
            status: "running".to_owned(),
            terminal: false,
            spec_slug: Some("phase11-blockN".to_owned()),
        };
        let v = serde_json::to_value(&payload).expect("serialize");
        assert_eq!(v["spec_slug"], "phase11-blockN");
    }

    #[test]
    fn run_transition_payload_rejects_missing_required_fields() {
        let raw = r#"{"run_id":"x","status":"running"}"#;
        let result: Result<RunTransitionPayload, _> = serde_json::from_str(raw);
        assert!(result.is_err(), "missing terminal must fail to parse");
    }

    // ── RunStreamStatusPayload ───────────────────────────────────────────

    #[test]
    fn run_stream_status_payload_round_trips_available() {
        let payload = RunStreamStatusPayload {
            available: true,
            reason: None,
        };
        let json = serde_json::to_string(&payload).expect("serialize");
        let back: RunStreamStatusPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(payload, back);
    }

    #[test]
    fn run_stream_status_payload_round_trips_unavailable_with_reason() {
        let payload = RunStreamStatusPayload {
            available: false,
            reason: Some("DATABASE_URL not set".to_owned()),
        };
        let json = serde_json::to_string(&payload).expect("serialize");
        let back: RunStreamStatusPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(payload, back);
    }

    #[test]
    fn run_stream_status_payload_omits_absent_reason_key() {
        let payload = RunStreamStatusPayload {
            available: true,
            reason: None,
        };
        let v = serde_json::to_value(&payload).expect("serialize");
        assert!(
            v.get("reason").is_none(),
            "reason must be an absent key, not null, when None"
        );
        assert_eq!(v["available"], true);
    }

    #[test]
    fn run_stream_status_payload_includes_present_reason_key() {
        let payload = RunStreamStatusPayload {
            available: false,
            reason: Some("engine not mounted".to_owned()),
        };
        let v = serde_json::to_value(&payload).expect("serialize");
        assert_eq!(v["reason"], "engine not mounted");
    }

    #[test]
    fn run_stream_status_payload_rejects_missing_available() {
        let raw = r#"{"reason":"x"}"#;
        let result: Result<RunStreamStatusPayload, _> = serde_json::from_str(raw);
        assert!(result.is_err(), "missing available must fail to parse");
    }

    // ── CommandMode ───────────────────────────────────────────────────────

    #[test]
    fn command_mode_inject_serializes_snake_case() {
        let v = serde_json::to_value(CommandMode::Inject).expect("serialize");
        assert_eq!(v, json!("inject"));
    }

    #[test]
    fn command_mode_spawn_serializes_snake_case() {
        let v = serde_json::to_value(CommandMode::Spawn).expect("serialize");
        assert_eq!(v, json!("spawn"));
    }

    #[test]
    fn command_mode_round_trips() {
        for mode in [CommandMode::Inject, CommandMode::Spawn] {
            let json = serde_json::to_string(&mode).expect("serialize");
            let decoded: CommandMode = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(mode, decoded, "round-trip failed for {json}");
        }
    }

    #[test]
    fn command_mode_unknown_variant_fails() {
        let result: Result<CommandMode, _> = serde_json::from_str(r#""restart""#);
        assert!(
            result.is_err(),
            "unrecognised mode string must fail to deserialize"
        );
    }

    // ── CommandRequest deserialization ───────────────────────────────────

    #[test]
    fn command_request_deserializes_valid_inject_payload() {
        let raw = r#"{"mode":"inject","session":"main","command":"/status"}"#;
        let req: CommandRequest = serde_json::from_str(raw).expect("deserialize inject payload");
        assert_eq!(req.mode, CommandMode::Inject);
        assert_eq!(req.session.as_deref(), Some("main"));
        assert_eq!(req.command, "/status");
        assert!(req.name.is_none());
        assert!(req.dir.is_none());
        assert!(req.model.is_none());
    }

    #[test]
    fn command_request_deserializes_valid_spawn_payload() {
        let raw =
            r#"{"mode":"spawn","name":"work","dir":"/repo","model":"sonnet","command":"/status"}"#;
        let req: CommandRequest = serde_json::from_str(raw).expect("deserialize spawn payload");
        assert_eq!(req.mode, CommandMode::Spawn);
        assert_eq!(req.name.as_deref(), Some("work"));
        assert_eq!(req.dir.as_deref(), Some("/repo"));
        assert_eq!(req.model.as_deref(), Some("sonnet"));
        assert_eq!(req.command, "/status");
    }

    #[test]
    fn command_request_deserializes_spawn_payload_without_optional_fields() {
        let raw = r#"{"mode":"spawn","name":"work","command":"/status"}"#;
        let req: CommandRequest = serde_json::from_str(raw).expect("deserialize");
        assert!(req.dir.is_none());
        assert!(req.model.is_none());
    }

    #[test]
    fn command_request_rejects_unknown_mode() {
        let raw = r#"{"mode":"restart","session":"main","command":"/status"}"#;
        let result: Result<CommandRequest, _> = serde_json::from_str(raw);
        assert!(result.is_err(), "unknown mode must fail to deserialize");
    }

    #[test]
    fn command_request_rejects_missing_mode() {
        let raw = r#"{"session":"main","command":"/status"}"#;
        let result: Result<CommandRequest, _> = serde_json::from_str(raw);
        assert!(result.is_err(), "missing mode must fail to deserialize");
    }

    #[test]
    fn command_request_rejects_missing_command() {
        let raw = r#"{"mode":"inject","session":"main"}"#;
        let result: Result<CommandRequest, _> = serde_json::from_str(raw);
        assert!(result.is_err(), "missing command must fail to deserialize");
    }

    #[test]
    fn command_request_round_trips() {
        let req = CommandRequest {
            mode: CommandMode::Spawn,
            session: None,
            name: Some("work".to_owned()),
            dir: Some("/repo".to_owned()),
            model: Some("opus".to_owned()),
            command: "/status".to_owned(),
        };
        let json = serde_json::to_string(&req).expect("serialize");
        let decoded: CommandRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(req, decoded);
    }

    #[test]
    fn command_request_serializes_omits_absent_optional_fields() {
        let req = CommandRequest {
            mode: CommandMode::Inject,
            session: Some("main".to_owned()),
            name: None,
            dir: None,
            model: None,
            command: "/status".to_owned(),
        };
        let v = serde_json::to_value(&req).expect("serialize");
        assert!(v.get("name").is_none(), "name must be omitted when None");
        assert!(v.get("dir").is_none(), "dir must be omitted when None");
        assert!(v.get("model").is_none(), "model must be omitted when None");
    }

    // ── CommandRequest::validate ─────────────────────────────────────────

    fn inject_request(session: Option<&str>) -> CommandRequest {
        CommandRequest {
            mode: CommandMode::Inject,
            session: session.map(str::to_owned),
            name: None,
            dir: None,
            model: None,
            command: "/status".to_owned(),
        }
    }

    fn spawn_request(name: Option<&str>, model: Option<&str>) -> CommandRequest {
        CommandRequest {
            mode: CommandMode::Spawn,
            session: None,
            name: name.map(str::to_owned),
            dir: None,
            model: model.map(str::to_owned),
            command: "/status".to_owned(),
        }
    }

    #[test]
    fn validate_accepts_inject_with_session() {
        assert_eq!(inject_request(Some("main")).validate(), Ok(()));
    }

    #[test]
    fn validate_rejects_inject_without_session() {
        assert_eq!(
            inject_request(None).validate(),
            Err(CommandValidationError::InjectMissingSession)
        );
    }

    #[test]
    fn validate_rejects_inject_with_empty_session() {
        assert_eq!(
            inject_request(Some("")).validate(),
            Err(CommandValidationError::InjectMissingSession)
        );
    }

    #[test]
    fn validate_accepts_spawn_with_name() {
        assert_eq!(spawn_request(Some("work"), None).validate(), Ok(()));
    }

    #[test]
    fn validate_rejects_spawn_without_name() {
        assert_eq!(
            spawn_request(None, None).validate(),
            Err(CommandValidationError::SpawnMissingName)
        );
    }

    #[test]
    fn validate_rejects_spawn_with_empty_name() {
        assert_eq!(
            spawn_request(Some(""), None).validate(),
            Err(CommandValidationError::SpawnMissingName)
        );
    }

    #[test]
    fn validate_accepts_spawn_with_opus_model() {
        assert_eq!(spawn_request(Some("work"), Some("opus")).validate(), Ok(()));
    }

    #[test]
    fn validate_accepts_spawn_with_sonnet_model() {
        assert_eq!(
            spawn_request(Some("work"), Some("sonnet")).validate(),
            Ok(())
        );
    }

    #[test]
    fn validate_rejects_spawn_with_unknown_model() {
        assert_eq!(
            spawn_request(Some("work"), Some("haiku")).validate(),
            Err(CommandValidationError::UnknownModel("haiku".to_owned()))
        );
    }

    #[test]
    fn validate_rejects_inject_with_unknown_model_too() {
        // Model validation applies regardless of mode.
        let mut req = inject_request(Some("main"));
        req.model = Some("gpt-5".to_owned());
        assert_eq!(
            req.validate(),
            Err(CommandValidationError::UnknownModel("gpt-5".to_owned()))
        );
    }

    #[test]
    fn command_validation_error_display_messages() {
        assert_eq!(
            CommandValidationError::InjectMissingSession.to_string(),
            "mode:\"inject\" requires a non-empty \"session\" field"
        );
        assert_eq!(
            CommandValidationError::SpawnMissingName.to_string(),
            "mode:\"spawn\" requires a non-empty \"name\" field"
        );
        assert!(
            CommandValidationError::UnknownModel("haiku".to_owned())
                .to_string()
                .contains("haiku")
        );
    }

    // ── CommandResponse ───────────────────────────────────────────────────

    #[test]
    fn command_response_serializes_session_field() {
        let resp = CommandResponse {
            session: "work".to_owned(),
        };
        let v = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(v["session"], "work");
    }

    #[test]
    fn command_response_round_trips() {
        let resp = CommandResponse {
            session: "main".to_owned(),
        };
        let json = serde_json::to_string(&resp).expect("serialize");
        let decoded: CommandResponse = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(resp, decoded);
    }

    #[test]
    fn command_response_rejects_missing_session() {
        let raw = r#"{}"#;
        let result: Result<CommandResponse, _> = serde_json::from_str(raw);
        assert!(
            result.is_err(),
            "CommandResponse must fail when session is missing"
        );
    }

    // ── BoardScope ────────────────────────────────────────────────────────

    #[test]
    fn board_scope_deserializes_from_each_lowercase_string() {
        assert_eq!(
            serde_json::from_str::<BoardScope>(r#""hq""#).expect("hq"),
            BoardScope::Hq
        );
        assert_eq!(
            serde_json::from_str::<BoardScope>(r#""tier""#).expect("tier"),
            BoardScope::Tier
        );
        assert_eq!(
            serde_json::from_str::<BoardScope>(r#""project""#).expect("project"),
            BoardScope::Project
        );
        assert_eq!(
            serde_json::from_str::<BoardScope>(r#""business""#).expect("business"),
            BoardScope::Business
        );
    }

    #[test]
    fn board_scope_missing_defaults_to_hq() {
        assert_eq!(BoardScope::default(), BoardScope::Hq);
    }

    #[test]
    fn board_scope_unknown_value_fails_to_deserialize() {
        let result: Result<BoardScope, _> = serde_json::from_str(r#""bogus""#);
        assert!(result.is_err(), "unknown scope string must fail to parse");
    }

    // ── BoardDto ──────────────────────────────────────────────────────────

    fn sample_board_block() -> BoardBlockDto {
        BoardBlockDto {
            id: "BA.11.K".to_owned(),
            title: "Cross-brain board read endpoint".to_owned(),
            repo: "bastion".to_owned(),
            status: Some("in_progress".to_owned()),
            blocked_by: vec![okf_core::BlockedBy::External {
                what: "reviewer availability".to_owned(),
            }],
            epics: vec!["bastion-surfaces".to_owned()],
            wave: Some(3),
            priority: Some(1),
            due: Some("2026-07-15".to_owned()),
            track: Some("Phase 11".to_owned()),
            last_touched: None,
            dependent_count: None,
            ready: None,
            unmet_count: None,
        }
    }

    fn sample_board_dto() -> BoardDto {
        let lanes = BoardLaneDto {
            now: vec![sample_board_block()],
            next: vec![],
            blocked: vec![],
            deferred: vec![],
            finished: vec![],
        };
        BoardDto {
            scope: BoardScope::Hq,
            tier: None,
            lanes: lanes.clone(),
            repos: vec![RepoBoardDto {
                repo: "bastion".to_owned(),
                tier: Some("core".to_owned()),
                lanes,
            }],
            stale: true,
        }
    }

    #[test]
    fn board_dto_round_trips() {
        let dto = sample_board_dto();
        let json = serde_json::to_string(&dto).expect("serialize");
        let back: BoardDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(dto, back);
    }

    #[test]
    fn board_dto_serializes_expected_fields() {
        let dto = sample_board_dto();
        let v = serde_json::to_value(&dto).expect("serialize");
        assert_eq!(v["scope"], "hq");
        assert_eq!(v["stale"], true);
        assert_eq!(v["lanes"]["now"][0]["id"], "BA.11.K");
        assert_eq!(v["repos"][0]["repo"], "bastion");
        assert_eq!(v["repos"][0]["tier"], "core");
    }

    #[test]
    fn board_dto_scope_defaults_when_absent() {
        let raw = r#"{"lanes":{"now":[],"next":[],"blocked":[],"finished":[]},"stale":false}"#;
        let dto: BoardDto = serde_json::from_str(raw).expect("scope should default to hq");
        assert_eq!(dto.scope, BoardScope::Hq);
        assert!(dto.tier.is_none());
        assert!(dto.repos.is_empty());
    }

    // ── render_clears_when (BA.ticket.carryover-triage-dto task 1) ──────────

    #[test]
    fn render_clears_when_prose() {
        let cw = okf_core::ClearsWhen::Prose("the docs are updated".to_owned());
        assert_eq!(render_clears_when(&cw), "the docs are updated");
    }

    #[test]
    fn render_clears_when_block_closed_without_note() {
        let cw = okf_core::ClearsWhen::Predicate(okf_core::ClearsWhenPredicate::BlockClosed {
            repo: "bastion".to_owned(),
            id: "BA.11.P".to_owned(),
            note: None,
        });
        assert_eq!(render_clears_when(&cw), "block bastion/BA.11.P is closed");
    }

    #[test]
    fn render_clears_when_block_closed_with_note() {
        let cw = okf_core::ClearsWhen::Predicate(okf_core::ClearsWhenPredicate::BlockClosed {
            repo: "bastion".to_owned(),
            id: "BA.11.P".to_owned(),
            note: Some("the DTO ships".to_owned()),
        });
        assert_eq!(
            render_clears_when(&cw),
            "block bastion/BA.11.P is closed (the DTO ships)"
        );
    }

    #[test]
    fn render_clears_when_file_exists_without_note() {
        let cw = okf_core::ClearsWhen::Predicate(okf_core::ClearsWhenPredicate::FileExists {
            path: ".env.example".to_owned(),
            note: None,
        });
        assert_eq!(render_clears_when(&cw), ".env.example exists");
    }

    #[test]
    fn render_clears_when_file_exists_with_note() {
        let cw = okf_core::ClearsWhen::Predicate(okf_core::ClearsWhenPredicate::FileExists {
            path: ".env.example".to_owned(),
            note: Some("documents the engine mount".to_owned()),
        });
        assert_eq!(
            render_clears_when(&cw),
            ".env.example exists (documents the engine mount)"
        );
    }

    #[test]
    fn render_clears_when_file_contains_without_note() {
        let cw = okf_core::ClearsWhen::Predicate(okf_core::ClearsWhenPredicate::FileContains {
            path: ".env.example".to_owned(),
            pattern: "BASTION_ENGINE_API_KEY".to_owned(),
            note: None,
        });
        assert_eq!(
            render_clears_when(&cw),
            ".env.example contains \"BASTION_ENGINE_API_KEY\""
        );
    }

    #[test]
    fn render_clears_when_file_contains_with_note() {
        let cw = okf_core::ClearsWhen::Predicate(okf_core::ClearsWhenPredicate::FileContains {
            path: ".env.example".to_owned(),
            pattern: "BASTION_ENGINE_API_KEY".to_owned(),
            note: Some("the key is documented".to_owned()),
        });
        assert_eq!(
            render_clears_when(&cw),
            ".env.example contains \"BASTION_ENGINE_API_KEY\" (the key is documented)"
        );
    }

    #[test]
    fn render_clears_when_command_exits_zero_without_note() {
        let cw = okf_core::ClearsWhen::Predicate(okf_core::ClearsWhenPredicate::CommandExitsZero {
            command: "cargo test carryover".to_owned(),
            note: None,
        });
        assert_eq!(render_clears_when(&cw), "`cargo test carryover` exits zero");
    }

    #[test]
    fn render_clears_when_command_exits_zero_with_note() {
        let cw = okf_core::ClearsWhen::Predicate(okf_core::ClearsWhenPredicate::CommandExitsZero {
            command: "cargo test carryover".to_owned(),
            note: Some("covers all four predicate variants".to_owned()),
        });
        assert_eq!(
            render_clears_when(&cw),
            "`cargo test carryover` exits zero (covers all four predicate variants)"
        );
    }

    // ── AttentionDto (BA.11.P) ──────────────────────────────────────────────

    fn sample_attention_carryover() -> AttentionCarryoverDto {
        AttentionCarryoverDto {
            repo: "bastion".to_owned(),
            slug: "engine-mount-env".to_owned(),
            kind: "env".to_owned(),
            text: "engine routes need DATABASE_URL + BASTION_ENGINE_API_KEY set".to_owned(),
            clears_when: Some("the engine mount is documented in .env.example".to_owned()),
            created: Some("2026-07-01".to_owned()),
            reviewed: None,
            age_days: Some(23),
            threshold_days: 3,
            lane: TriageLane::Aging,
            priority: None,
            effective_priority: None,
            unmet_blocks: Vec::new(),
            finding_id: None,
            clears_when_satisfied: false,
        }
    }

    fn sample_attention_backlog() -> AttentionBacklogDto {
        AttentionBacklogDto {
            repo: "bastion".to_owned(),
            slug: "serve-attention-endpoint".to_owned(),
            title: "Attention projection".to_owned(),
            kind: "feature".to_owned(),
            status: "ready".to_owned(),
            notes: None,
            created: Some("2026-07-10".to_owned()),
            reviewed: None,
            age_days: 14,
            threshold_days: 7,
        }
    }

    fn sample_attention_dto() -> AttentionDto {
        AttentionDto {
            scope: BoardScope::Hq,
            tier: None,
            as_of: "2026-07-24".to_owned(),
            lanes: AttentionLanesDto {
                stale_carryover: vec![sample_attention_carryover()],
                aging_backlog: vec![sample_attention_backlog()],
                orphaned_captures: vec![],
            },
            thresholds: AttentionThresholdsDto {
                env_days: 3,
                deferred_days: 5,
                known_issue_days: 10,
                constraint_days: 10,
                backlog_days: 7,
            },
        }
    }

    #[test]
    fn attention_dto_round_trips() {
        let dto = sample_attention_dto();
        let json = serde_json::to_string(&dto).expect("serialize");
        let back: AttentionDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(dto, back);
    }

    #[test]
    fn attention_dto_serializes_expected_fields() {
        let dto = sample_attention_dto();
        let v = serde_json::to_value(&dto).expect("serialize");
        assert_eq!(v["scope"], "hq");
        assert_eq!(v["as_of"], "2026-07-24");
        assert_eq!(v["lanes"]["stale_carryover"][0]["slug"], "engine-mount-env");
        assert_eq!(v["lanes"]["stale_carryover"][0]["age_days"], 23);
        assert_eq!(
            v["lanes"]["aging_backlog"][0]["slug"],
            "serve-attention-endpoint"
        );
        assert!(
            v["lanes"]["orphaned_captures"]
                .as_array()
                .expect("orphaned_captures should be an array")
                .is_empty()
        );
        assert_eq!(v["thresholds"]["backlog_days"], 7);
    }

    #[test]
    fn attention_dto_lanes_default_to_empty_when_absent() {
        let raw = r#"{}"#;
        let lanes: AttentionLanesDto = serde_json::from_str(raw).expect("lanes should default");
        assert!(lanes.stale_carryover.is_empty());
        assert!(lanes.aging_backlog.is_empty());
        assert!(lanes.orphaned_captures.is_empty());
    }

    #[test]
    fn attention_backlog_dto_used_by_both_aging_and_orphaned_lanes() {
        // Same DTO type backs both lanes — assert a capture-flavoured instance (notes carrying
        // the origin.notes fallback) round-trips identically to a plain backlog instance.
        let capture = AttentionBacklogDto {
            notes: Some("captured from a stray idea".to_owned()),
            ..sample_attention_backlog()
        };
        let json = serde_json::to_string(&capture).expect("serialize");
        let back: AttentionBacklogDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(capture, back);
    }

    #[test]
    fn attention_carryover_dto_round_trips_with_ranking_fields_populated() {
        let dto = AttentionCarryoverDto {
            lane: TriageLane::Blocking,
            priority: Some(0),
            effective_priority: Some(0),
            unmet_blocks: vec!["bastion:BA.1.A".to_owned()],
            finding_id: Some("finding-x".to_owned()),
            clears_when_satisfied: true,
            ..sample_attention_carryover()
        };
        let json = serde_json::to_string(&dto).expect("serialize");
        let back: AttentionCarryoverDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(dto, back);
    }

    #[test]
    fn attention_carryover_dto_absent_optionals_are_absent_keys_not_null() {
        // Testing Strategy item 6 — `priority`, `effective_priority`, `finding_id`,
        // and `age_days` (plus the pre-existing `clears_when`/`reviewed`/
        // `unmet_blocks`) must serialize as absent keys when `None`/empty, never
        // as `null`/`[]`, matching this repo's established absent-is-never-neutral
        // convention.
        let dto = AttentionCarryoverDto {
            clears_when: None,
            reviewed: None,
            age_days: None,
            priority: None,
            effective_priority: None,
            unmet_blocks: Vec::new(),
            finding_id: None,
            ..sample_attention_carryover()
        };
        let v = serde_json::to_value(&dto).expect("serialize");
        let obj = v.as_object().expect("object");
        for key in [
            "clears_when",
            "reviewed",
            "age_days",
            "priority",
            "effective_priority",
            "unmet_blocks",
            "finding_id",
        ] {
            assert!(
                !obj.contains_key(key),
                "expected '{key}' to be an absent key, not present (possibly as null)"
            );
        }
        // Sanity check the negative assertions above aren't vacuous — a field left
        // populated by the base fixture is still present.
        assert!(obj.contains_key("created"));
    }

    #[test]
    fn attention_carryover_dto_never_serializes_a_blocking_field() {
        // Contract §6 rule 2 — `blocking` is always derived from `unmet_blocks`,
        // never authored as its own field, whether or not the entry is BLOCKING.
        let blocking = AttentionCarryoverDto {
            lane: TriageLane::Blocking,
            unmet_blocks: vec!["bastion:BA.1.A".to_owned()],
            ..sample_attention_carryover()
        };
        let v = serde_json::to_value(&blocking).expect("serialize");
        assert!(!v.as_object().expect("object").contains_key("blocking"));
    }

    #[test]
    fn attention_carryover_dto_lane_serializes_kebab_case() {
        for (lane, expected) in [
            (TriageLane::Blocking, "blocking"),
            (TriageLane::Hot, "hot"),
            (TriageLane::Aging, "aging"),
            (TriageLane::Standing, "standing"),
        ] {
            let dto = AttentionCarryoverDto {
                lane,
                ..sample_attention_carryover()
            };
            let v = serde_json::to_value(&dto).expect("serialize");
            assert_eq!(v["lane"], expected);
        }
    }

    // ── RunStateDto / NodeTransitionDto (BA.11.M) ──────────────────────────

    fn sample_run_state_dto() -> RunStateDto {
        RunStateDto {
            run_id: "b6a1c1e0-0000-4000-8000-000000000000".to_owned(),
            event: serde_json::json!({ "ticket_id": "T-1" }),
            metadata: serde_json::json!({ "workflow": "sdlc-flow" }),
            nodes: vec![
                NodeTransitionDto {
                    node: "DataIngestionNode".to_owned(),
                    status: "success".to_owned(),
                    started_at: Some("2026-07-24T12:00:00Z".to_owned()),
                    completed_at: Some("2026-07-24T12:00:01Z".to_owned()),
                    error: None,
                    input: None,
                    output: Some(serde_json::json!({ "documents_loaded": 3 })),
                    usage: None,
                },
                NodeTransitionDto {
                    node: "SummarizeNode".to_owned(),
                    status: "failed".to_owned(),
                    started_at: Some("2026-07-24T12:00:01Z".to_owned()),
                    completed_at: Some("2026-07-24T12:00:02Z".to_owned()),
                    error: Some("timeout".to_owned()),
                    input: Some(serde_json::json!({ "documents": 3 })),
                    output: None,
                    usage: Some(RunUsageDto {
                        input_tokens: Some(512),
                        output_tokens: Some(128),
                        model: "claude-sonnet-5".to_owned(),
                    }),
                },
            ],
        }
    }

    #[test]
    fn run_state_dto_round_trips() {
        let dto = sample_run_state_dto();
        let json = serde_json::to_string(&dto).expect("serialize");
        let back: RunStateDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(dto, back);
    }

    #[test]
    fn run_state_dto_serializes_expected_fields() {
        let dto = sample_run_state_dto();
        let v = serde_json::to_value(&dto).expect("serialize");
        assert_eq!(v["run_id"], "b6a1c1e0-0000-4000-8000-000000000000");
        assert_eq!(v["event"]["ticket_id"], "T-1");
        assert_eq!(v["nodes"][0]["node"], "DataIngestionNode");
        assert_eq!(v["nodes"][0]["status"], "success");
        assert!(v["nodes"][0]["error"].is_null());
        assert_eq!(v["nodes"][1]["status"], "failed");
        assert_eq!(v["nodes"][1]["error"], "timeout");
        assert_eq!(v["nodes"][1]["usage"]["model"], "claude-sonnet-5");
    }

    #[test]
    fn run_state_dto_usage_null_when_absent() {
        let dto = sample_run_state_dto();
        let v = serde_json::to_value(&dto).expect("serialize");
        assert!(v["nodes"][0]["usage"].is_null());
        assert!(v["nodes"][0]["output"].is_object());
        assert!(v["nodes"][1]["output"].is_null());
    }

    // ── RunSummaryDto (BA.11.T) ──────────────────────────────────────────────

    fn sample_run_summary_dto() -> RunSummaryDto {
        RunSummaryDto {
            run_id: "b6a1c1e0-0000-4000-8000-000000000000".to_owned(),
            workflow_type: Some("sdlc-flow".to_owned()),
            status: "running".to_owned(),
            spec_slug: Some("11.T-run-summary-projection".to_owned()),
            started_at: Some("2026-07-24T12:00:00Z".to_owned()),
            updated_at: Some("2026-07-24T12:00:01Z".to_owned()),
            repo: Some("bastion".to_owned()),
        }
    }

    #[test]
    fn run_summary_dto_round_trips_fully_populated() {
        let dto = sample_run_summary_dto();
        let json = serde_json::to_string(&dto).expect("serialize");
        let back: RunSummaryDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(dto, back);
    }

    #[test]
    fn run_summary_dto_optional_fields_serialize_as_expected() {
        let dto = RunSummaryDto {
            run_id: "c7b2d2f1-0000-4000-8000-000000000000".to_owned(),
            workflow_type: None,
            status: "pending".to_owned(),
            spec_slug: None,
            started_at: None,
            updated_at: None,
            repo: None,
        };
        let v = serde_json::to_value(&dto).expect("serialize");

        // workflow_type / spec_slug / repo: `None` -> absent key, not `null`.
        assert!(
            !v.as_object().expect("object").contains_key("workflow_type"),
            "workflow_type should be an absent key when None, got: {v}"
        );
        assert!(
            !v.as_object().expect("object").contains_key("spec_slug"),
            "spec_slug should be an absent key when None, got: {v}"
        );
        assert!(
            !v.as_object().expect("object").contains_key("repo"),
            "repo should be an absent key when None, got: {v}"
        );

        // started_at / updated_at: `None` -> explicit `null`, key present.
        assert!(v.as_object().expect("object").contains_key("started_at"));
        assert!(v["started_at"].is_null());
        assert!(v.as_object().expect("object").contains_key("updated_at"));
        assert!(v["updated_at"].is_null());

        // round-trip still works with the absent/null mix.
        let json = serde_json::to_string(&dto).expect("serialize");
        let back: RunSummaryDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(dto, back);
    }

    #[test]
    fn run_summary_dto_populated_fields_serialize_present() {
        let dto = sample_run_summary_dto();
        let v = serde_json::to_value(&dto).expect("serialize");
        assert_eq!(v["run_id"], "b6a1c1e0-0000-4000-8000-000000000000");
        assert_eq!(v["workflow_type"], "sdlc-flow");
        assert_eq!(v["status"], "running");
        assert_eq!(v["spec_slug"], "11.T-run-summary-projection");
        assert_eq!(v["started_at"], "2026-07-24T12:00:00Z");
        assert_eq!(v["updated_at"], "2026-07-24T12:00:01Z");
        assert_eq!(v["repo"], "bastion");
    }

    #[test]
    fn run_summary_dto_repo_round_trips_when_present() {
        let dto = sample_run_summary_dto();
        assert_eq!(dto.repo.as_deref(), Some("bastion"));

        let json = serde_json::to_string(&dto).expect("serialize");
        let v: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(v["repo"], "bastion");

        let back: RunSummaryDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.repo.as_deref(), Some("bastion"));
        assert_eq!(dto, back);
    }

    #[test]
    fn run_summary_dto_repo_absent_key_when_none() {
        let mut dto = sample_run_summary_dto();
        dto.repo = None;

        let v = serde_json::to_value(&dto).expect("serialize");
        assert!(
            !v.as_object().expect("object").contains_key("repo"),
            "repo should be an absent key (not null) when unattributed, got: {v}"
        );

        // Deserializing a payload with no `repo` key at all (e.g. a pre-A7 golden) still
        // works and yields `None`, thanks to `#[serde(default)]`.
        let json = serde_json::to_string(&v).expect("serialize value");
        let back: RunSummaryDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.repo, None);
        assert_eq!(dto, back);
    }

    // ── DocTreeDto / DocEntryDto / DocFileDto (BA.11.Q) ────────────────────

    fn sample_doc_tree_dto() -> DocTreeDto {
        DocTreeDto {
            repo: "bastion".to_owned(),
            root: "planning".to_owned(),
            entries: vec![
                DocEntryDto {
                    path: "planning/status.md".to_owned(),
                    name: "status.md".to_owned(),
                    is_dir: false,
                },
                DocEntryDto {
                    path: "planning/decisions".to_owned(),
                    name: "decisions".to_owned(),
                    is_dir: true,
                },
            ],
        }
    }

    fn sample_doc_file_dto() -> DocFileDto {
        DocFileDto {
            repo: "bastion".to_owned(),
            path: "planning/status.md".to_owned(),
            content: "---\ntype: Status\n---\n\n# bastion — Status\n".to_owned(),
            bytes: 41,
            modified: Some("2026-07-24T18:03:11Z".to_owned()),
        }
    }

    #[test]
    fn doc_tree_dto_round_trips() {
        let dto = sample_doc_tree_dto();
        let json = serde_json::to_string(&dto).expect("serialize");
        let back: DocTreeDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(dto, back);
    }

    #[test]
    fn doc_tree_dto_serializes_expected_fields() {
        let dto = sample_doc_tree_dto();
        let v = serde_json::to_value(&dto).expect("serialize");
        assert_eq!(v["repo"], "bastion");
        assert_eq!(v["root"], "planning");
        assert_eq!(v["entries"][0]["path"], "planning/status.md");
        assert_eq!(v["entries"][0]["name"], "status.md");
        assert_eq!(v["entries"][0]["is_dir"], false);
        assert_eq!(v["entries"][1]["is_dir"], true);
    }

    #[test]
    fn doc_tree_dto_entries_default_to_empty_when_absent() {
        let raw = r#"{"repo":"bastion","root":""}"#;
        let dto: DocTreeDto = serde_json::from_str(raw).expect("entries should default");
        assert!(dto.entries.is_empty());
    }

    #[test]
    fn doc_file_dto_round_trips() {
        let dto = sample_doc_file_dto();
        let json = serde_json::to_string(&dto).expect("serialize");
        let back: DocFileDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(dto, back);
    }

    #[test]
    fn doc_file_dto_serializes_expected_fields() {
        let dto = sample_doc_file_dto();
        let v = serde_json::to_value(&dto).expect("serialize");
        assert_eq!(v["repo"], "bastion");
        assert_eq!(v["path"], "planning/status.md");
        assert_eq!(v["bytes"], 41);
        assert_eq!(v["modified"], "2026-07-24T18:03:11Z");
        assert!(v["content"].as_str().unwrap().starts_with("---\ntype"));
    }

    #[test]
    fn doc_file_dto_modified_defaults_to_null_when_absent() {
        let raw = r#"{"repo":"bastion","path":"README.md","content":"hello","bytes":5}"#;
        let dto: DocFileDto = serde_json::from_str(raw).expect("modified should default");
        assert!(dto.modified.is_none());
    }

    // ── Board / Epics (BA.11.R) ─────────────────────────────────────────────

    #[test]
    fn board_block_dto_deserializes_pre_block_body() {
        let raw = r#"{"id":"BA.11.K","title":"t","repo":"bastion","status":"in_progress","blocked_by":[]}"#;
        let dto: BoardBlockDto = serde_json::from_str(raw).expect("pre-block body should decode");
        assert_eq!(dto.epics, Vec::<String>::new());
        assert!(dto.wave.is_none());
        assert!(dto.priority.is_none());
        assert!(dto.due.is_none());
        assert!(dto.track.is_none());
        assert!(dto.last_touched.is_none());
    }

    #[test]
    fn board_block_dto_last_touched_round_trips_some() {
        let mut original = sample_board_block();
        original.last_touched = Some("2026-07-28T12:00:00Z".to_string());
        let json = serde_json::to_string(&original).expect("serialize");
        let decoded: BoardBlockDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(
            decoded.last_touched.as_deref(),
            Some("2026-07-28T12:00:00Z"),
            "last_touched must round-trip verbatim"
        );
    }

    #[test]
    fn board_block_dto_last_touched_none_omits_key_from_json() {
        let mut dto = sample_board_block();
        dto.last_touched = None;
        let v = serde_json::to_value(&dto).expect("serialize");
        assert!(
            v.get("last_touched").is_none(),
            "last_touched must be an absent key when None, not null: {v:?}"
        );
    }

    #[test]
    fn board_block_dto_round_trips_all_fields() {
        let original = BoardBlockDto {
            id: "BA.11.R".to_string(),
            title: "Epic + ranking enrichment".to_string(),
            repo: "bastion".to_string(),
            status: Some("in_progress".to_string()),
            blocked_by: Vec::new(),
            epics: vec!["bastion-surfaces".to_string()],
            wave: Some(3),
            priority: Some(1),
            due: Some("2026-07-15".to_string()),
            track: Some("Phase 11".to_string()),
            last_touched: Some("2026-07-28T12:00:00Z".to_string()),
            dependent_count: Some(4),
            ready: Some(true),
            unmet_count: None,
        };
        let json = serde_json::to_string(&original).expect("serialize");
        let decoded: BoardBlockDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, decoded, "round-trip must preserve all fields");
    }

    #[test]
    fn board_block_dto_graph_enrichment_all_populated_round_trips() {
        let mut dto = sample_board_block();
        dto.dependent_count = Some(3);
        dto.ready = Some(true);
        dto.unmet_count = Some(0);
        let json = serde_json::to_string(&dto).expect("serialize");
        let decoded: BoardBlockDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(dto, decoded, "round-trip must preserve all three fields");
        let v = serde_json::to_value(&dto).expect("serialize to value");
        assert_eq!(v["dependent_count"], serde_json::json!(3));
        assert_eq!(v["ready"], serde_json::json!(true));
        assert_eq!(v["unmet_count"], serde_json::json!(0));
    }

    #[test]
    fn board_block_dto_graph_enrichment_all_absent_omits_keys() {
        let mut dto = sample_board_block();
        dto.dependent_count = None;
        dto.ready = None;
        dto.unmet_count = None;
        let v = serde_json::to_value(&dto).expect("serialize to value");
        let obj = v.as_object().expect("object");
        assert!(
            !obj.contains_key("dependent_count"),
            "dependent_count must be an absent key when None, got: {v}"
        );
        assert!(
            !obj.contains_key("ready"),
            "ready must be an absent key when None, got: {v}"
        );
        assert!(
            !obj.contains_key("unmet_count"),
            "unmet_count must be an absent key when None, got: {v}"
        );
        let decoded: BoardBlockDto = serde_json::from_str(&v.to_string()).expect("deserialize");
        assert_eq!(dto, decoded, "round-trip must preserve all-absent state");
    }

    #[test]
    fn board_block_dto_graph_enrichment_unmet_count_present_others_absent() {
        // The blocked-lane shape: dependent_count/ready absent (block not in the
        // graph export) while unmet_count is populated. Distinct from the mix
        // above — pins that the three fields are independently optional, not a
        // single all-or-nothing enrichment bundle.
        let mut dto = sample_board_block();
        dto.dependent_count = None;
        dto.ready = None;
        dto.unmet_count = Some(2);
        let v = serde_json::to_value(&dto).expect("serialize to value");
        let obj = v.as_object().expect("object");
        assert!(!obj.contains_key("dependent_count"));
        assert!(!obj.contains_key("ready"));
        assert_eq!(v["unmet_count"], serde_json::json!(2));
        let decoded: BoardBlockDto = serde_json::from_str(&v.to_string()).expect("deserialize");
        assert_eq!(dto, decoded, "round-trip must preserve the mixed state");
    }

    #[test]
    fn board_block_dto_serializes_empty_epics_as_array() {
        let dto = BoardBlockDto {
            id: "BA.11.R".to_string(),
            title: "t".to_string(),
            repo: "bastion".to_string(),
            status: None,
            blocked_by: Vec::new(),
            epics: Vec::new(),
            wave: None,
            priority: None,
            due: None,
            track: None,
            last_touched: None,
            dependent_count: None,
            ready: None,
            unmet_count: None,
        };
        let v = serde_json::to_value(&dto).expect("serialize");
        assert!(v["epics"].is_array());
        assert!(v["epics"].as_array().unwrap().is_empty());
    }

    #[test]
    fn epic_dto_round_trips() {
        let original = EpicDto {
            slug: "bastion-surfaces".to_string(),
            title: "Bastion Surfaces".to_string(),
            description: Some("Surfaces initiative".to_string()),
            status: Some("active".to_string()),
            weight: Some(80),
            plan: Some("core/planning/master-plan.md".to_string()),
            repos: vec!["bastion".to_string(), "bastion-web".to_string()],
            closed: 3,
            in_progress: 1,
            open: 0,
            deferred: 2,
            total: 6,
            fully_deferred: false,
        };
        let json = serde_json::to_string(&original).expect("serialize");
        let decoded: EpicDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, decoded, "round-trip must preserve all fields");
    }

    #[test]
    fn epic_dto_defaults_optional_fields() {
        let raw = r#"{"slug":"s","title":"T"}"#;
        let dto: EpicDto = serde_json::from_str(raw).expect("minimal body should decode");
        assert!(dto.description.is_none());
        assert!(dto.status.is_none());
        assert!(dto.plan.is_none());
        assert!(dto.repos.is_empty());
        assert!(
            dto.weight.is_none(),
            "absent weight key must decode to None via #[serde(default)]"
        );
    }

    #[test]
    fn epic_dto_authored_weight_serializes_as_number() {
        let dto = EpicDto {
            weight: Some(80),
            ..epic_dto_fixture()
        };
        let v = serde_json::to_value(&dto).expect("serialize");
        assert_eq!(v["weight"], serde_json::json!(80));
    }

    #[test]
    fn epic_dto_unauthored_weight_serializes_as_null() {
        let dto = EpicDto {
            weight: None,
            ..epic_dto_fixture()
        };
        let v = serde_json::to_value(&dto).expect("serialize");
        assert!(
            v.get("weight").is_some(),
            "weight must be present on the wire (EpicDto does not skip_serializing Options)"
        );
        assert!(
            v["weight"].is_null(),
            "unauthored weight must serialize as null, not be omitted"
        );
    }

    #[test]
    fn epic_dto_out_of_policy_weight_round_trips_unchanged() {
        // mev owns the 0..=100 range policy (E_STATE_EPIC_BAD_WEIGHT); the DTO
        // must carry whatever was authored, unclamped.
        let original = EpicDto {
            weight: Some(200),
            ..epic_dto_fixture()
        };
        let json = serde_json::to_string(&original).expect("serialize");
        let decoded: EpicDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.weight, Some(200));
    }

    /// Minimal `EpicDto` used by the `weight` cases — every field but `weight`
    /// held constant so each test varies exactly one thing.
    fn epic_dto_fixture() -> EpicDto {
        EpicDto {
            slug: "bastion-surfaces".to_string(),
            title: "Bastion Surfaces".to_string(),
            description: None,
            status: None,
            weight: None,
            plan: None,
            repos: Vec::new(),
            closed: 0,
            in_progress: 0,
            open: 0,
            deferred: 0,
            total: 0,
            fully_deferred: false,
        }
    }

    #[test]
    fn board_scope_deserializes_epic() {
        let scope: BoardScope = serde_json::from_str("\"epic\"").expect("epic should decode");
        assert_eq!(scope, BoardScope::Epic);
    }

    #[test]
    fn board_scope_rejects_unknown_variant() {
        let result: Result<BoardScope, _> = serde_json::from_str("\"bogus\"");
        assert!(result.is_err());
    }

    // ── BlockLaneDto ───────────────────────────────────────────────────────

    #[test]
    fn block_lane_dto_serializes_snake_case() {
        let cases = [
            (BlockLaneDto::Now, "now"),
            (BlockLaneDto::Next, "next"),
            (BlockLaneDto::Blocked, "blocked"),
            (BlockLaneDto::Deferred, "deferred"),
            (BlockLaneDto::Closed, "closed"),
            (BlockLaneDto::Other, "other"),
        ];
        for (variant, expected) in cases {
            let v = serde_json::to_value(variant).expect("serialize BlockLaneDto");
            assert_eq!(
                v,
                json!(expected),
                "variant {variant:?} must serialize to {expected:?}"
            );
        }
    }

    #[test]
    fn block_lane_dto_deserializes_snake_case() {
        let cases = [
            ("\"now\"", BlockLaneDto::Now),
            ("\"next\"", BlockLaneDto::Next),
            ("\"blocked\"", BlockLaneDto::Blocked),
            ("\"deferred\"", BlockLaneDto::Deferred),
            ("\"closed\"", BlockLaneDto::Closed),
            ("\"other\"", BlockLaneDto::Other),
        ];
        for (raw, expected) in cases {
            let decoded: BlockLaneDto =
                serde_json::from_str(raw).expect("deserialize BlockLaneDto");
            assert_eq!(decoded, expected);
        }
    }

    #[test]
    fn block_lane_dto_rejects_unknown_variant() {
        let result: Result<BlockLaneDto, _> = serde_json::from_str("\"bogus\"");
        assert!(result.is_err());
    }

    // ── BlockEdgeKindDto ───────────────────────────────────────────────────

    #[test]
    fn block_edge_kind_dto_serializes_snake_case() {
        let cases = [
            (BlockEdgeKindDto::BlockedBy, "blocked_by"),
            (BlockEdgeKindDto::CrossRepo, "cross_repo"),
        ];
        for (variant, expected) in cases {
            let v = serde_json::to_value(variant).expect("serialize BlockEdgeKindDto");
            assert_eq!(
                v,
                json!(expected),
                "variant {variant:?} must serialize to {expected:?}"
            );
        }
    }

    #[test]
    fn block_edge_kind_dto_deserializes_snake_case() {
        let cases = [
            ("\"blocked_by\"", BlockEdgeKindDto::BlockedBy),
            ("\"cross_repo\"", BlockEdgeKindDto::CrossRepo),
        ];
        for (raw, expected) in cases {
            let decoded: BlockEdgeKindDto =
                serde_json::from_str(raw).expect("deserialize BlockEdgeKindDto");
            assert_eq!(decoded, expected);
        }
    }

    #[test]
    fn block_edge_kind_dto_rejects_unknown_variant() {
        let result: Result<BlockEdgeKindDto, _> = serde_json::from_str("\"bogus\"");
        assert!(result.is_err());
    }

    // ── BlockGraphNodeDto ──────────────────────────────────────────────────

    fn sample_node() -> BlockGraphNodeDto {
        BlockGraphNodeDto {
            key: "bastion:BA.17.A".to_owned(),
            repo: "bastion".to_owned(),
            id: "BA.17.A".to_owned(),
            title: "GET /api/blocks/graph endpoint".to_owned(),
            status: Some("in_progress".to_owned()),
            lane: BlockLaneDto::Now,
            track: Some("Phase 17".to_owned()),
            wave: Some(1),
            priority: Some(1),
            effective_priority: Some(1),
            due: None,
            epics: vec!["bastion-surfaces".to_owned()],
            layer: 0,
            topo_index: 4,
            ready: true,
            in_cycle: false,
            in_scope: true,
            external_deps: vec![],
            unmet_count: 0,
            dependent_count: 2,
        }
    }

    #[test]
    fn block_graph_node_dto_round_trip() {
        let node = sample_node();
        let json = serde_json::to_string(&node).expect("serialize BlockGraphNodeDto");
        let decoded: BlockGraphNodeDto =
            serde_json::from_str(&json).expect("deserialize BlockGraphNodeDto");
        assert_eq!(node, decoded);
    }

    #[test]
    fn block_graph_node_dto_serializes_expected_field_names() {
        let node = sample_node();
        let v = serde_json::to_value(&node).expect("serialize BlockGraphNodeDto");
        for field in [
            "key",
            "repo",
            "id",
            "title",
            "status",
            "lane",
            "track",
            "wave",
            "priority",
            "effective_priority",
            "due",
            "epics",
            "layer",
            "topo_index",
            "ready",
            "in_cycle",
            "in_scope",
            "external_deps",
            "unmet_count",
            "dependent_count",
        ] {
            assert!(
                v.get(field).is_some(),
                "BlockGraphNodeDto JSON must contain field {field:?}, got {v}"
            );
        }
        assert_eq!(v["lane"], "now");
    }

    #[test]
    fn block_graph_node_dto_defaults_optional_fields() {
        let raw = r#"{
            "key":"bastion:BA.1","repo":"bastion","id":"BA.1","title":"T",
            "lane":"next","layer":0,"topo_index":0,"ready":true,"in_cycle":false,
            "in_scope":true,"unmet_count":0,"dependent_count":0
        }"#;
        let node: BlockGraphNodeDto =
            serde_json::from_str(raw).expect("minimal body should decode");
        assert!(node.status.is_none());
        assert!(node.track.is_none());
        assert!(node.wave.is_none());
        assert!(node.priority.is_none());
        assert!(node.effective_priority.is_none());
        assert!(node.due.is_none());
        assert!(node.epics.is_empty());
        assert!(node.external_deps.is_empty());
    }

    // ── BlockGraphEdgeDto ──────────────────────────────────────────────────

    #[test]
    fn block_graph_edge_dto_round_trip() {
        let edge = BlockGraphEdgeDto {
            from: "bastion:BA.17.A".to_owned(),
            to_ref: "mev:MV.10.B".to_owned(),
            kind: BlockEdgeKindDto::BlockedBy,
            target_node_id: Some("mev:MV.10.B".to_owned()),
            blocking: false,
        };
        let json = serde_json::to_string(&edge).expect("serialize BlockGraphEdgeDto");
        let decoded: BlockGraphEdgeDto =
            serde_json::from_str(&json).expect("deserialize BlockGraphEdgeDto");
        assert_eq!(edge, decoded);
    }

    #[test]
    fn block_graph_edge_dto_dangling_target_is_none() {
        let edge = BlockGraphEdgeDto {
            from: "bastion:BA.17.A".to_owned(),
            to_ref: "unknown:XX.1".to_owned(),
            kind: BlockEdgeKindDto::CrossRepo,
            target_node_id: None,
            blocking: true,
        };
        let v = serde_json::to_value(&edge).expect("serialize BlockGraphEdgeDto");
        assert_eq!(v["target_node_id"], serde_json::Value::Null);
        assert_eq!(v["kind"], "cross_repo");
    }

    // ── BlockGraphDto ──────────────────────────────────────────────────────

    fn sample_graph_dto() -> BlockGraphDto {
        BlockGraphDto {
            version: "1".to_owned(),
            root: "/repo/hq".to_owned(),
            scope: BoardScope::Hq,
            tier: None,
            epic: None,
            repo: None,
            include_closed: false,
            include_boundary: false,
            nodes: vec![sample_node()],
            edges: vec![BlockGraphEdgeDto {
                from: "bastion:BA.17.A".to_owned(),
                to_ref: "mev:MV.10.B".to_owned(),
                kind: BlockEdgeKindDto::BlockedBy,
                target_node_id: Some("mev:MV.10.B".to_owned()),
                blocking: false,
            }],
            cycles: vec![],
            total_nodes: 1,
            truncated: false,
            stale: false,
        }
    }

    #[test]
    fn block_graph_dto_round_trip() {
        let dto = sample_graph_dto();
        let json = serde_json::to_string(&dto).expect("serialize BlockGraphDto");
        let decoded: BlockGraphDto =
            serde_json::from_str(&json).expect("deserialize BlockGraphDto");
        assert_eq!(dto, decoded);
    }

    #[test]
    fn block_graph_dto_serializes_expected_top_level_fields() {
        let dto = sample_graph_dto();
        let v = serde_json::to_value(&dto).expect("serialize BlockGraphDto");
        for field in [
            "version",
            "root",
            "scope",
            "tier",
            "epic",
            "repo",
            "include_closed",
            "include_boundary",
            "nodes",
            "edges",
            "cycles",
            "total_nodes",
            "truncated",
            "stale",
        ] {
            assert!(
                v.get(field).is_some(),
                "BlockGraphDto JSON must contain field {field:?}, got {v}"
            );
        }
        assert_eq!(v["scope"], "hq");
    }

    #[test]
    fn block_graph_dto_defaults_scope_to_hq_when_absent() {
        let raw = r#"{
            "version":"1","root":"/repo","include_closed":false,"include_boundary":false,
            "total_nodes":0,"truncated":false,"stale":false
        }"#;
        let dto: BlockGraphDto = serde_json::from_str(raw).expect("minimal body should decode");
        assert_eq!(dto.scope, BoardScope::Hq);
        assert!(dto.nodes.is_empty());
        assert!(dto.edges.is_empty());
        assert!(dto.cycles.is_empty());
    }

    // ── Cost read API DTOs (BA.11.J) ────────────────────────────────────────

    fn sample_workflow_cost_dto(name: &str, usd: f64) -> WorkflowCostDto {
        WorkflowCostDto {
            workflow_name: name.to_owned(),
            runs: 3,
            tokens_in: 1000,
            tokens_out: 200,
            usd,
        }
    }

    #[test]
    fn cost_summary_dto_round_trips_with_breached_budget() {
        let dto = CostSummaryDto {
            window: "7d".to_owned(),
            rows: vec![
                sample_workflow_cost_dto("content-pipeline", 1.32),
                sample_workflow_cost_dto("research-pipeline", 0.44),
            ],
            totals: sample_workflow_cost_dto("TOTAL", 1.76),
            unpriced_models: vec!["some-unpriced-model".to_owned()],
            budget: BudgetStateDto {
                max_total_tokens: Some(100_000),
                max_cost_usd: Some(1.5),
                total_tokens: 3600,
                total_cost_usd: 1.76,
                breached: true,
                breach: Some(BudgetBreachDto {
                    cap: "max_cost_usd".to_owned(),
                    spent: 1.76,
                    limit: 1.5,
                }),
            },
        };

        let json = serde_json::to_string(&dto).expect("serialize CostSummaryDto");
        let decoded: CostSummaryDto =
            serde_json::from_str(&json).expect("deserialize CostSummaryDto");
        assert_eq!(dto, decoded);

        let breach = decoded.budget.breach.expect("breach present");
        assert_eq!(breach.cap, "max_cost_usd");
    }

    #[test]
    fn cost_summary_dto_round_trips_with_absent_caps() {
        let dto = CostSummaryDto {
            window: "all".to_owned(),
            rows: vec![],
            totals: WorkflowCostDto {
                workflow_name: "TOTAL".to_owned(),
                runs: 0,
                tokens_in: 0,
                tokens_out: 0,
                usd: 0.0,
            },
            unpriced_models: vec![],
            budget: BudgetStateDto {
                max_total_tokens: None,
                max_cost_usd: None,
                total_tokens: 0,
                total_cost_usd: 0.0,
                breached: false,
                breach: None,
            },
        };

        let json = serde_json::to_string(&dto).expect("serialize CostSummaryDto");
        let decoded: CostSummaryDto =
            serde_json::from_str(&json).expect("deserialize CostSummaryDto");
        assert_eq!(dto, decoded);
        assert!(decoded.budget.max_total_tokens.is_none());
        assert!(decoded.budget.max_cost_usd.is_none());
        assert!(!decoded.budget.breached);
        assert!(decoded.budget.breach.is_none());
    }

    #[test]
    fn budget_breach_dto_cap_strings_match_cap_as_str_exactly() {
        use crate::costs::budget::Cap;

        let tokens_breach = BudgetBreachDto {
            cap: Cap::MaxTotalTokens.as_str().to_owned(),
            spent: 100_000.0,
            limit: 100_000.0,
        };
        let cost_breach = BudgetBreachDto {
            cap: Cap::MaxCostUsd.as_str().to_owned(),
            spent: 5.0,
            limit: 5.0,
        };

        assert_eq!(tokens_breach.cap, "max_total_tokens");
        assert_eq!(cost_breach.cap, "max_cost_usd");
    }
}
