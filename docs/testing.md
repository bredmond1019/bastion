---
type: Guide
title: Testing bastion
description: Runbook for testing bastion — the fast loop, the full gate suite, regenerating goldens, and the hand-verification recipes nothing automated can cover.
doc_id: testing
layer: [console]
project: bastion
status: active
keywords: [testing, gates, harness, drift, hand-verification, smoke test]
related: [commands, tuning, serve-api]
---

# Testing bastion

Answers "how do I know this change is safe to ship" — from a 20-second fast loop up to the exact
recipe for the checks nothing automated can run. The gate list mirrors
[`planning/harness.json`](../planning/harness.json) exactly; if the two ever disagree, the JSON
wins — this page is a reader-friendly copy of it, not a second source of truth.

## Quickstart — the loop you run while iterating

```bash
cargo nextest run --lib --bins          # whole suite, parallel processes — use this, not `cargo test`
cargo nextest run --lib --bins <path>   # scope to one module, e.g. serve::mod::engine_mount_tests
```

Requires `cargo-nextest` on `PATH` (`brew install cargo-nextest`). `cargo test` remains the
**authoritative** full-suite gate (see below) — nextest is strictly for fast local iteration.

## The full gate suite — what CI-equivalent means here

Run every row before calling anything "done." Each is one line in
[`planning/harness.json`](../planning/harness.json)'s `validation.checks[]`; all seven are
`gates: true`.

| # | Command | What it catches |
|---|---|---|
| 1 | `cargo fmt --check` | Formatting |
| 2 | `cargo clippy --all-features --all-targets -- -D warnings` | Lints, as errors |
| 3 | `cargo test` | The full test suite — **authoritative**, never substitute nextest here |
| 4 | `cargo build --release` | The release build actually compiles |
| 5 | `scripts/check-contract-corpus-drift.sh` | A `types/contract-corpus/` golden is stale vs. the real serve handlers |
| 6 | `scripts/check-typeshare-drift.sh` | `types/serve.ts` is stale vs. `src/serve/dto.rs` |
| 7 | `scripts/check-serve-api-version.sh` | `docs/serve/serve-api.md`'s header version and §21 sentence disagree |

One command for all seven, in order (stops at the first failure):

```bash
cargo fmt --check && \
cargo clippy --all-features --all-targets -- -D warnings && \
cargo test && \
cargo build --release && \
scripts/check-contract-corpus-drift.sh && \
scripts/check-typeshare-drift.sh && \
scripts/check-serve-api-version.sh
```

**Two more run but never block a push** (`gates: false`, `cargo-audit` and `capture-scenes-{text,image}`
in `harness.json`) — informational only, safe to ignore in a hurry, worth reading when you have time:

```bash
command -v cargo-audit >/dev/null 2>&1 && cargo audit || true
cargo run --quiet --release --manifest-path ../celia/Cargo.toml -p celia-cli -- check --manifest celia.toml --tier text
```

## When a drift check fails legitimately — regenerate, don't hand-edit

A drift check failing because you *meant* to change the contract is normal. Never hand-edit the
generated file — regenerate it through the script that owns the invocation, then commit the diff
(and bump the version/Amendment Log where the doc says to):

| Drifted | Regenerate with | Then |
|---|---|---|
| `types/serve.ts` | `scripts/gen-types.sh` | Commit; re-run `scripts/check-typeshare-drift.sh` |
| `types/contract-corpus/*.json` | `BASTION_DUMP_CORPUS=1 cargo test --lib serve::contract_corpus` | Commit; bump `docs/serve/serve-api.md`'s contract-corpus section if the change is a real contract change |
| `docs/serve/serve-api.md` version mismatch | Edit the header `**Version:**` line AND §21's `The current contract is **vX.Y.Z**.` sentence together — never just one | Re-run `scripts/check-serve-api-version.sh` |

`scripts/gen-types.sh` and `scripts/gen-contract-corpus.sh` are each the **single source of
truth** for their invocation — both the doc instructions above and the drift-check scripts call
through them, so the two can never diverge. Never write the `typeshare`/dump invocation by hand
anywhere else.

## Integration tests that need a real database (opt-in, `#[ignore]`)

Seven tests in `src/db/{costs,workflows}.rs` assert against a real orchestrator Postgres instead
of a fixture and are `#[ignore]`d by default so the fast loop never needs one:

```bash
BASTION_INTEGRATION_TEST=1 cargo test -- --ignored
```

Requires `DATABASE_URL` pointed at a real orchestrator database (see
[docs/operations/setup.md](operations/setup.md)). Skipped silently (not failed) when
`BASTION_INTEGRATION_TEST` is unset — this is the expected state for `cargo test`'s ordinary run.

## Hand-verification recipes — what no automated test can cover

Some acceptance criteria are declared **un-gateable** (D64) because their evidence lives outside
this repo's own checks — another process, real hardware, a human's phone. Each recipe below is
the actual command sequence used in past runs; **run it, don't just read it**, and record what you
saw (a message id, an exit code, a screen capture) wherever the change's spec or run-record asks
for it.

### tmux session control (`sessions` / `new` / `send` / `capture` / `ask` / `attach`)

```bash
bastion new demo-test                    # create a detached session
bastion send demo-test echo hello        # send a command without attaching
bastion capture demo-test                # confirm "hello" appears in recent pane output
bastion sessions                         # confirm demo-test shows activity state
bastion kill demo-test                   # clean up
```

For `attach`, which resolves a `<repo>/<lane>` pair against the live coordination registry rather
than a raw session name (`BA.25.E`):

```bash
bastion coord register --agent-name hv-test --repo bastion --lane hvtest --roadmap <any-slug>
tmux new-session -d -s lane-bastion-hvtest
bastion attach bastion/hvtest             # confirm it resolves and hands off to a real tmux attach
bastion attach bastion/nonexistent-lane   # confirm the negative case names the registry path checked
tmux kill-session -t lane-bastion-hvtest
bastion coord release --agent-name hv-test
```

A real terminal is required for the tmux attach itself to land visibly — running this from a
non-interactive shell (a CI runner, an agent's Bash tool) fails only with `open terminal failed:
not a terminal`, which is an environment limitation, not a defect; everything up to the handoff
(registry check, session-name resolution) is still confirmed.

### `bastion serve` — HTTP/WebSocket surface

```bash
BASTION_SERVE_TOKEN=test-token cargo run -- serve --addr 127.0.0.1:4317 &
curl -s http://127.0.0.1:4317/health                                    # public, no token needed
curl -s -H "Authorization: Bearer test-token" http://127.0.0.1:4317/api/board
curl -s http://127.0.0.1:4317/api/board                                 # confirm 401 with no token
kill %1
```

Full route reference and version: [docs/serve/serve-api.md](serve/serve-api.md).

### Fleet coordination (`coord`) — lease/registry round trip

```bash
bastion coord register --agent-name test-agent --repo bastion --lane test --roadmap <any-slug>
bastion coord lease --repo bastion --lane test --agent-name test-agent --kind exclusive
bastion coord status                       # confirm the claim and lease both appear
bastion coord unlease --repo bastion
bastion coord release --agent-name test-agent
bastion coord status                       # confirm both are gone
```

### Roadmap sweep / drain — dry-run before a real fire

```bash
bastion sweep <roadmap-slug> --dry-run     # every measurement/routing step, no write
bastion drain <repo>/<lane>                # prints queue/drained/completed counts + the emit outcome verbatim
```

### Live Telegram delivery (needs the Mac Mini, real bot credentials, a phone)

Cannot be run from a dev machine or an agent sandbox — this is the recipe an operator runs on the
Mini, not something to attempt elsewhere:

```bash
# On the Mini, with BASTION_TELEGRAM_BOT_TOKEN/BASTION_TELEGRAM_CHAT_ID set and the
# engine-mounted `bastion serve` (com.brandon.engine-serve, :8090) running:
bastion notify send --text "manual smoke test"     # confirm it arrives on the phone
# To confirm the engine-dispatcher path specifically (SWEEP/ORCHESTRATION -> Telegram):
# trigger a Rust chain bail and confirm a Telegram message arrives, then record the
# escalation line and the Telegram message id as closing evidence.
```

## See also

- [docs/commands.md](commands.md) — every subcommand, one line each
- [docs/operations/setup.md](operations/setup.md) — connecting to the orchestrator's database
- [docs/serve/serve-api.md](serve/serve-api.md) — the HTTP/WebSocket contract these tests exercise
- [docs/tuning.md](tuning.md) — the knobs these tests touch (poll cadence, budget ceilings, secrets)
