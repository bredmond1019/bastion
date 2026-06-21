---
type: Reference
title: Session Control Surface
description: Verb reference and operator workflow for bastion's tmux session-control commands (sessions / attach / new / kill / send / capture).
---

# Session Control

bastion's session-control surface manages the long-running tmux sessions on the Mac Mini that hold
Claude Code and other persistent work. It shells out to the `tmux` CLI via `std::process::Command` —
bastion **manages** these sessions; it does not run Claude Code itself.

## Guarantees

- **Database-free (D4).** This surface never opens a Postgres pool or loads `DATABASE_URL`. Every
  verb here runs with Postgres stopped and with no orchestrator dependency.
- **Synchronous (D5).** The session verbs are blocking `std::process::Command` calls — no async
  ceremony. They do one thing and return.
- **Graceful degradation.** Missing tmux, no running server, and unknown session names produce clear
  messages, never a panic (see [Error behavior](#error-behavior)).

Prerequisite: `tmux` must be installed and on `PATH`.

## Operator workflow

The intended path is hands-on from anywhere, including a phone:

1. SSH into the Mac Mini over Tailscale.
2. Run `bastion`.
3. Drive the sessions with the verbs below — list what's running, send a command into a session,
   check on it, or attach for an interactive turn.

Because the surface needs no database, this works even when the orchestrator stack is down.

## Verb reference

### `bastion sessions`

List all tmux sessions, each with its running/idle state and the last line of pane output.

```bash
bastion sessions
```

### `bastion attach <session>`

Attach to an existing session, handing the terminal to tmux. Blocks until you detach
(`Ctrl-b d`), then returns cleanly to the shell.

```bash
bastion attach work
```

### `bastion new <session> [--dir PATH]`

Create a new **detached** session. `--dir` sets the session's starting working directory.

```bash
bastion new work
bastion new build --dir ~/agentic-portfolio
```

### `bastion send <session> <cmd...>`

Send a command into a session's active pane, followed by Enter — without attaching. The command is
multi-word and needs no quoting; everything after the session name is captured as the command.

```bash
bastion send work cargo test
bastion send work git commit -m "wip"
```

The command text is sent **literally** (tmux `send-keys -l --`), so multi-word commands, commands
containing tmux key names (e.g. a literal `Enter`), and commands starting with a hyphen are all
delivered verbatim rather than being interpreted as tmux key sequences. The Enter keypress is sent
as a separate step so it registers as the Return key.

### `bastion kill <session>`

Remove a session.

```bash
bastion kill work
```

### `bastion capture <session> [--lines N]`

Print the recent pane output for a session. By default, all non-blank trailing content is shown.
Use `--lines N` to cap the output to the last `N` meaningful lines. Trailing blank/whitespace-only
lines (tmux pane-height padding) are always stripped before the line limit is applied, so `N`
counts against real content.

```bash
bastion capture work
bastion capture work --lines 50
```

## Error behavior

The surface degrades gracefully rather than panicking:

| Condition | Behavior |
|---|---|
| tmux not installed | Prints `tmux not installed — install tmux to use \`bastion <verb>\`` and exits successfully. |
| No tmux server running | Prints `no tmux server running` and exits successfully. |
| Unknown session (`attach` / `kill` / `send` / `capture`) | Prints `error: session '<name>' not found` and exits non-zero. |
| Session already exists (`new`) | Prints `error creating session '<name>': <tmux stderr>` and exits non-zero. |

---

*The verbs above are the Phase 5 Blocks A–D surface. Block E (TUI session view) is planned — see [planning/master-plan.md](../planning/master-plan.md).*
