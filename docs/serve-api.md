---
type: Guideline
title: "serve-api contract v0.6"
description: "HTTP + WebSocket API contract for `bastion serve` — base URL, bearer-auth scheme, GET /health, /ws hub (topic subscriptions, live pane, needs-input event, workflow_done event), the v0.2 frame envelope, the v0.1 session REST surface (list/pane/send/key/create/delete), the v0.3 repo/workflow status REST surface (GET /repos, GET /repos/{name}/status, GET /repos/{name}/handoff, GET /repos/{name}/workflows), the v0.4 quick-action command endpoint (POST /actions/command, inject/spawn modes), and the v0.6 cross-brain board endpoint (GET /api/board) that bastion-ui pins against."
doc_id: serve-api
layer: [console, surface, engine]
project: bastion
status: active
keywords: [serve, api, websocket, sessions, status, actions, quick-action, board, cross-brain, rollup, bastion-ui, contract, engine-serve, abort, X-API-Key]
related: [config, observ, data-contract, abort, master-plan]
---

# serve-api — v0.6 Contract

**Version:** v0.6  
**Produced by:** `bastion` (this repo, `src/serve/`) — Sections 1–13, 15, 16 — plus, when mounted,
`engine-serve` (`../engine-rs/crates/engine-serve/`, embedded per D48) — Section 14.  
**Consumed by:** `bastion-ui` (Flutter mobile Surface, D28) for Sections 1–13, 15, 16; `bastion abort`
(`src/run/abort.rs`, this repo) for Section 14's abort route.

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
mandatory bearer-token authentication (Section 2.1–2.3). The embedded engine routes (Section 14,
mounted only when config allows) are a **separate, unmounted-at-`/api` surface with their own
`X-API-Key` gate** — the two auth schemes coexist side by side and are never double-applied to the
same request:

| Route family | Scheme | Header |
|---|---|---|
| `/health`, `/api/*`, `/ws` (Sections 3–13) | Bearer | `Authorization: Bearer <BASTION_SERVE_TOKEN>` |
| Engine routes (Section 14): `/events/`, `/events/{run_id}/abort` | API key | `X-API-Key: <BASTION_ENGINE_API_KEY>` |
| Engine routes (Section 14): `GET /health`, `GET /workflows`, `GET /workflows/{type}/graph` | None (public) | — |

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

## 11. Repo / workflow status REST API (v0.3)

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

**Response (200 OK):** `HandoffInfo`

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
    "updated_at": "2026-06-25T19:02:33Z"
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

## 13. Cross-brain board API (v0.6, BA.11.K)

One read-only route projecting the cross-brain now/next/blocked/finished rollup — the same
aggregate `bastion emit-state` / `bastion validate-brain --state` already compute from the
mev/okf-core brain walk — onto HTTP. Lives under the bearer-protected `/api` scope. This route
never mutates any tier's or repo's `state.json` (D25 — bastion is a read-only surface over the
brain).

### 13.1 `GET /api/board` — cross-brain now/next/blocked/finished board

**Query parameters:**

| Parameter | Type | Required | Default | Description |
|---|---|---|---|---|
| `scope` | string | No | `"hq"` | One of `"hq"` \| `"tier"` \| `"project"` \| `"business"` (Section 13.2). An unrecognized value fails query deserialization and returns `400` (Section 13.4). |
| `tier` | string | No | `"core"` | Tier name; only consulted when `scope` is `"tier"` or `"project"` (Section 13.2). Ignored for `"hq"`/`"business"`. |

**Request:**

```
GET /api/board?scope=hq HTTP/1.1
Authorization: Bearer <token>
```

### 13.2 Scope semantics

| `scope` | Resolved walk scope | `tier` param | `BoardDto.tier` |
|---|---|---|---|
| `"hq"` (default) | `TierScope::All` — whole-brain aggregate | Ignored | `null` |
| `"tier"` | `TierScope::Tier(<tier>)` — that tier's aggregate board | Optional, default `"core"` | Resolved tier name |
| `"project"` | `TierScope::Tier(<tier>)`, same walk as `"tier"` — the client renders each project's board from `repos[]` | Optional, default `"core"` | Resolved tier name |
| `"business"` | `TierScope::Tier("business")` — shortcut, ignores `tier` param | Ignored | `"business"` |

An empty-string `tier` param (`?tier=`) is treated the same as an absent one — it falls back to the
`"core"` default. An unknown `tier` name (no matching tier in `brain.toml`) is **not** an error: the
brain walk simply finds no in-scope repos for that tier, and the response comes back with empty
lanes and `repos: []` rather than a 4xx/5xx.

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
      { "id": "BA.11.K", "title": "Cross-brain board read endpoint", "repo": "bastion", "status": "in_progress", "blocked_by": [] }
    ],
    "next": [],
    "blocked": [],
    "finished": [
      { "id": "BA.11.D", "title": "Repo status REST surface", "repo": "bastion", "status": "closed", "blocked_by": [] }
    ]
  },
  "repos": [],
  "stale": false
}
```

| Field | Type | Description |
|---|---|---|
| `scope` | string | Echoes the resolved `scope` (`"hq"`, `"tier"`, `"project"`, or `"business"`). |
| `tier` | string \| null | The resolved tier name for tier-scoped responses (`"tier"`/`"project"`/`"business"`); `null` for `"hq"`. |
| `lanes` | `BoardLaneDto` | Aggregate now/next/blocked/finished lanes across every in-scope repo. |
| `repos` | array of `RepoBoardDto` | Per-project lane breakdown for every in-scope repo (populated for all scopes — the client picks whether to render the aggregate `lanes` or the per-project `repos[]` breakdown). |
| `stale` | boolean | `true` when any in-scope repo's `planning/status.md` cache lags its `state.json`, per `mev::brain::sync::check_sync`. |

#### `BoardLaneDto`

| Field | Type | Description |
|---|---|---|
| `now` | array of `BoardBlockDto` | Blocks currently in progress. |
| `next` | array of `BoardBlockDto` | Blocks queued next (ordered). |
| `blocked` | array of `BoardBlockDto` | Blocks waiting on something; each entry's `blocked_by` is populated. |
| `finished` | array of `BoardBlockDto` | Blocks whose `status == "closed"` — the terminal value in `mev::brain::state`'s `VALID_TRACK_BLOCK_STATUSES` (`open`/`in_progress`/`closed`). |

#### `BoardBlockDto`

| Field | Type | Description |
|---|---|---|
| `id` | string | Canonical block ID (e.g. `"BA.11.K"`). |
| `title` | string | Block title, looked up from the owning repo's `tracks[].blocks[]`. |
| `repo` | string | Owning repo slug. |
| `status` | string \| null | Lifecycle status when known (`"open"`/`"in_progress"`/`"closed"`). |
| `blocked_by` | array | What this block is waiting on; populated for `blocked`-lane entries, empty elsewhere. |

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

## 14. Embedded engine route table (v0.5, BA.7.C)

`bastion serve` embeds `engine-serve`'s route table (`engine_serve::http::configure`) at the
**server root** — not under `/api` — per D48 ("the abort endpoint and the rest of the engine
surface are served through `bastion serve`, embedding the Engine per D42") and the block's scope
growth (`planning/7.C-cost-budget-alerts-abort/tasks.md`, *Scope growth* section). This is the
same `engine-serve` surface engine-rs's own `EN.1.C`/`EN.2.B` shipped as an embeddable library —
`bastion serve` is the first (and, as of this writing, only) process that actually mounts it.

### 14.1 Mount decision

The engine routes are mounted only when **both** `DATABASE_URL` and `BASTION_ENGINE_API_KEY` are
set (non-empty) at boot — decided once, pure, by `serve::decide_engine_mount` (`src/serve/
mod.rs`). Absent-tolerant: with either value missing, `bastion serve` still boots its existing
`/api`/`/ws` surface (Sections 1–12) with the engine routes simply left unmounted; it prints why on
stderr and emits a `tracing::warn!` `observ` event rather than failing to boot or mounting a route
that would 500 on every request. A `DATABASE_URL` present but unreachable (connection failure at
boot) also leaves the engine routes unmounted, logged the same way.

### 14.2 Routes

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

### 14.3 Testing

Covered by the in-process integration test `tests/abort_contract.rs`, which builds a real
`engine-serve` `App` (via `AppState`) and asserts the 401 / 404 / 202 paths against it — the
worked reference is `../engine-rs/crates/engine-serve/tests/abort_integration.rs`. The mount
decision itself (`decide_engine_mount`) is unit-tested element-by-element in `src/serve/mod.rs`
against all four presence/absence combinations of `DATABASE_URL` / `BASTION_ENGINE_API_KEY`,
including the empty-string-counts-as-absent case.

---

## 15. Configuration reference

| Env var | Required | Default | Description |
|---|---|---|---|
| `BASTION_SERVE_ADDR` | No | `0.0.0.0:4317` | `host:port` to bind |
| `BASTION_SERVE_TOKEN` | **Yes** | — | Bearer token for protected routes; absent token is a typed error at startup |
| `DATABASE_URL` | No | — | Postgres URL for the engine's durable writer. Absent (or unreachable) leaves the Section 14 engine routes unmounted; bastion's own `/api`/`/ws` surface never needed this and still doesn't. |
| `BASTION_ENGINE_API_KEY` | No | — | `X-API-Key` secret the engine routes (Section 14) check. Absent leaves those routes unmounted. |

`bastion serve` loads config via `load_serve_config()` (`src/config.rs`), which
is DB-free and does **not** require `DATABASE_URL` for its own `/api`/`/ws` surface. The
`[workspaces]` registry consumed by Section 11's routes is loaded separately via
`load_workspace_registry()` — also DB-free — once at server startup. `DATABASE_URL` and
`BASTION_ENGINE_API_KEY` are read directly from the environment at boot (Section 14.1), not
through `load_serve_config()`.

---

## 16. Versioning policy

This document follows a simple monotonic version scheme:

| Change type | Version bump |
|---|---|
| New route or frame kind | v0.x minor bump |
| Breaking change to an existing route/shape | v1 major bump |

`bastion-ui` MUST pin to a specific version tag.  The current contract is **v0.6**.

---

## Amendment Log

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
