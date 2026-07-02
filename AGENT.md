# AGENT.md — bastion

Personal Rust CLI — unified control panel for monitoring, validating, and operating the agentic engineering stack.

## Before you start

- **Strategic context:** `planning/context.md` (read first) → `planning/status.md` (current state)
- **Plan:** `planning/master-plan.md` — the phase/block sequence
- **Pipeline config:** `planning/harness.json` — the validation skills + UI-test config the
  SDLC engines run (see `planning/harness.examples.md` for ready-made stack profiles)
- **Decisions log:** `planning/decisions/` (start at `planning/decisions/index.md`) — check
  before relitigating any settled choice

## Standing rules

1. **Every new function, module, or behaviour change ships with tests.** No exceptions — this applies to ad-hoc fixes and one-off changes just as much as formal blocks/tasks. If you add or change code, add or update the tests that cover it.
2. **OKF frontmatter is required on every new `.md` file under `docs/` and `planning/`.** Three fields are **required**: `type`, `title`, `description`. Six are **optional but strongly encouraged**: `doc_id` (kebab-case filename stem), `layer` (list from closed vocab: `brain` · `engine` · `factory` · `console` · `surface` · `infra` · `business` · `content` · `meta`), `project` (closed vocab slug — use `bastion` for this repo; omit for cross-cutting docs), `status` (`active` · `draft` · `deprecated` · `superseded` · `archived`), `keywords` (3–7 free-form topic terms), `related` (list of `doc_id` values referencing other docs). Canonical guide: company-brain `docs/okf-frontmatter.md`; governing decision: brain **D27**. **Adding a new file to a directory requires updating that directory's `index.md`** (propagate up to the parent `index.md` if the scope changes).
3. **Sequence, not calendar** — work the order in `master-plan.md`; pick up where you left off.
4. **Decisions are append-only** — never edit a settled decision; supersede it with a new
   atomic file in `planning/decisions/` and link back.
5. **Verified identity / handles:** GitHub: bredmond1019 · Site: learn-agentic-ai.com · LinkedIn: bredmond1019 — treat these as the only authoritative
   identities/URLs; flag any other handle or profile link as unverified before publishing it.
6. **Coverage bar — separate pure logic from I/O, test the logic exhaustively.** A block is not
   "done" on a green `cargo test` alone; each pass must satisfy all of:
   - **Pure logic is exhaustively unit-tested without I/O.** Command/arg construction, parsing,
     formatting, and classification live in pure functions and are asserted directly (e.g.
     `*_args()` return `Vec<String>` checked element-by-element; parsers run against fixtures).
     Keep I/O boundaries (process spawns, Postgres, HTTP) thin shells over that pure core so the
     core stays testable — this is the established `tmux.rs` construction-vs-execution split.
   - **Error and degradation paths are tested, not just happy paths.** Every typed error variant
     and graceful-degradation branch a block introduces gets an explicit case (see
     `degrade_tmux_error` / `classify_no_server`).
   - **The thin I/O shell that can't be unit-tested is manually smoke-tested**, and the result is
     recorded in the task spec's `## Notes`. An untested execution fn is acceptable only when it is
     a trivial wrapper over already-tested pure functions.

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
> block). Keep the `<test>`/`<build>` skills here in sync with that file's
> `validation.checks[]` so humans and the pipeline run the same thing.

## Environment

Copy `.env.example` to `.env` and fill in:
```
DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres
BASTION_API_URL=http://localhost:8080
BASTION_POLL_INTERVAL=2
```

`DATABASE_URL` must point to the **Python orchestrator's PostgreSQL** instance. `bastion` reads from it directly (no changes to the Python side required).

To bring that instance up, run the orchestrator's dev stack **from the `python-orchestration-system/` repo** (starts Postgres + Redis + FastAPI on `:8080` + Celery in a tmux session):

```bash
./scripts/dev.sh        # START
./scripts/dev.sh stop   # STOP
```

Only the observability track (`monitor`, `costs`) needs this; the session surface runs DB-free (D4).

## Directory map

```
bastion/
├── .claude/            ← Agent skills + SDLC workflow engines
├── planning/           ← context, status, master-plan, harness.json, decisions/
├── crates/
│   └── bastion/        ← the bastion package (workspace member)
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs         ← clap dispatch
│           ├── cli.rs          ← subskill definitions
│           ├── config.rs       ← env/config loading
│           ├── observ/         ← structured error taxonomy (C001–C014) + tracing helpers (Phase 7)
│           ├── db/             ← PostgreSQL queries (workflows, costs)
│           ├── api/            ← reqwest client for FastAPI
│           ├── monitor/        ← live TUI graph inspector (ratatui + petgraph)
│           ├── inspect/        ← static post-mortem graph view
│           ├── validate/       ← markdown/MDX content validation
│           ├── costs/          ← LLM spend summary
│           ├── run/            ← workflow trigger + stack health check
│           ├── sessions/       ← tmux session control (Phase 5; shells to tmux, no DB) — D4
│           └── brain/          ← OKF corpus reader + petgraph structural queries (Phase 6)
└── Cargo.toml          ← workspace root manifest (members = ["crates/bastion"])
```

## What NOT to touch

<!-- Reference-only code, generated files, migration history, etc. List them as they appear. -->

---

## SDLC pipeline

This project carries the curated SDLC harness. Run `/prime` to orient, then drive structured
work through `/generate-tasks → /implement → /test → /review-task → /document → /log-work`.
See `.agents/skills/README.md` for the full pipeline reference.

> **Stack note:** the SDLC engines carry no stack defaults. Point them at this project's stack
> by filling `planning/harness.json` (validation skills + optional UI-test config). Copy a
> ready-made profile from `planning/harness.examples.md` (Rust / Python / Next.js). Do **not**
> edit the `workflows/*.js` engines for stack reasons — that's what `harness.json` is for.
