---
type: Reference
title: bastion ⇄ mev Carryover Triage Ranking Contract (Consumer)
description: bastion's pinned view of mev's versioned carryover triage ranking contract — how CarryoverRanking maps to AttentionCarryoverDto. The canonical contract lives in the mev repo.
doc_id: carryover-contract
layer: [console, brain]
project: bastion
status: active
keywords: [carryover, triage, ranking, attention, effective-priority, contract, AttentionCarryoverDto]
related: [data-contract, serve-api]
---

# Carryover Triage Ranking Contract (Consumer View)

**Pinned Contract Version: 1.0.0**

The **canonical, authoritative** contract is owned by `mev`:
`core/mev/docs/carryover-contract.md`. This file is bastion's *consumer* view — it pins the
version bastion is built against and maps `mev::CarryoverRanking` (the type mev's
`rank_carryover` returns) to bastion's `AttentionCarryoverDto` (the wire shape `GET
/api/attention` serves). This is a **different contract from a different producer** than
`docs/data-contract.md` (which pins the orchestrator's contract, on its own independent version line) — the two
version lines are independent and must never be conflated. See the D20 pattern
(`docs/data-contract.md`'s header carries the model this file follows).

`BA.ticket.carryover-triage-dto` is this contract's **first pin** — no prior bastion version
existed against it.

## Quickstart

This page is a **pin**, not a tutorial — there is nothing here to run. Use it for one of two
things:

| You are… | Go to |
|---|---|
| checking which version of the canonical contract this repo is built against | the **Pinned Contract Version** line at the top of this page |
| reacting to the canonical contract bumping | [Re-pin checklist](#re-pin-checklist-when-the-canonical-contract-bumps) |

The canonical document is owned by another repo (named just above). **Never edit the mappings
here to describe new upstream behaviour without bumping the pinned version** — a mapping that
silently describes a newer contract than the pin claims is the failure this file exists to
prevent.

---

---

## Rules bastion honours

Per the canonical contract's §6 ("Rules a consumer MUST honour"), `build_attention`
(`src/serve/handlers/attention.rs`) honours all six:

1. **Calls `mev::brain::carryover::rank_carryover`; never re-derives the ranking.** Lane
   assignment, effective-priority propagation, and sort order are mev's. `build_attention`
   projects the returned `Vec<CarryoverRanking>` in the order returned — it does not re-sort.
2. **Never authors a `blocking: bool` field.** `AttentionCarryoverDto` has no such field;
   consumers derive it from `!unmet_blocks.is_empty()` at their own boundary.
3. **Never reconciles divergent per-repo priorities.** `priority` and `effective_priority` are
   projected verbatim, per entry, with no cross-repo averaging or override.
4. **Never auto-merges suggestions.** Out of scope for this projection — `CarryoverReport
   .suggestions` and `FindingCluster` are not consumed by `build_attention`.
5. **Passes the full entry set, not a stale-filtered subset (contract §2).** The
   `carryover_stale_age`-gated membership filter that previously hid 136 of 142 entries is
   removed; every entry evaluated by `evaluate_carryover` reaches `rank_carryover`. Staleness is
   consulted only as one input to AGING-lane membership, inside mev.
6. **`stale` is read only from `carryover_stale_age`, never reimplemented.** `carryover_stale_age`
   remains the single staleness definition, called exactly once, upstream of the ranking, to
   populate each `CarryoverVerdict.stale` flag that `rank_carryover` reads.

`evaluate_carryover` is called with `allow_exec: false` — a security boundary, not a tuning knob.
`GET /api/attention` is a read-only-by-default route whose input is corpus data; a
`command_exits_zero` predicate authored in that corpus runs a shell command, so `allow_exec: true`
would let anyone who can write a `planning/state.json` achieve command execution on the serve
host. This is pinned, not incidental — see `docs/serve-api.md`'s Amendment Log entry for this
version.

---

## Field mappings

### `mev::CarryoverRanking` → `AttentionCarryoverDto` (`src/serve/dto.rs`)

| Contract (`CarryoverRanking`) | bastion (`AttentionCarryoverDto`) |
|---|---|
| `repo: String` | `repo: String` |
| `slug: String` | `slug: String` |
| `kind: String` | `kind: String` |
| `lane: TriageLane` | `lane: TriageLane` — reused verbatim from mev, not re-derived; typeshares as a `string` (`"blocking"` \| `"hot"` \| `"aging"` \| `"standing"`) since `TriageLane` lives outside typeshare's `src/serve` scan root |
| `priority: Option<u8>` | `priority: Option<u8>` — absent key when `None` |
| `effective_priority: Option<u8>` | `effective_priority: Option<u8>` — absent key when `None` |
| `age_days: Option<i64>` | `age_days: Option<i64>` — absent key when `None`; was `i64` (non-optional) pre-pin, widened because a snoozed entry or unparseable anchor now reaches the board |
| `stale: bool` | not projected onto the DTO directly — consumed upstream, by `carryover_stale_age`, to compute the `age_days`/`threshold_days` fields already on the DTO; `stale` itself has no dedicated wire field |
| `unmet_blocks: Vec<String>` | `unmet_blocks: Vec<String>` — absent key (empty array omitted) when empty |
| `clears_when_satisfied: bool` | `clears_when_satisfied: bool` — always present |
| `finding_id: Option<String>` | `finding_id: Option<String>` — absent key when `None` |
| *(not on `CarryoverRanking`)* | `text: String`, `clears_when: Option<String>`, `created: Option<String>`, `reviewed: Option<String>`, `threshold_days: i64` — carried from the source `Carryover` item / `carryover_stale_age`, unrelated to the ranking projection |

`clears_when` on the DTO is a **rendered display string**, never the typed `okf_core::ClearsWhen`
enum — see `dto::render_clears_when`. The typed enum never crosses the serve boundary.

### Inputs `build_attention` assembles for `rank_carryover`

| `rank_carryover` parameter | bastion source |
|---|---|
| `entries: &[CarryoverVerdict]` | `mev::brain::carryover::evaluate_carryover(..., allow_exec: false)` over every loaded `carryover[]` item in scope — the **full** set, not a stale-filtered subset |
| `block_priorities: &HashMap<String, u8>` | `mev::brain::state::effective_priorities(&graph, files)`, built from `mev::brain::state::build_state_graph` — the same graph construction `src/serve/handlers/board.rs` already performs |
| `block_status: &HashMap<String, Option<String>>` | `mev::brain::state::block_status_map(files)` — mev's shared implementation, **not** the local duplicate at `board.rs:218` (out of scope for this pin; filed as a carryover instead) |

---

## Re-pin checklist (when the canonical contract bumps)

1. Read `core/mev/docs/carryover-contract.md`'s changelog; update the **Pinned Contract Version**
   above.
2. Update the field mapping table above for any changed/added/removed `CarryoverRanking` field.
3. Update `AttentionCarryoverDto` (`src/serve/dto.rs`) and `build_attention`
   (`src/serve/handlers/attention.rs`) accordingly.
4. Bump `docs/serve-api.md`'s version and add an Amendment Log entry describing the wire-visible
   change.
5. Regenerate `types/serve.ts` (`scripts/gen-types.sh`) and the contract-corpus goldens
   (`scripts/gen-contract-corpus.sh`), inspecting both diffs line-by-line.
6. Note the re-pin in `planning/status.md` in **both** `core/bastion` and `core/mev`, per the
   canonical contract's §7 rule 3.

---

## Changelog (this pin)

| Pinned At | Date | Change |
|---|---|---|
| 1.0.0 | 2026-08-10 | Initial pin against mev's canonical 1.0.0 (`BA.ticket.carryover-triage-dto`). `build_attention` now calls `rank_carryover` over the full carryover entry set instead of a `carryover_stale_age`-filtered subset; `AttentionCarryoverDto` gains `lane`, `priority`, `effective_priority`, `unmet_blocks`, `finding_id`, `clears_when_satisfied`, and widens `age_days` to `Option<i64>`. Fleet-wide response size grows from ~6 to ~138 entries. See `docs/serve-api.md` v0.24 → v0.25 Amendment Log for the full wire-visible delta. |
