---
type: Decision
title: "D12: Agent-state detection is a TOML-manifest engine, not inline heuristics"
description: BA.11.C0 builds a data-driven src/detect/ engine that classifies agent panes (Idle/Working/Blocked/Unknown) from TOML manifest rules; BA.11.C's needs-input is a thin adapter over it.
doc_id: D12-toml-manifest-detection
status: active
keywords: [detection, manifest, TOML, agent-state, needs-input, claude]
related: [D11-herdr-reference-only, D13-unified-console-target, D9-claude-readiness-via-classify-state]
---

# D12 — Agent-state detection is a TOML-manifest engine, not inline heuristics

**Date:** 2026-06-29
**Supersedes:** none
**Builds on:** D9 (Claude readiness via classify-state), the serve WS hub (BA.11.C)

## Context

`BA.11.C` (the WebSocket hub + live pane streaming) needs to detect when an agent session is **blocked
waiting for input** so it can emit `event{needs_input}` — the killer alert that lets BastionUI surface a
one-tap approval. Today `src/sessions/claude_state.rs` is only a **workspace-trust observer**; it does
not classify run state. The path of least resistance is to hardcode a Claude-specific permission-prompt
heuristic directly inside the Block C hub. That couples detection to one agent and to one call site, and
makes the (I/O-adjacent) hub logic hard to unit-test.

## Decision

**Insert `BA.11.C0` (prework, gates `BA.11.C`): a TOML-manifest-driven agent-state detection engine in a
new `src/detect/` module.**

- Public surface: `detect(screen, manifest) -> AgentDetection { state, visible_* }`, where
  `state ∈ { Idle, Working, Blocked, Unknown }`.
- Manifests are **TOML** rule sets with `any` / `all` / `not` combinators and region selectors
  (e.g. `after_last_horizontal_rule`, `bottom_non_empty_lines(n)`, `whole_recent`, `osc_title`).
- Ship **Claude + Pi manifests only** — a minimal two-agent seam, not a port of Herdr's full agent set.
- `BA.11.C`'s needs-input check becomes a **thin adapter**:
  `detect::detect(pane, manifest).state == Blocked && visible_blocker`.

This is a clean reimplementation of Herdr's manifest pattern (`herdr/src/detect/`), used as **design
reference only** per [D11](./D11-herdr-reference-only.md) — no Herdr code or license enters the build.

## Alternatives considered

- **Inline Claude heuristic inside Block C.** Rejected: agent-coupled (every new agent edits the hub),
  buries classification inside an I/O-adjacent actor where it can't be exhaustively unit-tested, and
  violates the construction-vs-execution split the repo enforces (Rule 6).
- **Port Herdr's full manifest set / 18-agent coverage.** Rejected: AGPL (D11) and scope — Claude + Pi
  are the only agents bastion drives today.
- **Reuse `claude_state.rs` as-is.** Rejected: it observes workspace *trust*, not run *state*; the two
  are different signals.

## Consequences

- Detection is **data-driven**: adding a new agent is a new TOML manifest with **zero engine change**
  (the acceptance bar for `BA.11.C0`).
- The pure engine (rules, combinators, region selectors) is **exhaustively unit-tested without I/O**
  against captured-pane fixtures, per Rule 6; the thin pane-capture shell is smoke-tested.
- `BA.11.C0` is a hard prerequisite of `BA.11.C` — it must land first.
- The `Agent` trait + `--agent` seam (`BA.12.D`) builds on this engine: per-agent detection is selected
  by manifest, mirroring the per-agent driver selected by the trait.
