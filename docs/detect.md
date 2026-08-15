---
type: Reference
title: bastion detect — Agent State Detection Engine
description: Reference for the pure, config-driven agent-state detection module — manifest schema, compiled types, and the detect() entry point.
doc_id: bastion-detect
layer: [console]
project: bastion
status: active
keywords: [detect, agent state, manifest, tmux, pane capture, gate, rule]
related: [bastion-cli-docs-index]
---

# bastion detect — Agent State Detection Engine

The `detect` module (`src/detect/`) classifies a captured tmux pane as one of four
agent states by evaluating a priority-ordered rule list loaded from a per-agent TOML
manifest. The entire evaluation path is pure (no I/O, no process spawns).

## Core types

### `AgentState`

Serializable enum (`snake_case`) returned by every detection.

| Variant | Meaning |
|---|---|
| `Idle` | Agent is alive but waiting for input |
| `Working` | Agent is actively processing |
| `Blocked` | Agent is blocked and needs human intervention |
| `Unknown` | No rule matched the captured screen |

`AgentState::as_str()` returns the lowercase name (`"idle"`, `"working"`, `"blocked"`, `"unknown"`).

### `AgentDetection`

Full outcome struct returned by `detect()`.

| Field | Type | Description |
|---|---|---|
| `state` | `AgentState` | Classified state |
| `visible_idle` | `bool` | Show idle UI indicator |
| `visible_blocker` | `bool` | Show blocker / needs-input UI indicator |
| `visible_working` | `bool` | Show working UI indicator |
| `skip_state_update` | `bool` | When `true`, caller must not write a new state record |
| `blocked_reason` | `Option<BlockedReason>` | Sub-classifies a `Blocked` state; `None` when the matching rule declares no `reason` |

`AgentDetection::unknown()` is the sentinel returned when no rule matches (all flags `false`,
`blocked_reason` `None`).

### `BlockedReason`

Serializable enum (`snake_case`) that narrows `AgentState::Blocked`. Added by `BA.20.A` so
consumers can tell a question apart from a yes/no approval dialog without a fifth `AgentState`
variant — `AgentState` deliberately stays four-valued and is matched exhaustively in 14 files.

| Variant | Wire string | Meaning |
|---|---|---|
| `PermissionPrompt` | `permission_prompt` | A tool-use approval dialog is on screen |
| `AwaitingQuestion` | `awaiting_question` | A Claude Code `AskUserQuestion` prompt is on screen |

`BlockedReason::as_str()` returns the wire string. The value is carried through `Session` and
surfaces on the wire as the optional `SessionDto.blocked_reason` field (absent when `None`).
`sessions::ask_question::parse_ask_question` turns an `AwaitingQuestion` pane into structured
options — see [sessions.md](sessions.md).

## Public API

```rust
pub fn detect(screen: &str, manifest: &CompiledManifest) -> AgentDetection
```

Evaluates `manifest.rules` in descending priority order against `screen`. Returns the
first matching rule's `AgentDetection`, or `AgentDetection::unknown()` on no match.

## Manifest schema (TOML)

Each agent has one TOML manifest file under `src/detect/manifests/`. Bundled manifests:
`claude.toml`, `pi.toml`.

### Top-level fields

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | Human-readable agent name |
| `rules` | array of `RuleSpec` | no | Detection rules; empty list → always `Unknown` |

### `RuleSpec` fields

| Field | Type | Default | Description |
|---|---|---|---|
| `state` | `"idle"` \| `"working"` \| `"blocked"` \| `"unknown"` | — | State to report on match |
| `gate` | `GateSpec` | — | Matcher expression |
| `priority` | i32 | 0 | Higher values evaluated first |
| `region` | `RegionSpec` | `whole` | Which screen slice to inspect |
| `visible_idle` | bool | false | UI flag |
| `visible_blocker` | bool | false | UI flag |
| `visible_working` | bool | false | UI flag |
| `skip_state_update` | bool | false | Suppress state record write |
| `reason` | `"permission_prompt"` \| `"awaiting_question"` | unset | Sub-classification carried into `AgentDetection.blocked_reason`. Omitting the key yields `None`, so every pre-existing manifest parses unchanged |

### `RegionSpec`

Selects which part of the captured pane string a rule inspects.

| TOML form | Behaviour |
|---|---|
| omitted | Entire screen (`Whole`, the default) |
| `region = { kind = "last_lines", n = 5 }` | Final `n` lines joined with `\n`; falls back to whole screen when the screen has fewer than `n` lines |

### `GateSpec`

A matcher leaf or boolean combinator. TOML inline-table syntax:

| Form | Matches when |
|---|---|
| `{ contains = "text" }` | Region contains the literal substring (case-sensitive) |
| `{ regex = "pattern" }` | Compiled regex matches anywhere in the region |
| `{ line_regex = "^>" }` | Compiled regex matches any single line of the region |
| `{ any = [gate, ...] }` | At least one child gate matches (OR) |
| `{ all = [gate, ...] }` | All child gates match (AND) |
| `{ not = gate }` | Child gate does not match (NOT) |

`contains`, `regex`, and `line_regex` may be used as any node in a combinator tree.

### Example manifest snippet

```toml
name = "claude"

# Must outrank the permission-prompt rule so the two blocked sub-states never collide.
[[rules]]
state = "blocked"
priority = 110
visible_blocker = true
reason = "awaiting_question"
gate = { contains = "Enter to select" }

[[rules]]
state = "blocked"
priority = 100
visible_blocker = true
reason = "permission_prompt"
region = { kind = "last_lines", n = 5 }
gate = { contains = "Do you want to proceed?" }

[[rules]]
state = "working"
priority = 50
visible_working = true
gate = { regex = "esc to interrupt" }

[[rules]]
state = "idle"
priority = 10
visible_idle = true
gate = { line_regex = "^> " }
```

## Compile flow

1. `parse_manifest(src: &str) -> Result<Manifest, ManifestError>` — TOML deserialize.
2. `Manifest::compile() -> Result<CompiledManifest, ManifestError>` — pre-compiles all
   `Regex` / `LineRegex` patterns and sorts rules by descending `priority`.
3. `detect(screen, &compiled_manifest)` — evaluates rules in order, returns first match.

### `ManifestError`

| Variant | Cause |
|---|---|
| `Toml(toml::de::Error)` | TOML parse or schema mismatch |
| `Regex(regex::Error)` | Invalid regex pattern in a `regex` or `line_regex` gate |

## Golden test fixtures

Test fixtures live in `src/detect/fixtures/` and are loaded via `include_str!` (zero I/O at
test time). Each `.txt` file is a captured pane snapshot. Golden tests in
`src/detect/golden_tests.rs` assert expected `AgentState`, `blocked_reason`, and flag values for
both the `claude` and `pi` manifests, including a cross-agent isolation case.

| Fixture | Expected state |
|---|---|
| `claude_awaiting_question.txt` | `Blocked`, `visible_blocker = true`, `blocked_reason = AwaitingQuestion` |
| `claude_ask_question_freetext_filled.txt` | Same as above; free-text option's label is the operator's typed reply, not the placeholder |
| `claude_ask_question_freetext_selected.txt` | Same as above; the `❯` selection marker sits on the free-text option instead of the first choice |
| `claude_blocked.txt` | `Blocked`, `visible_blocker = true`, `blocked_reason = PermissionPrompt` |
| `claude_working.txt` | `Working`, `visible_working = true` |
| `claude_idle.txt` | `Idle`, `visible_idle = true` |
| `pi_working.txt` | `Working`, `visible_working = true` |
| `pi_idle.txt` | `Idle`, `visible_idle = true` |

> **All three `AskUserQuestion` fixtures are real captures.** `claude_awaiting_question.txt` and
> its two siblings were replaced 2026-08-14 with `tmux capture-pane -p` output from a live Claude
> Code v2.1.233 session (operator email redacted before committing — bastion is a public repo).
> They cover the happy path, a filled-in free-text answer, and the selection marker sitting on the
> free-text option instead of the first choice. `sessions::ask_question`'s tests
> (`src/sessions/ask_question.rs`) run against all three and pin the widget-region bounding fix
> (discarding scrollback above the widget) and the `OptionKind` structural classification against
> real rendering, not an assumed layout.
