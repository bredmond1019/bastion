---
type: Guideline
title: "serve-api contract v0.35"
description: "HTTP + WebSocket API contract for `bastion serve` — base URL, bearer-auth scheme, GET /health, /ws hub (topic subscriptions, live pane, needs-input event, workflow_done event), the v0.2 frame envelope, the v0.1 session REST surface (list/pane/send/key/create/delete), the v0.3 repo/workflow status REST surface (GET /repos, GET /repos/{name}/status, GET /repos/{name}/handoff, GET /repos/{name}/workflows), the v0.4 quick-action command endpoint (POST /actions/command, inject/spawn modes), the v0.6 cross-brain board endpoint (GET /api/board) that bastion-ui pins against, the v0.7 generated-TypeScript-types artifact (types/serve.ts, typeshare) for BastionWeb, the v0.8 live run read API (GET /api/runs, GET /api/runs/{id}) projecting the embedded engine's in-memory LiveStateStore for bastion-web's node drill-in (BA.11.M, D42 read half), the v0.9 Attention / carryover API (GET /api/attention) projecting the stale-carryover / aging-backlog / orphaned-capture board for bastion-ui, the TUI, and bastion-web BW.1.C (BA.11.P), the v0.10 Docs read API (GET /api/docs/{repo}/tree, GET /api/docs/{repo}/file) — an allowlisted, traversal-rejecting markdown tree + raw-file read across repos for bastion-web's reader (BW.2.A, BA.11.Q), and the v0.11 epic + ranking enrichment (epics/wave/priority/due/track on `BoardBlockDto`, `blocked_by` on all four lanes, GET /api/epics, and GET /api/board?scope=epic) for cutting work by cross-repo initiative (BA.11.R), the v0.12 pipeline / opportunities read API (GET /api/pipeline, GET /api/pipeline/{slug}) projecting the business sub-brain's opportunity markdown (researched companies + prospecting sweeps + job postings, with contacts, actions, and the body's ```json research brief) for bastion-web's pipeline board (BW.3.A), the v0.13 block-graph read API (GET /api/blocks/graph) — a mechanical projection of mev's enriched block-graph export (nodes/edges/cycles/lanes/topo-order, reusing the same board brain-walk) with zero derivation performed by bastion, for bastion-web's node-graph view (BW.9.B), the v0.14 `last_touched` field on `BoardBlockDto` — mev's derived per-block SDLC recency (`MV.10.D`) carried verbatim, with zero derivation by bastion, absent (not `null`) when a block has never been worked (BA.11.S), the v0.15 read-only Cost read API (GET /api/costs) — a projection of the existing `src/costs/` aggregation (BA.7.B) and budget-gate evaluation (BA.7.C) over HTTP, with `?window=` only (no `?repo=`, since the events contract carries no repo dimension) for bastion-ui and any web dashboard to render spend/budget without shelling to the CLI (BA.11.J), and the v0.16 `GET /api/runs` summary widening — from bare run-id strings to `RunSummaryDto` (run_id, workflow_type, status, spec_slug, started_at, updated_at), scoped strictly to `list_active()` live runs, reusing the existing `db::workflows::derive_run_status` for status and leaving `workflow_type` always absent pending the engine-rs follow-up ticket `EN.ticket.expose-live-run-workflow-type` (BA.11.T), and the v0.17 `suspended` run status — `db::workflows::derive_run_status` now reads `metadata.suspension.suspended` (engine-rs's `suspend.rs` marker) and reports it on `RunSummaryDto.status` wherever `cancelled`/`budget_halted` already were; `RunStateDto` (Section 14.2) has no aggregate status field to begin with (only per-node `NodeTransitionDto.status`, unaffected, and the raw `metadata` blob, which already carried `suspension` verbatim), so it needed no change; and the v0.18 `run_id` on `WorkflowStateDto` — the engine's `events.id` run UUID that engine-rs `EN.6.J` already stamps into `sdlc-flow-state.json`, carried through Section 11.4's response with `run_id` absent (not `null`) when the state predates that stamp or was written by base-template's JS `sdlc-flow.js` engine — plus a typeshared `HandoffInfoDto` mirroring Section 11.3's `HandoffInfo` domain type, unblocking bastion-web's `BW.3.F` band-merge and `BW.8.K3` briefing handoff feed; and the v0.19 `dependent_count`/`ready`/`unmet_count` enrichment on `BoardBlockDto` (A5) — mev's corpus-wide `build_block_graph_export` output carried verbatim onto every board lane entry behind an opt-in `?graph=1` query param (task 1 measured the unconditional call as roughly doubling `/api/board`'s wall-clock on the live HQ corpus), with `dependent_count`/`ready` populated for all five lanes and `unmet_count` populated only for `blocked`-lane entries — `ready`, not `unmet_count == 0`, is the readiness signal, since mev defines `unmet_count` as `0` for every non-blocked lane, and the v0.20 cross-repo workflows aggregate (GET /api/workflows, A2) — Section 11.6's new route returns every registered workspace's Section 11.4 flow states in one response, each entry tagged with a new typeshared `RepoWorkflowStateDto.repo` field, reusing `collect_flow_states` verbatim per workspace with no second flow-state walk, ordered deterministically by (repo, spec_slug), retiring the residual N+1 bastion-web's `/engine` on-disk band and briefing diff had left after A1/A5; the existing per-repo `GET /api/repos/{name}/workflows` route is unchanged; and the v0.21 `weight` field on `EpicDto` (GET /api/epics, `BA.ticket.epic-weight-dto`) — the authored `okf_core::Epic.weight` carried verbatim onto the wire with zero derivation by bastion (mev's `check_epics` owns the `0..=100` range policy via `E_STATE_EPIC_BAD_WEIGHT`, so an out-of-policy authored value passes through unclamped), `null` when unauthored — unblocking bastion-web's ranking of initiatives by authored weight, and
the v0.22 `repo` field on `RunSummaryDto` (`GET /api/runs`, A7) — an exact `run_id` join against
every registered workspace's flow state (`RepoWorkflowStateDto` from `collect_all_workflows`, A2),
absent (never guessed) when no flow state carries a run's `run_id`, gated behind an opt-in
`?with_repo=1` query param (mirroring `/api/board`'s `?graph=1`, A5) since task 1's measurement
found the registry walk roughly 6x the unenriched baseline against the live HQ registry (23 repos),
so the route's hottest consumer (bastion-web's ~2-6s run rail) does not pay for it unless it asks,
and the v0.23 subscribable `runs` WebSocket topic (`BA.11.N`, D17) — bastion pushes run-level
aggregate status transitions over the existing bearer-authed `/ws` hub by polling and diffing the
in-process `LiveStateStore` (`RunWatcher`, mirroring `FlowWatcher`), delivered as a new
`event{run_transition}` frame (`RunTransitionPayload`: run_id/status/terminal/spec_slug) gated on
subscription to the `\"runs\"` topic, plus an `event{run_stream_status}` frame
(`RunStreamStatusPayload`: available/reason) pushed immediately at subscribe time so a client learns
engine-mount availability without inferring it from silence; `terminal` means lifecycle-terminal
(the run left `LiveStateStore`'s live map), so `status: \"suspended\"` always pairs with
`terminal: false`; `GET /api/runs`/`GET /api/runs/{id}` are unchanged and remain the poll fallback;
and the v0.24 skipped-workspace report on `GET /api/workflows` (`BA.ticket.report-skipped-
workspaces`, A9) — an opt-in `?with_skipped=1` query param returns `{entries, skipped}` instead of
the bare array, naming which registered workspaces Section 11.6's walk could not fully report and
why (`unreadable_root` / `no_planning_dir` / `malformed_flow_state`), reusing the same single walk
`collect_flow_states` already performs with no second traversal, while the unparameterized default
response stays byte-identical to v0.23; and the v0.25 carryover triage ranking projection
(`BA.ticket.carryover-triage-dto`) — `GET /api/attention`'s `stale_carryover` lane now projects
mev's `rank_carryover` over the **full** carryover entry set instead of a
`carryover_stale_age`-filtered subset, so response size grows from roughly 6 entries fleet-wide to
roughly 138; `AttentionCarryoverDto` gains `lane`, `priority`, `effective_priority`,
`unmet_blocks`, `finding_id`, `clears_when_satisfied` (all verbatim from mev's
`CarryoverRanking`, contract-pinned in the new `docs/carryover-contract.md`, D20 pattern), and
`age_days` widens from `i64` to `Option<i64>` (the one non-additive change — a snoozed or
unparseable-anchor entry now reaches the board with no age), and the v0.26 `agent_state` field on
`SessionDto` (`BA.ticket.session-dto-agent-state`) — `From<&Session>` now carries the detected
`AgentState` (`\"idle\"` / `\"working\"` / `\"blocked\"` / `\"unknown\"`, from `detect/`) through to
both `GET /api/sessions` and the `\"sessions\"` WS push payload, closing the gap that left every
consumer of the sessions REST surface unable to tell whether a session is working, idle, or
blocked; `classify_state` and attachment handling are unchanged; the v0.27 operator-notification
transport (Section 26, `BA.18.B`) — an outbound-only background capability with no REST/WS route
of its own; and the v0.28 `POST /api/notify/test` trigger (Section 26.7,
`ticket-notify-send-trigger`) — an authenticated route inside the existing `/api` scope that sends
one real validated payload over the configured transport and registers it in a new bounded,
in-memory `PendingPayloads` registry so the `NotifyPollLoop`'s `PendingLookup` can resolve
`Accepted`/`StaleDigest`/`UnknownGate` for payloads this process sent, replacing the
`|_gate_id: &str| None` stub Section 26.6 previously described; adds one new typeshared DTO,
`NotifyTestResponseDto`; and the v0.29 acknowledgement contract (Section 26.8,
`ticket-telegram-answer-callback`) — every resolved verdict is now acknowledged via
`OperatorTransport::acknowledge` (`answerCallbackQuery`, then a best-effort message edit that
clears the live buttons and shows the chosen option) before the `VerdictSink` runs, fixing the
defect where Telegram silently timed out a tapped button because `answerCallbackQuery` was never
called; the ack handle and message handle are opaque, transport-agnostic, and `None`-safe for any
transport with no such concept, and no new DTO or route is introduced; and the v0.30 approve-and-run resolution (Section 26.9, `ticket-approve-and-run-seams`) — `PendingLookup` is now composed over the engine's `ApproveAndRunSeams::lookup_pending` first and the `/api/notify/test` registry as fallback, and the `VerdictSink` no longer merely logs: it records an approval-ledger row and, on an authorized matched-digest verdict, executes via `resolve_verdict` spawned onto the same actix local set rather than blocking the worker that serves HTTP and WS; no new DTO or route is introduced; and the v0.34 `effective_priority` field on `BoardBlockDto` (`BA.19.B`) — mev's min-propagated ranking (`mev::brain::block_graph::BlockGraphNode::effective_priority`) carried verbatim onto every board lane entry behind the same opt-in `?graph=1` gate as `dependent_count`/`ready`/`unmet_count`, absent when the block was absent from the graph export, `?graph=1` was not requested, or mev's own min-propagation never landed a value in the real `0..=3` range; bastion-web's `board-view.ts` already duck-types a read of this field, so no client-side change is required, and the v0.35 fleet-scoped lane API (GET /api/lanes, `BA.19.C`) — one aggregate row per lane SEGMENT across every registered roadmap in a single call, with an optional `?epic=<slug>` filter that never fans out to a per-roadmap call, a pure pass-through over `mev::lanes_brain` with zero availability derivation performed by bastion, unblocking the engine-orchestration roadmap's input contract (D74)"
doc_id: serve-api
layer: [console, surface, engine]
project: bastion
status: active
keywords: [serve-api, websocket, bastion-ui, contract, X-API-Key, cross-brain, block-graph]
related: [config, observ, data-contract, abort, master-plan]
---

# serve-api — v0.35 Contract

**Version:** v0.35  
**Produced by:** `bastion` (this repo, `src/serve/`) — Sections 1–17, 19–26 — plus, when mounted,
`engine-serve` (`../engine-rs/crates/engine-serve/`, embedded per D48) — Section 18.  
**Consumed by:** `bastion-ui` (Flutter mobile Surface, D28) for Sections 1–13, 15–17, 19–21, 24;
bastion-web (`BW.3.B`) for Section 14; bastion-web (`BW.1.C`) for Section 15; bastion-web
(`BW.2.A`) for Section 16; `bastion abort` (`src/run/abort.rs`, this repo) for Section 18's abort
route; bastion-web (`BW.9.B`) for Section 23; bastion-web (`BW.3.C`) for Section 6/8.3's `runs` topic;
Section 26 (the operator-notification transport) is an outbound-only background loop documented
here for the env-var contract and the no-webhook posture — no `bastion-ui`/`bastion-web` client
calls it; Section 26.7's `POST /api/notify/test` is likewise not a client-facing route, only an
operator-triggered smoke-test aid.

This document is the pinned contract between `bastion serve` and the Flutter
`bastion-ui` client.  `bastion-ui` MUST NOT rely on any behaviour not
documented here.  When a later block extends the API it bumps this version
(v0.2, v0.3, …) and records the delta in the Amendment Log at the bottom.

---

## 1. Base URL and bind address

| Configuration | Default | Env override |
|---|---|---|
| Bind address | `0.0.0.0:4317` | `BASTION_SERVE_ADDR` |

The server listens on the configured address.  In a Tailscale deployment the
host machine's tailnet IP is the reachable surface; `bastion-ui` connects to
`http://<tailnet-ip>:4317` (HTTP/1.1) or `ws://<tailnet-ip>:4317` (WS).

No TLS is provided at this layer — Tailscale's encrypted overlay handles
transport security on the tailnet.

---

## 2. Authentication

All routes **except** `GET /health` under bastion's own `/api` and `/ws` scopes are protected by
mandatory bearer-token authentication (Section 2.1–2.3). The embedded engine routes (Section 18,
mounted only when config allows) are a **separate, unmounted-at-`/api` surface with their own
`X-API-Key` gate** — the two auth schemes coexist side by side and are never double-applied to the
same request:

| Route family | Scheme | Header |
|---|---|---|
| `/health`, `/api/*`, `/ws` (Sections 3–13) | Bearer | `Authorization: Bearer <BASTION_SERVE_TOKEN>` |
| Engine routes (Section 18): `/events/`, `/events/{run_id}/abort` | API key | `X-API-Key: <BASTION_ENGINE_API_KEY>` |
| Engine routes (Section 18): `GET /health`, `GET /workflows`, `GET /workflows/{type}/graph` | None (public) | — |

The engine's own `GET /health` is shadowed by bastion's `/health` handler (first-registration-wins
for duplicate exact-path routes — verified empirically, not a panic), so the process's `/health`
contract (Section 3) is unchanged regardless of whether the engine is mounted.

### 2.1 Scheme

Clients MUST send an `Authorization` header on every protected request:

```
Authorization: Bearer <token>
```

`<token>` is the value of `BASTION_SERVE_TOKEN` (set on the server).  The
token is checked inside the pure `token_matches` helper (`src/serve/auth.rs`).
The scheme prefix `Bearer ` is matched case-sensitively.

### 2.2 Failure response

A missing, malformed, or incorrect token returns:

```
HTTP/1.1 401 Unauthorized
Content-Type: application/json
```

```json
{"error": "unauthorized", "code": "unauthorized"}
```

The client MUST treat any `401` as a fatal auth failure and prompt the operator
to verify the configured token.

### 2.3 Auth policy summary

| Route | Auth required |
|---|---|
| `GET /health` | No (public) |
| `GET /ws` (WS upgrade) | Yes — `Authorization: Bearer <token>` |
| `GET /api/sessions` | Yes — `Authorization: Bearer <token>` |
| `GET /api/sessions/{name}/pane` | Yes — `Authorization: Bearer <token>` |
| `POST /api/sessions/{name}/send` | Yes — `Authorization: Bearer <token>` |
| `POST /api/sessions/{name}/key` | Yes — `Authorization: Bearer <token>` |
| `POST /api/sessions` | Yes — `Authorization: Bearer <token>` |
| `DELETE /api/sessions/{name}` | Yes — `Authorization: Bearer <token>` |
| `GET /api/repos` | Yes — `Authorization: Bearer <token>` |
| `GET /api/repos/{name}/status` | Yes — `Authorization: Bearer <token>` |
| `GET /api/repos/{name}/handoff` | Yes — `Authorization: Bearer <token>` |
| `GET /api/repos/{name}/workflows` | Yes — `Authorization: Bearer <token>` |
| `POST /api/actions/command` | Yes — `Authorization: Bearer <token>` |
| `GET /api/board` | Yes — `Authorization: Bearer <token>` |
| `GET /api/attention` | Yes — `Authorization: Bearer <token>` |
| `GET /api/docs/{repo}/tree` | Yes — `Authorization: Bearer <token>` |
| `GET /api/docs/{repo}/file` | Yes — `Authorization: Bearer <token>` |
| `GET /api/epics` | Yes — `Authorization: Bearer <token>` |

---

## 3. `GET /health`

Liveness probe.  No authentication required.

### Request

```
GET /health HTTP/1.1
```

### Response

```
HTTP/1.1 200 OK
Content-Type: application/json
```

```json
{
  "status": "ok",
  "service": "bastion"
}
```

| Field | Type | Value |
|---|---|---|
| `status` | string | Always `"ok"` when the server is healthy |
| `service` | string | Always `"bastion"` |

### Error responses

| Status | Condition |
|---|---|
| `405 Method Not Allowed` | Any method other than `GET` on this path |

---

## 4. `GET /ws` — WebSocket upgrade (hub, v0.2)

Session hub socket.  Protected by bearer auth.  Replaced the v0 echo actor in v0.2.

### Upgrade request

```
GET /ws HTTP/1.1
Authorization: Bearer <token>
Connection: Upgrade
Upgrade: websocket
Sec-WebSocket-Key: <base64-key>
Sec-WebSocket-Version: 13
```

### Upgrade response (success)

```
HTTP/1.1 101 Switching Protocols
Connection: Upgrade
Upgrade: websocket
Sec-WebSocket-Accept: <accept-key>
```

### Upgrade failure responses

| Status | Condition |
|---|---|
| `401 Unauthorized` | Missing or invalid `Authorization` header |
| `400 Bad Request` | Malformed WS upgrade request |

After a successful upgrade the client interacts with the hub using the frame
protocol defined in Sections 5 and 6.

---

## 5. WebSocket frame envelope (v0.2)

All application-level messages are JSON objects wrapped in the frame envelope:

```json
{
  "kind": "<kind>",
  "payload": <any JSON value>
}
```

| Field | Type | Description |
|---|---|---|
| `kind` | string (snake_case) | Frame type discriminant.  Flutter client dispatches on this. |
| `payload` | any JSON | Frame body.  Shape is defined per-kind (see below). |

### Defined `kind` values (v0.2)

#### Client → server frames

| Kind | Description |
|---|---|
| `"subscribe"` | Subscribe to a topic (`sessions` or `pane:<name>`) |
| `"unsubscribe"` | Unsubscribe from a topic |
| `"send"` | Send literal keystrokes (+ Enter) to a tmux session |
| `"send_key"` | Send a single named tmux key to a tmux session |

#### Server → client frames

| Kind | Description |
|---|---|
| `"sessions"` | Snapshot of the current session list, pushed to `sessions` subscribers |
| `"pane"` | Pane diff pushed to `pane:<name>` subscribers when output changes |
| `"event"` | Async event pushed on significant state changes (e.g. `needs_input`) |
| `"error"` | Server-side error notification |

---

## 6. Topic model

After upgrading, clients opt in to data streams by subscribing to **topics**.
All pushes are server-initiated after subscription.

### Available topics

| Topic string | Data pushed | Cadence |
|---|---|---|
| `"sessions"` | `sessions` frame (session list snapshot) | Every poll interval (~2 s) when output changes |
| `"pane:<name>"` | `pane` frame (pane output diff) | Every poll interval (~2 s) when pane output changes |
| `"runs"` (v0.23, `BA.11.N`) | An `event{run_stream_status}` frame immediately on subscribe (availability), then `event{run_transition}` frames (Section 8.3) | `BASTION_POLL_INTERVAL` (~2 s), only while at least one connection is subscribed — the poller starts on the first `runs` subscriber and stops on the last |

`<name>` is the tmux session name (e.g. `"pane:work"`, `"pane:claude-1"`).

A connection may subscribe to multiple topics simultaneously.  Subscriptions are
per-connection and are released automatically on disconnect.

---

## 7. WebSocket frame payload shapes

### 7.1 `"subscribe"` payload (client → server)

```json
{ "topic": "pane:work" }
```

| Field | Type | Required | Description |
|---|---|---|---|
| `topic` | string | Yes | Topic to subscribe to (`"sessions"`, `"pane:<name>"` — name must be non-empty — or `"runs"`, v0.23) |

### 7.2 `"unsubscribe"` payload (client → server)

Same shape as `subscribe`:

```json
{ "topic": "sessions" }
```

### 7.3 `"send"` payload (client → server)

```json
{ "session": "main", "keys": "cargo test" }
```

| Field | Type | Required | Description |
|---|---|---|---|
| `session` | string | Yes | tmux session name to target |
| `keys` | string | Yes | Literal text to send (forwarded with `-l`), followed by `Enter` |

### 7.4 `"send_key"` payload (client → server)

```json
{ "session": "main", "key": "Escape" }
```

| Field | Type | Required | Description |
|---|---|---|---|
| `session` | string | Yes | tmux session name to target |
| `key` | string | Yes | Symbolic tmux key name (e.g. `"Escape"`, `"C-c"`, `"Enter"`) |

### 7.5 `"sessions"` payload (server → client)

Pushed to all `sessions` subscribers each poll cycle when the session list changes.

```json
{
  "sessions": [
    { "name": "main", "state": "running", "last_line": "$ cargo test", "agent_state": "working" },
    { "name": "scratch", "state": "idle", "last_line": "", "agent_state": "idle" }
  ]
}
```

| Field | Type | Description |
|---|---|---|
| `sessions` | array | Array of `SessionDto` objects (see Section 9.1) |

`last_line` is populated (as of v0.5) with each session's pane's last
non-blank captured line. As of `BA.18.A`, this pane capture is provided by
the always-on blocked-edge poller's shared sweep (`SharedSessionsSweep`,
`src/serve/poll.rs`) when that poller is wired in production — the
sessions-list poller reads the shared sweep instead of running its own
independent `list_sessions_raw` + per-session `capture_pane_raw` sweep on
the same cadence, so a subscribed client no longer doubles the tmux
subprocess count per interval. The sessions-list poller falls back to
running its own sweep only when no shared poller is wired (e.g. neither
`XDG_STATE_HOME` nor `HOME` is set — see Section 8.1). An idle session with
no captured output (or a capture failure) still yields `""`. `GET
/api/sessions` (Section 10.3) is **not** brought to the same parity in v0.5 —
it still returns empty `last_line` for every session, unchanged from prior
versions.

As of v0.31, a blocked session may additionally carry `blocked_reason`
(`"permission_prompt"` / `"awaiting_question"`) on `GET /api/sessions`; see `SessionDto` in
Section 10.3 for the absent-when-unknown semantics.

As of v0.26, `agent_state` (`"idle"` / `"working"` / `"blocked"` / `"unknown"`) is populated from
`Session::agent_state` (`detect/`) on both this push payload and `GET /api/sessions` alike — unlike
`last_line`, this field was added with full parity across the WS and REST surfaces from the start.
It is distinct from `state` above (tmux pane liveness) and from session attachment (the lease,
`engine-rs:EN.9.B`) — `classify_state` ignores attachment for state purposes and continues to.

### 7.6 `"pane"` payload (server → client)

Pushed to `pane:<name>` subscribers when captured pane output changes since the last push.

```json
{ "session": "main", "seq": 42, "lines": ["$ cargo build", "   Compiling bastion v0.1.0"] }
```

| Field | Type | Description |
|---|---|---|
| `session` | string | tmux session name |
| `seq` | integer (u64) | Monotonically increasing sequence number; increments on each diff push |
| `lines` | array of string | Current pane output lines at time of push |

### 7.7 `"event"` payload (server → client)

Pushed when a significant event is detected.

```json
{ "session": "main", "event": "needs_input" }
```

| Field | Type | Description |
|---|---|---|
| `session` | string | tmux session name where the event was detected (empty string for repo-scoped events such as `workflow_done`, which carry their own `repo`/`spec_slug` fields instead) |
| `event` | string | Event name (see table below) |

#### Defined event names

| Event | Since | Trigger condition |
|---|---|---|
| `"needs_input"` | v0.2 | Session pane is on a permission/approval prompt (`Blocked` state with `visible_blocker`, per `detect::detect()` over the Claude manifest).  Emitted once per rising edge (Blocked→not-Blocked→Blocked emits again; continuous Blocked does not repeat). |
| `"workflow_done"` | v0.3 | A spec's `sdlc-flow-state.json` transitions from a non-terminal `status` (e.g. `"running"`) to a terminal one (`"done"` or `"blocked"`), per `FlowWatcher::observe()` (`src/serve/poll.rs`).  Carries `repo`, `spec_slug`, and `status` fields alongside the `event` field (see Section 11.5). |
| `"run_transition"` | v0.23 | A `runs`-topic subscriber's tracked run's aggregate status changes, or the run disappears from `LiveStateStore::list_active()` (gone lifecycle-terminal), per `RunWatcher::observe()` (`src/serve/poll.rs`). Carries `run_id`, `status`, `terminal`, and an optional `spec_slug` field alongside the `event` field (see Section 8.3). |
| `"run_stream_status"` | v0.23 | Pushed once, immediately, to a connection when it subscribes to the `runs` topic — reports whether the engine is mounted (`available`) and, when not, why (`reason`). Not tied to any run; fires exactly once per subscribe (see Section 8.3). |

### 7.8 `"error"` payload (server → client)

```json
{ "code": "WS_ERR", "message": "<human-readable message>" }
```

| Field | Type | Description |
|---|---|---|
| `code` | string | Machine-readable error code |
| `message` | string | Human-readable error description |

---

## 8. Event semantics

### 8.1 `event{needs_input}` (v0.2; detection moved to the sessions-list poller in v0.5; relocated to an always-on independent poller in `BA.18.A`)

Needs-input detection runs in an always-on **blocked-edge poller**
(`BlockedEdgePoller`, `src/serve/blocked_edge/poller.rs`, `BA.18.A`), spawned
once at server boot (`src/serve/mod.rs::run_server`) independent of any
WebSocket subscription — **not** in the sessions-list poller, which no
longer owns any needs-input state. On its first tick the poller only
*seeds* its previous-state map (no emission), so a server restart with N
already-blocked sessions never replays N rising edges onto the
notification transport; only transitions observed on a tick *after* the
seed tick emit. Each tick the poller captures every live session's pane
output, calls `status::detect` to determine the agent state, and diffs the
result against the previous tick's per-session state using the pure
`sessions_needing_input(prev, current)` helper (`src/serve/poll.rs`,
unchanged since v0.5). The `needs_input` condition for a session is:

```
state == Blocked && visible_blocker == true
```

and the session's *previous* recorded state was not already `Blocked`
(rising edge — see below). Every crossing is appended to a durable,
append-only JSONL sink (`BlockedEdgeSink`; session, host, from/to state,
timestamp) regardless of whether any WebSocket client is connected. When
the poller is wired to the `Hub` (production wiring), each crossing is
also delivered to it as a `BlockedEdgeCrossed` actix message; the hub's
`Handler<BlockedEdgeCrossed>` fans the event out to whichever `sessions`
subscribers are connected *right now* (a no-op when there are none),
carrying that session's name — **a client needs no `pane:<name>`
subscription to receive it**. When the poller is also wired with
`.with_edge_tx(...)` (`tokio::sync::mpsc::Sender<BlockedEdgeRecord>`,
`BA.20.C` task 3), each crossing already appended to the durable sink is
additionally fanned out, via non-blocking `try_send`, to that channel — a
*second* consumer alongside the hub, not a replacement for it; a closed or
full channel is dropped and logged, never blocking the sink write. This is
the session-QA bridge's (Section 27) sole source of inbound crossings. This is what lets `bastion-ui`, which
subscribes only to `sessions` on connect, surface a needs-input alert for a
background session it has not opened a pane view for. The hub owns no
previous-state map of its own and never computes the rising edge itself —
it is purely a fan-out consumer of the poller's decision, so an
unsubscribe/resubscribe cycle can never replay a crossing the hub never
stored.

The blocked-edge poller uses a **rising-edge debounce**: the event is
emitted once per Blocked→Unblocked→Blocked transition cycle (i.e. once per
"new prompt"), not on every poll tick while the session remains blocked.
Consecutive blocked polls without an intervening non-blocked state produce
at most one event.

The event drives the BastionUI alert flow: the mobile operator is notified once
and can respond via a `send` or `send_key` frame to unblock the agent.

Needs-input is emitted from exactly one place (the always-on blocked-edge
poller); the sessions-list poller (Section 7.5) and the per-pane poll
interval (Section 7.6) never compute the rising edge themselves — the
former only reads the poller's shared pane-capture sweep for `last_line`,
and the latter only pushes pane-content diffs.

### 8.2 `event{workflow_done}` (v0.3)

[`FlowWatcher`](../src/serve/poll.rs) tracks the last-known `status` for every
`(repo, spec_slug)` pair it has observed from parsed `sdlc-flow-state.json`
files (Section 11.4).  `FlowWatcher::observe()` emits a `workflow_done` payload
when:

```
prev_status.is_some() && !is_terminal(prev_status) && is_terminal(current.status)
```

where `is_terminal(status)` is `true` for `"done"` and `"blocked"`.  No event is
emitted on the **first** observation of a given `(repo, spec_slug)` pair (no
`prev_status` to compare against), nor when the status is unchanged or was
already terminal on the previous observation.

The payload carries `{ "repo", "spec_slug", "status" }` flattened alongside the
`event` field (Section 7.7) — `status` is whichever terminal value (`"done"` or
`"blocked"`) triggered the transition.

This push is wired: `Hub` owns a `FlowWatcher` and runs an always-on poll
(`src/serve/ws/server.rs`, cadence = `BASTION_POLL_INTERVAL`, not gated on
subscribers) that broadcasts each emitted frame to every connected `/ws`
client, regardless of topic subscription.

### 8.3 `event{run_transition}` / `event{run_stream_status}` (v0.23, `BA.11.N`, D17)

Unlike `workflow_done` (Section 8.2), which is completion-only and broadcast to every connection,
this push is **run-level aggregate status**, transition-by-transition (not only on completion), and
**subscription-gated** — only connections subscribed to the `"runs"` topic (Section 6) receive it.
[`RunWatcher`](../src/serve/poll.rs) tracks the last-known aggregate `status` for every run id it has
observed from `LiveStateStore` (`../engine-rs/crates/engine-serve/src/live_state.rs`), derived via
`db::workflows::derive_run_status` — the same function `GET /api/runs` uses (Section 14.1), so the
stream and the poll fallback can never disagree.

**Availability, pushed at subscribe time (D17 constraint 2).** The instant a connection subscribes to
`"runs"`, the hub pushes one `event{run_stream_status}` frame to that connection (and only that
connection) before any `run_transition` frame can arrive:

```json
{ "session": "", "event": "run_stream_status", "available": true }
```

```json
{ "session": "", "event": "run_stream_status", "available": false, "reason": "DATABASE_URL not set" }
```

| Field | Type | Description |
|---|---|---|
| `available` | boolean | Whether the engine is mounted (Section 18.1), i.e. whether `LiveStateStore` can ever be written and a `run_transition` frame can ever arrive on this connection. |
| `reason` | string \| absent | Human-readable reason the engine is not mounted (e.g. a missing env var or a failed pool connect). Absent (not `null`) when `available` is `true`. |

`available: false` means the client must fall back to polling `GET /api/runs` / `GET /api/runs/{id}`
(Section 14) — those routes remain the retained, unconditional poll fallback (D17 constraint 3) and
are unaffected by this section either way.

**Run-transition emit predicate.** `RunWatcher::observe()` runs once per poll tick (only while
`runs_subs` is non-empty — Section 6's cadence row) over every id currently in
`LiveStateStore::list_active()`, and emits a `run_transition` payload for exactly two edges:

1. **Status change on a live run** — a previous status was recorded for this run id and the current
   one differs → emit with `terminal: false`. Nothing is emitted on the **first** observation of a
   run id (no previous status to compare against), nor when the status is unchanged.
2. **Disappearance** — a run id previously observed is no longer present in `list_active()` (it went
   lifecycle-terminal and `mark_terminal` moved it out of the live map into the completed ring). Its
   final status is read back via `LiveStateStore::get_record(run_id)` and emitted with
   `terminal: true`. When the record has since been evicted from the bounded completed ring, the
   watcher still emits `terminal: true`, carrying the last-known status rather than silently dropping
   the transition.

**Terminal status is derived differently from live status — deliberately.** Edge 1 uses
`db::workflows::derive_run_status` (the live aggregate, shared with Section 14.1). Edge 2 must not:
that aggregate reports `pending` whenever any node is still pending, which is right for a running
workflow but wrong for a finished one, because **a completed run legitimately leaves every never-taken
branch `Pending` forever**. A successful `SDLC_FLOW` run retains 15 `success` nodes and 4 `pending`
ones (untaken router branches), so the live aggregate would call it `"pending"`. The disappearance
edge therefore derives status from the retained snapshot with a terminal-specific rule, checked in
this order — mirroring `engine-serve`'s own `derive_terminal_status`
(`engine-rs/crates/engine-serve/src/http.rs:295`) condition for condition, mapped onto this
contract's vocabulary:

| # | Condition on the retained snapshot | `status` |
|---|---|---|
| 1 | `metadata.cancellation.cancelled == true` | `cancelled` |
| 2 | `metadata.budget.halted == true` | `budget_halted` |
| 3 | `metadata.failure.failed == true`, **or** any `node_runs[..].status == failed` | `failed` |
| 4 | none of the above | `success` |

Still-`pending` nodes are ignored at this edge — the run is over, so they mean "never executed", not
"not yet executed". `suspended` is unreachable here by construction: a suspended run is still in
`list_active()` and so never reaches this edge (D17 constraint 1). Note this reports `success` where
the engine's separate protocol reports `succeeded` — this stream deliberately agrees with
`GET /api/runs` (Section 14.1), not with the engine's wire vocabulary.

```json
{ "session": "", "event": "run_transition", "run_id": "b6a1c1e0-0000-4000-8000-000000000000", "status": "running", "terminal": false, "spec_slug": "11.N-run-transition-ws-push" }
```

```json
{ "session": "", "event": "run_transition", "run_id": "b6a1c1e0-0000-4000-8000-000000000000", "status": "success", "terminal": true }
```

| Field | Type | Description |
|---|---|---|
| `run_id` | string | The run's UUID as a string. |
| `status` | string | Aggregate run status string, from `db::workflows::derive_run_status` — the same values `RunSummaryDto.status` reports (Section 14.1: `pending`/`running`/`success`/`failed`/`cancelled`/`budget_halted`/`suspended`). |
| `terminal` | boolean | `true` only when the run has left `LiveStateStore`'s live map (lifecycle-terminal — the disappearance edge above); `false` for every other emitted status, including `"suspended"`. |
| `spec_slug` | string \| absent | The triggering event's `spec_slug`, when known. Absent (not `null`) when unknown. |

**Sampling limitation — silence is not evidence that nothing ran.** This is a *sampling* stream, not
an event log: `RunWatcher` sees only what `list_active()` holds at each poll tick. A run that starts
and reaches a terminal state **within a single poll interval** is never sampled, so it produces **no
frames at all** — not even a terminal one — because edge 1 needs a prior observation and edge 2 needs
the run to have been in `last_status` to be noticed leaving it. Verified live 2026-08-03: an
`OPPORTUNITY_SET_STAGE` run that failed at its first node completed in well under the 2 s cadence and
yielded zero frames. **The poll fallback has exactly the same blind spot** — `GET /api/runs` returned
`[]` across the same window, since it reads the identical `list_active()` set — so the stream is
neither better nor worse than polling here, which is the deliberate trade of D17's poll-diff design.
Consumers needing a guaranteed record of every run must read the durable Postgres `events` history
(`engine-store`'s writer), not this stream and not Section 14. Shortening `BASTION_POLL_INTERVAL`
narrows the window but cannot close it.

**D17 constraint 1 — wire-terminal is not lifecycle-terminal.** The embedded engine's own
`publish_suspended` push (`engine-rs/crates/engine-serve/src/stream.rs:185-191`) sends
`terminal: true` **together with** `status: "suspended"` on its own protocol — a naive
`if (frame.terminal) done` on that stream would misclassify a paused run as finished. This section's
`run_transition.terminal` deliberately means the opposite kind of terminal: lifecycle-terminal only.
A suspended run stays in `list_active()` (it is paused, not gone), so it flows through edge 1 above
and always emits `status: "suspended", terminal: false` — never `terminal: true`. A client doing
`if (frame.terminal) done` on **this** stream is safe.

This push is wired: `Hub` extends `runs_subs` (a `HashSet<ConnId>`) and starts a poll
(`src/serve/ws/server.rs`, cadence = `BASTION_POLL_INTERVAL`) only while at least one connection is
subscribed to `"runs"`, stopping it when the last such subscriber unsubscribes or disconnects —
unlike `workflow_done`'s always-on, ungated poll. Frames are fanned out only to `runs_subs`, never
broadcast to every connection.

---

## 9. Keep-alive / disconnect behaviour

Each `WsConn` runs a server-side heartbeat, installed in `started()`
(`src/serve/ws/session.rs`): every `HEARTBEAT_INTERVAL` (**5s**, default) tick,
the server sends a `Ping` frame; if no activity has been observed from the
client within `CLIENT_TIMEOUT` (**10s**, default) of the last-seen instant, the
server stops the actor (triggering a Disconnect) instead of sending another
ping. The client MUST respond to `Ping` with `Pong`; any inbound frame (`Pong`,
`Text`, or client `Ping`) updates the connection's last-seen instant and resets
the timeout window. Clients that fail to respond within the keep-alive window
are disconnected.

On disconnect (clean close, protocol error, or keep-alive timeout):
- All topic subscriptions for that connection are released atomically.
- Per-pane poll intervals are reference-counted: the pane poller is stopped when
  its last subscriber disconnects.
- The sessions-list poller is stopped when its last subscriber disconnects.

Binary frames received by the server are silently dropped.  Unknown client-sent
`kind` values that correspond to server-only frame types are ignored without error.

---

## 10. Sessions REST API (v0.1)

Six routes projecting the synchronous tmux session-control surface onto HTTP.
All routes live under the bearer-protected `/api` scope and return
`Content-Type: application/json`.

### 10.1 Response DTOs

#### `SessionDto`

Returned by `GET /api/sessions` (one element per session in the array).

```json
{
  "name": "main",
  "state": "running",
  "last_line": "$ cargo test",
  "agent_state": "working"
}
```

A blocked session awaiting an `AskUserQuestion` prompt carries the optional sub-classification:

```json
{
  "name": "main",
  "state": "running",
  "last_line": "",
  "agent_state": "blocked",
  "blocked_reason": "awaiting_question"
}
```

| Field | Type | Description |
|---|---|---|
| `name` | string | tmux session name |
| `state` | string | `"running"` when the foreground process is not a shell; `"idle"` otherwise |
| `last_line` | string | Last non-blank line from the session's pane, or `""` when unavailable |
| `agent_state` (v0.26) | string | Detected agent state — `"idle"` \| `"working"` \| `"blocked"` \| `"unknown"`, from `Session::agent_state` (`detect/`). Distinct from `state` (tmux pane liveness) and from attachment (the lease, `engine-rs:EN.9.B`) — `classify_state` ignores attachment and continues to. |
| `blocked_reason` (v0.31) | string, optional | Sub-classification of `agent_state == "blocked"` — `"permission_prompt"` (a tool-use approval dialog) or `"awaiting_question"` (a Claude Code `AskUserQuestion` prompt), from `Session::blocked_reason` (`detect/`). **Absent from the payload entirely — not `null` — when unknown or when the state is not blocked**, so a consumer must treat a missing key as "no sub-classification", never as an error. `agent_state` itself stays four-valued; this field exists precisely so no fifth variant was added. |

#### `PaneDto`

Returned by `GET /api/sessions/{name}/pane`.

```json
{
  "session_name": "main",
  "lines": ["$ cargo build", "   Compiling bastion v0.1.0", "    Finished"]
}
```

| Field | Type | Description |
|---|---|---|
| `session_name` | string | tmux session name this pane belongs to |
| `lines` | array of string | Captured pane output lines (trailing blank padding stripped) |

### 10.2 Request-body DTOs

#### `SendBody` — `POST /api/sessions/{name}/send`

```json
{ "keys": "cargo test" }
```

| Field | Type | Required | Description |
|---|---|---|---|
| `keys` | string | Yes | Literal text to send to the session (forwarded with `-l`), followed by `Enter` |

#### `KeyBody` — `POST /api/sessions/{name}/key`

```json
{ "key": "Escape" }
```

| Field | Type | Required | Description |
|---|---|---|---|
| `key` | string | Yes | Symbolic tmux key name (see accepted key names below) |

**Accepted key names** (non-exhaustive — tmux resolves these without the `-l` flag):

| Key name | Description |
|---|---|
| `Escape` | Escape key |
| `Enter` | Return / Enter key |
| `Up` | Arrow up |
| `Down` | Arrow down |
| `Left` | Arrow left |
| `Right` | Arrow right |
| `C-c` | Ctrl+C (SIGINT) |
| `C-d` | Ctrl+D (EOF) |
| `C-z` | Ctrl+Z (SIGTSTP) |

Any tmux-recognised key name or modifier combination (e.g. `M-f`, `C-Left`) is
accepted; the server forwards it verbatim to `tmux send-keys -t <name> <key>`
without `-l`/`--`.

#### `NewSessionBody` — `POST /api/sessions`

```json
{ "name": "mysession", "dir": "/optional/start/dir" }
```

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | Yes | Name of the new tmux session to create |
| `dir` | string | No | Starting directory for the session; omit for tmux default |

`dir` is omitted from the JSON object when `None` (`skip_serializing_if = "Option::is_none"`).

### 10.3 Routes

#### `GET /api/sessions` — list sessions

Returns all current tmux sessions.

**Request:**

```
GET /api/sessions HTTP/1.1
Authorization: Bearer <token>
```

**Response (200 OK):**

```json
[
  { "name": "main", "state": "running", "last_line": "$ cargo test", "agent_state": "working" },
  { "name": "scratch", "state": "idle", "last_line": "", "agent_state": "idle" }
]
```

An empty tmux server returns `[]`.  Tmux degradation returns an error object
(see Section 10.4).

---

#### `GET /api/sessions/{name}/pane` — read pane output

Captures the visible pane content for the named session.

**Path parameters:**

| Parameter | Description |
|---|---|
| `name` | tmux session name |

**Query parameters:**

| Parameter | Type | Required | Description |
|---|---|---|---|
| `lines` | integer | No | Maximum number of trailing lines to return.  Omit to return all non-blank lines. |

**Request:**

```
GET /api/sessions/main/pane?lines=20 HTTP/1.1
Authorization: Bearer <token>
```

**Response (200 OK):**

```json
{
  "session_name": "main",
  "lines": ["line1", "line2", "line3"]
}
```

Returns `404` when the session does not exist (see Section 10.4).

---

#### `POST /api/sessions/{name}/send` — send literal keystrokes

Sends a literal string to the session followed by `Enter`.  Uses tmux
`send-keys -l --` (literal flag) so the text is never interpreted as tmux key
names.

**Path parameters:**

| Parameter | Description |
|---|---|
| `name` | tmux session name |

**Request:**

```
POST /api/sessions/main/send HTTP/1.1
Authorization: Bearer <token>
Content-Type: application/json

{ "keys": "cargo test" }
```

**Response:** `204 No Content` on success (no body).

Returns `404` when the session does not exist (see Section 10.4).

---

#### `POST /api/sessions/{name}/key` — send a named key

Sends a single symbolic tmux key name (e.g. `Escape`, `Up`, `C-c`) to the
session.  Does **not** use `-l`/`--` so tmux resolves the key name and
dispatches the corresponding key event.

**Path parameters:**

| Parameter | Description |
|---|---|
| `name` | tmux session name |

**Request:**

```
POST /api/sessions/main/key HTTP/1.1
Authorization: Bearer <token>
Content-Type: application/json

{ "key": "Escape" }
```

**Response:** `204 No Content` on success (no body).

Returns `404` when the session does not exist (see Section 10.4).

---

#### `POST /api/sessions` — create a session

Creates a new detached tmux session.

**Request:**

```
POST /api/sessions HTTP/1.1
Authorization: Bearer <token>
Content-Type: application/json

{ "name": "mysession", "dir": "/home/user/project" }
```

**Response:** `201 Created` on success (no body).

Returns `500` when the session name is already in use (tmux exits non-zero).

---

#### `DELETE /api/sessions/{name}` — kill a session

Removes the named tmux session.

**Path parameters:**

| Parameter | Description |
|---|---|
| `name` | tmux session name |

**Request:**

```
DELETE /api/sessions/mysession HTTP/1.1
Authorization: Bearer <token>
```

**Response:** `204 No Content` on success (no body).

Returns `404` when the session does not exist (see Section 10.4).

---

### 10.4 Tmux degradation → HTTP status mapping

When a tmux call fails the server classifies the error and returns a JSON
error body using the `ErrorPayload` shape:

```json
{
  "code": "<C-code>",
  "message": "<human-readable description>"
}
```

| Condition | HTTP status | `code` |
|---|---|---|
| tmux binary not installed | `503 Service Unavailable` | `C001` |
| No tmux server running | `503 Service Unavailable` | `C001` |
| Unknown / missing session target | `404 Not Found` | `C002` |
| Other tmux exit error | `500 Internal Server Error` | `C010` |
| Unexpected server error | `500 Internal Server Error` | `C010` |

Error codes are from the C0xx taxonomy defined in `src/observ/errors.rs`.

**Example 503 body:**

```json
{ "code": "C001", "message": "no tmux server running" }
```

**Example 404 body:**

```json
{ "code": "C002", "message": "session not found: can't find session: nosuch" }
```

---

## 11. Repo / workflow status REST API (v0.3; cross-repo aggregate v0.20, A2)

Four read-only routes projecting per-workspace `planning/status.md`,
`planning/handoff.md`, and `sdlc-flow-state.json` files onto HTTP.  All routes
live under the bearer-protected `/api` scope and return
`Content-Type: application/json`.  Workspace roots are resolved from the
`[workspaces]` registry loaded at server startup (`load_workspace_registry()`,
`src/config.rs`) — the same registry the CLI's `--workspace` flag uses.

This surface is **read-only**: no route under `/api/repos` writes or mutates
any file.

### 11.1 `GET /api/repos` — list workspace registry entries

Returns a summary of every registered workspace.

**Request:**

```
GET /api/repos HTTP/1.1
Authorization: Bearer <token>
```

**Response (200 OK):**

```json
[
  { "name": "bastion", "now": "BA.11.D in progress — repo status API", "has_handoff": false },
  { "name": "bella", "now": "", "has_handoff": true }
]
```

| Field | Type | Description |
|---|---|---|
| `name` | string | Workspace registry name (`RepoSummaryDto`) |
| `now` | string | Frontmatter `now:` scalar from that workspace's `planning/status.md`; empty string when `status.md` is missing/unreadable/malformed |
| `has_handoff` | boolean | Whether `planning/handoff.md` exists for that workspace |

An empty/absent `[workspaces]` registry returns `[]`. Entries are sorted by
`name`.

---

### 11.2 `GET /api/repos/{name}/status` — full parsed `status.md`

**Path parameters:**

| Parameter | Description |
|---|---|
| `name` | Workspace registry name |

**Request:**

```
GET /api/repos/bastion/status HTTP/1.1
Authorization: Bearer <token>
```

**Response (200 OK):** `RepoStatusDto`

```json
{
  "name": "bastion",
  "now": "BA.11.D in progress — repo status API",
  "next": "Wire WS event push",
  "blocked": "[]",
  "has_handoff": false,
  "momentum_now": "BA.11.D in progress — repo status API",
  "momentum_next": "Wire WS event push",
  "momentum_blocked": "nothing blocked",
  "momentum_improve": "tighten parser edge cases",
  "momentum_recurring": "none yet"
}
```

| Field | Type | Description |
|---|---|---|
| `name` | string | Workspace registry name |
| `now` / `next` / `blocked` | string | Frontmatter scalars (D30) |
| `has_handoff` | boolean | Whether `planning/handoff.md` exists |
| `momentum_now` / `momentum_next` / `momentum_blocked` / `momentum_improve` / `momentum_recurring` | string | Body `## Momentum` queue line text; empty string when the section or bullet is absent |

Returns `404` (`ErrorPayload`, code `C005`) when `name` is not a registered
workspace, or `404` (code `C002`) when that workspace **is** registered but its
`planning/status.md` is missing or fails to parse (no well-formed frontmatter).
The two 404s are distinguishable by `code`: `C005` = unregistered workspace
name; `C002` = registered workspace with a missing/malformed `status.md`.

---

### 11.3 `GET /api/repos/{name}/handoff` — parsed `handoff.md`

**Path parameters:**

| Parameter | Description |
|---|---|
| `name` | Workspace registry name |

**Request:**

```
GET /api/repos/bastion/handoff HTTP/1.1
Authorization: Bearer <token>
```

**Response (200 OK):** `HandoffInfoDto` (v0.18; typeshared `#[typeshare]` DTO in
`src/serve/dto.rs`, mirroring the internal `HandoffInfo` domain type field-for-field per the
`RepoStatusDto` precedent — `HandoffInfo` itself carries no `#[typeshare]` annotation, since
`dto.rs` is the single source of truth for typeshared contract types)

```json
{
  "title": "Handoff — BA.11.C wrap-up",
  "body": "---\ntype: Handoff\n...\n# Handoff — BA.11.C wrap-up\n..."
}
```

| Field | Type | Description |
|---|---|---|
| `title` | string | Frontmatter `title:` scalar if present, else the `# Handoff —`/`# Handoff -` heading text, else `""` |
| `body` | string | The full raw markdown content of `handoff.md` (including frontmatter) |

Returns `404` (`ErrorPayload`, code `C005`) when `name` is not a registered
workspace, or `404` (code `C002`) when the workspace **is** registered but
`planning/handoff.md` does not exist for it. The two 404s are distinguishable
by `code`: `C005` = unregistered workspace name; `C002` = registered workspace
with no `handoff.md`.

---

### 11.4 `GET /api/repos/{name}/workflows` — parsed `sdlc-flow-state.json` entries

Walks `{workspace_root}/planning/*/sdlc/sdlc-flow-state.json` and parses each
match.

**Path parameters:**

| Parameter | Description |
|---|---|
| `name` | Workspace registry name |

**Request:**

```
GET /api/repos/bastion/workflows HTTP/1.1
Authorization: Bearer <token>
```

**Response (200 OK):** array of `WorkflowStateDto`

```json
[
  {
    "spec_slug": "phase6-blockA",
    "branch": "phase6-blockA-flow",
    "status": "done",
    "current_task": 5,
    "started_at": "2026-06-25T18:30:59Z",
    "updated_at": "2026-06-25T19:02:33Z",
    "run_id": "9c6c6f1e-6d1a-4b3a-9b1a-1e2f3a4b5c6d"
  }
]
```

| Field | Type | Description |
|---|---|---|
| `spec_slug` | string | Spec directory name under `planning/` |
| `branch` | string | Worktree branch name |
| `status` | string | Raw flow status (e.g. `"running"`, `"done"`, `"blocked"`) |
| `current_task` | integer | Current task index |
| `started_at` / `updated_at` | string (RFC 3339) | Timestamps from `sdlc-flow-state.json` |
| `run_id` (v0.18) | string \| absent | The engine's `events.id` run UUID that produced this write, stamped by engine-rs `EN.6.J` into the top-level `run_id` key of `sdlc-flow-state.json`. **Absent (not `null`) means the state predates that stamp, or was written by base-template's JS `sdlc-flow.js` engine (which never sets it) — never that the run lacks an id.** `#[serde(skip_serializing_if = "Option::is_none")]`, matching the `last_touched` (v0.14) convention on `BoardBlockDto`. |

Returns `404` (`ErrorPayload`, code `C005`) only when `name` is not a
registered workspace. A workspace with no specs, or no matching
`sdlc-flow-state.json` files, returns `200` with `[]`. Individual malformed
`sdlc-flow-state.json` files are skipped (not failed) — the route returns
whatever parses.

---

### 11.5 `event{workflow_done}` — pushed over `/ws`

Not a REST response — pushed asynchronously over the `/ws` hub connection (an
`"event"` frame per Section 7.7) when [`FlowWatcher::observe()`](../src/serve/poll.rs)
detects a `running`→terminal transition while polling the same
`sdlc-flow-state.json` files this section's routes read. See Section 8.2 for
the full transition semantics.

```json
{ "session": "", "event": "workflow_done", "repo": "bastion", "spec_slug": "phase11-blockD", "status": "done" }
```

| Field | Type | Description |
|---|---|---|
| `repo` | string | Workspace registry name the workflow belongs to |
| `spec_slug` | string | `sdlc-flow-state.json` spec slug |
| `status` | string | The terminal status that triggered the event (`"done"` or `"blocked"`) |

---

### 11.6 `GET /api/workflows` — cross-repo flow-state aggregate (v0.20, A2; skipped-workspace report v0.24, A9)

Every registered workspace's Section 11.4 flow states in one response, so consumers stop issuing
`GET /api/repos` followed by one `GET /api/repos/{name}/workflows` per repo — retiring the residual
N+1 on bastion-web's `/engine` on-disk band and briefing diff
(`planning/arch-review-asks-bastion-web/notes.md`, ask A2). Reuses `collect_flow_states` (Section
11.4's route) verbatim, once per registered workspace — no second flow-state walk exists.

An opt-in `?with_skipped=1` query param (v0.24, `BA.ticket.report-skipped-workspaces`, ask A9)
additionally reports which registered workspaces this walk could not fully account for, and why —
restoring the per-repo reachability signal that was otherwise indistinguishable from "this repo
simply has no runs."

**Request (default — unchanged from v0.20):**

```
GET /api/workflows HTTP/1.1
Authorization: Bearer <token>
```

**Response (200 OK):** array of `RepoWorkflowStateDto` — byte-identical to v0.23, proven by the
`workflows__empty` / `workflows__populated` contract-corpus goldens

```json
[
  {
    "repo": "bastion",
    "spec_slug": "phase6-blockA",
    "branch": "phase6-blockA-flow",
    "status": "done",
    "current_task": 5,
    "started_at": "2026-06-25T18:30:59Z",
    "updated_at": "2026-06-25T19:02:33Z",
    "run_id": "9c6c6f1e-6d1a-4b3a-9b1a-1e2f3a4b5c6d"
  },
  {
    "repo": "bella",
    "spec_slug": "phase2-blockB",
    "branch": "phase2-blockB-flow",
    "status": "running",
    "current_task": 2,
    "started_at": "2026-07-20T14:02:11Z",
    "updated_at": "2026-07-20T14:10:45Z"
  }
]
```

| Field | Type | Description |
|---|---|---|
| `repo` | string | Workspace registry name this flow state belongs to — the field `WorkflowStateDto` lacks, added because two repos can hold the same `spec_slug` and a flattened cross-repo list would otherwise be ambiguous |
| `spec_slug` / `branch` / `status` / `current_task` / `started_at` / `updated_at` / `run_id` | — | Same fields, same semantics, as Section 11.4's `WorkflowStateDto` |

Entries are ordered deterministically by `(repo, spec_slug)` — workspace names are sorted (the
`build_repo_summaries` precedent), and `collect_flow_states` already sorts each repo's entries by
`spec_slug`, so the composed ordering is diffable between polls with no extra sort step. A repo
with no `planning/` directory, an unresolvable root, or only malformed
`sdlc-flow-state.json` files contributes zero entries to this default response and does not fail
the request — the same degrade-gracefully behaviour Section 11.1's `GET /api/repos` and Section
11.4's per-repo route already have. That degradation is silent by design in the default response;
opt into `?with_skipped=1` below to observe it. An empty/absent `[workspaces]` registry returns
`200 []`.

**Request (`?with_skipped=1`, v0.24):**

```
GET /api/workflows?with_skipped=1 HTTP/1.1
Authorization: Bearer <token>
```

**Response (200 OK):** `WorkflowsAggregateDto` — a two-key object, never the bare array

```json
{
  "entries": [
    {
      "repo": "bastion",
      "spec_slug": "phase6-blockA",
      "branch": "phase6-blockA-flow",
      "status": "done",
      "current_task": 5,
      "started_at": "2026-06-25T18:30:59Z",
      "updated_at": "2026-06-25T19:02:33Z",
      "run_id": "9c6c6f1e-6d1a-4b3a-9b1a-1e2f3a4b5c6d"
    }
  ],
  "skipped": [
    { "repo": "amistad", "reason": "unreadable_root" },
    { "repo": "bella", "reason": "malformed_flow_state" },
    { "repo": "mev", "reason": "no_planning_dir" }
  ]
}
```

| Field | Type | Description |
|---|---|---|
| `entries` | `RepoWorkflowStateDto[]` | Identical array, identical ordering, to the default (no-param) response |
| `skipped` | `SkippedWorkspaceDto[]` | One entry per registered workspace whose flow-state report is incomplete, ordered by `repo` (the same sorted-registry order `entries` derives from — no extra sort step) |

`SkippedWorkspaceDto` fields:

| Field | Type | Description |
|---|---|---|
| `repo` | string | The registered workspace name whose report is incomplete |
| `reason` | string | One of `"unreadable_root"`, `"no_planning_dir"`, `"malformed_flow_state"` — see below |

The three `reason` values, checked in this order (first match wins — a repo yields at most one
`skipped` entry):

1. **`unreadable_root`** — the registered path is not a readable directory.
2. **`no_planning_dir`** — the root is readable but `{root}/planning` is not a readable directory.
3. **`malformed_flow_state`** — at least one `planning/*/sdlc/sdlc-flow-state.json` was found and
   failed to parse.

Two rules that follow directly from that vocabulary, and that a consumer must not get wrong:

- **A readable `planning/` directory with zero flow-state files is healthy, not skipped.** "No
  runs yet" and "unreachable" are different states; conflating them by omission is the exact defect
  this report exists to end. Such a repo appears in neither `entries` nor `skipped`.
- **A `malformed_flow_state` repo still contributes whatever parsed.** `skipped` means "this repo's
  report is incomplete," never "this repo contributed nothing" — every flow state that parsed
  successfully still appears in `entries` alongside the repo's `skipped` entry.

`?with_skipped=1` performs no additional filesystem traversal relative to the default path. Skip
detection is bookkeeping folded into the single walk `collect_flow_states` already performs per
workspace — classification happens as a side effect of that walk, not a second pass over it. The
query param exists **only** to keep the default (no-param) response non-breaking for existing
consumers (bastion-web's `lib/workflows.ts:218` reads a bare `RepoWorkflowStateDto[]` today); unlike
`?with_repo=1` (Section 14.1) or `?graph=1` (Section 13), it is not a cost gate — there is no
expensive second traversal being avoided, since the walk runs either way.

The route sits under the same bearer-auth `/api` scope as the rest of this section, with or without
`?with_skipped=1`; a request without a valid token gets `401` before reaching the handler. Section
11.4's per-repo `GET /api/repos/{name}/workflows` route is unchanged by either the v0.20 or v0.24
addition — it remains the endpoint to use when only one repo's flow states are needed.

---

## 12. Quick-action command API (v0.4)

One route projecting `ask`'s spawn/readiness mechanics
(`src/sessions/ask.rs`) onto a single one-tap HTTP call: inject a command into
an existing session, or spawn a fresh Claude session and send it a command
once ready.  Lives under the bearer-protected `/api` scope.

### 12.1 `POST /api/actions/command` — inject or spawn a quick-action command

**Request body:** `CommandRequest`

| Field | Type | Required | Description |
|---|---|---|---|
| `mode` | string | Yes | `"inject"` or `"spawn"` |
| `session` | string | Required when `mode:"inject"` | Existing tmux session name to target. Empty string counts as missing. |
| `name` | string | Required when `mode:"spawn"` | Name of the tmux session to create. Empty string counts as missing. |
| `dir` | string | No | Starting directory for a spawned session; omitted from the wire object when absent. |
| `model` | string | No | Claude model for a spawned session; one of `"opus"` \| `"sonnet"`. Defaults to `"sonnet"` when omitted. Only meaningful for `mode:"spawn"`. |
| `command` | string | Yes | The slash command (or literal text) sent once the target session is ready. |

**Inject request:**

```
POST /api/actions/command HTTP/1.1
Authorization: Bearer <token>
Content-Type: application/json

{ "mode": "inject", "session": "main", "command": "/status" }
```

**Spawn request:**

```
POST /api/actions/command HTTP/1.1
Authorization: Bearer <token>
Content-Type: application/json

{ "mode": "spawn", "name": "work", "dir": "/repo", "model": "opus", "command": "/status" }
```

**Response (200 OK):** `CommandResponse`

```json
{ "session": "work" }
```

| Field | Type | Description |
|---|---|---|
| `session` | string | The target tmux session id — the existing session for `inject`, the newly created session for `spawn` |

### 12.2 Dispatch behaviour

- `mode:"inject"` sends `command` as literal keystrokes (tmux `send-keys -l --`,
  followed by `Enter`) into the existing `session`.
- `mode:"spawn"` ensures a session named `name` exists (creating it via
  `tmux::new_session` when absent, in `dir` when given), launches
  `claude --model <model> --permission-mode bypassPermissions`, waits for
  readiness using `ask`'s readiness mechanics (`ensure_session_with_claude`,
  `src/sessions/ask.rs`), then sends `command` the same way as `inject`.

### 12.3 Error responses

A malformed request body — non-JSON payload, or JSON that fails to deserialize
into `CommandRequest` (e.g. a wrong-typed field) — is caught by the server's
`web::JsonConfig` error handler **before** the handler body runs, and returns
`400` with the `ErrorPayload` shape, code `C006`. This applies to every `POST`
route that deserializes a JSON body (not just this one), and is distinct from
the handler-level `mode`/field validation below (both use `C006`, but the
`JsonConfig` path never reaches the handler's own validation logic):

```json
{ "code": "C006", "message": "Json deserialize error: ..." }
```

Validation failures (bad `mode`/field combination) are checked before any I/O
and return `400` with the `ErrorPayload` shape (Section 10.4):

| Condition | HTTP status | `code` |
|---|---|---|
| `mode:"inject"` without a non-empty `session` | `400 Bad Request` | `C006` |
| `mode:"spawn"` without a non-empty `name` | `400 Bad Request` | `C006` |
| `model` present but not `"opus"`/`"sonnet"` | `400 Bad Request` | `C006` |

Execution-path failures (after validation passes) map as follows:

| Condition | HTTP status | `code` |
|---|---|---|
| `inject` targets an unknown/missing tmux session | `404 Not Found` | `C002` |
| tmux binary not installed / no tmux server running | `503 Service Unavailable` | `C001` |
| Other tmux exit error | `500 Internal Server Error` | `C010` |
| Spawn target directory is untrusted (Claude Code trust prompt) | `400 Bad Request` | `C006` |
| Spawned Claude fails to reach a ready state before the readiness timeout | `504 Gateway Timeout` | `C007` |
| Unexpected server/thread-pool error | `500 Internal Server Error` | `C010` |

**Example 400 body (bad mode/field combination):**

```json
{ "code": "C006", "message": "mode:\"inject\" requires a non-empty \"session\" field" }
```

**Example 504 body (spawn readiness timeout):**

```json
{ "code": "C007", "message": "timed out waiting for claude to become ready in session \"work\" after 30s" }
```

---

## 13. Cross-brain board API (v0.6, BA.11.K; enriched v0.11, BA.11.R; last_touched v0.14, BA.11.S; block-graph enrichment v0.19, A5; block detail fields v0.33, `BA.ticket.block-fields-serve-dto`; effective_priority v0.34, `BA.19.B`)

One read-only route projecting the cross-brain now/next/blocked/finished rollup — the same
aggregate `bastion emit-state` / `bastion validate-brain --state` already compute from the
mev/okf-core brain walk — onto HTTP. Lives under the bearer-protected `/api` scope. This route
never mutates any tier's or repo's `state.json` (D25 — bastion is a read-only surface over the
brain).

### 13.1 `GET /api/board` — cross-brain now/next/blocked/finished board

**Query parameters:**

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `scope` | string | No | `"hq"` | One of `"hq"` \| `"tier"` \| `"project"` \| `"business"` \| `"epic"` (Section 13.2). An unrecognized value fails query deserialization and returns `400` (Section 13.4). |
| `tier` | string | No | `"core"` | Tier name; only consulted when `scope` is `"tier"` or `"project"` (Section 13.2). Ignored for `"hq"`/`"business"`/`"epic"`. |
| `epic` | string | Only for `scope=epic` | — | Epic slug (from the HQ `epics[]` registry, Section 17) to filter the board to. Required, and only consulted, when `scope=epic`; missing or unknown → `404`/`C005` (Section 13.4). Ignored for every other scope. |
| `graph` (v0.19; widened v0.34) | boolean | No | `false` | Opt-in gate for the A5 `dependent_count`/`ready`/`unmet_count` enrichment on every `BoardBlockDto` (Section 13.3), and, as of v0.34, `effective_priority` too. When `false`/absent, `assemble_board` skips the `mev::build_block_graph_export` call entirely and all four fields are omitted from the JSON body. When `true` (`?graph=1` or `?graph=true`), the export is computed once per request (task 1 measured this as roughly doubling `/api/board`'s wall-clock on the live HQ corpus — see the block's Notes) and the four fields are populated on every lane entry present in the export. |

**Request:**

```
GET /api/board?scope=hq HTTP/1.1
Authorization: Bearer <token>
```

```
GET /api/board?scope=epic&epic=bastion-surfaces HTTP/1.1
Authorization: Bearer <token>
```

### 13.2 Scope semantics

| `scope` | Resolved walk scope | `tier` param | `epic` param | `BoardDto.tier` |
|---|---|---|---|---|
| `"hq"` (default) | `TierScope::All` — whole-brain aggregate | Ignored | Ignored | `null` |
| `"tier"` | `TierScope::Tier(<tier>)` — that tier's aggregate board | Optional, default `"core"` | Ignored | Resolved tier name |
| `"project"` | `TierScope::Tier(<tier>)`, same walk as `"tier"` — the client renders each project's board from `repos[]` | Optional, default `"core"` | Ignored | Resolved tier name |
| `"business"` | `TierScope::Tier("business")` — shortcut, ignores `tier` param | Ignored | Ignored | `"business"` |
| `"epic"` | `TierScope::All`, then pruned to blocks tagged with `epic` (Section 13.3's `epics` field) across every repo | Ignored | **Required** — missing or unknown slug → `404`/`C005` | `null` |

An empty-string `tier` param (`?tier=`) is treated the same as an absent one — it falls back to the
`"core"` default. An unknown `tier` name (no matching tier in `brain.toml`) is **not** an error: the
brain walk simply finds no in-scope repos for that tier, and the response comes back with empty
lanes and `repos: []` rather than a 4xx/5xx.

For `scope=epic`, the filter is applied to both the aggregate `lanes` and every entry in `repos[]`;
a repo contributing no member block for that epic is omitted from `repos[]` entirely (rather than
appearing with four empty lanes).

**Future refinement (not implemented in BA.11.K):** a context-aware default for `tier` — deriving
the "current" tier from the serving repo's own location in the brain tree instead of the
hardcoded `"core"` fallback. Tracked as a follow-up, not part of this contract.

### 13.3 Response (200 OK): `BoardDto`

```json
{
  "scope": "hq",
  "tier": null,
  "lanes": {
    "now": [
      { "id": "BA.11.K", "title": "Cross-brain board read endpoint", "repo": "bastion", "status": "in_progress", "blocked_by": [], "epics": ["bastion-surfaces"], "wave": 3, "priority": 1, "due": "2026-07-15", "track": "Phase 11" }
    ],
    "next": [],
    "blocked": [],
    "finished": [
      { "id": "BA.11.D", "title": "Repo status REST surface", "repo": "bastion", "status": "closed", "blocked_by": [], "epics": [], "wave": null, "priority": null, "due": null, "track": null }
    ]
  },
  "repos": [],
  "stale": false
}
```

| Field | Type | Description |
|---|---|---|
| `scope` | string | Echoes the resolved `scope` (`"hq"`, `"tier"`, `"project"`, `"business"`, or `"epic"`). |
| `tier` | string \| null | The resolved tier name for tier-scoped responses (`"tier"`/`"project"`/`"business"`); `null` for `"hq"`/`"epic"`. |
| `lanes` | `BoardLaneDto` | Aggregate now/next/blocked/finished lanes across every in-scope repo (for `scope=epic`, pruned to that epic's member blocks). |
| `repos` | array of `RepoBoardDto` | Per-project lane breakdown for every in-scope repo (populated for all scopes — the client picks whether to render the aggregate `lanes` or the per-project `repos[]` breakdown). For `scope=epic`, a repo contributing no member block is omitted. |
| `stale` | boolean | `true` when any in-scope repo's `planning/status.md` cache lags its `state.json`, per `mev::brain::sync::check_sync`. |

#### `BoardLaneDto`

| Field | Type | Description |
|---|---|---|
| `now` | array of `BoardBlockDto` | Blocks currently in progress. |
| `next` | array of `BoardBlockDto` | Blocks queued next (ordered). |
| `blocked` | array of `BoardBlockDto` | Blocks waiting on something; each entry's `blocked_by` is populated. |
| `deferred` | array of `BoardBlockDto` | Blocks deliberately parked on the back burner (authored `status == "deferred"`). Real roadmap work that is not being surfaced as next. **Never overlaps `next`** — a deferred block is structurally excluded from ready-order — and **never overlaps `blocked`**, even when it carries unmet deps (deferral is a terminal lane assignment). `blocked_by` *is* populated, since a deferred block can still have real unmet deps worth showing in a detail view. Absent/`[]` for repos that defer nothing. |
| `finished` | array of `BoardBlockDto` | Blocks whose `status == "closed"` — the terminal value in `mev::brain::state`'s `VALID_TRACK_BLOCK_STATUSES` (`open`/`in_progress`/`deferred`/`closed`). |

#### `BoardBlockDto`

| Field | Type | Default | Description |
|---|---|---|---|
| `id` | string | — | Canonical block ID (e.g. `"BA.11.K"`). |
| `title` | string | — | Block title, looked up from the owning repo's `tracks[].blocks[]`. |
| `repo` | string | — | Owning repo slug. |
| `status` | string \| null | `null` | Authored lifecycle status from `tracks[].blocks[].status` (`"open"`/`"in_progress"`/`"deferred"`/`"closed"`), on **every** lane — `now`/`next`/`blocked`/`deferred`/`finished` alike. `null` means the block's id has no `tracks[].blocks[]` match, not that its status is unknown-but-real. (Corrected 2026-07-30 — see the Amendment Log; before this, `now`/`next`/`blocked`/`deferred` reported a lane-fabricated placeholder instead of the authored value.) |
| `blocked_by` | array | `[]` | What this block is waiting on. **Populated on all four lanes as of v0.11/BA.11.R** — earlier versions of this contract populated it for `blocked`-lane entries only; it is now the unmet-dependency set (any `BlockedBy::External`, or a `BlockedBy::Block{repo, id}` whose target's authored status is not `"closed"`) computed the same way for `now`/`next`/`finished` as it always was for `blocked`. A block with no unmet dependency comes back `blocked_by: []`. |
| `epics` (v0.11) | array of string | `[]` | Cross-repo epic membership — slugs into the HQ `epics[]` registry (Section 17). Joined back from the owning repo's `tracks[].blocks[]`; a block whose id has no `tracks[]` match keeps `[]`. |
| `wave` (v0.11) | number \| null | `null` | Execution-order rank ("what's next"), from the authoring `TrackBlock.wave`. Typed to mirror `okf_core::TrackBlock.wave` (`Option<i64>`) — see the Amendment Log deviation note below. |
| `priority` (v0.11) | number \| null | `null` | Execution priority (e.g. `1`, `2`, `3`), from the authoring `TrackBlock.priority`. |
| `due` (v0.11) | string \| null | `null` | Target due date or timing string (e.g. `"2026-07-15"`), from the authoring `TrackBlock.due`. |
| `track` (v0.11) | string \| null | `null` | Title of the enclosing `tracks[]` phase/wave entry (`okf_core::Track.title`). |
| `last_touched` (v0.14) | string \| absent | absent | mev's derived per-block SDLC recency (`MV.10.D`, `mev::brain::last_touched::derive_last_touched`) carried verbatim — the newest `updated_at` across the block's spec-folder `sdlc/sdlc-{flow,task,run,}-state.json` files. Bastion performs **zero derivation** of its own. **Absence means "never worked" — a block with no resolvable SDLC run — not "worked long ago"**; no sentinel date or epoch is ever substituted, and `state.json.updated` is explicitly never substituted either. Unlike the v0.11 fields above (which serialize as `null` when unknown), this field is **omitted from the JSON body entirely** when unknown (`#[serde(skip_serializing_if = "Option::is_none")]`) — a deliberate divergence from the v0.11 sibling fields' `null` convention. |
| `dependent_count` (v0.19) | number \| absent | absent | Number of other blocks (corpus-wide, across every repo) that declare this block as a dependency, carried verbatim from `mev::brain::block_graph::BlockGraphNode::dependent_count`. **Corpus-wide, not scope-filtered** — mev computes it once over the full unscoped corpus before any `scope=`/`tier=`/`epic=` filtering is applied, so the value for a given block is identical whether the board is fetched at `scope=hq` or a narrower tier/project scope (the property client-side re-derivation from a scoped lane projection cannot have). Populated on **all five lanes** (`now`/`next`/`blocked`/`deferred`/`finished`). Omitted entirely (not `null`, not `0`) when `?graph=1` was not requested, or when the block is present on the board but absent from the graph export (e.g. `max_nodes`-truncated, or filtered out of the export's scope). |
| `ready` (v0.19) | boolean \| absent | absent | Membership in mev's `ready_order` set, carried verbatim from `BlockGraphNode::ready`. **This is the readiness signal consumers should use** — see the "readiness signal" note below. Populated on all five lanes under the same `?graph=1` gate and absence rule as `dependent_count`. |
| `unmet_count` (v0.19) | number \| absent | absent | Count of unmet dependencies, carried verbatim from `BlockGraphNode::unmet_count`, but populated **only for `blocked`-lane entries**. Absent (not `null`, not `0`) for every `now`/`next`/`deferred`/`finished` entry, and also absent when `?graph=1` was not requested or the block is absent from the export — see the "readiness signal" note below for why this field must never be read as a bare zero-check. |
| `effective_priority` (v0.34) | number \| absent | absent | Min-propagated ranking priority, carried verbatim from `mev::brain::block_graph::BlockGraphNode::effective_priority` — bastion performs **zero derivation** of its own. Populated on **all five lanes** under the same `?graph=1` gate as `dependent_count`/`ready`. Absent has three distinct causes, none distinguishable from the others in the payload: the block was absent from the graph export, `?graph=1` was not requested, or mev's own min-propagation never landed a value in the real `0..=3` range for that block. Never falls back to the block's own authored `priority` field — that fallback, when a client wants one, is bastion-web's own (`board-view.ts`'s `resolveEffectivePriority`), not bastion's. |
| `description` (v0.33) | string \| absent | absent | Longer human-facing description, from the authoring `okf_core::TrackBlock.description`. Absent (not `null`) when unauthored — the overwhelming majority of blocks until the D65 backfill populates it, so an untagged block's payload is byte-identical to pre-v0.33. |
| `created` (v0.33) | string (`YYYY-MM-DD`) \| absent | absent | Authoring date, from `okf_core::TrackBlock.created` — when this block record was written. Absent when unauthored. |
| `closed` (v0.33) | string (`YYYY-MM-DD`) \| absent | absent | Status-transition date to `closed`, from `okf_core::TrackBlock.closed`. Paired with `commit`. Absent when unauthored, including for every non-closed block. |
| `commit` (v0.33) | string \| absent | absent | Git hash of the commit that closed this block, from `okf_core::TrackBlock.commit`. Absent when unauthored. |
| `origin` (v0.33) | `BlockOriginDto` \| absent | absent | Backlog/carryover promotion provenance, from `okf_core::TrackBlock.origin`. `{ "kind": "backlog" \| "carryover", "slug": "<origin-node-slug>" }` — the `"carryover"` kind is D65's marker that this block was filed to resolve a `carryover[]` entry, letting a Surface link back to it. Absent when the block has no recorded origin. |

All five v0.11 fields are **additive and optional** — a JSON body written by the pre-v0.11 DTO
shape (no `epics`/`wave`/`priority`/`due`/`track` keys) still deserializes into the current
`BoardBlockDto`, so `bastion-ui` and the TUI, which do not read these fields, keep working
unchanged against either shape. A lane entry whose id has no match in its repo's `tracks[].blocks[]`
serializes with `epics: []` and the four `Option` fields absent/`null` rather than panicking or
being dropped from the lane.

`last_touched` (v0.14) is likewise **additive and optional** — a JSON body written before this
block (no `last_touched` key) still deserializes into the current `BoardBlockDto`, yielding `None`.
It is computed exactly once per request, inside `assemble_board`, by the same call to
`mev::brain::last_touched::derive_last_touched` that populates `mev::build_block_graph_export`'s
node field of the same name (Section 23's one-derivation contract extends to this field too — the
two read paths are cross-checked to agree, including both being absent for the same block).

`description`/`created`/`closed`/`commit`/`origin` (v0.33) are likewise **additive and optional**
— all five carry `skip_serializing_if = "Option::is_none"`, so a JSON body written before this
block (no matching keys) still deserializes into the current `BoardBlockDto`, and a block carrying
none of them (still the common case pending the D65 description backfill) serializes a payload
byte-identical to pre-v0.33. All five are populated by `enrich_block` (Section 13's `board.rs`)
and its epic-board sibling (`handlers/epics.rs`) directly from the owning repo's authored
`okf_core::TrackBlock`, with zero derivation — `description`/`created`/`closed`/`commit` are
carried verbatim as `Option<String>`, and `origin` is carried by mapping `okf_core::state::Origin`
onto the local `BlockOriginDto` mirror (`src/serve/dto.rs`, declared just below `BoardBlockDto`)
field-for-field.

#### `dependent_count` / `ready` / `unmet_count` / `effective_priority` (v0.19 + v0.34, A5) — readiness signal

These four fields are **additive and optional**, gated behind `?graph=1` (Section 13.1): a
pre-v0.19 client (or a v0.19+ request made without `?graph=1`) still deserializes the response
correctly, with all four fields absent. When `?graph=1` **is** requested, `assemble_board` calls
`mev::build_block_graph_export` **at most once** for the whole request (an unscoped export, per
the one-derivation contract Section 23 already established for `last_touched`) and threads the
result into every lane the same way `last_touched` is threaded — bastion performs **zero
derivation** of its own; every value is carried verbatim from mev. `effective_priority` rides the
same single export call as the other three; it does not trigger a second `mev` computation.

**`ready` — not `unmet_count == 0` — is the readiness signal.** mev defines `unmet_count` as `0`
for every lane other than `blocked` (`BlockGraphNode::unmet_count`'s own doc comment: *"`0` for
every other lane"*), so a client that reads `unmet_count == 0` on a `now`/`next`/`deferred`/
`finished` entry as "this block is ready" is reading a structural artifact of the lane it's
already in, not a measurement — this is exactly the false-ready failure mode A5 exists to kill.
`unmet_count` is therefore only ever present on `blocked`-lane entries; every consumer MUST branch
on `ready` (present on all five lanes) for readiness, and treat `unmet_count` as detail-only
context for an already-`blocked` entry.

**`dependent_count` is corpus-wide, not scope-filtered** — mev computes it once over the full
corpus before any `scope=`/`tier=`/`epic=` filtering, so a test in `block_graph.rs` proves the
value for a given block is identical whether fetched at `scope=hq` or a narrower tier/project
scope. This is the property a client-side reverse-dependency count derived from a scoped lane
projection structurally cannot have, and is the entire justification for shipping this field from
the server rather than leaving it to bastion-web to re-derive.

#### `RepoBoardDto`

| Field | Type | Description |
|---|---|---|
| `repo` | string | Repo slug. |
| `tier` | string \| null | Tier classification when known (e.g. `"core"`, `"business"`). |
| `lanes` | `BoardLaneDto` | This repo's own four lanes. |

### 13.4 Error responses

| Condition | HTTP status | Body |
|---|---|---|
| Missing/invalid `Authorization` header | `401 Unauthorized` | JSON `ErrorPayload` (`{"error": "unauthorized", "code": "unauthorized"}`, Section 2.2) |
| Unrecognized `scope` value (fails `BoardScope` query deserialization) | `400 Bad Request` | Plain text — actix's default `web::Query` extractor failure. `GET /api/board` has **no** `QueryConfig` error handler installed (unlike the `web::JsonConfig` handler that gives `POST /api/actions/command` its JSON `C006` body, Section 12.3) — a bad `scope` query value returns actix's stock `text/plain` 400, not an `ErrorPayload`. |
| `scope=epic` with a missing/blank `&epic=` param | `404 Not Found` | JSON `ErrorPayload`, code `C005`, message naming the missing param (`"scope=epic requires a non-empty &epic=<slug> query param"`). |
| `scope=epic` with an `&epic=<slug>` value absent from the HQ `epics[]` registry | `404 Not Found` | JSON `ErrorPayload`, code `C005`, message naming the unknown slug (`"unknown epic: <slug>"`). One uniform "no such epic board" response shape for both `scope=epic` miss cases, matching Section 11's registry-miss convention. |
| Unresolvable brain root (no `brain.toml` walking up from the workspace root) or unparseable `brain.toml` | `500 Internal Server Error` | JSON `ErrorPayload`, code `C010` |
| `web::block` thread-pool failure | `500 Internal Server Error` | JSON `ErrorPayload`, code `C010` |

Individual malformed/unreadable `state.json` files under an otherwise-resolvable brain root are
skipped (degrade gracefully, matching `derive_rollup`'s own behavior) rather than failing the
whole request — only an unresolvable brain root is a hard error.

**Example 400 body (unknown scope, verified against the running handler):**

```
Query deserialize error: unknown variant `bogus`, expected one of `hq`, `tier`, `project`, `business`
```

(`Content-Type: text/plain; charset=utf-8` — not JSON.)

**Example 500 body (unresolvable brain root):**

```json
{ "code": "C010", "message": "could not resolve brain root from /some/workspace: <error detail>" }
```

---

## 14. Live run read API (v0.8, BA.11.M)

Two read-only routes projecting the embedded engine's in-memory `LiveStateStore` (`engine_serve::
live_state::LiveStateStore`, `../engine-rs/crates/engine-serve/src/live_state.rs`) onto HTTP, so a
remote client (bastion-web `BW.3.B` node drill-in) can read a run's current per-node state without
polling Postgres. Live under the bearer-protected `/api` scope (Section 2) — same auth as Sections
3–13, distinct from the Section 18 engine `X-API-Key` scheme.

`LiveStateStore` is a single instance shared between the mounted engine's `on_progress` writer and
these read handlers (`src/serve/mod.rs`): when the Section 18 engine mount is active, the engine
records every node transition into this store as the run executes, and these routes read the same
store. **When the engine is not mounted** (Section 18.1 — `DATABASE_URL` / `BASTION_ENGINE_API_KEY`
absent), the store still exists but stays empty for the lifetime of the process: `GET /api/runs`
returns `200 []` and `GET /api/runs/{id}` returns `404` for every id — the same graceful-degradation
posture as the rest of this contract, not an error.

**`GET /api/runs` and `GET /api/runs/{id}` are a read-only snapshot, not a transition-by-transition
run stream.** A client observes the current state only when it requests it, and no `engine-serve`
change is introduced here. There is still no fine-grained push of **individual node** transitions in
this API — that remains out of scope (Section 14.4). Bastion does, however, ship a **bearer-authed
`/ws` hub** (Section 4), carrying a `sessions`/`pane`/`runs` topic vocabulary (Section 6), plus two
pushes built on it: an `event{workflow_done}` push (`src/serve/poll.rs`'s `FlowWatcher::observe`)
that fires once when a flow transitions to a terminal status (`"done"` or `"blocked"`) —
**completion-only**, it tells a subscriber a run finished, not what happened along the way — and, as
of v0.23 (`BA.11.N`, D17), an `event{run_transition}` push (Section 8.3) that fires on every
**run-level aggregate status change**, not only on completion, for connections subscribed to the
`"runs"` topic. Node-level (per-`NodeTransitionDto`) granularity is still not pushed — only the
run's aggregate `status` (Section 8.3, Section 14.1's same derivation) is. `BW.3.A`'s ~2s client
polling against these two routes remains the retained fallback (D17 constraint 3), used whenever a
client is not subscribed to `runs` or `run_stream_status.available` is `false`.

**How `BA.11.N` resolved the wire-terminal/lifecycle-terminal gotcha:** the embedded engine's own
`publish_suspended` push sends `terminal: true` **together with** `status: "suspended"`
(`engine-rs/crates/engine-serve/src/stream.rs:185-191`) on its own protocol — wire-terminal there is
not lifecycle-terminal. Section 8.3's `run_transition.terminal` deliberately means the opposite:
lifecycle-terminal only (the run left `LiveStateStore`'s live map). A suspended run stays in the live
map, so it always emits `status: "suspended", terminal: false` on this stream — a client doing
`if (frame.terminal) done` against `run_transition` is safe from the engine's own gotcha by
construction.

### 14.1 `GET /api/runs` — currently-tracked run summaries (v0.16, BA.11.T; `suspended` status added v0.17; `repo` added v0.22)

**Request:**

```
GET /api/runs HTTP/1.1
Authorization: Bearer <token>
```

**Query parameters:**

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `with_repo` (v0.22) | boolean | No | `false` | Opt-in gate for the A7 `repo` enrichment (below), accepting `1`/`0` in addition to `true`/`false`. When `false`/absent, `list_runs` skips the registry walk (`collect_all_workflows`) **entirely** and `repo` is omitted from every entry. When `true` (`?with_repo=1`), the walk runs once per request and `repo` is resolved per run via an exact `run_id` match. This is an **enrichment flag, not a repo filter** — unlike `?repo=` elsewhere in this API (`BlockGraphQuery.repo`, Section 23.1; the documented "No `?repo=` filter" on `/api/costs`, Section 24), `?with_repo=1` never narrows which runs are returned, it only adds a field to each one. Mirrors `/api/board`'s `?graph=1` gate (Section 13.1, A5): task 1 measured the registry walk as ~6x the unenriched baseline against the live HQ registry (23 repos), so the route's hottest consumer (bastion-web's ~2-6s run rail) must not pay for it unless it opts in. |

**Response (200 OK): `Vec<RunSummaryDto>`** — one entry per run currently tracked by the shared
`LiveStateStore` (`list_active()`), each projected via `project_run_summary` (`src/serve/handlers/
runs.rs`). A run that races out of the store between `list_active()` and the per-id `get()` fetch is
silently dropped rather than erroring. `[]` when no run is tracked, including when the engine is not
mounted. A run that has gone terminal (evicted from the live map by `mark_terminal`) no longer
appears in this response.

```json
[
  {
    "run_id": "b6a1c1e0-0000-4000-8000-000000000000",
    "status": "running",
    "spec_slug": "11.T-run-summary-projection",
    "started_at": "2026-07-24T12:00:00Z",
    "updated_at": "2026-07-24T12:00:01Z"
  },
  {
    "run_id": "c7b2d2f1-0000-4000-8000-000000000000",
    "status": "pending",
    "started_at": null,
    "updated_at": null
  }
]
```

The second entry above shows the omitted-key cases: its triggering event carried no `spec_slug`, so
the field is absent (not `null`) from the JSON body, and it has no recorded node transitions yet, so
`started_at`/`updated_at` are explicit `null`. `workflow_type` is likewise absent from both entries —
see the field table below.

No query parameters. No 404 case — an empty store is a normal 200.

#### `RunSummaryDto`

| Field | Type | Description |
|---|---|---|
| `run_id` | string | The run's UUID as a string. |
| `workflow_type` | string \| absent | Workflow identity (e.g. `"sdlc-flow"`). **Always absent today** — no production code stamps a workflow-identity key anywhere `bastion` can read it from a live `TaskContext`; `engine-serve` only tracks it in a process-local, `pub(crate)`-scoped side table (`http.rs::live_run_metadata()`). Tracked by the engine-rs follow-up ticket `EN.ticket.expose-live-run-workflow-type` (`core/engine-rs/planning/ticket-expose-live-run-workflow-type/`); this DTO does not fabricate a value in the meantime. |
| `status` | string | Lowercase wire status, derived via `db::workflows::derive_run_status` over the run's current `node_runs` (mapped to minimal `NodeState`s) and `metadata`: `pending`/`running`/`success`/`failed`/`cancelled`/`budget_halted`/`suspended` (v0.17). `suspended` is not terminal — a resumed run's `metadata.suspension.suspended` flips back to `false` (the key itself is never deleted), so it falls through to the ordinary node-aggregate rules again. |
| `spec_slug` | string \| absent | The triggering event's `spec_slug` field, when present. Omitted (not `null`) when the run's event carries no `spec_slug` key. |
| `started_at` | string \| null | Earliest non-null `node_runs[*].started_at` across all tracked nodes, as RFC3339. `null` when the run has no recorded node transitions yet. |
| `updated_at` | string \| null | Latest non-null `node_runs[*].started_at` **or** `completed_at` across all tracked nodes, as RFC3339. `null` when the run has no recorded node transitions yet. |
| `repo` (v0.22) | string \| absent | The repo that owns this run, resolved by an **exact `run_id` match** against the registry's flow state (`RepoWorkflowStateDto` from `collect_all_workflows`, Section 11.6, A2). Absent (never `null`, never guessed via substring/prefix/spec-slug similarity) when no flow state carries this run's `run_id` — a wrong label is strictly worse than an absent one (A7). Also absent whenever the request omits `?with_repo=1`, regardless of whether a match would exist, since the registry walk that resolution requires does not run at all in that case. |

### 14.2 `GET /api/runs/{id}` — one run's per-node snapshot

**Path parameter:**

| Parameter | Type | Description |
|---|---|---|
| `id` | string (UUID) | The run id, as returned by `GET /api/runs` or minted by the Section 18 `/events/` trigger. Must parse as a UUID. |

**Request:**

```
GET /api/runs/b6a1c1e0-0000-4000-8000-000000000000 HTTP/1.1
Authorization: Bearer <token>
```

**Response (200 OK): `RunStateDto`** — the run's `TaskContext` snapshot projected to wire shape,
joining each tracked node's `node_runs[class]` (status/timing/error/input/usage) with its
`nodes[class]` (output) by class name, sorted by class name for deterministic output:

```json
{
  "run_id": "b6a1c1e0-0000-4000-8000-000000000000",
  "event": { "ticket_id": "T-1" },
  "metadata": { "workflow": "sdlc-flow" },
  "nodes": [
    {
      "node": "DataIngestionNode",
      "status": "success",
      "started_at": "2026-07-24T12:00:00Z",
      "completed_at": "2026-07-24T12:00:01Z",
      "error": null,
      "input": null,
      "output": { "documents_loaded": 3 },
      "usage": null
    },
    {
      "node": "SummarizeNode",
      "status": "failed",
      "started_at": "2026-07-24T12:00:01Z",
      "completed_at": "2026-07-24T12:00:02Z",
      "error": "timeout",
      "input": { "documents": 3 },
      "output": null,
      "usage": { "input_tokens": 512, "output_tokens": 128, "model": "claude-sonnet-5" }
    }
  ]
}
```

#### `RunStateDto`

| Field | Type | Description |
|---|---|---|
| `run_id` | string | The run's UUID, echoed back. |
| `event` | JSON value | The triggering event payload, carried through from `TaskContext::event`. |
| `metadata` | JSON value | Workflow-level metadata, carried through from `TaskContext::metadata`. |
| `nodes` | array of `NodeTransitionDto` | One entry per node class present in `TaskContext::node_runs`, sorted by class name. Empty when the run has no recorded node transitions yet. |

#### `NodeTransitionDto`

| Field | Type | Description |
|---|---|---|
| `node` | string | The node's class name — the map key in both `TaskContext::nodes` and `TaskContext::node_runs`. |
| `status` | string | Lowercase wire status: `"pending"` \| `"running"` \| `"success"` \| `"failed"`. |
| `started_at` | string \| null | ISO-8601 UTC timestamp set on entry; `null` while `pending`. |
| `completed_at` | string \| null | ISO-8601 UTC timestamp set on success or failure; `null` before completion. |
| `error` | string \| null | Error message; present only for a `failed` node. |
| `input` | JSON value \| null | The node's recorded input; present only for a `failed` node. |
| `output` | JSON value \| null | The node's output from `TaskContext::nodes`; `null` when not yet produced (e.g. still `running`, or `failed` before producing output). |
| `usage` | `RunUsageDto` \| null | Token/model usage; present only for LLM nodes, `null` for non-LLM nodes and for nodes that have not yet reported usage. |

#### `RunUsageDto`

| Field | Type | Description |
|---|---|---|
| `input_tokens` | number \| null | Prompt token count, when reported by the provider. |
| `output_tokens` | number \| null | Completion token count, when reported by the provider. |
| `model` | string | Model identifier used for this node's LLM call. |

### 14.3 Error responses

| Condition | HTTP status | Body |
|---|---|---|
| Missing/invalid `Authorization` header | `401 Unauthorized` | JSON `ErrorPayload` (`{"error": "unauthorized", "code": "unauthorized"}`, Section 2.2) |
| Malformed `{id}` (not a valid UUID) | `400 Bad Request` | JSON `ErrorPayload`, code `C006` |
| Unknown or no-longer-tracked run id | `404 Not Found` | JSON `ErrorPayload`, code `C002` |

`GET /api/runs` has no error case beyond the shared 401 — an empty or non-existent store is a
normal `200 []`.

### 14.4 Out of scope

No SSE/WS stream, no `engine-serve` broadcast/tee, and no Postgres history read are part of this
API — see the block-level scope note above. Token-by-token LLM output streaming and orchestrator-
workflow surfaces (`BA.11.G`) are likewise out of scope.

### 14.5 Testing

`project_run` (`src/serve/handlers/runs.rs`) is the pure `TaskContext` → `RunStateDto` projection —
exhaustively unit-tested with no I/O (multi-node mixed statuses, failed-node error+input, LLM-node
usage vs. non-LLM `None`, empty `node_runs`, output joined by class name). `project_run_summary`
(alongside its `node_states_from`/`run_status_str`/`spec_slug_from_event`/`run_timestamps` helpers,
same file) is the equivalent pure `TaskContext` → `RunSummaryDto` projection backing Section 14.1 —
likewise exhaustively unit-tested with no I/O against representative fixtures (spec_slug present/
absent, cancelled/budget-halted metadata cases). The async handlers (`list_runs`/`get_run`) and the
`LiveStateStore`-sharing wiring in `src/serve/mod.rs` are the thin I/O shell — covered by
`#[actix_web::test]` handler-level tests plus `src/serve/mod.rs` integration tests asserting the
bearer-auth 401, the empty-store `200 []`, the unknown-id `404`, and (BA.11.T) a terminal run's
absence from `GET /api/runs` against a real `App`, and manually smoke-tested end-to-end against a
running `bastion serve` with the engine mounted (recorded in `planning/11.M-live-run-read-endpoint/
tasks.md`'s `## Notes`).

---

## 15. Attention / carryover API (v0.9, BA.11.P)

One read-only route projecting the tier-scoped **Attention board** — the stale `carryover[]`,
aging `backlog[]`, and orphaned-capture lanes that `mev emit-state` splices into every brain-level
`status.md` and the local `/attention` slash command reads — onto HTTP, so BastionWeb (`BW.1.C`),
the TUI, and the phone can see the items that never resurface without reading a repo's
`status.md`. Lives under the bearer-protected `/api` scope (Section 2). Same read-only posture as
`GET /api/board` (Section 13, D25) — this route never mutates any tier's or repo's `state.json`,
and the dispositions `/attention` offers locally (promote · keep · snooze · resolve · archive) are
**writes** and stay CLI-only; this route never exposes them.

### 15.1 `GET /api/attention` — attention / carryover projection

**Query parameters** — identical semantics to `GET /api/board` (Section 13.2), so one scope switch
can drive both fetches:

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `scope` | string | No | `"hq"` | One of `"hq"` \| `"tier"` \| `"project"` \| `"business"` (Section 13.2). An unrecognized value fails query deserialization and returns `400` (Section 15.3). |
| `tier` | string | No | `"core"` | Tier name; only consulted when `scope` is `"tier"` or `"project"` (Section 13.2). Ignored for `"hq"`/`"business"`. An unknown tier is not an error — it yields empty lanes. |

**Request:**

```
GET /api/attention?scope=hq HTTP/1.1
Authorization: Bearer <token>
```

### 15.2 Scoping rule

Scope resolution is identical to `GET /api/board` — see Section 13.2 for the full
`scope` → `TierScope` table (including the empty-string-`tier`-falls-back-to-`"core"` and
unknown-tier-yields-empty-lanes rules). What differs is which items each resolved scope pulls in,
mirroring `mev::brain::emit::plan_attention_board`:

- **`hq`** (`TierScope::All`) — unions `carryover[]` from **every** loaded repo/tier file, plus the
  **whole** HQ `backlog[]`.
- **`tier`** / **`project`** (`TierScope::Tier(t)`) — unions `carryover[]` from that tier's leaf
  repos **plus the tier brain file itself**, and only those HQ `backlog[]` nodes whose `repo`
  belongs to tier `t`.
- **`business`** (`TierScope::Tier("business")`) — same rule as `tier`, ignoring a stray `&tier=`.

The HQ `backlog[]` is the one on the single `kind == "brain"` file whose scope is `TierScope::All`.
`backlog[]` nodes with `origin.type == "capture"` are never counted toward `aging_backlog` — they
are split into `orphaned_captures` instead, carrying `origin.notes` (falling back to the node's own
`notes`). A capture never appears in both lanes.

Item filtering is delegated to `mev::brain::state::carryover_stale_age` / `backlog_stale_age` — the
same predicates the `W_STATE_*_STALE` warnings use. `Some(age_days)` means the item belongs on the
board; `None` (under its per-`kind`/`backlog_days` threshold, snoozed via `today < snoozed_until`,
or lacking both a `created` and a `reviewed` anchor date) means the item is **absent** from the
response rather than present-and-flagged. Every lane is sorted **oldest-first** (descending
`age_days`). Item text is returned untruncated — the 60/80-char row clamp `mev`'s markdown renderer
applies is a rendering concern and stays in `mev`.

### 15.3 Response (200 OK): `AttentionDto`

```json
{
  "scope": "hq",
  "tier": null,
  "as_of": "2026-07-24",
  "lanes": {
    "stale_carryover": [
      {
        "repo": "bastion", "slug": "engine-mount-env", "kind": "env",
        "text": "engine routes need DATABASE_URL + BASTION_ENGINE_API_KEY set",
        "clears_when": "the engine mount is documented in .env.example",
        "created": "2026-07-01", "reviewed": null,
        "age_days": 23, "threshold_days": 3
      }
    ],
    "aging_backlog": [
      {
        "repo": "bastion", "slug": "serve-attention-endpoint", "title": "Attention projection",
        "kind": "feature", "status": "ready", "notes": null,
        "created": "2026-07-10", "reviewed": null,
        "age_days": 14, "threshold_days": 7
      }
    ],
    "orphaned_captures": []
  },
  "thresholds": {
    "env_days": 3, "deferred_days": 5, "known_issue_days": 10,
    "constraint_days": 10, "backlog_days": 7
  }
}
```

| Field | Type | Description |
|---|---|---|
| `scope` | string | Echoes the resolved `scope` (`"hq"`, `"tier"`, `"project"`, or `"business"`), reusing `BoardDto`'s `BoardScope`. |
| `tier` | string \| null | The resolved tier name for tier-scoped responses; `null` for `"hq"`. |
| `as_of` | string | `YYYY-MM-DD` the ages were computed against. |
| `lanes` | `AttentionLanesDto` | The three Attention lanes. |
| `thresholds` | `AttentionThresholdsDto` | The resolved `brain.toml` `[attention]` thresholds, so a client can render "Nd over" without reading `brain.toml`. |

#### `AttentionLanesDto`

| Field | Type | Description |
|---|---|---|
| `stale_carryover` | array of `AttentionCarryoverDto` | `carryover[]` entries past their per-`kind` threshold, oldest-first. |
| `aging_backlog` | array of `AttentionBacklogDto` | Non-capture `backlog[]` nodes (`status` `"idea"`/`"ready"`) past `backlog_days`, oldest-first. |
| `orphaned_captures` | array of `AttentionBacklogDto` | `backlog[]` nodes with `origin.type == "capture"` past `backlog_days`, oldest-first. Never duplicated into `aging_backlog`. |

#### `AttentionCarryoverDto`

| Field | Type | Description |
|---|---|---|
| `repo` | string | Owning repo slug. |
| `slug` | string | Stable item slug. |
| `kind` | string | Carryover kind (`"env"`, `"deferred"`, `"known_issue"`, `"constraint"`, or other). |
| `text` | string | The carryover text itself, untruncated. |
| `clears_when` | string \| null | What clears this item, when recorded — a rendered display string (`dto::render_clears_when`); the typed `okf_core::ClearsWhen` enum never crosses this boundary. Absent key when `None`. |
| `created` | string \| null | Creation date (`YYYY-MM-DD`), when recorded. |
| `reviewed` | string \| null | Last-reviewed date (`YYYY-MM-DD`), when recorded. |
| `age_days` | number \| absent (v0.25) | Days since `max(created, reviewed)`, as computed by `carryover_stale_age`. **Widened from a non-optional `number` to optional in v0.25** — absent when the entry is currently snoozed or has no parseable anchor date; such entries still reach the board (they are no longer excluded pre-ranking). |
| `threshold_days` | number | The per-`kind` threshold this item tripped (`AttentionThresholds::carryover_threshold`). |
| `lane` (v0.25) | string | The triage lane `mev::rank_carryover` assigned — one of `"blocking"` \| `"hot"` \| `"aging"` \| `"standing"` — reused verbatim from mev's `TriageLane`, never re-derived here. See `docs/carryover-contract.md`. |
| `priority` (v0.25) | number \| absent | Authored priority (`0`=hottest .. `3`=coldest), verbatim from `mev::CarryoverRanking`. Absent key when unauthored. |
| `effective_priority` (v0.25) | number \| absent | Priority after min-propagation across `blocks[]` edges, verbatim from `mev::CarryoverRanking`. Absent key when the entry has no own priority and no hotter transitive target. |
| `unmet_blocks` (v0.25) | array of string \| absent | Every unmet `blocks[]` target key. Non-empty iff this entry is in the BLOCKING lane. Absent key (not `[]`) when empty. There is deliberately no `blocking: bool` field — derive it from `!unmet_blocks.is_empty()`. |
| `finding_id` (v0.25) | string \| absent | Cross-repo finding identity, verbatim from `mev::CarryoverRanking`. Absent key when unauthored. |
| `clears_when_satisfied` (v0.25) | boolean | Whether the source verdict's evaluated `clears_when` references are currently satisfied, verbatim from `mev::CarryoverRanking`. Always present. |

**v0.25 behavioral change:** membership in this array no longer gates on `carryover_stale_age`
alone — the full carryover entry set is ranked by `mev::rank_carryover` (contract §2), and
`stale`/`age_days`/`threshold_days` are consulted only as inputs to lane assignment, not as a
pre-filter. A response that previously carried ~6 entries fleet-wide now carries ~138. Ordering is
mev's — this repo never re-sorts the returned vector.

#### `AttentionBacklogDto`

Used by both the `aging_backlog` and `orphaned_captures` lanes.

| Field | Type | Description |
|---|---|---|
| `repo` | string | Owning repo slug. |
| `slug` | string | Stable item slug. |
| `title` | string | Human-readable title. |
| `kind` | string | Backlog kind (serde-renamed from `type` on the domain type). |
| `status` | string | Lifecycle status (`"idea"` / `"ready"`, the only statuses that can age). |
| `notes` | string \| null | For `orphaned_captures` this is `origin.notes`, falling back to the node's own `notes`; for `aging_backlog` it is the node's own `notes`. |
| `created` | string \| null | Creation date (`YYYY-MM-DD`), when recorded. |
| `reviewed` | string \| null | Last-reviewed date (`YYYY-MM-DD`), when recorded. |
| `age_days` | number | Days since `max(created, reviewed)`, as computed by `backlog_stale_age`. |
| `threshold_days` | number | `AttentionThresholds::backlog_days`. |

#### `AttentionThresholdsDto`

| Field | Type | Description |
|---|---|---|
| `env_days` | number | Threshold (days) for `kind == "env"` carryover. Default `3`. |
| `deferred_days` | number | Threshold (days) for `kind == "deferred"` carryover. Default `5`. |
| `known_issue_days` | number | Threshold (days) for `kind == "known_issue"` carryover. Default `10`. |
| `constraint_days` | number | Threshold (days) for `kind == "constraint"` carryover. Default `10`. |
| `backlog_days` | number | Threshold (days) for aging/orphaned `backlog[]` nodes. Default `7`. |

Thresholds are read from `brain.toml`'s `[attention]` table (`mev::brain::config::AttentionThresholds`);
an absent table yields the defaults shown above.

### 15.4 Error responses

| Condition | HTTP status | Body |
|---|---|---|
| Missing/invalid `Authorization` header | `401 Unauthorized` | JSON `ErrorPayload` (`{"error": "unauthorized", "code": "unauthorized"}`, Section 2.2) |
| Unrecognized `scope` value (fails `BoardScope` query deserialization) | `400 Bad Request` | Plain text — actix's default `web::Query` extractor failure, same as `GET /api/board` (Section 13.4) — no `QueryConfig` error handler is installed for this route either. |
| Unresolvable brain root (no `brain.toml` walking up from the workspace root) or unparseable `brain.toml` | `500 Internal Server Error` | JSON `ErrorPayload`, code `C010` |
| `web::block` thread-pool failure | `500 Internal Server Error` | JSON `ErrorPayload`, code `C010` |

Individual malformed/unreadable `state.json` files under an otherwise-resolvable brain root are
skipped (degrade gracefully) rather than failing the whole request — only an unresolvable brain
root is a hard error.

### 15.5 Out of scope

- The **"what next" priority ranking** half of `BW.1.C` — not this route; `BW.1.C` ranks
  client-side from `BoardDto` + `age_days` in the interim.
- **Any mutation.** `/attention`'s promote · keep (`reviewed`) · snooze · resolve · archive
  dispositions stay CLI-only — `serve` is read-only over the brain (D25).
- Any change to `mev` or `okf-core`; the markdown Attention board (`mev` keeps owning `status.md`
  splicing); SSE/streaming of anything.

### 15.6 Testing

`tier_of_repo` and `build_attention` (`src/serve/handlers/attention.rs`) are the pure, I/O-free
projection — exhaustively unit-tested with `today` threaded in as a parameter (scope resolution
including the business/empty-tier cases; tier lookup hit/miss; hq vs. tier carryover union; HQ
`backlog[]` tier filtering; the capture-vs-backlog lane split including the `origin.notes` →
`notes` fallback; the snoozed/under-threshold/no-anchor absences; oldest-first ordering; per-`kind`
`threshold_days`). The thin async handler (`get_attention`) and route wiring in `src/serve/mod.rs`
are covered by `#[actix_web::test]` integration tests against a temp brain-root fixture, asserting
the bearer-auth `401`, the `200` + three-lane body, the unknown-tier `200` + empty lanes, and the
unrecognized-scope `400` — and manually smoke-tested end-to-end against a running `bastion serve`
(recorded in `planning/11.P-attention-read-endpoint/tasks.md`'s `## Notes`).

---

## 16. Docs read API (v0.10, BA.11.Q)

Two read-only routes for browsing and reading brain markdown across repos: a file tree and a raw-file
read, both **allowlisted and traversal-rejecting**. The allowlist is a security boundary and lives
here, in the process that owns the filesystem — never in a client or BFF. Read-only (D25): no route
under `/api/docs` writes, creates, or deletes anything.

The `{repo}` path segment is a name from the **existing `[workspaces]` registry** (`src/config.rs`) —
the same registry `GET /api/repos` (Section 11.1) lists and the CLI's `--workspace` flag uses. A
client populates its repo switcher from `GET /api/repos`; there is no separate docs-only naming
scheme.

### 16.1 `GET /api/docs/{repo}/tree` — allowlisted markdown tree

| Parameter | In | Required | Default | Description |
|---|---|---|---|---|
| `repo` | path | Yes | — | Workspace registry name (e.g. `brain`, `bastion`). Unknown → `404`/`C005`. |
| `path` | query | No | all allowlisted roots | Relative directory to scope the listing to. Must satisfy the path rules in 16.3. |

```
GET /api/docs/bastion/tree?path=planning HTTP/1.1
Authorization: Bearer <token>
```

**Response (200 OK): `DocTreeDto`**

```json
{
  "repo": "bastion",
  "root": "planning",
  "entries": [
    { "path": "planning/status.md", "name": "status.md", "is_dir": false },
    { "path": "planning/decisions", "name": "decisions", "is_dir": true }
  ]
}
```

| Field | Type | Description |
|---|---|---|
| `repo` | string | Echoes the resolved registry name. |
| `root` | string | The allowlisted root (or subtree) the listing is relative to; `""` when the whole allowlist was walked. |
| `entries` | array of `DocEntryDto` | `{ path, name, is_dir }` — `path` is repo-root-relative and is exactly what `16.2`'s `?path=` accepts. Sorted directories-first, then by `name`. |

Only markdown files appear (`.md`/`.mdx`); directories that contain no markdown at any depth are
omitted. The walk hides the same directories the corpus crawl does — `brain.toml`'s `[crawl].skip_dirs`
plus any nested-`.git` subtree — via `mev::brain::crawl::crawl_brain`.

### 16.2 `GET /api/docs/{repo}/file` — raw markdown content

| Parameter | In | Required | Description |
|---|---|---|---|
| `repo` | path | Yes | Workspace registry name. |
| `path` | query | Yes | Repo-root-relative file path. Must satisfy the path rules in 16.3. Absent `path` maps to the same `403`/`C003` response as any other rejected path. |

```
GET /api/docs/bastion/file?path=planning/status.md HTTP/1.1
Authorization: Bearer <token>
```

**Response (200 OK): `DocFileDto`**

```json
{
  "repo": "bastion",
  "path": "planning/status.md",
  "content": "---\ntype: Status\n---\n\n# bastion — Status\n…",
  "bytes": 8421,
  "modified": "2026-07-24T18:03:11Z"
}
```

| Field | Type | Description |
|---|---|---|
| `repo` / `path` | string | Echo the resolved repo and the validated relative path. |
| `content` | string | The file's **raw** markdown, byte-for-byte, no rendering, no frontmatter stripping, no sentinel removal. Rendering is the client's job. |
| `bytes` | integer | Content length in bytes. |
| `modified` | string \| null | Filesystem mtime, RFC 3339 UTC; `null` when unavailable. |

### 16.3 Path rules (the security contract)

A `path` is accepted only when **all** of the following hold. The first four are checked **before any
filesystem access**:

1. It is **relative** — no leading `/`, no drive prefix.
2. It contains **no `..`** component (and no root/prefix component anywhere).
3. It contains **no NUL byte and no backslash**.
4. For `16.2`, its extension is **`.md` or `.mdx`**.
5. After joining and `canonicalize()`, the result is **contained within a canonicalized allowlisted
   root** — which also rejects a symlink *inside* the tree that points out of it.

**Allowlisted roots**, per repo: `docs/`, `planning/`, `business/`, plus repo-root-level `*.md` files
(`README.md`, `CLAUDE.md`). Source code, dotfiles, and non-markdown data are never served — the
extension rule means a `.env`, a key, or a `state.json` is unreadable through this route even from
inside an allowed root.

> **Note on `planning/`:** in every repo `planning/` is a **symlink into the company-brain vault**
> (`core/_planning/<repo>/`). Containment is therefore checked against each *allowlisted root*
> canonicalized independently — not against a canonicalized repo root, which would reject every
> legitimate `planning/` read.

### 16.4 Error responses

| Condition | HTTP status | Body |
|---|---|---|
| Missing/invalid `Authorization` header | `401 Unauthorized` | JSON `ErrorPayload` (Section 2.2) |
| Path fails any rule in 16.3 — traversal, disallowed extension, a missing required `?path=` on the file route, or outside every allowlisted root | `403 Forbidden` | JSON `ErrorPayload`, code `C003`. **One uniform response for all four** — the API never discloses whether a path outside the allowlist exists. |
| Unknown `{repo}` (not in the `[workspaces]` registry) | `404 Not Found` | JSON `ErrorPayload`, code `C005` (matches Section 11's convention) |
| File absent inside an allowlisted root | `404 Not Found` | JSON `ErrorPayload`, code `C002` |
| File is not valid UTF-8 | `500 Internal Server Error` | JSON `ErrorPayload`, code `C014` |
| Other read failure | `500 Internal Server Error` | JSON `ErrorPayload`, code `C009` |
| `web::block` thread-pool failure | `500 Internal Server Error` | JSON `ErrorPayload`, code `C010` |

### 16.5 Out of scope

Writes of any kind; rendering, wikilink resolution, and link rewriting (all client-side, `BW.2.A`);
frontmatter/title extraction on tree entries; a configurable allowlist; non-markdown assets (images,
PDFs); search (that is the retrieval path, not this one); caching/ETags.

### 16.6 Testing

`validate_rel_path` and `resolve_allowlisted_path` (`src/serve/docs.rs`) are the pure/allowlist
security core — exhaustively unit-tested with no I/O for the pure rejection vectors (`../x`,
`a/../../b`, `/etc/passwd`, a Windows-style `C:\…`, an embedded NUL, a backslash, an empty path,
`foo.md/../../.env`, a bare `.env`) and, for `resolve_allowlisted_path`, against a real temp-dir
symlink fixture asserting both that a symlinked allowlist root (mirroring `planning/`) is accepted
and that a symlink inside a root pointing outside it is rejected. The thin `web::block` shells
(`get_docs_tree` / `get_docs_file`, `src/serve/handlers/docs.rs`) and route wiring in
`src/serve/mod.rs` are covered by `#[actix_web::test]` integration tests against a temp workspace
fixture with a real symlinked `planning/` dir, asserting the bearer-auth `401`, a `200` tree listing
that excludes non-markdown files, a `200` raw read through the symlinked root, the `403`/`C003`
uniform rejection across traversal / bad-extension / absolute-path vectors, and the distinct
`404`/`C005` (unknown repo) vs. `404`/`C002` (missing file) cases.

---

## 17. Epics registry API (v0.11, BA.11.R; `weight` v0.21, `BA.ticket.epic-weight-dto`)

One read-only route projecting the HQ `epics[]` cross-repo initiative registry onto HTTP. Lives
under the bearer-protected `/api` scope (Section 2). This route never mutates any tier's or repo's
`state.json` (D25 — bastion is a read-only surface over the brain). The same registry backs
`GET /api/board`'s `scope=epic&epic=<slug>` projection (Section 13.2/13.3) — this route exposes the
registry itself so a client can list available epics before filtering the board to one.

### 17.1 `GET /api/epics` — HQ cross-repo initiative registry

**Query parameters:** none.

**Request:**

```
GET /api/epics HTTP/1.1
Authorization: Bearer <token>
```

### 17.2 Response (200 OK): `EpicDto[]`

```json
[
  {
    "slug": "bastion-surfaces",
    "title": "Bastion Surfaces",
    "description": "Cross-repo surfaces initiative",
    "status": "active",
    "weight": 85,
    "plan": "core/planning/master-plan.md",
    "repos": ["bastion", "bastion-ui"],
    "closed": 4,
    "in_progress": 1,
    "open": 7,
    "deferred": 0,
    "total": 12,
    "fully_deferred": false
  }
]
```

| Field | Type | Default | Description |
|---|---|---|---|
| `slug` | string | — | Stable kebab-case key — the value blocks reference in their `epics[]`. |
| `title` | string | — | Human-readable name (e.g. `"Bastion Surfaces"`). |
| `description` | string \| null | `null` | One-line description of what the initiative covers. |
| `status` | string \| null | `null` | Lifecycle: `"active"` · `"paused"` · `"complete"`. |
| `weight` | number \| null | `null` | **(v0.21)** Authored initiative weight, carried **verbatim** from `okf_core::Epic.weight`. Range policy (`0..=100`) is mev's — enforced by its `check_epics` (`E_STATE_EPIC_BAD_WEIGHT`), *not* by bastion, which never clamps, defaults, or range-checks it, so an out-of-policy authored value reaches the wire unchanged. `null` means unauthored, which stays distinguishable from an authored `0` (bastion-web currently falls back to `60`). |
| `plan` | string \| null | `null` | Repo-relative path to the owning master-plan / plan doc, when one exists. |
| `repos` | array of string | `[]` | Repos the initiative is expected to touch — an authored hint, not the membership source of truth (membership is authored on the blocks via `epics[]`, not here). |
| `closed` | number | `0` | Member blocks with authored `status == "closed"`. |
| `in_progress` | number | `0` | Member blocks with authored `status == "in_progress"`. |
| `open` | number | `0` | Member blocks that are open (authored `open`, or status absent). |
| `deferred` | number | `0` | Member blocks with authored `status == "deferred"`. |
| `total` | number | `0` | Every member block, in any state. `0` = no members yet. |
| `fully_deferred` | boolean | `false` | Is the epic's remaining work entirely parked? True iff ≥1 deferred member **and** no unfinished non-deferred work. An all-`closed` epic is *complete*, not deferred, so this stays `false` for it. Lets a Surface say "this whole initiative is parked" on load instead of drawing four empty lane columns. |

The six count fields are **derived**, joined from the corpus via mev's `epic_members` +
`epic_progress` — the same predicate behind the collapsed markdown epic board and
`mev sync-epics`, so the three cannot disagree about what a deferred epic is. They are *not*
authored in `epics[]`.

**Relationship to `status`.** `status` is authored human intent (`active`/`paused`/`complete`);
`fully_deferred` is derived from the blocks. They normally agree — `mev defer-epic <slug>` sets
both, and `mev sync-epics` reconciles drift — but a Surface should treat `status == "paused"` as
the authoritative "is this parked" signal and `fully_deferred` as the corroborating detail. A
`paused` epic with `fully_deferred: false` means some member work is still open or in flight.

The registry is HQ-only (same precedent as `backlog[]`, Section 15): the single `(source, file)`
pair whose `kind == "brain"` **and** whose resolved [`TierScope`] is `All` (mirrors mev's private
`epic_registry` helper). Order is preserved from the authoring `epics[]` list.

### 17.3 Error responses

| Condition | HTTP status | Body |
|---|---|---|
| Missing/invalid `Authorization` header | `401 Unauthorized` | JSON `ErrorPayload` (Section 2.2) |
| Unresolvable brain root (no `brain.toml` walking up from the workspace root) or unparseable `brain.toml` | `500 Internal Server Error` | JSON `ErrorPayload`, code `C010` |
| `web::block` thread-pool failure | `500 Internal Server Error` | JSON `ErrorPayload`, code `C010` |
| No HQ `kind:"brain"`/`TierScope::All` file found | `200 OK` | `[]` — an absent registry is **not** an error; the route reports the registry it found, and an absent HQ file is an operator-config problem the board route (Section 13) already surfaces. |

### 17.4 Testing

`hq_epic_registry` (HQ-file selection) and `build_epics` (registry → `EpicDto[]` projection) are
pure and unit-tested with no filesystem access, covering: the HQ file found among non-matching
files, a `kind:"brain"` file that is tier-scoped (not `All`) and therefore ignored, no files at
all, multiple candidate files with the HQ one picked correctly, all seven authored `EpicDto`
fields mapped (including `weight`), minimal-entry defaults (`repos: []`, the five `Option` fields
`None`), an empty registry, and order preservation.

`weight` passthrough is pinned by its own cases: an authored value survives verbatim, an
unauthored one stays `None`, and an out-of-policy value (`200`) plus the boundary values
`0`/`100`/`255` all reach the DTO untouched — asserting the no-derivation rule directly.
`EpicDto`'s serde round-trip tests (`src/serve/dto.rs`) additionally cover `weight` serializing as
a number when authored, as `null` (not omitted) when unauthored, and an absent key decoding to
`None`.

The thin `web::block` I/O shell (`get_epics`, `src/serve/handlers/epics.rs`)
and route wiring in `src/serve/mod.rs` are covered by `#[actix_web::test]` integration tests
against a temp brain-root fixture, asserting the bearer-auth `401`, a `200` registry listing
with all fields populated, an authored `weight` on the wire, and `null` for the unauthored one.

---

## 18. Embedded engine route table (v0.5, BA.7.C)

`bastion serve` embeds `engine-serve`'s route table (`engine_serve::http::configure`) at the
**server root** — not under `/api` — per D48 ("the abort endpoint and the rest of the engine
surface are served through `bastion serve`, embedding the Engine per D42") and the block's scope
growth (`planning/7.C-cost-budget-alerts-abort/tasks.md`, *Scope growth* section). This is the
same `engine-serve` surface engine-rs's own `EN.1.C`/`EN.2.B` shipped as an embeddable library —
`bastion serve` is the first (and, as of this writing, only) process that actually mounts it.

### 18.1 Mount decision

The engine routes are mounted only when **both** `DATABASE_URL` and `BASTION_ENGINE_API_KEY` are
set (non-empty) at boot — decided once, pure, by `serve::decide_engine_mount` (`src/serve/
mod.rs`). Absent-tolerant: with either value missing, `bastion serve` still boots its existing
`/api`/`/ws` surface (Sections 1–12) with the engine routes simply left unmounted; it prints why on
stderr and emits a `tracing::warn!` `observ` event rather than failing to boot or mounting a route
that would 500 on every request. A `DATABASE_URL` present but unreachable (connection failure at
boot) also leaves the engine routes unmounted, logged the same way.

### 18.2 Routes

| Route | Method | Auth | Description |
|---|---|---|---|
| `/health` | `GET` | None | Shadowed by bastion's own `/health` (Section 3) — always answers, engine-mounted or not. |
| `/workflows` | `GET` | `X-API-Key` | Registered workflow types (sorted). |
| `/workflows/{workflow_type}/graph` | `GET` | `X-API-Key` | The DAG schema for a registered type; `404` for an unknown one. |
| `/events/` | `POST` | `X-API-Key` | Trigger dispatch — resolves `workflow_type`, runs the workflow, mints a `run_id` and a `CancellationToken`. |
| `/events/{run_id}/abort` | `POST` | `X-API-Key` | The abort endpoint this block's `bastion abort <run>` calls — see [abort.md](abort.md) and [data-contract.md](data-contract.md)'s Abort section for the full 401/404/202 contract. |
| every other route in `engine_serve::http::configure` (`/events/suspended`, `/events/{event_id}`, `/events/{event_id}/resume`, `/events/{event_id}/stream`, `/webhooks/email/inbound`, `/webhooks/email/events`) | — | `X-API-Key` | Same gate, applied uniformly across the whole mount (Section 18.2.1). |

`X-API-Key` is required on **every route the engine mount registers except `/health`**. A request
with no `X-API-Key` header, an empty header, or a wrong value gets **401**; the configured
`BASTION_ENGINE_API_KEY` value gets through. This is entirely separate from bastion's own
`BASTION_SERVE_TOKEN` Bearer check (Section 2) — neither scheme is layered on the other's routes.

#### 18.2.1 `BA.ticket.engine-surface-auth` — reject-unauthenticated (2026-08-12)

Landed early, ahead of the rest of `BA.11.F` (below): `GET /workflows` and `GET
/workflows/{workflow_type}/graph` took no `HttpRequest` parameter and skipped `engine-serve`'s own
inline `check_api_key` call entirely, answering `200` to a bogus, rotated-away, or absent
`X-API-Key`. The other 9 routes already checked `check_api_key` inline and were unaffected. The
fix wraps the **whole** engine mount (`src/serve/mod.rs`) in one `ApiKeyAuthMiddleware` —
mirroring `BearerAuthMiddleware`'s shape (`src/serve/auth.rs`) — so every route in the table is
gated the same way regardless of whether its handler also calls `check_api_key` itself (redundant
on the 9, now load-bearing on the 2). The pure comparison helper, `api_key_matches`, rejects an
empty configured key (and an empty provided header) rather than falling through to `"" == ""`;
`decide_engine_mount` already refuses to mount the engine at all on an empty key, so this is
defense-in-depth, not a live gap. `GET /health` stays unauthenticated — it is shadowed by
bastion's own `/health` handler before the engine mount is registered (Section 18.1's
first-registration-wins note), so the middleware wrapping the engine scope never sees it.

### 18.3 Testing

Covered by the in-process integration test `tests/abort_contract.rs`, which builds a real
`engine-serve` `App` (via `AppState`) and asserts the 401 / 404 / 202 paths against it — the
worked reference is `../engine-rs/crates/engine-serve/tests/abort_integration.rs`. The mount
decision itself (`decide_engine_mount`) is unit-tested element-by-element in `src/serve/mod.rs`
against all four presence/absence combinations of `DATABASE_URL` / `BASTION_ENGINE_API_KEY`,
including the empty-string-counts-as-absent case. `ApiKeyAuthMiddleware`'s pure comparison helper,
`api_key_matches`, is unit-tested exhaustively in `src/serve/auth.rs` (absent header, empty
header, empty configured key, wrong value, correct value); the per-route reject/accept behavior
(no header / bogus key / correct key against every route in the table, `GET /health` unaffected,
`/api/*` bearer routes unaffected) is covered by integration tests in `src/serve/mod.rs`
(`BA.ticket.engine-surface-auth` task 3).

---

## 19. Generated TypeScript types (v0.7, BA.11.L; okf-core scan widened v0.32, `BA.19.A`)

The contract DTOs in `src/serve/dto.rs` are annotated with `#[typeshare]` and are the **single
source of truth** for the TypeScript types consumed by BastionWeb (`BW.0.B`) and any other TS
client of this contract. `bastion-ui` (Flutter) is unaffected — it has no TS surface. As of
`BA.19.A`, the generated artifact also carries four types mirrored from the sibling `okf-core`
schema crate — see below.

### 19.1 Generated artifact

`types/serve.ts` (committed at the bastion package root) is the generated TypeScript output. It
is produced by the `typeshare` CLI reading the `#[typeshare]`-annotated types in **two** source
trees: `src/serve/dto.rs` (this repo's own wire DTOs) and the sibling `okf-core` crate's `src/`
(`../okf-core/src`, the same path `Cargo.toml` already pins as a dependency), via `typeshare.toml`.
**`types/serve.ts` MUST NOT be hand-edited** — any change belongs in `dto.rs` (bastion's own DTOs)
or upstream in `okf-core` (the four payload structs below), followed by regeneration. The file
carries typeshare's own `/* Generated by typeshare … */` header, which already marks it as
generated (no separate hand-added banner is layered on top, so the committed file stays
byte-identical to raw CLI output).

The `okf-core` scan emits four interfaces — `BlockDep`, `ExternalDep`, `OperatorDep`,
`ApprovalDep` — the payload structs backing `okf_core::state::BlockedBy`'s four variants. They are
generated from `okf-core` rather than mirrored by hand in `src/serve/dto.rs` because `okf-core` is
the schema crate that owns their field shapes; bastion only serialises them verbatim. See
**D19** (`planning/decisions/D19-typeshare-scan-includes-okf-core.md`) for the full rationale, including why the
scan is scoped to `okf-core` specifically rather than sibling crates generally.

typeshare cannot express an internally-tagged algebraic enum, so the `BlockedBy` enum itself is
never generated — only its four payload variants are. `types/serve.ts` therefore keeps a
pre-existing dangling `blocked_by?: BlockedBy[]` reference with no matching declaration; that is
not a bug introduced here. The four-arm union that resolves it is hand-written downstream, in
BastionWeb (`BW.16.A`), from the four generated interfaces.

Two exclusions from generation, both internal-only types that never cross the wire: `Topic`
(parsed from a WS subscription string, never itself serialized) and `CommandValidationError` (a
`Display`/`Error`-only validation enum, not serde). Neither derives `Serialize`/`Deserialize`, and
each carries a data variant with no serde representation for `typeshare` to mirror, so both are
left unannotated rather than forced.

### 19.2 Regenerating

Prerequisite: the `typeshare` CLI on `PATH` (`cargo install typeshare-cli --locked`).

```bash
scripts/gen-types.sh                 # writes types/serve.ts in place
# equivalent raw invocation:
typeshare src/serve ../okf-core/src --lang typescript --output-file types/serve.ts --config-file typeshare.toml
```

Run this after any change to `src/serve/dto.rs`'s public types (new field, new type, new enum
variant, etc.) or to `okf-core`'s `#[typeshare]`-annotated payload structs, and commit the
regenerated `types/serve.ts` alongside the source change.

### 19.3 Drift check

`scripts/check-typeshare-drift.sh` regenerates the types to a temp file (via the same invocation
`gen-types.sh` uses, so the two scripts can never diverge on flags) and diffs it against the
committed `types/serve.ts`:

- Exits **0** and prints `OK: types/serve.ts is up to date with src/serve/dto.rs and
  okf-core/src.` when identical.
- Exits **non-zero** and prints the unified diff when `types/serve.ts` is stale relative to
  `dto.rs` or `okf-core`'s annotated types (e.g. a DTO field was added without regenerating).
- Exits **non-zero** with an actionable install hint (`cargo install typeshare-cli --locked`)
  when the `typeshare` binary is absent from `PATH`, rather than a confusing tool error.

CI and BastionWeb rely on this script to guarantee `types/serve.ts` never silently drifts from
either source tree. It is a standalone script — it is **not** wired into `planning/harness.json`
(out of scope for this block; see `planning/11.L-typeshare-ts-generation/tasks.md`).

No `serve` runtime behaviour changed as part of this section — `#[typeshare]` annotations are
compile-time no-ops, and generation/drift-check are build-time-only tooling. Every DTO shape
documented in Sections 3, 5–13 is unchanged.

---

## 20. Configuration reference

| Env var | Required | Default | Description |
|---|---|---|---|
| `BASTION_SERVE_ADDR` | No | `0.0.0.0:4317` | `host:port` to bind |
| `BASTION_SERVE_TOKEN` | **Yes** | — | Bearer token for protected routes; absent token is a typed error at startup |
| `DATABASE_URL` | No | — | Postgres URL for the engine's durable writer. Absent (or unreachable) leaves the Section 18 engine routes unmounted; bastion's own `/api`/`/ws` surface never needed this and still doesn't. |
| `BASTION_ENGINE_API_KEY` | No | — | `X-API-Key` secret the engine routes (Section 18) check. Absent leaves those routes unmounted. |

`bastion serve` loads config via `load_serve_config()` (`src/config.rs`), which
is DB-free and does **not** require `DATABASE_URL` for its own `/api`/`/ws` surface. The
`[workspaces]` registry consumed by Section 11's routes is loaded separately via
`load_workspace_registry()` — also DB-free — once at server startup. `DATABASE_URL` and
`BASTION_ENGINE_API_KEY` are read directly from the environment at boot (Section 18.1), not
through `load_serve_config()`.

---

## 21. Versioning policy

This document follows a simple monotonic version scheme:

| Change type | Version bump |
|---|---|
| New route or frame kind | v0.x minor bump |
| Breaking change to an existing route/shape | v1 major bump |

`bastion-ui` MUST pin to a specific version tag.  The current contract is **v0.27**.

---

## 22. Pipeline / opportunities API (v0.12, BW.3.A)

A read-only (D25) projection over the **business sub-brain**'s opportunity
markdown files, so a web client can browse a sales pipeline of "opportunities"
(researched companies, prospecting sweeps, and future job postings). Backing
handlers live in `src/serve/handlers/pipeline.rs`.

The data source lives at the **HQ brain root** (a sibling of `brain.toml`):

- `business/docs/pipeline.md` — its `## Stages` line
  (`` `identified` → `researching` → `contacted` → `conversation` → `proposal-sent` → `closed-won` → `closed-lost` ``)
  is the canonical stage vocabulary/order.
- `business/docs/opportunities/*.md` and `business/docs/leads/*.md` — one file
  per opportunity. `index.md` and `README.md` are skipped, as is any non-`.md`
  file. When the same slug exists in both directories, `opportunities/` wins.

The HQ root is resolved exactly like `GET /api/board` / `GET /api/attention`:
`resolve_workspace_root(None, None, registry)` picks a starting path, then
`find_brain_root` walks up to the directory containing `brain.toml`;
`business/docs/...` is read relative to that root. A missing/unreadable
`business/` tree is **not** an error for the list route — it degrades to empty
`stages`/`opportunities`.

Each opportunity file is `YAML frontmatter + body`. Frontmatter fields (all
optional except a title fallback to the slug): `kind` (`company` |
`prospecting-sweep` | `job-posting`, default `company`), `stage`, `source`,
`url`, `links[]`, `last_contact`, `next_action`, `research_ref`,
`contacts[]` (`{name, role, emails[], whatsapp[], phones[], links[], note}`),
`actions[]` (`{at, kind, note}`). The **research brief** is not frontmatter — it
is the first ` ```json ` fenced block in the body (raw EN.4.A output): a
CompanyBrief (has `company_name`) or a ProspectingResult (has
`prospects`/`vertical`).

### 22.1 `GET /api/pipeline` — stage vocab + opportunity summaries

Bearer-protected (Section 2). Response `200 OK`: `PipelineDto`.

```json
{
  "stages": ["identified", "researching", "contacted", "conversation", "proposal-sent", "closed-won", "closed-lost"],
  "opportunities": [
    {
      "slug": "anthropic",
      "kind": "company",
      "title": "Anthropic",
      "source": "RESEARCH_AGENT test run (company mode)",
      "stage": "identified",
      "last_contact": null,
      "next_action": null,
      "has_findings": true
    }
  ]
}
```

Opportunities are sorted by **stage order** (the index of `stage` in `stages`;
an unknown or absent stage sorts last) then `title` (case-insensitive).
`has_findings` is `true` when the body carries a parseable ` ```json ` brief.

`PipelineDto`:

| Field | Type | Notes |
|---|---|---|
| `stages` | string[] | Canonical stage vocabulary in pipeline order (`[]` when `pipeline.md` is absent). |
| `opportunities` | `OpportunitySummaryDto[]` | One per opportunity file. |

`OpportunitySummaryDto`: `slug` (string, file stem), `kind` (string), `title`
(string), `source` (string?), `stage` (string?), `last_contact` (string?),
`next_action` (string?), `has_findings` (boolean).

### 22.2 `GET /api/pipeline/{slug}` — one opportunity's full projection

Bearer-protected. `{slug}` is the file stem. Response `200 OK`:
`OpportunityDetailDto`.

| Field | Type | Notes |
|---|---|---|
| `slug` | string | File stem. |
| `kind` | string | Default `company`. |
| `title` | string | Falls back to `slug`. |
| `source` / `stage` / `last_contact` / `next_action` / `url` / `research_ref` | string? | Frontmatter scalars. |
| `links` | string[] | Frontmatter list. |
| `contacts` | `ContactDto[]` | `{name?, role?, emails[], whatsapp[], phones[], links[], note?}`. |
| `findings` | `ResearchBriefDto?` | Parsed from the body's first ` ```json ` fence. |
| `actions` | `OpportunityActionDto[]` | `{at, kind, note}` activity log. |
| `body_markdown` | string? | The body after the frontmatter (absent when empty). |

`ResearchBriefDto`: `kind` (string — `"company"` \| `"prospecting"`),
`company_name` (string?), `summary` (string?), `recent_developments` (string[]),
`pain_points` (string[]), `outreach_hooks` (string[]), `sources` (string[]),
`vertical` (string?), `common_pain_points` (string[]), `prospects`
(`ProspectLeadDto[]`). `ProspectLeadDto`: `name` (string), `pillar` (string),
`pain_points` (string[]), `outreach_hook` (string?), `source` (string?).

### 22.3 Error responses

| Condition | Status | `code` |
|---|---|---|
| `{slug}` not found in either directory, or a slug containing a path separator / `.` / NUL (rejected before any filesystem access, so an invalid slug is indistinguishable from an absent one) | 404 | `C002` |
| Brain root unresolvable (no `brain.toml`) | 500 | `C010` |
| `web::block` thread-pool failure | 500 | `C010` |

### 22.4 Testing

The pure projection functions (`parse_stages`, `split_frontmatter`,
`extract_json_brief`, `parse_opportunity`, `to_summary`, `stage_rank`,
`sort_opportunities`, `valid_slug`) are unit-tested with inline fixture markdown
(company brief, prospecting sweep, structured contacts, minimal title-only,
defaults). Route tests cover 401-without-token on both routes, a 405 on
wrong-method, and hermetic 200s (a temp brain root with `brain.toml` +
`business/docs/...` registered as `default_workspace`), plus the 404/`C002`
unknown-slug branch.

---

## 23. Block-graph API (v0.13, BA.17.A, program block BA.2.A)

One read-only route (D25) projecting mev's enriched block-graph export — the same
nodes/edges/cycles/topo-order data `mev block-graph` computes from a `depends_on`-edge walk of the
brain — onto HTTP, so a client can render the whole dependency graph rather than only the four
flattened board lanes (Section 13). Lives under the bearer-protected `/api` scope (Section 2).
Backing handler: `src/serve/handlers/block_graph.rs`.

**Bastion performs zero derivation of its own here.** The handler reuses `board::assemble_board`
(Section 13) for the brain-walk — config, loaded files, the `StateGraph`, and `stale` — so the
graph and the board read one corpus in one request shape, then calls
`mev::build_block_graph_export` directly and copies every field of the resulting
`BlockGraphExport` straight across into the wire DTO. This is the one-derivation contract
bastion-web's node-graph view (`BW.9.B`) relies on: the graph the client renders and the board the
client also renders are guaranteed to agree on node count, edge count, and per-node lane, because
both are read off the same corpus by the same brain-walk.

### 23.1 `GET /api/blocks/graph` — cross-brain block dependency graph

**Query parameters:**

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `scope` | string | No | `"hq"` | One of `"hq"` \| `"tier"` \| `"project"` \| `"business"` \| `"epic"`. Resolved to a `TierScope` identically to `GET /api/board` (Section 13.2) via the shared `board::resolve_scope`. An unrecognized value fails query deserialization and returns `400` (Section 23.3), same as Section 13.4's `scope=` case. |
| `tier` | string | No | `"core"` | Tier name; only consulted when `scope` is `"tier"` or `"project"`. Ignored for `"hq"`/`"business"`/`"epic"`. An empty-string or unknown tier behaves exactly as documented in Section 13.2 — no in-scope repos rather than an error. |
| `epic` | string | Only for `scope=epic` | — | Epic slug (from the HQ `epics[]` registry, Section 17) to filter the graph to. Required, and only consulted, when `scope=epic`; missing/blank or unknown → `404`/`C005` (Section 23.3). Ignored for every other scope. |
| `repo` | string | No | — | Narrows the export to a single repo slug. Absent means every in-scope repo. |
| `include_closed` | bool | No | `false` | Whether nodes whose lane is `"closed"` are retained in the export. |
| `include_boundary` | bool | No | `false` | Whether direct dependency neighbours of the in-scope node set are re-added as boundary nodes (visible context outside the strict scope, e.g. a cross-repo blocker). |
| `max_nodes` | number | No | `400` | Cap on returned nodes. Any value (including `0` and any value above the clamp) is clamped to at most `2000` — `resolve_max_nodes` in the handler. |

**Request:**

```
GET /api/blocks/graph?scope=hq HTTP/1.1
Authorization: Bearer <token>
```

```
GET /api/blocks/graph?scope=epic&epic=bastion-surfaces&max_nodes=200 HTTP/1.1
Authorization: Bearer <token>
```

### 23.2 Response (200 OK): `BlockGraphDto`

```json
{
  "version": "1",
  "root": "/Users/brandon/Dev/agentic-portfolio",
  "scope": "hq",
  "tier": null,
  "epic": null,
  "repo": null,
  "include_closed": false,
  "include_boundary": false,
  "nodes": [
    {
      "key": "bastion:BA.17.A",
      "repo": "bastion",
      "id": "BA.17.A",
      "title": "GET /api/blocks/graph endpoint",
      "status": "in_progress",
      "lane": "now",
      "track": "Phase 17",
      "wave": 1,
      "priority": 1,
      "effective_priority": 1,
      "due": null,
      "epics": ["bastion-surfaces"],
      "layer": 0,
      "topo_index": 4,
      "ready": true,
      "in_cycle": false,
      "in_scope": true,
      "external_deps": [],
      "unmet_count": 0,
      "dependent_count": 0
    }
  ],
  "edges": [
    { "from": "bastion:BA.17.A", "to_ref": "mev:MV.10.B", "kind": "blocked_by",
      "target_node_id": "mev:MV.10.B", "blocking": false }
  ],
  "cycles": [],
  "total_nodes": 115,
  "truncated": true,
  "stale": false
}
```

| Field | Type | Description |
|---|---|---|
| `version` | string | Schema version of the underlying mev export — currently `"1"`. |
| `root` | string | Display path of the brain root used for the build. |
| `scope` | string | Echoes the resolved response-level scope (`"hq"`/`"tier"`/`"project"`/`"business"`/`"epic"`) — the same `BoardScope` enum Section 13.3 uses. |
| `tier` | string \| null | Resolved tier name for tier-scoped responses; `null` for `"hq"`/`"epic"`. |
| `epic` | string \| null | The resolved `&epic=` slug, echoed from `mev`'s scope, when `scope=epic`. |
| `repo` | string \| null | The resolved `&repo=` restriction, when present. |
| `include_closed` | boolean | Echoes the resolved `include_closed` param. |
| `include_boundary` | boolean | Echoes the resolved `include_boundary` param. |
| `nodes` | array of `BlockGraphNodeDto` | Nodes, emitted in `topo_index` order (see below). |
| `edges` | array of `BlockGraphEdgeDto` | One entry per surviving scoped `depends_on` edge (see below). |
| `cycles` | array of array of string | Dependency cycles found over the **full corpus**, not the scoped subgraph — each inner array is a cycle's member node keys. `[]` when the corpus is acyclic. |
| `total_nodes` | number | Node count before any `max_nodes` truncation — lets a client tell "showing 400 of 115" from "showing all 115". |
| `truncated` | boolean | Whether `max_nodes` cut the node list short. |
| `stale` | boolean | Freshness flag: `true` when any in-scope repo's `planning/status.md` cache lags its `state.json` — same posture and same `mev::brain::sync::check_sync` source as `BoardDto.stale` (Section 13.3). |

#### `BlockGraphNodeDto`

| Field | Type | Description |
|---|---|---|
| `key` | string | Canonical `"repo:id"` key. |
| `repo` | string | Owning repo slug. |
| `id` | string | Canonical block ID (e.g. `"BA.17.A"`). |
| `title` | string | Brief human description. |
| `status` | string \| null | Authored lifecycle status (`"open"`/`"in_progress"`/`"deferred"`/`"closed"`), if any. |
| `lane` | string | One of `"now"` \| `"next"` \| `"blocked"` \| `"deferred"` \| `"closed"` \| `"other"` — same six-value lane vocabulary as `BoardBlockDto`'s lane placement, computed identically (Section 23's one-derivation cross-check test enforces this). |
| `track` | string \| null | Title of the containing `tracks[]` phase/wave, if resolvable. |
| `wave` | number \| null | Authored execution-order rank (`TrackBlock.wave`). |
| `priority` | number \| null | Authored own priority. |
| `effective_priority` | number \| null | Effective priority; absent when it never lands in the real `0..=3` range. |
| `due` | string \| null | Authored due date/timing string. |
| `epics` | array of string | Cross-repo epic membership (`[]` when none). |
| `layer` | number | Longest path over resolved `depends_on` edges (`0` = no resolved prerequisites). |
| `topo_index` | number | Position in the full-corpus topological order. |
| `ready` | boolean | Membership in the ready-order set. |
| `in_cycle` | boolean | Whether this node participates in a `depends_on` cycle. |
| `in_scope` | boolean | Whether this node survives the scope pipeline's tier/repo/epic/closed stages. |
| `external_deps` | array of string | `what` strings from this block's `{type:"external"}` `depends_on` entries. |
| `unmet_count` | number | Count of unmet dependencies for a `"blocked"` node — `0` for every other lane. |
| `dependent_count` | number | Corpus-wide count of in-corpus blocks whose `blocked_by` edges point at this node. |

#### `BlockGraphEdgeDto`

| Field | Type | Description |
|---|---|---|
| `from` | string | `"repo:id"` key of the source (dependent) block. |
| `to_ref` | string | Raw, as-authored `"repo:id"` reference. |
| `kind` | string | `"blocked_by"` \| `"cross_repo"`. |
| `target_node_id` | string \| null | `Some(to_ref)` when it resolves to a node in this export; `null` when dangling. |
| `blocking` | boolean | `false` when either endpoint is `"closed"`. |

### 23.3 Error responses

| Condition | HTTP status | Body |
|---|---|---|
| Missing/invalid `Authorization` header | `401 Unauthorized` | JSON `ErrorPayload` (`{"error": "unauthorized", "code": "unauthorized"}`, Section 2.2) |
| Unrecognized `scope` value (fails `BoardScope` query deserialization) | `400 Bad Request` | Plain text — same actix stock extractor failure as `GET /api/board` (Section 13.4); no `QueryConfig` error handler is installed for this route either. |
| `scope=epic` with a missing/blank `&epic=` param | `404 Not Found` | JSON `ErrorPayload`, code `C005` — reuses `board::epic_param_missing` / `board::epic_error_response` verbatim, same message shape as Section 13.4. |
| `scope=epic` with an `&epic=<slug>` value absent from the HQ `epics[]` registry (Section 17) | `404 Not Found` | JSON `ErrorPayload`, code `C005` — reuses `board::epic_known` / `board::epic_error_response` verbatim, message naming the unknown slug (`"unknown epic: <slug>"`). |
| Unresolvable brain root (no `brain.toml` walking up from the workspace root) or unparseable `brain.toml` | `500 Internal Server Error` | JSON `ErrorPayload`, code `C010`, via `board::brain_root_error_response`. |
| `web::block` thread-pool failure | `500 Internal Server Error` | JSON `ErrorPayload`, code `C010`, via `board::blocking_error_response`. |

Individual malformed/unreadable `state.json` files under an otherwise-resolvable brain root are
skipped (degrade gracefully) rather than failing the whole request — same posture as
`board::assemble_board` (Section 13.4).

### 23.4 Testing

`resolve_max_nodes`, `block_graph_scope_from_query`, and `block_graph_dto` (the export → DTO
mechanical projection, including the six-variant `BlockLane`/`BlockLaneDto` mapping and the
two-variant `StateEdgeKind`/`BlockEdgeKindDto` mapping) are pure and unit-tested with no filesystem
access. A dedicated **cross-check test** builds one fixture brain root covering all five real
board lanes (now/next/blocked/deferred/finished), feeds the same `(config, files, graph)` triple
assembled once by `board::assemble_board` into both `mev::build_block_graph_export` →
`block_graph_dto` and `board::build_board`, and asserts the two independent read paths agree on
node count, edge count, and every node's lane — mechanically enforcing the one-derivation
contract. The thin `web::block` I/O shell (`get_block_graph`) and route wiring in
`src/serve/mod.rs` are covered by `#[actix_web::test]` integration tests asserting the bearer-auth
`401`, the `scope=epic` 404/`C005` pair, and a `200` body against both a real and a fixture brain
root, plus manually smoke-tested end-to-end against a running `bastion serve` with the real HQ
brain root (recorded in `planning/17.A-block-graph-endpoint/tasks.md`'s `## Notes`).

---

## 24. Cost read API (v0.15, BA.11.J)

One read-only route projecting the existing `src/costs/` module — BA.7.B's exact per-workflow
token/cost aggregation and BA.7.C's budget-gate evaluation — onto HTTP, so `bastion-ui` and any web
dashboard can render spend and budget state without shelling to the CLI. Lives under the
bearer-protected `/api` scope (Section 2). Backing handler: `src/serve/handlers/costs.rs`.

**Read-only.** Nothing over HTTP mutates the configured budget caps — cap mutation stays CLI/D48.
There is no streaming/push variant; the response is a point-in-time snapshot for the resolved
window, computed fresh on every request.

**Mounts unconditionally.** Unlike the embedded engine routes (Section 18, gated on
`DATABASE_URL` + the engine API key being present), `/api/costs` is always registered on both
`serve` app factories. A missing or unreachable database is answered as a typed error response
(below), never a `404` — this keeps the route's presence a stable contract fact for clients, and
matches how `bastion costs` itself degrades (an actionable message, never a panic).

**No `?repo=` filter.** The block definition named an optional `?repo=` query param, but the
`events` contract carries no repo dimension to filter on: `db::costs::fetch_all_runs` reads
`WorkflowRun { id, workflow_name, status, budget_halt, nodes, started_at, elapsed_secs }` rows —
no `repo` field — and `costs::aggregate` groups by `workflow_name` only. This is a deliberate
deviation from the block definition, decided with the owner 2026-07-29 (see
`planning/11.J-cost-read-endpoint/tasks.md`'s Amendment Log); `repo` is **not** silently aliased
onto `workflow_name`. This remains true here in v0.22: the events contract itself still carries no
repo dimension, and that has not changed. What changed is `GET /api/runs` (Section 14.1, A7) —
its `repo` field is **not** sourced from the events contract at all. It is joined from a different
source entirely: `RepoWorkflowStateDto` (Section 11.6, A2), which pairs each registered workspace's
on-disk flow state with the `run_id` A1 stamped onto it. So the exact-`run_id` join `/api/runs`
performs sidesteps this cost gap rather than closing it — `/api/costs` still cannot filter or
group by repo, and this note's underlying constraint is unchanged.

### 24.1 `GET /api/costs` — cost + budget-state summary

**Query parameters:**

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `window` | string | No | `"7d"` | One of `"7d"` \| `"30d"` \| `"all"`, case-insensitive, parsed by `costs::parse_window`. Missing or empty/whitespace-only applies the default (matching `bastion costs`' own `--last` default). An unparseable value returns `400`/`C006` (Section 24.3). |

**Request:**

```
GET /api/costs HTTP/1.1
Authorization: Bearer <token>
```

```
GET /api/costs?window=30d HTTP/1.1
Authorization: Bearer <token>
```

### 24.2 Response (200 OK): `CostSummaryDto`

```json
{
  "window": "7d",
  "rows": [
    { "workflow_name": "content-pipeline", "runs": 12, "tokens_in": 48000, "tokens_out": 9000, "usd": 1.32 },
    { "workflow_name": "research-pipeline", "runs": 3, "tokens_in": 5000, "tokens_out": 700, "usd": 0.18 }
  ],
  "totals": { "workflow_name": "TOTAL", "runs": 15, "tokens_in": 53000, "tokens_out": 9700, "usd": 1.50 },
  "unpriced_models": ["some-unpriced-model"],
  "budget": {
    "max_total_tokens": null,
    "max_cost_usd": null,
    "total_tokens": 62700,
    "total_cost_usd": 1.50,
    "breached": false,
    "breach": null
  }
}
```

| Field | Type | Description |
|---|---|---|
| `window` | string | The resolved window, echoed back (`"7d"` / `"30d"` / `"all"`), independent of the case the caller sent. |
| `rows` | array of `WorkflowCostDto` | One row per distinct `workflow_name`, sorted by `usd` descending — same order `costs::aggregate` produces, carried through verbatim. |
| `totals` | `WorkflowCostDto` | Sum across all rows; `workflow_name` is `"TOTAL"`. |
| `unpriced_models` | array of string | Model IDs seen in the data with no price entry — spend for these is under-reported rather than silently omitted, so a client can surface the gap. |
| `budget` | `BudgetStateDto` | Configured caps + current gate state for this window (below). |

#### `WorkflowCostDto`

| Field | Type | Description |
|---|---|---|
| `workflow_name` | string | Workflow name this row aggregates (or `"TOTAL"` for the totals row). |
| `runs` | number | Count of contributing runs. |
| `tokens_in` | number | Summed input tokens. |
| `tokens_out` | number | Summed output tokens. |
| `usd` | number | Summed USD cost. |

#### `BudgetStateDto`

| Field | Type | Description |
|---|---|---|
| `max_total_tokens` | number \| null | Configured token cap (`Config::max_total_tokens`), when set; `null` when no cap is configured. |
| `max_cost_usd` | number \| null | Configured USD cost cap (`Config::max_cost_usd`), when set; `null` when no cap is configured. |
| `total_tokens` | number | `tokens_in + tokens_out` from `totals` — the current spend reading against `max_total_tokens`. |
| `total_cost_usd` | number | `totals.usd` — the current spend reading against `max_cost_usd`. |
| `breached` | boolean | Whether any configured cap has been **reached** (`>=`, not merely approached) — `costs::budget::evaluate`'s documented boundary. Always `false` when no caps are configured (`Budget::default()`). |
| `breach` | `BudgetBreachDto` \| null | Present only when `breached` is `true`. |

#### `BudgetBreachDto`

| Field | Type | Description |
|---|---|---|
| `cap` | string | Which cap was breached — exactly `"max_total_tokens"` or `"max_cost_usd"`, `Cap::as_str`'s wire strings (matching what the embedded engine stamps into `metadata.budget.reason.cap`). |
| `spent` | number | The spend value that tripped the cap. |
| `limit` | number | The configured limit that was reached. |

### 24.3 Error responses

| Condition | HTTP status | Body |
|---|---|---|
| Missing/invalid `Authorization` header | `401 Unauthorized` | JSON `ErrorPayload` (`{"error": "unauthorized", "code": "unauthorized"}`, Section 2.2) — the handler is never reached. |
| Unparseable `?window=` value | `400 Bad Request` | JSON `ErrorPayload`, code `C006`, message names the bad value. |
| `DATABASE_URL` unset (`Config::load` fails) | `503 Service Unavailable` | JSON `ErrorPayload`, code `C005` — the route is present but degrades; not a `404`. |
| Postgres unreachable, or the `events` query fails | `503 Service Unavailable` | JSON `ErrorPayload`, code `C009`. |
| `web::block` thread-pool failure | `500 Internal Server Error` | JSON `ErrorPayload`, code `C010`, mirroring `handlers/board.rs::blocking_error_response`. |

### 24.4 Testing

`resolve_window`, `budget_from_config`, `budget_state_dto`, and `cost_summary_dto` (the whole
projection, including the window-label echo and the `Cap::as_str` mapping) are pure and
unit-tested with no I/O — covering the default/case-insensitive/garbage `window` cases, the
no-caps/tokens-cap/cost-cap/at-limit-boundary budget cases, and a **CLI-parity check** that runs
`costs::aggregate` over the same `src/db/fixtures/*.json` runs `src/db/workflows.rs` already uses
and asserts the DTO's rows/totals equal `CostSummary`'s fields verbatim, so the endpoint and
`bastion costs` cannot drift apart. The thin `web::block` I/O shell (`get_costs`) and route wiring
in `src/serve/mod.rs` are covered by `#[actix_web::test]` integration tests asserting the
bearer-auth `401`, the `DATABASE_URL`-unset `503`/`C005` case (no live Postgres required), the
database-connect-failure `503`/`C009` case (`DATABASE_URL` set to a syntactically-invalid
connection string, so `db::costs::fetch_all_runs`'s `PgPoolOptions::connect` fails fast at parse
time — exercising the same `Err` -> `db_error_response` branch a genuinely unreachable Postgres
takes, without incurring sqlx's ~30s default `acquire_timeout` retry/backoff on an actually-refused
TCP connection), the `?window=nonsense` `400`/`C006` case (short-circuits before any database
access), and reachability on both app factories — plus manually smoke-tested end-to-end against a
running `bastion serve`
and a real Postgres, comparing the returned totals against `bastion costs --last 7d` for the same
window (recorded in `planning/11.J-cost-read-endpoint/tasks.md`'s `## Notes`).

---

## 25. Contract-corpus goldens (stub-fidelity check, ask A4)

`types/contract-corpus/` (committed at the bastion package root, beside `types/serve.ts`) is a
directory of checked-in golden JSON files, one per (route × scenario), produced by dispatching a
real request through the **real** `serve` app factory and handlers and capturing the **real**
`serde` serializer's output. It exists so `bastion-web`'s e2e stub can assert against real serve
behaviour instead of a hand-maintained approximation of it — the corpus is the mechanism that
turned two real shipped bugs (a docs-tree file path 404ing where real serve returns `200 +
entries: []`, and a `workflow_type` key-case mismatch) into something a PR diff would have caught
before either shipped. Consumer side: `../bastion-web/ticket-stub-fidelity-check/`.

### 25.1 Naming convention

Each golden is named `<route>__<scenario>.json` — e.g. `runs__budget_halted.json`,
`docs-tree__file.json`, `pipeline__keyless.json`. Every file is a pretty-printed JSON object with
at least `{status_code, body}`, where `body` is the parsed response payload (not a raw string), so
diffs read as structured JSON rather than an escaped blob.

**Goldens must never be hand-edited — exactly like `types/serve.ts` (Section 19.1).** The only
way a golden may change is by regenerating it from the real handlers (below); a hand-authored or
hand-tweaked golden defeats the entire point of the corpus, since it would no longer prove
anything about what `serve` actually returns.

### 25.2 Regenerating

```bash
scripts/gen-contract-corpus.sh                 # writes types/contract-corpus/ in place
```

The script sets `BASTION_DUMP_CORPUS=1` and runs the `#[cfg(test)] src/serve/contract_corpus.rs`
scenario tests (`*_scenarios::`), which switch the harness's `dump()` function from its default
**verify** mode (assert the response matches the checked-in golden — what a normal `cargo test`
run exercises) into **generate** mode (write the golden to disk). Run this after any change to a
handler or DTO shape covered by the corpus, and commit the regenerated files alongside the source
change.

### 25.3 Determinism / redaction rules

A non-deterministic golden is worse than none — it trains reviewers to ignore the diff. Every
value in a golden that could legitimately vary between two otherwise-identical runs is neutralized
by `contract_corpus.rs`'s `redact_value` before being written or compared (see that module's own
header comment for the authoritative, line-numbered version of these rules):

1. RFC3339 timestamps and bare `YYYY-MM-DD` calendar dates (`started_at`, `updated_at`,
   `last_touched`, `as_of`, `created`, `reviewed`, …) → `"<TIMESTAMP>"`.
2. UUIDs (`run_id`, …) → `"<UUID>"`. Fixtures still seed a fixed UUID rather than
   `Uuid::new_v4()` so the pre-redaction value is deterministic too; this rule catches anything
   that slips through.
3. Absolute temp-dir paths (fixture scratch workspaces) → `"<TMP_PATH>"`. **Every spelling** of
   the temp root is matched, not just `std::env::temp_dir()`'s: on macOS that returns
   `/var/folders/…/T/` while `/var` symlinks to `/private/var`, so a handler that canonicalizes its
   paths — `BlockGraphDto.root` does — emits the `/private/var/…` form. Tightened 2026-08-01 by
   `ticket-contract-corpus-uncovered-routes`, which found the un-tightened rule leaking a live
   per-run temp path into `blocks-graph__populated.json`.
4. Object key ordering is confirmed sorted (not insertion-order) by a dedicated unit test rather
   than assumed from `serde_json`'s default configuration.
5. `age_days` (a live-clock-derived `Number`, not a string, so rule 1 cannot pattern-match it) is
   redacted by key name to the sentinel `0` regardless of its pre-redaction value.

Redaction touches values only — it never touches object keys, and it never touches the
`RunStatus` wire strings (e.g. `budget_halted`) the corpus exists to freeze.

### 25.4 Drift check (gating)

`scripts/check-contract-corpus-drift.sh` regenerates the corpus to a temp directory (via
`scripts/gen-contract-corpus.sh`, so the two scripts can never diverge on invocation) and diffs it
against the committed `types/contract-corpus/`, mirroring `scripts/check-typeshare-drift.sh`'s
(Section 19.3) structure and exit-code conventions:

- Exits **0** and prints `OK: types/contract-corpus/ is up to date with the real serve handlers.`
  when identical.
- Exits **non-zero** and prints the unified diff when the corpus is stale relative to the real
  handlers.

Unlike the typeshare drift check, this one **is** wired into `planning/harness.json`'s
`validation.checks[]` as a gating check, so a stale corpus fails the SDLC pipeline and CI, not
just a human's memory.

### 25.5 Standing rule — a changed golden IS a contract change

**A changed golden in a PR diff is a contract change**, exactly as a `types/serve.ts` diff is,
and must carry the same treatment: a version bump in this document's header/versioning history
(Section 21) and a dated entry in the Amendment Log below, describing what shifted and why. This
holds even though the corpus itself does not encode a version number — the *reason* a golden
changes is always either an intentional DTO/handler change (which already needs a version bump
for its own sake) or a redaction-rule gap letting a volatile field leak into a golden (which needs
a task-2-style fix, not a silent re-generate-and-commit). Treat an unexplained corpus diff in a PR
the same way an unexplained `types/serve.ts` diff would be treated: as a signal to stop and find
out what changed, not as routine churn to regenerate past.

No `serve` runtime behaviour changed as part of adding this section — the dump harness is
`#[cfg(test)]`-gated throughout and never compiles into the production binary.

### 25.6 Scenario inventory

As of 2026-08-02 (`ticket-costs-200-contract-golden`) **every route the original A4 corpus left
uncovered is frozen, and `costs`' populated `200` shape is frozen too.** The corpus holds 37
goldens across ten routes:

| Route | Scenarios (`<route>__<scenario>.json`) |
|---|---|
| `GET /api/runs` | `empty`, `active`, plus one per `RunStatus` variant: `pending`, `running`, `success`, `failed`, `cancelled`, `budget_halted`, `suspended` |
| `GET /api/board` | `hq`, `tier`, `epic`, `epic-404`, `last_touched`, `dependent_count` |
| `GET /api/attention` | `populated`, `empty` |
| `GET /api/pipeline` | `populated`, `keyless` |
| `GET /api/docs/{repo}/tree` | `dir`, `file`, `unknown` |
| `GET /api/workflows` (v0.20) | `workflows__empty`, `workflows__populated` |
| `GET /api/repos/{name}/handoff` (v0.18) | `handoff__populated`, `handoff__missing`, `handoff__unknown-repo` |
| `GET /api/epics` (v0.21) | `epics__populated`, `epics__empty` |
| `GET /api/blocks/graph` (v0.13) | `blocks-graph__populated` |
| `GET /api/costs` (v0.15) | `costs__bad-window`, `costs__no-database-url`, `costs__populated`, `costs__budget-breached`, `costs__empty`, `costs__windowed`, `costs__db-error` |

Notes on the five routes added by `ticket-contract-corpus-uncovered-routes`:

- **`workflows`** — `empty` freezes the "absent registry is `200 []`, never a 404" contract;
  `populated` seeds two repos so the per-repo `repo` tagging, the handler's `(repo, spec_slug)`
  sort (without which the `HashMap`-sourced registry would make the golden flap), and *both*
  `run_id` serialization branches (absent key vs present) are frozen in one response.
- **`handoff`** — the two 404s carry **different** structured error codes and both are frozen:
  `handoff__unknown-repo` is `404 + C005` (workspace name not in the registry) while
  `handoff__missing` is `404 + C002` (registered repo, absent `handoff.md`). A consumer that
  collapses them into one "not found" branch is wrong in a way only a frozen `code` catches.
- **`epics`** — `epics__populated` freezes the **post-v0.21** shape: one registry entry authoring
  `weight: 85` and one omitting it (wiring `null`, per `EpicDto`'s no-`skip_serializing_if`
  convention), so both branches are pinned. The route takes no path or query params and so exposes
  no per-epic 404; the second scenario is therefore the empty-registry shape — an absent HQ
  `epics[]` is `200 []`, not an error.
- **`blocks-graph`** — dispatched at an **explicit `max_nodes=50`**, never the route's absent-param
  default (400). The default is a tunable; pinning the input keeps the golden's node set and
  `truncated` flag from being rewritten by a change that has nothing to do with the wire contract.
  50 comfortably exceeds the fixture's block count, so the export is complete (`truncated: false`).
- **`costs`** — seven scenarios. `costs__bad-window` (`400 + C006`, an unparseable `?window=`,
  produced by `resolve_window` before any database access) and `costs__no-database-url`
  (`503 + C005`, `DATABASE_URL` genuinely absent) freeze the error shapes. `costs__populated`,
  `costs__budget-breached`, `costs__empty`, `costs__windowed`, and `costs__db-error` freeze the
  populated `200` (`CostSummaryDto`, including the nested `BudgetStateDto`/`BudgetBreachDto`) and
  its remaining error branch, via the compile-time fetch seam described below.

**Residual gap.** These goldens exercise the real handler from `Config::load` onward — window
resolution, budget construction, every error mapping, and the real `aggregate`/`cost_summary_dto`
serialization — but the row-fetching step itself is faked (`get_costs_with`'s `fetch_runs`
parameter, `src/serve/handlers/costs.rs`). They do not prove that `db::costs::fetch_all_runs`
deserializes real `events` rows into `WorkflowRun` in the shape the fixtures assume; that remains
covered only by the `#[ignore]`d `integration_fetch_all_runs_returns_vec` test in `src/db/costs.rs`
(run with `BASTION_INTEGRATION_TEST=1 cargo test -- --ignored`).

**Pricing-table coupling (accepted).** `costs__populated`'s `usd` values derive from
`src/costs/pricing.rs`'s hardcoded price table. Editing a model's price changes this golden. That
churn is expected and mildly useful — it makes a pricing edit visible in review — but it is *not*
a wire-contract change; do not read it as one.

Env discipline for all seven `costs` scenarios is non-negotiable and documented at their call sites
in `src/serve/contract_corpus.rs`: each takes `crate::testsupport::lock_env()` once, mutates
`DATABASE_URL` (and `XDG_CONFIG_HOME`, so a developer's real `~/.config/bastion/config.toml` cannot
supply a `database_url` or a budget cap and make the golden machine-dependent) only through
`EnvVarGuard`, shadows the repo `.env` with `DotenvShadow` (otherwise `dotenvy`'s upward walk
restores a working `DATABASE_URL` from an ancestor checkout and the expected 503 becomes a 200), and
calls `dump_with(&CorpusConfig::from_env_locked(&lock), …)` rather than `dump` — because `dump`
would take the same **non-reentrant** mutex a second time on the same thread and deadlock. The five
`get_costs_with` scenarios additionally pin **both** `BASTION_MAX_TOTAL_TOKENS` and
`BASTION_MAX_COST_USD` explicitly (unset for the no-cap scenarios, set for `budget-breached`) —
`Config::load` reads both from env, so an unpinned cap on the developer's machine would regenerate a
different `budget` block. `DATABASE_URL` is set to an unroutable dummy
(`postgres://corpus@127.0.0.1:1/corpus`) and never connected to, since the fetch function is faked;
a real Postgres being up or down cannot change any golden.

---

## 26. Operator-notification transport (v0.27, `BA.18.B`; acknowledgement contract v0.29, `ticket-telegram-answer-callback`)

An **outbound-only** background capability, not a REST/WS route — `bastion serve` delivers an
already-validated operator gate payload to a human over Telegram and long-polls for the tap that
answers it. There is no client of this section in the sense the rest of this document uses that
word: no `bastion-ui`/`bastion-web` request ever touches it, and it registers nothing under
`/api`. It is documented here because it lives in `src/serve/` and boots (or doesn't) as part of
`run_server`, the same way Section 18's engine mount does.

### 26.1 The payload contract this consumes

`bastion` does **not** define the operator payload shape — it is owned, validated, and versioned
by `engine-rs:EN.8.A`. See
[`../engine-rs/docs/operator-payload-contract.md`](../../engine-rs/docs/operator-payload-contract.md)
for `OperatorPayload { gate_id, rendered_summary, options, digest }` and the single
`ValidatedOperatorPayload::validate` constructor. This transport (`src/serve/notify/`) accepts
only a `ValidatedOperatorPayload` and fails closed — via `NotifyError::PayloadRejected` — on
anything that would not survive the narrowest target channel's limits (today, WhatsApp's confirmed
ceiling: ≤3 options, ≤20-char labels, ≤1024-char summary), so a Telegram-only POC never ships a
payload the eventual WhatsApp leg (`EN.8.B`+, out of scope here) could not also render.

### 26.2 Transport seam

`OperatorTransport` (`src/serve/notify/mod.rs`) is one `async_trait` with two halves:

- `send(&self, payload: &ValidatedOperatorPayload) -> Result<DeliveredMessage, NotifyError>`
- `poll_responses(&self, since: Option<UpdateCursor>) -> Result<(Vec<OperatorResponse>, Option<UpdateCursor>), NotifyError>`

`TelegramTransport` (`src/serve/notify/telegram.rs`) is the first and, as of this writing, only
implementation. The trait itself carries no channel-specific shape (no Telegram field, no
WhatsApp field) so a future WhatsApp implementation is a second `impl`, not a rewrite of this
seam. `NotifyError` splits along one axis — `is_retryable()` — separating transient transport
failure (`Transport`, `RateLimited { retry_after_secs }`) from permanent rejection
(`PayloadRejected`, `Unauthorized`, `Malformed`); no variant's `Display`/`Debug` may interpolate a
token, asserted by unit test.

### 26.3 No webhook — long-polling only, on purpose

Inbound is `getUpdates` long-polling exclusively. `bastion serve` registers **no** webhook route
for Telegram (or any future transport) and the poll loop opens **no** listening socket — it only
makes outbound HTTPS calls to `api.telegram.org`. This is a deliberate, non-negotiable posture,
not a placeholder pending a nicer webhook implementation later: a Telegram webhook needs a public
route into the Mini, and that reopens exactly what
`brain:HQ.ticket.tailscale-bind-and-token-rotation` closed. A route-table test
(`src/serve/mod.rs`, `BA.18.B` task 5) walks the app factory's registered routes and fails if any
path resembles a Telegram webhook, so a future refactor cannot quietly reintroduce one.

### 26.4 Configuration

| Env var | Required | Default | Description |
|---|---|---|---|
| `BASTION_TELEGRAM_BOT_TOKEN` | No | — | Telegram bot token. Absent (with `BASTION_TELEGRAM_CHAT_ID` also absent) leaves the transport unconfigured — `bastion serve` boots exactly as it does today, logged as `tracing::info!`, not a warning. |
| `BASTION_TELEGRAM_CHAT_ID` | No | — | The operator's Telegram chat id the bot delivers to. Same absent-is-fine contract as the token, paired with it. |

Exactly one of the two set (token without chat id, or the reverse) is a typed
`ConfigError::IncompleteTelegramConfig(&'static str)` naming the missing variable — a
half-configured transport is a startup error, never a silent partial boot. Both resolve through
`src/config.rs`'s existing pure-`from_env`-over-an-`Option<String>`-tuple convention
(`build_telegram_config` / `load_telegram_config`), the same shape as every other env-backed
config in this repo. The token is held in a `BotToken` newtype whose `Debug` renders
`BotToken(<redacted>)` — never the raw value — so a `{:?}` in a future log line cannot leak it by
accident.

**The token never lands in a tracked file.** `.env.example` carries empty placeholders for both
vars with a comment pointing at the Mini's `com.brandon.engine-serve.plist` as the only place the
real values live (see [config.md](config.md)). Neither var is read, echoed, or interpolated by any
task, test, fixture, or doc in this repo — the live-transport smoke test is an operator-run step,
not an agent-run one (`planning/BA.18.B/tasks.md`'s Notes).

### 26.5 Response resolution and stale-digest rejection

Each Telegram `callback_query` encodes `gate_id` + a bounded digest prefix + `option_key` into
`callback_data` (`encode_callback_data`/`decode_callback_data`, ≤64 bytes, Telegram's own
ceiling). On receipt, `resolve_response` compares it against the `ValidatedOperatorPayload` the
gate is still waiting on:

- **Gate id and digest prefix both match** → `Accepted { gate_id, option_key }`.
- **Gate id matches, digest prefix does not** → `StaleDigest` — **rejected**, never applied. This
  is what makes a payload that was mutated (re-rendered with different options or summary) after
  the operator was shown the original re-queue instead of silently executing against content the
  operator never saw.
- **Gate id has no pending payload** → `UnknownGate`.

The cursor threaded through `poll_responses` is `max(update_id) + 1` per Telegram's own `offset`
semantics; an empty poll result leaves the cursor **unchanged** (never reset to `None`), so a
restart resumes the backlog rather than replaying or dropping it. The background loop
(`NotifyPollLoop`, `src/serve/mod.rs::run_server`) applies exponential backoff (floor 1s, ceiling
60s, reset on a successful tick) to a retryable `NotifyError` so a persistent outage never spins
hot; the cursor itself is left untouched on failure either way.

### 26.6 Out of scope here

The queue this loop's resolved verdicts feed into, its depth limit, and the blocked-edge sink's
write path belong to `engine-rs:EN.8.B` — this spec's production wiring passes a `PendingLookup`
closure that always returns `None` until that queue exists, so every observed response resolves to
`UnknownGate` rather than being silently applied against nothing. WhatsApp itself and any
`bastion-ui` push surface are also out of scope; Section 26.1's portability guard exists precisely
so adding WhatsApp later does not require touching this section's payload handling.

### 26.7 `POST /api/notify/test` — test-send trigger (v0.28, `ticket-notify-send-trigger`)

Section 26.6's `|_gate_id: &str| None` `PendingLookup` stub made three of `BA.18.B`'s acceptance
criteria — inline render with declared options, response resolving back to gate + digest, stale
digest rejected — unverifiable, since nothing in the binary ever called
[`OperatorTransport::send`]. This route exists **solely** to make the
`operator-telegram-live-smoke` operator session runnable in one sitting; it is not a general
notification API and sends no operator-authored content.

**Route:** `POST /api/notify/test`, mounted inside the existing `web::scope("/api")`
(`src/serve/mod.rs`) — it inherits `BearerAuthMiddleware` like every other route in that scope,
and, per constraint 1 of `ticket-notify-send-trigger`, is **never** mounted at the app root (the
route-table test at `src/serve/mod.rs` asserts this, extended alongside the existing Telegram
webhook-absence check from Section 26.3).

**Auth:** bearer token, same as every other `/api` route. No token, no chat id, and no other
credential is ever read from the request body — the route takes no input at all; the transport
resolves its own credentials from env (Section 26.4), exactly as the background poll loop does.

**Behavior:** on each call, the handler (`src/serve/handlers/notify.rs`) builds a fixed
`OperatorPayload` — summary `"bastion notify test-send — operator smoke check"`, exactly 2
`OperatorResponseOption`s (`approve` / `reject`) — validates it through the real
`engine_core::operator::validate` against `OperatorPayloadLimits::default()` (constraint 3: never
a hand-rolled struct, so the smoke test actually exercises the contract it's meant to prove), and
generates a fresh `gate_id` (uuid v4) per call so repeated smoke-test runs never collide in the
registry. It registers the validated payload in the process-local `PendingPayloads` registry
(Section 26.7.1) **before** sending, so a response that arrives before `send` even returns can
still resolve, then sends it via the configured `OperatorTransport` and returns what it sent.

**Response (200 OK):** `NotifyTestResponseDto`

```json
{ "gate_id": "b3b8c9b0-...", "digest": "a1b2c3..." }
```

`gate_id` and `digest` are exactly what the payload carries — the same pair a subsequent Telegram
tap's `callback_data` encodes, so the operator can correlate the delivered message against this
response by eye.

**Error responses:**

| Condition | Status | Body |
|---|---|---|
| No bearer token | 401 | existing `BearerAuthMiddleware` rejection — unchanged, not specific to this route |
| Transport unconfigured (both `BASTION_TELEGRAM_BOT_TOKEN`/`BASTION_TELEGRAM_CHAT_ID` unset, or only one set) | 503 | `ErrorPayload { code: "C005", message: "operator notification transport not configured — set <var(s)>" }` — names the missing var(s) **by name only**, never a value |
| `OperatorTransport::send` failure | 502 | `ErrorPayload` with `code` distinguishing retryable (`C009` transport I/O, `C013` rate-limited) from permanent (`C006` payload rejected, `C012` unauthorized, `C008` malformed) failure — see `notify_error_code` in `src/serve/handlers/notify.rs` |

An unconfigured server still boots unchanged: the route is registered unconditionally and resolves
the transport from env **at call time**, degrading to 503 per-request rather than being
conditionally mounted at startup.

#### 26.7.1 `PendingPayloads` — the in-memory, process-local registry

`PendingPayloads` (`src/serve/notify/mod.rs`) is a `Mutex`-guarded map from `gate_id` to the
`ValidatedOperatorPayload` this process sent, with bounded capacity (a named const) and
oldest-first eviction on overflow — a registry fed by a test route must not grow without limit.
One instance is constructed in `run_server` and shared (`Arc`) between this route's app data and
the `NotifyPollLoop`'s `PendingLookup`, replacing the `|_gate_id: &str| None` stub from Section
26.6. On `Accepted`, the resolved entry is removed, so a replayed tap of the same button resolves
to `UnknownGate` on its second arrival rather than applying twice.

**This registry is in-memory and process-local — it does not survive a restart, and it is not
shared across `bastion serve` instances.** It holds only payloads *this* process sent via this
route (or, once wired, any other future sender that shares the same `PendingPayloads`); it is
deliberately not a reimplementation of a durable queue. `engine-rs:EN.8.B`'s eventual queue
replaces `PendingPayloads` as the `PendingLookup` source for payloads the *engine* sent — the
`PendingLookup` seam type itself does not change when that lands, and the two sources are not
expected to ever need to merge, since a restart already drops this registry's contents.

Resolution against it follows the same rules as Section 26.5:

- Sent, then tapped with a matching digest → `Accepted`.
- Sent, then the payload changed, then tapped → `StaleDigest` — rejected, never applied.
- Never sent (or already resolved once) → `UnknownGate`.

### 26.8 Acknowledgement contract (v0.29, `ticket-telegram-answer-callback`)

**Root cause:** Telegram holds a callback button in its loading state until the bot calls
`answerCallbackQuery` for that tap's `callback_query_id`, then times it out silently. Before this
section, `bastion` never made that call — the button spun, un-highlighted, and the operator had
no way to tell a registered decision from a dropped one, even though the server had already
resolved a verdict correctly. Discovered live during `operator-telegram-live-smoke`
(`planning/BA.18.B/tasks.md`), 2026-08-13.

**Every resolved verdict is acknowledged — not only `Accepted`.** `NotifyPollLoop::tick`
(`src/serve/notify/mod.rs`) calls `OperatorTransport::acknowledge` for `Accepted`, `StaleDigest`,
and `UnknownGate` alike, and does so **before** dispatching to the caller-supplied
`VerdictSink` — Telegram callback queries expire, and the sink may block. An acknowledgement
failure is logged and retried at most once; it never discards the response, never skips the sink
dispatch, and never causes the same response to be reprocessed on a later tick.

**The ack handle is opaque, transport-agnostic, and optional.** `OperatorResponse` carries
`ack: Option<AckHandle>` and `message: Option<MessageHandle>` (`src/serve/notify/mod.rs`) — both
newtypes over transport-specific encodings (Telegram: the callback query's `id`, and the
`chat_id`/`message_id` pair the original prompt was sent as) that callers round-trip without
parsing. `OperatorTransport::acknowledge` has a no-op default implementation returning `Ok(())`,
so a transport with no acknowledgement concept (WhatsApp, and every existing test fake) compiles
and behaves correctly against `None` with no change required.

**Telegram's implementation answers, then edits, best-effort:**

1. `answerCallbackQuery` (skipped cleanly when `ack` is `None`) clears the button's loading state
   and shows short text distinguishing the three verdict arms — the accepted option's name, "this
   question changed — no longer valid" for `StaleDigest`, "already answered or no longer pending"
   for `UnknownGate`. Text is clamped to Telegram's 200-character ceiling, counted by
   `.chars().count()`, never bytes.
2. `editMessageText` (attempted whenever `message` is present, independent of whether the answer
   above succeeded) rewrites the original message to show which option was taken and sets an
   **empty** `reply_markup` rather than omitting it — an absent `reply_markup` leaves Telegram's
   prior keyboard in place, which is the second half of the bug this section fixes: without the
   edit, a closed gate keeps inviting a second tap.

The edit is deliberately best-effort: a failed edit after a successful answer is a logged warning,
never an error return. Losing the button-clearing is cosmetic; losing the ack itself is not — that
asymmetry is why the answer call and the edit call are not folded into one all-or-nothing step.
Both calls reuse the existing `classify_http_status` mapping (no second status-mapping path), and
neither logs a request URL — the bot token lives in the Telegram URL path, so only the method name
and verdict arm are logged.

### 26.9 Approve-and-run resolution (v0.30, `ticket-approve-and-run-seams`)

Sections 26.7 and 26.8 get a payload to the operator and acknowledge the tap. This section is what
makes the tap *do* something: a resolved verdict is recorded in an approval ledger and, when
authorized, executed against `engine-rs`'s `ApproveAndRunSeams`.

**The pending-gate lookup is composed, not replaced.** `PendingLookup` resolves in this order:

1. `ApproveAndRunSeams::lookup_pending` — the real source for a gate an engine run queued via
   `ApproveAndRunSeams::drain`.
2. `PendingPayloads` — the process-local registry populated only by `POST /api/notify/test`
   (Section 26.7).

Both are always consulted; neither is skipped. Composing rather than swapping keeps the test-send
route working, which is how the operator smoke test is run. The two id spaces cannot collide by
construction — the engine's `gate_id_for` and the test route's per-request uuid draw from disjoint
generators — so trying the engine source first can never shadow a test-route gate.

**Execution never blocks the server.** `ApproveAndRunSeams::resolve_verdict` is `async` (a matched
`Approved` verdict performs a POST), but the `VerdictSink` is a synchronous `Fn` invoked inline from
`NotifyPollLoop::tick`, which itself runs under `actix_web::rt::spawn` — that is, `spawn_local` on
the single-threaded `LocalSet` that also drives this process's HTTP and WebSocket surface. A
`block_on` there would stall every co-resident request for the duration of the ledger write and the
POST. The resolution is therefore spawned onto that same local set, so the sink returns immediately
and the next `tick` is never queued behind it.

**Failure arms are deliberately distinguished:**

| Arm | Level | Why |
|---|---|---|
| `UnknownGate` | `info` | Expected whenever the gate came from `POST /api/notify/test` rather than a real engine-drained item. Not a regression. |
| `UnknownOption` | `warn` | An option key the payload did not declare — a contract problem, not a failure to act. |
| `Execution` | **`error`** | An authorized decision that failed to POST. This is the one failure mode this path must never swallow. |

**Approval ledger path.** `FileApprovalLedger` resolves its file with the same XDG-first precedence
as the blocked-edge sink (`$XDG_STATE_HOME/bastion/`, else `$HOME/.local/state/bastion/`), differing
only in filename. Unlike the sink it never resolves to `None`: ledger construction touches no
filesystem — the file and its parent are created lazily on first write — so a relative fallback
filename in the process's working directory is a safe always-constructible default when neither
variable is set.

**Ledger attribution is a chat id, not a person.** The `who` recorded on each row is the configured
`BASTION_TELEGRAM_CHAT_ID`, because the bot is configured against a single chat and there is no
operator identity to read. This is honest about *which channel* approved something, but it means the
ledger **cannot distinguish two people with access to that chat**. If this surface ever serves more
than one operator, that is an audit gap requiring a real identity source, not a renamed field.

---

## 27. Session-QA bridge (background bot, no new HTTP surface; `BA.20.C`)

The session-QA bridge lets an agent stuck on a yes/no-shaped question ping the operator over
Telegram and get an answer injected back into its tmux pane, without the operator opening a
terminal. It adds **zero** new HTTP routes or DTOs (Section 8.1's route-table test asserts a
handful of plausible bridge-shaped paths — `/api/session-qa`, `/api/session-qa/webhook`,
`/api/notify/session-qa`, `/telegram/webhook` — all stay 404) — it is a pair of background tasks
(`SessionQaBridge::run_inbound` / `run_outbound`) spawned from `run_server` alongside the existing
poller, entirely separate from Section 26's `OperatorTransport`/approve-reject gate machinery.

**Deliberately a second bot, not a reuse of Section 26's transport.** CodeSessionsBot
(`BASTION_CODESESSIONS_BOT_TOKEN` / `BASTION_CODESESSIONS_CHAT_ID`, [config.md](config.md)) is
distinct from BastionBot (`BASTION_TELEGRAM_BOT_TOKEN` / `BASTION_TELEGRAM_CHAT_ID`) — two bots,
two token pairs, never conflated. `code_sessions_bot_config` (`src/config.rs`) mirrors
`telegram_config`'s both-or-neither rule exactly (same typed `ConfigError::IncompleteTelegramConfig`
error, different env var names); both absent is `Ok(None)` and leaves the bridge disabled, which is
the expected state as of `BA.20.C` (CodeSessionsBot does not exist yet). `run_server` logs only
whether the bridge is enabled/disabled, never the token or chat id.

**Wiring at boot.** When configured, `run_server` constructs one `SessionQaBridge`, wires its
`mpsc::Sender<BlockedEdgeRecord>` onto the always-on `BlockedEdgePoller` via `.with_edge_tx(...)`
(Section 8.1) alongside `.with_hub(...)`, and spawns `run_inbound`/`run_outbound` as two independent
`actix_web::rt::spawn` tasks. When absent, boot is byte-identical to pre-`BA.20.C` — no channel, no
tasks.

**Inbound (crossing → Telegram message).** Each `BlockedEdgeRecord` received over the channel is
turned into a bounded, deduplicated `PendingQuestions` entry (`src/serve/session_qa/mod.rs`) and
sent as a Telegram `sendMessage` with inline-keyboard buttons — one per parsed
`sessions::ask_question::QuestionOption`, whose `OptionKind` is now classified in **three**
variants, not the earlier single escape hatch:

| `OptionKind` | Button rendering | What selecting it does |
|---|---|---|
| `Choice` | `N. label`, unchanged | An ordinary answer to the question. |
| `FreeText` | speech-balloon icon + `label` | Opens Telegram's own free-form reply UI; the operator's typed text becomes the answer. |
| `ChatAbout` | speech-balloon icon + `label` | Closes the widget without answering; the operator's typed text becomes an ordinary next turn to the agent, not an answer. |

`FreeText` and `ChatAbout` are rendered identically (both get the leading speech-balloon icon) because the
operator-facing action is the same first step — type a reply — even though the pane-side effect
differs; `resolve_question_response` (below) is what tells the two apart when the reply arrives.
Callback data is encoded via `encode_question_callback`/`decode_question_callback` under Telegram's
64-byte `callback_data` cap (`QA_CALLBACK_DATA_MAX_BYTES`), reimplemented locally rather than shared
with Section 26's `encode_callback_data` — deliberately, per the task's own instruction, since the
two callback shapes (gate digest vs. question id) are not the same contract.

**Outbound (Telegram tap → tmux injection).** `run_outbound` long-polls `getUpdates` (same
no-webhook rationale as Section 26.3) and, for each `callback_query`, resolves a `QuestionVerdict`
via `resolve_question_response`, always calls `answerCallbackQuery` first (mirroring Section 26.8's
acknowledgement ordering), then injects into the originating session's tmux pane via
`sessions::tmux::send_keys`/`send_keys_no_enter` called **directly** — not through the
`POST /api/sessions/{name}/send` HTTP handler, which is bound to actix-web extractors and not
factored for a direct call (recorded as a task-spec Amendment). The injected keystroke sequence is
**per-`OptionKind`**, verified against a real pane (`BA.ticket.session-qa-freetext-injection`):

- **`Choice`** — digit, then Enter. Moves the highlight and selects in one round trip; unchanged
  from the original single-escape-hatch design.
- **`FreeText`** — digit **without** a trailing Enter (selecting the option opens Telegram's
  free-form reply UI without submitting anything in the pane), then, when the operator's reply
  arrives, the typed text **with** a trailing Enter. Sending Enter on the bare digit here would
  close the widget with an empty free-text answer, before the operator's text ever exists.
- **`ChatAbout`** — digit, then Enter (closes the widget and returns to the normal session
  prompt), then, when the operator's reply arrives, the typed text **with** a trailing Enter, sent
  as an ordinary turn — it does not answer the original question at all.

A second `FreeText`/`ChatAbout` tap while already awaiting a reply replaces the pending state
(newest tap wins) rather than being ignored or erroring. `ChatFollowUpState` is keyed by the
update's own Telegram `chat.id` (not a single global slot), so the state machine is correct even
though only one chat is configured today.

**Auth: no bearer-gated surface added.** The bridge is poll-based (`getUpdates`) and adds **zero**
new HTTP routes, DTOs, or bearer-gated endpoints — there is nothing here for auth middleware to
gate. This is asserted, not merely claimed: Section 8.1's route-table test
(`session_qa_bridge_adds_no_new_route`, `src/serve/mod.rs`) confirms `/api/session-qa`,
`/api/session-qa/webhook`, `/api/notify/session-qa`, `/api/telegram/webhook`, and `/telegram/webhook`
all still 404 with the bridge configured and running. `BA.20.D` re-ran this assertion and adds no
new middleware.

**Testing.** `src/serve/session_qa/tests.rs` covers the full inbound/outbound cycle hermetically
against a fake `QaTelegramClient` and injected capture/inject closures — no real network or tmux
calls. `src/serve/mod.rs`'s `run_server_with_no_codesessions_config_spawns_no_bridge` pins the
disabled-by-default boot decision against a real `run_server` call (bound to an ephemeral port,
aborted after boot) by asserting on captured tracing output.

---

## 28. Fleet-scoped lane API (v0.35, `BA.19.C`)

One read-only route (D25) projecting mev's corpus-wide **lane-segment availability**
computation onto HTTP — one aggregate row per lane SEGMENT across every registered roadmap,
in a single call, with an optional `?epic=<slug>` filter that never fans out to a per-roadmap
call. Lives under the bearer-protected `/api` scope (Section 2). Backing handler:
`src/serve/handlers/lanes.rs`.

**Bastion performs zero derivation of its own here.** The handler calls `mev::lanes_brain(&root)`
(mev's read-only CLI/library surface, not the emit-state planner — it never writes
`planning/lane-availability.json`) exactly once, and copies every field of the resulting
`LaneAvailabilityArtifact`/`LaneAvailabilityEntry`/`SegmentStatus` straight onto the wire DTOs.
Bastion neither reads the on-disk `planning/lane-availability.json` artifact (only as fresh as
the last nightly `mev emit-state --write`) nor shells out to an installed `mev` binary (whose
version can silently drift from the linked crate) — the in-process library call is both fresher
and version-locked, mirroring `handlers/board.rs`'s existing `use mev::brain::config::...` link.

### 28.1 `GET /api/lanes` — fleet-wide lane-segment availability

**Query parameters:**

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `epic` | string | No | — | Roadmap/epic slug. When present and non-blank, filters `segments` to entries whose `roadmap` field equals the slug, in the same call — never a per-roadmap fan-out (the block's explicit out-of-scope). A segment's `roadmap` is used because `SegmentStatus` carries no `epic` field of its own; deriving one from the segment's head block's `epics[]` would be serve computing something this block forbids. Validated against the same HQ `epics[]` registry `GET /api/epics` (Section 17) and `?scope=epic` on `/api/board` (Section 13.2) use, via the shared `board::epic_known`/`hq_epic_registry` helpers — not a second, parallel epic-validation contract. Present-but-blank (`?epic=`) is an error, matching `board.rs::epic_param_missing`'s handling of a blank `scope=epic` slug (Section 13.4) rather than being silently ignored. |

**Request:**

```
GET /api/lanes HTTP/1.1
Authorization: Bearer <token>
```

```
GET /api/lanes?epic=engine-orchestration HTTP/1.1
Authorization: Bearer <token>
```

### 28.2 Response (200 OK): `LanesDto`

```json
{
  "derived_at": "2026-08-18T10:00:00-07:00",
  "degraded": false,
  "segments": [
    {
      "roadmap": "engine-orchestration",
      "lane": "derive",
      "segment": 0,
      "repo": "mev",
      "head": "mev:MV.13.C",
      "availability": "startable",
      "leverage_lanes_freed": 2
    }
  ]
}
```

| Field | Type | Description |
|---|---|---|
| `derived_at` | string | RFC 3339 timestamp of the derivation run, carried verbatim from `LaneAvailabilityArtifact::derived_at`. |
| `degraded` | boolean | `true` when the fleet-lock read that feeds the `held-slot` availability state degraded — lets a consumer tell a corpus with zero live holds apart from one where the fleet-lock read itself failed. Never dropped. |
| `segments` | array of `LaneSegmentDto` | One row per lane segment fleet-wide (or, with `?epic=<slug>`, per segment whose `roadmap` matches). A known-but-unmatched slug legitimately returns an empty array with `200` — a real answer ("no lanes for that epic right now"), not an error. |

#### `LaneSegmentDto`

| Field | Type | Description |
|---|---|---|
| `roadmap` | string | Owning roadmap slug. |
| `lane` | string | Lane name within the roadmap. |
| `segment` | number | Segment index within the lane. |
| `repo` | string | Owning repo slug of the segment's head block. |
| `head` | string \| absent | Canonical `"repo:id"` key of the segment's head block. Absent (not `null`) for a `done` segment — every block in it is closed, so there is no frontier entry and therefore no head. |
| `availability` | string | mev's `SegmentAvailability` variant, kebab-case, carried verbatim (`"done"`, `"held-block"`, `"held-operator"`, `"held-repo-busy"`, `"held-slot"`, or `"startable"`). Deliberately a plain string, not a bastion enum: mev owns this vocabulary and its precedence order, and a mirrored enum would silently stop covering a state the moment mev adds a seventh one. |
| `reason` | string \| absent | Human-readable why. Absent (not `null`) only for `startable` and `done`, which need no explanation. |
| `leverage_lanes_freed` | number | Count of distinct `(roadmap, lane)` pairs freed by closing this segment. Deliberately **not** the same thing as `BlockGraphNodeDto.dependent_count` (Section 23.2), which counts individual dependent blocks corpus-wide, not lanes. **On a `done` segment this value is historical, not actionable** — mev can report a non-zero count here even though the lanes it gated are already free; bastion carries it verbatim regardless, but a Surface reading this field on a `done` segment must not treat it as something still to unblock. |

### 28.3 Error responses

| Condition | HTTP status | Body |
|---|---|---|
| Missing/invalid `Authorization` header | `401 Unauthorized` | JSON `ErrorPayload` (`{"error": "unauthorized", "code": "unauthorized"}`, Section 2.2) |
| `?epic=` present but blank | `404 Not Found` | JSON `ErrorPayload`, code `C005` — reuses `board::epic_param_missing`/`board::epic_error_response` verbatim, same message shape as Section 13.4. |
| `?epic=<slug>` present but absent from the HQ `epics[]` registry (Section 17) | `404 Not Found` | JSON `ErrorPayload`, code `C005` — reuses `board::epic_known`/`board::epic_error_response` verbatim, message naming the unknown slug (`"unknown epic: <slug>"`). |
| Unresolvable brain root (no `brain.toml` walking up from the workspace root), OR `mev::lanes_brain` itself failing — missing/unreadable `brain.toml`, or the underlying block-graph export reporting `truncated: true` | `500 Internal Server Error` | JSON `ErrorPayload`, code `C010`, via `board::brain_root_error_response`, message intact. A `lanes_brain` failure is never mapped to an empty `segments` list — that would read as "nothing to do" when the truth is "the corpus could not be measured". |
| `web::block` thread-pool failure | `500 Internal Server Error` | JSON `ErrorPayload`, code `C010`, via `board::blocking_error_response`. |

### 28.4 Testing

`availability_to_string`, `segment_to_dto`, `artifact_to_dto`, and `filter_segments_to_epic` are
pure and unit-tested with no filesystem access — including the `leverage_lanes_freed`-on-`done`
carryover case above and the ABSENT-not-`null` serialization of `head`/`reason` when `None`. The
thin `web::block` I/O shell (`get_lanes`) and route wiring in `src/serve/mod.rs` (registered in
**both** the production `App` and the test `build_app`, alongside `handlers::board::get_board`)
are covered by `#[actix_web::test]` integration tests asserting the bearer-auth `401` and a `200`
against a fixture brain root, per the block's own task-4 test plan. Not tested here: mev's
availability computation, its six states, or their precedence order — that suite belongs to
`mev:MV.13.C` and is deliberately not duplicated in this repo.

---

## Amendment Log

- **2026-08-18 — v0.34 → v0.35 (`BA.19.C`, additive):** Added Section 28 (Fleet-scoped lane API) —
  `GET /api/lanes[?epic=<slug>]`, one aggregate row per lane SEGMENT across every registered
  roadmap in a single call, a pure pass-through over `mev::lanes_brain` (mev's read-only
  CLI/library surface, never the emit-state planner) with zero availability derivation performed
  by bastion. New typeshared DTOs `LanesDto` (`derived_at`/`degraded`/`segments`) and
  `LaneSegmentDto` (`roadmap`/`lane`/`segment`/`repo`/`head`/`availability`/`reason`/
  `leverage_lanes_freed`), mirroring `mev::brain::availability::SegmentStatus` flattened with its
  `LaneAvailabilityEntry.leverage`. `availability` is a plain string carrying mev's
  `SegmentAvailability` variant verbatim (not a mirrored bastion enum — mev owns that vocabulary);
  `head`/`reason` are absent (not `null`) on `done`/`startable` segments respectively, per the
  established board-DTO convention. `leverage_lanes_freed` is deliberately distinct from
  `BlockGraphNodeDto.dependent_count` (Section 23.2) — it counts distinct `(roadmap, lane)` pairs,
  not dependent blocks — and is carried verbatim even when historical on a `done` segment (the
  `lanes-freed-is-history-on-done-segments` carryover): a Surface must not read it as actionable
  there. `?epic=<slug>` filters `segments` to entries whose `roadmap` equals the slug, in the same
  call (never a per-roadmap fan-out, the block's explicit out-of-scope), validated against the
  same HQ `epics[]` registry `?scope=epic` on `/api/board` (Section 13.2) already uses via the
  shared `board::epic_known`/`hq_epic_registry` helpers; an unknown slug is `404`/`C005`, a
  known-but-unmatched slug is `200` with an empty `segments` array. Bearer auth inherited from the
  `/api` scope, registered in both the production `App` and the test `build_app` (mirroring
  `handlers::board::get_board`'s two registration sites) — the auth policy table (Section 2.3)
  already covers `/api/*` routes generically (per the v0.12 precedent) and gained no new row.
  `types/serve.ts` regenerated (no drift); the contract-corpus goldens under
  `types/contract-corpus/` are unaffected (no `*_scenarios` module yet exists for this route).
  No breaking change — new route, new DTOs, no existing field renamed, retyped, or removed.
- **2026-08-17 — v0.33 → v0.34 (`BA.19.B`, additive):** `BoardBlockDto` (Section 13.3) gains
  `effective_priority: Option<u8>`, carried verbatim from mev's already-computed, corpus-wide
  min-propagation (`mev::brain::block_graph::BlockGraphNode::effective_priority`,
  `../mev/src/brain/block_graph.rs:85`, populated at `:400`) — bastion derives nothing. Rides the
  same `?graph=1` opt-in gate and single per-request `mev::build_block_graph_export` call as its
  v0.19 siblings `dependent_count`/`ready`/`unmet_count`, and is populated on all five lanes
  (`now`/`next`/`blocked`/`deferred`/`finished`) like `dependent_count`/`ready` (unlike
  `unmet_count`, this value is lane-independent). `skip_serializing_if = "Option::is_none"`, so a
  request without `?graph=1` — and every pre-v0.34 golden — serializes byte-identical to before
  this bump; goldens generated without `?graph=1` were confirmed byte-identical after
  regeneration. `board__dependent_count.json` (the `?graph=1` golden) did not gain a visible
  `effective_priority` key in this pass, because its fixture blocks all carry `priority: null` and
  no hot dependents, so mev's min-propagation legitimately leaves the value absent for every node
  in that fixture — confirmed against `mev`'s own
  `effective_priorities_absent_own_priority_and_no_hot_dependents_is_absent` test, not assumed.
  This is a correctness fix only, not a visible ranking or briefing change (per the block's own
  grounding measurement): min-propagation changes the rank of 1 of 70 startable blocks fleet-wide
  as of 2026-08-13, leaving the four triggering picks tied. `types/serve.ts` regenerated (no
  drift). bastion-web's `board-view.ts:547-552` already duck-types a read of this field — no
  client-side change required or made. No breaking change — no field renamed, retyped, or removed.
- **2026-08-17 — v0.32 → v0.33 (`BA.ticket.block-fields-serve-dto`, additive):** `BoardBlockDto`
  (Section 13.3) gains five optional fields carried verbatim from `okf_core::TrackBlock`, all
  `skip_serializing_if = "Option::is_none"` so a block carrying none of them (still the common
  case, since 0 of 894 blocks carried a `description` before this ticket's companion D65 backfill)
  serializes byte-identical to pre-v0.33: `description`, `created` (`YYYY-MM-DD`), `closed`
  (`YYYY-MM-DD`), `commit` (the git hash that closed the block), and `origin` (`BlockOriginDto` —
  `{ kind: "backlog" | "carryover", slug }`, a local mirror of `okf_core::state::Origin` following
  the same mirror convention as `BlockOriginDto`'s neighbors, since that upstream type is not
  itself typeshare-annotated). Populated by both `enrich_block` (`handlers/board.rs`) and its
  epic-board sibling (`handlers/epics.rs`) from the same `TrackBlock` lookup that already supplies
  `epics`/`wave`/`priority`/`due`/`track`. `types/serve.ts` regenerated; a `#[cfg(test)]` assertion
  pins the five new field names present in the generated output, closing the same generated-file
  drift gap Section 19's Amendment Log entry (v0.32) names for the `BlockedBy` payload structs.
  No breaking change — no field renamed, retyped, or removed. Data-contract re-pin per the brain's
  update protocol: this document is bastion's canonical DTO contract for the fields TrackBlock
  contributes to the board/epics read surface (`docs/data-contract.md` is a separate, narrower pin
  — the orchestrator↔bastion `events`/`node_runs` contract — and carries no block-graph fields, so
  it is untouched by this bump); noted in `planning/status.md`.

- **2026-08-14 — Section 27 corrected (`BA.20.D`, no contract version bump — corrects the docs
  only, no route or DTO changed):** Section 27's inbound description previously read "one
  escape-hatch button (distinguished by a leading emoji glyph)" — the real widget has **two**
  trailing option kinds with different semantics, `OptionKind::FreeText` and `OptionKind::ChatAbout`,
  classified structurally by `sessions::ask_question::parse_ask_question` (see
  [sessions.md](sessions.md)). The outbound paragraph previously described injection as a single
  `sessions::tmux::send_keys` call carrying "the operator's answer" — the shipped behaviour is a
  **per-`OptionKind` keystroke sequence** (`BA.ticket.session-qa-freetext-injection`): `Choice` is
  digit+Enter; `FreeText` is digit with no Enter, then the operator's text, then Enter; `ChatAbout`
  is digit+Enter to close the widget, then the text+Enter as an ordinary turn. Both corrections are
  documentation-only — no HTTP route, DTO, or wire-frame shape changed, so this entry does not bump
  the contract version. Also states explicitly, per this block's auth-surface acceptance criterion,
  that the bridge adds no bearer-gated HTTP surface, naming Section 8.1's
  `session_qa_bridge_adds_no_new_route` route-table test as the standing evidence.

- **2026-08-15 — v0.31 → v0.32 (`BA.19.A`, additive):** Section 19's generated-types artifact
  (`types/serve.ts`) now also mirrors four types from the sibling `okf-core` schema crate —
  `BlockDep`, `ExternalDep`, `OperatorDep`, `ApprovalDep`, the payload structs backing
  `okf_core::state::BlockedBy`'s variants — by widening `scripts/gen-types.sh`'s typeshare
  invocation to scan `../okf-core/src` alongside `src/serve`. Purely additive: no existing
  declaration in `types/serve.ts` moved or changed, no HTTP route or bastion-owned DTO changed,
  and the pre-existing dangling `blocked_by?: BlockedBy[]` reference is untouched — typeshare
  cannot emit the internally-tagged `BlockedBy` enum itself, so the four-arm union stays
  hand-written downstream in BastionWeb (`BW.16.A`). See
  **D19** (`planning/decisions/D19-typeshare-scan-includes-okf-core.md`).

- **2026-08-14 — v0.30 → v0.31 (`BA.20.A`, additive; documented retroactively at chain close-out):**
  `SessionDto` gained the optional `blocked_reason` field (`"permission_prompt"` |
  `"awaiting_question"`), populated from `Session::blocked_reason` via the `detect` manifest's new
  `reason` key. Additive and absent-when-`None` (the key is omitted, never `null`), so no existing
  consumer breaks. `AgentState` was deliberately NOT given a fifth variant — it is matched
  exhaustively in 14 files plus a hand-enumerated wire test, and this field is the sub-classification
  that avoids that ripple. **Two corrections folded in:** the field shipped in `BA.20.A` with no doc
  coverage at all (the plan deferred one docs pass to `BA.20.D`, which is blocked on an external
  dependency, so leaving a live wire field undocumented indefinitely was the worse trade); and the
  `**Version:**` line still read `v0.29` while the document heading already read `v0.30` — the
  2026-08-13 `ticket-approve-and-run-seams` bump updated the heading only. Both now read v0.31.

- **2026-08-14 — Section 27 added (`BA.20.C`, no contract version bump — adds no HTTP route or
  DTO):** New Section 27, "Session-QA bridge". A second Telegram bot (CodeSessionsBot,
  `BASTION_CODESESSIONS_BOT_TOKEN`/`BASTION_CODESESSIONS_CHAT_ID`) distinct from Section 26's
  BastionBot lets an agent post a bounded yes/no-shaped question against a `BlockedEdgeCrossed`
  crossing and get the operator's tap injected back into the originating tmux pane via
  `sessions::tmux::send_keys` directly, bypassing the HTTP send route. `BlockedEdgePoller` gained
  `.with_edge_tx(...)` (Section 8.1) as an additive second consumer of each crossing, alongside
  `.with_hub(...)`; absent CodeSessionsBot config, boot is unchanged from pre-`BA.20.C`. No DTO,
  route, or wire-frame shape changed, so the `serve-api` contract version is not bumped.

- **2026-08-13 — v0.29 → v0.30 (`ticket-approve-and-run-seams`):** New Section 26.9,
  "Approve-and-run resolution". Before this, a tap on a real engine-queued approval resolved to
  `UnknownGate` and the `VerdictSink` only logged — no ledger row, no execution — because
  `PendingLookup` pointed solely at the `/api/notify/test` registry. `PendingLookup` is now
  **composed** (engine queue first, test registry as fallback, neither skipped), and the sink calls
  `ApproveAndRunSeams::resolve_verdict`, spawned onto the same actix local set rather than blocked
  on, so the single-threaded worker serving HTTP and WS is never stalled behind a ledger write or a
  POST. `ResponseVerdict::Accepted`/`StaleDigest` widened to carry the digest and decision time that
  `ApproveAndRunVerdict` needs. Failure arms are levelled deliberately: `UnknownGate` at `info` (the
  expected arm for a test-route gate), `UnknownOption` at `warn`, and `Execution` at **`error`** —
  an authorized decision that failed to POST must never be swallowed. Documents the approval
  ledger's XDG path resolution and records the known attribution limit: rows are attributed to the
  Telegram chat id, so the ledger cannot distinguish two operators sharing one chat.

- **2026-08-13 — v0.28 → v0.29 (`ticket-telegram-answer-callback`):** New Section 26.8,
  "Acknowledgement contract". Root cause: Telegram holds a callback button in its loading state
  until `answerCallbackQuery` is called, and `bastion` never called it — a tap resolved correctly
  server-side but the operator got no feedback and the closed gate's message kept its live
  buttons, inviting a second tap. `NotifyPollLoop::tick` now calls the new
  `OperatorTransport::acknowledge` for every resolved verdict (`Accepted`, `StaleDigest`,
  `UnknownGate`), before dispatching to the `VerdictSink`; a failed ack is logged, retried once,
  and never discards or double-applies the response. `OperatorResponse` gains two opaque,
  transport-agnostic fields — `ack: Option<AckHandle>` and `message: Option<MessageHandle>` — with
  `acknowledge` no-op-defaulted so WhatsApp and every existing test fake need no change.
  Telegram's implementation calls `answerCallbackQuery` (distinct, ≤200-char text per verdict arm,
  clamped by `.chars().count()`) then, best-effort, `editMessageText` with an **empty**
  `reply_markup` to drop the live buttons and show the chosen option — a failed edit after a
  successful answer is a logged warning, not an error. No token or chat id is logged by either
  call. No DTO or route changed; this is a behavioural amendment to the existing background loop
  and Telegram transport impl, not a new wire shape.

- **2026-08-13 — v0.27 → v0.28 (`ticket-notify-send-trigger`):** New Section 26.7, `POST
  /api/notify/test` — an authenticated route mounted inside the existing `web::scope("/api")`
  (never at the app root; the route-table test is extended to assert this) that sends one real
  `ValidatedOperatorPayload` (fixed 2-option test content, validated through the same
  `engine_core::operator::validate` contract Section 26.1 describes) over the configured
  `OperatorTransport` and returns `NotifyTestResponseDto { gate_id, digest }`. This makes three of
  `BA.18.B`'s previously-unverifiable acceptance criteria — inline render, response resolving back
  to gate + digest, stale-digest rejection — exercisable without waiting on
  `engine-rs:EN.8.B`'s queue. Backing it: a new bounded, oldest-first-evicting, process-local
  `PendingPayloads` registry (`src/serve/notify/mod.rs`) wired into `run_server` as the
  `PendingLookup` source, replacing the `|_gate_id: &str| None` stub Section 26.6 described; an
  `Accepted` resolution removes the entry so a replayed tap of the same button resolves to
  `UnknownGate` on its second arrival. Error responses: 401 (no bearer token, existing
  middleware), 503 + `C005` naming the missing env var(s) by name only when the transport is
  unconfigured, 502 with a retryable/permanent-distinguishing `code` on a transport send failure.
  No token or chat id is read, echoed, or logged by this route — it takes no request body at all.
  `PendingPayloads` is explicitly in-memory and process-local; it does not survive a restart and
  is not shared across instances, and `engine-rs:EN.8.B`'s eventual queue replaces it as the
  `PendingLookup` source without the seam type changing. One new typeshared DTO,
  `NotifyTestResponseDto`; `types/serve.ts` regenerated (`scripts/gen-types.sh`);
  `scripts/check-typeshare-drift.sh` and `scripts/check-contract-corpus-drift.sh` pass.

- **2026-08-13 — v0.26 → v0.27 (`BA.18.B`):** New Section 26, "Operator-notification transport" —
  an outbound-only background capability (`src/serve/notify/`), not a REST/WS route. Delivers an
  `engine-rs:EN.8.A` `ValidatedOperatorPayload` to a human over Telegram, rendered inline with its
  declared 2-3 tap options, and resolves the response back to `gate_id` via `getUpdates`
  long-polling — no webhook route, no listening socket (deliberate, per
  `brain:HQ.ticket.tailscale-bind-and-token-rotation`). A response whose digest no longer matches
  the payload it answers is rejected as stale, never applied. Configured by two new optional env
  vars, `BASTION_TELEGRAM_BOT_TOKEN` / `BASTION_TELEGRAM_CHAT_ID` (Section 26.4; also
  [config.md](config.md)) — both absent leaves `bastion serve` byte-identical to before this
  block; exactly one present is a typed startup error. Neither var, nor the bot token itself,
  appears anywhere in this document, `.env.example`, a test fixture, or a log line. This section
  adds no route, no DTO, and no typeshared type — `types/serve.ts` is unaffected and
  `scripts/check-typeshare-drift.sh` / `scripts/check-contract-corpus-drift.sh` were not expected
  to (and did not) flag any change.

- **2026-08-12 — v0.25 → v0.26 (`BA.ticket.session-dto-agent-state`):** `SessionDto`'s
  `From<&Session>` (`src/serve/dto.rs`) previously discarded `agent_state`, so every consumer of
  the sessions REST/WS surface could list sessions but not tell whether any was working, idle, or
  blocked. `SessionDto` gains a typeshared `agent_state: String` field (`"idle"` / `"working"` /
  `"blocked"` / `"unknown"`), populated verbatim from `Session::agent_state` (`detect/`), on both
  `GET /api/sessions` (Section 10.1) and the `"sessions"` WS push payload (Section 7.5) — full
  parity across both surfaces from the start, unlike `last_line`'s v0.5 REST/WS split. No change
  to `classify_state` or to how attachment is treated — attachment still feeds the session lease
  (`engine-rs:EN.9.B`), not the state classifier. Unblocks `bastion-ui:BU.ticket.session-agent-
  state` and `bastion-web:BW.ticket.approval-ledger-view`, both previously held on this landing.
  `types/serve.ts` regenerated (`scripts/gen-types.sh`); `scripts/check-typeshare-drift.sh` and
  `scripts/check-contract-corpus-drift.sh` pass.

- **2026-08-10 — v0.24 → v0.25 (`BA.ticket.carryover-triage-dto`):** `GET /api/attention`'s
  `stale_carryover` lane (Section 15) stops gating membership on `carryover_stale_age` alone.
  `build_attention` now calls `mev::brain::carryover::rank_carryover` (with
  `evaluate_carryover(..., allow_exec: false)` feeding it the full carryover entry set — never a
  stale-filtered subset) and projects its ranking verbatim, preserving mev's sort order. **This is
  a semantic change a client must account for**: the array previously carried roughly 6 entries
  fleet-wide (only items already past their per-`kind` staleness threshold); it now carries
  roughly 138 (every evaluated entry, ranked into BLOCKING/HOT/AGING/STANDING). A client that
  sized a view (pagination, a fixed-height list, a polling budget) around the old ~6-entry
  behaviour will be surprised by the new volume. `AttentionCarryoverDto` gains `lane`, `priority`,
  `effective_priority`, `unmet_blocks`, `finding_id`, `clears_when_satisfied` (all typeshared,
  verbatim from `mev::CarryoverRanking` — see the field table in Section 15.3 and the new
  `docs/carryover-contract.md`, mev's contract pinned at v1.0.0 per the D20 pattern). **The one
  non-additive change:** `age_days` widens from a non-optional `number` to `number | absent` —
  absent for a currently-snoozed entry or one whose anchor date does not parse, both of which now
  reach the board instead of being excluded pre-ranking. There is deliberately no `blocking: bool`
  field; derive it client-side from `!unmet_blocks.is_empty()`. `types/serve.ts` regenerated
  (`scripts/gen-types.sh`); `scripts/check-typeshare-drift.sh` and
  `scripts/check-contract-corpus-drift.sh` pass; `types/contract-corpus/attention__populated.json`
  regenerated and inspected line-by-line (new fields present, `clears_when`/`reviewed` drop to
  absent keys where `None`, entry count grows as expected — no other drift).

- **2026-08-04 — v0.23 → v0.24 (`BA.ticket.report-skipped-workspaces`, ask A9):** `GET
  /api/workflows` gains an opt-in `?with_skipped=1` query param (Section 11.6) that returns
  `{entries, skipped}` instead of the bare `RepoWorkflowStateDto[]`, naming which registered
  workspaces `collect_flow_states`'s walk could not fully account for and why — restoring the
  per-repo reachability signal deleted alongside `ListRunsResult`/`RunFetchFailure`/
  `<DegradedNotice>` when bastion-web moved `listRuns()` onto this aggregate (`d421aba`;
  `bastion-web/lib/workflows.ts:196-208`, `bastion-web/app/(cockpit)/engine/page.tsx:298-310`).
  Added typeshared `SkippedWorkspaceDto` (`repo`, `reason`) and `WorkflowsAggregateDto`
  (`entries`, `skipped`) to `src/serve/dto.rs`. Three `reason` values, first-match-wins:
  `unreadable_root` (registered path not a readable directory), `no_planning_dir` (root readable,
  `planning/` is not), `malformed_flow_state` (>=1 `sdlc-flow-state.json` found but failed to
  parse) — a repo with a readable `planning/` and zero flow-state files is healthy and reported in
  neither array, and a `malformed_flow_state` repo still contributes every flow state that did
  parse to `entries`. Skip classification is bookkeeping over the same single walk
  `collect_flow_states` already performs per workspace (now factored through a private
  `scan_flow_states`); `?with_skipped=1` adds no second traversal, so the opt-in exists solely to
  keep the default (no-param) response non-breaking, not as a cost gate. The default response and
  both existing `workflows__empty` / `workflows__populated` contract-corpus goldens are
  byte-identical to v0.23; a new `workflows__skipped` golden freezes the envelope shape, and
  `collect_flow_states`, `collect_repo_workflows`, and `collect_all_workflows` all keep their pre-
  v0.24 signatures and behaviour, so `src/serve/ws/server.rs`'s flow-watch loop, `handlers/
  runs.rs`'s `?with_repo=1` join (Section 14.1), and Section 11.4's per-repo route are unaffected.
  The backlog's original sketch — an unconditional `{entries, skipped}` envelope flip — was
  rejected as a breaking change: bastion-web's `lib/workflows.ts:218` reads today's response as a
  bare array, and the repo's established precedent for a contract-affecting addition on a hot poll
  path is an opt-in param (`?with_repo=1`, A7; `?graph=1`, A5), not an unconditional shape change.
- **2026-08-03 — v0.22 → v0.23 (`BA.11.N`, D17 — `planning/decisions/
  D17-live-run-stream-ownership.md`):** Bastion now pushes run-level aggregate status transitions
  over the existing bearer-authed `/ws` hub, as a new subscribable `"runs"` topic (Section 6). Two
  deliberate design decisions, settled at spec time rather than left open: **delivery is a
  subscribable topic** (the `sessions` precedent — a poller gated on subscriber count, starting on
  the first `runs` subscriber and stopping on the last), not the always-on `broadcast_all` the
  `workflow_done` push uses and not a per-run `run:<uuid>` topic; and **unavailability is signalled
  explicitly**, via a new `event{run_stream_status}` frame (`available`/`reason`) pushed to a
  connection the instant it subscribes to `runs`, so a client learns engine-mount availability
  without inferring it from silence (Section 8.3). Added `RunTransitionPayload`
  (`run_id`/`status`/`terminal`/`spec_slug`) and `RunStreamStatusPayload` (`available`/`reason`) as
  typeshared DTOs; added Section 8.3 documenting the emit predicate (status change on a live run,
  and the `list_active()`-disappearance edge read back via `get_record`), no-first-observation and
  no-unchanged-status rules, and D17 constraint 1 (`terminal` here is lifecycle-terminal only — a
  `"suspended"` run always pairs with `terminal: false`, unlike the embedded engine's own
  `publish_suspended`, which sends `terminal: true` with `status: "suspended"` on its own protocol).
  Corrected Section 14's now-stale "no fine-grained push" framing: node-level granularity is still
  not pushed (Section 14.4, unchanged), but run-level aggregate status now is; the former "Gotcha for
  whoever builds `BA.11.N`" paragraph is replaced with a statement of how this block resolved it.
  `GET /api/runs` and `GET /api/runs/{id}` — request shape, response shape, status codes — are
  byte-identical to v0.22 (D17 constraint 3): no route, DTO field, or contract-corpus golden under
  `types/contract-corpus/` moved. `types/serve.ts` regenerated (`scripts/gen-types.sh`), adding only
  `RunTransitionPayload`/`RunStreamStatusPayload`; `scripts/check-typeshare-drift.sh` passes clean.
  Also corrected Section 21's versioning-policy line, which had lagged four version bumps behind the
  frontmatter (stuck at "the current contract is v0.18" since v0.19) — it now reads v0.23.
  **Post-implementation live-testing addendum (same day, same version — no wire change):** Section
  8.3 gained a *Sampling limitation* paragraph. Live testing against a running `bastion serve` proved
  a run that starts and finishes inside one poll interval yields **no frames at all**, not even a
  terminal one — edge 1 needs a prior observation and edge 2 needs the run to have been seen before
  it can be noticed leaving. The poll fallback shares the blind spot exactly (`GET /api/runs`
  returned `[]` across the same window, reading the identical `list_active()` set), so this is the
  deliberate trade of D17's poll-diff design rather than a regression against Section 14 — but it was
  undocumented, and a consumer reading Section 8.3 alone could reasonably have read stream silence as
  "no run happened." Documented rather than fixed: closing the window entirely would require an
  event-sourced feed, which D17 explicitly rejected.
  **Live-testing defect fix (same day, same version — the wire *shape* is unchanged, but a status
  *value* changes).** The same live session caught a real bug and it is fixed here, not deferred: the
  disappearance edge derived its final status with `derive_run_status`, the **live** aggregate, which
  reports `pending` whenever any node is pending. A finished workflow legitimately leaves every
  never-taken branch `Pending` forever, so a *successful* `SDLC_FLOW` run (15 `success` nodes, 4
  `pending` untaken router branches — real run `3adfdd1b`, `smoke-sdlc-flow`) emitted
  `status: "pending", terminal: true` while the engine's own readback said `succeeded`. Edge 2 now
  uses a terminal-specific derivation mirroring `engine-serve`'s `derive_terminal_status`
  (cancelled → budget_halted → failed → success, ignoring still-pending nodes), documented as a table
  in Section 8.3. `GET /api/runs` never exposed this because a terminal run has already left
  `list_active()` and so never appears there — the stream's disappearance edge is the first consumer
  to derive a status from a *terminal* snapshot, and it surfaced the latent weakness. No route, DTO
  field, or golden moved; only the value carried in `run_transition.status` at the terminal edge.

- **2026-08-02 — Section 14 corrected (`ticket-stream-ownership-decision`, ask A8,
  `planning/arch-review-asks-bastion-web/notes.md`; no version bump — wire shape unchanged):**
  Section 14 previously stated "There is no SSE/WS push and no `engine-serve` change in this API,"
  which was false — bastion ships a bearer-authed `GET /ws` hub (Section 4) with a `sessions`/`pane`
  topic vocabulary (Section 6) and an `event{workflow_done}` completion push (`BA.11.D`,
  `src/serve/poll.rs`) that predates this correction. Reworded to state accurately that
  `GET /api/runs`/`GET /api/runs/{id}` are a read-only snapshot with no fine-grained
  transition-by-transition push, while `/ws`'s `workflow_done` event exists and is completion-only.
  Replaced the unqualified "proposed `BA.11.N`" phrasing (here and in the v0.7 → v0.8 log entry
  below) with a reference to **D17** (`planning/decisions/D17-live-run-stream-ownership.md`), which
  resolves the ambiguity A8 raised: `BA.11.N` is promoted to a tracked block scoped to bastion
  pushing run transitions over the existing `/ws` hub (poll-diff against the in-process
  `LiveStateStore`, per `BA.11.D`'s pattern) — no `engine-serve` change, no second streaming
  protocol. Also documented the suspended/terminal gotcha D17 carries forward as a binding
  constraint: the engine's `publish_suspended` sends `terminal: true` **with** `status: "suspended"`
  (`engine-rs/crates/engine-serve/src/stream.rs:185-191`), so a naive `if (frame.terminal) done`
  would misclassify a paused run as finished. No route, DTO, or wire shape changed — documentation
  correction only.

- **2026-08-02 — v0.21 → v0.22 (`ticket-run-summary-repo-join`, ask A7,
  `planning/arch-review-asks-bastion-web/notes.md`):** Section 14.1's `RunSummaryDto` gains
  `repo: Option<String>`, resolved by an **exact `run_id` match** against every registered
  workspace's flow state (`RepoWorkflowStateDto` from `collect_all_workflows`, Section 11.6, A2) —
  never a substring, prefix, or spec-slug similarity guess, since a wrong repo label is strictly
  worse than an absent one. Absent (not `null`) when no flow state carries a run's `run_id`,
  preserving the honest-degradation posture bastion-web already built for the bare-UUID fallback.
  Gated behind an opt-in `?with_repo=1` query param (Section 14.1), mirroring `/api/board`'s
  `?graph=1` (A5): task 1 measured the registry walk `collect_all_workflows` requires as ~6x the
  unenriched `GET /api/runs` baseline (median 2.51ms vs. 0.41ms/request over 20 requests against
  the live HQ registry, 23 repos, 0 active runs — see `planning/ticket-run-summary-repo-join/
  tasks.md`'s Notes), so the route's hottest consumer (bastion-web's ~2-6s run rail) does not pay
  for the walk unless it opts in — settled with bastion-web ahead of this ticket, 2026-08-02.
  `?with_repo=1`, not `?repo=1`: `repo` already carries filter semantics elsewhere in this contract
  (`BlockGraphQuery.repo`, Section 23.1; the documented "No `?repo=` filter" on `/api/costs`,
  Section 24), and `?with_repo=1` is an enrichment flag, never a filter — it narrows nothing.
  Section 24's "No `?repo=` filter" note is amended to clarify it still holds: the `events`
  contract still carries no repo dimension, and `/api/runs`' new `repo` field sidesteps that gap
  via the flow-state join rather than closing it. Serialized with
  `#[serde(skip_serializing_if = "Option::is_none")]`, matching `spec_slug`/`workflow_type`'s
  absent-key convention — the nine pre-existing `runs__*` contract-corpus goldens stay
  byte-identical, and a tenth, `runs__with-repo`, freezes the populated shape.
  `types/serve.ts` regenerated (`scripts/gen-types.sh`); `scripts/check-typeshare-drift.sh` and
  `scripts/check-contract-corpus-drift.sh` both pass clean.

- **2026-08-02 — costs' populated 200 shape frozen via a compile-time fetch seam
  (`ticket-costs-200-contract-golden`; no version bump):** `get_costs` (`src/serve/handlers/costs.rs`)
  is reduced to a delegation to `get_costs_with<F, Fut>`, a generic taking the row-fetching function
  as a compile-time parameter — production passes `db::costs::fetch_all_runs`, and only test code
  linking against `get_costs_with` directly can supply fixture rows. There is no env var, config
  key, `app_data` entry, or CLI flag by which a deployed `bastion serve` could serve fabricated cost
  figures. `types/contract-corpus/` grows from 32 to 37 goldens, adding `costs__populated`,
  `costs__budget-breached`, `costs__empty`, `costs__windowed`, and `costs__db-error` (the previously
  untested `503 + C009` branch). Section 25.6 rewritten: the "why costs' populated 200 shape is not
  in the corpus" exclusion is removed and replaced by a residual-gap note — the goldens cover
  `Config::load` onward but not the `events`-row-to-`WorkflowRun` deserialization itself, which
  stays covered by the `#[ignore]`d integration test in `src/db/costs.rs`. **No wire shape
  changed** — `get_costs_with` is a pure extraction of the existing handler body, so no version bump
  and `types/serve.ts` is untouched. Regenerated three times (plain, second run for idempotency, and
  a third with both `BASTION_MAX_*` env vars exported to arbitrary values in the invoking shell) with
  zero diff each time — proof the budget caps are pinned inside the scenarios via `EnvVarGuard`
  rather than inherited from the environment; `scripts/check-contract-corpus-drift.sh` passes clean.

- **2026-08-01 — contract corpus extended to the five uncovered routes
  (`ticket-contract-corpus-uncovered-routes`; no version bump):** `types/contract-corpus/` grows
  from 22 to 32 goldens, adding `GET /api/workflows` (v0.20, 2 scenarios),
  `GET /api/repos/{name}/handoff` (v0.18 `HandoffInfoDto`, 3 scenarios — including both distinct
  404 codes, `C005` unknown-repo and `C002` missing-file), `GET /api/epics` (post-v0.21, 2
  scenarios, freezing both the authored `weight: 85` and the unauthored `weight: null` branches),
  `GET /api/blocks/graph` (v0.13, 1 scenario at a pinned `max_nodes=50`), and `GET /api/costs`
  (v0.15, 2 scenarios — `400 C006` and `503 C005`; the populated `200` shape is deliberately
  excluded because it needs a live Postgres the corpus cannot assume, rationale in Section 25.6).
  New Section 25.6 carries the full scenario inventory. **No wire shape changed** — every addition
  is `#[cfg(test)]` scenario code plus generated goldens plus documentation, so **no version bump**
  and `types/serve.ts` is untouched. One genuine fix rode along: redaction rule 3 now matches the
  *canonicalized* temp root as well as `std::env::temp_dir()`'s, because macOS's
  `/var → /private/var` symlink meant `BlockGraphDto.root` (which is canonicalized upstream in
  `mev`) leaked a live per-run temp path past the old single-prefix check — caught by generating
  the golden, which is exactly the failure mode Section 25.5 says to stop and diagnose rather than
  regenerate past. `DotenvShadow` moved from `serve::mod`'s test module into `crate::testsupport`
  beside the env lock it requires as a witness, so the costs scenario and the `serve` route tests
  share one implementation. Corpus regenerated twice byte-identically;
  `scripts/check-contract-corpus-drift.sh` passes clean.

- **2026-08-01 — v0.20 → v0.21 (`BA.ticket.epic-weight-dto`):** Section 17.2's `EpicDto` gains
  `weight: Option<u8>`, the authored initiative weight `okf_core::Epic` has carried since
  `MV.11.A`. Pure additive passthrough: `build_epics` (`src/serve/handlers/epics.rs`) copies
  `epic.weight` verbatim and bastion performs **zero** derivation — mev's `check_epics` owns the
  `0..=100` range policy (`E_STATE_EPIC_BAD_WEIGHT`), so an out-of-policy authored value reaches
  the wire unclamped (asserted by test, alongside the `0`/`100`/`255` boundaries), following the
  `BA.11.S`/`BA.17.A` zero-derivation precedent. Serialized with `#[serde(default)]` and **no**
  `skip_serializing_if` — matching `EpicDto`'s own convention (`description`/`status`/`plan` all
  emit `null` when absent), deliberately *not* `BoardBlockDto`'s absent-key convention, so
  unauthored reads as `null` on the wire while staying distinguishable from an authored `0`.
  No new route, no new handler, no query parameter. Unblocks bastion-web's ranking of initiatives
  by authored weight. `types/serve.ts` regenerated (`scripts/gen-types.sh`);
  `scripts/check-typeshare-drift.sh` passes clean. No `epics` contract-corpus goldens exist yet, so
  `scripts/check-contract-corpus-drift.sh` is unaffected — `ticket-contract-corpus-uncovered-routes`
  runs after this ticket and freezes the post-`weight` shape.

- **2026-08-01 — contract-corpus goldens added (ask A4, `planning/arch-review-asks-bastion-web/notes.md`;
  no version bump):** Added Section 25 and `types/contract-corpus/` — checked-in golden JSON per
  (route × scenario), produced by the real `serve` handlers and real serializer via the
  `#[cfg(test)]` dump harness in `src/serve/contract_corpus.rs`, plus `scripts/gen-contract-corpus.sh`
  / `scripts/check-contract-corpus-drift.sh` (the latter wired into `planning/harness.json` as a
  gating check). Consumer side: `../bastion-web/ticket-stub-fidelity-check/`. Purely additive
  tooling and documentation — no wire shape changed, so no version bump. Per Section 25.5, any
  *future* PR where a golden's content changes is itself a contract change requiring its own
  version bump and Amendment Log entry.

- **2026-08-01 — v0.19 → v0.20 (ask A2, `planning/arch-review-asks-bastion-web/notes.md`):**
  New Section 11.6 route, `GET /api/workflows` — a cross-repo flow-state aggregate so consumers stop
  issuing `GET /api/repos` followed by one `GET /api/repos/{name}/workflows` per repo, retiring the
  residual N+1 on bastion-web's `/engine` on-disk band and briefing diff left after A1/A5 already
  removed the hot-path 2s re-fire (`GET /api/runs`). Adds a typeshared `RepoWorkflowStateDto` in
  `src/serve/dto.rs` — a flat struct mirroring `WorkflowStateDto`'s fields plus a `repo: String`
  (via `From<(String, WorkflowStateDto)>`), chosen over `#[serde(flatten)]` composition after
  regenerating and inspecting typeshare's output for both shapes; the flat mirror is what ships.
  `collect_all_workflows` (`src/serve/handlers/status.rs`) follows the `build_repo_summaries`
  precedent — sorts registered workspace names, resolves each root, and calls the existing
  `collect_flow_states` verbatim per workspace (no second flow-state walk); a repo with no
  `planning/` dir, an unresolvable root, or only malformed `sdlc-flow-state.json` files contributes
  zero entries without failing the request. Output is ordered by `(repo, spec_slug)` — sorting repo
  names is sufficient since `collect_flow_states` already sorts by `spec_slug`. The route is
  registered in both `src/serve/mod.rs` app factories (production and the test `build_app` mirror)
  under the existing bearer-auth `/api` scope; Section 11.4's per-repo
  `GET /api/repos/{name}/workflows` is unchanged. `types/serve.ts` regenerated
  (`scripts/gen-types.sh`); `scripts/check-typeshare-drift.sh` passes clean.

- **2026-08-01 — v0.17 → v0.18 (asks A1 + A3, `planning/arch-review-asks-bastion-web/notes.md`):**
  Two smallest asks from the bastion-web architecture review, shipped together. **A1:** Section
  11.4's `WorkflowStateDto` gains `run_id: Option<String>`, carrying the engine's `events.id` run
  UUID that engine-rs `EN.6.J` already stamps into the top-level `run_id` key of
  `sdlc-flow-state.json` (read by `FlowState`, `#[serde(default)]`, unchanged for states written
  before that fix or by base-template's JS `sdlc-flow.js` engine, neither of which sets it).
  Serializes with `skip_serializing_if = "Option::is_none"` (the `BoardBlockDto.last_touched`
  precedent) so `None` is an absent key, not `null` — a consumer can distinguish "predates the
  stamp" from "field not understood". No new I/O, no engine-rs change; unblocks bastion-web's
  `BW.3.F` band-merge, joining `GET /api/repos/{name}/workflows` against `GET /api/runs`'s existing
  `RunSummaryDto.run_id` (v0.16). **A3:** Section 11.3's response type is now the typeshared
  `HandoffInfoDto` in `src/serve/dto.rs`, mirroring the internal `HandoffInfo` domain type
  (`title`, `body`) field-for-field per the `RepoStatusDto` precedent (Section 11.2) — `HandoffInfo`
  itself stays unannotated, preserving `dto.rs`-is-source-of-truth for typeshared contract types.
  `get_repo_handoff`'s existing 404/`C002` behaviour for a missing `handoff.md` is unchanged.
  Unblocks bastion-web's `BW.8.K3` briefing handoff feed. `types/serve.ts` regenerated
  (`scripts/gen-types.sh`); `scripts/check-typeshare-drift.sh` passes clean.

- **2026-08-01 — v0.18 → v0.19 (ask A5, `planning/arch-review-asks-bastion-web/notes.md`):**
  Section 13's `BoardBlockDto` gains `dependent_count: Option<u32>`, `ready: Option<bool>`, and
  `unmet_count: Option<u32>`, all carried verbatim from mev's already-computed, corpus-wide
  `mev::build_block_graph_export` (`../mev/src/brain/block_graph.rs`) — bastion derives nothing.
  `dependent_count`/`ready` are populated on all five lanes (`now`/`next`/`blocked`/`deferred`/
  `finished`); `unmet_count` is `Some(n)` only for `blocked`-lane entries, `None` everywhere else,
  since mev defines it as a structural `0` for every non-blocked lane and shipping it unqualified
  would let a consumer read "0 unmet ⇒ ready" and reproduce, server-blessed, the exact false-ready
  bug this ask exists to kill — **`ready`, not `unmet_count == 0`, is the readiness signal** (see
  Section 13.3's new subsection). Task 1 measured an unconditional `build_block_graph_export` call
  as adding ~82ms on the live HQ corpus and roughly doubling `/api/board`'s end-to-end wall-clock
  (~80ms → ~162ms) — over the "doubles board assembly time" line in the harness's rule of thumb —
  so the enrichment ships opt-in behind a new `?graph=1` query param (Section 13.1) rather than
  unconditionally; all three fields are omitted (not `null`, not `0`) when the param is absent, or
  when a board block has no matching entry in the graph export (e.g. `max_nodes`-truncated). The
  one-derivation invariant is asserted by extending the existing `build_board` /
  `mev::build_block_graph_export` cross-check test at `block_graph.rs:587-820`, and corpus-wide
  stability is asserted by a dedicated test proving `dependent_count` for a fixture block is
  identical at `scope=hq` and at a narrower tier/project scope. `types/serve.ts` regenerated
  (`scripts/gen-types.sh`); `scripts/check-typeshare-drift.sh` passes clean.

- **2026-07-31 — v0.16 → v0.17 (bastion-web DAG status-color work):** `db::workflows::derive_run_status`
  gains a third metadata check — `metadata.suspension.suspended == true` → `RunStatus::Suspended` —
  checked after `budget`/`cancellation` (both terminal, so still take priority per the existing
  reasoning) and before the node-aggregate fallback. Without this, a paused run's boundary node
  (genuinely `Pending`, per engine-rs's `suspend.rs` mechanics — neither the `SuspendNode` nor the
  `OperatorPause` path fakes a status) fell through to the ordinary node-aggregate rules and read as
  plain `pending`/`running`, indistinguishable from a run that hadn't started or wasn't paused at
  all. `Suspended` is added as a fourth run-level-only `RunStatus` variant (`#[serde(skip_deserializing)]`,
  same pattern as `Cancelled`/`BudgetHalted`) and surfaces on `RunSummaryDto.status` as the wire
  string `"suspended"`. Not terminal: `stamp_resumed` flips `suspended: false` on resume (never
  deletes the key), so a resumed run correctly falls back through to the node-aggregate rules again.
  `RunStateDto` (Section 14.2) needed no change — it has no aggregate status field, only per-node
  `NodeTransitionDto.status` (unaffected) and the raw `metadata` blob (already carried `suspension`
  verbatim). Also threaded through `monitor/alerts.rs` (new `AlertEvent::RunSuspended`, "Pop" desktop
  notification sound) and `monitor/ui.rs` (TUI status color `Cyan`, symbol `"="`) for parity with the
  existing `Cancelled`/`BudgetHalted` treatment in both.

- **2026-07-30 — v0.15 → v0.16 (BA.11.T):** Widened Section 14.1 (`GET /api/runs`) from
  `Vec<String>` (bare run-id UUIDs) to `Vec<RunSummaryDto>` (`run_id`, `workflow_type`, `status`,
  `spec_slug`, `started_at`, `updated_at`), scoped strictly to `LiveStateStore::list_active()` —
  live runs only. `status` reuses the existing `db::workflows::derive_run_status` (no new status
  derivation) over a `TaskContext::node_runs` mapped into minimal `NodeState`s. `spec_slug` is read
  straight off the triggering event's `spec_slug` key and is **omitted** (`skip_serializing_if`,
  not `null`) when absent. `started_at`/`updated_at` are derived purely from
  `node_runs[*].started_at`/`completed_at` (earliest/latest respectively across all tracked nodes),
  both `null` when the run has no recorded node transitions yet. `workflow_type` is
  `Option<String>`, always `None` on the wire today — no production code stamps a workflow-identity
  key anywhere `bastion` can read it from a live `TaskContext`; `engine-serve` only tracks it in a
  process-local, `pub(crate)`-scoped side table (`http.rs::live_run_metadata()`). Tracked by the
  engine-rs follow-up ticket `EN.ticket.expose-live-run-workflow-type`
  (`core/engine-rs/planning/ticket-expose-live-run-workflow-type/`); this block does not fabricate a
  value or block on that ticket landing. A run evicted from the live map by `mark_terminal` is
  absent from the response. No engine-side change, no new route, and no `GET /api/runs/{id}`
  (Section 14.2) change are part of this block. New DTO: `RunSummaryDto`. `types/serve.ts`
  regenerated; `scripts/check-typeshare-drift.sh` passes.

- **2026-07-30 — `BoardBlockDto.status` semantics correction (BA.ticket.enrich-block-authored-status,
  no version bump):** `enrich_block` (Section 13's `board.rs`) now copies the authored
  `tracks[].blocks[].status` onto `status` alongside the five fields it already enriched
  (`epics`/`wave`/`priority`/`due`/`track`), on every lane. Previously, `status` on the
  `now`/`next`/`blocked`/`deferred` lanes came straight from `mev::brain::state::derive_rollup`'s
  lane rollup, which **fabricates** a status per lane rather than reading the real one:
  `Some("in_progress")` for every `now` entry, `None` for every `next`/`blocked`/`deferred` entry,
  regardless of the block's actual authored status. Only `finished` ever reported the true value.
  This caused `bastion-web`'s `/business` surface to render every open block as `UNKNOWN` and its
  header as `0 READY BLOCKS` (a count of `status === "open"`, which never arrived). No `BoardBlockDto`
  field was added or removed — this is a semantics-only fix to an existing field, so no version bump
  and no `types/serve.ts` regeneration were needed.

- **2026-07-29 — v0.14 → v0.15 (BA.11.J):** Added Section 24, `GET /api/costs` — a read-only
  projection of the existing `src/costs/` module (BA.7.B's exact per-workflow token/cost
  aggregation and BA.7.C's budget-gate evaluation) over HTTP, so `bastion-ui` and any web
  dashboard can render spend/budget without shelling to the CLI. The route accepts `?window=`
  (`"7d"`/`"30d"`/`"all"`, case-insensitive, default `"7d"`) only — it deliberately drops the
  block definition's optional `?repo=` param, because the `events` contract carries no repo
  dimension to filter on (`WorkflowRun` has no repo field; `costs::aggregate` groups by
  `workflow_name` only); `repo` is not silently aliased onto `workflow_name`. The route mounts
  unconditionally on both `serve` app factories (unlike the gated engine routes) and degrades to
  typed errors — `503`/`C005` when `DATABASE_URL` is unset, `503`/`C009` on database
  unreachability — rather than being absent. Nothing over HTTP mutates the configured budget caps;
  mutation stays CLI/D48. New DTOs: `CostSummaryDto`, `WorkflowCostDto`, `BudgetStateDto`,
  `BudgetBreachDto`.

- **2026-07-29 — v0.13 → v0.14 (BA.11.S):** Added `last_touched: Option<String>` to
  `BoardBlockDto` (Section 13.3) — mev's derived per-block SDLC recency
  (`MV.10.D`, `mev::brain::last_touched::derive_last_touched`) carried verbatim onto the board,
  populated on all five lanes (`now`/`next`/`blocked`/`deferred`/`finished`). Bastion performs
  **zero derivation** of its own: the field is computed exactly once per request, inside
  `assemble_board`, from the same `config` + loaded `files` that fn already walks, and looked up by
  the `"{repo}:{id}"` key mev owns — the same one-derivation guarantee Section 23 established for
  `GET /api/blocks/graph`, now extended (and cross-checked) to include this field: the two read
  paths (`build_board` and `mev::build_block_graph_export`) are asserted to agree on `last_touched`
  for every block, including both being absent for the same block. Additive and backward
  compatible — a JSON body written before this block (no `last_touched` key) still deserializes
  into the current `BoardBlockDto`, yielding `None`. **Absence means "never worked", not "worked
  long ago"** — a block with no resolvable SDLC run gets no entry in mev's map and no key in the
  serialized DTO; no sentinel date, epoch, or the file-level `state.json.updated` is ever
  substituted. Serialization deliberately **diverges** from the v0.11 sibling fields
  (`epics`/`wave`/`priority`/`due`/`track`, which serialize as `[]`/`null` when unknown):
  `last_touched` carries `#[serde(skip_serializing_if = "Option::is_none")]`, so an unknown value
  is **omitted from the JSON body entirely** rather than emitted as `null`. `types/serve.ts`
  regenerated (`last_touched?: string` on `BoardBlockDto`); `scripts/check-typeshare-drift.sh`
  passes. No other `/api/board` field, `/api/epics`, or `/api/blocks/graph` response shape
  changed. Updated frontmatter title, description, keywords, and the current-contract version
  note.

- **2026-07-28 — v0.12 → v0.13 (BA.17.A, program block BA.2.A):** Added Section 23 (Block-graph
  API) — `GET /api/blocks/graph?scope=hq|tier|project|business|epic[&tier=<name>][&epic=<slug>]
  [&repo=<slug>][&include_closed=bool][&include_boundary=bool][&max_nodes=<n>]` (`max_nodes`
  default `400`, clamped to `2000`), a mechanical projection of mev's enriched block-graph export
  (`mev::build_block_graph_export`) onto HTTP, reusing `board::assemble_board`'s brain-walk
  (Section 13) so the graph and the board read one corpus in one request shape. Bastion performs
  **zero derivation** of its own — every `BlockGraphDto`/`BlockGraphNodeDto`/`BlockGraphEdgeDto`/
  `BlockLaneDto`/`BlockEdgeKindDto` field is copied straight across from the upstream mev type —
  the one-derivation contract bastion-web's node-graph view (`BW.9.B`) relies on, mechanically
  enforced by a cross-check test comparing this route's output against `GET /api/board`'s for the
  same fixture corpus (node count, edge count, per-node lane). Error mapping mirrors
  `GET /api/board` (Section 13.4) verbatim: `400` plain-text for an unrecognized `scope`, `404`/
  `C005` for the two `scope=epic` registry-miss cases (missing/blank param, unknown slug, via the
  same `board::epic_param_missing`/`board::epic_known`/`board::epic_error_response` helpers), and
  `500`/`C010` for an unresolvable brain root or a `web::block` failure. No write path, no UI, and
  no bastion-side derivation is introduced (D25). Appended as Section 23 (after Pipeline /
  opportunities API) — no existing endpoint's documented contract changed; the auth policy table
  (Section 2.3) already covers `/api/*` routes generically (per the v0.12 precedent) and is not
  amended here. Updated frontmatter title, description, and `keywords`; updated the
  **Consumed by** line to add bastion-web (`BW.9.B`) for Section 23.

- **2026-07-26 — v0.11 → v0.12 (BW.3.A):** Added Section 22 (Pipeline /
  opportunities API) — `GET /api/pipeline` (canonical stage vocabulary parsed
  from `business/docs/pipeline.md`'s `## Stages` line + one summary per
  opportunity file, sorted by stage order then title) and
  `GET /api/pipeline/{slug}` (the full projection for one opportunity:
  frontmatter contacts/actions/links, the research brief parsed from the body's
  first ` ```json ` fence — a CompanyBrief or a ProspectingResult — and the raw
  body markdown), both under the existing bearer-protected `/api` scope. A
  read-only (D25) projection over the business sub-brain's
  `business/docs/opportunities/*.md` + `business/docs/leads/*.md` (skipping
  `index.md`/`README.md` and non-`.md` files), read from the HQ brain root
  resolved the same way as Sections 13/15 (`resolve_workspace_root` →
  `find_brain_root`). Documented the
  `PipelineDto`/`OpportunitySummaryDto`/`OpportunityDetailDto`/`ContactDto`/
  `OpportunityActionDto`/`ResearchBriefDto`/`ProspectLeadDto` response schema and
  the 404/`C002` (unknown/invalid slug, uniform so an invalid slug is
  indistinguishable from an absent one) + 500/`C010` (brain root unresolvable,
  `web::block` failure) error mapping. A missing/unreadable `business/` tree
  degrades the list route to empty `stages`/`opportunities` rather than erroring.
  Appended as Section 22 (after Versioning policy) to avoid renumbering the
  engine/types/config/versioning sections; the auth policy table (Section 2.3)
  already covers all `/api/*` routes generically. Updated frontmatter title,
  description, and the current-contract version note.

- **2026-07-25 — v0.10 → v0.11 (BA.11.R):** Additive `BoardBlockDto` fields
  `epics: Vec<String>` (default `[]`), `wave: Option<i64>`, `priority: Option<u8>`,
  `due: Option<String>`, `track: Option<String>` (Section 13.3) — a pre-v0.11 JSON body still
  deserializes into the current struct, so `bastion-ui` and the TUI keep working unchanged.
  **Deviation from the master plan:** `wave` is typed `Option<i64>` (mirroring
  `okf_core::TrackBlock.wave` exactly) rather than the plan's `Option<u32>`; casting would
  silently mangle out-of-range or negative authored values. Corrected the `blocked_by` prose
  (Section 13.3) — it previously documented `blocked_by` as populated for the `blocked` lane only;
  it is now populated on all four lanes (`now`/`next`/`blocked`/`finished`), computed as the same
  unmet-dependency set `derive_focus` uses (any `BlockedBy::External`, or a `BlockedBy::Block`
  whose target's authored status is not `"closed"`). Added `scope=epic` + the `&epic=<slug>` query
  param to `GET /api/board` (Section 13.1/13.2), with the two 404/`C005` branches (missing param,
  unknown slug) documented in Section 13.4. Added a new Section 17 (Epics registry API) —
  `GET /api/epics`, returning the HQ `epics[]` registry as `EpicDto[]` (`slug`/`title`/
  `description`/`status`/`plan`/`repos`), `200 []` when no HQ registry file is found (not an
  error). Renumbered Embedded engine route table → Section 18 (subsections 18.1–18.3), Generated
  TypeScript types → Section 19 (subsections 19.1–19.3), Configuration reference → Section 20,
  Versioning policy → Section 21, and updated every in-document cross-reference to the moved
  sections (Section 2.3's auth table gained a `GET /api/epics` row).

- **2026-07-24 — v0.9 → v0.10 (BA.11.Q):** Added Section 16 (Docs read API) —
  `GET /api/docs/{repo}/tree?path=<rel-dir>` (allowlisted markdown tree, directories-first then by
  name, `path` optional and defaults to walking every allowlisted root with `root: ""`) and
  `GET /api/docs/{repo}/file?path=<rel-file>` (raw markdown read — no rendering, frontmatter
  stripping, or sentinel removal — plus `bytes` and an RFC-3339 `modified`), both under the
  existing bearer-protected `/api` scope. Documented the `DocTreeDto`/`DocEntryDto`/`DocFileDto`
  response schema and the security contract this block introduces: a pure pre-check
  (`validate_rel_path`) rejecting an absolute path, a `..` component, a NUL byte, a backslash, an
  empty path, and (for the file route) a non-`.md`/`.mdx` extension before any filesystem access,
  followed by canonicalize-then-contain allowlist resolution (`resolve_allowlisted_path`) against
  the allowlisted roots (`docs/`, `planning/`, `business/`, plus repo-root `*.md` files) —
  **each allowlisted root canonicalized independently**, never a canonicalized repo root, so that
  `planning/`'s company-brain-vault symlink resolves correctly while a symlink inside a root
  pointing outside it is still rejected. All three rejection reasons (traversal, bad extension,
  outside every allowlisted root) and a missing required `?path=` on the file route collapse into
  one uniform `403`/`C003` response so the API never discloses whether an out-of-allowlist path
  exists; an unknown `{repo}` is `404`/`C005`, a missing in-allowlist file is `404`/`C002`. This
  promotes the formerly-planned §20 draft (banner-marked "NOT YET IMPLEMENTED", removed from its
  end-of-document position) to its own numbered Section 16 placed after Section 15 — no existing
  endpoint's documented contract changed. Added both `/api/docs/{repo}/tree` and
  `/api/docs/{repo}/file` rows to the auth policy table (Section 2.3). Renumbered Embedded engine
  route table → Section 17 (subsections 17.1–17.3), Generated TypeScript types → Section 18
  (subsections 18.1–18.3), Configuration reference → Section 19, Versioning policy → Section 20,
  and updated every in-document cross-reference to the renumbered sections. Updated frontmatter
  title, description, `keywords`, and the current-contract version note.
- **2026-07-24 — v0.8 → v0.9 (BA.11.P):** Added Section 15 (Attention / carryover API) —
  `GET /api/attention?scope=hq|tier|project|business[&tier=<name>]`, projecting the tier-scoped
  Attention board (stale `carryover[]`, aging `backlog[]`, orphaned `/capture` notes) that
  `mev emit-state` already splices into `status.md`, onto HTTP. Query-param and scope semantics
  are identical to `GET /api/board` (Section 13.2, reuses `BoardScope`). Documented the
  `AttentionDto`/`AttentionLanesDto`/`AttentionCarryoverDto`/`AttentionBacklogDto`/
  `AttentionThresholdsDto` response schema, the scoping rule mirroring
  `mev::brain::emit::plan_attention_board` (HQ unions everything; a tier scope unions that tier's
  leaf repos' carryover plus the tier brain file's own, and only HQ `backlog[]` nodes whose `repo`
  is in that tier), the capture-vs-aging lane split (`origin.type == "capture"` → `orphaned_captures`
  only, carrying `origin.notes` falling back to `notes`), oldest-first lane ordering, and that
  snoozed/under-threshold/no-anchor items are absent rather than flagged. This promotes the
  formerly-planned §13.5 draft (removed from Section 13) to its own numbered section — no existing
  endpoint's documented contract changed. Added the `/api/attention` row to the auth policy table
  (Section 2.3). Renumbered Embedded engine route table → Section 16 (subsections 16.1–16.3),
  Generated TypeScript types → Section 17 (subsections 17.1–17.3), Configuration reference →
  Section 18, Versioning policy → Section 19, and updated every in-document cross-reference to the
  renumbered sections. Updated frontmatter title, description, `keywords`, and the
  current-contract version note.
- **2026-07-24 — v0.7 → v0.8 (BA.11.M, read half):** Added Section 14 (Live run read API) —
  `GET /api/runs` (currently-tracked run ids) and `GET /api/runs/{id}` (per-node `RunStateDto`
  snapshot: status, timing, output, and for a failed node its error + input, plus LLM-node
  token/model usage), both under the existing bearer-protected `/api` scope. The routes project
  the embedded engine's in-memory `LiveStateStore`, which is now shared as a single instance
  between the engine's `on_progress` writer (when the Section 15 engine mount is active) and these
  read handlers; with the engine unmounted the store stays empty (`200 []` / `404`) rather than
  erroring. This is a read-only snapshot — no fine-grained SSE/WS run-transition stream and no
  `engine-serve` change are introduced by this block; the D42 live **stream** half of the original
  `BA.11.M` scope is split into a follow-on block, `BA.11.N` (per D17: bastion pushes run
  transitions over the existing bearer-authed `/ws` hub by polling the in-process `LiveStateStore`
  and diffing per `BA.11.D`'s pattern — no `engine-serve` change, no second streaming protocol),
  with `BW.3.A`'s ~2s polling as the standing fallback until then. Renumbered Embedded engine route
  table → Section 15 (subsections 15.1–15.3), Generated TypeScript types → Section 16
  (subsections 16.1–16.3), Configuration reference → Section 17, Versioning policy → Section 18.
  Updated frontmatter title, description, `keywords`, and the current-contract version note.
- **2026-07-23 — v0.6 → v0.7 (BA.11.L):** Added Section 15 (Generated TypeScript types) —
  documents `types/serve.ts` (committed, generated from `#[typeshare]`-annotated `src/serve/
  dto.rs` via `typeshare.toml`, MUST NOT be hand-edited), the regenerate command
  (`scripts/gen-types.sh` / the raw `typeshare` invocation), the drift check
  (`scripts/check-typeshare-drift.sh`, relied on by CI and BastionWeb), the `typeshare` CLI
  prerequisite (`cargo install typeshare-cli --locked`), and the `Topic`/`CommandValidationError`
  exclusions (internal-only, no serde representation). No `serve` runtime behaviour changed — the
  annotations are compile-time no-ops and generation/drift-check are build-time-only tooling; no
  existing endpoint's documented request/response contract was altered. Renumbered Configuration
  reference → Section 16, Versioning policy → Section 17. Updated frontmatter title, description,
  `keywords`, and the current-contract version note.
- **2026-06-26 — v0 → v0.1 (Block 11.B):** Added Sessions REST API (six routes), response
  DTOs (`SessionDto`, `PaneDto`), request-body DTOs (`SendBody`, `KeyBody`, `NewSessionBody`),
  named-key endpoint, and tmux degradation → HTTP status mapping (503/404/500) with
  `ErrorPayload` shape.  Updated auth policy table to list all six session routes.
- **2026-06-30 — v0.1 → v0.2 (Block 11.C):** Replaced the `/ws` echo actor with the real
  session hub.  Added Section 5 (frame envelope v0.2 with all `kind` values), Section 6
  (topic model: `sessions` and `pane:<name>`), Section 7 (all payload shapes for the nine
  frame kinds: `subscribe`, `unsubscribe`, `send`, `send_key`, `sessions`, `pane`, `event`,
  `error`), Section 8 (`event{needs_input}` semantics and rising-edge debounce), Section 9
  (keep-alive / disconnect behaviour).  Renumbered former Sessions REST API → Section 10,
  Configuration → Section 11, Versioning → Section 12.  Updated auth policy table (Section
  2.3) to reflect `/ws` is now hub-backed.  Updated frontmatter title and description.
- **2026-06-30 — v0.2 → v0.3 (Block 11.D):** Added Section 11 (Repo / workflow status REST
  API — `GET /repos`, `GET /repos/{name}/status`, `GET /repos/{name}/handoff`,
  `GET /repos/{name}/workflows`; response DTOs `RepoSummaryDto`, `RepoStatusDto`,
  `HandoffInfo`, `WorkflowStateDto`; 404/`C002` mapping for unknown workspaces and
  missing/malformed `status.md`/`handoff.md`).  Added the `workflow_done` event name to
  Section 7.7's event table and Section 8.2 (`FlowWatcher`-driven non-terminal→terminal
  transition semantics, `WorkflowDonePayload` shape).  Updated auth policy table (Section
  2.3) to list the four new `/api/repos*` routes.  Renumbered Configuration → Section 12,
  Versioning → Section 13.  Updated frontmatter title and description.
- **2026-07-14 — v0.3 → v0.4 (Block 11.E):** Added Section 12 (Quick-action command API —
  `POST /api/actions/command`; `CommandRequest`/`CommandResponse` DTOs; `inject`/`spawn`
  dispatch behaviour reusing `ask`'s spawn/readiness mechanics; validation-failure (400/`C006`)
  and execution-failure (404/`C002`, 503/`C001`, 500/`C010`, 504/`C007`) error mapping).
  Updated auth policy table (Section 2.3) to list the new `/api/actions/command` route.
  Renumbered Configuration → Section 13, Versioning → Section 14.  Updated frontmatter title,
  description, and the current-contract version note.
- **2026-07-16 — v0.4 → v0.5 (BA.7.C task 2):** Added Section 13 (Embedded engine route table) —
  `bastion serve` now mounts `engine-serve`'s route table (`GET /health`, `GET /workflows`,
  `GET /workflows/{type}/graph`, `POST /events/`, `POST /events/{run_id}/abort`) at server root,
  gated by its own `X-API-Key` scheme (`BASTION_ENGINE_API_KEY`) entirely separate from bastion's
  `Bearer` scheme, mounted only when `DATABASE_URL` + `BASTION_ENGINE_API_KEY` are both present
  (`serve::decide_engine_mount`).  Rewrote Section 2 to document the two auth schemes side by
  side.  Added `DATABASE_URL` / `BASTION_ENGINE_API_KEY` to the Configuration reference (now
  Section 14).  Renumbered Configuration → Section 14, Versioning → Section 15.  Updated
  frontmatter title, description, `layer`, `keywords`, `related`, and the current-contract
  version note.
- **2026-07-18 — v0.5 doc catch-up (`serve-ui-contract-gaps`):** No version bump — these are
  server-side bug fixes bringing the implementation into line with intent, not new routes or
  breaking changes. (1) Section 8.1: needs-input detection moved from the per-pane poll interval
  into the sessions-list poller, so `event{needs_input}` now reaches `sessions` subscribers with
  no `pane:<name>` subscription required. (2) Section 9: documented the now-implemented WS
  keep-alive heartbeat (`HEARTBEAT_INTERVAL` 5s / `CLIENT_TIMEOUT` 10s) and client-timeout
  reaping. (3) Section 7.5: documented that WS `sessions` frames now carry a populated
  `last_line`; REST `GET /api/sessions` is unchanged (still empty). (4) Sections 11.2–11.4:
  an unknown/unregistered workspace name now returns `404`/`C005` (ConfigError), distinguishable
  from a registered workspace missing `status.md`/`handoff.md` (still `404`/`C002`). (5) Section
  12.3: documented that a malformed/non-JSON request body on any JSON-consuming route now returns
  `400`/`C006` via a `web::JsonConfig` error handler, instead of actix's plain-text 400. Also
  fixed the frontmatter `title` scalar, which had lagged at "v0.4" since the previous entry.
- **2026-07-23 — v0.5 → v0.6 (BA.11.K):** Added Section 13 (Cross-brain board API) —
  `GET /api/board?scope=hq|tier|project|business[&tier=<name>]`, projecting the mev/okf-core
  cross-brain now/next/blocked/finished rollup (the same aggregate `bastion emit-state` /
  `bastion validate-brain --state` already compute) onto HTTP. Documented the scope→`TierScope`
  resolution table (`hq`→`All`; `tier`/`project`→`Tier(<tier>` or default `"core">`;
  `business`→`Tier("business")`), the `BoardDto`/`BoardLaneDto`/`BoardBlockDto`/`RepoBoardDto`
  response schema, the `finished` lane's `status == "closed"` definition, the `stale` freshness
  flag (`mev::brain::sync::check_sync`), and the context-aware-tier-default as a documented
  future refinement (not implemented in this block). Noted that an unrecognized `scope` value
  returns actix's default plain-text `400` (no `QueryConfig` error handler is installed for this
  route, unlike the `web::JsonConfig` handler backing Section 12.3's `C006` JSON body) — verified
  against the running handler, not assumed. An unknown `tier` name is not an error: it resolves
  to an empty in-scope rollup. Added the `/api/board` row to the auth policy table (Section 2.3).
  Renumbered Embedded engine route table → Section 14 (subsections 14.1–14.3), Configuration
  reference → Section 15, Versioning policy → Section 16. Updated frontmatter title, description,
  `keywords`, `related`, and the current-contract version note.
