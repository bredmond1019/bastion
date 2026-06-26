---
type: ProjectStatus
title: bastion Status
description: Current state and progress tracker for bastion.
---

# STATUS — Current State & Progress

**Last updated:** 2026-06-25 — phase6-blockB complete: multi-workspace Brain shipped + code-review fixes merged. 519 tests pass. PASS verdict.
**Current focus:** Bastion-program track — **phase6-blockC** (Structural code navigation — code-as-graph). Phase 4 Blocks B–C remain blocked on orchestrator D28 Phases 4–5 (SSE streaming, TUI node re-run).

---

## How to Read / Update This File

- Status values: `Not started` · `In progress` · `Done` · `Blocked` · `Skipped`
- Keep `Current focus` and `Last updated` accurate; update as work happens.
- This file is **state only**. For what the work means, see `master-plan.md`.

---

## Progress Table

### Phase 0 — Foundation
| Block | What | Status | Notes |
|---|---|---|---|
| Block A | Foundation setup | Done | Both tasks merged (2026-06-20). Toolchain verified, config.rs reads DATABASE_URL + BASTION_API_URL with typed errors, .env.example added. Health probes (API + DB) implemented. `bastion status` command works offline and prints service reachability. All 17 tests pass; all gated checks green. |

### Phase 1 — Monitor
| Block | What | Status | Notes |
|---|---|---|---|
| Block A | DB queries + graph layout | Done | All tasks complete: test fixtures created (in-progress + completed run samples); node_runs JSON → NodeState parsing implemented with RunStatus deserialization; DB queries (list_active_runs, get_run_state) filled with sqlx; topological layout algorithm with grid position assignment verified against linear chains and diamond DAGs; all validation gates pass (cargo fmt, clippy, test, build --release). Cross-contract sync: v1.0.0 aligned (D3). |
| Block B | TUI render loop and event-driven updates | Done | Two-pane ratatui monitor shipped: `App` state model (navigation + `replace_runs`), `ui.rs` render (graph pane with RunStatus coloring + detail pane), `events.rs` event loop (keyboard nav + DB poll via `tokio::select!`). 265 tests pass; all gating checks green. PASS in 2 review attempts (fix: smoke-test degrade paths recorded in ## Notes per Rule 6). Live render path noted as a follow-up when Docker orchestrator stack is available (`docs/index.md` flagged for `monitor.md` addition). |

### Phase 2 — Inspect + Costs
| Block | What | Status | Notes |
|---|---|---|---|
| Block A | bastion inspect | Done | Static post-mortem graph view shipped: `src/monitor/events.rs` widened 3 functions to `pub(crate)`; `src/inspect/mod.rs` replaced `todo!()` with full static loop reusing monitor graph/UI primitives. `build_inspect_app` exhaustively unit-tested (9 cases). 272 tests pass (net +7 over 265 baseline). PASS in 2 review attempts (fix: deferred smoke-test record written to tasks.md § Notes per Rule 6). `docs/inspect.md` created; `docs/index.md` flagged NEEDS_REVIEW for inspect.md row addition. |
| Block B | bastion costs | Done | LLM spend summary shipped: `bastion costs --last <window>` with hardcoded pricing table (`src/costs/pricing.rs`), pure `parse_window`/`within_window`/`aggregate`/`render_table` logic, thin `db::costs::fetch_all_runs` I/O shell reusing `parse_event_row`. 302 tests pass (+30 over 272 baseline). PASS in 1 review attempt. Smoke test deferred per Rule 6 (orchestrator stack not up). Docs: `docs/costs.md` created; `docs/index.md` + `docs/data-contract.md` updated. |

### Phase 3 — Run + Validate
| Block | What | Status | Notes |
|---|---|---|---|
| Block A | bastion run | Done | Workflow trigger shipped: `trigger_workflow` (api/client.rs) and `run::trigger` (run/mod.rs). Pure helpers `trigger_body` (None→`{}` default), `trigger_url` (trailing-slash normalisation), `parse_args` (JSON validation + non-object rejection), `format_trigger_success` (greppable `task_id:` line). 316 tests pass (+14 over 302 baseline). PASS in 1 review attempt. Live smoke test deferred (needs orchestrator stack); recorded in tasks.md §Notes per Rule 6. Docs: `docs/run.md` created; `docs/index.md` flagged NEEDS_REVIEW for run.md row. |
| Block B | bastion validate | Done | Tasks 1–5 complete. Module skeleton, frontmatter validation, link checking, report rendering, and fixtures all implemented and tested. Validation gate passed: all four gating checks (fmt, clippy, 404 tests, release build) pass. Smoke tests verify correct behavior on both clean and dirty fixtures. No new crate dependencies added. Ready for merge. |

### Phase 4 — Polish
| Block | What | Status | Notes |
|---|---|---|---|
| Block A | Config file + help/man polish | Done | Config file support (`~/.config/bastion/config.toml`; env > file > built-in precedence), `bastion help` enrichment (`long_about` + `after_help` examples), and `bastion man` hidden subcommand (pure `render_man()` + `clap_mangen`). 428 tests pass (+24 from 404 baseline). PASS in 1 review attempt. Two remaining Phase 4 items (SSE streaming, TUI node re-run) deferred — blocked on orchestrator D28 Phases 4–5. |
| Block B | SSE streaming | Blocked | Blocked on orchestrator D28 Phase 4 |
| Block C | TUI node re-run | Blocked | Blocked on orchestrator D28 Phase 5 |

### Phase 5 — Session Management (independent, ungated track — D4)
| Block | What | Status | Notes |
|---|---|---|---|
| Block A | `bastion sessions` + tmux wrapper + lazy DB pool | Done | sessions/ module shipped: tmux.rs (pure arg construction + typed errors), model.rs (Session/Pane + fixture parsing), commands.rs (list verb, graceful degradation, render), CLI wiring. DB-free guarantee enforced by architecture and locked in by test. 20 new tests; 73 total pass. All gating checks green. PASS in 1 review attempt. |
| Block B | `attach` / `new` / `kill` (lifecycle) | Done | attach/new/kill verbs shipped: pure arg-construction functions, interactive attach_session (.status()), new_session, kill_session; graceful degradation for NotInstalled/NoServer/ExitError; format_created/format_killed helpers unit-tested. PASS in 1 review attempt. Follow-up chore (2026-06-21) closed the error-path test gaps: extracted pure `classify_no_server` (tmux.rs) + `degrade_tmux_error`/`Degraded` (commands.rs); 9 new tests, 88 total, all gating checks green. **Deferred manual smoke test now COMPLETE** — verified live against tmux 3.6b: new (incl. `--dir` cwd), sessions list, kill (valid + unknown-session error), attach unknown-session error, and the interactive `attach`→`Ctrl-b d` detach round-trip (returns cleanly to shell). |
| Block C | `bastion send` (keystrokes) | Done | `bastion send` shipped: `send_keys_args` with `-l`/`--` for literal delivery, `send_enter_args` for separate Enter keypress, execution fn + CLI wiring + graceful degradation. 9 new tests; 96 total pass. All gating checks green. PASS in 1 review attempt. |
| Block D | `bastion capture` (pane output) | Done | `Pane::last_lines` with trailing-blank stripping; capture verb + CLI wiring + graceful degradation. 14 new tests; 110 total pass. All gating checks green. PASS in 1 review attempt. |
| Block E | session view in the TUI | Done | ratatui session dashboard shipped: `SessionApp` state model (29 unit tests), `ui.rs` render + event loop (6 unit tests + smoke-tested), CLI wired so bare `bastion` and `bastion tui` both launch the dashboard. 145 tests pass; all gating checks green. PASS in 1 review attempt. |
| Block F | session activity indicator + Claude trust observer | Done | Activity indicator shipped: `classify_state(pane_current_command)` replaces session_attached as the state source; detached-but-running sessions now correctly show `running (cmd)`. Trust observer shipped: new `claude_state.rs` reads `~/.claude.json` as a read-only observer and prints advisory trust pre-flight on `bastion new --dir`. 36 new tests (145 → 181). All gating checks green. PASS in 1 review attempt. Smoke-tested DB-free (D4) and synchronous (D5). |
| Block G | `bastion ask` (one Claude Code turn) | Done | `bastion ask` shipped: pure `done_path`/`trigger_text`/`poll_plan`/`has_session_args` helpers; `AskError` enum (`UntrustedDir`, `Tmux`, `Launch`, `Timeout`); thin I/O shell (ensure session+Claude → send trigger → poll `<out>.done`). 26 new tests (181 → 206+). All gating checks green. PASS in 2 review attempts (fix: `classify_state()==Running` replaces exact `"claude"` string check, since Claude Code v2.1.185 sets `ucomm` to its version string). Smoke-tested: cold start → PONG written → exit 0; warm reuse (no relaunch); timeout → exit 1 + stderr; untrusted dir → fail fast; unknown dir → proceeds. DB-free (D4) and synchronous (D5) confirmed. |

### Bastion-program track (Phases 6–10) — demand-first by D26 wave

> bastion's execution slice of the cross-repo Bastion program (brain `planning/bastion-product/`,
> governed by brain **D24 / D25 / D26**). Sequenced to follow the program's demand-first **wave**
> order, not bastion-internal dependency. `prog.` = the program's global block letter; `½` = the
> bastion half of a cross-repo block (orchestrator/base-template peer is a prerequisite for the
> *combined* claim; the bastion half is independently shippable). Opportunistic and ungated.

#### Phase 6 — Brain & code retrieval (Wave 1)
| Block | What | Prog. | Status | Notes |
|---|---|---|---|---|
| Block A | Vendor `knowledge_graph` → structural query over the OKF `[[link]]` graph | A | Done | petgraph-backed BrainGraph + structural query layer (dependents/blast-radius/lineage) + `bastion brain` CLI wired. 522 tests pass. PASS 2026-06-25. |
| Block B | Multi-workspace Brain — graph reader over per-repo/per-client roots | C½ | Done | Workspace registry + pure resolver in config.rs; portable OKF fixture corpus; `--workspace`/`--knowledge-dir` CLI flag; DB-free confirmed. Code-review fixes: MalformedFile propagation, NoWorkspaceRegistry variant, double-print, empty-corpus hint, Config::load dedup. 519 tests pass. PASS 2026-06-25. |
| Block C | Structural code navigation (code-as-graph) | Q | Not started | Builds on 6A. Structural twin of semantic code search (program Block P, Engine). |

#### Phase 7 — Observability & control (Wave 2)
| Block | What | Prog. | Status | Notes |
|---|---|---|---|---|
| Block A | Tracing + `C0xx` structured-error spine | H | Not started | Foundational for the track. Vendors the `claude-sdk-rs` error taxonomy once (`src/observ/errors.rs`). |
| Block B | Vendor tiktoken counter → exact `bastion costs` | D | Not started | Swaps the Phase 2 Block B estimation core for exact counts. No deps. |
| Block C | Cost as a budgeted resource: `--watch`, alerts, `bastion kill`, gate | I½ | Not started | Builds on 7A (7B strengthens). D25 — kill *triggers* an orchestrator abort endpoint; D20 contract bump. |

#### Phase 8 — Client-grade Brain integrity (Wave 3)
| Block | What | Prog. | Status | Notes |
|---|---|---|---|---|
| Block A | Deterministic Brain-integrity validation | K | Not started | Builds on 6A. The hard anti-hallucination layer; produces the findings record Phase 10 consumes. Forward-looking. |

#### Phase 9 — Protocol & local inference (Wave 4)
| Block | What | Prog. | Status | Notes |
|---|---|---|---|---|
| Block A | Vendor `workflow-engine-mcp` → Console MCP / tool client | E½ | Not started | Built with the Python Brain-as-MCP-server (program Block R). Uses 7A's `C0xx` model. Forward-looking. |
| Block B | Rust local-model node from the `claude-sdk-rs` spine | F | Not started | Python-free `bastion ask` path (D23). Reuses 7A's error model; DB-free (D4) / synchronous (D5) preserved. Forward-looking. |

#### Phase 10 — Self-healing loop (Wave 5)
| Block | What | Prog. | Status | Notes |
|---|---|---|---|---|
| Block A | Proactive scanner → issue backlog | M | Not started | Builds on 7A + 8A. Writes an OKF issue backlog (dedup/priority/dismiss). Forward-looking. |
| Block B | Findings → spec → draft PR via `sdlc-flow` (no auto-merge) | N½ | Not started | Builds on 10A. D25 — bastion *triggers* `sdlc-flow`; never authors/merges the PR. Cross-repo peer: base-template findings→spec entry point. Forward-looking. |

<!-- Add one sub-table per phase as the plan is fleshed out. -->

---

## Decisions & Deviations Log

*Record deviations from the plan and notable in-flight choices here. Promote durable ones to
`decisions/` via `/log-work`.*

- **2026-06-25 — Bastion-program track added to the master-plan (Phases 6–10).** Extracted bastion's execution slice of the reorganized cross-repo Bastion program (brain `planning/bastion-product/`, now 20 blocks A–S across 7 phases, governed by brain **D24** Python/Rust seam, **D25** read-only-state/triggered-mutations, **D26** Bastion-the-system naming + demand-first ordering + code-aware Brain + MCP client/server split). Pulled in all 11 bastion-touching blocks (program A, C½, Q, H, D, I½, K, E½, F, M, N½), organized into bastion Phases 6–10 mapped one-to-one onto program **Waves 1–5** (demand-first, *not* moat-first). Excluded as non-bastion: program B, J, L, O, P, R, S (orchestrator/brain) and G (loop-proof — coordinated from bastion, artifact in the brain's `docs/content/`). Notable cross-repo seams to honor: D25 (kill + self-healing PR *trigger* the Engine/Factory, bastion never mutates), D20 contract bump for the abort/budget-gate endpoints (Phase 7 Block C), and single-vendor of the `C001–C014` error taxonomy in Phase 7 Block A (reused by Phase 9 Block B). An earlier moat-first Phase 6/7 draft was discarded and regenerated against the new source. First runnable block: **phase6-blockA**.
- **2026-06-21 — Orchestrator D28 confirmed landed → phase1-blockB unblocked.** Verified in `../python-orchestration-system` that the monitor's read contract is now fully satisfied on the orchestrator side: (1) incremental node-level persistence — `Workflow.run()` takes an `on_progress` callback (`app/core/workflow.py:126,158,168`) the worker wires to `persist_progress` (`app/worker/tasks.py:52`), so `task_context.node_runs` is now written at every node boundary, not only at terminal completion (the original D2/D28 gate); (2) per-node status/timing/input/output/token stamping in `core/task.py` + `nodes/agent.py` + `nodes/tool_use.py`; (3) DAG edges via `GET /workflows` + `GET /workflows/{type}/graph` (`app/api/graph.py`). Orchestrator status log records D28 "Done (2026-06-20)" — all 8 tasks merged via /sdlc-block, 244 tests. D28 Phases 4 (status column) and 5 (SSE push) remain deferred *by original scope* and do not block Block B (the 2s poll over incremental Postgres state is exactly what D2/D3 designed for). bastion **D2** gate lifted. Consumed + deleted `planning/handoff.md`.
- **2026-06-21 — Phase 5 Block G added: `bastion ask` for the cross-repo Claude Code LLM provider.** The Python orchestrator is adding two ways to run an LLM node on Claude Code (subscription): a headless `claude-agent-sdk` mode (no bastion dependency) and a `CLAUDE_CODE_SESSION` mode that drives the live interactive session **through bastion** so the turn is observable in `bastion sessions`. Block G adds `bastion ask` — one command (ensure session+Claude → fixed trigger → wait for `<out>.done`) — as the stable contract the orchestrator shells out to. Surface is pinned in the brain at `agentic-portfolio/docs/integrations/claude-code-llm-provider.md` §2 (`bastion ask` v0.1.0); spec at `planning/phase5-blockG/tasks.md`. Reuses Block F's `classify_state` (skip relaunch) + trust observer (fail fast on untrusted dir). DB-free (D4) + synchronous (D5) preserved.
- **2026-06-21 — Phase 5 Block F rule 6 backfill + doc drift fix.** The live smoke test that the pipeline skipped (Detached sessions showing running command vs idle shell, and trust pre-flight) was backfilled manually against tmux 3.6b this session and recorded in planning/phase5-blockF/tasks.md ## Notes (Rule 6 coverage bar met). A doc audit discovered README.md had drifted: missing the `capture` verb shipped back in Block D + the table and examples were out of sync. Root cause: `/document` was not reconciling the README command table against the evolving `src/cli.rs` Commands enum. Fixed in `.claude/commands/document.md` so `/document` now auto-syncs the README table on any cli.rs changes. Added a "Verifying the surface" runbook to docs/sessions.md covering how to smoke-test blocks manually (session creation, attach, send, capture, kill).
- **2026-06-21 — Phase 5 Block F added from the blockE live test.** Driving Claude Code through bastion live surfaced two gaps: a running Claude Code session reads "idle" (SessionState is keyed on `session_attached`, not the pane's foreground process), and hands-off `send`-launches stall on Claude's one-time workspace-trust prompt per new directory. Verified `#{pane_current_command}` is available in `tmux list-sessions -F`, and that Claude already persists trust in `~/.claude.json` (`projects[dir].hasTrustDialogAccepted`) — so a bastion-owned trust store would be a redundant, drift-prone second source of truth; Block F reads it as a read-only observer instead (same posture as toward the orchestrator's Postgres). Added Block F to master-plan.md (Phase 5 section + quick-ref table).
- **2026-06-21 — bastion absorbs tmux session management; second surface added (D4).** What was
  sketched as a standalone tool (working name `brain`) is folded into bastion as modules instead:
  a `sessions/` module and a `bastion sessions` command family, with a session view in the TUI.
  Rationale: the standalone tool's charter ("operator interface that grows into the client
  appliance shell") was already bastion's, and bastion is shaped for it (single crate; already
  has `clap`/`ratatui`/`crossterm`; tmux needs no new deps). bastion now has two surfaces —
  workflow observability (Postgres, gated by D2) and process/session control (tmux, **ungated**).
  Added **Phase 5** (Blocks A–E) to `master-plan.md` as an independent track. One real constraint:
  the Postgres pool must become **lazy** so session commands run with zero DB. Cross-repo: brain
  **D21**; recorded as bastion **D4**.
- **2026-06-21 — Decisions D5–D6 promoted to registry (phase5-blockA deferred choices).** D5 (Session verbs are synchronous: tmux shell-outs are blocking `std::process::Command` calls, so no async ceremony or tokio coupling to the sessions/ surface) and D6 (Malformed `tmux list-sessions` output lines are skipped with stderr warning rather than aborting; partial system state is more useful than none). Both build on D4 and finalize the sessions/ surface contract.
- **2026-06-21 — User-facing documentation + test-coverage standing rule.** Reviewed phase5-blockC test coverage (judged sufficient); added CLAUDE.md standing rule 6 "Coverage bar" codifying the separate-pure-logic-from-I/O testing pattern already in use (D5/D6); filled in README.md skeleton and added docs/sessions.md (operator manual) + docs/index.md (router). Docs-only chore; all gates green.
- **2026-06-20 — Pinned the orchestrator data contract v1.0.0 (D3).** The orchestrator now publishes a single, versioned contract (`python-orchestration-system/docs/data-contract.md`) for the execution state bastion reads; bastion holds a consumer view (`docs/data-contract.md`) pinned to v1.0.0. Confirmed the **Hybrid** read path (direct Postgres for the live poll; reserved HTTP read API later) and the **two-source merge model** (DAG edges from `GET /workflows/{type}/graph`, live state from `events.task_context.node_runs`, joined by node **class name**). Realigned `master-plan.md` and the Phase-1 stub type defs to reality (no relational `workflow_runs`/`node_states` tables exist): `NodeState` gained `model`/`input`; `RunStatus` deserializes lowercase status strings; `ApiClient::workflow_graph()` added; `build_layout` now takes API edges. Orchestrator-side additions that complete the contract: per-node `input` + serializable output (orchestrator D30). `cargo fmt`/`clippy`/`test` (17) all green. Cross-repo: brain D20 / orchestrator D30. `/log-work` gained a contract sync-checklist step.
- **2026-06-18 — Pre-Block-A reconnaissance against the live orchestrator.** Read the
  python-orchestration-system to ground Block A. Findings: (1) orchestrator state is one `events`
  table with JSON `data` + `task_context` columns — no relational runs/nodes tables; the DAG is
  reconstructed by parsing `task_context`. (2) `/health` returns only `{status, version}` on port
  **8080** (not 8000 as the scaffold `.env.example` said); DB is `postgres`/`postgres`@5432, db
  name `postgres` (not `orchestrator_db`). Both config defaults to be corrected in Block A. (3)
  Worker count / queue depth live in Redis, out of bastion's configured scope → **Block A status
  scoped to DB + API only**; Redis-backed metrics deferred (see D2). (4) **Critical upstream
  dependency:** `task_context` is persisted only once, at the end of a run — so a live monitor has
  no intermediate state to read. The orchestrator owns the fix (incremental node-level
  persistence): orchestrator DECISIONS **D28** + plan `incremental-execution-observability.md`.
  bastion Phase 1 (monitor) is gated on that plan's Phase 1 landing. Recorded as bastion **D2**.
  Test path for Block A: stand up a local Postgres + apply the orchestrator migration for true
  end-to-end verification, plus unit tests for the unreachable/degraded path.

---

## Quick Self-Check

- Is `Current focus` accurate?
- Any `In progress` rows that are actually `Done`?
- Anything `Blocked` that needs surfacing?

---

*State only. For what things mean, see master-plan.md. For orientation, see context.md.*
