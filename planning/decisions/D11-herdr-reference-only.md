---
type: Decision
title: "D11: Herdr is reference-only — do not build on it"
description: Herdr (Rust "tmux for agents") is studied for patterns only; bastion reimplements them clean rather than forking, vendoring, or depending on it.
doc_id: D11-herdr-reference-only
status: active
keywords: [herdr, reference-only, AGPL, console, serve, transport, reimplement]
related: [D12-toml-manifest-detection, D13-unified-console-target, D2-observability-consumer-contract]
---

# D11 — Herdr is reference-only — do not build on it

**Date:** 2026-06-29
**Supersedes:** none
**Builds on:** the serve track (BA.11.A/B/C) + the unified-console direction (D13)

## Context

`herdr` (a Rust "tmux for agents" TUI, on disk at `core/reference-repos/herdr/`) solves several of
the exact problems the Bastion Console and `bastion serve` are about to solve: live agent-state
detection, a compute-then-render TUI with a sidebar of entities, tiled panes with hit-testing, and a
`wait` primitive. The temptation is to fork it, vendor part of it, or take it as a crate dependency to
shortcut `BA.11.C`–`BA.12.x`.

## Decision

**Herdr is a design reference only. bastion does not fork it, vendor it, or depend on it as a crate.
Useful patterns are reimplemented clean.**

Disqualifying factors:

- **License:** Herdr is **AGPL-3.0** — incompatible with bastion's portfolio/closed positioning and
  with shipping a `bastion` binary that other (non-AGPL) work links against.
- **Transport mismatch:** Herdr uses a **binary `bincode` Unix-socket** transport. The Bastion Surface
  (BastionUI) requires **HTTP + WebSocket over Tailscale** with a durable, network-traversable,
  mobile-reconnect-safe wire format (the `WsFrame { kind, payload }` JSON contract — see `src/serve/dto.rs`).
- **Non-durable pane IDs:** Herdr's pane identifiers are not stable across reconnects; the Surface needs
  durable session/pane addressing.
- **Heavy native deps:** vendored C (`libghostty-vt`) + a patched `portable-pty` — a maintenance and
  portability burden bastion does not want to inherit.
- **Single-author risk:** taking a hard dependency on a single-maintainer AGPL project is a supply-chain
  and continuity risk for personal infrastructure.

The patterns worth borrowing — **compute-then-render split** (`src/ui.rs`), **TOML-manifest agent-state
detection** (`src/detect/`, see D12), **sidebar-of-entities layout**, and the **`wait`/poll primitive**
— are reimplemented from scratch against bastion's own types and the `serve` transport.

## Alternatives considered

- **Fork/vendor Herdr.** Rejected: AGPL contaminates the binary; the bincode/Unix-socket transport is
  the wrong shape for the Surface; native deps add portability cost.
- **Depend on Herdr as a crate.** Rejected: same license + transport problems, plus single-author risk.

## Consequences

- `src/detect/` (D12), the unified console layout (D13/`BA.12.A`), and mouse support (`BA.12.C`) are all
  **original implementations**. Mouse specifically follows **Bella's** `map_mouse` pattern (same
  ratatui 0.30 / crossterm 0.29 stack), **not** Herdr's — see D13.
- `core/reference-repos/herdr/` carries no `.git` and is never vendored; it exists purely to read.
- No AGPL code or transitive native dependency enters the bastion build graph.
