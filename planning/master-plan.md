---
type: Plan
title: bastion Master Plan
description: Strategic roadmap and phase specifications for bastion.
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

A third track (**Phases 6–10**) is bastion's slice of the **Bastion program** — the five-layer
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

### Block A — Foundation setup
- **What:** Verify the Rust toolchain compiles the scaffolded project. Implement `bastion status` end-to-end: connect to PostgreSQL and the FastAPI health endpoint, print a summary of what's reachable. Add a `.env.example`.
- **Why:** Proves the DB connection and HTTP client work before any TUI work starts. Useful as a pre-flight check from day one.
- **Build notes:** `config.rs` reads `DATABASE_URL` and `BASTION_API_URL` from env. `run::status()` calls `api::client::ApiClient::health()` and a test PostgreSQL query. Print a formatted table: `DB ✓`, `API ✓` (or `unreachable` per service). Worker count / queue depth live in Redis, which is out of bastion's configured scope — **scoped out** of `status` (see D2).
- **Acceptance criteria:** `cargo build` passes. `cargo test` passes. `bastion status` prints real health data against the running Python orchestrator.

---

## Phase 1 — `bastion monitor`

### Block A — DB queries + graph layout
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

### Block B — TUI render loop
- **What:** Implement `monitor::ui` (two-pane ratatui layout) and `monitor::events` (tokio loop with keyboard + DB poll). Wire through `monitor::app` state. `bastion monitor` (no arg → auto-pick the active run) enters the TUI and displays live workflow state. Detail pane reads, per the [data contract](../docs/data-contract.md) §6: status/timing/error/input/tokens from `node_runs[name]`, output from `nodes[name]`, run input from `events.data`.
- **Why:** The core deliverable.
- **Acceptance criteria:** `bastion monitor` renders a running workflow as a live graph. Arrow-key navigation moves the selected node. State updates within the poll interval (the orchestrator persists at every node boundary, so each transition is observable). `q` exits cleanly.

---

## Phase 2 — Inspect + Costs

### Block A — `bastion inspect`
- **What:** Reuse monitor graph/UI code with polling disabled. Load a completed run by ID from PostgreSQL and render it as a static navigable graph.
- **Acceptance criteria:** `bastion inspect <run-id>` renders any completed run. Navigation works. Exiting returns to the shell cleanly.

### Block B — `bastion costs`
- **What:** Implement `db::costs` aggregation queries. `bastion costs --last 7d` prints a formatted table of workflow names, run counts, token totals, and estimated USD cost.
- **Acceptance criteria:** Output matches manual SQL queries against the same data. Handles `7d`, `30d`, `all` windows.

---

## Phase 3 — Run + Validate

### Block A — `bastion run`
- **What:** Implement `api::client::trigger_workflow`. `bastion run <workflow> [--args '{}'] [--monitor]` issues `POST /` with `{workflow_type, data}` (the orchestrator's generic dispatcher — see [data contract](../docs/data-contract.md) §7), prints the returned `task_id`, optionally drops into `bastion monitor` for that run.
- **Acceptance criteria:** Successfully triggers a workflow. `--monitor` flag works.

### Block B — `bastion validate`
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

### Block A — `bastion sessions` (+ tmux wrapper + lazy DB pool)
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

### Block B — `bastion attach` / `new` / `kill` (session lifecycle)
- **What:** `bastion attach <session>` (exec into `tmux attach -t`), `bastion new <session>
  [--dir PATH]` (`tmux new-session -d -s … -c …`), `bastion kill <session>` (`tmux kill-session`).
- **Why:** Core lifecycle — create, enter, dispose.
- **Acceptance criteria:** Each verb performs the corresponding tmux action against a real server.
  `attach` hands the terminal cleanly to tmux and returns to the shell on detach. `new` honors
  `--dir`. Bad/unknown session names produce clear errors. Command-construction logic is unit
  tested without spawning tmux.

### Block C — `bastion send` (send keystrokes without attaching)
- **What:** `bastion send <session> <cmd>` → `tmux send-keys -t <session> <cmd> Enter`.
- **Why:** Trigger actions in a session from the phone without a full attach.
- **Acceptance criteria:** Keystrokes arrive in the target session. Quoting/escaping of
  multi-word commands is correct and unit tested. Clear error on unknown session.

### Block D — `bastion capture` (read pane output non-interactively)
- **What:** `bastion capture <session> [--lines N]` → `tmux capture-pane -p -t <session>`,
  print the last N lines.
- **Why:** Check what a session is doing without attaching — the read counterpart to `send`.
- **Acceptance criteria:** Prints recent pane output for a session. `--lines` bounds the output.
  Output parsing/trimming is unit tested against fixtures.

### Block E — session view in the TUI
- **What:** A `ratatui` session dashboard (reachable as `bastion` no-arg or `bastion tui`,
  alongside the monitor view): list of sessions with status + last line; `[a]` attach (drop into
  full tmux attach), `[n]` new, `[s]` inline send, `[k]` kill, `[q]` quit.
- **Why:** The ergonomic operator surface that ties the verbs together — the piece that makes it
  pleasant from a phone.
- **Acceptance criteria:** The dashboard lists live sessions and refreshes; selection + the
  documented key actions work; `a` drops into a real tmux attach and returns cleanly; `q` exits.
  Built entirely on the Block A–D primitives.

### Block F — session activity indicator + Claude trust observer
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

### Block G — `bastion ask` (one Claude Code turn for an external caller)
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

## Bastion-program track (Phases 6–10) — orientation

Phases 6–10 are **bastion's execution slice of the cross-repo Bastion program** (brain
`planning/bastion-product/master-plan.md`). That program is wave-ordered **demand-first** (D26) across
five repos and uses global block letters **A–S**; the blocks whose execution home is the Console land
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

> Distant blocks (Phases 8–10) carry the full skeleton but are **forward-looking** — expect their
> Files / interface lines to need refinement when each becomes next.

---

## Phase 6 — Brain & code retrieval (program Wave 1)

Deepen the Brain with **structural** retrieval (the model-free, Console-side twin of the Engine's
semantic retrieval) and extend it from docs to **code**. Per `ownership.md`, code is just another
corpus that is both semantic (Engine) and structural (Console). This phase vendors the
`knowledge_graph` crate and runs its algorithms over graphs derived from the OKF `[[link]]` corpus and,
later, from source.

### Block A — Vendor `knowledge_graph` → structural query over the OKF `[[link]]` graph *(program Block A)*
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
- **Acceptance criteria:** `bastion brain` returns correct dependents/lineage for a known OKF node (e.g.
  D20's dependents match its stated relations); the graph is built from the live brain repo corpus; the
  vendored crate compiles with **no** Dgraph dependency; per CLAUDE.md Rule 6 the pure OKF→graph builder
  and query functions are exhaustively unit-tested against the fixture and the thin file-walk I/O shell
  is smoke-tested + recorded in `tasks.md §Notes`; gated checks pass (`cargo fmt --check`,
  `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`).

### Block B — Multi-workspace Brain (bastion graph reader over per-repo / per-client roots) *(program Block C, bastion half)*
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
- **Acceptance criteria:** the bastion graph reader indexes and answers over a **second**, non-repo OKF
  workspace selected by `--workspace` / config; the default still resolves to the brain repo; a
  portability fixture is covered; gated checks pass (`cargo fmt --check`, `cargo clippy -- -D warnings`,
  `cargo test`, `cargo build --release`).

### Block C — Structural code navigation (code-as-graph) *(program Block Q)*
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

### Block A — Tracing + structured-error spine *(program Block H)*
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
- **Acceptance criteria:** every subcommand emits a structured start/outcome/duration event; errors
  carry a `C0xx` code + context; `--json-logs` produces machine-parseable output; the vendored taxonomy
  compiles in bastion; per Rule 6 event-emission and error-code mapping are unit-tested; gated checks
  pass (`cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`).

### Block B — Vendor `workflow-engine-core` token counter → exact `bastion costs` *(program Block D)*
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
- **Acceptance criteria:** `bastion costs` reports counts that **match** the tiktoken encoders for a
  known input (exact, not estimated); a unit test asserts exact-count parity on a fixed sample
  (element-level); the vendored counter compiles in bastion; gated checks pass (`cargo fmt --check`,
  `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`).

### Block C — Cost as a budgeted resource: `--watch`, alerts, `bastion kill`, pre-dispatch gate *(program Block I, bastion half)*
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
- **Acceptance criteria:** `bastion costs --watch` shows live spend; crossing a threshold emits an alert
  event; `bastion kill <run>` aborts via the orchestrator endpoint and the run reaches terminal state in
  `node_runs`; `bastion run` honors the budget gate; the pure budget/gate logic is unit-tested per Rule
  6 and the contract additions are recorded in `data-contract.md`; gated checks pass (`cargo fmt
  --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`).

---

## Phase 8 — Client-grade Brain integrity (program Wave 3)

Make the Brain *trustworthy*, not just searchable: catch correctness defects deterministically, before
they ever reach an LLM. This is the **hard** anti-hallucination layer (prompt-based grounding is soft),
and the Rust graph code already supports it. Forward-looking — refine Files when it becomes next.

### Block A — Deterministic Brain-integrity validation *(program Block K)*
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
- **Acceptance criteria:** `bastion validate --integrity` reports broken links, orphans, stale refs, and
  structurally-contradictory decisions over the live brain with **zero false positives** on a curated
  fixture; PageRank / community outputs are exposed; the findings record is documented as the Phase 10
  contract; per Rule 6 the pure check logic is unit-tested against fixtures; gated checks pass (`cargo
  fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`).

---

## Phase 9 — Protocol & local inference (program Wave 4)

Give the Console a protocol seam (MCP client) and its own offline brain (a Python-free local-model
path). Forward-looking — refine Files when each becomes next.

### Block A — Vendor `workflow-engine-mcp` → Console MCP / tool client *(program Block E, bastion half)*
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
- **Acceptance criteria:** bastion connects to an MCP example server over at least one transport and
  lists/invokes a tool; the vendored client compiles in bastion; a transport round-trip test passes
  (mock or example server); gated checks pass (`cargo fmt --check`, `cargo clippy -- -D warnings`,
  `cargo test`, `cargo build --release`).

### Block B — Seed the Rust local-model node from the `claude-sdk-rs` spine *(program Block F)*
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

### Block A — Proactive scanner → issue backlog *(program Block M)*
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
- **Acceptance criteria:** a scheduled run produces a deduped, prioritized backlog; re-running does not
  duplicate open findings; dismissed/deferred findings stay suppressed; per Rule 6 dedup + dismissal
  logic is unit-tested; gated checks pass (`cargo fmt --check`, `cargo clippy -- -D warnings`,
  `cargo test`, `cargo build --release`).

### Block B — Findings → spec → draft PR via `sdlc-flow` *(program Block N, bastion half)*
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
- **Acceptance criteria:** a seeded clear-cut finding produces a draft PR through `sdlc-flow` with the
  review gate passing in the target repo; the PR is labeled self-healing and linked to its backlog item;
  a landed fix closes the item so it is not re-proposed; bastion performs no direct merge (D25 upheld);
  per Rule 6 the pure findings→spec + link logic is unit-tested; gated checks pass (`cargo fmt --check`,
  `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`).

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
| 8 | A | Deterministic Brain-integrity validation *(prog. K, Wave 3)* | A hard correctness guarantee vs. soft prompt grounding | The real anti-hallucination layer; feeds self-healing |
| 9 | A | Vendor `workflow-engine-mcp` → Console MCP / tool client *(prog. E½, Wave 4)* | Protocol seam (Layer 3 client) for the Brain-MCP server | Console speaks MCP across HTTP/WS/stdio |
| 9 | B | Rust local-model node from the `claude-sdk-rs` spine *(prog. F, Wave 4)* | Python-free local inference for `bastion ask` (D23) | The Console gets its own offline brain |
| 10 | A | Proactive scanner → issue backlog *(prog. M, Wave 5)* | The missing proactive front-half of self-healing | The Brain finds its own problems |
| 10 | B | Findings → spec → draft PR via `sdlc-flow` (no auto-merge) *(prog. N½, Wave 5)* | Triggers fixes for human review; reuses the audited engine (D25) | The Brain proposes its own fixes |

> Phases 0–4 (workflow observability) and Phase 5 (session control) are **independent tracks**.
> Phase 5 has no dependency on the orchestrator and is not gated by D2 — it can be worked at any
> time, including before the monitor track completes.
>
> Phases 6–10 are **bastion's slice of the cross-repo Bastion program** (brain
> `planning/bastion-product/`, governed by brain **D24 / D25 / D26**), sequenced to follow the
> program's **demand-first wave order** (the `Wave N` tag on each row), not bastion-internal
> dependency. Each row notes its **program block letter** (`prog. X`); `½` marks the bastion half of a
> cross-repo block whose orchestrator/base-template peer is a prerequisite for the *combined* claim (the
> bastion half is independently shippable). This whole track is **opportunistic and ungated** — pull
> blocks as the need appears. Within it the only hard local prerequisites are: 6B/6C build on 6A;
> 7C builds on 7A (and is strengthened by 7B); 8A builds on 6A; 9B reuses 7A's error model;
> 10A builds on 7A + 8A; 10B builds on 10A. Program blocks **B, J, L, O, P, R, S, G** are **not** here
> — they execute in python-orchestration / base-template / the brain (G, the loop-proof, is coordinated
> from bastion but builds no bastion code; its artifact lands in the brain's `docs/content/`).

---

*Sequenced by dependency and competence, not calendar. When life gets in the way, pick up
where you left off.*
