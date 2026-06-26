---
type: Guideline
title: "serve-api contract v0.1"
description: "HTTP + WebSocket API contract for `bastion serve` — base URL, bearer-auth scheme, GET /health, /ws echo, the v0 frame envelope, and the v0.1 session REST surface (list/pane/send/key/create/delete) that bastion-ui pins against."
doc_id: serve-api
layer: [console, surface]
project: bastion
status: active
keywords: [serve, api, websocket, bearer-auth, health, sessions, bastion-ui, contract]
related: [config, observ]
---

# serve-api — v0.1 Contract

**Version:** v0.1  
**Produced by:** `bastion` (this repo, `src/serve/`)  
**Consumed by:** `bastion-ui` (Flutter mobile Surface, D28)

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

All routes **except** `GET /health` are protected by mandatory bearer-token
authentication.

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

## 4. `GET /ws` — WebSocket upgrade

Minimal echo socket.  Protected by bearer auth.

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

### Echo behaviour (v0)

After a successful upgrade, the server reflects every text frame back to the
sender unchanged.  Fragmented text messages (continuation frames) are buffered
and echoed when the final frame arrives.  Binary frames are silently dropped at v0.

Client sends:

```
TEXT: hello
```

Server responds:

```
TEXT: hello
```

This echo surface exists so `bastion-ui` can verify connectivity before the
real session-hub (Block C) is ready.

---

## 5. WebSocket frame envelope (v0 skeleton)

All application-level messages (v0.1+) will be wrapped in the frame envelope
defined here.  At v0 the only concrete `kind` is `echo` (the raw echo actor
does not use the envelope; the envelope is defined here for `bastion-ui` to
pin the schema before Block C ships).

### Wire format

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

### Defined `kind` values (v0)

| Kind | Direction | Description |
|---|---|---|
| `"echo"` | server → client | Reflect the received payload back unchanged |
| `"error"` | server → client | Server-side error notification |

Later blocks extend this table (e.g. `"session_list"`, `"session_attach"`).

### `"echo"` payload

Identical to the payload the client sent.  No defined schema constraint at v0.

### `"error"` payload

```json
{
  "code": "<C-code>",
  "message": "<human-readable message>"
}
```

| Field | Type | Description |
|---|---|---|
| `code` | string | Machine-readable error code from the C0xx taxonomy (`src/observ/`) |
| `message` | string | Human-readable error description |

---

## 6. Sessions REST API (v0.1)

Six routes projecting the synchronous tmux session-control surface onto HTTP.
All routes live under the bearer-protected `/api` scope and return
`Content-Type: application/json`.

### 6.1 Response DTOs

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

### 6.2 Request-body DTOs

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

### 6.3 Routes

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
(see Section 6.4).

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

Returns `404` when the session does not exist (see Section 6.4).

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

Returns `404` when the session does not exist (see Section 6.4).

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

Returns `404` when the session does not exist (see Section 6.4).

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

Returns `404` when the session does not exist (see Section 6.4).

---

### 6.4 Tmux degradation → HTTP status mapping

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

## 7. Configuration reference

| Env var | Required | Default | Description |
|---|---|---|---|
| `BASTION_SERVE_ADDR` | No | `0.0.0.0:4317` | `host:port` to bind |
| `BASTION_SERVE_TOKEN` | **Yes** | — | Bearer token for protected routes; absent token is a typed error at startup |

`bastion serve` loads config via `load_serve_config()` (`src/config.rs`), which
is DB-free and does **not** require `DATABASE_URL`.

---

## 8. Versioning policy

This document follows a simple monotonic version scheme:

| Change type | Version bump |
|---|---|
| New route or frame kind | v0.x minor bump |
| Breaking change to an existing route/shape | v1 major bump |

`bastion-ui` MUST pin to a specific version tag.  The current contract is **v0.1**.

---

## Amendment Log

- **2026-06-26 — v0 → v0.1 (Block 11.B):** Added Section 6 (Sessions REST API): six routes
  (`GET /api/sessions`, `GET /api/sessions/{name}/pane?lines=N`,
  `POST /api/sessions/{name}/send`, `POST /api/sessions/{name}/key`,
  `POST /api/sessions`, `DELETE /api/sessions/{name}`), response DTOs (`SessionDto`,
  `PaneDto`), request-body DTOs (`SendBody`, `KeyBody`, `NewSessionBody`), named-key
  endpoint with accepted key names, and tmux degradation → HTTP status mapping
  (503/404/500) with `ErrorPayload` shape.  Updated auth policy table (Section 2.3) to
  list all six session routes explicitly.  Renumbered Configuration → Section 7 and
  Versioning → Section 8.
