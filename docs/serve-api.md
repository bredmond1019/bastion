---
type: Guideline
title: "serve-api contract v0.22"
description: "HTTP + WebSocket API contract for `bastion serve` — base URL, bearer-auth scheme, GET /health, /ws hub (topic subscriptions, live pane, needs-input event, workflow_done event), the v0.2 frame envelope, the v0.1 session REST surface (list/pane/send/key/create/delete), the v0.3 repo/workflow status REST surface (GET /repos, GET /repos/{name}/status, GET /repos/{name}/handoff, GET /repos/{name}/workflows), the v0.4 quick-action command endpoint (POST /actions/command, inject/spawn modes), the v0.6 cross-brain board endpoint (GET /api/board) that bastion-ui pins against, the v0.7 generated-TypeScript-types artifact (types/serve.ts, typeshare) for BastionWeb, the v0.8 live run read API (GET /api/runs, GET /api/runs/{id}) projecting the embedded engine's in-memory LiveStateStore for bastion-web's node drill-in (BA.11.M, D42 read half), the v0.9 Attention / carryover API (GET /api/attention) projecting the stale-carryover / aging-backlog / orphaned-capture board for bastion-ui, the TUI, and bastion-web BW.1.C (BA.11.P), the v0.10 Docs read API (GET /api/docs/{repo}/tree, GET /api/docs/{repo}/file) — an allowlisted, traversal-rejecting markdown tree + raw-file read across repos for bastion-web's reader (BW.2.A, BA.11.Q), and the v0.11 epic + ranking enrichment (epics/wave/priority/due/track on `BoardBlockDto`, `blocked_by` on all four lanes, GET /api/epics, and GET /api/board?scope=epic) for cutting work by cross-repo initiative (BA.11.R), the v0.12 pipeline / opportunities read API (GET /api/pipeline, GET /api/pipeline/{slug}) projecting the business sub-brain's opportunity markdown (researched companies + prospecting sweeps + job postings, with contacts, actions, and the body's ```json research brief) for bastion-web's pipeline board (BW.3.A), the v0.13 block-graph read API (GET /api/blocks/graph) — a mechanical projection of mev's enriched block-graph export (nodes/edges/cycles/lanes/topo-order, reusing the same board brain-walk) with zero derivation performed by bastion, for bastion-web's node-graph view (BW.9.B), the v0.14 `last_touched` field on `BoardBlockDto` — mev's derived per-block SDLC recency (`MV.10.D`) carried verbatim, with zero derivation by bastion, absent (not `null`) when a block has never been worked (BA.11.S), the v0.15 read-only Cost read API (GET /api/costs) — a projection of the existing `src/costs/` aggregation (BA.7.B) and budget-gate evaluation (BA.7.C) over HTTP, with `?window=` only (no `?repo=`, since the events contract carries no repo dimension) for bastion-ui and any web dashboard to render spend/budget without shelling to the CLI (BA.11.J), and the v0.16 `GET /api/runs` summary widening — from bare run-id strings to `RunSummaryDto` (run_id, workflow_type, status, spec_slug, started_at, updated_at), scoped strictly to `list_active()` live runs, reusing the existing `db::workflows::derive_run_status` for status and leaving `workflow_type` always absent pending the engine-rs follow-up ticket `EN.ticket.expose-live-run-workflow-type` (BA.11.T), and the v0.17 `suspended` run status — `db::workflows::derive_run_status` now reads `metadata.suspension.suspended` (engine-rs's `suspend.rs` marker) and reports it on `RunSummaryDto.status` wherever `cancelled`/`budget_halted` already were; `RunStateDto` (Section 14.2) has no aggregate status field to begin with (only per-node `NodeTransitionDto.status`, unaffected, and the raw `metadata` blob, which already carried `suspension` verbatim), so it needed no change; and the v0.18 `run_id` on `WorkflowStateDto` — the engine's `events.id` run UUID that engine-rs `EN.6.J` already stamps into `sdlc-flow-state.json`, carried through Section 11.4's response with `run_id` absent (not `null`) when the state predates that stamp or was written by base-template's JS `sdlc-flow.js` engine — plus a typeshared `HandoffInfoDto` mirroring Section 11.3's `HandoffInfo` domain type, unblocking bastion-web's `BW.3.F` band-merge and `BW.8.K3` briefing handoff feed; and the v0.19 `dependent_count`/`ready`/`unmet_count` enrichment on `BoardBlockDto` (A5) — mev's corpus-wide `build_block_graph_export` output carried verbatim onto every board lane entry behind an opt-in `?graph=1` query param (task 1 measured the unconditional call as roughly doubling `/api/board`'s wall-clock on the live HQ corpus), with `dependent_count`/`ready` populated for all five lanes and `unmet_count` populated only for `blocked`-lane entries — `ready`, not `unmet_count == 0`, is the readiness signal, since mev defines `unmet_count` as `0` for every non-blocked lane, and the v0.20 cross-repo workflows aggregate (GET /api/workflows, A2) — Section 11.6's new route returns every registered workspace's Section 11.4 flow states in one response, each entry tagged with a new typeshared `RepoWorkflowStateDto.repo` field, reusing `collect_flow_states` verbatim per workspace with no second flow-state walk, ordered deterministically by (repo, spec_slug), retiring the residual N+1 bastion-web's `/engine` on-disk band and briefing diff had left after A1/A5; the existing per-repo `GET /api/repos/{name}/workflows` route is unchanged; and the v0.21 `weight` field on `EpicDto` (GET /api/epics, `BA.ticket.epic-weight-dto`) — the authored `okf_core::Epic.weight` carried verbatim onto the wire with zero derivation by bastion (mev's `check_epics` owns the `0..=100` range policy via `E_STATE_EPIC_BAD_WEIGHT`, so an out-of-policy authored value passes through unclamped), `null` when unauthored — unblocking bastion-web's ranking of initiatives by authored weight, and
the v0.22 `repo` field on `RunSummaryDto` (`GET /api/runs`, A7) — an exact `run_id` join against
every registered workspace's flow state (`RepoWorkflowStateDto` from `collect_all_workflows`, A2),
absent (never guessed) when no flow state carries a run's `run_id`, gated behind an opt-in
`?with_repo=1` query param (mirroring `/api/board`'s `?graph=1`, A5) since task 1's measurement
found the registry walk roughly 6x the unenriched baseline against the live HQ registry (23 repos),
so the route's hottest consumer (bastion-web's ~2-6s run rail) does not pay for it unless it asks."
doc_id: serve-api
layer: [console, surface, engine]
project: bastion
status: active
keywords: [serve, api, websocket, sessions, status, actions, quick-action, board, cross-brain, rollup, bastion-ui, contract, engine-serve, abort, X-API-Key, typeshare, typescript, codegen, live-state, runs, task-context, d42, attention, carryover, backlog, staleness, orphaned-captures, docs, markdown, allowlist, path-traversal, file-tree, read-endpoint, epics, ranking, wave, priority, due, blocked_by, block-graph, nodes, edges, cycles, topo-order, lanes, mechanical-projection, one-derivation, last_touched, recency, costs, budget, spend, run-summary, RunSummaryDto, spec_slug, workflow_type, run_id, handoff, dependent_count, ready, unmet_count, block-graph-enrichment, corpus-wide, one-derivation, a5, workflows-aggregate, RepoWorkflowStateDto, cross-repo, n-plus-one, a2, contract-corpus, goldens, stub-fidelity, redaction, drift-check, a4]
related: [config, observ, data-contract, abort, master-plan]
---

# serve-api — v0.22 Contract

**Version:** v0.22  
**Produced by:** `bastion` (this repo, `src/serve/`) — Sections 1–17, 19–25 — plus, when mounted,
`engine-serve` (`../engine-rs/crates/engine-serve/`, embedded per D48) — Section 18.  
**Consumed by:** `bastion-ui` (Flutter mobile Surface, D28) for Sections 1–13, 15–17, 19–21, 24;
bastion-web (`BW.3.B`) for Section 14; bastion-web (`BW.1.C`) for Section 15; bastion-web
(`BW.2.A`) for Section 16; `bastion abort` (`src/run/abort.rs`, this repo) for Section 18's abort
route; bastion-web (`BW.9.B`) for Section 23.

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
| `topic` | string | Yes | Topic to subscribe to (`"sessions"` or `"pane:<name>"` — name must be non-empty) |

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
    { "name": "main", "state": "running", "last_line": "$ cargo test" },
    { "name": "scratch", "state": "idle", "last_line": "" }
  ]
}
```

| Field | Type | Description |
|---|---|---|
| `sessions` | array | Array of `SessionDto` objects (see Section 9.1) |

`last_line` is populated (as of v0.5) with each session's pane's last
non-blank captured line, reusing the same per-session pane-capture pass the
sessions-list poller performs for needs-input detection (Section 8.1) — panes
are captured once per tick and used for both. An idle session with no
captured output (or a capture failure) still yields `""`. `GET
/api/sessions` (Section 10.3) is **not** brought to the same parity in v0.5 —
it still returns empty `last_line` for every session, unchanged from prior
versions.

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

### 8.1 `event{needs_input}` (v0.2; detection moved to the sessions-list poller in v0.5)

Needs-input detection runs in the **sessions-list poller**, on every tick, over
**every live session** — not only sessions whose pane a client has subscribed
to. Each tick the hub captures every session's pane output, calls
`detect::detect(pane_output, claude.toml)` from Block C₀ to determine the agent
state, and diffs the result against the previous tick's per-session state
(`sessions_last_state`, keyed by session name) using the pure
`sessions_needing_input(prev, current)` helper (`src/serve/poll.rs`). The
`needs_input` event is emitted for a session when:

```
state == Blocked && visible_blocker == true
```

and the session's *previous* recorded state was not already `Blocked` (rising
edge — see below). The event is delivered to the connection's `sessions`
subscribers, carrying that session's name — **a client needs no `pane:<name>`
subscription to receive it**. This is what lets `bastion-ui`, which subscribes
only to `sessions` on connect, surface a needs-input alert for a background
session it has not opened a pane view for.

The hub uses a **rising-edge debounce**: the event is emitted once per
Blocked→Unblocked→Blocked transition cycle (i.e. once per "new prompt"), not on
every poll tick while the session remains blocked.  Consecutive blocked polls
without an intervening non-blocked state produce at most one event.

The event drives the BastionUI alert flow: the mobile operator is notified once
and can respond via a `send` or `send_key` frame to unblock the agent.

Needs-input is emitted from exactly one place (the sessions-list poller); the
per-pane poll interval (Section 7.6) only pushes pane-content diffs and no
longer performs its own needs-input detection.

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
  "last_line": "$ cargo test"
}
```

| Field | Type | Description |
|---|---|---|
| `name` | string | tmux session name |
| `state` | string | `"running"` when the foreground process is not a shell; `"idle"` otherwise |
| `last_line` | string | Last non-blank line from the session's pane, or `""` when unavailable |

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
  { "name": "main", "state": "running", "last_line": "$ cargo test" },
  { "name": "scratch", "state": "idle", "last_line": "" }
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

### 11.6 `GET /api/workflows` — cross-repo flow-state aggregate (v0.20, A2)

Every registered workspace's Section 11.4 flow states in one response, so consumers stop issuing
`GET /api/repos` followed by one `GET /api/repos/{name}/workflows` per repo — retiring the residual
N+1 on bastion-web's `/engine` on-disk band and briefing diff
(`planning/arch-review-asks-bastion-web/notes.md`, ask A2). Reuses `collect_flow_states` (Section
11.4's route) verbatim, once per registered workspace — no second flow-state walk exists.

**Request:**

```
GET /api/workflows HTTP/1.1
Authorization: Bearer <token>
```

**Response (200 OK):** array of `RepoWorkflowStateDto`

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
`sdlc-flow-state.json` files contributes zero entries and does not fail the request — the same
degrade-gracefully behaviour Section 11.1's `GET /api/repos` and Section 11.4's per-repo route
already have. An empty/absent `[workspaces]` registry returns `200 []`.

The route sits under the same bearer-auth `/api` scope as the rest of this section; a request
without a valid token gets `401` before reaching the handler. Section 11.4's per-repo
`GET /api/repos/{name}/workflows` route is unchanged by this addition — it remains the endpoint to
use when only one repo's flow states are needed.

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

## 13. Cross-brain board API (v0.6, BA.11.K; enriched v0.11, BA.11.R; last_touched v0.14, BA.11.S; block-graph enrichment v0.19, A5)

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
| `graph` (v0.19) | boolean | No | `false` | Opt-in gate for the A5 `dependent_count`/`ready`/`unmet_count` enrichment on every `BoardBlockDto` (Section 13.3). When `false`/absent, `assemble_board` skips the `mev::build_block_graph_export` call entirely and all three fields are omitted from the JSON body. When `true` (`?graph=1` or `?graph=true`), the export is computed once per request (task 1 measured this as roughly doubling `/api/board`'s wall-clock on the live HQ corpus — see the block's Notes) and the three fields are populated on every lane entry present in the export. |

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

#### `dependent_count` / `ready` / `unmet_count` (v0.19, A5) — readiness signal

These three fields are **additive and optional**, gated behind `?graph=1` (Section 13.1): a
pre-v0.19 client (or a v0.19 request made without `?graph=1`) still deserializes the response
correctly, with all three fields absent. When `?graph=1` **is** requested, `assemble_board` calls
`mev::build_block_graph_export` **at most once** for the whole request (an unscoped export, per
the one-derivation contract Section 23 already established for `last_touched`) and threads the
result into every lane the same way `last_touched` is threaded — bastion performs **zero
derivation** of its own; every value is carried verbatim from mev.

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

**This is a read-only snapshot, not a stream.** There is no SSE/WS push and no `engine-serve` change
in this API — a client observes the current state only when it requests it. Live push (token-by-
token / transition-by-transition) is split into a follow-on block (proposed `BA.11.N`): SSE over a
`tokio::sync::broadcast` tee added to `engine-serve`'s `on_progress` closure. Until that ships,
`BW.3.A`'s ~2s client polling against these two routes is the standing fallback.

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
| `clears_when` | string \| null | What clears this item, when recorded. |
| `created` | string \| null | Creation date (`YYYY-MM-DD`), when recorded. |
| `reviewed` | string \| null | Last-reviewed date (`YYYY-MM-DD`), when recorded. |
| `age_days` | number | Days since `max(created, reviewed)`, as computed by `carryover_stale_age`. |
| `threshold_days` | number | The per-`kind` threshold this item tripped (`AttentionThresholds::carryover_threshold`). |

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
| `/workflows` | `GET` | None | Registered workflow types (sorted). |
| `/workflows/{workflow_type}/graph` | `GET` | None | The DAG schema for a registered type; `404` for an unknown one. |
| `/events/` | `POST` | `X-API-Key` | Trigger dispatch — resolves `workflow_type`, runs the workflow, mints a `run_id` and a `CancellationToken`. |
| `/events/{run_id}/abort` | `POST` | `X-API-Key` | The abort endpoint this block's `bastion abort <run>` calls — see [abort.md](abort.md) and [data-contract.md](data-contract.md)'s Abort section for the full 401/404/202 contract. |

`X-API-Key` is checked by `engine_serve::http::check_api_key` against `BASTION_ENGINE_API_KEY` —
an exact string match, entirely separate from bastion's own `BASTION_SERVE_TOKEN` Bearer check
(Section 2). Neither scheme is layered on the other's routes.

### 18.3 Testing

Covered by the in-process integration test `tests/abort_contract.rs`, which builds a real
`engine-serve` `App` (via `AppState`) and asserts the 401 / 404 / 202 paths against it — the
worked reference is `../engine-rs/crates/engine-serve/tests/abort_integration.rs`. The mount
decision itself (`decide_engine_mount`) is unit-tested element-by-element in `src/serve/mod.rs`
against all four presence/absence combinations of `DATABASE_URL` / `BASTION_ENGINE_API_KEY`,
including the empty-string-counts-as-absent case.

---

## 19. Generated TypeScript types (v0.7, BA.11.L)

The contract DTOs in `src/serve/dto.rs` are annotated with `#[typeshare]` and are the **single
source of truth** for the TypeScript types consumed by BastionWeb (`BW.0.B`) and any other TS
client of this contract. `bastion-ui` (Flutter) is unaffected — it has no TS surface.

### 19.1 Generated artifact

`types/serve.ts` (committed at the bastion package root) is the generated TypeScript output. It
is produced by the `typeshare` CLI reading the `#[typeshare]`-annotated types in `src/serve/dto.rs`
via `typeshare.toml`. **`types/serve.ts` MUST NOT be hand-edited** — any change belongs in
`dto.rs`, followed by regeneration. The file carries typeshare's own `/* Generated by typeshare
… */` header, which already marks it as generated (no separate hand-added banner is layered on
top, so the committed file stays byte-identical to raw CLI output).

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
typeshare src/serve --lang typescript --output-file types/serve.ts --config-file typeshare.toml
```

Run this after any change to `src/serve/dto.rs`'s public types (new field, new type, new enum
variant, etc.) and commit the regenerated `types/serve.ts` alongside the `dto.rs` change.

### 19.3 Drift check

`scripts/check-typeshare-drift.sh` regenerates the types to a temp file (via the same invocation
`gen-types.sh` uses, so the two scripts can never diverge on flags) and diffs it against the
committed `types/serve.ts`:

- Exits **0** and prints `OK: types/serve.ts is up to date with src/serve/dto.rs.` when identical.
- Exits **non-zero** and prints the unified diff when `types/serve.ts` is stale relative to
  `dto.rs` (e.g. a DTO field was added without regenerating).
- Exits **non-zero** with an actionable install hint (`cargo install typeshare-cli --locked`)
  when the `typeshare` binary is absent from `PATH`, rather than a confusing tool error.

CI and BastionWeb rely on this script to guarantee `types/serve.ts` never silently drifts from the
Rust source of truth. It is a standalone script — it is **not** wired into `planning/harness.json`
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

`bastion-ui` MUST pin to a specific version tag.  The current contract is **v0.18**.

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

## Amendment Log

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
  erroring. This is a read-only snapshot — no SSE/WS stream and no `engine-serve` change are
  introduced; the D42 live **stream** half of the original `BA.11.M` scope is split into a
  follow-on block (proposed `BA.11.N` — SSE over an `engine-serve` broadcast tee), with
  `BW.3.A`'s ~2s polling as the standing fallback until then. Renumbered Embedded engine route
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
