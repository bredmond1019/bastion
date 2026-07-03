---
type: Index
title: bastion Decisions Registry
description: Index of atomic, append-only architectural decision records for bastion. Active records live here; D1–D10 are retired under archive/decisions/.
doc_id: decisions-index
layer: [console]
project: bastion
status: active
keywords: [decisions, ADR, registry, console, serve, detection]
related: [planning-index, D11-herdr-reference-only, D12-toml-manifest-detection, D13-unified-console-target, D14-bella-engine-dependency-contract]
---

# Decisions Registry

Architectural decision records (ADRs) for bastion. Each decision is **one atomic file**,
append-only — never edit a settled decision; supersede it with a new one and link back.

## Active

- [D11: Herdr is reference-only](./D11-herdr-reference-only.md) — Herdr (Rust "tmux for agents")
  is studied for patterns only; bastion does not fork, vendor, or depend on it (AGPL-3.0, bincode
  Unix-socket transport, non-durable pane IDs, native deps, single-author risk). Patterns reimplemented clean.
- [D12: TOML-manifest agent-state detection](./D12-toml-manifest-detection.md) — `BA.11.C0` builds a
  data-driven `src/detect/` engine (Idle/Working/Blocked/Unknown) from TOML manifest rules (Claude + Pi
  only); `BA.11.C`'s needs-input is a thin adapter over it. Builds on D9.
- [D13: Unified operator console](./D13-unified-console-target.md) — `monitor`, `sessions`, and
  costs/momentum converge into one ratatui shell (`BA.12.A`): sidebar of live-state entities + tabbed/paned
  main + mouse + compute-then-render over one event loop and the `serve` transport. Mouse follows Bella, not Herdr.
- [D14: bella-engine dependency discipline](./D14-bella-engine-dependency-contract.md) — bastion
  depends on `bella-engine` as an unpinned path dependency; treat its public surface as a cross-repo
  contract; don't exclude the `images` feature; `Theme::bastion()` renamed to `Theme::mission_control()`.
  Mirrors bella's own D3.

## Cross-Repo (brain-level)

- **D40: BA.15 posture — open source, built for myself first**
  (`../../../../docs/decisions/D40-ba15-posture-open-source-for-myself.md`) — resolves the cross-repo
  plan's `HQ.D1` gate: BA.15 (Phase 15, Bastion Product Packaging) is demand-first for the operator's
  own use *and* released open source; BA.15.3 (licensing + README) stays in scope, designed for a
  stranger's repo, not just HQ's. Extends brain D26/D22.

## Retired (history)

D1–D10 are settled and live under [`../archive/decisions/`](../archive/decisions/index.md) — durable
residue distilled into `knowledge.md` / `memory.md` (D35). Notable forward references: **D4**
(session-management surface), **D9** (Claude readiness via classify-state), **D10** (code-graph
qualified node IDs).
