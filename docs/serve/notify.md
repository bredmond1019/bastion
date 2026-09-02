---
type: Reference
title: bastion notify
description: CLI entrypoint to the shared operator transport — send a fire-and-forget message or ask a gated question and wait for a resolving tap, routed to any configured bot by --bot.
doc_id: notify
layer: [console]
project: bastion
status: active
keywords: [notify, telegram, operator, ask, send, bot, cutover]
related: [config, serve-api]
---

# bastion notify

`bastion notify` exposes the existing Telegram operator transport — the same protocol code
`bastion serve` uses for its approve/reject gate and the session-QA bridge — as two CLI verbs:
`send` (fire-and-forget) and `ask` (gated question, block until answered). Both are thin I/O
shells: all protocol logic (`sendMessage` bodies, `getUpdates` long-poll, digest resolution,
callback acknowledgement) is reused verbatim from `src/serve/notify/telegram.rs` and
`engine_core::operator`. This file documents the verb; see [config.md](../operations/config.md) for the
underlying env vars.

## Quickstart

Both verbs are shell commands.

```bash
# fire-and-forget: tell the operator something, do not wait
bastion notify send --text "lane BA.21 finished, 3 docs rewritten"

# gated question: post buttons and BLOCK until the operator taps one
bastion notify ask \
  --gate-id ba21-merge \
  --summary "Merge BA.21 docs pass to main?" \
  --option yes:Merge --option no:Hold \
  --timeout-secs 300
echo $?     # 0 answered · 2 timed out · 3 stale digest · 4 another ask holds the lock
```

| Must exist first | If it is missing |
|---|---|
| `BASTION_LANE_BOT_TOKEN` + `BASTION_LANE_CHAT_ID` (for the default `--bot lane`) | Hard error naming **both** env vars. There is no silent fallback to another bot. |

`ask` blocks the calling shell for up to `--timeout-secs`. Do not put it in a path that must
return promptly, and read the exit code — a timeout is not a "no".

## `--bot <slug>` routing

Both verbs take `--bot <slug>`, defaulting to `lane`. The slug selects which bot's credentials
are used — nothing else about either verb changes. Credentials for slug `<slug>` are read from
two env vars named by one fixed pattern:

```
BASTION_<SLUG>_BOT_TOKEN
BASTION_<SLUG>_CHAT_ID
```

(`<SLUG>` is `<slug>` upper-cased.) Four slugs exist today:

| Slug | Bot | Env pair |
|---|---|---|
| `telegram` | BastionBot | `BASTION_TELEGRAM_BOT_TOKEN` / `BASTION_TELEGRAM_CHAT_ID` |
| `codesessions` | CodeSessionsBot | `BASTION_CODESESSIONS_BOT_TOKEN` / `BASTION_CODESESSIONS_CHAT_ID` |
| `lane` | OrchestrationBot (`@bastion_orchestrator_bot`) | `BASTION_LANE_BOT_TOKEN` / `BASTION_LANE_CHAT_ID` |
| `pricescout` | PriceScoutBot | `BASTION_PRICESCOUT_BOT_TOKEN` / `BASTION_PRICESCOUT_CHAT_ID` |

The slug and the bot's Telegram display name are independent: the `lane` slug names the
credential pair and the `--bot` value, while OrchestrationBot is what the bot is called in
Telegram. Renaming the bot in Telegram changes no env var and no slug.

Adding a **fourth** bot needs only a new env pair — no code change, because credential
resolution is generic over the slug (`named_bot_config` in `src/config.rs`).

An unknown or unconfigured `--bot` slug is a hard error naming both derived env vars and
listing which slugs DO have a complete pair present — never a silent fallback to another bot.

**Credential-visibility gotcha.** `BASTION_LANE_BOT_TOKEN` / `BASTION_LANE_CHAT_ID` live in
`~/.zshenv`, so they are only visible to a shell that sources it — zsh does, a bash/sh/CI runner
does not. Attribute a `bot 'lane' is not configured` error to *the shell the runner launches*, not
to the bot being missing. (`~/.zshrc` is the wrong home regardless: zsh sources it for interactive
shells only, so a non-interactive lane call would see nothing.)

## Why a third bot (OrchestrationBot)

Telegram delivers each update to exactly one `getUpdates` consumer per bot token. `bastion
serve` already runs three such consumers in the background:

- `NotifyPollLoop::run` polls `telegram` (BastionBot) for the approve/reject gate.
- `SessionQaBridge::run_outbound` polls `codesessions` (CodeSessionsBot).
- `PricescoutBridge` polls `pricescout` (PriceScoutBot) for the family's `/shop` command
  (`BA.ticket.pricescout-telegram-bot`; see [config.md](../operations/config.md#pricescoutbot-bridge-baticketpricescout-telegram-bot)).

A CLI `notify ask` process polling any of those same tokens would randomly steal the taps
those loops exist to receive. OrchestrationBot (`lane`) gives the CLI a stream nothing else consumes, so
it is the default `--bot`.

**Per-verb, per-slug hazard:**

- `notify send` is outbound-only — it never calls `getUpdates` — and is safe against **any**
  bot, at any time, including `telegram` or `codesessions`.
- `notify ask` against `telegram` or `codesessions` **while `bastion serve` is up** competes
  with that background poller for the same update stream. This is allowed but a deliberate act:
  the CLI prints a warning to stderr when `--bot` names a serve-polled slug.

**Operator note:** OrchestrationBot's credentials (`BASTION_LANE_BOT_TOKEN` /
`BASTION_LANE_CHAT_ID`) are provisioned — the `operator-lanebot-credential` gate closed
2026-08-24, and `bastion notify send --bot lane` was verified end to end.

## `bastion notify send`

Fire-and-forget: sends one plain-text message. No buttons, no digest, no poll, no lock.

```bash
bastion notify send --text "deploy finished" --bot lane
# or read the message from stdin:
echo "deploy finished" | bastion notify send --text -
```

Prints nothing on success, exits `0`. Exits `1` (naming the missing env vars) if `--bot`'s
credential pair is not fully configured.

## `bastion notify ask`

Sends a gated question with up to 3 response buttons and blocks until a resolving tap or the
timeout elapses.

```bash
bastion notify ask \
  --gate-id deploy-2026-08-23 \
  --summary "Deploy is green. Promote to prod?" \
  --option yes:Promote \
  --option no:Hold \
  --timeout-secs 300 \
  --bot lane
```

The payload is built as an `engine_core::operator::OperatorPayload` and validated through the
real `engine_core::operator::validate` contract, then through
`check_whatsapp_portability` — so the ≤3 options / ≤20-char label / ≤1024-char summary limits
are enforced by code, **before any HTTP request is attempted**, not by a skill's prose.

On a tap resolving to this gate, `ask` calls `resolve_response`, acknowledges it (answering the
callback query and stripping the inline keyboard), and prints exactly one JSON object to
stdout. A tap for a **different** gate does not terminate the ask, does not appear in its
output, and does not stop the loop — the update cursor still advances past it.

### Outcome contract

Exactly one flat JSON object is printed to stdout; `status` is total over four values:

| `status` | Fields | Exit code | Meaning |
|---|---|---|---|
| `answered` | `gate_id`, `option_key`, `decided_at` (RFC 3339) | `0` | A tap resolved to this gate and the current digest. |
| `timeout` | — | `2` | No resolving tap arrived before `--timeout-secs` elapsed. |
| `stale_digest` | `gate_id`, `option_key`, `digest`, `decided_at` | `3` | A tap resolved to this gate but against an already-re-rendered digest — the payload changed after the tap's prompt was shown, so it is not treated as an answer. |
| `busy` | — | `4` | A concurrent `notify ask` on the same `--bot` already holds the ask lock. |

Exit code `1` is reserved for unconfigured-bot, unknown-bot, and usage errors — those are
reported (on stderr) before any `AskOutcome` can be constructed, so they never appear on
stdout as a JSON object.

Every field above is asserted key-by-key in `src/notify_cli.rs`'s tests, so a rename fails a
test rather than silently breaking a caller.

## One ask at a time — the per-bot lock

Two concurrent `notify ask` processes on the **same** `--bot` would steal each other's taps at
random (the hazard is per-token, not per-process). `ask` takes an exclusive lock for its whole
duration at:

```
<lock_dir>/notify-ask-<slug>.lock
```

keyed by the `--bot` slug — two `ask`es on *different* bots never contend with each other.
`send` never takes this lock; it never reads updates and so cannot steal anything.

`lock_dir` is resolved fleet-standard, highest precedence first:

1. `--lock-dir <dir>` — explicit CLI argument.
2. `FLEET_LOCK_DIR` env var.
3. A `brain.toml` discovered by walking up from `cwd`, joined with `.fleet-locks`.
4. If no `brain.toml` is found: `cwd` joined with `.fleet-locks`.

`ask` waits up to `--timeout-secs` to acquire the lock; if it cannot, it exits `busy` (exit
code `4`) rather than polling without holding it.

## Unconfigured-bot contract

With a `--bot` slug's credential pair not fully set (or unknown), both verbs exit `1`, naming
**both** derived env vars by name and printing neither value — never a silent no-op. See
[config.md](../operations/config.md) for the full env-var reference and the underlying
`ConfigError::IncompleteTelegramConfig` shape.

## engine-rs cutover

**This verb is a bridge, not a destination.** `engine-rs` owns the whole operator seam
(`crates/engine-core/src/operator/`) and is intended to own outbound operator notification once
it is in service. When that lands, `bastion notify` becomes a thin caller over engine-rs, or is
retired. If you are looking for this verb's successor, start at `engine-rs`'s
`crates/engine-core/src/operator/`.

## Out of scope

- `bastion serve`'s three background `getUpdates` pollers (`NotifyPollLoop`, `SessionQaBridge`,
  `PricescoutBridge`) — this verb adds a CLI path alongside them and does not touch any of them.
- Free-text replies — `ask` resolves inline-keyboard taps only.
- The `engine_core::operator` payload/digest/limits contract itself — this verb only consumes
  it.
