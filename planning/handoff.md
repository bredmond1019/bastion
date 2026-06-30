---
type: Handoff
title: Handoff — BA.11.C0 complete; BA.11.C (WebSocket hub) is next
description: Session handoff after shipping the agent-state detection engine (BA.11.C0); detect() seam is live, Block C is the clear next step.
doc_id: handoff
layer: [console]
project: bastion
status: active
keywords: [handoff, detect, agent state, websocket, phase 11, BA.11.C]
related: [bastion-status, bastion-detect, master-plan]
created: 2026-06-30
---

# Handoff — BA.11.C0 complete; BA.11.C (WebSocket hub) is next

> **For the next agent:** Read this immediately after `/prime`. Delete this file once consumed.

## What we're doing and why

We are building Bastion's Phase 11 — the live pane streaming + agent-state layer that
BastionUI (D28) needs to show agents as Idle / Working / Blocked in real time.

BA.11.C0 (the agent-state detection engine) shipped this session with a clean PASS and is
now on `main`. It is the seam BA.11.C depends on: the WebSocket hub will call `detect()`
on each captured pane to decide which state to broadcast. The natural next step is to start
BA.11.C (WebSocket hub + live pane streaming), which is the highest-priority remaining
BastionUI item.

## Completed this session

- **BA.11.C0 fully implemented and merged** — pure config-driven agent-state detection engine:
  - `src/detect/mod.rs` — `AgentState`, `AgentDetection`, `detect()` entry point
  - `src/detect/manifest.rs` — TOML manifest schema (`RegionSpec` / `GateSpec` / `RuleSpec`), `Manifest::compile()`, `CompiledManifest`, `CompiledGate::eval()`
  - `src/detect/manifests/claude.toml` + `pi.toml` — seeded Blocked/Working/Idle rules
  - `src/detect/fixtures/` — five captured-pane golden fixtures
  - `src/detect/golden_tests.rs` — six golden tests (zero I/O via `include_str!`)
  - 37 tests in `detect::`, 812 total — all pass; PASS verdict, no review findings
- **`docs/detect.md` created** — full reference doc (manifest schema, gate types, `detect()` API, compile flow, golden fixture table)
- **`docs/index.md` updated** — detect.md row added
- **Worktree merged and cleaned** — branch `11.C0-agent-state-detection-flow-2` merged FF into `main`, removed
- **PR #7** opened (already merged into local main; remote may still show it as open)
- **Light code review (low)** — clean, no findings

## Remaining work

1. **Start BA.11.C** — WebSocket hub + live pane streaming. This consumes `detect()` from
   `src/detect/` for the "needs input" detector seam. Spec is in `planning/master-plan.md`
   Phase 11, Block C. Use `/sdlc-flow <spec-slug>` as with BA.11.C0 and prior blocks.
2. **BA.7.B** (vendor tiktoken counter → exact `bastion costs`) remains available as a
   lower-priority interleave if Block C hits a blocker.
3. **Minor improvement backlog** (from status.md, non-blocking):
   - Confirm `bastion validate` skips `trees/` if worktrees accumulate `.md` files
   - `status` config-file API URL not loaded when `DATABASE_URL` absent (acceptable edge case)

## Open questions / choices

None — clear to proceed. BA.11.C approach is settled: actix WS actor pattern (from Block A),
extend with topic subscriptions and a background poll → watch channel, with `detect()` as the
needs-input seam. Full spec in `planning/master-plan.md`.

## Context the next agent needs

- **812 tests** is the current baseline.
- **`detect()` signature:** `pub fn detect(screen: &str, manifest: &CompiledManifest) -> AgentDetection` in `src/detect/mod.rs`. Manifests are compiled once at startup via `parse_manifest(src)?.compile()?`.
- **Adding a new agent** requires only a new TOML under `src/detect/manifests/` — zero engine-code changes (extensibility property proven by the cross-agent isolation golden test).
- **Block C runtime model constraint:** `bastion serve` runs `actix_web::rt::System::new().block_on(...)` on a dedicated OS thread via `tokio::task::spawn_blocking`. Block C WebSocket actors must follow the same pattern — do not change the runtime model.
- **`serve-api.md` at v0.1.** Any new Block C frame kinds or routes must be documented before the block ships.
- **PR #7** on GitHub may still show as open if the remote hasn't been updated. Local `main` has all 12 commits from the flow.

## First command after `/prime`

`/sdlc-flow <BA.11.C-spec-slug>`
