---
type: Index
title: Knowledge Graph & Validation — Docs
description: Index of bastion's corpus surfaces — structural queries over docs and Rust source, validation, diagnostics, and the mev/bella pass-throughs.
doc_id: bastion-docs-knowledge-index
layer: [console, brain]
project: bastion
status: active
keywords: [knowledge graph, validation, okf, tree-sitter, pass-through, index]
related: [bastion-cli-docs-index, commands, tuning]
---

# Knowledge Graph & Validation

These commands treat your repo as a graph and ask structural questions of it, or check that the
graph is well-formed. None of them touch a database.

Two axes to orient by:

- **Docs or code.** [brain.md](brain.md) queries the OKF `[[link]]` graph across markdown;
  [code.md](code.md) asks the same three questions of Rust symbols via tree-sitter.
- **Query or check.** `brain` / `code` *ask*; `validate`, `assess` and `validate-brain` *judge*.

| File | What it covers |
|---|---|
| [brain.md](brain.md) | `bastion brain` — dependents / blast-radius / lineage over the OKF corpus |
| [code.md](code.md) | `bastion code` — definition / references / dependents over Rust source (`.rs` only) |
| [validate.md](validate.md) | `bastion validate` — Markdown/MDX frontmatter and link validation; non-zero exit on error |
| [assess.md](assess.md) | `bastion assess` — read-only coverage / readiness diagnostic; writes nothing |
| [brainval.md](brainval.md) | `bastion validate-brain` / `manifest` / `graph` / `emit-state` — the `mev` pass-throughs. **Flags do not compose: one per invocation.** |
| [okf.md](okf.md) | `core/okf-core` — the shared frontmatter model, parser and serializer. A library crate, **not** a subcommand |
| [docview.md](docview.md) | `bastion view` / `edit` — the `bella` terminal viewer pass-throughs |

**Which corpus gets scanned** — `--root`, `--workspace`, and the registry precedence shared by
`brain`, `code` and `momentum` — is documented once in
[tuning.md](../tuning.md#corpus-and-workspace-roots).
