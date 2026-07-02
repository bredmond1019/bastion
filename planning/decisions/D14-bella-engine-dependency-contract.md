---
type: Decision
title: "D14: bella-engine dependency discipline — cross-repo contract, images stance, theme rename"
description: bastion depends on bella-engine as an unpinned Cargo path dependency (BA.12.C/BA.12.G). Mirrors bella's D3 — records the same cross-repo contract discipline from bastion's side, bastion's stance on the images feature, and the Theme::bastion() → Theme::mission_control() rename.
doc_id: D14-bella-engine-dependency-contract
layer: [console]
project: bastion
status: active
keywords: [bella-engine, bella, cross-repo dependency, path dependency, shared substrate, images feature, Theme]
related: [D11-herdr-reference-only, D13-unified-console-target]
---

# D14 — bella-engine dependency discipline: cross-repo contract, images stance, theme rename

**Date:** 2026-07-02
**Status:** Accepted
**Supersedes:** —
**Builds on:** D13 (unified console — bella-engine added as a path dep "when a markdown pane is
needed"). Mirrors bella's own `D3-bella-engine-shared-with-bastion.md`; read that one for the
bella-side framing of the same decision.

## Context

`Cargo.toml` pins `bella-engine = { path = "../bella/crates/bella-engine" }` — an unpinned, unversioned
path dependency into a sibling repo. bastion currently imports `bella_engine::{palette::rgb, Theme,
browser::{Browser, BrowserEntryKind}, render_with_edit, links::TableExpansions}` across
`ui_theme.rs` and `sessions/{app,ui}.rs`. bella has five blocks left in its own plan (`BE.2.F`
config/themes, `BE.2.G` images/packaging, `BE.3.H` editor, `BE.3.I` mev validation, `BE.3.J`
formal-absorption cleanup), several of which touch the exact engine surface bastion depends on —
most notably `BE.3.H`, which changes `render_with_edit`/`Rendered`, the two symbols bastion already
calls.

## Decision

1. **bella stays a separate repo, released independently of bastion.** bastion never vendors or
   forks bella's source — it consumes `bella-engine` as a library only, and that will remain true
   through `BE.3.J`.

2. **`bella-engine`'s public API is treated as a cross-repo contract**, informally (no version pin,
   no cross-repo CI — that ceremony isn't worth it yet for a single-maintainer two-repo setup). In
   practice: whenever a bella block changes `bella-engine`'s public surface, verify `cargo build &&
   cargo test` still passes here before treating that block as done. This is a manual discipline
   applied from the bella side (see bella's D3, standing rule 6) — nothing needs to change in
   bastion's own process, just awareness that a build break originating from `../bella` is expected
   from time to time and should be treated as a coordination signal, not a bastion regression to
   chase blindly.

3. **Do not pin `default-features = false` on the `bella-engine` dependency.** bastion does not want
   to rule out image rendering in its own panes (e.g. Space Overview) — excluding the `images`
   feature by default would foreclose that. When bella's `BE.2.G` lands its images decision, bastion
   opts in per-feature deliberately rather than being excluded by a bella-only default.

4. **`Theme::bastion()` is renamed to `Theme::mission_control()`** in `bella-engine` (2026-07-02).
   bastion's call site (`sessions/ui.rs`) is updated to match. The rendered theme itself is unchanged
   and remains bastion's pinned default — only the name changed, so a general-purpose engine crate
   doesn't carry a specific consumer's name as a public API identifier.

## Alternatives considered

- **Pin `bella-engine` as a git dependency with tagged releases.** Deferred — not rejected. Revisit if
  bella-engine's public surface starts changing often enough that untracked breakage becomes a
  recurring cost; until then the path dependency + manual-verify discipline (point 2) is cheaper.
- **Exclude the `images` feature by default to keep bastion's dependency tree lean.** Rejected per
  Brandon: bastion showing images in its own panes is a live possibility, not something to foreclose
  for a dependency-tree optimization that hasn't been shown to matter yet.

## Consequences

- bastion's own decisions log now cross-references bella's D3 — read both together when touching the
  `bella_engine::*` call sites in `ui_theme.rs` or `sessions/`.
- `Theme::mission_control()` is the current call site name; any future engine-side rename must update
  `sessions/ui.rs:304` in the same change (grep `bella_engine::Theme::` in this repo before renaming
  anything upstream).
- No bastion `Cargo.toml` change is made now (no explicit feature list added) — this is a deliberate
  no-op recorded so a future session doesn't "helpfully" add `default-features = false`.
