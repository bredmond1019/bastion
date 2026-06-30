---
type: Plan
title: bastion Master Plan
description: Strategic roadmap and phase specifications for bastion.
doc_id: master-plan
layer: [console]
project: bastion
status: active
keywords: [master plan, phases, blocks, strategy, roadmap, bastion program, TUI]
related: [context, planning-index]
---

# bastion — Master Plan

*Living document. Created 2026-06-18.*

## The Goal, Stated Plainly

`bastion` is a personal Rust CLI that makes the agentic engineering stack observable and operable from a single terminal command. The problem it solves: when a Python orchestrator workflow fails at node 7 of 12, you currently piece together what happened from Celery logs, Redis state, and raw SQL across three terminal panes. `bastion monitor` collapses that into one command — a live graph where nodes go green or red in real time, and the selected node shows its full input, output, error trace, and token count in a side pane.

"Ready" means: `bastion monitor` works against the live Python orchestrator, showing at least two distinct workflow types as navigable TUI graphs with accurate real-time state. Secondary commands (`inspect`, `costs`, `run`, `validate`, `status`) are functional at whatever phase they ship.

## The Destination

A single binary — `bastion` — that is the terminal entry point for the entire personal engineering stack. You open one pane, run one command family, and know what your system is doing. Longer term: a credible example of custom observability tooling you can describe to engineering clients.

bastion has **two surfaces** under one roof (brain D21 / bastion D4):

1. **Workflow observability** (`monitor`, `inspect`, `costs`, `run`) — reads the orchestrator's PostgreSQL state. The phases below (0–4) build this. Phase 1 is gated by D2.
2. **Process / session control** (`status`, `sessions` family) — shells out to tmux to manage the long-running Claude Code sessions on the Mac Mini. Phase 5 builds this. It depends on neither Postgres nor the orchestrator and is therefore an **independent, ungated track** — workable at any time, accessible from desktop or phone via SSH over Tailscale.

A third track (**Phases 6–11**) is bastion's slice of the **Bastion program** — the five-layer
practice OS planned in the brain at `planning/bastion-product/` and governed by brain decisions
**D24** (the Python/Rust seam — Rust harvested as a tested parts-bin, never a second engine), **D25**
(bastion stays **read-only** for state; every mutation — abort, PR — is *triggered* through the Engine
or Factory, never performed by bastion directly), and **D26** (Bastion = the system, `bastion` = this
Console binary; **demand-first** ordering; the Brain spans docs *and* code *and* memory; MCP is split
into a Python server + a Rust client). In that program the **Console (`bastion`)** owns the
deterministic, model-free substrate: structural graph queries over the Brain, exact cost + budget
control, the tracing/observability spine, Brain-integrity validation, an MCP client, a local-model
node, and the proactive scanner that *triggers* self-healing PRs. These phases are sequenced to follow
the program's **demand-first wave order** (D26), not bastion-internal dependency; the per-phase notes
below map each bastion phase to its program wave. The Brain is the named moat — the layer competitors
can't trivially clone — so the Console's job here is to make it queryable, fresh, correct, observable,
and self-healing. This track is opportunistic and ungated; pick up blocks as the need appears.

A fourth track (**Phase 11**) gives the Console a **network face** — `bastion serve`, an actix-web
HTTP+WebSocket API that projects tmux/session control, repo/workflow status, and quick-actions onto
the Tailscale tailnet so the Flutter **`bastion-ui`** app can operate the stack from a phone. It is
bastion's server-side slice of the separate **BastionUI** cross-repo program (brain
`planning/bastion-ui/master-plan.md`, governed by brain **D28**, upholding **D21**/**D25**). It is
fully independent of Phases 0–10 — additive, touching only a new `src/serve/` module — and runs in
parallel with current work.

## North-Star Alignment (umbrella view)

> **Added 2026-06-27.** The cross-repo program master-plan was reorganized around the
> north star into
> **capability tracks** (see `planning/bastion-product/master-plan.md`). This section maps **bastion's
> phases onto those tracks** so the two plans read as one — **nothing here is removed or renumbered**;
> the phase/block structure below is load-bearing (the `phaseN-blockX` convention `/generate-tasks`
> parses) and stays exactly as is. This is just the *capability lens* over it, plus the new program
> blocks bastion now owns. **This file is the worked reference the orchestrator + bastion-ui reorgs copy.**

**bastion phase → program capability track:**

| bastion phase | Program capability track | What bastion owns in it |
|---|---|---|
| Phase 6 (Brain & code retrieval) | **Track 1 — Brain: Context & Memory** | structural graph queries (docs + code), multi-workspace reader |
| Phase 7 (Observability & control) | **Track 2 — Console: Observe, Cost & Control** | the tracing/`C0xx` spine, exact cost, budget+kill, **+ the momentum/metrics surface (new Block V)** |
| Phase 8 (Brain integrity) | **Track 3 — Verification & Brain Integrity** | deterministic integrity validation (the hard anti-hallucination layer) |
| Phase 9 (Protocol & local) | **Track 2 — Console** (protocol/local) | the MCP **client** half, the Rust local-model node |
| Phase 10 (Self-healing loop) | **Track 4 — Self-Improvement & Self-Healing** + **Track 5/6** | proactive scanner + self-healing-PR trigger, **+ the incident harness (new Block C) and the autonomy/trust-ladder half (new Block D)** |
| Phase 11 (`bastion serve`) | **BastionUI sub-program** (umbrella §Surface) | the Console network face the Flutter Surface pins |

**In-flight vs queued (bastion's program-track slice — authoritative status in `status.md`):**
- **🟢 Done:** Phase 6 A/B/C (graph, multi-workspace, code-nav) · Phase 7 A (tracing/`C0xx` spine) ·
  Phase 11 A/B (serve scaffold + session REST).
- **🟡 In flight:** Phase 11 **C₀ (agent-state detection manifest engine — new prework, gates C)** ·
  Phase 11 C (WS hub — current focus) · Phase 7 B (exact `costs`) deferred-but-next.
- **⚪ Queued:** Phase 7 C (budget+kill) · Phase 7 **D (momentum/metrics surface — new)** · Phase 8 A
  (integrity) · Phase 9 A/B (MCP client, local model) · Phase 10 A/B (scanner, self-healing PR) ·
  Phase 10 **C (incident harness — new)** · Phase 10 **D (trust ladder — new)** · Phase 11 D–I.

**The north-star tracks bastion owns** (its deterministic, model-free substrate, D25 read-only/trigger):
observability + exact cost + the kill switch (Phase 7), the structural graph + code navigation
(Phase 6), Brain-integrity validation (Phase 8), the MCP client + local-model node (Phase 9), the
proactive scanner that *triggers* self-healing PRs (Phase 10), and — **new from the umbrella reorg** —
the **incident & recovery harness** (program Block Y), the **momentum/metrics Console surface** (program
Block V), and **bastion's half of the autonomy/trust ladder** (program Block X). The three new blocks are
specified in their phases below and added to the sequence table; the eval engine (program Block U) and
external-intelligence loop (program Block W) stay in the **orchestrator**, not here.

## Architecture / Design Overview

`bastion` is an **observer, never a writer** of the Python orchestrator. It reconstructs a live run
by merging **two sources**, joined on **node class name** (the identity key):

1. **DAG shape** — `GET /workflows/{type}/graph` (FastAPI) → `{nodes, edges}`. Fetched once per
   workflow type; this is the *only* source of edges and of nodes that haven't run yet.
2. **Live per-node state** — the orchestrator's **PostgreSQL** `events.task_context.node_runs`
   (read-only, polled). Every node is present as `pending` from the first write, then transitions
   `running → success|failed` with timing, token usage, input, and errors.

There are **no** relational `workflow_runs` / `node_states` tables — all run state is JSON in the
`events` table. The contract for all of this (table shape, `node_runs` fields, endpoints, status
strings) is the orchestrator-owned, versioned
[data contract](../docs/data-contract.md); bastion **pins** a version of it.

Read path is **Hybrid**: direct Postgres for the live poll now; a reserved orchestrator HTTP read
API (`GET /events/{id}`) is documented for later but not depended on. The TUI uses `ratatui` +
`crossterm` for rendering and event handling. `petgraph` manages the DAG structure and topological
layout. `tokio` drives the async event loop (DB poll + keyboard events). `reqwest` handles FastAPI
calls — the graph endpoint, `bastion run`, and future node re-run.

```
src/
├── main.rs          clap dispatch → subcommand modules
├── cli.rs           all subcommand + flag definitions
├── config.rs        DATABASE_URL / BASTION_API_URL from env
├── db/
│   ├── workflows.rs  parse events.task_context: active runs, node_runs, outputs
│   └── costs.rs      aggregate node_runs[*].usage token totals
├── api/client.rs     reqwest: workflow_graph (DAG), trigger run, health check
├── monitor/          live TUI (ratatui loop, petgraph layout, crossterm events)
│   ├── app.rs        state: selected run/node, should_quit
│   ├── graph.rs      WorkflowRun → petgraph DAG → grid positions
│   ├── ui.rs         ratatui render: two-pane layout
│   └── events.rs     tokio loop: keyboard + DB poll interval
├── inspect/          static post-mortem view (monitor minus polling)
├── validate/         markdown/MDX validation (mirrors markdown-engine-validator)
├── costs/            LLM spend summary (tabular stdout)
├── run/              workflow trigger (FastAPI) + stack status check
└── sessions/         tmux session control (Phase 5)
    ├── tmux.rs       thin wrapper over `std::process::Command` → tmux CLI
    ├── model.rs      Session/Pane types parsed from tmux output
    ├── commands.rs   sessions/attach/new/send/capture/kill verbs
    └── ui.rs         ratatui session view (Block E)
```

> **Lazy DB pool (D4):** the `sessions/` surface must run with zero Postgres connectivity. The
> pool is opened on demand by the workflow-observability commands only — `sessions` commands and
> the session TUI never touch it. If `main` currently opens the pool eagerly, Phase 5 Block A
> makes it lazy.

---

## Phase 0 — Foundation

### BA.0.A — Foundation setup
- **What:** Verify the Rust toolchain compiles the scaffolded project. Implement `bastion status` end-to-end: connect to PostgreSQL and the FastAPI health endpoint, print a summary of what's reachable. Add a `.env.example`.
- **Why:** Proves the DB connection and HTTP client work before any TUI work starts. Useful as a pre-flight check from day one.
- **Build notes:** `config.rs` reads `DATABASE_URL` and `BASTION_API_URL` from env. `run::status()` calls `api::client::ApiClient::health()` and a test PostgreSQL query. Print a formatted table: `DB ✓`, `API ✓` (or `unreachable` per service). Worker count / queue depth live in Redis, which is out of bastion's configured scope — **scoped out** of `status` (see D2).
- **Acceptance criteria:** `cargo build` passes. `cargo test` passes. `bastion status` prints real health data against the running Python orchestrator.

---

## Phase 1 — `bastion monitor`

### BA.1.A — DB queries + graph layout
- **What:** Implement `db::workflows` against the **`events` table** (not relational tables):
  list active runs (rows whose `node_runs` aren't all terminal), and parse one row's
  `task_context` into per-node state (`node_runs[name]` → status/timing/error/input/usage;
  `nodes[name]` → output). Add `api::client::workflow_graph(type)` for the DAG `{nodes, edges}`.
  Build `monitor::graph` — construct a `petgraph` DAG from the **API edges**, overlay live node
  state by **class-name join**, and compute a topological grid layout.
- **Why:** The data layer must be solid before any TUI rendering. Layout bugs are easier to debug
  in unit tests than inside a live TUI. Edges come from the API; status comes from the DB — keep
  the two sources explicit (see [data contract](../docs/data-contract.md) §2).
- **Acceptance criteria:** Unit tests cover the `node_runs` JSON → state parse (against a captured
  fixture), the graph-endpoint → edges parse, the class-name join, topological ordering, and
  position assignment. Status strings deserialize from `pending|running|success|failed`.

### BA.1.B — TUI render loop
- **What:** Implement `monitor::ui` (two-pane ratatui layout) and `monitor::events` (tokio loop with keyboard + DB poll). Wire through `monitor::app` state. `bastion monitor` (no arg → auto-pick the active run) enters the TUI and displays live workflow state. Detail pane reads, per the [data contract](../docs/data-contract.md) §6: status/timing/error/input/tokens from `node_runs[name]`, output from `nodes[name]`, run input from `events.data`.
- **Why:** The core deliverable.
- **Acceptance criteria:** `bastion monitor` renders a running workflow as a live graph. Arrow-key navigation moves the selected node. State updates within the poll interval (the orchestrator persists at every node boundary, so each transition is observable). `q` exits cleanly.

---

## Phase 2 — Inspect + Costs

### BA.2.A — `bastion inspect`
- **What:** Reuse monitor graph/UI code with polling disabled. Load a completed run by ID from PostgreSQL and render it as a static navigable graph.
- **Acceptance criteria:** `bastion inspect <run-id>` renders any completed run. Navigation works. Exiting returns to the shell cleanly.

### BA.2.B — `bastion costs`
- **What:** Implement `db::costs` aggregation queries. `bastion costs --last 7d` prints a formatted table of workflow names, run counts, token totals, and estimated USD cost.
- **Acceptance criteria:** Output matches manual SQL queries against the same data. Handles `7d`, `30d`, `all` windows.

---

## Phase 3 — Run + Validate

### BA.3.A — `bastion run`
- **What:** Implement `api::client::trigger_workflow`. `bastion run <workflow> [--args '{}'] [--monitor]` issues `POST /` with `{workflow_type, data}` (the orchestrator's generic dispatcher — see [data contract](../docs/data-contract.md) §7), prints the returned `task_id`, optionally drops into `bastion monitor` for that run.
- **Acceptance criteria:** Successfully triggers a workflow. `--monitor` flag works.

### BA.3.B — `bastion validate`
- **What:** Port or shell-out to `markdown-engine-validator` logic. Scan a content directory, validate frontmatter, check links, report errors with file + line.
- **Acceptance criteria:** Detects known-bad frontmatter and broken links in test fixtures.

---

## Phase 4 — Polish

- SSE streaming from FastAPI instead of DB polling (orchestrator plan Phase 5 — the `on_progress` seam is reserved for it; not built yet)
- Node re-run from TUI (`r` key → `api::client::rerun_node`) — **requires new orchestrator support** (no re-run endpoint exists today; would be a contract addition)
- `~/.config/bastion/config.toml` support so DB URL isn't always an env var
- `bastion help` improvements; man page

---

## Phase 5 — Session Management (independent, ungated track)

The process/session-control surface (brain D21 / bastion D4). Manages the tmux sessions on the
Mac Mini that hold long-running Claude Code sessions. Shells out to the tmux CLI via
`std::process::Command` — **no Postgres, no orchestrator dependency**, so this track is not gated
by D2 and can be picked up at any time. Workflow: from the phone, SSH into the Mini over Tailscale
→ run `bastion` → use the session verbs or the TUI. bastion **manages** these sessions; it does
not run Claude Code itself.

Build order is strict and incremental — each verb ships only when reached for.

### BA.5.A — `bastion sessions` (+ tmux wrapper + lazy DB pool)
- **What:** Stand up the `sessions/` module: `tmux.rs` (thin `std::process::Command` wrapper),
  `model.rs` (`Session`/`Pane` parsed from `tmux list-sessions` / `list-panes` output).
  Implement `bastion sessions` — list sessions, each with its last pane output line (via
  `capture-pane -p`). **Make the Postgres pool lazy** so this command runs with zero DB
  connectivity (D4).
- **Why:** First useful thing, and it forces the two foundations everything else builds on — the
  tmux wrapper and the lazy-DB refactor.
- **Acceptance criteria:** `bastion sessions` lists real tmux sessions with last-line output and a
  running/idle indicator. Runs with Postgres stopped. Unit tests parse captured `tmux` output
  fixtures into `Session`/`Pane` (no live tmux required in CI). Gracefully reports when tmux isn't
  installed or no server is running.

### BA.5.B — `bastion attach` / `new` / `kill` (session lifecycle)
- **What:** `bastion attach <session>` (exec into `tmux attach -t`), `bastion new <session>
  [--dir PATH]` (`tmux new-session -d -s … -c …`), `bastion kill <session>` (`tmux kill-session`).
- **Why:** Core lifecycle — create, enter, dispose.
- **Acceptance criteria:** Each verb performs the corresponding tmux action against a real server.
  `attach` hands the terminal cleanly to tmux and returns to the shell on detach. `new` honors
  `--dir`. Bad/unknown session names produce clear errors. Command-construction logic is unit
  tested without spawning tmux.

### BA.5.C — `bastion send` (send keystrokes without attaching)
- **What:** `bastion send <session> <cmd>` → `tmux send-keys -t <session> <cmd> Enter`.
- **Why:** Trigger actions in a session from the phone without a full attach.
- **Acceptance criteria:** Keystrokes arrive in the target session. Quoting/escaping of
  multi-word commands is correct and unit tested. Clear error on unknown session.

### BA.5.D — `bastion capture` (read pane output non-interactively)
- **What:** `bastion capture <session> [--lines N]` → `tmux capture-pane -p -t <session>`,
  print the last N lines.
- **Why:** Check what a session is doing without attaching — the read counterpart to `send`.
- **Acceptance criteria:** Prints recent pane output for a session. `--lines` bounds the output.
  Output parsing/trimming is unit tested against fixtures.

### BA.5.E — session view in the TUI
- **What:** A `ratatui` session dashboard (reachable as `bastion` no-arg or `bastion tui`,
  alongside the monitor view): list of sessions with status + last line; `[a]` attach (drop into
  full tmux attach), `[n]` new, `[s]` inline send, `[k]` kill, `[q]` quit.
- **Why:** The ergonomic operator surface that ties the verbs together — the piece that makes it
  pleasant from a phone.
- **Acceptance criteria:** The dashboard lists live sessions and refreshes; selection + the
  documented key actions work; `a` drops into a real tmux attach and returns cleanly; `q` exits.
  Built entirely on the Block A–D primitives.

### BA.5.F — session activity indicator + Claude trust observer
- **What:** Make the session list tell the truth about what is *running*, and pre-flight the
  Claude Code trust prompt.
  - **Activity indicator:** add `#{pane_current_command}` to the `list-sessions` format and use the
    active pane's foreground command to classify each session — *idle shell* (command ∈
    `{zsh, bash, sh, fish}`) vs *running `<cmd>`* (anything else, e.g. `claude` / `node` / `cargo`).
    This replaces the current attached-client signal, which mislabels a detached-but-busy Claude
    Code session as "idle". Optionally surface "active Ns ago" from the already-fetched
    `session_activity` epoch. Render the new state in both `bastion sessions` and the TUI dashboard.
  - **Trust observer:** read `~/.claude.json` `projects["<dir>"].hasTrustDialogAccepted` as a
    **read-only observer** (never written — Claude owns that file) to warn, before/at launch, that a
    directory is untrusted and will show Claude Code's one-time trust prompt. Degrade gracefully:
    missing file/field → "trust: unknown", never an error, never blocks the launch.
- **Why:** The live test exposed that a running Claude Code session reports `idle` (state is keyed on
  whether a *client* is attached, not on what the pane is doing) — the one fact the phone workflow
  needs is "is this still working?". And hands-off `send`-launches silently stall on the trust prompt
  the first time per new directory. Both make the operator surface honest about real session state.
- **Acceptance criteria:** `bastion sessions` and the TUI distinguish a running command (incl. a live
  Claude Code session) from an idle shell, derived from `pane_current_command`. The trust observer
  reports whether a target directory is trusted by reading `~/.claude.json`, degrading gracefully when
  the file or field is absent, and never writes to it. Pure parsing/classification (command →
  activity, JSON → trust flag) is exhaustively unit-tested against fixtures; the thin I/O shell is
  smoke-tested. DB-free (D4) and synchronous (D5) invariants preserved.

### BA.5.G — `bastion ask` (one Claude Code turn for an external caller)
- **What:** A new non-interactive subcommand that performs a **single Claude Code "turn"** against an
  interactive session and exits when the answer file is ready — the contract the Python orchestrator's
  `CLAUDE_CODE_SESSION` provider shells out to (so a workflow LLM node runs on the subscription, billed
  to the live session, and is observable in `bastion sessions`).
  ```
  bastion ask --session <name> --prompt-file <path> --out <path>
              [--dir <trusted-workdir>] [--timeout <secs=180>]
              [--launch-cmd "claude --permission-mode bypassPermissions"]
  ```
  Behavior: (1) ensure the session + Claude are running — if `has_session` is false, `new-session -d`,
  send `--launch-cmd`, wait for readiness; reuse Block F's `classify_state` to skip launch when Claude
  is already running and Block F's trust observer to **fail fast** on an untrusted `--dir`. (2) Send a
  short fixed **trigger** (the only keystrokes): `Read <prompt-file> and follow its instructions exactly.
  Write your complete answer to <out>. When finished, create an empty file <out>.done`. (3) Poll for
  `<out>.done` up to `--timeout`, remove it, exit `0` (answer at `<out>`); on timeout `capture-pane` to
  stderr and exit non-zero. bastion is **payload-agnostic** — it guarantees `<out>` exists and is
  complete; the caller decides JSON vs markdown via the prompt file.
- **Why:** Closes the loop for the cross-repo "Claude Code as an LLM provider" feature — gives the
  orchestrator one clean, stable command instead of choreographing raw `send`/`capture` itself, and
  keeps all tmux/session logic in bastion (its proper home). Contract is pinned in the brain:
  `agentic-portfolio/docs/integrations/claude-code-llm-provider.md` §2 (`bastion ask` v0.1.0).
- **Build notes:** new module `src/sessions/ask.rs` reusing `tmux.rs` (`new_session`, `send_keys`,
  `capture_pane`, `has_session`) + Block F's `classify_state` / `trust_status`. Keep the Coverage-bar
  split: trigger-string construction, `send-keys` arg vectors, `<out>`→`<out>.done` derivation, and
  launch/readiness command building are pure and unit-tested element-by-element; the poll loop + process
  spawn is the thin I/O shell, smoke-tested and recorded in `## Notes`. DB-free (D4) and synchronous (D5)
  preserved.
- **Acceptance criteria:** `bastion ask` against a pre-trusted dir produces `<out>` and exits 0; the
  session it used appears in `bastion sessions` (Block F) as *running (claude)*; a timeout exits non-zero
  with stderr diagnostics and does not falsely report success; an untrusted `--dir` fails fast (never
  stalls). Pure logic exhaustively unit-tested; gated checks (`cargo fmt --check`, `cargo clippy -- -D
  warnings`, `cargo test`, `cargo build --release`) pass.

---

## Bastion-program track (Phases 6–11) — orientation

Phases 6–11 are **bastion's execution slice of the cross-repo Bastion program** (brain
`planning/bastion-product/master-plan.md`). That program is wave-ordered **demand-first** (D26) across
five repos and uses global block letters **A–Z** (plus the **HL1–HL5** Harness Library); the blocks whose execution home is the Console land
here. Each bastion phase below corresponds to one program **wave**, and each block notes its **program
letter** (e.g. *program Block Q*) so the two plans stay cross-referenceable. bastion block letters are
**local to each phase** (Block A, B, C…) per the `phaseN-blockX` convention `/generate-tasks` parses —
they are *not* the program's global letters.

Cross-cutting rules for this whole track:
- **D25 — read-only state, triggered mutations.** bastion never writes orchestrator state or merges a
  PR. It *reads* (Postgres, files) and *triggers* (an Engine abort endpoint, an `sdlc-flow` run). Every
  block that "acts" does so by calling out, never by direct mutation.
- **D24 — vendor, don't depend.** Rust crates are copied from the read-only `workflow-engine-rs` /
  `claude-sdk-rs` portfolio repos into bastion and adapted here; they are never live dependencies.
- **Consolidate the harvested error model.** The `claude-sdk-rs` `C001–C014` taxonomy is vendored
  **once** (Phase 7 Block A) into `src/observ/errors.rs`; later blocks (e.g. the local-model node)
  reuse it rather than re-vendoring.
- Several blocks are the **bastion half of a cross-repo block**; their orchestrator/base-template peer
  is called out under *Out of scope* as a prerequisite for the combined claim. The bastion half is
  authored to be independently shippable.
- **North-star block-contract trio (added 2026-06-27).** Every program-track capability block (Phases
  6–11) carries three north-star fields before its *Acceptance criteria* — **Ratchet** (the reusable
  asset it leaves behind), **Eval slice** (the eval domain it feeds in the orchestrator's eval engine,
  program Block U — or "n/a — deterministic acceptance only"), and **Ladder rung** (its position on
  solve→repeatable→skill→workflow→harness→eval→automation→monitor→trust→package). These mirror the
  fields the brain `/generate-master-plan` command was extended with (see
  `planning/bastion-product/master-plan.md`); a block is "done" only when it graduates a rung and leaves
  a ratchet behind. The pre-program Phases 0–5 (the original bastion CLI) predate this convention and
  are left as-is.

> Distant blocks (Phases 8–10) carry the full skeleton but are **forward-looking** — expect their
> Files / interface lines to need refinement when each becomes next.

---

## Phase 6 — Brain & code retrieval (program Wave 1)

Deepen the Brain with **structural** retrieval (the model-free, Console-side twin of the Engine's
semantic retrieval) and extend it from docs to **code**. Per `ownership.md`, code is just another
corpus that is both semantic (Engine) and structural (Console). This phase vendors the
`knowledge_graph` crate and runs its algorithms over graphs derived from the OKF `[[link]]` corpus and,
later, from source.

### BA.6.A — Vendor `knowledge_graph` → structural query over the OKF `[[link]]` graph *(program Block A)*
- **What:** Vendor the `knowledge_graph` crate (A\*, Dijkstra, topological sort, traversal; PageRank +
  community detection available for Phase 8) from `workflow-engine-rs` into bastion, **decoupled from
  its Dgraph backing**, and run its algorithms over a graph derived from the OKF corpus — markdown
  documents as nodes, `[[link]]` references as edges. Expose a `bastion brain` subcommand answering
  structural questions: dependents ("what depends on D21"), blast-radius ("what breaks if the data
  contract changes"), and lineage ("trace a decision's lineage").
- **Why:** Program Wave 1, the retrieval-depth foundation everything else in this track leans on. A
  self-contained crate that *adds* structural retrieval rather than duplicating the Engine's semantic
  retrieval. The OKF brain is *already* a graph (docs joined by `[[links]]`).
- **Files:**
  - *New* `src/brain/mod.rs` (subcommand entry + wiring)
  - *New* `src/brain/graph.rs` (vendored `knowledge_graph` algorithms, Dgraph-free)
  - *New* `src/brain/okf.rs` (pure OKF reader: parse frontmatter + extract `[[link]]` edges → node/edge lists)
  - *New* `src/brain/query.rs` (pure dependents / blast-radius / lineage queries over the built graph)
  - *New* `src/brain/fixtures/` (small OKF corpus fixture for tests)
  - *Modified* `src/cli.rs` (add `brain` subcommand + flags), `src/main.rs` (dispatch), `Cargo.toml` (vendored-crate deps; reuse `petgraph` if it suffices)
- **Interfaces / shared surface:** Consumes the OKF `[[link]]` convention as the edge contract. The
  graph builder's `(root) → (nodes, edges)` signature and `graph.rs` algorithm API are the shared
  surface Blocks B and C (and Phase 8) build on.
- **Out of scope:** No Dgraph stand-up (file-derived graph only). No semantic/pgvector retrieval (Engine
  / program Block B — stays Python). No semantic+structural merge into one ranked answer (the query
  router — a later per-consumer decision). No code graph yet (Block C). No integrity checks yet (Phase
  8). Source repo `workflow-engine-rs` is read-only.
- **Ratchet:** the vendored Dgraph-free `knowledge_graph` query layer + the OKF→graph builder
  (`src/brain/`) — the structural-retrieval engine Blocks B/C and Phase 8 reuse.
- **Eval slice:** structural-retrieval correctness (dependents / blast-radius / lineage) over the OKF
  fixture — feeds the structural domain of program Block U.
- **Ladder rung:** solve→repeatable→skill→workflow — a reusable Console structural-query capability
  (rung 4).
- **Acceptance criteria:** `bastion brain` returns correct dependents/lineage for a known OKF node (e.g.
  D20's dependents match its stated relations); the graph is built from the live brain repo corpus; the
  vendored crate compiles with **no** Dgraph dependency; per CLAUDE.md Rule 6 the pure OKF→graph builder
  and query functions are exhaustively unit-tested against the fixture and the thin file-walk I/O shell
  is smoke-tested + recorded in `tasks.md §Notes`; gated checks pass (`cargo fmt --check`,
  `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`).

### BA.6.B — Multi-workspace Brain (bastion graph reader over per-repo / per-client roots) *(program Block C, bastion half)*
- **What:** Generalize the bastion graph reader to point at an **arbitrary knowledge workspace** (a
  config/CLI-provided root) and to address **multiple** workspaces (per-repo, per-client), not only this
  repo's hardcoded path. Same OKF + graph behavior over any conforming directory, selectable by name.
  This is the Console half of the cross-repo multi-workspace block.
- **Why:** Program Wave 1. Proves the Brain is a *capability* over any knowledge dir, not a hardcode of
  one repo — the groundwork the code corpora (Block C), entity/memory, and the loop-proof all need.
- **Files:**
  - *Modified* `src/brain/okf.rs` (take a workspace root parameter — keep it pure: root in, node/edge lists out)
  - *Modified* `src/brain/mod.rs` (resolve the workspace root from flag/config; address by name)
  - *Modified* `src/cli.rs` (`--workspace` / `--knowledge-dir` on the `brain` subcommand)
  - *Modified* `src/config.rs` (workspace registry + default; follow the existing env > file > built-in precedence)
  - *New* `src/brain/fixtures/portable/` (a second, non-repo OKF workspace fixture)
- **Interfaces / shared surface:** Produces/consumes a shared **"knowledge workspace" convention** (a
  named root + OKF expectations) consumed identically by the Python RAG reader and this graph reader.
  Builds on Block A's `(root) → (nodes, edges)` signature.
- **Out of scope:** The Python indexer/retriever multi-workspace generalization (program Block C
  orchestrator half — separate repo, separate sitting; the combined "multi-workspace Brain" claim needs
  both, but this half is independently shippable). De-opinionating the OKF format. Multi-brain switching
  UX beyond name selection. Packaging/install. **Cross-repo prerequisite:** program Block B (semantic
  store) for the semantic half; this block is structural-only.
- **Ratchet:** the workspace-root resolver + the shared "knowledge workspace" convention (the bastion
  half) — reused by code corpora (Block C), the momentum surface (Phase 7 Block D), and per-client
  sub-brains.
- **Eval slice:** portability — graph correctness over a second, non-repo OKF workspace — a program
  Block U domain.
- **Ladder rung:** skill→workflow — generalizes the graph reader from one hardcoded root into a
  parameterized capability (rung 4).
- **Acceptance criteria:** the bastion graph reader indexes and answers over a **second**, non-repo OKF
  workspace selected by `--workspace` / config; the default still resolves to the brain repo; a
  portability fixture is covered; gated checks pass (`cargo fmt --check`, `cargo clippy -- -D warnings`,
  `cargo test`, `cargo build --release`).

### BA.6.C — Structural code navigation (code-as-graph) *(program Block Q)*
- **What:** Console-side exact symbol / definition / reference lookup and a **code-as-graph** (imports,
  calls) alongside the docs-as-graph — answering "where is this defined, what calls it, what breaks if I
  change it." Deterministic and model-free (tree-sitter / ripgrep over source), reusing Block A's
  `knowledge_graph` algorithms over a graph built from code.
- **Why:** Program Wave 1. The structural twin of the Engine's semantic code search (program Block P),
  mirroring how Block A is the structural twin of semantic doc retrieval. Fast, exact, offline — the
  Console's wheelhouse.
- **Files:**
  - *New* `src/brain/code.rs` (source → symbols/defs/refs extraction via tree-sitter; pure where possible)
  - *New* `src/brain/code_graph.rs` (build the imports/calls graph; run `graph.rs` queries over it)
  - *New* `src/brain/fixtures/code/` (small multi-file source fixture for tests)
  - *Modified* `src/cli.rs` (a `bastion brain code` / `bastion code` surface for def/refs/dependents), `src/main.rs` (dispatch), `Cargo.toml` (tree-sitter grammar deps)
- **Interfaces / shared surface:** Reuses `src/brain/graph.rs` (Block A) and the workspace-root resolver
  (Block B) over a code-derived graph. Produces a Console code-navigation surface.
- **Out of scope:** Semantic "how does X work" code search (program Block P — Engine/Python). Cross-repo
  refactoring or edits. Whole-repo call-graph completeness for every language (scope to the project's
  primary languages; note coverage). Source repo `workflow-engine-rs` read-only.
- **Ratchet:** the code-graph builder (`src/brain/code*.rs`) over Block A's algorithms — exact
  symbol/def/refs as a reusable, model-free Console capability (the deterministic twin of program
  Block P).
- **Eval slice:** structural code-nav correctness (def / refs / dependents) — a program Block U domain.
- **Ladder rung:** skill→workflow — the deterministic code-navigation capability (rung 4).
- **Acceptance criteria:** `bastion` returns the correct definition + references for a known symbol in a
  target repo and answers a code-dependents query over the fixture; extraction respects
  function/class/symbol boundaries; per Rule 6 the pure extraction/graph logic is unit-tested and the
  file-walk shell smoke-tested + recorded in `tasks.md §Notes`; gated checks pass (`cargo fmt --check`,
  `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`).

---

## Phase 7 — Observability & control (program Wave 2)

Give the operator a real spine: see what is happening, what it costs, and stop it. Today bastion has
**no** structured logging/tracing/metrics (errors are `eprintln!`), `costs` is retroactive estimation,
and there is **no** kill switch or budget enforcement. Much of the machinery is **under-harvested Rust
code** (`claude-sdk-rs` ships the `C001–C014` taxonomy + telemetry hooks; `workflow-engine-mcp` ships
circuit breakers + metrics). Per D25, control actions *trigger* the Engine; they never mutate it.

### BA.7.A — Tracing + structured-error spine *(program Block H)*
- **What:** Introduce `tracing` (spans + structured fields) across bastion and vendor the
  `claude-sdk-rs` **`C001–C014` error taxonomy + `ErrorContext`** as the Console's error model,
  replacing the ad-hoc `eprintln!` / bare-`anyhow` surface. Every command emits structured events
  (start, outcome, duration, error code) to a local rolling log; add a `--verbose` / `--json-logs`
  surface.
- **Why:** Program Wave 2, and foundational for the rest of this track — you cannot alert on, cap, or
  self-heal what you cannot see. Harvests the *telemetry half* of `claude-sdk-rs` that D24 left on the
  table; the taxonomy is vendored **once here** and reused by later blocks.
- **Files:**
  - *New* `src/observ/mod.rs` (tracing init + structured event emission helpers)
  - *New* `src/observ/errors.rs` (vendored `C001–C014` taxonomy + `ErrorContext`; the Console error model)
  - *Modified* `src/main.rs` (init tracing/subscriber; top-level error → `C0xx` mapping), `src/cli.rs` (global `--verbose` / `--json-logs` flags), `Cargo.toml` (`tracing`, `tracing-subscriber`)
- **Interfaces / shared surface:** Produces a structured event stream + the `C0xx` error model that
  every later block (cost alerts, kill, integrity findings, scanner) emits into. Per-command event
  emission is folded into each command module incrementally (append-style within those modules).
- **Out of scope:** Distributed tracing / OpenTelemetry export (later, only if a backend is stood up).
  Orchestrator-side tracing (the Engine owns its own). A metrics backend/dashboard. Source repo
  read-only.
- **Ratchet:** the `tracing` spine + the vendored `C0xx` taxonomy (`src/observ/`) — the structured
  event stream every later block (cost, kill, scanner, incidents) emits into; vendored **once** here,
  reused thereafter.
- **Eval slice:** n/a — deterministic acceptance only (event-emission + error-code-mapping tests).
- **Ladder rung:** solve→repeatable→skill — the foundational observability skill the whole track builds
  on (rung 3).
- **Acceptance criteria:** every subcommand emits a structured start/outcome/duration event; errors
  carry a `C0xx` code + context; `--json-logs` produces machine-parseable output; the vendored taxonomy
  compiles in bastion; per Rule 6 event-emission and error-code mapping are unit-tested; gated checks
  pass (`cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`).

### BA.7.B — Vendor `workflow-engine-core` token counter → exact `bastion costs` *(program Block D)*
- **What:** Vendor the `workflow-engine-core` token counter (real `tiktoken-rs`, `cl100k_base` /
  `o200k_base` encoders) into bastion and replace the current hardcoded-pricing **estimation** in
  `bastion costs` with **exact** token counts feeding the USD spend summary. Minimal surface change —
  `bastion costs` already exists (Phase 2 Block B).
- **Why:** Program Wave 2. Exact > estimated, available as a library for free — and the input the
  budget/kill controls (Block C) act on.
- **Files:**
  - *New* `src/costs/tokens.rs` (vendored exact token counter; pure `count(text, model) → usize`)
  - *Modified* `src/costs/mod.rs` (use exact counts in `aggregate`/`render`), `src/costs/pricing.rs` (feed exact counts into the USD math), `Cargo.toml` (add `tiktoken-rs`)
- **Interfaces / shared surface:** Consumes the orchestrator's cost/usage fields in the **D20 data
  contract**. Produces exact token/cost figures (the spend signal Block C watches). No data-contract
  change (read path).
- **Out of scope:** Re-pricing logic / new pricing tables beyond what `bastion costs` has. Budget
  enforcement, `--watch`, kill (Block C). Source repo read-only.
- **Ratchet:** the vendored exact token counter (`src/costs/tokens.rs`) — a reusable exact-cost
  primitive for `costs`, the budget gate (Block C), and the cost-to-success metric (program Block U).
- **Eval slice:** n/a — deterministic acceptance only (exact-count parity test); feeds cost-to-success
  in program Block U.
- **Ladder rung:** solve→repeatable — swaps estimation for an exact, reusable counter (rung 2).
- **Acceptance criteria:** `bastion costs` reports counts that **match** the tiktoken encoders for a
  known input (exact, not estimated); a unit test asserts exact-count parity on a fixed sample
  (element-level); the vendored counter compiles in bastion; gated checks pass (`cargo fmt --check`,
  `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`).

### BA.7.C — Cost as a budgeted resource: `--watch`, alerts, `bastion kill`, pre-dispatch gate *(program Block I, bastion half)*
- **What:** Make cost *actionable*: a live `bastion costs --watch`, configurable **budget thresholds
  with alerts**, a **kill switch** (`bastion kill <run>`) that aborts a run by **calling a new
  orchestrator abort endpoint** (the Engine performs the cancel + terminal stamp — D25), and a
  **pre-dispatch budget gate** so `bastion run` refuses/warns when a run would exceed the ceiling.
- **Why:** Program Wave 2. Retroactive estimation is not control; this closes *observe spend → cap
  spend → stop spend*. The kill is operator/threshold-triggered with confirmation, never silent.
- **Files:**
  - *New* `src/costs/watch.rs` (live spend-watch loop emitting `observ` events), `src/costs/budget.rs` (pure threshold + gate evaluation), `src/run/kill.rs` (kill → orchestrator abort call)
  - *Modified* `src/api/client.rs` (call the new abort endpoint), `src/cli.rs` (`costs --watch`, `kill` subcommand, `run` budget-gate flag), `src/config.rs` (budget thresholds)
- **Interfaces / shared surface:** Consumes D20 cost/usage fields and Block A's event stream (to alert
  on). **Produces two new D20 contract additions** — an authenticated abort endpoint and a budget-gate
  field/response — bumped per the CLAUDE.md D20 protocol and re-pinned in bastion's `data-contract.md`.
  Consumes Block B's exact counts (estimates work until B lands).
- **Out of scope:** The orchestrator-side abort endpoint + server-side budget enforcement (program Block
  I orchestrator half — the enforcement point; this block is the Console surface + trigger). Per-client
  billing. Direct Celery/Redis manipulation by bastion (D2/D25 — the Engine owns the abort). Silent
  auto-kill.
- **Ratchet:** the budget-gate + kill-switch surface + the two D20 contract additions (abort endpoint +
  budget field) — the first real *gated action* the trust ladder (Phase 10 Block D) attaches to.
- **Eval slice:** control-action correctness (gate honored, kill reaches terminal state) — a
  policy/safety slice for program Block U.
- **Ladder rung:** workflow→automation→monitor — closes observe→cap→stop spend (rung toward automation +
  monitor; earned auto-proceed comes via Phase 10 Block D).
- **Acceptance criteria:** `bastion costs --watch` shows live spend; crossing a threshold emits an alert
  event; `bastion kill <run>` aborts via the orchestrator endpoint and the run reaches terminal state in
  `node_runs`; `bastion run` honors the budget gate; the pure budget/gate logic is unit-tested per Rule
  6 and the contract additions are recorded in `data-contract.md`; gated checks pass (`cargo fmt
  --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`).

### BA.7.D — Console reads momentum & metrics (the hybrid dashboard surface) *(program Block V)* *(new — north-star)*
- **What:** Surface the **D30 momentum queues** (now/next/blocked/improve/recurring) + the **Metrics**
  block across every workspace's `status.md` — reading the **frontmatter scalars** (`now`/`next`/`blocked`)
  for a glanceable cross-repo view and the body sections for detail. Extend `bastion status` (or a
  `bastion status --momentum` / `bastion momentum`) to answer "what's in flight, what's blocked, where is
  momentum" across the whole registry in one place.
- **Why:** Program Wave 2 (✲, pull forward once ≥3 repos carry the D30 sections). north-star §"The
  interface should answer these questions immediately." The D30 scalars were designed
  **queryable-but-not-embedded** precisely so a Console surface can sort across repos from the YAML head
  cheaply. Read-only (D25) — `/log-work` writes the queues; bastion only reads them.
- **Files:**
  - *New* `src/momentum/mod.rs` (subcommand/flag entry), `src/momentum/parse.rs` (pure `status.md`
    frontmatter-scalar + Momentum/Metrics section parser → typed rollup), `src/momentum/render.rs`
    (pure cross-repo table render)
  - *Modified* `src/cli.rs` (`status --momentum` / `momentum` surface), `src/main.rs` (dispatch),
    `src/config.rs` (reuse `load_workspace_registry` for the repo list)
  - *(Reuse opportunity: Phase 11 Block D builds a `status.md` parser in `src/serve/status/repo.rs` —
    share the pure parser between the CLI surface and the serve surface rather than duplicating it.)*
- **Interfaces / shared surface:** Consumes the **D30 `status.md` frontmatter scalars + body sections**
  (read-only) + the workspace registry (multi-workspace Brain, Phase 6 Block B). Produces no mutation.
- **Out of scope:** Writing/mutating the queues (D25 — that's `/log-work`, brain HQ-Restructure Block J).
  Metrics *computation* beyond reading the section. The serve/HTTP projection of momentum (a later Phase
  11 surface if wanted).
- **Depends on:** the D30 convention stamped across repos (brain HQ-Restructure Thread 1 / Block Q);
  Phase 6 Block B (the workspace registry) — light.
- **Ratchet:** the cross-repo momentum/metrics rollup surface (`src/momentum/`) — the glanceable
  operational dashboard over the D30 scalars; its pure parser is shared with Phase 11 Block D's serve
  surface.
- **Eval slice:** n/a — deterministic acceptance only (output matches source `status.md` fixtures).
- **Ladder rung:** skill→workflow→monitor — turns the D30 scalars into a monitored cross-repo view
  (rung toward monitor).
- **Acceptance criteria:** `bastion status --momentum` (or `bastion momentum`) lists each registry repo's
  now/next/blocked from frontmatter and rolls up the Metrics sections; output matches the source
  `status.md` files on a fixture; per Rule 6 the pure parse/render logic is exhaustively unit-tested and
  the file-walk shell smoke-tested + recorded; gated checks pass (`cargo fmt --check`, `cargo clippy -- -D
  warnings`, `cargo test`, `cargo build --release`).

---

## Phase 8 — Client-grade Brain integrity (program Wave 3)

Make the Brain *trustworthy*, not just searchable: catch correctness defects deterministically, before
they ever reach an LLM. This is the **hard** anti-hallucination layer (prompt-based grounding is soft),
and the Rust graph code already supports it. Forward-looking — refine Files when it becomes next.

### BA.8.A — Deterministic Brain-integrity validation *(program Block K)*
- **What:** Extend `bastion validate` + the Phase 6 Block A graph to catch defects deterministically:
  broken `[[links]]`, orphan nodes, stale cross-references (a doc points at a section/decision that
  moved or changed), and **structurally-contradictory decisions** (two decisions making opposite claims
  on the same topic). Use `knowledge_graph`'s **PageRank + community detection** to surface the
  most-central docs and detect drift / isolated clusters.
- **Why:** Program Wave 3. A deterministic graph check is a *hard* correctness guarantee — the single
  highest-leverage anti-hallucination move on the corpus side, and the structured findings it produces
  feed the Phase 10 self-healing loop.
- **Files:**
  - *New* `src/validate/integrity.rs` (broken-link / orphan / stale-ref / contradiction checks over the brain graph; emits a structured findings record)
  - *Modified* `src/validate/mod.rs` (`--integrity` mode + report), `src/brain/graph.rs` (expose PageRank / community outputs if not already), `src/cli.rs` (`validate --integrity`)
- **Interfaces / shared surface:** Consumes the OKF `[[link]]` graph (Phase 6 Block A) + OKF frontmatter
  conventions. Produces a structured **integrity-findings** record — the documented input contract for
  Phase 10 Block A.
- **Out of scope:** LLM-judged *semantic* contradiction (this block is deterministic/structural only;
  fuzzy contradiction detection is a later LLM-assisted refinement — program Block L, Engine-side).
  Auto-fixing (Phase 10). Answer-time grounding (program Block L). **Depends on** Phase 6 Block A.
- **Ratchet:** `bastion validate --integrity` + the structured integrity-findings record — the hard,
  deterministic anti-hallucination corpus check that feeds the Phase 10 self-healing loop.
- **Eval slice:** integrity-check precision (zero false positives on a curated fixture) — a program
  Block U domain.
- **Ladder rung:** skill→workflow→eval — the deterministic verification skill that licenses
  self-healing (rung 6).
- **Acceptance criteria:** `bastion validate --integrity` reports broken links, orphans, stale refs, and
  structurally-contradictory decisions over the live brain with **zero false positives** on a curated
  fixture; PageRank / community outputs are exposed; the findings record is documented as the Phase 10
  contract; per Rule 6 the pure check logic is unit-tested against fixtures; gated checks pass (`cargo
  fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`).

---

## Phase 9 — Protocol & local inference (program Wave 4)

Give the Console a protocol seam (MCP client) and its own offline brain (a Python-free local-model
path). Forward-looking — refine Files when each becomes next.

### BA.9.A — Vendor `workflow-engine-mcp` → Console MCP / tool client *(program Block E, bastion half)*
- **What:** Vendor the multi-transport (`HTTP` / `WS` / `stdio`) MCP client with connection pooling from
  `workflow-engine-mcp` into bastion as the Console's protocol/tool client. Demonstrate it against an
  existing MCP server (the crate ships example servers); built together with the Engine's
  Brain-as-MCP-**server** (program Block R) as distinct seam halves (D26).
- **Why:** Program Wave 4 — the protocol seam. `brain-rag` Layer 3 = MCP; this is the *client* side that
  connects Console ↔ Brain-as-MCP-server ↔ tools.
- **Files:**
  - *New* `src/mcp/mod.rs` (client entry + optional demo subcommand wiring), `src/mcp/client.rs` (vendored MCP client + connection pooling), `src/mcp/transport.rs` (HTTP / WS / stdio)
  - *Modified* `src/cli.rs` (optional `mcp` demo subcommand to list/invoke a tool), `src/main.rs` (dispatch), `Cargo.toml` (transport deps as needed)
- **Interfaces / shared surface:** Produces a Console-side **MCP client** across three transports. Uses
  Phase 7 Block A's `C0xx` error model for transport/process errors. The **Brain-as-MCP-server**
  (program Block R, Python) is the server peer — its contract is defined there.
- **Out of scope:** Building the Brain-as-MCP-server (program Block R, python-orchestration — the
  prerequisite for the end-to-end Brain-query claim; this block targets the crate's example servers for
  acceptance). Wiring specific brain tools. Auth beyond what the vendored client provides. Source repo
  read-only.
- **Ratchet:** the vendored multi-transport MCP client (`src/mcp/`) — the reusable protocol/tool client
  for the Console, ready when the Brain-as-MCP-server (program Block R) lands.
- **Eval slice:** MCP transport round-trip success — a protocol slice for program Block U.
- **Ladder rung:** solve→repeatable→skill — a reusable protocol-client skill (rung 3).
- **Acceptance criteria:** bastion connects to an MCP example server over at least one transport and
  lists/invokes a tool; the vendored client compiles in bastion; a transport round-trip test passes
  (mock or example server); gated checks pass (`cargo fmt --check`, `cargo clippy -- -D warnings`,
  `cargo test`, `cargo build --release`).

### BA.9.B — Seed the Rust local-model node from the `claude-sdk-rs` spine *(program Block F)*
- **What:** Vendor two patterns from `claude-sdk-rs` into a small Rust **local-model runner** in
  bastion: the subprocess→typed→streaming spine (→ a local/open-weight node driving Ollama /
  `llama-cli`) and the embedded SQLite session store (→ local-first `bastion ask` conversation memory,
  no Postgres). Reuse the `C001–C014` error model already vendored in Phase 7 Block A (`src/observ/errors.rs`)
  rather than re-vendoring it. Give `bastion ask` a Python-free local-model path, selected by flag.
- **Why:** Program Wave 4 — local inference via a CLI binary is on the **Rust** side of the seam (D24):
  the Console gets an offline brain for quick summarization / commit messages / cost estimates without
  round-tripping through Celery. Ties to brain decision **D23** (Ollama-on-M2).
- **Files:**
  - *New* `src/sessions/local_model.rs` (subprocess→typed→streaming spine driving Ollama / `llama-cli`; pure command/arg construction + typed parse split from the spawn, per Rule 6)
  - *New* `src/sessions/memory.rs` (embedded SQLite conversation store — schema + pure query/serialization helpers over a thin `rusqlite` shell)
  - *Modified* `src/sessions/ask.rs` (route to `local_model` when a local-model flag/config is set — additive), `src/cli.rs` (`ask` flags selecting the local model + model name), `Cargo.toml` (`rusqlite` + any local-model deps)
- **Interfaces / shared surface:** Consumes brain decision **D23**'s local-model strategy (Ollama /
  `llama-cli` CLI as the driven process) and **reuses** Phase 7 Block A's `C0xx` error model. Produces a
  Rust local-model node + local SQLite conversation store for `bastion ask`.
- **Out of scope:** The Python open-weight node (program/orchestrator Project H / D19 / D23 Python side)
  — stays Python; this is **not** a competing local-inference stack (D24 guardrail). A general inference
  service. Option D's compile-to-Rust runtime. **Replacing** the existing Claude-Code-session path in
  `bastion ask` (the local path is additive, flag-selected). Re-vendoring the error taxonomy (reuse
  Phase 7 Block A). Source repo read-only.
- **Ratchet:** the Rust local-model runner + local SQLite conversation store
  (`src/sessions/local_model.rs` + `memory.rs`) — a Python-free offline brain for the Console.
- **Eval slice:** local-model answer quality vs. the cloud path (a small parity slice) — a program
  Block U domain.
- **Ladder rung:** solve→repeatable→skill — a reusable local-inference skill (rung 3).
- **Acceptance criteria:** `bastion ask` answers a one-turn prompt against a local model with **no**
  Python process involved; conversation history persists in local SQLite across turns; per Rule 6 the
  pure logic (command/arg construction, typed parse, SQLite query/serialization) is exhaustively
  unit-tested and the spawn/poll I/O shell smoke-tested + recorded in `tasks.md §Notes`; the `sessions/`
  surface's DB-free (D4 — the Postgres pool) and synchronous (D5) invariants are preserved (the SQLite
  store is local sessions memory, **not** the orchestrator Postgres); gated checks pass (`cargo fmt
  --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`).

---

## Phase 10 — Self-healing loop (program Wave 5)

The synthesis: the Brain finds its own problems and *triggers* fixes for human review — the
`sdlc-flow` pattern run **proactively** instead of spec-driven. Per D25, bastion **detects and
triggers**; the Factory **authors the PR**; a human **reviews the draft**; nothing auto-merges.
Forward-looking — refine Files when each becomes next.

### BA.10.A — Proactive scanner → issue backlog *(program Block M)*
- **What:** A scheduled `bastion doctor` (or equivalent) that runs the available scans — Phase 8 Block A
  Brain integrity, plus cross-repo health (test/lint/build status, doc staleness) — and writes findings
  to a **persistent OKF issue backlog** with dedup, priority, and dismiss/defer semantics (so the same
  finding isn't re-filed every run).
- **Why:** Program Wave 5. `sdlc-flow` is reactive (spec-in → PR-out); there is no proactive detector
  today. The backlog is the missing front half of self-healing and is what stops duplicate work and
  lets a human triage.
- **Files:**
  - *New* `src/doctor/mod.rs` (scan orchestration + `doctor` subcommand), `src/doctor/backlog.rs` (OKF issue-backlog writer: dedup, priority, dismiss/defer — pure record logic over a thin file shell)
  - *Modified* `src/cli.rs` (`doctor` subcommand), `src/main.rs` (dispatch), `Cargo.toml` (if new deps needed)
- **Interfaces / shared surface:** Consumes Phase 8 Block A integrity findings + repo health signals +
  Phase 7 Block A events. Produces the **issue-backlog record** (OKF docs) — the documented input for
  Block B. The backlog is a *trigger artifact*, not a mutation (D25).
- **Out of scope:** Fixing anything (Block B). Scanning external/non-brain repos beyond health status.
  Auto-triage of fuzzy findings (human-triaged in the backlog). **Depends on** Phase 7 Block A
  (events) + Phase 8 Block A (integrity findings).
- **Ratchet:** the proactive scanner + the persistent OKF issue backlog (`src/doctor/`) with
  dedup/priority/dismiss — the standing *detector* half of self-healing (a D30 `improve`-queue feeder).
- **Eval slice:** scanner precision + dedup correctness — a program Block U domain.
- **Ladder rung:** workflow→automation→monitor — a scheduled monitor over corpus + repo health
  (rung 8).
- **Acceptance criteria:** a scheduled run produces a deduped, prioritized backlog; re-running does not
  duplicate open findings; dismissed/deferred findings stay suppressed; per Rule 6 dedup + dismissal
  logic is unit-tested; gated checks pass (`cargo fmt --check`, `cargo clippy -- -D warnings`,
  `cargo test`, `cargo build --release`).

### BA.10.B — Findings → spec → draft PR via `sdlc-flow` *(program Block N, bastion half)*
- **What:** For clear-cut backlog items, generate a machine-authored spec and **trigger `sdlc-flow`**
  (base-template) to take it spec → fix → review → **draft PR for human review**. bastion triggers and
  links the resulting PR back to the backlog item; on landing, the item closes (preventing
  re-proposal). bastion never authors the PR itself or merges (D25).
- **Why:** Program Wave 5 — closes the self-healing loop ("self-improve when it finds issues and create
  PRs to fix things for human review, just like `sdlc-flow`"). Reuses the audited PR-terminating engine
  rather than reimplementing PR creation in bastion.
- **Files:**
  - *New* `src/doctor/dispatch.rs` (findings → spec; trigger `sdlc-flow`; record the backlog↔PR link)
  - *Modified* `src/doctor/mod.rs` (wire the dispatch step), `src/doctor/backlog.rs` (close item on landed PR), `src/cli.rs` (a `doctor --fix` / dispatch flag)
- **Interfaces / shared surface:** Consumes the Block A backlog record. Produces a draft PR via
  `sdlc-flow` + a backlog↔PR link. Uses `sdlc-flow`'s existing review gate / triage / `state.json`
  unchanged; **does not** use `--auto-merge`.
- **Out of scope:** The base-template **findings→spec entry point** + self-healing-PR label convention
  (program Block N base-template half — the Factory-side prerequisite). Auto-merge (human reviews every
  self-healing PR — D25). Fixes outside what `sdlc-flow`'s review gate can verify. Fuzzy/ambiguous
  findings (only clear-cut items are auto-specced). **Depends on** Phase 10 Block A.
- **Ratchet:** the findings→spec→`sdlc-flow` dispatch (`src/doctor/dispatch.rs`) + the backlog↔PR link —
  the closing *fixer* half of the self-heal loop, reusing the audited PR engine (not reimplementing it).
- **Eval slice:** self-healing-PR success rate (review-gate-passing draft PRs from clear-cut findings) —
  a program Block U domain, gated by the loop.
- **Ladder rung:** automation→trust — automated fix *proposal* under mandatory human review (rung 7;
  earned auto-proceed for low-risk classes comes via Phase 10 Block D, never here).
- **Acceptance criteria:** a seeded clear-cut finding produces a draft PR through `sdlc-flow` with the
  review gate passing in the target repo; the PR is labeled self-healing and linked to its backlog item;
  a landed fix closes the item so it is not re-proposed; bastion performs no direct merge (D25 upheld);
  per Rule 6 the pure findings→spec + link logic is unit-tested; gated checks pass (`cargo fmt --check`,
  `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`).

### BA.10.C — Incident & recovery harness: postmortem generation + incident records *(program Block Y, bastion half)* *(new — north-star)*
- **What:** Build the genuinely-new piece of the **Incident & Recovery harness** (program Harness HL5):
  structured **incident records** (severity · timeline · impacted goals/tasks · root cause · remediation ·
  preventative improvement) + **automated postmortem generation**, built on the Phase 7 Block A error
  spine and the Phase 10 Block A scanner. Closes the loop: detection → incident → postmortem →
  preventative backlog item → fix (Block B).
- **Why:** Program Wave 5. north-star Layer K + §"Incident and recovery harness" — the one harness with
  **no current substrate**. Severity/timeline/postmortem discipline turns failures into guardrails (the
  north-star *failure loop*: a failure that recurs should be much harder to repeat undetected).
- **Files:**
  - *New* `src/doctor/incident.rs` (pure incident-record construction over `observ` events + Block A
    findings; postmortem-markdown generation), `src/doctor/fixtures/` (seeded failure fixtures)
  - *Modified* `src/doctor/mod.rs` (wire incident detection into the scan), `src/doctor/backlog.rs`
    (emit a preventative backlog item per incident), `src/cli.rs` (a `doctor --incidents` / report surface)
- **Interfaces / shared surface:** Consumes Phase 7 Block A structured events (`C0xx`) + Phase 10 Block A
  findings. Produces incident OKF records + postmortems feeding the D30 `improve` queue and Block B. The
  incident record is HL5's structured artifact.
- **Out of scope:** External paging/alerting integrations (PagerDuty etc.). Auto-remediation (human
  reviews every fix — D25). The base-template preventative-fix path (that's Block B / program Block N).
  **Depends on** Phase 7 Block A + Phase 10 Block A.
- **Ratchet:** the Incident & Recovery harness (program HL5) — structured incident records + automated
  postmortem generation (`src/doctor/incident.rs`); turns failures into guardrails (the failure loop).
- **Eval slice:** incident-harness domain (record completeness, postmortem quality, repeat-failure
  detection) — a program Block U domain.
- **Ladder rung:** harness→monitor — builds the one harness with no current substrate, wired into the
  incident loop (rung 5→8).
- **Acceptance criteria:** a seeded failure produces an incident record with severity/timeline/root-cause
  + a generated postmortem + a preventative backlog item linked into Block A; per Rule 6 the pure
  incident-construction + postmortem-render logic is unit-tested against fixtures; gated checks pass
  (`cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`).

### BA.10.D — Autonomy / trust ladder: trust registry + earned promotion *(program Block X, bastion half)* *(new — north-star)*
- **What:** The Console half of the **autonomy/trust ladder**: a **per-skill / per-domain trust
  registry** (supervised → guided → autonomous → trusted) with promotion *and demotion* rules **earned
  from measured outcomes** (eval pass-rate, intervention rate, regression history — from the orchestrator
  eval engine, program Block U), never hand-declared. The registry is the gate the cost/kill action
  (Phase 7 Block C), the scanner (Block A), and the self-healing trigger (Block B) consult before acting.
  Destructive / security / deploy / trust-threshold changes stay **deny-first** and always require human
  approval (D25).
- **Why:** Program Wave 5. north-star Layer H + the trust loop — "per-skill trust is better than one
  global autonomy switch." Today D25 is binary at the seam (trigger vs perform); this adds the graduated,
  outcome-driven ladder so a proven-safe low-risk action class can proceed on its own, *safely*.
- **Files:**
  - *New* `src/trust/mod.rs` (registry entry/lookup), `src/trust/registry.rs` (pure trust-level model +
    promotion/demotion rules over recorded outcomes), `src/trust/gate.rs` (pure "may this action proceed
    at this level?" decision)
  - *Modified* `src/cli.rs` (`bastion trust` inspect surface), `src/run/kill.rs` + `src/doctor/dispatch.rs`
    (consult the gate before acting), `src/config.rs` (registry persistence path)
- **Interfaces / shared surface:** Consumes the orchestrator eval-engine outcome signals (program Block U
  — the only legitimate promotion source) and Phase 7 Block A events. Produces a **trust-level field** the
  gated actions read. May add a trust-level field to the D20 contract (bump per protocol) if the Engine
  enforces it server-side.
- **Out of scope:** The orchestrator's dispatch-side enforcement + the eval engine itself (program Block X
  orchestrator half + Block U — separate repo; this is the Console registry + gate, independently
  shippable as a read/inspect surface). Auto-trusting destructive/security/deploy actions (always
  human-approved). A human-RBAC/identity system (Phase 11 serve auth handles operator identity).
- **Depends on:** program Block U (eval outcomes, orchestrator) for live promotion; Phase 7 Block C (the
  first real gated action to attach a level to); D25.
- **Ratchet:** the per-skill / per-domain trust registry + earned promotion/demotion policy
  (`src/trust/`) — the governance gate the cost/kill (Phase 7 Block C), scanner (Block A), and self-heal
  (Block B) actions consult before acting.
- **Eval slice:** promotion correctness (outcomes promote; regressions demote; high-risk stays
  deny-first) — a policy/safety slice for program Block U.
- **Ladder rung:** trust — converts measured program-Block-U outcomes into earned, leveled autonomy
  (rung 9), never hand-declared.
- **Acceptance criteria:** a skill/domain accrues a trust level from recorded outcomes; crossing a
  threshold changes gate behavior (e.g. guided→autonomous lets a specific low-risk action proceed without
  prompt) in a test; a regression *demotes* the level in a test; high-risk tiers still require approval
  regardless of level; per Rule 6 the pure registry/gate logic is exhaustively unit-tested; gated checks
  pass (`cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`).

---

## Phase 11 — BastionUI Console API (`bastion serve`) *(independent track; brain D28)*

> **A new, fully independent track** — bastion's server-side slice of the **BastionUI** cross-repo
> program (brain `planning/bastion-ui/master-plan.md`, governed by **D28**, upholding **D21**/**D25**).
> It grows a *network face* on the Console: `bastion serve`, an actix-web HTTP+WebSocket API that
> projects what the CLI already does (tmux session control, repo/workflow status, quick-actions) onto
> the Tailscale tailnet, so the Flutter `bastion-ui` app can operate the stack from a phone. **This
> track neither blocks nor is blocked by Phases 0–10** — it touches only a new `src/serve/` module plus
> three additive seams (`cli.rs` arm, `main.rs` dispatch, `Cargo.toml` deps), and **runs in parallel
> with the current Phase 7 work** (Block B — tiktoken `costs`). It reuses existing `pub` substrate:
> `sessions::tmux`/`model` (D21), `config::load_workspace_registry`, `observ` (C0xx errors), and `ask`.
> The load-bearing output is the **`docs/serve-api.md` contract** (D20-style: this repo produces +
> versions it; `bastion-ui` pins it). Blocks A–F are v0.1; G–I are forward-looking (post-v1).
>
> **Verified source facts** (read directly, 2026-06-26): `main.rs:238` is `#[tokio::main]` (`tokio`
> "full"); tmux fns `list_sessions_raw`/`capture_pane_raw`/`new_session`/`kill_session`/`send_keys`
> are `pub` (`tmux.rs:187–219`), `capture-pane -p` is ANSI-stripped, `send_keys` uses `-l` and
> **cannot** send named keys (Escape/arrows/bare-Enter); `Session`/`SessionState`/`Pane` derive only
> `Debug, Clone` (need serde DTOs); `config::load_workspace_registry` is DB-free (`config.rs:114`);
> `ask::wait_for_claude` is **private** (`ask.rs:240`); `actix-web`/`actix` are **not yet** in
> `Cargo.toml`. The WS actor skeleton is harvested from `rag-engine-rs/src/services/chat/` (actix 0.13
> / actix-web 4.9 / actix-web-actors 4.3).

### BA.11.A — `serve` scaffold + serve-api contract v0 *(verification gate)* *(prog. A)*
- **What:** New `src/serve/` module + a `Commands::Serve { addr, token }` arm; add actix deps
  (pinned to rag-engine-rs versions for copy-compatibility); `GET /health`; a **minimal `/ws` upgrade
  that accepts + echoes** (so the Flutter foundation has a live socket before the real hub exists);
  **mandatory** bearer-token middleware + tailnet bind (`BASTION_SERVE_ADDR` default `0.0.0.0:4317`,
  `BASTION_SERVE_TOKEN`); and the first cut of `docs/serve-api.md` (v0: health, auth, `/ws`, frame
  skeleton). **Runtime spike (the one real integration risk):** the harvested server runs under
  `#[actix_web::main]` and `actix-web-actors` WS actors need an actix `System`/Arbiter in scope, which
  bastion's plain `#[tokio::main]` runtime does not provide — **start from running actix on its own
  thread** (`actix_web::rt::System::new().block_on(serve::run(...))`), treating "it just works inside
  the existing tokio runtime" as the thing to disprove. Settle this before any endpoint work.
- **Why:** Nothing else can be built or pinned until the server boots and the contract exists; this is
  the foundational producer the Flutter Surface pins against.
- **Files:**
  - *New* `src/serve/mod.rs` (actix `HttpServer` bootstrap, routing, runtime integration), `src/serve/auth.rs` (bearer middleware + tailnet bind), `src/serve/dto.rs` (serde DTOs + frame envelope), `src/serve/ws/echo.rs` (minimal accept+echo)
  - *Modified* `src/cli.rs` (`Commands::Serve` arm), `src/main.rs` (dispatch), `src/config.rs` (`BASTION_SERVE_ADDR`/`BASTION_SERVE_TOKEN`, DB-free path), `Cargo.toml` (`actix`, `actix-web`, `actix-web-actors`, `futures`)
  - *New* `docs/serve-api.md` (v0)
- **Interfaces / shared surface:** **Produces** `docs/serve-api.md` v0 — the contract every later serve
  block and every `bastion-ui` block reads/extends. Reuses `observ` for error mapping.
- **Out of scope:** any session/status/action endpoints (later blocks); the real WS hub (Block C); all
  Flutter work (lives in `bastion-ui`).
- **Ratchet:** the `bastion serve` scaffold + the versioned `docs/serve-api.md` contract — the
  load-bearing producer every later serve block and every `bastion-ui` block pins against.
- **Eval slice:** n/a — deterministic acceptance only (boot + health + `/ws` echo + auth; runtime-spike
  outcome documented).
- **Ladder rung:** solve→repeatable→skill — stands up the reusable network-face skill for the Console
  (rung 3).
- **Acceptance criteria:** `bastion serve` boots, serves `GET /health` + a `/ws` echo over a tailnet
  bind with mandatory bearer middleware; the runtime-spike outcome is documented; `docs/serve-api.md`
  v0 committed; gated checks pass (`cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`,
  `cargo build --release`).

### BA.11.B — Session REST + named-key helper *(prog. D)*
- **What:** `GET /sessions`, `GET /sessions/{n}/pane?lines=N`, `POST /sessions/{n}/send`,
  `POST /sessions/{n}/key` (named keys), `POST /sessions`, `DELETE /sessions/{n}` — wrapping
  `sessions::tmux`/`model` via `web::block` (the tmux fns are synchronous blocking). Add
  `tmux::send_named_key`/`send_named_keys` (`send-keys <KeyName>`, no `-l`) with pure element-wise
  `*_args` tests (mirrors `send_keys_args`) — closes the verified gap that `send_keys` can't send
  Escape/bare-Enter/arrows. Map tmux degradation to clean HTTP statuses via `observ`. Extend
  `serve-api.md` to v0.1.
- **Why:** Session control is the first pillar (the daily-friction win); REST is the simplest correct
  surface and the named-key helper is required for approve buttons (Esc) + menu navigation.
- **Files:**
  - *New* `src/serve/handlers/sessions.rs` (REST handlers), `src/serve/dto.rs` additions (`SessionDto`, `PaneDto`)
  - *Modified* `src/sessions/tmux.rs` (add `send_named_key`/`send_named_keys` + `*_args` tests), `src/serve/mod.rs` (route wiring), `docs/serve-api.md` (→ v0.1)
- **Interfaces / shared surface:** **Produces** the session routes in `serve-api.md` v0.1. Consumes the
  `pub` tmux fns (D21).
- **Out of scope:** live streaming + needs-input detection (Block C); any Flutter UI.
- **Depends on:** Block A.
- **Ratchet:** the session REST surface (`src/serve/handlers/sessions.rs`) + the reusable
  `send_named_key`/`send_named_keys` helper (closes the verified Esc/arrows/bare-Enter gap) —
  serve-api.md v0.1.
- **Eval slice:** n/a — deterministic acceptance only (`*_args` + DTO-serde unit tests; live `curl`
  smoke).
- **Ladder rung:** skill→workflow — the remote session-control capability (rung 4).
- **Acceptance criteria:** `curl` against a live server lists sessions, reads a pane, sends keys, sends
  `Escape`, creates and kills a session; per Rule 6 the `*_args` + DTO-serde logic is unit-tested;
  gated checks pass.

### BA.11.C0 — Agent-state detection manifest engine *(prework for Block C; agent-agnostic seam)*
- **What:** A pure, config-driven agent-state detection engine, **reimplemented clean-room** (not
  copied) from Herdr's `src/detect/` pattern. Per-agent TOML manifests (`region` selector +
  `contains`/`regex`/`line_regex` matchers + `any`/`all`/`not` gate combinators + `priority` +
  `visible_idle`/`visible_blocker`/`visible_working`/`skip_state_update` flags) compile into rules; a
  pure `detect(screen, manifest) -> AgentDetection { state: Idle|Working|Blocked|Unknown, visible_*,
  skip_state_update }` matcher evaluates rules in priority order over an extracted screen region.
  **Seed with `manifests/claude.toml` + `manifests/pi.toml` only.** New deps: none (`regex`, `toml`,
  `serde` already in tree).
- **Why:** Block C's needs-input detector is otherwise a Claude-coupled heuristic that drifts with
  Claude's TUI (the block's own "highest-risk component" note). A manifest engine makes detection
  **data-driven and agent-agnostic by construction** — a Claude layout change is a one-manifest edit,
  and adding Pi (or any agent) is a new TOML, not new Rust. This is the **"minimal seam now"** step
  toward the agent-agnostic direction (decision D-y) without the full launch registry/`--agent` port,
  and it fills a real gap: Bastion has *no* agent-state detection today (`sessions/claude_state.rs` is
  a workspace-*trust* observer only). **Reimplement clean — Herdr is AGPL-3.0, reference only (D-x).**
- **Files:**
  - *New* `src/detect/mod.rs` (public API + `AgentState` / `AgentDetection`), `src/detect/manifest.rs`
    (TOML schema + compile + recursive gate matcher + `region()` resolver), `src/detect/manifests/claude.toml`,
    `src/detect/manifests/pi.toml`, fixtures under `src/detect/fixtures/`
- **Interfaces / shared surface:** **Produces** `detect::detect(screen, agent) -> AgentDetection`,
  consumed by **Block C's** needs-input detect, the future **unified-console sidebar** state chips
  (working/blocked/idle/done), and the later **`Agent` trait / `--agent` seam**.
- **Out of scope:** Per-agent launch + registry + `--agent` flag (the later Agent-trait seam — see
  the brain research note's prioritized Block 5); the broad 18-agent manifest set (Claude + Pi only);
  remote/over-the-air manifest updates.
- **Depends on:** nothing new (Block A scaffolding exists).
- **Ratchet:** the pure manifest engine + Claude/Pi manifests; gate matcher + every `region` resolver
  fixture-tested; adding a manifest requires zero engine-code change.
- **Eval slice:** agent-state detection precision/recall over captured-pane fixtures (Claude + Pi) —
  the self-contained detector eval that Block C's needs-input detection reuses (program Block U domain).
- **Ladder rung:** skill→workflow — a reusable, agent-neutral detection primitive.
- **Acceptance criteria:** a captured Claude prompt-box fixture → `detect()` returns `Blocked` +
  `visible_blocker`; a Pi `Working...` fixture → `Working`; the `any`/`all`/`not` combinators and each
  `region` selector are unit-tested; a new agent manifest is added with no engine change; per Rule 6
  the pure engine is exhaustively tested; gated checks pass.

### BA.11.C — WebSocket hub + live pane + "needs input" detection *(prog. E)*
- **What:** Adapt the `rag-engine-rs` `ChatServer`/`ChatSession` actors into `src/serve/ws/`;
  topic-based subscriptions (`sessions`, `pane:<name>`); background poll tasks → `watch` channels →
  fan-out (one session-list poll ~2s; ref-counted per-session pane polls only while subscribed →
  diff-and-push). The pure `detect.rs` needs-input flag is **driven by Block C₀'s manifest engine**
  (`detect::detect(pane, manifest).state == Blocked && visible_blocker`) rather than inline literals →
  emits `event{needs_input}`. Extend `serve-api.md` to v0.2 (subscribe/unsubscribe/send/send_key +
  sessions/pane/event/error frames).
- **Why:** Live pane + the needs-input event are what make the phone genuinely useful (watch + alert +
  unblock), not just a polling viewer. The detect heuristic + approve-key mappings are still coupled to
  agent TUI layout, but **Block C₀ moves that coupling into TOML manifests** — a layout drift is a
  one-manifest edit, and the same engine powers Pi and every future agent, not just Claude.
- **Files:**
  - *New* `src/serve/ws/server.rs` (hub actor, adapted from `ChatServer`), `src/serve/ws/session.rs` (per-conn actor, adapted from `ChatSession`), `src/serve/poll.rs` (poll tasks → watch → fan-out), `src/serve/status/detect.rs` (thin needs-input adapter that calls `detect::detect()` from Block C₀ + maps `Blocked`→`needs_input`; fixtures)
  - *Modified* `src/serve/dto.rs` (frame union), `src/serve/mod.rs` (`/ws` upgrade → real hub), `docs/serve-api.md` (→ v0.2)
- **Interfaces / shared surface:** **Consumes** Block C₀'s `detect::detect()`. **Produces** the WebSocket frame schema in `serve-api.md` v0.2.
- **Out of scope:** status/workflow topics (Block D); Flutter rendering; the detection engine itself (Block C₀).
- **Depends on:** Block C₀ (agent-state detection engine), Block A (WS scaffold), Block B (session ops reused by send/send_key frames).
- **Reference — study before designing the WS layer:** `~/Dev/agentic-portfolio/Healthie/media_streams/` is a production-proven Ruby WebSocket service (Zoom RTMS transcript capture) with clean architecture directly analogous to what this block needs. Study it before writing the hub. Key patterns to port to Rust/Tokio:
  - **Dual-connection split** (`SignalingConnection` + `TranscriptConnection`) — one socket for control/keep-alive, one for data; the data URL is negotiated through the control channel. Maps to our control/pane-stream topic split.
  - **Thread-safe future coordination** (`Concurrent::IVar`) — WebSocket callback thread sets a future; main thread blocks with a timeout waiting for the negotiated URL. Rust equivalent: `tokio::sync::oneshot`. Same shape for "wait until the hub confirms subscription before streaming."
  - **Keep-alive checker with pluggable failure handler** (`KeepAliveChecker` + `FailKeepAliveChecker`) — a timer monitors last-seen timestamp; on timeout delegates to a failure handler. Direct analogue for detecting dead Flutter connections and cleaning up poll tasks.
  - **Message type dispatch** (`ZoomRtmsMessageTypes` + `react_to_message` per connection class) — clean enum-keyed dispatch that maps directly to our frame union in `dto.rs`.
  - **Runner lifecycle** (`runner.rb`) — `Success`/`Failure` result types, ensure-block cleanup, retry-on-exception vs. no-retry-on-known-failure distinction. Mirrors what `ws/server.rs` needs for per-connection lifecycle.
- **Ratchet:** the WebSocket hub + the needs-input event driven by Block C₀'s manifest engine (a
  Claude-TUI layout drift is a one-*manifest* edit; Pi and future agents come free) — serve-api.md v0.2.
- **Eval slice:** needs-input detection precision/recall over captured-pane fixtures — a self-contained
  detector eval (a program Block U candidate domain).
- **Ladder rung:** skill→workflow→monitor — live pane streaming + the killer needs-input alert (rung
  toward monitor).
- **Acceptance criteria:** `websocat` subscribes to a pane and receives live `pane` pushes; sending keys
  + `Escape` over the socket lands in the session; a session on a permission prompt produces
  `event{needs_input}`; per Rule 6 the diff/seq + detect logic is unit-tested (the actor/poll I/O shell
  smoke-tested + recorded); gated checks pass.

### BA.11.D — Repo + workflow status reads *(prog. G)*
- **What:** `GET /repos` (enumerate `config::load_workspace_registry` roots → name, current-focus line,
  status-table snapshot, has-handoff flag), `GET /repos/{name}/status`, `GET /repos/{name}/handoff`
  (raw markdown), `GET /repos/{name}/workflows` (glob `planning/*/sdlc/sdlc-flow-state.json`, parse).
  Pure parsers for `status.md` / `handoff.md` / `sdlc-flow-state.json`; `poll.rs` watches the
  flow-state files and emits `event{workflow_done}` on a `running→done|blocked` transition. Optional
  `gh pr` status via process call (degrade gracefully if absent). Extend `serve-api.md` to v0.3.
- **Why:** Answers "lots of moving parts, where are we"; read-only so lower risk; reuses the registry
  multi-workspace Brain already maintains. Completion is detected from committed git state, not pane
  scraping.
- **Files:**
  - *New* `src/serve/status/repo.rs` (pure `status.md` parser), `src/serve/status/handoff.rs` (reader), `src/serve/status/flow.rs` (pure `sdlc-flow-state.json` parser), `src/serve/handlers/status.rs` (REST handlers)
  - *Modified* `src/serve/poll.rs` (watch flow-state files → `workflow_done`), `src/serve/dto.rs` (`RepoStatusDto`, `WorkflowStateDto`), `docs/serve-api.md` (→ v0.3)
- **Interfaces / shared surface:** Consumes the workspace registry + each repo's `planning/` file
  conventions. **Produces** status routes + events in `serve-api.md` v0.3.
- **Out of scope:** Engine/orchestrator run state from Postgres (Block G); Flutter rendering.
- **Depends on:** Block A; the topic/poll plumbing from Block C.
- **Ratchet:** the pure `status.md` / `handoff.md` / `sdlc-flow-state.json` parsers + the status routes
  — serve-api.md v0.3; the `status.md` parser is shared with Phase 7 Block D's CLI surface.
- **Eval slice:** n/a — deterministic acceptance only (parser fixtures + a simulated flow-state
  transition).
- **Ladder rung:** skill→workflow→monitor — "where does everything stand" surfaced + the
  `workflow_done` event (rung toward monitor).
- **Acceptance criteria:** `curl /repos` returns every registry repo with its focus line; a simulated
  `sdlc-flow-state.json` transition produces a `workflow_done` event over the socket; per Rule 6 the
  parser logic is fixture-tested; gated checks pass.

### BA.11.E — Quick-action command endpoint (inject / spawn) *(prog. I)*
- **What:** `POST /actions/command {mode, session?, name?, dir?, model?, command}` — `mode:"inject"`
  sends the command into a chosen session; `mode:"spawn"` creates a session, launches
  `claude --model <opus|sonnet> --permission-mode bypassPermissions`, waits for readiness (reuse the
  `ask` readiness mechanics — `ask::wait_for_claude` is **private**, so make it `pub(crate)` or reuse
  the public `ask()` entry), then sends the command. Returns the target session id. Extend
  `serve-api.md` to v0.4.
- **Why:** Turns frequent slash-commands into one-tap remote triggers in both modes Brandon uses.
- **Files:**
  - *New* `src/serve/handlers/actions.rs`
  - *Modified* `src/sessions/ask.rs` (`wait_for_claude` → `pub(crate)` if reused), `src/serve/dto.rs` (`CommandRequest`/response), `src/serve/mod.rs` (route), `docs/serve-api.md` (→ v0.4)
- **Interfaces / shared surface:** **Produces** the actions route in `serve-api.md` v0.4. Reuses session
  ops (Block B) + `bastion ask` spawn mechanics.
- **Out of scope:** the command list itself (app-side config in `bastion-ui`); Engine workflow triggers
  (Block G).
- **Depends on:** Block B, Block C.
- **Ratchet:** the quick-action inject/spawn endpoint (`src/serve/handlers/actions.rs`) — serve-api.md
  v0.4; turns frequent slash-commands into one-tap remote triggers in both modes.
- **Eval slice:** n/a — deterministic acceptance only (request-parse/dispatch unit tests; spawn I/O
  smoke).
- **Ladder rung:** skill→workflow — remote one-tap action triggers (rung 4).
- **Acceptance criteria:** an inject call lands a command in a named session; a spawn call creates a
  session with the chosen model and returns its id; per Rule 6 the request-parse/dispatch logic is
  unit-tested (spawn I/O smoke-tested + recorded); gated checks pass.

### BA.11.F — Auth hardening, contract freeze, docs *(prog. K)*
- **What:** Finalize the **mandatory** bearer middleware across HTTP + `/ws` upgrade; confirm
  tailnet-only bind; add a lightweight audit log of mutating actions (send/spawn/kill) via `observ`;
  `bastion serve` man-page entry + README/help; route all errors through `observ`; **freeze
  `docs/serve-api.md` at v1.0.0** and record a bastion-local serve-api decision in
  `planning/decisions/`.
- **Why:** A server that injects keystrokes and spawns Claude with `bypassPermissions` must be locked
  down before daily use; freezing the contract stabilizes the Surface.
- **Files:**
  - *New* `planning/decisions/<Dn>-serve-api-contract.md` (bastion-local serve-api decision), man-page/README serve entry
  - *Modified* `src/serve/auth.rs` (refuse-without-token), `src/serve/mod.rs` (audit-log hook), `docs/serve-api.md` (→ v1.0.0 frozen)
- **Interfaces / shared surface:** **Produces** `serve-api.md` v1.0.0 (frozen) + the bastion-local
  decision.
- **Out of scope:** any new endpoints; Flutter polish.
- **Depends on:** Blocks B, C, D, E.
- **Ratchet:** the frozen `serve-api.md` v1.0.0 contract + the mutating-action audit log + the
  bastion-local serve-api decision — the stable Surface contract and the security posture for daily use.
- **Eval slice:** n/a — deterministic acceptance only (refuse-without-token, reject-unauthenticated,
  audit-log tests).
- **Ladder rung:** workflow→package — freezes + packages the v1 Console network API (rung toward
  package).
- **Acceptance criteria:** the server refuses to start (or rejects all requests) without a token set;
  unauthenticated requests are rejected; mutating actions are audit-logged; man page + README updated;
  `serve-api.md` v1.0.0 committed; gated checks pass.

### BA.11.G — Engine workflow surfaces in `serve` *(prog. N; forward-looking, post-v1)*
- **What:** Expose orchestrator workflow trigger (existing `api::client`), run state (`db/` Postgres
  reads), and kill (proxying the orchestrator's **Block I** abort endpoint) over `serve`, using a
  generic "workload" DTO shared with tmux sessions. Because orchestrator D28 incremental persistence
  already ships per-node `task_context`, this can expose **live-ish per-node progress by polling**
  `node_runs` (not just post-hoc reads) — only push-based mid-run DAG *streaming* stays out of scope.
  *Provisional files; firm up when next.*
- **Why:** Brings the Engine into the same mobile gateway. Build the generic workload DTO **here, when
  Engine control arrives** — in Blocks A–F just avoid session DTO choices that would *prevent* it.
- **Files (provisional):** *New* `src/serve/handlers/engine.rs`; *Modified* `src/serve/dto.rs` (generic `WorkloadDto`), `src/api/` + `src/db/` reuse, `docs/serve-api.md` (later version).
- **Interfaces / shared surface:** Consumes the orchestrator API/contract (incl. its Block I abort) +
  Postgres; **produces** Engine-workflow routes in a later `serve-api.md` version.
- **Out of scope:** push-based mid-run DAG streaming; Flutter UI (`bastion-ui` Phase 5).
- **Depends on:** the orchestrator's **Block I** (abort endpoint — its Wave 2); the v1 server (Blocks A–F).
- **Ratchet:** the generic Engine "workload" DTO + Engine-workflow routes over `serve` — brings the
  Engine into the same mobile gateway (consuming the orchestrator's Block I abort).
- **Eval slice:** n/a — deterministic acceptance only (trigger / inspect / kill of a run over serve).
- **Ladder rung:** skill→workflow — extends the gateway to Engine control (rung 4).
- **Acceptance criteria:** trigger + inspect + kill of an orchestrator run work over `serve`; per-node
  state renders; gated checks pass.

### BA.11.H — Chat backend via Claude Code SDK *(prog. P; forward-looking, post-v1)*
- **What:** A `serve` chat endpoint/WS topic backed by the **Claude Code SDK** (already a dependency in
  the orchestrator: `claude-agent-sdk`, `app/services/claude_code/`), reusing the harvested
  actor-streaming pattern. Backend location (bastion vs orchestrator) firmed up when built.
  *Provisional files.*
- **Why:** Lets Brandon think out loud with an assistant while workflows run, without leaving the app.
- **Files (provisional):** *New* `src/serve/ws/chat.rs`; *Modified* `src/serve/dto.rs` (chat frames), `docs/serve-api.md` (later version).
- **Interfaces / shared surface:** Consumes the Claude Code SDK; **produces** a chat topic in a later
  `serve-api.md` version.
- **Out of scope:** Ollama/Anthropic-API-direct backends (rejected); the Flutter chat UI.
- **Depends on:** the v1 WebSocket foundation (Block C).
- **Ratchet:** a `serve` chat topic backed by the Claude Code SDK, reusing the WS actor-streaming
  pattern — think-out-loud with an assistant while workflows run.
- **Eval slice:** chat-response quality — a program Block U candidate domain (overlaps HL1
  general-dynamic).
- **Ladder rung:** skill→workflow — adds a conversational surface to the gateway (rung 4).
- **Acceptance criteria:** a chat turn streams tokens back over the socket from the Claude Code SDK;
  gated checks pass.

### BA.11.I — FCM background push relay *(prog. R server-half; forward-looking, post-v1)*
- **What:** A Firebase Cloud Messaging relay from the Mac Mini so `needs_input` / `workflow_done`
  events reach the phone when the app is closed; the v1 event model was designed so this bolts on
  without rework. *Provisional files.*
- **Why:** True background alerts (the v1 limitation is in-app/foreground-only).
- **Files (provisional):** *New* `src/serve/push.rs` (FCM relay + device registration); *Modified* `src/serve/poll.rs` (fan-out to relay), `docs/serve-api.md` (later version).
- **Interfaces / shared surface:** Consumes the v1 `event{...}` model; **produces** a push relay +
  device-registration path (the `bastion-ui` FCM client is the peer half).
- **Out of scope:** changing in-app event semantics; pushing payloads beyond minimal metadata.
- **Depends on:** Block C (events), and the shipped v0.1 app.
- **Ratchet:** the FCM background push relay + device-registration path (`src/serve/push.rs`) — true
  background alerts; the `bastion-ui` FCM client is the peer half.
- **Eval slice:** n/a — deterministic acceptance only (closed-app notification delivery).
- **Ladder rung:** workflow→automation→monitor — background event delivery completes the monitor loop
  off-device (rung 8).
- **Acceptance criteria:** a `needs_input` event delivers a phone notification with the app closed;
  gated checks pass.

---

## Quick Reference Sequence Table

| Phase | Block | What | Why | Role in destination |
|---|---|---|---|---|
| 0 | A | Scaffold + `bastion status` | DB/API connection validated | Prerequisite for everything |
| 1 | A | DB queries + graph layout | Data layer before TUI | Enables render loop |
| 1 | B | TUI render loop | Core feature | The primary deliverable |
| 2 | A | `bastion inspect` | Post-mortem graph view | Completes the monitoring story |
| 2 | B | `bastion costs` | LLM spend tracking | Operational awareness |
| 3 | A | `bastion run` | Workflow trigger | Closes the control loop |
| 3 | B | `bastion validate` | Content validation | Unifies the Rust tool surface |
| 4 | — | Polish | SSE, re-run, config, man page | Production-quality tooling |
| 5 | A | `bastion sessions` + tmux wrapper + lazy DB | First session verb; foundations | Process-control surface (ungated) |
| 5 | B | `attach` / `new` / `kill` | Session lifecycle | Operate sessions from any device |
| 5 | C | `bastion send` | Send keystrokes | Act without attaching |
| 5 | D | `bastion capture` | Read pane output | Observe without attaching |
| 5 | E | Session TUI view | Ergonomic operator surface | Pleasant from a phone |
| 5 | F | Activity indicator + trust observer | Honest session state (running vs idle); pre-flight the Claude trust prompt | Trustworthy at-a-glance from a phone |
| 5 | G | `bastion ask` (one Claude Code turn) | Stable command for the orchestrator's `CLAUDE_CODE_SESSION` LLM provider | Subscription-billed, observable LLM nodes |
| 6 | A | Vendor `knowledge_graph`; structural query over the OKF `[[link]]` graph *(prog. A, Wave 1)* | Adds structural retrieval (deps / blast-radius / lineage) | The Brain answers "what is connected", not just "what is similar" |
| 6 | B | Multi-workspace Brain — graph reader over per-repo/per-client roots *(prog. C½, Wave 1)* | Brain as a capability over any OKF workspace, not one repo | Groundwork for code corpora, memory, loop-proof |
| 6 | C | Structural code navigation (code-as-graph) *(prog. Q, Wave 1)* | Exact def/refs/dependents over source, model-free | "Ask my own system how my own code is wired" |
| 7 | A | Tracing + `C0xx` structured-error spine *(prog. H, Wave 2)* | You can't cap/alert/self-heal what you can't see | The observability foundation for the whole track |
| 7 | B | Vendor tiktoken counter → exact `bastion costs` *(prog. D, Wave 2)* | Exact > estimated, as a library for free | Console reports exact, not estimated, spend |
| 7 | C | Cost as a budgeted resource: `--watch`, alerts, `bastion kill`, gate *(prog. I½, Wave 2)* | Observe spend → cap spend → stop spend (D25 trigger) | Operator control over runaway cost |
| 7 | D | Console reads momentum & metrics *(prog. V, Wave 2 ✲; new)* | "What's in flight / blocked / where's momentum" across the registry (D30 scalars, read-only) | The portfolio dashboard surface |
| 8 | A | Deterministic Brain-integrity validation *(prog. K, Wave 3)* | A hard correctness guarantee vs. soft prompt grounding | The real anti-hallucination layer; feeds self-healing |
| 9 | A | Vendor `workflow-engine-mcp` → Console MCP / tool client *(prog. E½, Wave 4)* | Protocol seam (Layer 3 client) for the Brain-MCP server | Console speaks MCP across HTTP/WS/stdio |
| 9 | B | Rust local-model node from the `claude-sdk-rs` spine *(prog. F, Wave 4)* | Python-free local inference for `bastion ask` (D23) | The Console gets its own offline brain |
| 10 | A | Proactive scanner → issue backlog *(prog. M, Wave 5)* | The missing proactive front-half of self-healing | The Brain finds its own problems |
| 10 | B | Findings → spec → draft PR via `sdlc-flow` (no auto-merge) *(prog. N½, Wave 5)* | Triggers fixes for human review; reuses the audited engine (D25) | The Brain proposes its own fixes |
| 10 | C | Incident & recovery harness — postmortem gen + records *(prog. Y½, Wave 5; new)* | Turns failures into guardrails (the failure loop); the one harness with no substrate | The Brain learns from its own incidents |
| 10 | D | Autonomy/trust ladder — trust registry + earned promotion *(prog. X½, Wave 5; new)* | Per-skill trust earned from eval outcomes gates how much self-healing can proceed (D25) | Safely-increasing autonomy |
| 11 | A | `serve` scaffold + serve-api v0 (runtime spike) *(BastionUI; prog. A)* | The gateway + the contract everything pins | `bastion serve` boots; Surface has a socket |
| 11 | B | Session REST + named-key helper *(BastionUI; prog. D)* | Session control server-side; Esc/arrows now sendable | Pillar 1 server half |
| 11 | C | WebSocket hub + live pane + needs-input *(BastionUI; prog. E)* | Live streaming + the killer phone alert | Pillar 1 realtime |
| 11 | D | Repo + workflow status reads *(BastionUI; prog. G)* | "Where does everything stand" over the registry | Pillar 2 server half |
| 11 | E | Quick-action endpoint (inject / spawn) *(BastionUI; prog. I)* | One-tap slash-command triggers | Pillar 3 server half |
| 11 | F | Auth hardening + serve-api v1.0.0 freeze + docs *(BastionUI; prog. K)* | Lock down keystroke/spawn API; stabilize contract | v0.1 contract frozen |
| 11 | G | Engine workflow surfaces in `serve` *(BastionUI; prog. N, post-v1)* | Trigger/inspect/kill runs via the gateway (D25) | Engine control server half |
| 11 | H | Chat backend via Claude Code SDK *(BastionUI; prog. P, post-v1)* | Think out loud while building | Chat server half |
| 11 | I | FCM background push relay *(BastionUI; prog. R, post-v1)* | Alerts when the app is closed | Background push server half |

> Phases 0–4 (workflow observability) and Phase 5 (session control) are **independent tracks**.
> Phase 5 has no dependency on the orchestrator and is not gated by D2 — it can be worked at any
> time, including before the monitor track completes.
>
> Phases 6–11 are **bastion's slice of the cross-repo Bastion program** (brain
> `planning/bastion-product/`, governed by brain **D24 / D25 / D26**), sequenced to follow the
> program's **demand-first wave order** (the `Wave N` tag on each row), not bastion-internal
> dependency. Each row notes its **program block letter** (`prog. X`); `½` marks the bastion half of a
> cross-repo block whose orchestrator/base-template peer is a prerequisite for the *combined* claim (the
> bastion half is independently shippable). This whole track is **opportunistic and ungated** — pull
> blocks as the need appears. Within it the only hard local prerequisites are: 6B/6C build on 6A;
> 7C builds on 7A (and is strengthened by 7B); **7D** needs the D30 sections stamped across repos
> (brain HQ-Restructure) + 6B (registry); 8A builds on 6A; 9B reuses 7A's error model;
> 10A builds on 7A + 8A; 10B builds on 10A; **10C** builds on 7A + 10A; **10D** needs program Block U
> (orchestrator eval outcomes) + 7C (the first gated action). The three **new** program blocks bastion
> now owns from the north-star umbrella reorg are **V** (7D), **Y½** (10C), and **X½** (10D). Program
> blocks **B, J, L, O, P, R, S, U, W, G** (of the *bastion-product* program) are **not** here — they
> execute in python-orchestration / base-template / the brain (**U** = eval engine and **W** =
> external-intelligence loop are orchestrator+brain; **G**, the loop-proof, is coordinated from bastion
> but builds no bastion code; its artifact lands in the brain's `docs/content/`).
>
> **Phase 11 (BastionUI Console API) is a fourth, fully independent track** — bastion's server-side
> slice of the separate **BastionUI** cross-repo program (brain `planning/bastion-ui/master-plan.md`,
> **D28**). It neither blocks nor is blocked by Phases 0–10 and **runs in parallel with the current
> Phase 7 work**; it touches only `src/serve/` + three additive seams (`cli.rs`, `main.rs`,
> `Cargo.toml`). Its local order is linear (A→B→C→D/E→F; G/H/I post-v1), and each block produces a
> `docs/serve-api.md` version the Flutter `bastion-ui` repo pins. Its block letters (A–I) and program
> tags (`prog. A/D/E/G/I/K/N/P/R`) belong to the *BastionUI* program and are unrelated to the
> bastion-product letters above. The peer Flutter blocks execute in the `bastion-ui` repo; later
> Engine-kill (11G) consumes the orchestrator's existing **Block I** abort endpoint.

---

*Sequenced by dependency and competence, not calendar. When life gets in the way, pick up
where you left off.*
