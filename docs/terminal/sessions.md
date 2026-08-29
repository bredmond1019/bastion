---
type: Reference
title: Session Control Surface
description: Verb reference and operator workflow for bastion's tmux session-control commands (sessions / attach / new / kill / send / capture).
doc_id: sessions
layer: [console]
project: bastion
status: active
keywords: [tmux, session control, attach, send, capture, ask, TUI dashboard]
related: [claude-code-workflow, monitor, serve-api, config]
---

# Session Control

bastion's session-control surface manages the long-running tmux sessions on the Mac Mini that hold
Claude Code and other persistent work. It shells out to the `tmux` CLI via `std::process::Command` —
bastion **manages** these sessions; it does not run Claude Code itself.

## Quickstart

```bash
bastion sessions                     # what is running right now
bastion new work --dir ~/Dev/repo    # create a detached session
bastion send work cargo nextest run  # run something in it without attaching
bastion capture work --lines 40      # read the last 40 lines of its output
bastion attach work                  # take the terminal over (Ctrl-b d to detach)
bastion kill work                    # destroy it — no confirmation prompt
bastion                              # or drive all of the above from the TUI
```

| Must exist first | If it is missing |
|---|---|
| `tmux` installed and on `PATH` | Every verb prints a clear message and exits — see [Error behavior](#error-behavior). |
| A running tmux server (any session) | `bastion sessions` reports no server rather than failing. |

No database, no orchestrator, no network. This surface works with the whole stack down.

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
  in the content pane (using the `bella-engine` parser with the console's active theme, selectable
  via the `[theme]` config section — see [config.md](../operations/config.md#theme-section)). Pressing `t` opens the
  selected markdown file as a transient full-screen overlay instead of a new tab (overlay polish
  is deferred; tab machinery has been removed).
- **Tier overview (selecting a tier header — `HQ`/`core`/`side`/`client`/`portfolio`):** Routes
  the main area to that tier's `<tier>/planning/status.md`. If the file or tier directory is
  absent, the pane degrades gracefully to an empty state instead of panicking.
- **Agents · priority strip (always on):** A bottom strip, reserved and rendered under every
  spine selection (Mission Control, `HQ`, tier headers, and spaces alike), lists every live tmux
  session as one row — a themed status dot plus session name — sorted by urgency (`Blocked`/
  needs-input first, then `Working`/`Running`, then `Idle`/`Unknown`). The strip's height grows
  from 3 to a 7-line cap as session count increases, and shrinks toward 0 (never panics) when the
  frame is too short to spare the room, so it degrades gracefully on small terminals instead of
  crowding out the 1-line main area and footer.

The Kanban board view described in earlier revisions of this doc has been removed along with the
top tab bar; a dedicated Kanban view is tracked separately and remains out of scope here. Mouse
support has since returned in a different shape: click-to-select and wheel scroll now route
through a pure per-pane dispatcher (see [Mouse support](#mouse-support) below), not the old
tab-switching behavior.

### Mouse support

Mouse capture is enabled for the whole TUI session. Left-clicks and wheel scroll are routed by
comparing the click/hover coordinate against the current frame's per-pane viewport rectangles
(spine, file browser, content, agent-panel strip) via `bella_engine::geometry::point_in`:

- **Spine:** clicking a row selects it (same effect as navigating there with `↑`/`↓`/`j`/`k`).
- **File Browser (Space Overview):** clicking an entry selects it and moves focus to the Browser
  pane, matching `Enter`/arrow-key navigation.
- **Agent · priority strip:** clicking a session row jumps the spine selection to the Space whose
  slug equals that session's name (v1 slug-equality rule); a session with no matching space is a
  no-op.
- **Wheel scroll:** scrolling over the Content pane scrolls `space_overview_scroll`; scrolling over
  the Browser pane moves the file cursor; scrolling over the spine moves the selection up/down.
- Clicks outside every known pane, or before the first frame has been drawn, are no-ops — nothing
  panics.

Sub-tab click routing (a future `SubTab` bar) is not yet implemented; the dispatcher is structured
to add that arm later without a rewrite.

### Key bindings

Keyboard and mouse drive the same underlying actions — see [Mouse support](#mouse-support) above
for the click/scroll-to-pane mapping.

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

### `bastion ask` — one Claude Code turn (brain contract v0.2.0)

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
| `--out` | yes | — | Path Claude should write the answer to; bastion waits for `<out>.{nonce}.done` |
| `--dir` | no | — | Working directory if the session must be created; must be Claude-trusted |
| `--timeout` | no | `180` | Seconds to wait for `<out>.{nonce}.done` to appear |
| `--launch-cmd` | no | `claude --permission-mode bypassPermissions` | Command to start Claude if the session is cold |

**Protocol:**

1. Before doing anything else, bastion sweeps `--out`'s parent directory for stale `*.done`
   markers past the age-based GC threshold (see below) — run first, so this invocation's own
   marker cannot possibly exist yet and can never be reaped by its own sweep.
2. If the named session does not exist, bastion creates it (using `--dir` if provided) and
   launches Claude Code with `--launch-cmd`.
3. If the session exists but Claude is not the foreground process, bastion launches Claude Code.
4. bastion generates a per-invocation nonce, records the send time immediately before sending,
   then sends a trigger keystroke that instructs Claude Code to read `--prompt-file`, write its
   answer to `--out`, then write `<out>.{nonce}.done` **containing exactly the nonce** to signal
   completion.
5. bastion polls until the marker exists AND its content equals the nonce AND `--out`'s mtime
   postdates the send time (or `--timeout` expires). A marker that exists but doesn't yet satisfy
   all three conditions is neither success nor error — bastion keeps polling. On success, bastion
   never removes the marker; a GC sweep reaps markers past `max(--timeout, 1h)` on a later
   invocation instead (contract rule 3).

**Exit semantics (contract):**

- Exits `0` only when the nonce'd marker (or, during the dual-read window, the legacy bare
  marker) was observed satisfying the wait and the turn completed.
- Exits non-zero with a diagnostic message on stderr on timeout or any error.

**Dual-read window:**

For one release, `bastion ask` also accepts the v0.1.0 bare `<out>.done` marker: if it exists and
`--out`'s mtime postdates the send time, the wait is satisfied regardless of the marker's content
(v0.1.0 markers were specified as empty files, so the nonce-content check does not apply to this
form). Accepting a legacy marker emits one deprecation warning per invocation on stderr, naming
the bare path and the v0.2.0 nonce'd form. Neither marker form is ever deleted by `ask()` —
dual-read acceptance ends when the orchestrator's `BastionSessionBackend` pin advances to
v0.2.0, at which point legacy-marker support is removed as its own follow-up.

**Trust pre-flight:**

If `--dir` is provided and `bastion` must create the session, the directory is checked against
`~/.claude.json`. An untrusted directory causes `bastion ask` to fail immediately with exit 1
and a clear stderr message — no session is created. An `unknown` directory (not listed in
`~/.claude.json`) proceeds without error. The check is read-only.

**Guarantees:** DB-free (D4) — no Postgres connection. Synchronous (D5) — no async/await.

### `sessions::ask_question` — parsing an `AskUserQuestion` pane (`BA.20.B`)

A pure, I/O-free parser turning a captured pane into structured data. It is the bridge between
[`detect`](detect.md)'s `BlockedReason::AwaitingQuestion` signal and any consumer that needs the
actual question — today that is the session-QA Telegram bridge
([serve-api.md](../serve/serve-api.md) §27), which builds one button per option from it.

```rust
pub const ASK_QUESTION_MARKER: &str = "Enter to select";
pub fn parse_ask_question(screen: &str) -> Option<AskQuestionPrompt>
```

`AskQuestionPrompt` carries an optional `header` (the widget's header-chip text, e.g. `Colour`,
captured separately and never folded into `question`), the `question` text, and a
`Vec<QuestionOption>`. Each option has a 1-indexed `number`, a `label`, an optional `description`
(indented continuation lines joined with single spaces), and a `kind: OptionKind`.

`OptionKind` (`Choice` / `FreeText` / `ChatAbout`) replaces the earlier single `is_escape_hatch:
bool`, which conflated two genuinely different behaviours: the widget's trailing options are not
one escape hatch but two, an inline free-text reply and a widget-closing "chat about this" —
verified live 2026-08-14. `OptionKind` is documented in full, including the per-kind injected
keystroke sequence the session-QA bridge sends on selection, in [serve-api.md](../serve/serve-api.md) §27.

Behaviours worth knowing before you call it:

- **`None` is the common, important return.** Any screen not carrying `ASK_QUESTION_MARKER` — a
  permission dialog, ordinary shell output, an empty string — returns `None`. So does a screen that
  has the marker but no numbered options. Callers must treat `None` as "not a question", never as
  an error: injecting a digit into a yes/no approval dialog is a worse failure than sending nothing.
- **`ASK_QUESTION_MARKER` is the single source of truth for recognition**, and the same substring
  the `claude.toml` manifest rule gates on. Change one and you must change the other; they are
  deliberately narrower than the full footer (`Enter to select · ↑/↓ to navigate · Esc to cancel`)
  because the separators and arrow glyphs vary by terminal width and Claude Code version. **Do not
  widen it to include the `ctrl+g to edit in VS Code` footer fragment** — that fragment appears only
  while the free-text option is highlighted, not on every `AskUserQuestion` render, so folding it
  into the marker would make recognition depend on which option currently has focus.
- **`OptionKind` is classified STRUCTURALLY, then softly confirmed.** Whether an option sits above,
  at, or below the widget's separating horizontal rule is the reliable discriminator: the option
  immediately above the rule is `FreeText`, everything below it is `ChatAbout`, everything else is
  `Choice`. Label text (`looks_like_free_text` / `looks_like_chat_about`) is only ever a secondary,
  non-authoritative signal, logged at `debug` when it disagrees — never the primary classifier,
  because once the free-text option is filled in with typed text the placeholder wording is gone
  entirely, and a text-first classifier would misclassify it.
- **The widget region is bounded, not open-ended.** The parser walks upward from the first numbered
  option to find the widget's top boundary — the header-chip line (either checkbox glyph
  `strip_prefix` matches on) if present, else the
  nearest horizontal rule — and discards everything above it as scrollback (startup banners, MCP
  auth warnings, the operator's own prior prompt). The earlier "prose above the first numbered
  option is part of the question" rule was wrong on a real pane that had scrollback sitting above
  the widget; a real capture with no framing at all still falls back to the start-of-screen
  behaviour, unchanged.

The parser strips box-drawing borders and selection-marker glyphs before classifying, and is tested
to produce identical results for the same prompt rendered plain, bordered, marked, and hard-wrapped
at different terminal widths, and across three **real** `tmux capture-pane` fixtures (a live Claude
Code v2.1.233 session, see [detect.md](detect.md)) covering the happy path, a filled-in free-text
answer, and the selection marker sitting on a different option.

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

Full contract: [serve-api.md](../serve/serve-api.md).

---

*Block G (`bastion ask` — one Claude Code turn) is complete. Block F (activity indicator + Claude trust observer), Block E (TUI session dashboard), and all earlier verbs remain available for scripting. Block 11.B (Session REST surface) adds remote HTTP access over `bastion serve`.*
