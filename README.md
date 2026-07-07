---
type: Index
title: bastion
description: Personal Rust CLI — unified control panel for monitoring, validating, and operating the agentic engineering stack.
doc_id: bastion
layer: [console]
project: bastion
status: active
keywords: [bastion, CLI, monitor, sessions, tmux, TUI, workflow observability]
related: [context, master-plan]
---

# bastion

> **Built within the Bastion workspace.** This crate depends on sibling repos via path dependency (`../okf-core`, `../mev`, `../bella/crates/bella-engine`) and is not designed to build standalone. See the [bastion-os](https://github.com/bredmond1019/bastion-os) meta-repo for the full ecosystem.
> Part of the **Bastion** ecosystem — see the [bastion-os](https://github.com/bredmond1019/bastion-os) front door for the full architecture.

Personal Rust CLI — unified control panel for monitoring, validating, and operating the agentic
engineering stack. Two surfaces: **workflow observability** (reads the Python orchestrator's
PostgreSQL) and **process/session control** (shells out to tmux, no database).

## Prerequisites

- **Rust** — stable toolchain via [rustup](https://rustup.rs).
- **tmux** — required for the session-control surface (`sessions` / `attach` / `new` / `kill` / `send`).
- **PostgreSQL** — *optional*. Only the workflow-observability track (`monitor`, `costs`) reads it;
  it points at the Python orchestrator's database. The session surface runs with **no database** (D4).

### Bringing up the orchestrator (for `monitor` / `costs`)

The workflow-observability track reads the Python orchestrator's PostgreSQL. To bring up the
orchestrator stack (Postgres + Redis + FastAPI on `:8080` + Celery worker, in a tmux session), run
**from the `orchestrator/` repo**:

```bash
./scripts/dev.sh        # START — ensures Postgres + Redis are up, launches FastAPI + Celery
./scripts/dev.sh stop   # STOP  — tears the dev tmux session down
```

`monitor` and `costs` need this running (or at least its Postgres reachable at `DATABASE_URL`).
The session-control surface does not — it never touches the database (D4).

## Setup

```bash
# 1. Clone, then from the bastion/ directory:
cp .env.example .env

# 2. Fill in .env (see the Environment section in CLAUDE.md):
#    DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres
#    BASTION_API_URL=http://localhost:8080
#    BASTION_POLL_INTERVAL=2
```

`DATABASE_URL` points at the **Python orchestrator's PostgreSQL** instance — bastion reads from it
directly as an observer (it never writes). The session-control surface does **not** require
`DATABASE_URL`; those commands run with Postgres stopped.

## Running locally

```bash
cargo run -- --help            # list all commands
cargo run -- status            # quick stack health check (API + DB reachability)

# Session control (no database needed):
cargo run -- sessions          # list tmux sessions with state + last-line output
cargo run -- new work --dir .  # create a detached session in the current dir
cargo run -- send work cargo test   # send keystrokes + Enter without attaching
cargo run -- attach work       # hand the terminal to tmux (Ctrl-b d to detach)
cargo run -- capture work      # print the session's recent pane output
cargo run -- kill work         # remove the session

# Workflow observability (needs the orchestrator stack up — ./scripts/dev.sh):
cargo run -- monitor               # live TUI graph of the active workflow run
cargo run -- monitor --workflow-id <id>   # monitor a specific run
```

See [docs/sessions.md](docs/sessions.md) for the full session-control reference and
[docs/monitor.md](docs/monitor.md) for the live monitor reference.

### Commands

| Command | Status | What it does |
|---|---|---|
| `status` | Shipped | Quick stack health check (API + DB reachability) |
| `sessions` | Shipped | List tmux sessions with activity state (`running (cmd)` / `idle`) + last-line output |
| `attach <session>` | Shipped | Attach to a session; returns to shell on detach |
| `new <session> [--dir PATH]` | Shipped | Create a detached session, optional working dir + trust pre-flight |
| `send <session> <cmd...>` | Shipped | Send keystrokes + Enter without attaching |
| `capture <session> [--lines N]` | Shipped | Print a session's recent pane output without attaching |
| `kill <session>` | Shipped | Remove a session |
| `monitor [--workflow-id ID]` | Shipped | Live two-pane TUI graph monitor — graph pane (nodes colored by state) + node detail pane; polls the orchestrator DB every `BASTION_POLL_INTERVAL`s |
| `inspect <run_id>` | Shipped | Static post-mortem graph TUI — one-shot DB load, nodes colored by status |
| `costs [--last W]` | Shipped | LLM spend summary — per-workflow token totals and estimated USD for `7d`, `30d`, or `all` |
| `run <workflow> [--args '{}'] [--monitor]` | Shipped | POST to orchestrator, print `task_id`; optional live-monitor hand-off |
| `validate <path>` | Shipped | Recursively validate `.md`/`.mdx` OKF frontmatter and relative links; greppable report, exits non-zero on errors |
| `ask --session S --prompt-file P --out O` | Shipped | Send a prompt file to a Claude session and wait for the output file; creates the session if absent |
| `brain (--dependents\|--blast-radius\|--lineage) <NODE_ID> [--root DIR] [--workspace NAME]` | Shipped | Structural queries over the OKF `[[link]]` graph: direct dependents, transitive blast radius, or full lineage |
| `code (--def\|--refs\|--dependents) <SYMBOL> [--root DIR] [--workspace NAME]` | Shipped | Symbol-level code graph queries via tree-sitter: definition sites, call/import references, or direct dependents |

## Configuration

bastion reads configuration from three layers, highest precedence first:

1. **Environment variables** (`DATABASE_URL`, `BASTION_API_URL`, `BASTION_POLL_INTERVAL`)
2. **`~/.config/bastion/config.toml`** (or `$XDG_CONFIG_HOME/bastion/config.toml`)
3. **Built-in defaults** (`BASTION_API_URL=http://localhost:8080`, `BASTION_POLL_INTERVAL=2`)

A missing or unreadable config file is silently ignored. See [docs/config.md](docs/config.md) for
the full reference and an example `config.toml`.

## Help and man page

```bash
bastion --help                 # short help
bastion help <cmd>             # subcommand help
bastion <cmd> --help           # subcommand long help

bastion man                    # print the roff man page to stdout
bastion man --out /tmp/man     # write bastion.1 (+ one page per subcommand) to a directory
man -l /tmp/man/bastion.1      # view the generated man page
```

## Tests

```bash
cargo test                     # run the test suite
```

The full validation gate (the same suite the SDLC pipeline runs — see
[planning/harness.json](planning/harness.json)):

```bash
cargo fmt --check              # format gate
cargo clippy -- -D warnings    # lint gate
cargo test                     # test suite
cargo build --release          # build gate
```

## Directory map

```
bastion/
├── .claude/        ← Claude Code commands + SDLC workflow engines
├── planning/       ← context, status, master-plan, harness.json, decisions/
├── docs/           ← user-facing docs (this surface) + the orchestrator data contract
└── src/            ← clap dispatch, config, db/, api/, monitor/, sessions/, …
```

## Documentation

| Doc | Contents |
|---|---|
| [docs/index.md](docs/index.md) | Router for `docs/` |
| [docs/sessions.md](docs/sessions.md) | Session-control surface — verb reference + operator workflow |
| [docs/monitor.md](docs/monitor.md) | Live monitor surface — keybindings, two-pane layout, flags, degrade paths |
| [docs/data-contract.md](docs/data-contract.md) | Orchestrator field mappings (monitor track) |
| [docs/config.md](docs/config.md) | Configuration reference — env vars, config file, precedence |
| [docs/brain.md](docs/brain.md) | OKF knowledge-graph queries — `bastion brain`: corpus discovery, `--dependents` / `--blast-radius` / `--lineage`, workspace resolution |
| [docs/code.md](docs/code.md) | Symbol-level code graph queries — `bastion code`: tree-sitter extraction, `--def` / `--refs` / `--dependents`, degradation paths |
| [planning/context.md](planning/context.md) | Orientation + governing principles |
| [planning/master-plan.md](planning/master-plan.md) | Strategy + phase specifications |
| [planning/status.md](planning/status.md) | Current progress |
| [planning/harness.json](planning/harness.json) | SDLC validation/UI-test config (see `harness.examples.md`) |

## Roadmap / Known limitations

- **Error Handling:** Currently uses a hybrid approach. The `.contains()` keyword heuristic fallback for un-downcastable `anyhow` errors should be dropped in favor of full typed-error propagation.

---

*Initialized 2026-06-18 from `base-template` (commit `00ad2834e232d3243a3578132b02db01a7be40ab`).*
