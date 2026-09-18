---
type: Reference
title: Code Index — Content-Addressed Cache for `bastion code`
description: "Reference for the OID-keyed sqlite cache behind `bastion code index` / `query --json` / `status`: how blobs are keyed, the config override, self-healing, pruning, and the non-blocking git-hook warm lines."
doc_id: code-index
layer: [console]
project: bastion
status: active
keywords: [code index, cache, sqlite, tree-sitter, git hooks, warm cache, prune]
related: [code, config]
---

# Code Index — Content-Addressed Cache

`bastion code`'s query verbs (`--def` / `--refs` / `--dependents`, see
[code.md](knowledge/code.md)) used to re-parse every `.rs` file under the scan root on every
invocation — 3.6–4.3s per call on `core/engine-rs`. The code index removes that cost by keying
each file's extracted symbols and references on its **git blob OID**, so an unchanged file is
never reparsed, no matter how many times or from how many worktrees it is queried.

## Quickstart

```
bastion code index  [--rev <sha> | --staged] [--prune] [--root <DIR>] [--workspace <NAME>]
bastion code query --json [--rev <sha> | --staged] [--root <DIR>] [--workspace <NAME>]
bastion code status [--json] [--root <DIR>] [--workspace <NAME>]
```

These are nested subcommands of `bastion code`, alongside (and mutually exclusive with) the
pre-existing bare `--def`/`--refs`/`--dependents` flags — `bastion code index` and
`bastion code --def <SYMBOL>` cannot be combined in one invocation. `--root` / `--workspace`
resolve exactly as documented in [code.md](knowledge/code.md#quickstart).

### `index` — build or refresh the cache

Parses every blob in the resolved blob set that is not already cached at its current parser
version and writes its symbols/references into the index. Prints a summary line:

```
code index: <N> blobs — <hits> hits, <misses> misses, <reparses> reparses
```

- **No flags** — indexes the working tree (`git ls-files -s` plus `git hash-object` for dirty
  files).
- **`--staged`** — indexes the git index's (staged) content instead.
- **`--rev <sha>`** — indexes a historical commit's tree (`git ls-tree -r`) without checking it
  out. Mutually exclusive with `--staged`.
- **`--prune`** — after indexing, deletes cache rows for blobs unreachable from any ref
  (computed via `git rev-list --objects --all`) and reports the count removed. This is what
  reclaims rows orphaned by a bumped parser version or a deleted branch — see
  [Self-Healing and Pruning](#self-healing-and-pruning).

### `query --json` — batch-answer many queries from one warm process

Reads newline-delimited JSON query objects from stdin, one per line —
`{"def": "<name>"}`, `{"refs": "<name>"}`, or `{"dependents": "<name>"}`, mirroring the bare
flags' shape — and writes one JSON result envelope per line to stdout, in the same order. The
index is loaded once per process and reused across every line, so a caller that needs many
symbol answers (a spec linter, an implement-agent context probe) pays the parse cost once
instead of once per symbol.

```bash
$ printf '%s\n' '{"def": "SANCTIONED_STRING_TAKING_FNS"}' '{"refs": "SANCTIONED_STRING_TAKING_FNS"}' \
  | bastion code query --json
{"...def envelope..."}
{"...refs envelope..."}
```

`--rev` / `--staged` answer against a historical commit or the git index instead of the working
tree, same as `index`. The envelope shape is the same versioned JSON documented in
[brain-graph-output.md](brain-graph-output.md); `--json` is kept as an explicit flag on this verb
for symmetry with the other `code` verbs even though batch mode always emits JSON.

### `status` — cache hit/miss/reparse counters

Runs a no-op pass over the current blob set (warming the cache as a side effect, same as any
other query) and reports:

```
code status: <hits> hits, <misses> misses, <reparses> reparses — <total_rows> total rows, <distinct_blobs> distinct blobs
```

`--json` emits the same fields as a JSON object (`hits`, `misses`, `reparses`, `total_rows`,
`distinct_blobs`) instead.

## Cache Key

Each row is keyed on `(blob_oid, parser_version)`, not on file path — a file renamed without a
content change still hits the cache under its unchanged blob OID, and two files with identical
content share one row. `parser_version` is bumped whenever the tree-sitter extraction logic
changes; a query against a bumped version always misses and reparses, leaving the old-version
row in place until `--prune` removes it (see below).

## Self-Healing

The index is a pure performance optimization — it can never make an answer wrong, only slower.
Every query path assembles the blob list it needs fresh from git (working tree, `--staged`, or
`--rev`) and reparses any blob not present in the cache at the current parser version. Deleting
the index file entirely, or the whole `bastion-code/` directory, produces correct answers on the
next call — it is simply as slow as a fully cold cache, then warms back up as subsequent queries
populate it.

## Self-Healing and Pruning

Bumping the extractor's parser version orphans every row written under the old version — they
are never returned to a query (the lookup is keyed on the *current* version) but they are not
deleted automatically, since a row for a blob still reachable from some ref may be queried again
under a `--rev` pointed at that historical commit. `bastion code index --prune` removes rows for
blobs that are unreachable from **any** ref at all — computed from `git rev-list --objects --all`
— which is where both a bumped-parser-version's stale rows and a deleted branch's blobs actually
get reclaimed.

## Index Location and Config Override

The index lives at one sqlite file, shared by every worktree of the same repository:

```
$(git rev-parse --git-common-dir)/bastion-code/index.sqlite
```

Because `--git-common-dir` (not `--git-dir`) is used, every worktree of a repo shares the same
index — a query from a worktree checked out at a different commit still benefits from (and
contributes to) the same cache.

Override the location via the `[code]` table in `~/.config/bastion/config.toml`:

```toml
[code]
index_path = "/custom/path/index.sqlite"
```

`index_path` is used verbatim (relative paths resolve against the invoking process's own
current directory, not the config file's own directory — unlike `[workspaces]`/`[views]`
entries, since an index path is a per-machine cache location, not a corpus root). Omitting the
`[code]` table, or the `index_path` key within it, falls back to the git-common-dir default
above.

## Git Hook Warm Lines

Three of HQ's shared git hooks — `post-commit`, `post-checkout`, and `post-merge` — each carry a
non-blocking warm line that runs `bastion code index` in the background after the corresponding
git operation, so the next `bastion code query` in that tree answers from a warm cache instead
of paying a cold-cache reparse. This is a convenience only, never a requirement:

- `bastion` absent from `PATH` — skipped entirely, hook still exits 0.
- The index db is locked by a concurrent `bastion code` process — the warm attempt may fail
  silently; hook still exits 0.
- The warm command errors for any other reason — hook still exits 0.

The warm is backgrounded (`&`) so the hook returns immediately, run under `nice`/`ionice` when
available so it never competes with the foreground git operation for CPU/disk, and bounded by a
hand-rolled 120s watchdog (macOS's shell has no `timeout`) so a very large repo cannot warm
indefinitely in the background. None of this can block or fail the commit/checkout/merge the
hook is attached to. Source: `hooks/post-commit`, `hooks/post-checkout`, `hooks/post-merge` in
the company-brain repo root, distributed to every scaffolded repo by
`/sync-downstream-harness`.

## Symbol Coverage

The index carries every `SymbolKind` the Rust extractor emits, including `Const`, `Static`, and
`TypeAlias` (added alongside this cache so `--def`/`--refs` can answer for allowlist-shaped
consts such as engine-rs's `SANCTIONED_STRING_TAKING_FNS`, which previously returned empty). See
[code.md](knowledge/code.md#symbol-coverage) for the full kind table.

## Performance

Warm `bastion code query --json` against `core/engine-rs` (`--def SANCTIONED_STRING_TAKING_FNS`,
708 blobs already indexed) measured **0.19s** — well under the 0.5s target (D64: evidence
recorded by hand, outside this repo's gated checks, since the measurement is against a sibling
tree rather than this repo's own test suite). See
`planning/orchestration-run/runs-that-finish-cheaply/notes.md` for the full run.

## Known Gap

`--refs` (and `query --json`'s `"refs"` mode) only finds call expressions and `use` imports (see
[code.md](knowledge/code.md#from-id-rule-edge-construction)) — a `const`/`static` referenced only
by a bare identifier read (e.g. iterated in a `for` loop, not called or imported) currently
returns no results. Measured against engine-rs's own `SANCTIONED_STRING_TAKING_FNS`, whose only
use site is `for &(file_name, sanctioned) in SANCTIONED_STRING_TAKING_FNS` — `--refs` on it
returns empty even though `--def` correctly resolves the definition via the `Const` `SymbolKind`.
Extending reference extraction to bare identifier reads is out of this cache's scope (it is a
`src/brain/code.rs` extractor change, not an index-layer one) — tracked as follow-up work rather
than fixed here.

## Related

- [code.md](knowledge/code.md) — the query semantics (`--def`/`--refs`/`--dependents`), output
  format, and symbol coverage this cache accelerates.
- [brain-graph-output.md](brain-graph-output.md) — the versioned `--json` envelope shape.
- [operations/config.md](operations/config.md) — the full config file reference, including the
  `[code]` table alongside `[workspaces]`/`[views]`.
