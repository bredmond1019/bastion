---
type: Reference
title: docview — bella Viewer/Editor Pass-Throughs (view / edit)
description: "Reference for `bastion view` and `bastion edit` — thin subprocess pass-throughs to the `bella` terminal markdown viewer (D14, Block BA.15.2)."
doc_id: docview
layer: [console]
project: bastion
status: active
keywords: [bella, view, edit, markdown, terminal, pass-through, subprocess]
related: [validate, brain]
---

# docview — bella Viewer/Editor Pass-Throughs

`bastion view <path>` and `bastion edit <path>` are thin pass-throughs to the `bella` terminal
markdown viewer (Phase 15, Block BA.15.2 — see `planning/decisions/D14`). No `bella` or
`bella-engine` source is touched.

## Usage

```
bastion view <PATH>
bastion edit <PATH>
```

`PATH` must be an existing file (not a directory).

## Why a subprocess, not an in-process call

`bella-engine`'s public surface (`bella_engine::markdown::render_with_edit`) returns a `Rendered`
buffer — a one-shot layout of the document, not an interactive loop. The interactive Reader/Browser
event loop lives in the `bella` app crate (`crates/bella`), but that crate builds a binary only
(`[[bin]] name = "bella"`, no `[lib]` target) — its `app`/`events`/`ui` modules are private to that
binary and cannot be imported from bastion without modifying bella's `Cargo.toml` (out of scope).

Instead, both subcommands shell out to the `bella` binary (resolved via `PATH`) with `<path>` as
its argument and inherit the controlling terminal — the same construction-vs-execution split
already used for tmux (`sessions/tmux.rs`): argument-vector construction is pure and unit-tested;
the actual spawn is a thin, smoke-tested I/O shell.

## `view` vs `edit`

bella's own `Mode` enum (`crates/bella/src/app.rs`) currently has only two interactive modes —
`Reader` and `Browser` — with no separate edit-mode CLI flag or keybinding. `bastion view` and
`bastion edit` therefore both resolve to the identical `bella <path>` invocation today. They are
kept as distinct bastion subcommands/modules so a future bella edit-mode flag has a home here
without another CLI-shape change.

## Module Layout

All logic lives in `crates/bastion/src/docview/mod.rs`.

| Item | Kind | Description |
|---|---|---|
| `BELLA_BIN` | const | `"bella"` — the binary name resolved via `PATH` (D14: bella is consumed, never vendored/forked). |
| `DocViewError` | enum | `NotFound(PathBuf)` / `IsDirectory(PathBuf)` — pure path-validation failures, before any process is spawned. |
| `validate_path(path)` | `fn` (pure) | Confirms `path` exists and is a file. Called before either subcommand shells out, so a missing/invalid path degrades cleanly with a typed error. |
| `view_args(path)` | `fn` (pure) | Returns `["bella", "<path>"]` — the argument vector for `bastion view`. |
| `edit_args(path)` | `fn` (pure) | Identical to `view_args` today (see above); kept separate for independent wiring/testability. |
| spawn shell (I/O) | `fn` | Runs `Command::new(args[0]).args(&args[1..])`, inheriting stdio, and blocks until `bella` exits; manually smoke-tested (bella has no distinct edit mode to exercise separately). |

## Degradation Paths

| Condition | Behaviour |
|---|---|
| `path` does not exist | `DocViewError::NotFound` — no process spawned. |
| `path` is a directory | `DocViewError::IsDirectory` — no process spawned. |
| `bella` not on `PATH` | Process spawn fails; mapped to a C001-style message (see `observ`). |
| `bella` exits non-zero | Mapped to a C010-style message (see `observ`). |
