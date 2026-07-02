---
type: Reference
title: Session Control Surface
description: Verb reference and operator workflow for bastion's tmux session-control commands (sessions / attach / new / kill / send / capture).
doc_id: sessions
layer: [console]
project: bastion
status: active
keywords: [tmux, session control, attach, send, capture, ask, TUI dashboard]
related: [claude-code-workflow, monitor, serve-api]
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

## Unified Console (TUI Dashboard)

Running `bastion` (bare) or `bastion tui` opens the unified live ratatui console. There is no top
tab bar — a single left sidebar (the **spine**) is the primary navigator, and the main area routes
on whichever spine row is selected:

- **Sidebar (the spine):** A flat, selectable list built by `spine_rows()`
  (`src/brain/spaces.rs`) over the `brain.toml` workspace tree: `◆ Mission Control` is pinned
  first, followed by the `HQ` header and its children (`learn-ai`, `base-template` — the old
  standalone `brain` leaf is collapsed into `HQ`), then the `core`/`side`/`client`/`portfolio`
  tier headers and their spaces. Tier headers and `HQ` are selectable rows, not just section
  labels. `↑`/`↓`/`j`/`k` move through the spine and **wrap** at both ends.
- **Mission Control (selecting `◆ Mission Control`):** A unified "active work" view in the main
  area. The left pane lists all live tmux sessions alongside running orchestrator workflow DAGs.
  Selecting a session displays its agent state (`Working`, `Idle`, or `Blocked`), foreground
  command, and recent output in the right detail pane. Selecting a run displays its node
  progression. All session management (attach, new, kill, send) happens here.
- **Space Overview (selecting `HQ` or a space row):** A split-pane layout with a built-in file
  browser on the left and a scrollable content pane on the right. By default, it opens the
  space's `planning/status.md`. You can browse the space's directories or preview markdown files
  in the content pane (using the `bella-engine` parser with a dark theme). Pressing `t` opens the
  selected markdown file as a transient full-screen overlay instead of a new tab (overlay polish
  is deferred; tab machinery has been removed).
- **Tier overview (selecting a tier header — `HQ`/`core`/`side`/`client`/`portfolio`):** Routes
  the main area to that tier's `<tier>/planning/status.md`. If the file or tier directory is
  absent, the pane degrades gracefully to an empty state instead of panicking.

The Kanban board view and mouse-click tab switching described in earlier revisions of this doc
have been removed along with the top tab bar; mouse support and a dedicated Kanban view are
tracked separately and are out of scope here.

### Key bindings

**Global / Navigation:**
| Key | Action |
|---|---|
| `↑`/`↓` or `j`/`k` | Move selection through the spine (wraps at both ends) |
| `q` / `Esc` | Quit the dashboard |

**Space Overview (`HQ` / space rows):**
| Key | Action |
|---|---|
| `←` / `→` | Switch focus between the file Browser and the Content pane |
| `↑` / `↓` or `j` / `k` | Navigate the file list (when Browser is focused) |
| `Enter` | Descend into a directory or load a markdown file into the Content pane |
| `Backspace` | Ascend to the parent directory in the File Browser |
| `t` | Open the selected markdown file as a full-screen overlay |
| `PageUp` / `PageDown` | Scroll the Content pane (when focused) |

**Mission Control:**
| Key | Action |
|---|---|
| `↑` / `↓` or `j` / `k` | Navigate the combined sessions/runs list |
| `a` | Attach to the selected session (TUI suspends; returns cleanly on detach) |
| `n` | Create a new named session (prompts for name inline) |
| `s` | Send a command to the selected session (prompts for command inline) |
| `k` | Kill the selected session |

Inline prompts appear at the bottom of the screen. `Enter` confirms; `Esc` cancels without making any change.

tmux errors (missing tmux, no server, unknown session) surface as a status message inside the
TUI rather than crashing the loop.

### Configuration

The console reads the project's `planning/` tree (for the Space Overview and Kanban tabs) from the
current working directory by default. Set **`BASTION_PLANNING_ROOT`** to point it at a different
project's planning directory, e.g. when running `bastion tui` from outside the project root:

```bash
BASTION_PLANNING_ROOT=/path/to/project/planning bastion tui
```

An unset or empty value falls back to `./planning` relative to the current directory.

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

### `bastion ask` — one Claude Code turn (brain contract v0.1.0)

Run a single non-interactive Claude Code turn against an interactive tmux session. This is the
stable command the Python orchestrator's `CLAUDE_CODE_SESSION` LLM provider shells out to — it
makes a Claude Code session observable from the outside without attaching.

```bash
bastion ask \
  --session <name> \
  --prompt-file <path-to-prompt> \
  --out <path-for-answer> \
  [--dir <trusted-project-dir>] \
  [--timeout 180] \
  [--launch-cmd "claude --permission-mode bypassPermissions"]
```

**Flags:**

| Flag | Required | Default | Description |
|---|---|---|---|
| `--session` | yes | — | tmux session name; created if absent |
| `--prompt-file` | yes | — | Path to a file containing the full prompt text |
| `--out` | yes | — | Path Claude should write the answer to; bastion waits for `<out>.done` |
| `--dir` | no | — | Working directory if the session must be created; must be Claude-trusted |
| `--timeout` | no | `180` | Seconds to wait for `<out>.done` to appear |
| `--launch-cmd` | no | `claude --permission-mode bypassPermissions` | Command to start Claude if the session is cold |

**Protocol:**

1. If the named session does not exist, bastion creates it (using `--dir` if provided) and
   launches Claude Code with `--launch-cmd`.
2. If the session exists but Claude is not the foreground process, bastion launches Claude Code.
3. bastion sends a fixed trigger keystroke that instructs Claude Code to read `--prompt-file`,
   write its answer to `--out`, then write `<out>.done` to signal completion.
4. bastion polls until `<out>.done` appears (or `--timeout` expires), then removes the marker
   and exits.

**Exit semantics (contract):**

- Exits `0` only when `<out>.done` was observed and the turn completed.
- Exits non-zero with a diagnostic message on stderr on timeout or any error.

**Trust pre-flight:**

If `--dir` is provided and `bastion` must create the session, the directory is checked against
`~/.claude.json`. An untrusted directory causes `bastion ask` to fail immediately with exit 1
and a clear stderr message — no session is created. An `unknown` directory (not listed in
`~/.claude.json`) proceeds without error. The check is read-only.

**Guarantees:** DB-free (D4) — no Postgres connection. Synchronous (D5) — no async/await.

## Verifying the surface

A quick manual smoke test that exercises the activity indicator and the trust pre-flight against a
live tmux server (DB-free — Postgres need not be running):

```bash
cargo build
BIN=./target/debug/bastion

# Activity indicator — idle shell vs. detached-but-running command
$BIN new smoke-idle --dir /tmp                 # bare shell
$BIN new smoke-run  --dir /tmp
$BIN send smoke-run "sleep 300"                # gives the pane a foreground command
$BIN sessions                                  # smoke-idle → idle ; smoke-run → running (sleep)

# Trust observer — pre-flight line on `new --dir`
$BIN new smoke-trusted --dir <a-trusted-project-dir>   # → trust: trusted
$BIN new smoke-unknown --dir /tmp/never-opened          # → trust: unknown (session still created)

# Cleanup
for s in smoke-idle smoke-run smoke-trusted smoke-unknown; do $BIN kill "$s"; done
$BIN sessions                                  # → no tmux server running (graceful degradation)
```

Expected results: a **detached** session running a command reports `running (<cmd>)`, not `idle`;
the trust line is advisory (the session is created regardless), and `~/.claude.json` is only ever
read, never written. To find a trusted directory for the test, pick any path whose
`projects[<dir>].hasTrustDialogAccepted` is `true` in `~/.claude.json`.

## Error behavior

The surface degrades gracefully rather than panicking:

| Condition | Behavior |
|---|---|
| tmux not installed | Prints `tmux not installed — install tmux to use \`bastion <verb>\`` and exits successfully. |
| No tmux server running | Prints `no tmux server running` and exits successfully. |
| Unknown session (`attach` / `kill` / `send` / `capture`) | Prints `error: session '<name>' not found` and exits non-zero. |
| Session already exists (`new`) | Prints `error creating session '<name>': <tmux stderr>` and exits non-zero. |

---

## Remote access via REST (bastion serve)

The same session operations available at the CLI are also exposed over HTTP for remote clients
(e.g. `bastion-ui`). `bastion serve` mounts the Session REST surface under `/api/sessions`:

| CLI verb | REST equivalent |
|---|---|
| `bastion sessions` | `GET /api/sessions` |
| `bastion capture <name>` | `GET /api/sessions/{name}/pane` |
| `bastion send <name> <cmd>` | `POST /api/sessions/{name}/send` |
| — (named-key dispatch) | `POST /api/sessions/{name}/key` |
| `bastion new <name>` | `POST /api/sessions` |
| `bastion kill <name>` | `DELETE /api/sessions/{name}` |

The `POST /api/sessions/{name}/key` endpoint uses `tmux send-keys` without the `-l` literal
flag, enabling named-key dispatch (e.g. `Escape`, `Up`, `C-c`) that is not possible via the
CLI `send` verb. All REST routes require bearer-token authentication.

Full contract: [serve-api.md](serve-api.md).

---

*Block G (`bastion ask` — one Claude Code turn) is complete. Block F (activity indicator + Claude trust observer), Block E (TUI session dashboard), and all earlier verbs remain available for scripting. Block 11.B (Session REST surface) adds remote HTTP access over `bastion serve`.*
