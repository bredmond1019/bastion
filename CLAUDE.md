# CLAUDE.md — bastion

Personal Rust CLI — unified control panel for monitoring, validating, and operating the agentic engineering stack.

## Before you start

- **Strategic context:** `planning/context.md` (read first) → `planning/status.md` (current state)
- **Plan:** `planning/master-plan.md` — the phase/block sequence
- **Pipeline config:** `planning/harness.json` — the validation commands + UI-test config the
  SDLC engines run (see `planning/harness.examples.md` for ready-made stack profiles)
- **Decisions log:** `planning/decisions/` (start at `planning/decisions/index.md`) — check
  before relitigating any settled choice

## Standing rules

1. **Every block/task ships with tests** covering its core functionality. No exceptions.
2. **Maintain OKF frontmatter** on every markdown file.
3. **Sequence, not calendar** — work the order in `master-plan.md`; pick up where you left off.
4. **Decisions are append-only** — never edit a settled decision; supersede it with a new
   atomic file in `planning/decisions/` and link back.
5. **Verified identity / handles:** GitHub: bredmond1019 · Site: learn-agentic-ai.com · LinkedIn: bredmond1019 — treat these as the only authoritative
   identities/URLs; flag any other handle or profile link as unverified before publishing it.
6. <!-- Add project-specific standing rules here (prompt handling, registries, deployment
   boundaries, code style, etc.). -->

## Known bugs

None known at initialization.

## Build / test / run

```bash
cargo fmt --check          # format gate
cargo clippy -- -D warnings  # lint gate
cargo test                 # test suite
cargo build --release      # release build
cargo run -- --help        # verify CLI help
cargo run -- status        # smoke test (Phase 0+)
```

> The SDLC pipeline reads its validation suite from `planning/harness.json` (not from this
> block). Keep the `<test>`/`<build>` commands here in sync with that file's
> `validation.checks[]` so humans and the pipeline run the same thing.

## Environment

Copy `.env.example` to `.env` and fill in:
```
DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres
BASTION_API_URL=http://localhost:8080
BASTION_POLL_INTERVAL=2
```

`DATABASE_URL` must point to the **Python orchestrator's PostgreSQL** instance. `bastion` reads from it directly (no changes to the Python side required).

## Directory map

```
bastion/
├── .claude/            ← Claude Code commands + SDLC workflow engines
├── planning/           ← context, status, master-plan, harness.json, decisions/
├── src/
│   ├── main.rs         ← clap dispatch
│   ├── cli.rs          ← subcommand definitions
│   ├── config.rs       ← env/config loading
│   ├── db/             ← PostgreSQL queries (workflows, costs)
│   ├── api/            ← reqwest client for FastAPI
│   ├── monitor/        ← live TUI graph inspector (ratatui + petgraph)
│   ├── inspect/        ← static post-mortem graph view
│   ├── validate/       ← markdown/MDX content validation
│   ├── costs/          ← LLM spend summary
│   └── run/            ← workflow trigger + stack health check
└── Cargo.toml
```

## What NOT to touch

<!-- Reference-only code, generated files, migration history, etc. List them as they appear. -->

---

## SDLC pipeline

This project carries the curated SDLC harness. Run `/prime` to orient, then drive structured
work through `/generate-tasks → /implement → /test → /review-task → /document → /log-work`.
See `.claude/commands/README.md` for the full pipeline reference.

> **Stack note:** the SDLC engines carry no stack defaults. Point them at this project's stack
> by filling `planning/harness.json` (validation commands + optional UI-test config). Copy a
> ready-made profile from `planning/harness.examples.md` (Rust / Python / Next.js). Do **not**
> edit the `workflows/*.js` engines for stack reasons — that's what `harness.json` is for.
