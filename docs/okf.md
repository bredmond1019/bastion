---
type: Reference
title: okf-core — OKF Frontmatter Model, Parser & Serializer
description: "Reference for the okf-core workspace crate: the OkfFrontmatter model, parse_frontmatter/extract_frontmatter, and serialize_frontmatter — the single-sourced OKF frontmatter contract shared by bastion (and future consumers)."
doc_id: okf
layer: [console, factory, meta]
project: bastion
status: active
keywords: [OKF, frontmatter, serializer, parser, write path, okf-core, scaffolding, YAML]
related: [validate, brain, bastion-product-plan]
---

# okf-core — OKF Frontmatter Model, Parser & Serializer

`crates/okf-core` is the single-sourced OKF frontmatter contract: the parser (`extract_frontmatter`,
`parse_frontmatter`, `Frontmatter`, `ParseResult`) and the write-direction model + serializer
(`OkfFrontmatter`, `serialize_frontmatter`) now live together in one dependency-light workspace crate.
`crates/bastion/src/validate/frontmatter.rs` re-exports the parser and layers `validate_frontmatter` on
top of it; `crates/bastion/src/brain/okf.rs` calls `okf_core::parse_frontmatter` directly to extract
`doc_id`/`title` for the graph; `mev` validates a whole corpus against the same contract.

> **Why it exists:** the [Bastion Product plan](../planning/bastion-product/plan.md) turns `bastion` into
> an adoptable "agent OS." Standing up a brain in someone else's repo (`bastion init`) and backfilling
> frontmatter onto existing docs (`bastion adopt`, later) both require *producing* correct frontmatter, not
> just checking it. `okf-core` (plan block **BA.15.1**, after the workspace consolidation in **BA.15.0**) is
> that single source of truth — extracted from the in-repo `crates/bastion/src/okf` prototype (model +
> serializer) and the parser that previously lived embedded in `crates/bastion/src/validate/frontmatter.rs`.

## What OKF frontmatter is

OKF (governed by brain decision **D27**) is the YAML `---` fenced header every doc under `docs/` and
`planning/` carries. Three fields are **required** — `type`, `title`, `description` — and six are
**optional but encouraged**: `doc_id`, `layer`, `project`, `status`, `keywords`, `related`. Populated
frontmatter is what makes the corpus queryable as a graph (see [brain.md](brain.md)) and validatable
(see [validate.md](validate.md)).

## The parser — `extract_frontmatter` / `parse_frontmatter`

`crates/okf-core/src/parse.rs` owns the hand-rolled `---`-fence parser:

```rust
pub fn extract_frontmatter(content: &str) -> ParseResult
pub fn parse_frontmatter(content: &str) -> Option<Frontmatter>   // Ok(fm) variant, used by call sites
```

`ParseResult` is `Ok(Frontmatter) | UnterminatedFence { open_line } | MalformedLine { source_line } |
NoFrontmatter`. `Frontmatter` holds `fields: HashMap<String, (String, usize)>` (value + 1-based source
line, for error reporting) plus `open_line`/`close_line`. `crates/bastion/src/validate/frontmatter.rs`
re-exports all four items (`pub use okf_core::{Frontmatter, ParseResult, extract_frontmatter,
parse_frontmatter}`) and builds `validate_frontmatter`'s required/empty-field checks on top; `brain/okf.rs`
calls `okf_core::parse_frontmatter` directly to pull `doc_id`/`title` for the graph.

## The model — `OkfFrontmatter`

`crates/okf-core/src/frontmatter.rs` defines `OkfFrontmatter`, a `serde`-derived struct mirroring the OKF contract:

| Field | Type | Notes |
|---|---|---|
| `type_` | `Option<String>` | serialized as `type` (Rust keyword workaround) |
| `title` | `Option<String>` | required |
| `description` | `Option<String>` | required |
| `doc_id` | `Option<String>` | optional scalar |
| `layer` | `Vec<String>` | optional list (empty = absent) |
| `project` | `Option<String>` | optional scalar |
| `status` | `Option<String>` | optional scalar |
| `keywords` | `Vec<String>` | optional list |
| `related` | `Vec<String>` | optional list |

Required fields are `Option<String>` (not bare `String`) so a **partially-filled stamp** is
representable — e.g. `adopt`'s backfill can emit a block with the required keys present but empty, which
`validate_frontmatter` then flags as the "needs filling" signal.

## The write path — `serialize_frontmatter`

```rust
pub fn serialize_frontmatter(fm: &OkfFrontmatter) -> String
```

Emits a canonical `---`-fenced block (opening + closing fence + trailing newline). Rules:

- **Fixed field order:** `type, title, description, doc_id, layer, project, status, keywords, related`.
- **Required scalars are always emitted**, even when unset — as a bare `key:` (which validation reports as
  an empty field). This is intentional: serializing a `default()` produces a structurally complete but
  validation-failing stamp.
- **Optional fields are emitted only when present/non-empty.** An optional scalar that is `Some("")` is
  dropped; an empty `Vec` is dropped.
- **Lists render inline:** `layer: [brain, console]`.

### Quoting

The serializer is **hand-rolled** (no `serde_yaml` dependency) to match the house-style hand-rolled parser
that lives alongside it in `crates/okf-core/src/parse.rs`. A scalar is left bare unless it would be misparsed by YAML, in which case
it is double-quoted with `\` and `"` escaped. `needs_quote` quotes when the value:

- has significant leading/trailing whitespace,
- starts with a YAML indicator char (`# @ & * ! | > % `` ? , [ ] { } " ' -`),
- contains a structural/flow/comment/quote char (`:` `#` `[` `]` `{` `}` `,` `"` newline),
- is a bool/null-like token (`true`, `false`, `null`, `yes`, `no`, `on`, `off`, `~`), or
- parses as a number (so `title: "1.0"` stays a string, not a float).

## Round-trip guarantee

`okf-core`'s own tests (27 total, self-contained — the crate has zero dependency on `bastion`) assert that
`serialize_frontmatter` output:

1. parses cleanly through `parse_frontmatter` (all required scalars recovered, present-but-empty when
   unset), and
2. round-trips full field values end to end.

This is the proxy for the end-to-end **contract check** in the plan: a repo scaffolded by `bastion init`
must survive `bastion validate-brain`, and both now share the exact same parser/model via `okf-core`. If a
value ever serialized in a form the parser couldn't recover, these tests would fail first.

## API surface

| Item | Kind | Crate location | Purpose |
|---|---|---|---|
| `OkfFrontmatter` | struct | `okf-core` | the OKF frontmatter model (serde `Serialize`/`Deserialize`) |
| `serialize_frontmatter(&OkfFrontmatter) -> String` | fn | `okf-core` | emit a canonical `---`-fenced block |
| `Frontmatter` | struct | `okf-core` (re-exported by `bastion::validate::frontmatter`) | parsed field map + fence line numbers |
| `ParseResult` | enum | `okf-core` (re-exported by `bastion::validate::frontmatter`) | parse outcome (`Ok`/`UnterminatedFence`/`MalformedLine`/`NoFrontmatter`) |
| `extract_frontmatter(&str) -> ParseResult` | fn | `okf-core` (re-exported by `bastion::validate::frontmatter`) | parse the leading `---` block |
| `parse_frontmatter(&str) -> Option<Frontmatter>` | fn | `okf-core` (re-exported by `bastion::validate::frontmatter`) | `extract_frontmatter`'s `Ok(fm)` case as an `Option`, used by call sites |
| `validate_frontmatter(&str, &Path) -> Vec<ValidationError>` | fn | `bastion::validate::frontmatter` | required/empty-field validation built on the `okf-core` parser |

## Status & roadmap

Extraction complete (**BA.15.1**, after the workspace consolidation in **BA.15.0**): `crates/okf-core/`
is now the single-sourced contract that `bastion` depends on as a workspace crate. `mev` and the
scaffolder are expected to depend on it next, so `bastion init` can never write frontmatter that
`bastion validate-brain` would reject. See the [Bastion Product plan](../planning/bastion-product/plan.md).
