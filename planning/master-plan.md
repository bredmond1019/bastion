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

> Phases 0–4 (workflow observability) and Phase 5 (session control) are **independent tracks**.
> Phase 5 has no dependency on the orchestrator and is not gated by D2 — it can be worked at any
> time, including before the monitor track completes.

---

*Sequenced by dependency and competence, not calendar. When life gets in the way, pick up
where you left off.*
