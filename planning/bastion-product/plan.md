---
type: Plan
title: Bastion Product — Packaging the Agent OS for adoption
description: Phase/block roadmap to turn bastion into an installable, adoptable open-source agent OS via `bastion init`/`assess`, a workspace consolidation, and a shared OKF contract.
doc_id: bastion-product-plan
layer: [console, factory, meta]
project: bastion
status: active
keywords: [bastion init, agent os, okf-core, workspace consolidation, scaffolding, adopt, tasks.json]
related: [master-plan, planning-index, context]
---

# Bastion Product — Packaging the Agent OS for adoption

*Living plan. Created 2026-07-02. Full design rationale + exploration findings live in the approved
design doc; this file is the executable phase/block roadmap in bastion convention.*

## The Goal, Stated Plainly

Turn `bastion` from a personal CLI hardwired to Brandon's brain into a **self-contained, open-source
"agent OS"** anyone can adopt in one command. Two entry paths:

- **`bastion init`** — stand up a fresh brain (brain.toml + planning/ tree + SDLC skills + README +
  compliant OKF frontmatter) in an empty/greenfield repo.
- **`bastion assess`** — read-only readiness diagnostic for any repo (OKF coverage, graph, state).
- **`bastion adopt`** *(deferred)* — non-destructive migration of an existing repo, incl. frontmatter backfill.

The sibling projects (`mev`, `bella-engine`, `workflow-engine-rs`, `base-template`, optional Python
`orchestrator`) consolidate into **one cargo workspace** so shared logic becomes ordinary workspace
crates, and the three CLIs collapse into **one `bastion` binary** (sub-tools become libraries; optional
thin `mev`/`bella` shims preserved). Rust-first engine (`workflow-engine-rs`), Python orchestrator as an
optional extension.

"Ready" means: `cargo install bastion` → `cd my-repo && bastion init` → a working brain, and
`bastion validate-brain` on that fresh repo returns zero errors (the contract holds end-to-end).

## Decisions locked (from design review)

1. **Plan boundary:** consolidate into a workspace, then build `init`/`assess` on top — one sequenced plan.
2. **Distribution:** installable binary with **embedded** templates (`init` works standalone in any repo).
3. **Feature scope now:** `init` + `assess`; **`adopt` deferred** to a follow-on wave.
4. **Engine story:** Rust-first (`workflow-engine-rs`); Python `orchestrator` optional extension.
5. **Unified CLI:** one `bastion` binary; `mev`'s commands fold into `bastion validate-brain` /
   `emit-state` / `manifest` / `graph`; `bella` → `bastion view` / `edit`.

## Conventions this plan must uphold

### Block/task naming — `PREFIX.PHASE.BLOCK[.TASK]`

Every project/phase/block/task ID is globally unique and derived from the project's `prefix` in
`brain.toml`. Phases are numeric; blocks are numeric **or** letter; tasks are numeric.

- `BA.12.4` = **Ba**stion · Phase 12 · Block 4
- `MV.3.L.3` = **Mv** (mev) · Phase 3 · Block L · Task 3

This program is **bastion Phase 15**; its blocks are `BA.15.0 … BA.15.N`, tasks `BA.15.<block>.<n>`.
Uniqueness + format are enforced by tooling introduced in **BA.15.6**.

### Machine-parsable specs — `tasks.json` companion

Every command that produces a `tasks.md` (e.g. `/generate-tasks`) **also emits a machine-parsable
`tasks.json`** in the same concept folder, and merges it into the block's `tasks[]` entry in
`state.json`. `tasks.md` stays human-facing; `tasks.json` is the source of truth for tooling
(Kanban, `emit-state`, the naming validator). Wired in **BA.15.5**.

---

## Phase 15 — Bastion Product Packaging

Waves are dependency-ordered; blocks within a wave are parallelizable.

### Wave 1 — Consolidation & the shared contract

**BA.15.0 — Cargo workspace skeleton.** Add a root `[workspace]`; move today's `bastion` sources under
`crates/bastion/`; pull siblings in as members (`git subtree add` to preserve history where it matters).
Repoint each member's `Cargo.toml` to workspace-relative deps. *Note: bastion already path-depends on
`bella-engine` + `workflow-engine-*`, so consolidation is partly underway.*
Depends on: none.

**BA.15.1 — Extract `okf-core`.** Single-source the OKF frontmatter contract into one crate:
`OkfFrontmatter` model, `extract_frontmatter`/parse, closed-vocab validators (layer/project/status,
doc_id kebab, `is_kebab_case`), the `state.rs` serde schema, and **`serialize_frontmatter`** (the write
path). ✅ **Head start:** the model + `serialize_frontmatter` + 18 tests are already prototyped in
`crates/bastion/src/okf/` (was `src/okf/`) — lift them into the crate, then repoint bastion's
`brain/okf.rs` + `validate/frontmatter.rs` and mev at `okf-core`.
Depends on: BA.15.0.

**BA.15.2 — Unify the CLI (bastion-side); mev stays a path-dep repo.** *(Split from the original
BA.15.2 per [D15](../decisions/D15-mev-integration-cross-repo-path-dep.md); the mev-side dedup is now the
deferred BA.15.12.)* mev is **already** a library (`validate_brain`, `emit_state`, `manifest_brain`,
`graph_brain`, `visualize_brain`). Add `bastion` subcommands `validate-brain` / `emit-state` / `manifest`
/ `graph` that call **mev via a cross-repo path dep** (`mev = { path = "../mev" }`) and `bastion view` /
`edit` over `bella-engine`, following the declare→name→dispatch + DB-free pattern. **No `mev`/`bella`
source changes; no `bin-shims`** (they keep their own standalone binaries). Behaviour identical to the
`mev`/`bella` CLIs. Depends on: BA.15.1.

**BA.15.3 — Licensing + front-door README.** Root `LICENSE` (MIT OR Apache-2.0 dual), per-crate
`license` fields, top-level README framing the OS + install + `init` quickstart.
Depends on: BA.15.0.

### Wave 2 — Portable templates & machine-parsable specs

**BA.15.4 — Vendor + embed the template pack.** Copy `base-template/scaffold/` (tokenized D30 file
pack) and the harness (`.claude/commands` + `.claude/workflows` engines + `harness.schema.json` +
`harness.examples.md`) into `templates/`; embed at compile time via `rust-embed`/`include_dir!` in
`crates/bastion/src/init/templates.rs`. Expose `iter_template_files()` + `render(path, &TokenMap)`.
Token set: `{{PROJECT_NAME}}` `{{SLUG}}` `{{DESCRIPTION}}` `{{PROJECT_TYPE}}` `{{DATE}}` `{{PREFIX}}`
`{{STACK}}` `{{VERIFIED_HANDLES}}`.
Depends on: BA.15.0.

**BA.15.5 — `tasks.json` emission + state.json block sync.** Extend `/generate-tasks` (and siblings that
write `tasks.md`) to also emit `tasks.json` and merge it into the block's `tasks[]` in `state.json`.
Reconcile the two template drifts found in exploration: scaffold `status.md` `timestamp:` → `updated:`,
and add a seeded minimal `planning/state.json` (`kind:"project"`, empty `focus`/`tracks`, stamped
`updated`) so the TUI Kanban/overview + `emit-state` have day-one input.
Depends on: BA.15.4 (+ BA.15.1 for the state schema).

**BA.15.6 — Naming-convention engine.** Pure ID parser/validator for `PREFIX.PHASE.BLOCK[.TASK]`:
derive `PREFIX` from `brain.toml`, validate format, and check global uniqueness across a repo's
`state.json`/`tasks.json`. Surfaced through `bastion validate-brain` (error on malformed/duplicate IDs)
and reused by `init`/`assess`.
Depends on: BA.15.1.

### Wave 3 — Scaffolder & diagnostics

**BA.15.7 — `brain.toml` serializer + full `SpaceEntry` round-trip.** Enable the `toml` crate's
serialize/`display` feature (currently `parse`-only). Extend `spaces::SpaceEntry`/`BrainToml` to
round-trip the full schema (`slug, prefix, tier, repo_path, status_file, cache_doc, heading`) + the
top-level `[vocab]` (layer/status) and `[crawl] skip_dirs` tables. Pure serialize↔`parse_space_tree`
test pair.
Depends on: BA.15.0.

**BA.15.8 — `bastion init` (greenfield scaffold).** New `Init { path, name, prefix, stack, tier, yes }`
subcommand. Resolve target (default `.`); refuse if `brain.toml` exists (point at future `adopt`);
prompt or take flags; build `TokenMap`; render + write the embedded templates; emit `brain.toml`
(BA.15.7) with a self-referential `[[repos]]`; stamp `planning/.template-version`; print a `man.rs`-style
summary. Pure render/token/serialize core, thin write shell (repo rule #6).
Depends on: BA.15.4, BA.15.6, BA.15.7.

**BA.15.9 — `bastion assess` (read-only diagnostic).** New `Assess { path, json }`. Locate
brain.toml/planning via `config::walk_up_for`; discover markdown via `validate::find_markdown_files`;
compute OKF coverage (missing/invalid fields via `okf-core`), graph readiness (node count + dangling
`[[links]]` via `build_node_edge_lists`), state readiness (focus/tracks presence), and ID-convention
violations (BA.15.6). Human summary or `--json` envelope (mev convention).
Depends on: BA.15.1, BA.15.6.

### Deferred (follow-on waves, not scoped here)

**BA.15.10 — `bastion adopt`.** Non-destructive migration of an existing repo: `--dry-run` proposal +
`--apply`, with **hybrid** frontmatter backfill (bastion mechanically stamps required fields via
`serialize_frontmatter` inferring `type`/`title`/`description` from path + first heading; a Claude Code
skill enriches `layer`/`keywords`/`related` as a reviewable pass). Depends on BA.15.1/4/6/7.

**BA.15.11 — Engine packaging.** `bastion run` bundling the `workflow-engine-rs` runtime + optional
Python `orchestrator` extension. Its own plan once the workspace lands.

**BA.15.12 — mev/okf-core format convergence.** *(Split from BA.15.2 per D15 — the risky half.)* Extract
a `state.json` serde schema + reconciled `OkfFrontmatter` into `okf-core`, then repoint mev's
`brain/okf.rs` + `brain/state.rs` at `okf-core` and delete mev's dupes. Deferred: mev's graph/state
validation is proven and load-bearing; the CLI value already shipped in BA.15.2. Executed partly in mev's
own repo. Depends on BA.15.1/15.2.

---

## Critical files (by block)

| Block | Files |
|---|---|
| BA.15.0 | root `Cargo.toml`; move `src/` → `crates/bastion/src/`; member `Cargo.toml`s |
| BA.15.1 | `crates/okf-core/` (lift from `crates/bastion/src/okf/`); repoint `brain/okf.rs`, `validate/frontmatter.rs`, mev |
| BA.15.2 | `crates/bastion/Cargo.toml` (`mev = { path = "../mev" }`), `crates/bastion/src/cli.rs` + `main.rs` (declare→name→dispatch over mev + bella-engine) — no mev/bella changes, no `bin-shims` (D15) |
| BA.15.12 *(deferred)* | `crates/okf-core/` (add state schema + reconciled OKF model); mev `brain/okf.rs` + `brain/state.rs` (mev's own repo) |
| BA.15.4 | `templates/`, `crates/bastion/src/init/templates.rs` |
| BA.15.5 | `.claude/commands/generate-tasks*`, workflow engines, `okf-core` state schema |
| BA.15.6 | `crates/okf-core/` (ID parser), `mev-core` validate path |
| BA.15.7 | `crates/bastion/src/brain/spaces.rs`, `Cargo.toml` (`toml` serialize feature) |
| BA.15.8 | `crates/bastion/src/init/mod.rs`, `cli.rs`, `main.rs` |
| BA.15.9 | `crates/bastion/src/assess/mod.rs`, `cli.rs`, `main.rs` |

**Reused as-is:** `config::walk_up_for`, `brain::okf::build_node_edge_lists`,
`validate::find_markdown_files`, `spaces::parse_space_tree`, the `man.rs` command skeleton.

## Verification

1. **Gates across the workspace:** `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`,
   `cargo build --release` — all green.
2. **`init` e2e:** empty temp dir → `bastion init --name Demo --prefix DM --stack rust --yes` → assert
   `brain.toml`, `planning/{status,context,master-plan,state.json,harness.json}`, `.claude/`, README,
   CLAUDE.md exist; `grep -rn '{{'` finds no unsubstituted tokens; bare `bastion` TUI reads the seeded
   `state.json`.
3. **Contract check:** `bastion validate-brain` on the fresh repo → zero errors (proves `init` writes
   frontmatter the shared `okf-core` validator accepts). Confirm folded `bastion emit-state` /
   `bastion view <file>` work.
4. **Naming enforcement:** feed a duplicate/malformed ID → `validate-brain` errors with file + ID.
5. **`assess` e2e:** fresh repo → high coverage, 0 dangling; a repo with a bare `.md` → reports the gap.
6. **No-clobber:** `bastion init` where `brain.toml` exists → clean exit pointing at `adopt`, writes nothing.
