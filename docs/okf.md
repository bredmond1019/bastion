---
type: Reference
title: okf — OKF Frontmatter Model & Serializer (write path)
description: "Reference for the `crates/bastion/src/okf` module: the OkfFrontmatter model and serialize_frontmatter, the write direction of the OKF frontmatter contract used by scaffolding (bastion init) and future backfill (adopt)."
doc_id: okf
layer: [console, factory, meta]
project: bastion
status: active
keywords: [OKF, frontmatter, serializer, write path, okf-core, scaffolding, YAML]
related: [validate, brain, bastion-product-plan]
---

# okf — OKF Frontmatter Model & Serializer

`crates/bastion/src/okf` is the **write direction** of the OKF frontmatter contract. Everywhere else in the stack
*reads* or *validates* frontmatter — `crates/bastion/src/validate/frontmatter.rs` parses and checks it, `crates/bastion/src/brain/okf.rs`
extracts `doc_id`/`title` for the graph, and `mev` validates a whole corpus. Nothing could **emit** a
compliant OKF frontmatter block until this module. That gap is what `crates/bastion/src/okf` fills.

> **Why it exists:** the [Bastion Product plan](../planning/bastion-product/plan.md) turns `bastion` into
> an adoptable "agent OS." Standing up a brain in someone else's repo (`bastion init`) and backfilling
> frontmatter onto existing docs (`bastion adopt`, later) both require *producing* correct frontmatter, not
> just checking it. `crates/bastion/src/okf` is the in-repo prototype of the future **`okf-core`** crate (plan block
> **BA.15.1**) — kept pure and dependency-light so it lifts into a workspace crate cleanly.

## What OKF frontmatter is

OKF (governed by brain decision **D27**) is the YAML `---` fenced header every doc under `docs/` and
`planning/` carries. Three fields are **required** — `type`, `title`, `description` — and six are
**optional but encouraged**: `doc_id`, `layer`, `project`, `status`, `keywords`, `related`. Populated
frontmatter is what makes the corpus queryable as a graph (see [brain.md](brain.md)) and validatable
(see [validate.md](validate.md)).

## The model — `OkfFrontmatter`

`crates/bastion/src/okf/mod.rs` defines `OkfFrontmatter`, a `serde`-derived struct mirroring the OKF contract:

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
in `crates/bastion/src/validate/frontmatter.rs`. A scalar is left bare unless it would be misparsed by YAML, in which case
it is double-quoted with `\` and `"` escaped. `needs_quote` quotes when the value:

- has significant leading/trailing whitespace,
- starts with a YAML indicator char (`# @ & * ! | > % `` ? , [ ] { } " ' -`),
- contains a structural/flow/comment/quote char (`:` `#` `[` `]` `{` `}` `,` `"` newline),
- is a bool/null-like token (`true`, `false`, `null`, `yes`, `no`, `on`, `off`, `~`), or
- parses as a number (so `title: "1.0"` stays a string, not a float).

## Round-trip guarantee

The module's tests assert that `serialize_frontmatter` output:

1. parses cleanly through bastion's own `parse_frontmatter` (all required scalars recovered), and
2. passes `validate_frontmatter` with zero required-field errors (for a fully-populated model).

This is the in-repo proxy for the end-to-end **contract check** in the plan: a repo scaffolded by
`bastion init` must survive `bastion validate-brain` (which shares the same contract via `okf-core`). If a
value ever serialized in a form the validator rejected, these tests would fail first.

## API surface

| Item | Kind | Purpose |
|---|---|---|
| `OkfFrontmatter` | struct | the OKF frontmatter model (serde `Serialize`/`Deserialize`) |
| `serialize_frontmatter(&OkfFrontmatter) -> String` | fn | emit a canonical `---`-fenced block |

Both `parse_frontmatter` (crate-internal) and `validate_frontmatter` (public) in
`crates/bastion/src/validate/frontmatter.rs` are the complementary **read/validate** side.

## Status & roadmap

Prototyped in-repo as the head start on **BA.15.1**. When the workspace consolidation lands (BA.15.0), this
module lifts into `crates/okf-core/` and becomes the single-sourced contract that `bastion`, `mev`, and the
scaffolder all depend on — so `bastion init` can never write frontmatter that `bastion validate-brain`
would reject. See the [Bastion Product plan](../planning/bastion-product/plan.md).
