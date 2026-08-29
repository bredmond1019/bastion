---
type: Index
title: Setup & Operations — Docs
description: Index of the docs for getting bastion running and keeping it observable — first-run setup, the full configuration reference, and the error/logging spine.
doc_id: bastion-docs-operations-index
layer: [console, infra]
project: bastion
status: active
keywords: [setup, configuration, environment, logging, error taxonomy, index]
related: [bastion-cli-docs-index, commands, tuning]
---

# Setup & Operations

How to get bastion connected, how to configure it, and how to read what it tells you when
something goes wrong.

Start with [setup.md](setup.md) if this is a fresh machine. Reach for [config.md](config.md) when
you know which knob you want and need its exact name.

| File | What it covers |
|---|---|
| [setup.md](setup.md) | First-run setup: provisioning the database, the env vars, and the `~/.cargo/bin` PATH trap |
| [config.md](config.md) | The full reference — every env var, every config-file key, and the precedence between them |
| [observ.md](observ.md) | The observability spine: the `C001`–`C014` error taxonomy, command events, and logging init. A library surface, **not** a subcommand |

**The short version of config.md** — the handful of knobs most people actually change — is
[tuning.md](../tuning.md).
