---
type: Reference
title: momentum — Cross-Repo Momentum & Metrics Rollup
description: Reference for `bastion momentum` — one table showing every registered workspace's now/next/blocked queues and ## Metrics bullets, read from each repo's planning/status.md.
doc_id: momentum
layer: [console]
project: bastion
status: active
keywords: [momentum, metrics, rollup, cross-repo, status.md, workspaces]
related: [overview, commands, config]
---

# momentum — Cross-Repo Momentum & Metrics Rollup

`bastion momentum` answers one question: **across all my repos, what is moving and what is
stuck?** It walks every workspace in your `[workspaces]` registry, reads that repo's
`planning/status.md`, and prints a single table of `now` / `next` / `blocked` plus a rolled-up
`## Metrics` section. Plain stdout — no TUI, no database, no writes.

For a single repo's work as a Kanban board, use [`bastion overview`](overview.md) instead.

## Quickstart

```bash
bastion momentum          # prints the table and exits
bastion momentum | less   # if you have many repos
```

| Must exist first | If it is missing |
|---|---|
| A `[workspaces]` table in `~/.config/bastion/config.toml` (or `$XDG_CONFIG_HOME/bastion/config.toml`) | You get the header row and `(no repos in workspace registry)`. Add entries — see [config.md § Workspace registry](../operations/config.md#workspace-registry). |
| `<workspace-root>/planning/status.md` in each registered repo, with well-formed YAML frontmatter | That repo is **silently skipped**, not an error. See [When a repo is missing from the table](#when-a-repo-is-missing-from-the-table). |

## What the output looks like

```
Repo       | Now                            | Next                           | Blocked
-----------|--------------------------------|--------------------------------|--------
bastion    | BA.21.A docs cleanup           | BA.21.B index rewrite          | —
mev        | MV.4.E emit-state              | MV.5.A lane records            | —

Metrics
-------
bastion:
  - 2043 tests green
  - coverage 81%
```

Rows are sorted by workspace name. A repo with no `## Metrics` bullets is omitted from the
metrics section (not shown as empty); if **no** repo reports metrics you get
`(no metrics reported)`.

## Where each column comes from

| Column | Source in `status.md` |
|---|---|
| `Repo` | The **registry name** you gave the workspace in `config.toml` — not a field inside the file. |
| `Now` / `Next` / `Blocked` | The `now` / `next` / `blocked` scalars in the file's YAML frontmatter (the D30 momentum queues). |
| Metrics bullets | Every `- ` line under a `## Metrics` heading, stopping at the next `## ` heading or end of file. `> ` blockquote lines and blanks inside the section are skipped. |

The frontmatter parse is shared with `bastion serve`'s repo-status route
(`src/serve/status/repo.rs`), so the CLI table and the HTTP surface cannot drift apart.

## When a repo is missing from the table

By design, a broken workspace is skipped rather than failing the whole rollup. A repo drops out
when any of these is true:

- there is no `planning/` directory under its registered root;
- `planning/status.md` is missing or unreadable;
- `status.md` has no well-formed YAML frontmatter.

There is no per-repo error message. If a repo you expect is absent, check those three in order —
`cat <root>/planning/status.md | head -20` usually settles it in one command.

## Read-only by design

`momentum` never writes `status.md`. The queues are written by `/log-work` and the momentum
generators; bastion only reads them via the `[workspaces]` registry (brain decision **D25**).

## See also

- [overview.md](overview.md) — one repo's queues, as a Kanban TUI.
- [config.md § Workspace registry](../operations/config.md#workspace-registry) — how to register a workspace.
- [commands.md](../commands.md) — every bastion subcommand in one table.
