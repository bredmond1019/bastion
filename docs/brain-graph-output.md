---
type: Reference
title: bastion brain/code JSON Output Contract
description: The documented JSON envelope for `bastion brain --json` and `bastion code --json` — all six query shapes, the empty-result case, and the guarantee that the default text output is unversioned and unchanged.
doc_id: brain-graph-output
layer: [console, brain]
project: bastion
status: active
keywords: [json, output contract, brain, code, cli, envelope, brain-graph]
related: [commands, bastion-cli-docs-index]
---

# `bastion brain` / `bastion code` JSON Output Contract

This is the **first versioned CLI output contract** in this repo. It exists so a machine
consumer — starting with `base-template`'s `brain-graph` skill (block `BT.3.F`) — has a shape to
pin against instead of re-parsing greppable text with no promise behind it.

**Scope: `--json` only.** The default (non-`--json`) text output of both verbs is **unchanged
and carries no contract**. It is not versioned, not documented here, and may keep changing
freely. Do not build a machine parser against `<relation>: <id>\tpath` or
`# no <label> results for '<id>'` — those lines are prose, not an interface.

**Installed-vs-source gap.** The `bastion` binary installed on this machine lags this source
tree. If you shell out to `bastion brain --json` or `bastion code --json` today and don't see
this shape, you are exercising the stale installed binary, not this contract. Run
`cargo install --path core/bastion` (tracked as operator gate `OP.push-bastion-and-redispatch`)
before relying on `--json` from the installed binary.

## Envelope shape

Both verbs emit one JSON object on stdout and nothing else on stdout (stderr still carries the
usual tracing/log lines). The object always carries:

- `tool` — `"brain"` or `"code"`
- `root` — the resolved corpus/source root the query ran against
- `query` — the query label (see per-verb tables below)
- one identifying field for what was queried (`node_id` for `brain`, `name` for `code`)
- `results` — an array, **empty when there are no matches** (never the text path's `# no ...
  results for '<id>'` comment line)

## `bastion brain --json`

Three queries, all returning `BrainNode`-shaped rows (`{id, path}`):

| Flag | `query` label |
|---|---|
| `--dependents <id>` | `dependent` |
| `--blast-radius <id>` | `blast-radius` |
| `--lineage <id>` | `lineage` |

Populated example (`--lineage`):

```json
{"tool": "brain", "root": "/path/to/docs", "query": "lineage", "node_id": "D17-index-md-convention", "results": [{"id": "D16-okf-concept-folder-planning", "path": "/path/to/docs/D16-okf-concept-folder-planning.md"}, {"id": "D15-okf-lowercase-doc-names", "path": "/path/to/docs/D15-okf-lowercase-doc-names.md"}]}
```

Empty-result example (`--dependents`):

```json
{"tool": "brain", "root": "/path/to/docs", "query": "dependent", "node_id": "D17-index-md-convention", "results": []}
```

For comparison, the unchanged text path for the same empty query:

```
# no dependent results for 'D17-index-md-convention'
```

## `bastion code --json`

Three queries. **The three payloads are not the same shape** — each keeps its own fields rather
than being flattened into one row type:

| Flag | `query` label | Result row shape | Source type |
|---|---|---|---|
| `--def <sym>` | `def` | `{name, kind, path, line}` | `CodeSymbol` |
| `--refs <sym>` | `refs` | `{name, path, line}` | `CodeRef` |
| `--dependents <sym>` | `dependents` | `{id, title, path}` | `BrainNode` |

Examples:

```json
{"tool": "code", "root": "src", "query": "def", "name": "query_label", "results": [{"name": "query_label", "kind": "fn", "path": "src/brain/mod.rs", "line": 33}]}
```

```json
{"tool": "code", "root": "src", "query": "refs", "name": "query_label", "results": [{"name": "query_label", "path": "src/brain/mod.rs", "line": 177}]}
```

```json
{"tool": "code", "root": "src", "query": "dependents", "name": "query_label", "results": [{"id": "mod::fn::run", "title": "run", "path": "src/validate/mod.rs"}]}
```

An empty `code` query renders the same way as `brain`'s: `"results": []}`, never
`# no def results for '<name>'`.

## `--json` and the query-mode flags

`--json` is orthogonal to each verb's required-exactly-one query group (`query-mode` on `brain`,
`code-query-mode` on `code`). `bastion brain --json` with no query flag still fails with clap's
existing required-group usage error — `--json` did not join that group and does not change its
error.

## What this contract does not cover

- No version field or negotiation mechanism. This is the first contract; if the shape needs to
  change later, that is a new decision, not something pre-built here.
- No change to exit codes or stderr degradation messages for either verb.
- No other bastion subcommand. `assess --json` and `validate-brain`'s JSON output are their own,
  pre-existing contracts, not covered by this document.
