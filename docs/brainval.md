---
type: Reference
title: brainval — mev Brain-Ops Pass-Throughs (validate-brain / manifest / graph / emit-state)
description: "Reference for `bastion validate-brain`, `bastion manifest`, `bastion graph`, and `bastion emit-state` — thin pass-throughs to the `mev` crate's brain-ops library functions (D15, Block BA.15.2)."
doc_id: brainval
layer: [console, brain]
project: bastion
status: active
keywords: [mev, validate-brain, manifest, emit-state, pass-through, build-stamp, drift]
related: [brain, validate, okf]
---

# brainval — mev Brain-Ops Pass-Throughs

`bastion validate-brain`, `bastion manifest`, `bastion graph`, and `bastion emit-state` are thin
pass-through subcommands over the `mev` crate's brain-ops library functions (Phase 15, Block
BA.15.2 — see `planning/decisions/D15`). All four resolve `brain.toml` by walking up from a
`--path` argument (default `.`) via `mev::brain::config::find_brain_root`, then dispatch straight
into the matching `mev::*` function — no bastion-side reimplementation of validation, manifest,
or graph logic.

## Quickstart

```
bastion validate-brain [PATH] [--sync] [--graph] [--state] [--links] [--structure] [--json]
bastion manifest [PATH] [--pretty]
bastion graph [PATH]
bastion emit-state [PATH] [--write] [--fail-on-drift]
```

`PATH` defaults to `.` for all four subcommands.

## `--build-stamp` and the build-provenance drift guard

`bastion` stamps its own build provenance at compile time (`build.rs`, mirroring
`core/mev/build.rs`): the git SHA it was built from, whether the source tree was dirty at build
time, and the source directory. `bastion --build-stamp` is a top-level flag (no subcommand —
it works before dispatch) that prints that stamp to stdout as JSON and exits 0:

```
$ bastion --build-stamp
{"git_sha": "a1b2c3...", "dirty": false, "source_dir": "/Users/alice/agentic-portfolio/core/bastion"}
```

**This exact three-key shape — `git_sha`, `dirty`, `source_dir` — is a pinned cross-repo
contract with `mev`'s `toolchain-freshness` check
(`mev:MV.ticket.toolchain-freshness-covers-the-writer`).** Do not add, rename, or drop a key;
mev queries this shape verbatim to detect a stale-binary write before it silently no-ops or
destroys corpus state. `dirty` is a JSON boolean (`true`/`false`) when the build-time flag was
determinable, or the literal string `"unknown"` when it could not be (e.g. no `.git/` present at
build time) — never guessed.

### The drift banner on `emit-state --write`

`emit-state --write` is the only corpus-writing path in this binary. Before it calls
`mev::emit_state`, it re-derives the *live* HEAD of the stamped source directory and compares it
against the compiled-in stamp. When they disagree — a different SHA, or the tree was dirty at
build time — that is **drift**: the running binary's provenance does not match (or cannot vouch
for) the source it is about to write from.

On drift, a loud banner is printed to **stderr**, naming both the stamped SHA and the live SHA:

```
╔══════════════════════════════════════════════════════════════════╗
║  BUILD PROVENANCE DRIFT — this bastion binary may not match the   ║
║  source tree it is about to write from.                           ║
╚══════════════════════════════════════════════════════════════════╝
the running binary was built from <stamped-sha> but the source is now at <live-sha>; rebuild before any --write run
```

**The default is warn-and-proceed** — the write still completes with exit 0. Mid-flight drift
(a binary installed from an earlier commit than the current working tree) is normal in this
fleet's day-to-day operator workflow, and blocking every `--write` on it would be the wrong
trade for interactive use.

For unattended runs (e.g. the Mac Mini's nightly `scripts/routine.sh`), opt in to a hard fail
instead: pass `--fail-on-drift` on `emit-state`, or set the `BASTION_FAIL_ON_BUILD_DRIFT`
environment variable to a truthy value (`1`, `true`, `yes`, `on`, case-insensitive). Either one
turns the same drift into a non-zero exit **before** `mev::emit_state` is called, so nothing is
written. The env var exists specifically so an HQ-owned script can refuse to write from a stale
build without bastion's own CLI arguments needing to change.

A build tree that could not be evaluated at all (no `.git/`, an unknown stamp, or a missing
source directory) is reported as `NotEvaluable`, not drift — it never prints the banner and
never hard-fails, so a `.git`-less deployment is not made permanently unwritable.

## `validate-brain`

Dispatches to one of mev's `validate_brain*` functions based on which flags are set. Flag
precedence (first match wins) is mev's own documented order:

```
--links > --structure > --state > --graph > --sync > (base OKF pass, no flags)
```

| Flag | mev function called |
|---|---|
| (none) | `mev::validate_brain` |
| `--sync` | `mev::validate_brain_sync` |
| `--graph` | `mev::validate_brain_graph` |
| `--state` | `mev::validate_brain_state` |
| `--links` | `mev::validate_brain_links` |
| `--structure` | `mev::validate_brain_structure` |

With `--json`, emits mev's machine-readable `JsonReport` envelope (via `mev::JsonReport::new`) —
byte-identical to the equivalent `mev` binary invocation. Without `--json`, prints one line per
diagnostic plus a totals summary line. Exit code is 1 when the report carries any error-severity
diagnostic, 0 otherwise (including warnings-only reports).

## `manifest`

Thin pass-through to `mev::manifest_brain`: crawls the corpus and prints the resulting
`mev::Manifest` as JSON — compact by default, indented when `--pretty` is passed.

## `graph`

Thin pass-through to `mev::graph_brain`: builds the scope:doc_id knowledge graph and prints the
`mev::GraphExport` envelope as compact JSON. There is no `--pretty` flag — this subcommand mirrors
only mev's `emit-graph` default (compact) output, not its pretty mode.

## `emit-state`

Thin pass-through to `mev::emit_state`: discovers and loads every `planning/state.json` under the
resolved brain root, plans the derived writes, and reports the planned (or, with `--write`,
applied) actions using the same diagnostic-line + summary-line shape as the other three
subcommands. Defaults to a dry run.

## Module Layout

All four handlers live in `src/brainval/mod.rs`.

| Item | Kind | Description |
|---|---|---|
| `ValidateBrainMode` | enum | `Links` / `Structure` / `State` / `Graph` / `Sync` / `Base` — which `mev::validate_brain*` function to call. |
| `select_validate_brain_mode(sync, graph, state, links, structure)` | `fn` (pure) | Flag → mode selection, mirroring mev's own precedence exactly. |
| `report_to_exit_code(report)` | `fn` (pure) | Maps a `mev::Report` to `1` (any error-severity diagnostic) or `0`. |
| `render_human(report, root)` | `fn` (pure) | One line per diagnostic + a totals summary line. |
| `render_json(validator, root, report)` | `fn` (pure) | Serializes a `mev::Report` via `mev::JsonReport::new(..).to_json()`. |
| `render_manifest_json(manifest, pretty)` | `fn` (pure) | Compact or pretty JSON serialization of a `mev::Manifest`. |
| `render_graph_json(export)` | `fn` (pure) | Compact JSON serialization of a `mev::GraphExport`. |
| `run(path, sync, graph, state, links, structure, json)` | `fn` (I/O shell) | Handler for `validate-brain`. |
| `run_manifest(path, pretty)` | `fn` (I/O shell) | Handler for `manifest`. |
| `run_graph(path)` | `fn` (I/O shell) | Handler for `graph`. |
| `run_emit_state(path, write)` | `fn` (I/O shell) | Handler for `emit-state`. |

## Degradation Paths

| Condition | Behaviour |
|---|---|
| `brain.toml` unresolvable from `path` (no ancestor has one) | `find_brain_root` returns an error; wrapped as an `anyhow` error before any `mev::*` call — no panic. |
| Report has any error-severity diagnostic | Non-zero exit (`anyhow::bail!` after printing the report), matching the existing `validate::run` pattern. |
| Report has warnings only | Exit 0. |

## Verified Parity

Parity smoke-tested against the equivalent `mev` binary invocations on the real brain corpus
(`~/agentic-portfolio`): `bastion validate-brain <root> --json` diffs
byte-identical to `mev validate-brain <root> --json`, and the same holds for `manifest`, `graph`,
and `emit-state`. See `planning/15.2-unify-cli-bastion-side/tasks.md` §Notes for the recorded
transcripts.
