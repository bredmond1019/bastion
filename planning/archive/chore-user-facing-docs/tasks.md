---
type: ChorePlan
title: User-Facing Documentation for Shipped Surface
description: Fill in README.md and add docs/sessions.md + docs/index.md to document the shipped status and sessions command surface.
---

# Chore: User-Facing Documentation for the Shipped Surface

## Metadata
prompt: `Write user-facing documentation for the shipped surface: fill in README.md (Prerequisites, Setup, Running locally, Tests), add docs/sessions.md (verb reference for sessions/attach/new/kill/send + phone→SSH→Tailscale workflow + DB-free guarantee), and add docs/index.md (doc router for docs/). All markdown gets OKF frontmatter.`

## Chore Description

The codebase now has a fully working, user-facing surface — `bastion status` plus the entire
`sessions` family (`sessions` / `attach` / `new` / `kill` / `send`, Phase 5 Blocks A–C) — but the
docs are still scaffold placeholders. `README.md` is all empty `<!-- ... -->` stubs, and `docs/`
holds only `data-contract.md` (the monitor-track field mapping). This chore documents what is
actually shipped and usable, with **no forward documentation of vapor** — the `monitor`/`inspect`/
`costs`/`run`/`validate` verbs are stubbed and/or gated on the orchestrator (D2), so they are listed
as planned, not documented as working.

Scope is documentation only — no source changes, so the `cargo` gates should be untouched-green.
The bar: README is genuinely runnable from zero; `docs/sessions.md` is the operator manual for the
one complete surface; `docs/index.md` is a short router so `docs/` is navigable now that it has more
than one file.

Constraints:
- **OKF frontmatter** on every markdown file (standing rule 2): `type`, `title`, `description`.
- **No emoji** (universal harness rule).
- **No fabricated facts** — only document verbs/flags that exist in `src/cli.rs`. Cross-check every
  documented flag against the actual clap definitions before writing.
- Respect the **verified-identity rule** (rule 5): GitHub `bredmond1019`, site `learn-agentic-ai.com`.
  Do not invent URLs or handles.
- The `sessions` surface is **DB-free (D4)** and **synchronous (D5)** — state this explicitly as the
  guarantee that lets it run with Postgres stopped.

## Relevant Files

- `CLAUDE.md` — source of truth for the build/test/run commands, env vars, and directory map; the
  README's Setup/Running/Tests sections should mirror it (and the "keep in sync" note already there).
- `src/cli.rs` — authoritative list of subcommands + flags; every documented verb/flag must match it
  exactly (`sessions`, `attach <session>`, `new <session> --dir`, `kill <session>`,
  `send <session> <cmd...>`, `status`).
- `src/sessions/commands.rs` — the user-facing output strings + graceful-degradation messages
  (e.g. "tmux not installed…", "no tmux server running", "session '<x>' not found") to document the
  error behavior accurately.
- `src/sessions/tmux.rs` — confirms the underlying tmux invocations + the literal-send (`-l`/`--`)
  escaping behavior worth a one-line note in the `send` section.
- `planning/master-plan.md` (Phase 5 intro) — the phone → SSH over Tailscale → `bastion` workflow
  narrative to summarize in `docs/sessions.md`.
- `.env.example` — the env vars to reference in README Setup (DATABASE_URL, BASTION_API_URL,
  BASTION_POLL_INTERVAL); note DATABASE_URL is **not** required for the sessions surface.
- `docs/data-contract.md` — existing doc that `docs/index.md` must link.
- `README.md` — the file to fill in (currently placeholder).

### New Files
- `docs/sessions.md` — verb reference + operator workflow for the session-control surface.
- `docs/index.md` — router for `docs/`.

## Step by Step Tasks
IMPORTANT: Execute every step in order, top to bottom.

### 1. Verify the documented surface against the code
- Read `src/cli.rs` and list every currently-wired subcommand and its flags verbatim. Treat this as
  the allow-list — do not document any verb/flag not present here.
- Read the output/degradation strings in `src/sessions/commands.rs` so the error behavior documented
  in `docs/sessions.md` matches what the binary actually prints.
- Note which verbs are stubs/gated (`monitor`, `inspect`, `costs`, `run`, `validate`) so they are
  marked "planned", not documented as working.

### 2. Fill in README.md
- Keep the existing OKF frontmatter and the top-level title/description.
- **Prerequisites:** Rust toolchain (stable, via rustup); `tmux` (required for the sessions surface);
  PostgreSQL — optional, only for the monitor/costs track, explicitly **not** needed for sessions.
- **Setup:** clone, `cp .env.example .env`, fill the three env vars (from CLAUDE.md / `.env.example`),
  and the note that the sessions surface runs without `DATABASE_URL`.
- **Running locally:** the real commands — `cargo run -- --help`, `cargo run -- status`,
  `cargo run -- sessions`, and a representative `send`/`new`/`attach`/`kill` example. Add a one-line
  "Commands" table splitting **Shipped** (status, sessions family) from **Planned** (monitor, inspect,
  costs, run, validate) so nothing reads as working that isn't.
- **Tests:** `cargo test` one-liner plus the full gate list (fmt, clippy, test, build) pointing to
  `planning/harness.json` as the source of truth.
- Update the Documentation table to link `docs/index.md`, `docs/sessions.md`, and `docs/data-contract.md`.
- Do not leave any `<!-- ... -->` placeholder behind.

### 3. Write docs/sessions.md
- OKF frontmatter (`type: Reference` or similar, `title`, `description`).
- Opening: what the sessions surface is (manage long-running tmux sessions on the Mac Mini), and the
  **DB-free (D4) / synchronous (D5)** guarantee — runs with Postgres stopped, no orchestrator dependency.
- **Operator workflow:** from the phone, SSH into the Mini over Tailscale → run `bastion` → use the
  session verbs (summarized from master-plan Phase 5 intro). Do not over-specify infra that lives in
  the brain repo — link/describe at a high level only.
- **Verb reference** — one subsection each, matching `src/cli.rs` exactly:
  - `bastion sessions` — list sessions with state + last-line output.
  - `bastion attach <session>` — hand the terminal to tmux; returns to shell on `Ctrl-b d` detach.
  - `bastion new <session> [--dir PATH]` — create a detached session, optional working dir.
  - `bastion send <session> <cmd...>` — send keystrokes + Enter without attaching; note the literal
    `-l`/`--` escaping so multi-word/key-like/hyphen-leading commands are sent verbatim.
  - `bastion kill <session>` — remove a session.
- **Error behavior:** document the graceful-degradation messages (tmux not installed, no server
  running, unknown session) exactly as `commands.rs` emits them.

### 4. Write docs/index.md
- OKF frontmatter (`type: Index`, `title`, `description`).
- A short router table for `docs/`: `index.md` (this file), `sessions.md` (session-control surface),
  `data-contract.md` (orchestrator field mappings for the monitor track). One line each.
- A pointer up to `planning/` for internal context (context.md / master-plan.md / status.md).

### 5. Validate
- Run the Validation Commands listed below and confirm all pass (these should be unaffected — docs
  only — but run them to prove the tree is clean and green).
- Manually re-read each new/edited doc and confirm: no `<!-- -->` placeholders remain, every
  documented verb/flag exists in `src/cli.rs`, OKF frontmatter present on all three files, no emoji.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
- Documentation-only chore: no test changes are expected because no source changes. The "every
  change ships with tests" rule is satisfied here by the existing suite staying green — call this out
  in the implement report rather than inventing doc "tests".
- Source of truth for the README's build/run/test commands is CLAUDE.md; keep them identical so the
  existing "keep in sync with harness.json" note stays honest.
- Do not document the monitor track as functional — Phase 1 is gated on the orchestrator (D2) and only
  Block A (data layer) has landed.
