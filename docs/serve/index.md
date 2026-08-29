---
type: Index
title: Network Face & Operator Contact — Docs
description: Index of bastion's outward-facing surfaces — the HTTP+WebSocket API contract for bastion serve, and the Telegram operator transport behind bastion notify.
doc_id: bastion-docs-serve-index
layer: [console, surface]
project: bastion
status: active
keywords: [serve, websocket, api contract, notify, telegram, index]
related: [bastion-cli-docs-index, commands, tuning]
---

# Network Face & Operator Contact

The two ways bastion talks to something that is not your terminal: an authenticated HTTP +
WebSocket server that clients call, and an outbound Telegram transport that reaches the human.

Both are **secret-gated, with no anonymous mode**. Getting the secrets wrong is the most common
failure in this directory — [tuning.md § Secrets](../tuning.md#secrets-and-who-checks-them) lays
out which header carries which.

| File | What it covers |
|---|---|
| [serve-api.md](serve-api.md) | The pinned HTTP + WebSocket contract for `bastion serve` — bind address, the two auth schemes, the `/ws` hub and frame envelope, and every REST surface. `bastion-ui` and `bastion-web` pin against this file |
| [notify.md](notify.md) | `bastion notify send\|ask` — a message, or a gated question that **blocks** until the operator taps an answer |

`serve-api.md` is a **contract**: it is versioned, and per-version deltas live in its Amendment
Log. Do not describe new behaviour there without bumping the version.
