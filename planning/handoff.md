---
type: Handoff
created: 2026-07-02
---

# Handoff — BA.13.1 shipped; pick the next Phase 13/14 block

> **For the next agent:** Read this immediately after `/prime`. Delete this file once consumed.

## What we're doing and why
`bastion`'s Unified Console is mid-restructure (Phase 13/14) toward a single spine-based primary
navigator with config-driven theming. This session shipped **BA.13.1 (persistent global agent
panel)** — an always-visible bottom "agents · priority" strip listing every tmux session across
all spaces, sorted by urgency, rendered under every `SelectedNode`. It reuses the theme system
BA.14.0 landed last session and the spine/`SelectedNode` model BA.13.0 landed before that, so all
three Phase 13/14 dependencies chain cleanly now.

## Completed this session
- Fixed the same recurring spec-authoring defect noted in the prior handoff and already tracked
  in the `sdlc-flow-task-heading-format` carryover: `planning/13.1-persistent-agent-panel/tasks.md`
  task headings were `### BA.13.1.1 <title>` instead of the required `### N. BA.13.1.1 <title>`
  (D16 lint). Fixed by editing the four headings directly (in both the main worktree, which was
  reverted, and the spec's dedicated `sdlc-flow` worktree, where the fix was committed) before
  retrying the workflow. This is the third time this exact defect has hit a fresh spec — see the
  existing carryover for why the regex itself isn't the right fix.
- **Ran `/sdlc-flow 13.1-persistent-agent-panel` to completion** (after the heading fix) — merged
  as [PR #12](https://github.com/bredmond1019/bastion/pull/12) (squash-merged to `main`):
  - `src/monitor/app.rs`: extracted a pure `session_urgency(&Session) -> u8` out of
    `build_mission_items`'s inline sort key (Blocked=0, Working/Running=1, else=2) and reused it in
    place; `build_mission_items`'s signature is unchanged (still shared by `monitor/events.rs`).
  - `src/sessions/agent_panel.rs` (new): `AgentPanelRow` model + pure `agent_panel_rows(&[Session])`
    builder, sorted via `session_urgency`, no I/O/theme access — registered via `pub mod
    agent_panel;` in `src/sessions/mod.rs`.
  - `src/sessions/ui.rs`: reserved an always-on bottom strip in the top-level vertical split (now
    3 areas: main / strip / footer) via a new pure `agent_panel_strip_height(row_count,
    frame_height)` (grows 3→7 lines with session count, shrinks toward 0 without underflow/panic
    on tight frames); renders `agent_panel_rows` with themed state dots from
    `ui_theme::current_theme()` (BA.14.0) — never literal colors.
  - Full validation suite green (fmt/clippy `--all-targets -D warnings`/test — 1056 passed/build
    `--release`); one `clippy::collapsible_if` lint fixed in test code along the way. Manual tmux
    `capture-pane` smoke test across Mission Control, a tier, and a space (5 live sessions)
    confirmed the strip renders correctly with no panics at all three `SelectedNode` positions.
  - End review verdict: **PASS**, 0 findings, 1 attempt. Docs patched: `docs/sessions.md`.
- **`/code-review low` on the merged diff: 0 findings.**
- **Post-merge git hygiene:** GitHub's squash merge produced a commit hash on `origin/main`
  distinct from the worktree branch's chain (same pattern as the BA.14.0 close-out). The worktree
  branch also had one trailing bookkeeping commit (`chore: flow state — pr #12`, recording the PR
  URL into `sdlc-flow-state.json`/`worklog.md`) made *after* the push the PR was opened from, so
  it wasn't in the squash. Resolved by: fetch, confirm the ff-only merge fails as expected,
  content-verify `git diff origin/main <branch> --stat` shows only that trailing bookkeeping diff,
  `git reset --hard origin/main`, then `git cherry-pick` the trailing commit onto `main`. Pushed.
  Worktree `trees/13.1-persistent-agent-panel-flow` and its branch are removed.
- Closed `BA.13.1` in `state.json` `tracks[]` (status → `closed`, `tasks[]` array dropped, mirroring
  how `BA.13.0`/`BA.14.0` were closed), regenerated `focus` (`mev emit-state --write`, 0 errors),
  confirmed `mev validate-brain --state` shows no new warnings.

## Remaining work
- **Next Phase 13/14 block** — per regenerated `focus.next` (wave order, BA.7.B/BA.11.E predate
  Phase 13/14 and remain unaddressed ahead of it): `BA.13.2` (mouse interactivity), `BA.13.3`
  (session→space mapping, unblocks `BA.13.4`), `BA.13.5` (HQ file-browser exclusion), `BA.14.1`/
  `BA.14.2` (layout/Mission Control polish), `BA.14.3` (color pass). `BA.13.3` and `BA.13.5` have
  no deps and could run in parallel with anything else.
- **Kanban board TUI access is still gone** (removed with the tab bar in BA.13.0, carried forward
  unresolved across the last two handoffs) — no block explicitly owns restoring it; worth a
  decision (fold into `BA.13.4`'s per-space sub-tab bar, or a new small block).
- **Only the `bastion` theme preset is implemented** — `dark`/`light` were deliberately deferred in
  BA.14.0. `BA.14.3`'s color-value retune is the next place a second preset would plausibly land,
  if one is wanted.
- **Phase 15 (`BA.15.0`–`.11`) is tracked but not started** — still deferred in favor of Console
  work; unresolved since the prior two handoffs. `BA.15.5`/`BA.15.6` are the structural fix for the
  recurring task-heading defect above (see the carryover's `clears_when`).

## Durable State Updates
- `state.json` `tracks[]` — `BA.13.1` flipped to `status: "closed"`.
- No new `carryover[]` entries added. The existing `sdlc-flow-task-heading-format` constraint
  (created 2026-07-02, `clears_when: BA.15.5 and BA.15.6 land`) already covers exactly the
  heading-format issue hit again this session — its text still accurately describes the
  constraint. Worth flagging: this is now a *3rd* recurrence (BA.13.0, BA.14.0, BA.13.1) with no
  spec-authoring-time guard in place; if it recurs a 4th time before Phase 15, consider whether
  `/generate-tasks` should gain a local D16 self-check rather than waiting for BA.15.5/BA.15.6.
- `engine-fmt-red` carryover (pre-existing `src/engine/` fmt debt) is untouched, still open.

## Open questions / choices
- Which Phase 13/14 block to pick up next — no blocking dependency forces an order beyond what's
  in `focus.next`; any of `BA.13.2`/`BA.13.3`/`BA.13.5`/`BA.14.1`–`.3` are unblocked and reasonable
  picks.
- Whether to restore Kanban-board TUI access as its own block or fold it into `BA.13.4` — still
  unresolved from the prior two handoffs.

## Context the next agent needs
None beyond the carryover reference above — the approach for the next block is settled by
`focus.next`, and no in-session framing is needed to interpret it correctly.

## First command after `/prime`
Decide the next Phase 13/14 block from `focus.next` (see Remaining work — `BA.13.2`, `BA.13.3`, or
`BA.13.5` are the unblocked, no-dependency picks), then `/generate-tasks` for it to author the
spec — **double-check the generated `### N. <title>` heading format before running `/sdlc-flow`**
per the `sdlc-flow-task-heading-format` carryover.
