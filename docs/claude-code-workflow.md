---
type: Guide
title: Running Claude Code through bastion
description: A hands-on walkthrough for using bastion's session surface to spin up a tmux session, launch Claude Code in it, and drive it — interactively via attach or hands-off via send/capture — including from a phone over SSH.
---

# Running Claude Code through bastion

bastion **manages** tmux sessions; it does not run Claude Code itself. The pattern is:
create a tmux session in your project directory, launch the `claude` CLI **inside** that
session, then interact with it either by attaching (full interactive control) or by
`send`/`capture` (fire a prompt, read the result — no attach needed). Because the session
surface is database-free (D4), this all works with the orchestrator stack down, and from a
phone over SSH.

> Prerequisites: `tmux` and the `claude` CLI both installed and on `PATH` on the host
> (the Mac Mini). bastion only shells out to `tmux` — it does not install or update Claude Code.

For the full verb/key reference, see [sessions.md](sessions.md). This guide is the workflow.

## The mental model

```
bastion  ──shells out to──▶  tmux  ──holds──▶  a long-running `claude` process
   │                                                   ▲
   │  new / send / capture / attach / kill             │
   └───────────────────────────────────────────────────┘
            you drive it from a terminal or phone
```

A tmux session is a durable container. Claude Code runs *inside* it and keeps running after
you detach — so you can kick off a task, disconnect, and reconnect later to the same live
session. bastion is the remote control.

## 1. Open a session and launch Claude Code

Create a detached session rooted in the project you want Claude Code to work on, then send the
`claude` command into it:

```bash
# Create a detached tmux session in the target project directory
bastion new claude-bastion --dir /Users/brandon/Dev/agentic-portfolio/bastion

# Launch Claude Code inside that session
bastion send claude-bastion claude
```

`bastion send` types the command literally and presses Enter, so `claude` starts in the
session's working directory. The session keeps running detached — Claude Code is now live
inside `claude-bastion`, waiting for input.

> `--dir` matters: Claude Code picks up the `CLAUDE.md` and `.claude/` of whatever directory
> the session started in. Point each session at the sub-project you want it scoped to.

You can run several at once — one session per project:

```bash
bastion new claude-rag   --dir /Users/brandon/Dev/agentic-portfolio/rag-engine-rs
bastion new claude-learn --dir /Users/brandon/Dev/agentic-portfolio/learn-ai
bastion send claude-rag   claude
bastion send claude-learn claude
```

## 2. Interact — two modes

### a) Hands-off: `send` a prompt, `capture` the reply

To push a prompt into a running Claude Code session **without attaching**, send the prompt
text — it lands in Claude Code's input box and Enter submits it:

```bash
# Submit a prompt to the Claude Code session
bastion send claude-bastion "summarize planning/status.md and propose the next block"

# Read back what Claude Code has printed so far
bastion capture claude-bastion --lines 60
```

`send` delivers the text literally (tmux `send-keys -l --`), so multi-word prompts, prompts
containing words like `Enter`, and prompts starting with a hyphen all arrive verbatim rather
than being interpreted as tmux key names. `capture` prints the recent pane output with trailing
blank padding stripped; `--lines N` caps it to the last N meaningful lines.

This is the phone-friendly loop: **send a prompt → wait → capture the output → send the next
prompt.** No interactive terminal required. Poll `capture` again if Claude Code is still
working when you first read it.

> Caveat: `send` submits whatever you give it as a single prompt. It is great for one-shot
> instructions and checking progress, but it is not a substitute for the interactive UI when
> Claude Code asks a mid-task question (e.g. a permission prompt or a multiple-choice). For
> those, attach.

### b) Full interactive: `attach`

To drive Claude Code directly — answer its prompts, scroll, use its slash commands, approve
tool calls — attach to the session:

```bash
bastion attach claude-bastion
```

The terminal is handed to tmux and you are inside the live Claude Code session. Work normally.
When you want to leave it running and step away, **detach** with `Ctrl-b d` — Claude Code keeps
running; you return to your shell. Re-attach any time to pick up where it left off.

## 3. Do it all from the TUI

Running `bastion` (bare) or `bastion tui` opens the dashboard — the ergonomic way to manage
several Claude Code sessions at once. It lists every session with its state and last pane line
(so you can see at a glance which Claude Code is mid-task vs. idle), refreshing every 2 seconds.

From the dashboard:

| Key | Use with Claude Code |
|---|---|
| `↑` / `↓` | Move between your `claude-*` sessions |
| `n` | Create a new session (then `s` → `claude` to launch Claude Code in it) |
| `s` | Send a prompt to the selected Claude Code session inline |
| `a` | Attach — drop into the live Claude Code UI; `Ctrl-b d` returns to the dashboard |
| `k` | Kill a finished session |
| `q` / `Esc` | Quit the dashboard |

A typical TUI loop: highlight a session, press `s` and type a prompt to dispatch work, watch
the last-line column update as Claude Code runs, then press `a` to jump in when it needs you
and `Ctrl-b d` to pop back out.

## 4. Clean up

When a session's work is done:

```bash
bastion kill claude-bastion
```

or press `k` on it in the TUI. (Quitting Claude Code with `/exit` or `Ctrl-c` leaves the tmux
session alive with a shell; `kill` removes the session entirely.)

## End-to-end example: a phone-driven session

From a phone, SSH'd into the Mac Mini over Tailscale:

```bash
# 1. Spin up a Claude Code session on the workflow engine project
bastion new wf --dir /Users/brandon/Dev/agentic-portfolio/workflow-engine-rs
bastion send wf claude

# 2. Kick off a task
bastion send wf "run the test suite and tell me what's failing"

# 3. ...wait, then check progress
bastion capture wf --lines 40

# 4. It hit a question you need to answer — attach and handle it
bastion attach wf
#    (answer in the Claude Code UI, then Ctrl-b d to detach)

# 5. Later, when it's done
bastion kill wf
```

Everything above works with Postgres and the orchestrator offline — the session surface needs
neither.

## Notes & limits

- **bastion does not run Claude Code.** It starts/sends/reads tmux sessions; the `claude`
  process lives inside them. If `claude` is not installed on the host, `bastion send <s> claude`
  will just print a shell "command not found" into the pane (visible via `capture`).
- **`send` is one prompt at a time.** It types text and presses Enter. It does not stream a
  conversation or detect when Claude Code is done — use `capture` to poll, or `attach` for a
  real exchange.
- **Interactive approvals need `attach`.** Permission prompts and multi-choice questions are
  best handled in the live UI.
- **Errors degrade gracefully.** Missing tmux, no server, or an unknown session name surface as
  a clear message (CLI) or an in-TUI status line — never a panic. See
  [sessions.md → Error behavior](sessions.md#error-behavior).

---

*Companion to [sessions.md](sessions.md) (verb + key reference). This guide is the task-oriented
walkthrough for the Claude Code use case.*
