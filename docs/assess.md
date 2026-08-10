---
type: Reference
title: assess — Read-Only Repo Diagnostic
description: "Reference for `bastion assess` — a read-only diagnostic over a repo's OKF corpus that computes OKF-coverage, graph-readiness, and state-readiness check families and prints a human summary or a mev-convention `--json` envelope (Phase 15, Block BA.15.9)."
doc_id: assess
layer: [console, brain]
project: bastion
status: active
keywords: [assess, diagnostic, okf-core, report engine, json envelope, graph readiness, state readiness]
related: [brain, okf, brainval, validate]
---

# assess — Read-Only Repo Diagnostic

`bastion assess <path> [--json]` is a read-only diagnostic that computes three check families over
the OKF markdown corpus rooted at `<path>` and prints either a human summary or a machine-readable
`--json` envelope. It performs **zero filesystem writes** — no file under the assessed path is ever
created, modified, or deleted.

## Usage

```
bastion assess [PATH] [--json]
```

`PATH` defaults to `.`. Pass `--json` to emit the JSON envelope instead of the human summary.

## Check families

`assess` ships three families. A fourth family — **ID convention** — is deliberately **out of
scope until Block `BA.15.6` lands**, and is intentionally **absent** from both output modes (it
never appears as a zero-count entry, in either the human sections or the JSON keys).

### OKF coverage

Parses every discovered markdown file's frontmatter (via `okf_core::parse::extract_frontmatter`)
and reports:

- `files_scanned` / `files_with_valid_frontmatter` — how many files parsed with usable frontmatter.
- `missing_required_count` / `missing_required_findings` — files whose frontmatter parsed but is
  missing one of the three required fields (`type`, `title`, `description`), one finding per
  missing field with the file path.
- `invalid_frontmatter_count` / `invalid_frontmatter_findings` — files whose frontmatter didn't
  parse at all (`NoFrontmatter`, `UnterminatedFence`, or `MalformedLine`), each reported with the
  file path (and a line number when the parser produced one).
- `optional_field_coverage` — a `field -> count` map of how many files carry each optional field
  (`doc_id`/`layer`/`project`/`status`/`keywords`/`related`) present and non-empty.

A bare `.md` file with no frontmatter at all is not silently skipped — it is reported as an
`invalid_frontmatter` finding.

### Graph readiness

Builds a `GraphArtifact` from the discovered markdown files (nodes are files with an authored
`doc_id`; files without one get a `scope:stem` leaf key) and resolves every `related:` entry via
`okf_core::graph::resolve_edge`. Reports:

- `node_count` / `edge_count`.
- `resolved_count` — edges whose target resolved to a real node in this corpus.
- `leaf_target_count` — edges whose target resolved to a leaf key (a file without an authored
  `doc_id`) — **not** counted as dangling.
- `dangling_count` / `dangling_findings` — edges whose target matched neither a node nor a leaf
  key, each reported with the qualified `scope:doc_id` that couldn't be resolved.

### State readiness

Loads `planning/state.json` under the resolved root (via `okf_core::state::load_state`) and
reports:

- `ready` — `true` only when the file loaded and neither `focus` nor `tracks[]` is empty.
- `findings` — one entry per distinct gap: no `planning/` root resolved at all, `state.json`
  absent, `state.json` present but unparseable, `focus` present but empty, or `tracks[]` present
  but empty. A healthy `state.json` produces an empty `findings` list.
- `track_count` / `block_count` — `tracks[].len()` and the block total summed across every track,
  when the file loaded (both `0` otherwise).

## Output

### Human summary

One section per family (`OKF coverage`, `Graph readiness`, `State readiness`), followed by a
`Top gaps` block that ranks every non-zero gap across the three families — most severe first,
capped at three entries — and prints `none` when the repo is clean. Severity order: invalid
frontmatter and state-load failures (critical) rank above missing required fields and dangling
edges (major), which rank above empty focus/tracks (minor).

### `--json` envelope

`bastion assess --json <path>` prints a single JSON object following the mev envelope convention —
a stable top-level shape (`assessor` + `root` + counts + per-family objects) rather than a reuse of
`mev::JsonReport`, since `assess` emits a report shape, not a flat diagnostic list:

```json
{
  "assessor": "assess",
  "root": "/path/to/repo",
  "files_scanned": 3,
  "okf": { "...": "OkfFamily fields" },
  "graph": { "...": "GraphFamily fields" },
  "state": { "...": "StateFamily fields" }
}
```

The JSON round-trips: `serde_json::from_str` on the output deserializes back into the same
`AssessReport` struct that produced it. There is no `id_convention` key anywhere in the object.

## Read-only guarantee

`assess` reads the repo's markdown files and (if present) `planning/state.json`; it never writes,
creates, or deletes anything under the assessed path. This is asserted by a test that snapshots a
fixture tree's recursive file listing and contents before and after a full run (both the human and
JSON render paths) and checks the snapshots are byte-identical.

## Deferred: ID-convention family

The ID-convention check family (validating `doc_id` naming/uniqueness conventions) is scoped to
Block `BA.15.6` and has not shipped yet. Until it lands, `assess` output carries no trace of it —
no human section, no JSON key, not even a zero count — by design, so downstream tooling and readers
never mistake "not yet implemented" for "checked and found clean."

## Module layout

All logic lives under `src/assess/`:

| Module | Role |
|---|---|
| `discover` | Context resolution — `brain.toml` / `planning/` / markdown discovery from a given path. |
| `okf` | OKF-coverage family (pure). |
| `graph` | Graph-readiness family (pure). |
| `state` | State-readiness family (pure). |
| `render` | `render_human` / `render_json` + `top_gaps` ranking (pure). |
| `run` | The CLI-facing I/O shell: reads the discovered files, calls the three family functions, and prints the result. |
