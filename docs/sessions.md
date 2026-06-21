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
2. Run `bastion` (or `bastion tui`) to open the interactive session dashboard.
3. Drive the sessions from the TUI — navigate with arrow keys, attach, create, send commands, or
   kill sessions without leaving the terminal.

Alternatively, use the individual verbs below for one-shot scripting or shell pipelines.

Because the surface needs no database, this works even when the orchestrator stack is down.

> For the specific flow of launching and driving **Claude Code** inside a session, see the
> task-oriented guide: [claude-code-workflow.md](claude-code-workflow.md).

## TUI Session Dashboard

Running `bastion` (bare) or `bastion tui` opens a live ratatui dashboard that lists all tmux
sessions with their activity state (derived from `pane_current_command`) and last pane output,
refreshing automatically every 2 seconds. Sessions running a non-shell foreground command show
`running (cmd)` in the STATE column; idle shells show `idle`.

### Key bindings

| Key | Action |
|---|---|
| `↑` / `↓` | Navigate session list |
| `a` | Attach to the selected session (TUI suspends; returns on detach) |
| `n` | Create a new named session (prompts for name inline) |
| `s` | Send a command to the selected session (prompts for command inline) |
| `k` | Kill the selected session |
| `q` / `Esc` | Quit the dashboard |

Inline prompts appear at the bottom of the screen. `Enter` confirms; `Esc` cancels without
making any change.

tmux errors (missing tmux, no server, unknown session) surface as a status message inside the
TUI rather than crashing the loop.

## Verb reference

### `bastion sessions`

List all tmux sessions, each with their activity state and the last line of pane output.

```bash
bastion sessions
```

The STATE column is derived from the session's foreground command (`pane_current_command`):

| State | Meaning |
|---|---|
| `running (cmd)` | A non-shell process is in the foreground (e.g. `running (cargo)`, `running (claude)`). |
| `idle` | A bare shell (`zsh`, `bash`, `sh`, `fish`) is in the foreground — no active command. |

A detached session with a live `claude` process correctly shows `running (claude)` rather
than `idle`, fixing the previous mislabeling of detached-but-busy sessions.

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

When `--dir` is provided, bastion prints an advisory **trust pre-flight** line after creating
the session. This checks whether the target directory is listed as trusted in `~/.claude.json`
(the local Claude Code trust store):

```
trust: trusted      # directory has hasTrustDialogAccepted: true in ~/.claude.json
trust: untrusted    # directory is listed but hasTrustDialogAccepted is false
trust: unknown      # ~/.claude.json absent, directory not listed, or file unreadable
```

The trust check is **advisory only** — it never blocks or fails `bastion new`. The session
is created regardless of the trust status, and `unknown` is not an error. The check is
read-only: bastion never writes to `~/.claude.json`.

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

*Block F (activity indicator + Claude trust observer) is complete. Block E (TUI session dashboard) and all earlier verbs remain available for scripting.*
