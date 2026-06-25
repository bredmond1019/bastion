# Task Spec — Phase 6, Block B

**Status:** Not started · **Last run:** never

## Goal
Generalize the bastion brain graph reader to address an **arbitrary, named knowledge workspace** (per-repo / per-client root) selectable by `--workspace` or config, with the default still resolving to the brain repo — the Console half of the cross-repo multi-workspace block.

## Context Pointers
- **Plan:** `planning/master-plan.md` → *Phase 6 — Block B* (program Block C, bastion half). Carry its **Out of scope** verbatim (no Python indexer half; no OKF de-opinionating; no switching UX beyond name selection; no packaging).
- **What Block A already shipped (verify before building):** `bastion brain` already takes an arbitrary root — `brain::run(query, root: PathBuf)` and a `--root <path>` flag (default `.`); `okf::build_node_edge_lists(docs)` is already **pure** over pre-read `(PathBuf, String)` docs, and all I/O (file discovery via `validate::find_markdown_files`, reads) lives in `brain::run`. **So the plan's "make okf.rs take a root parameter" is already satisfied.** Block B's genuine delta is the **named workspace registry** (resolve a *name* → root via config), not raw-root support.
- **DB-free constraint (load-bearing — D4):** `bastion brain` must run with **no `DATABASE_URL`**. But `Config::load()` / `Config::from_sources` *require* `DATABASE_URL` (`ConfigError::MissingVar` otherwise — see `src/config.rs:95-97`). Therefore the workspace registry must be read on a **separate lightweight path** that parses the config file's workspace table **without** going through the DB-requiring `Config` constructor.
- **Config precedence to mirror:** `src/config.rs` — `FileConfig` (serde-deserialized TOML, unknown keys ignored), `parse_file`, `config_path` (pure, env-string args), and the env > file > built-in layering in `from_sources`. Add the workspace registry the same way.
- **Standing rules:** `CLAUDE.md` Rule 6 (pure resolver exhaustively unit-tested incl. unknown-name / fallback error paths; thin I/O shell smoke-tested + recorded in `## Notes`). Governing principle 4 (read-only over any workspace).

## Step-by-Step Tasks

### 1. Workspace registry + pure resolver in `config.rs` (DB-free)
- Extend `FileConfig` with the workspace table: `workspaces: Option<HashMap<String, PathBuf>>` (TOML `[workspaces]` name→root) and `default_workspace: Option<String>`. Keep unknown-key tolerance (no `deny_unknown_fields`).
- Add a typed error for an unknown workspace name (new `ConfigError::UnknownWorkspace(String)` variant, or a dedicated `WorkspaceError` — pick one and test it).
- Add a **pure** resolver, no I/O and **no `DATABASE_URL` dependency**:
  `resolve_workspace_root(explicit_root: Option<PathBuf>, workspace_name: Option<&str>, file: &FileConfig) -> Result<PathBuf, ConfigError>` with precedence: explicit `--root` → `--workspace <name>` lookup in the registry (unknown name → typed error) → `default_workspace` lookup → built-in default (preserve Block A's behavior: the brain repo corpus / `.`).
- Add a lightweight loader that reads **only** the workspace registry from the config file (reuse `parse_file` + `config_path`) and returns a `FileConfig` (or just its workspace fields) **without** constructing the DB-requiring `Config`. A missing/unreadable file degrades silently to an empty registry (default still resolves).
- **Tests (Rule 6):** exhaustively unit-test `resolve_workspace_root` — explicit root wins over name; named lookup hits; unknown name → typed error; `default_workspace` fallback; no-config built-in default; and `parse_file` round-trips a `[workspaces]` table.
- **Files:** *Modified* `src/config.rs`.

### 2. [~] Portable (non-repo) OKF workspace fixture + okf portability coverage
- Create `src/brain/fixtures/portable/` — a **second, non-repo** OKF corpus in a different domain than Block A's decision graph (e.g. a small client/project knowledge dir: a handful of interlinked `.md` nodes with OKF frontmatter, at least one lineage chain and one unresolved `[[link]]`), proving the reader is not hardcoded to this repo's ids.
- In `src/brain/okf.rs`, add a portability test module asserting `build_node_edge_lists` over the portable-fixture docs produces the expected nodes/edges (distinct ids from Block A's fixture) — demonstrating the pure reader works over *any* conforming corpus. No production code change is expected here; if one proves necessary, keep `okf` pure (root in / lists out) and justify in `## Notes`.
- **Files:** *New* `src/brain/fixtures/portable/*.md`; *Modified* `src/brain/okf.rs` (tests).

### 3. [~] Wire `--workspace` selection through CLI + `run()`
- `src/cli.rs`: add `--workspace <NAME>` (with `--knowledge-dir` as a documented alias) to the `Brain` subcommand; keep `--root <path>` as the explicit override. Change `--root` from `default_value = "."` to an `Option<PathBuf>` so "unset" is distinguishable and the resolver can choose the workspace/default.
- `src/brain/mod.rs`: in `run` (or a thin pre-step), call `config::resolve_workspace_root(explicit_root, workspace_name, &registry)` to compute the effective root, then proceed exactly as today (`find_markdown_files` → read → `build_node_edge_lists` → graph → query). Stay DB-free — load only the workspace registry, never `Config::load`. An unknown workspace name degrades gracefully (clear message, non-zero exit).
- `src/main.rs`: load the workspace registry (DB-free), pass the resolved root / resolution inputs into `brain::run`.
- **Tests:** unit-test any new pure helpers (flag → resolution input mapping). **Smoke test (Rule 6):** run `bastion brain --workspace <name> --dependents <id>` against the portable fixture **and** confirm the default (no `--workspace`/`--root`) still resolves to the brain repo corpus; record both in `## Notes`.
- **Files:** *Modified* `src/cli.rs`, `src/brain/mod.rs`, `src/main.rs`.
- **Depends on:** Tasks 1 & 2.

### 4. Validate
- Run the Validation Commands listed below and confirm all pass.
- Confirm `bastion brain` still runs with **no `DATABASE_URL`** set (DB-free, D4) — workspace resolution must not introduce a DB requirement.

## Acceptance Criteria
- The bastion graph reader indexes and answers (dependents / blast-radius / lineage) over a **second, non-repo OKF workspace** selected by `--workspace <name>` (resolved via config registry) or an explicit `--root`.
- With no `--workspace`/`--root`, the default still resolves to the **brain repo** corpus (Block A behavior preserved).
- An unknown workspace name fails with a clear, typed error and a non-zero exit (no panic, no silent wrong-corpus).
- `bastion brain` runs with **no `DATABASE_URL`** set (DB-free per D4) — verified.
- The portability fixture under `src/brain/fixtures/portable/` is covered; the pure `resolve_workspace_root` resolver is exhaustively unit-tested incl. unknown-name and fallback paths; the thin workspace-resolution I/O shell is smoke-tested and recorded in `## Notes`.
- All gated checks pass: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes

### Task 3 smoke tests (2026-06-25)

**Unknown workspace name — clear error, non-zero exit (DB-free):**
```
$ bastion brain --workspace unknown-name --dependents some-id
brain: unknown workspace 'unknown-name' — not found in [workspaces] registry
Exit: 1
```

**--root against portable fixture (explicit override):**
```
$ bastion brain --root src/brain/fixtures/portable --dependents proj-overview
dependent: team-roster    src/brain/fixtures/portable/team-roster.md
dependent: portable-index src/brain/fixtures/portable/index.md
Exit: 0
```

**--knowledge-dir alias resolves to the same typed error:**
```
$ bastion brain --knowledge-dir unknown --dependents proj-overview
brain: unknown workspace 'unknown' — not found in [workspaces] registry
Exit: 1
```

**DB-free confirmed:** all invocations above ran with no DATABASE_URL set.

**Default (no --workspace/--root) resolves to ".":** covered by
`resolve_no_config_returns_dot` unit test in config.rs.

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the spec. -->
_No amendments yet._
