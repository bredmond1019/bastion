---
type: Guideline
title: How to Tune bastion
description: The cross-cutting mechanisms — corpus roots, poll cadence, budget ceilings, secrets, logging, and the build-drift guard — that several bastion commands share, gathered in one place instead of being explained inside whichever feature happened to introduce them.
doc_id: tuning
layer: [console]
project: bastion
status: active
keywords: [tuning, configuration, poll interval, budget, secrets, degradation]
related: [commands, config, bastion-cli-docs-index]
---

# How to Tune bastion

Six mechanisms in bastion are **shared by several commands** but were each introduced by one
feature, so each used to be explained only inside that feature's doc. This page is the shared
explanation. It is deliberately short — the exhaustive variable table stays in
[config.md](operations/config.md).

## What this page is for

Read it when a knob seems to behave differently depending on which command you used, or when you
want to know what a setting affects **besides** the command you found it in. Each section names
every consumer.

## Quickstart

The four settings most people actually change:

```bash
export BASTION_POLL_INTERVAL=5          # slow every live view down to 5s
export BASTION_MAX_COST_USD=25          # refuse to dispatch past $25 of spend
export BASTION_SERVE_TOKEN="$(openssl rand -hex 32)"   # required by `bastion serve`
bastion -v --json-logs status           # structured DEBUG logs for one run
```

Nothing here is required. Every mechanism below is absent-tolerant except `BASTION_SERVE_TOKEN`,
which `bastion serve` refuses to start without.

---

## Corpus and workspace roots

**Consumers:** [`brain`](knowledge/brain.md), [`code`](knowledge/code.md),
[`momentum`](boards/momentum.md).

"Which repo am I asking about?" is answered the same way for all three, by one precedence chain:

1. `--root <DIR>` — explicit, always wins.
2. `--workspace <NAME>` (alias `--knowledge-dir`) — looks `NAME` up in the `[workspaces]` table of
   `~/.config/bastion/config.toml`.
3. `default_workspace` in that same config file.
4. The current directory.

An unknown name at step 2 or 3 is a **fatal** error (`ConfigError::UnknownWorkspace`), not a
silent fall-through to the current directory.

**`momentum` is the exception worth knowing.** It has no `--root` and no `--workspace`: it reads
*every* registered workspace, always. So an unregistered repo does not appear on the rollup, and
there is no flag that will make it appear — you register it or you do not see it.

Registry format: [config.md § Workspace registry](operations/config.md#workspace-registry).

## Poll cadence

**Consumers:** [`monitor`](workflows/monitor.md) (both the TUI and `--watch`),
[`costs --watch`](workflows/costs.md).

`BASTION_POLL_INTERVAL` (or `poll_interval` in the config file) sets how many seconds every live
view waits between refreshes. **Default: 2.**

Two behaviours here are not symmetric, and both have bitten people:

- **`0` is accepted.** `monitor` floors it at one second; `costs --watch` does **not**, so
  `BASTION_POLL_INTERVAL=0 bastion costs --watch` is a busy loop against your database.
- **An unparseable value is silently ignored**, falling back to the config file and then to `2`.
  This is the opposite of the budget caps below, where a malformed value is fatal. If a poll
  interval "isn't taking effect", suspect a typo before suspecting the code.

## Budget ceilings

**Consumers:** [`run`](workflows/run.md) (pre-dispatch gate), [`costs --watch`](workflows/costs.md)
(threshold alerts), and `bastion serve`'s `/api/costs` route.

`BASTION_MAX_TOTAL_TOKENS` and `BASTION_MAX_COST_USD` set ceilings on aggregate spend. All three
consumers call **one** shared evaluator (`costs::budget::evaluate`), so a cap cannot mean one
thing to the CLI and another to the API. Tokens are checked before cost.

| Situation | What happens |
|---|---|
| Neither set | No cap. This is a valid, unchanged configuration — not a warning. |
| Set and not breached | `run` dispatches normally. |
| Set and already breached | `run` refuses to dispatch. `bastion run --force` overrides it; `--force` is a no-op when no cap is configured. |
| Set to something unparseable | **Fatal** `ConfigError::MalformedBudgetValue`, naming the variable. Never a silent default — deliberately unlike the poll interval above. |

Comparison is `>=`, so a cap of exactly your current spend is already breached.

## Secrets, and who checks them

**Consumers:** [`serve`](serve/serve-api.md), [`abort`](workflows/abort.md),
[`notify`](serve/notify.md).

Three different secrets, in three different places. Reusing one for another is the most common
integration mistake against this binary.

| Secret | Header / mechanism | Who checks it | Needed by |
|---|---|---|---|
| `BASTION_SERVE_TOKEN` | `Authorization: Bearer …` | bastion's own routes | `bastion serve` — **mandatory**, no anonymous mode |
| `BASTION_ENGINE_API_KEY` | `X-API-Key: …` | the embedded `engine-serve` route table | `bastion abort`; without it those routes are not mounted at all |
| `BASTION_<SLUG>_BOT_TOKEN` + `_CHAT_ID` | Telegram bot credentials | Telegram | `bastion notify`, per `--bot <slug>` (default `lane`) |

Two rules that fall out of this:

- **Bot credentials come in pairs.** Exactly one of the two present is a typed
  `ConfigError::IncompleteTelegramConfig`, never a partial send. Both absent is fine — that
  transport is simply unconfigured.
- **An unknown `--bot` slug is a hard error naming both derived variables.** It never falls back
  to another bot, because a bot that silently answers for another bot is worse than a failure.

## Logging and verbosity

**Consumers:** every command. Details: [observ.md](operations/observ.md).

| Flag | Effect |
|---|---|
| `-v` / `--verbose` | INFO → DEBUG. Repeating it changes nothing. |
| `--json-logs` | Structured JSON lines instead of human text. |
| `RUST_LOG` | **Overrides `-v`** when both are set, and takes per-module filters (`RUST_LOG=bastion::db=trace`). |

Logs go to **stderr**, results to stdout — so `bastion --json-logs status 2>&1 >/dev/null | jq .`
gets you the logs alone, and an ordinary pipe still gets you the output.

## The build-provenance drift guard

**Consumers:** [`emit-state`](knowledge/brainval.md).

`bastion` compiles in the git SHA it was built from. When that has drifted from the live source
tree, `emit-state --write` is writing generated artifacts from a stale binary — which is how an
old format quietly gets rewritten across the corpus.

| Setting | Behaviour on drift |
|---|---|
| default | Loud stderr banner, **write proceeds**. |
| `--fail-on-drift`, or `BASTION_FAIL_ON_BUILD_DRIFT=1` | Non-zero exit, nothing written. |

Either form alone is sufficient — there is no precedence to reason about. The env var exists so
unattended runs (the Mac Mini's nightly `routine.sh`) can be strict without editing that
HQ-owned script's arguments. Check the stamp yourself with `bastion --build-stamp`.

## The degradation posture

Not a setting — a convention worth knowing, because it explains a lot of "why did it not error".

- **Session verbs degrade, never panic.** Missing tmux, no server, unknown session name all
  produce a message and an exit.
- **`momentum` skips a broken workspace silently.** One unreadable `status.md` does not fail the
  rollup — which is also why a missing repo produces no error to grep for.
- **`status` treats missing config as a result, not a failure.** `DATABASE_URL` unset renders as
  an unreachable row, so the command still tells you about the API.
- **Malformed budget values are the deliberate exception** and are fatal, per above.

## See also

- [config.md](operations/config.md) — the exhaustive variable and config-file reference.
- [commands.md](commands.md) — which commands exist and what each needs.
- [index.md](index.md) — the docs index.
