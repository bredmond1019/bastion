---
type: Reference
title: bastion ⇄ Orchestrator Knowledge Workspace Contract (Consumer)
description: bastion's pinned view of the orchestrator-owned knowledge-workspace contract — how each contract rule maps to bastion's Rust surface. The canonical contract lives in the Python repo.
doc_id: workspace-contract
layer: [console, brain]
project: bastion
status: active
keywords: [workspace, knowledge directory, data contract, brain, resolve_workspace_root, v1.0.0]
related: [brain, code, config, data-contract]
---

# Knowledge Workspace Contract (Consumer View)

**Pinned Contract Version: 1.0.0**

The **canonical, authoritative** contract is owned by the orchestrator:
`orchestrator/docs/workspace-contract.md`. This file is bastion's *consumer* view — it pins the
version bastion is built against and maps each contract rule to bastion's Rust surface.

> bastion's half of the convention (`BA.6.B`) shipped **before** the contract was pinned; the
> contract (brain decision **D47**) codifies bastion's shipped behavior as the binding semantics
> the Python half (`OR.C`) must match. When the canonical contract bumps, re-pin the version here
> and update the mappings.

---

## Rule mappings

### Workspace names (§2)

| Contract rule | bastion |
|---|---|
| kebab-case names; brain-family names = `brain.toml` `[[repos]].slug` | `[workspaces]` keys in `~/.config/bastion/config.toml` are free-form strings — **the operator is responsible** for using manifest slugs for brain-family entries and for not rebinding a slug to a foreign root (the contract's collision rule). Nothing in `config.rs` enforces the format |
| the name doubles as the Python `project` scoping key / future `workspace_id` | not consumed by bastion's graph reader; relevant only when correlating with orchestrator-side retrieval |

### Resolution (§3)

| Contract rule | bastion |
|---|---|
| registry: name → root + default name | `FileConfig.workspaces: Option<HashMap<String, PathBuf>>` + `default_workspace: Option<String>` (`src/config.rs`), loaded DB-free by `load_workspace_registry` from `$XDG_CONFIG_HOME/bastion/config.toml` else `~/.config/bastion/config.toml` |
| precedence: explicit root > named > default > cwd | `config::resolve_workspace_root(explicit_root, workspace_name, file)` — `--root` > `--workspace` (visible alias `--knowledge-dir`, on the `brain` and `code` subcommands, `src/cli.rs`) > `default_workspace` > `PathBuf::from(".")` |
| pure resolution — no I/O, canonicalization, or existence check | `resolve_workspace_root` is a pure function; paths returned verbatim |
| unknown name → fatal typed error | `ConfigError::UnknownWorkspace(name)` |
| name supplied but no registry → distinct fatal error | `ConfigError::NoWorkspaceRegistry` |
| registry file absent/unreadable → empty registry; malformed → load error | `load_workspace_registry`: read failure → `FileConfig::default()`; parse failure → `ConfigError::MalformedFile` |

### Corpus rules (§4)

| Contract rule | bastion |
|---|---|
| `.md`/`.mdx` only, recursive walk; skip hidden entries + `target/` | `validate::find_markdown_files` (`src/validate/mod.rs`) — also accepts a single-file root (a permitted consumer extension: a one-file corpus) |
| node id = frontmatter `doc_id` else filename stem; `title` same fallback | `brain::okf::parse_okf_node` (`src/brain/okf.rs`) |
| edges via `[[slug]]` / `[[slug\|alias]]`; unknown targets silently dropped | `brain::okf::extract_okf_links` + `BrainGraph::build` |
| empty corpus → fatal error naming the root | `brain::run` bails: "no markdown files found under '<root>' — check --root or --workspace" |

---

## Re-pin checklist (when the canonical contract bumps)

1. Read the canonical changelog; update the **Pinned Contract Version** above.
2. Update any changed rule mappings here.
3. Update affected Rust surface (`config.rs`, `cli.rs`, `validate::find_markdown_files`,
   `brain::okf`) and their tests.
4. Note it in `planning/status.md`.
