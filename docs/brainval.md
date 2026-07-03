---
type: Reference
title: brainval — mev Brain-Ops Pass-Throughs (validate-brain / manifest / graph / emit-state)
description: "Reference for `bastion validate-brain`, `bastion manifest`, `bastion graph`, and `bastion emit-state` — thin pass-throughs to the `mev` crate's brain-ops library functions (D15, Block BA.15.2)."
doc_id: brainval
layer: [console, brain]
project: bastion
status: active
keywords: [mev, validate-brain, manifest, graph, emit-state, OKF, pass-through]
related: [brain, validate, okf]
---

# brainval — mev Brain-Ops Pass-Throughs

`bastion validate-brain`, `bastion manifest`, `bastion graph`, and `bastion emit-state` are thin
pass-through subcommands over the `mev` crate's brain-ops library functions (Phase 15, Block
BA.15.2 — see `planning/decisions/D15`). All four resolve `brain.toml` by walking up from a
`--path` argument (default `.`) via `mev::brain::config::find_brain_root`, then dispatch straight
into the matching `mev::*` function — no bastion-side reimplementation of validation, manifest,
or graph logic.

## Usage

```
bastion validate-brain [PATH] [--sync] [--graph] [--state] [--links] [--structure] [--json]
bastion manifest [PATH] [--pretty]
bastion graph [PATH]
bastion emit-state [PATH] [--write]
```

`PATH` defaults to `.` for all four subcommands.

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

All four handlers live in `crates/bastion/src/brainval/mod.rs`.

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
(`/Users/brandon/Dev/agentic-portfolio`): `bastion validate-brain <root> --json` diffs
byte-identical to `mev validate-brain <root> --json`, and the same holds for `manifest`, `graph`,
and `emit-state`. See `planning/15.2-unify-cli-bastion-side/tasks.md` §Notes for the recorded
transcripts.
