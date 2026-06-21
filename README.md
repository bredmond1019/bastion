---
type: Index
title: bastion
description: Personal Rust CLI — unified control panel for monitoring, validating, and operating the agentic engineering stack.
---

# bastion

Personal Rust CLI — unified control panel for monitoring, validating, and operating the agentic
engineering stack. Two surfaces: **workflow observability** (reads the Python orchestrator's
PostgreSQL) and **process/session control** (shells out to tmux, no database).

## Prerequisites

- **Rust** — stable toolchain via [rustup](https://rustup.rs).
- **tmux** — required for the session-control surface (`sessions` / `attach` / `new` / `kill` / `send`).
- **PostgreSQL** — *optional*. Only the workflow-observability track (`monitor`, `costs`) reads it;
  it points at the Python orchestrator's database. The session surface runs with **no database** (D4).

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
cargo run -- kill work         # remove the session
```

See [docs/sessions.md](docs/sessions.md) for the full session-control reference.

### Commands

| Command | Status | What it does |
|---|---|---|
| `status` | Shipped | Quick stack health check (API + DB reachability) |
| `sessions` | Shipped | List tmux sessions with state + last-line output |
| `attach <session>` | Shipped | Attach to a session; returns to shell on detach |
| `new <session> [--dir PATH]` | Shipped | Create a detached session, optional working dir |
| `send <session> <cmd...>` | Shipped | Send keystrokes + Enter without attaching |
| `kill <session>` | Shipped | Remove a session |
| `monitor` | Planned | Live TUI graph monitor (Phase 1; gated on the orchestrator) |
| `inspect <run_id>` | Planned | Static post-mortem graph view |
| `costs` | Planned | LLM spend summary |
| `run <workflow>` | Planned | Trigger a workflow via the FastAPI API |
| `validate <path>` | Planned | Markdown/MDX content validation |

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
| [docs/data-contract.md](docs/data-contract.md) | Orchestrator field mappings (monitor track) |
| [planning/context.md](planning/context.md) | Orientation + governing principles |
| [planning/master-plan.md](planning/master-plan.md) | Strategy + phase specifications |
| [planning/status.md](planning/status.md) | Current progress |
| [planning/harness.json](planning/harness.json) | SDLC validation/UI-test config (see `harness.examples.md`) |

---

*Initialized 2026-06-18 from `base-template` (commit `00ad2834e232d3243a3578132b02db01a7be40ab`).*
