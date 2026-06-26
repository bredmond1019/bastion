---
type: Guideline
title: "serve-api contract v0"
description: "HTTP + WebSocket API contract for `bastion serve` — base URL, bearer-auth scheme, GET /health, /ws echo, and the v0 frame envelope that bastion-ui pins against."
doc_id: serve-api
layer: [console, surface]
project: bastion
status: active
keywords: [serve, api, websocket, bearer-auth, health, bastion-ui, contract]
related: [config, observ]
---

# serve-api — v0 Contract

**Version:** v0  
**Produced by:** `bastion` (this repo, `src/serve/`)  
**Consumed by:** `bastion-ui` (Flutter mobile Surface, D28)

This document is the pinned contract between `bastion serve` and the Flutter
`bastion-ui` client.  `bastion-ui` MUST NOT rely on any behaviour not
documented here.  When a later block extends the API it bumps this version
(v0.1, v0.2, …) and records the delta in the Amendment Log at the bottom.

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
token is compared with constant-time equality inside the pure `token_matches`
helper (`src/serve/auth.rs`).

### 2.2 Failure response

A missing, malformed, or incorrect token returns:

```
HTTP/1.1 401 Unauthorized
```

No body is returned.  The client MUST treat any `401` as a fatal auth failure
and prompt the operator to verify the configured token.

### 2.3 Auth policy summary

| Route | Auth required |
|---|---|
| `GET /health` | No (public) |
| `GET /ws` (WS upgrade) | Yes — `Authorization: Bearer <token>` |
| All future `/api/*` routes | Yes — `Authorization: Bearer <token>` |

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
sender unchanged.  Binary frames are accepted and echoed as binary.

Client sends:

```
TEXT: hello
```

Server responds:

```
TEXT: hello
```

This echo surface exists so `bastion-ui` can verify connectivity before the
real session-hub (Block C) is ready.  No state is maintained between frames.

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

## 6. Configuration reference

| Env var | Required | Default | Description |
|---|---|---|---|
| `BASTION_SERVE_ADDR` | No | `0.0.0.0:4317` | `host:port` to bind |
| `BASTION_SERVE_TOKEN` | **Yes** | — | Bearer token for protected routes; absent token is a typed error at startup |

`bastion serve` loads config via `load_serve_config()` (`src/config.rs`), which
is DB-free and does **not** require `DATABASE_URL`.

---

## 7. Versioning policy

This document follows a simple monotonic version scheme:

| Change type | Version bump |
|---|---|
| New route or frame kind | v0.x minor bump |
| Breaking change to an existing route/shape | v1 major bump |

`bastion-ui` MUST pin to a specific version tag.  The current contract is **v0**.

---

## Amendment Log

_No amendments yet._
