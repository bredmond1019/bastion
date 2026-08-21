//! Contract-corpus dump harness (Task 1, spec `plan-contract-corpus-goldens`, ask A4).
//!
//! Emits checked-in golden JSON per (route × scenario) so `bastion-web` can
//! assert its e2e stub against real serve behaviour, and contract drift shows
//! up as a visible PR diff instead of a silent client bug.
//!
//! # Real dispatch only
//!
//! [`dump`] takes an already-built `actix_web::test::TestRequest` and a
//! `Service` handle obtained from the **real** app factory (`super::build_app`
//! in `src/serve/mod.rs`'s test module) and dispatches through
//! `actix_web::test::call_service`. There is no code path in this module that
//! accepts a hand-authored `serde_json::Value` as a golden — the only way to
//! produce one is to run a real request through the real handlers and the
//! real serde serializer. A hand-written golden must never appear in the
//! corpus; this module makes that structurally true rather than a convention.
//!
//! # Generate vs. verify (env-var gated)
//!
//! Mirrors the existing generate-vs-check split already used by
//! `scripts/gen-types.sh` / `scripts/check-typeshare-drift.sh`:
//!
//! - `BASTION_DUMP_CORPUS` unset (the default): **VERIFY** mode. The golden
//!   must already exist on disk; [`dump`] reads it and asserts byte-for-byte
//!   (via parsed-`Value`) equality. A normal `cargo test` run therefore
//!   *enforces* the corpus rather than silently rewriting it.
//! - `BASTION_DUMP_CORPUS=1`: **GENERATE** mode. [`dump`] (re)writes the
//!   golden to disk. This is what `scripts/gen-contract-corpus.sh` (task 5)
//!   sets before running the dump test binary.
//!
//! `BASTION_CONTRACT_CORPUS_DIR` optionally overrides the corpus root
//! (default `types/contract-corpus`, relative to the crate root) — used by
//! this module's own unit tests so they never touch the real checked-in
//! corpus.
//!
//! # Determinism (Task 2, redaction rules)
//!
//! A non-deterministic golden is worse than none — it trains reviewers to
//! ignore the diff — so every value that can legitimately vary between two
//! otherwise-identical runs is neutralized by [`redact_value`] before a
//! golden is written *or* compared. Redaction is applied uniformly on both
//! the GENERATE and VERIFY paths (inside [`build_golden`]), so the checked-in
//! corpus and every future comparison are redacted the same way.
//!
//! Rules, applied to `serde_json::Value` **strings only** — object *keys* and
//! most non-string values (numbers, bools, the run-status strings this
//! corpus exists to freeze, etc.) are never touched. **One exception** (rule
//! 5 below) is a narrowly-scoped, key-named numeric redaction added by Task
//! 4 for the Attention corpus's live-clock `age_days` field — see that rule
//! for why a blanket "values only" redaction cannot cover it:
//!
//! 1. **RFC3339 timestamps AND bare calendar dates** (`started_at`,
//!    `updated_at`, `last_touched`, `as_of`, `created`, `reviewed`, …) — any
//!    string value matching an RFC3339 datetime shape (e.g.
//!    `2026-07-31T12:00:00Z`) **or** a bare `YYYY-MM-DD` calendar date (e.g.
//!    `2026-07-31`, as `AttentionDto::as_of` and the `Carryover`/`Backlog`
//!    `created`/`reviewed` fields all wire) is replaced with the sentinel
//!    `"<TIMESTAMP>"`. Bare dates were added by Task 4: `get_attention`
//!    computes `as_of` from `chrono::Local::now().date_naive()` at request
//!    time with no fixture-clock override, so every Attention golden would
//!    otherwise drift by exactly one day, every day, forever — the single
//!    most avoidable false-drift source this corpus could ship with.
//!    Preferred long-term fix at the call site is a fixed fixture clock; this
//!    redaction is the backstop for any value that slips through un-pinned.
//! 2. **UUIDs** (`run_id`, …) — any string value matching the canonical
//!    8-4-4-4-12 hex UUID shape is replaced with `"<UUID>"`. Fixtures should
//!    still prefer seeding with a fixed UUID (never `Uuid::new_v4()`) so the
//!    *pre-redaction* value is also deterministic; this rule catches the
//!    rest.
//! 3. **Absolute temp-dir paths** — any string value that starts with *any*
//!    spelling of the process's own temp-dir root (e.g. a fixture's scratch
//!    workspace root) is replaced with `"<TMP_PATH>"`, since that path
//!    embeds a per-run/per-OS temp root that is never stable across
//!    machines or invocations. "Any spelling" is load-bearing and was
//!    tightened by `ticket-contract-corpus-uncovered-routes`: on macOS
//!    `std::env::temp_dir()` yields `/var/folders/…/T/` while
//!    `/var -> /private/var`, so a handler that canonicalizes its paths (as
//!    `mev`'s block-graph export does for `BlockGraphDto::root`) emits
//!    `/private/var/folders/…/T/` and slipped straight through the original
//!    single-prefix check. See [`temp_dir_prefixes`].
//! 4. **Key ordering** — `serde_json::Value::Object` in this crate's actual
//!    build is confirmed (by this module's
//!    `object_keys_serialize_in_sorted_order_not_insertion_order` test, not by
//!    assumption — this crate's dependency graph pulls in a build-dependency
//!    that could in principle unify the `preserve_order` feature onto
//!    `serde_json`, which would make key order insertion-order instead of
//!    sorted) to serialize object keys in **sorted** order regardless of
//!    insertion order, so two dumps of the same logical data always produce
//!    byte-identical JSON text without this module doing any extra work.
//!    Any handler-level `Vec` sourced from iterating a `HashMap` is a
//!    separate concern this module cannot fix generically — that sorting
//!    must happen at the handler/serialization layer, before the value ever
//!    reaches [`dump`].
//! 5. **`age_days` (Task 4, key-named, numeric)** — `AttentionCarryoverDto`
//!    and `AttentionBacklogDto` both wire a live-clock-derived `age_days: i64`
//!    (`today - anchor`, computed inside `get_attention` at request time).
//!    Unlike rule 1's dates, this value is a **number**, so the
//!    strings-only [`redact_value`] cannot neutralize it by pattern-matching
//!    the value alone (an arbitrary small integer is indistinguishable from
//!    any other legitimate integer field). Instead, [`redact_object_entries`]
//!    special-cases the **key** name `"age_days"` and overwrites *its* value
//!    (regardless of number) with the sentinel `0`, keeping the field's JSON
//!    type (`Number`) intact so a consumer asserting the field is numeric
//!    still passes. Every fixture in the Attention scenarios (Task 4) is
//!    still built with a *fixed* `created`/`reviewed` offset from
//!    `chrono::Local::now()` at fixture-build time (not a hardcoded
//!    calendar date) precisely so the **pre-redaction** age is itself
//!    deterministic across days — this rule is the backstop for the
//!    still-live `today` the handler itself computes independently a moment
//!    later, not a replacement for building deterministic fixtures.

use std::path::PathBuf;
use std::sync::OnceLock;

use actix_web::body::MessageBody;
use actix_web::dev::{Service, ServiceResponse};
use actix_web::test;
use regex::Regex;

/// Root directory the corpus is checked in under, relative to the crate
/// root, unless overridden by `BASTION_CONTRACT_CORPUS_DIR` (test-only knob).
const DEFAULT_CORPUS_DIR: &str = "types/contract-corpus";

/// Env var gating file writes. See module docs.
const DUMP_ENV_VAR: &str = "BASTION_DUMP_CORPUS";

/// Test-only override for the corpus root directory, so unit tests never
/// write into the real checked-in `types/contract-corpus/`.
const CORPUS_DIR_OVERRIDE_ENV_VAR: &str = "BASTION_CONTRACT_CORPUS_DIR";

/// Resolve the corpus root directory: `BASTION_CONTRACT_CORPUS_DIR` if set
/// and non-empty, else [`DEFAULT_CORPUS_DIR`].
fn corpus_dir() -> PathBuf {
    match std::env::var(CORPUS_DIR_OVERRIDE_ENV_VAR) {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(DEFAULT_CORPUS_DIR),
    }
}

/// Compute the on-disk path for a given (route, scenario) pair:
/// `<corpus_dir>/<route_name>__<scenario>.json`.
///
/// Pure — no I/O — so it is unit-testable directly.
fn golden_path(route_name: &str, scenario: &str) -> PathBuf {
    corpus_dir().join(format!("{route_name}__{scenario}.json"))
}

/// Whether we're in GENERATE mode (write) vs VERIFY mode (compare against the
/// checked-in file). Anything other than the literal value `"1"` is VERIFY
/// mode, matching the fail-safe default of not silently overwriting goldens.
fn should_write() -> bool {
    std::env::var(DUMP_ENV_VAR).as_deref() == Ok("1")
}

/// The corpus root + generate-vs-verify mode a [`dump`] call should use,
/// **taken by parameter** rather than re-read from process env at each use
/// site (CLAUDE.md rule 6: a thin I/O shell over a pure core).
///
/// This split is what makes the harness's own tests race-free: they construct
/// a `CorpusConfig` directly and never touch process env, so they cannot
/// clobber the `BASTION_*` values a concurrently-running scenario test is
/// reading. Only [`CorpusConfig::from_env`] reads env, and it does so under
/// the one process-wide env lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorpusConfig {
    /// Root directory goldens are written to / verified against.
    pub dir: PathBuf,
    /// `true` = GENERATE (write); `false` = VERIFY (compare). Fail-safe
    /// default is VERIFY.
    pub write: bool,
}

impl CorpusConfig {
    /// Snapshot the env-driven configuration (`BASTION_DUMP_CORPUS`,
    /// `BASTION_CONTRACT_CORPUS_DIR`) **under the process-wide env lock**,
    /// then release it.
    ///
    /// Releasing before returning is deliberate and load-bearing:
    /// `crate::testsupport::lock_env` is a non-reentrant `std::sync::Mutex`
    /// and [`dump`] goes on to `.await` real actix dispatch. Holding across
    /// the await would both deadlock any caller that already holds the lock
    /// and trip clippy's `await_holding_lock`. A snapshot is sufficient: any
    /// test that mutates these vars holds the same lock for the whole
    /// lifetime of its [`crate::testsupport::EnvVarGuard`], so this read can
    /// never interleave with a mutation.
    ///
    /// Callers must **not** already hold the env lock — see the lock
    /// discipline in `crate::testsupport`.
    #[must_use]
    pub fn from_env() -> Self {
        let env = crate::testsupport::lock_env();
        Self::from_env_locked(&env)
    }

    /// [`CorpusConfig::from_env`] for a caller that **already holds** the env
    /// lock. Split out precisely so that acquiring the lock twice on one
    /// thread — which would deadlock — is never necessary.
    #[must_use]
    pub fn from_env_locked(_lock: &crate::testsupport::EnvLock) -> Self {
        Self {
            dir: corpus_dir(),
            write: should_write(),
        }
    }

    /// Compute the on-disk path for a given (route, scenario) pair:
    /// `<dir>/<route_name>__<scenario>.json`. Pure — no I/O, no env.
    #[must_use]
    pub fn golden_path(&self, route_name: &str, scenario: &str) -> PathBuf {
        self.dir.join(format!("{route_name}__{scenario}.json"))
    }
}

/// Matches an RFC3339 timestamp value (e.g. `2026-07-31T12:00:00Z` or
/// `2026-07-31T12:00:00.123456+00:00`) **or** a bare `YYYY-MM-DD` calendar
/// date (e.g. `2026-07-31`, as `AttentionDto::as_of` and the
/// `Carryover`/`Backlog` `created`/`reviewed` fields wire). The time-of-day
/// portion is optional. See module docs, redaction rule 1.
fn rfc3339_timestamp_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^\d{4}-\d{2}-\d{2}(T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2}))?$")
            .expect("static RFC3339/bare-date regex must compile")
    })
}

/// Matches a canonical 8-4-4-4-12 hex UUID value (case-insensitive), e.g.
/// `run_id`. See module docs, redaction rule 2.
fn uuid_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
            .expect("static UUID regex must compile")
    })
}

/// Every spelling of the process's temp-dir root that a response body can
/// legitimately contain, for redaction rule 3.
///
/// There is more than one. On macOS `std::env::temp_dir()` returns the
/// `TMPDIR` value (`/var/folders/…/T/`), but `/var` is a symlink to
/// `/private/var`, so any handler that *canonicalizes* a fixture path — as
/// `mev`'s block-graph export does for its `root` field, via
/// `find_brain_root`/`fs::canonicalize` — emits the `/private/var/folders/…/T/`
/// spelling instead. Matching only the un-canonicalized prefix therefore let a
/// live per-run temp path through into `blocks-graph__populated.json`'s `root`,
/// which would have made that golden drift on literally every regeneration.
/// Both spellings are checked, longest first so the more specific prefix wins.
fn temp_dir_prefixes() -> &'static [String] {
    static PREFIXES: OnceLock<Vec<String>> = OnceLock::new();
    PREFIXES.get_or_init(|| {
        let tmp = std::env::temp_dir();
        let mut out: Vec<String> = Vec::new();
        if let Some(s) = tmp.to_str()
            && !s.is_empty()
        {
            out.push(s.to_owned());
        }
        if let Ok(canon) = std::fs::canonicalize(&tmp)
            && let Some(s) = canon.to_str()
            && !s.is_empty()
            && !out.iter().any(|p| p == s)
        {
            out.push(s.to_owned());
        }
        out.sort_by_key(|p| std::cmp::Reverse(p.len()));
        out
    })
}

/// Redact a single string *value* per the rules documented in the module
/// docs' "Determinism (Task 2, redaction rules)" section. Returns `None`
/// when the value should be left exactly as-is — in particular, run-status
/// strings (`budget_halted`, etc.) never match any of these patterns and so
/// always fall through unredacted, which is the point: this corpus exists to
/// freeze those strings, not scrub them.
fn redact_string_value(value: &str) -> Option<&'static str> {
    if rfc3339_timestamp_re().is_match(value) {
        return Some("<TIMESTAMP>");
    }
    if uuid_re().is_match(value) {
        return Some("<UUID>");
    }
    if temp_dir_prefixes().iter().any(|p| value.starts_with(p)) {
        return Some("<TMP_PATH>");
    }
    None
}

/// Object-key names whose *value* is redacted regardless of its JSON type,
/// per module docs redaction rule 5. Currently just `age_days` — the one
/// live-clock-derived numeric field this corpus wires
/// (`AttentionCarryoverDto`/`AttentionBacklogDto`). Deliberately a short,
/// explicit allowlist rather than "any field ending in `_days`" — the
/// *authored* `threshold_days` field is a static config value (not
/// live-clock-derived) and must NOT be redacted, or the corpus would stop
/// pinning the resolved `brain.toml` `[attention]` thresholds it exists to
/// freeze.
const VOLATILE_NUMERIC_KEYS: [&str; 1] = ["age_days"];

/// Recursively redact every string *value* in `value` in place, plus the
/// small key-named numeric exception in [`VOLATILE_NUMERIC_KEYS`] (redaction
/// rule 5). Object **keys** are never touched — only
/// [`serde_json::Value::String`] leaves (found directly, or nested inside
/// arrays/objects) are candidates for pattern-based redaction, and only when
/// they match one of [`redact_string_value`]'s patterns; the
/// `VOLATILE_NUMERIC_KEYS` exception rewrites a *value* looked up by its
/// *key* name, never the key itself.
fn redact_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(s) => {
            if let Some(redacted) = redact_string_value(s) {
                *s = redacted.to_string();
            }
        }
        serde_json::Value::Array(items) => {
            for item in items.iter_mut() {
                redact_value(item);
            }
        }
        serde_json::Value::Object(map) => {
            // Iterate `(key, value)` pairs — keys are read-only here (never
            // reassigned), which is what makes "never redact keys" true by
            // construction rather than by convention; only the paired value
            // is ever written back.
            for (key, v) in map.iter_mut() {
                if VOLATILE_NUMERIC_KEYS.contains(&key.as_str()) && v.is_number() {
                    *v = serde_json::json!(0);
                    continue;
                }
                redact_value(v);
            }
        }
        serde_json::Value::Number(_) | serde_json::Value::Bool(_) | serde_json::Value::Null => {}
    }
}

/// Parse a captured `(status_code, raw response body bytes)` pair into the
/// golden JSON value `{status_code, body}`, with all volatile values
/// redacted per [`redact_value`].
///
/// Fails loudly (panics) rather than silently skipping when the body is not
/// valid JSON — a non-JSON body means either the route doesn't serialize to
/// JSON (out of scope for this corpus) or the handler under test returned
/// something unexpected. Either way it must never be swallowed into an
/// empty/null golden that looks like a passing scenario.
fn build_golden(status_code: u16, body_bytes: &[u8]) -> serde_json::Value {
    let mut body: serde_json::Value = serde_json::from_slice(body_bytes).unwrap_or_else(|e| {
        panic!(
            "contract corpus dump: response body for status {status_code} is not valid JSON \
             ({e}); refusing to write/verify a golden from a non-JSON body. Raw body: {:?}",
            String::from_utf8_lossy(body_bytes)
        )
    });
    redact_value(&mut body);
    serde_json::json!({
        "status_code": status_code,
        "body": body,
    })
}

/// Write the golden to disk (GENERATE mode) or verify it against the
/// checked-in copy (VERIFY mode — the default, so a normal `cargo test` run
/// enforces the corpus rather than rewriting it).
fn write_or_verify(
    config: &CorpusConfig,
    route_name: &str,
    scenario: &str,
    golden: &serde_json::Value,
) {
    let path = config.golden_path(route_name, scenario);
    if config.write {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap_or_else(|e| {
                panic!("contract corpus dump: could not create {parent:?}: {e}")
            });
        }
        let pretty = serde_json::to_string_pretty(golden).unwrap_or_else(|e| {
            panic!(
                "contract corpus dump: could not serialize golden for {route_name}__{scenario}: {e}"
            )
        });
        std::fs::write(&path, format!("{pretty}\n"))
            .unwrap_or_else(|e| panic!("contract corpus dump: could not write {path:?}: {e}"));
    } else {
        let existing = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "contract corpus dump: missing checked-in golden {path:?} ({e}). Regenerate \
                 with `{DUMP_ENV_VAR}=1` (scripts/gen-contract-corpus.sh) if this is an \
                 intentional new scenario."
            )
        });
        let existing_json: serde_json::Value =
            serde_json::from_str(&existing).unwrap_or_else(|e| {
                panic!("contract corpus dump: checked-in golden {path:?} is not valid JSON: {e}")
            });
        assert_eq!(
            golden, &existing_json,
            "contract drift detected for {route_name}__{scenario} at {path:?}. If this change \
             is intentional, regenerate with `{DUMP_ENV_VAR}=1` and bump the contract version + \
             Amendment Log per docs/serve-api.md."
        );
    }
}

/// Dispatch `req` through `app` (obtained from the real app factory, e.g.
/// `super::build_app`), capture `{status_code, body}` from the real response,
/// and either write it to `<corpus_dir>/<route_name>__<scenario>.json`
/// (GENERATE mode, `BASTION_DUMP_CORPUS=1`) or verify it against the
/// checked-in copy (VERIFY mode — the default).
///
/// `app` must be a `Service` produced by `actix_web::test::init_service`
/// wrapping the real app factory — this harness only ever dispatches through
/// actix's real request/response machinery, so there is no path to register
/// a hand-authored golden.
pub async fn dump<S, B, E>(route_name: &str, scenario: &str, req: test::TestRequest, app: &S)
where
    S: Service<actix_http::Request, Response = ServiceResponse<B>, Error = E>,
    E: std::fmt::Debug,
    B: MessageBody,
{
    dump_with(&CorpusConfig::from_env(), route_name, scenario, req, app).await;
}

/// [`dump`] with the corpus root + mode supplied explicitly instead of read
/// from process env.
///
/// This is the real work; `dump` is the thin env-reading shell over it. Tests
/// of the harness itself use this form so they need no env mutation at all —
/// which is why they can no longer race the scenario tests that call `dump`.
pub async fn dump_with<S, B, E>(
    config: &CorpusConfig,
    route_name: &str,
    scenario: &str,
    req: test::TestRequest,
    app: &S,
) where
    S: Service<actix_http::Request, Response = ServiceResponse<B>, Error = E>,
    E: std::fmt::Debug,
    B: MessageBody,
{
    let resp = test::call_service(app, req.to_request()).await;
    let status_code = resp.status().as_u16();
    let body_bytes = test::read_body(resp).await;
    let golden = build_golden(status_code, &body_bytes);
    write_or_verify(config, route_name, scenario, &golden);
}

#[cfg(test)]
mod harness_tests {
    // NOTE: deliberately *not* `use super::*;` and deliberately *not*
    // `use actix_web::test;` — importing the `actix_web::test` module
    // unqualified shadows/collides with the std-prelude `#[test]` attribute
    // macro used below. `actix_web::test::*` items are referenced with their
    // full path (`actix_web::test::TestRequest`, etc.) instead.
    use super::{
        CORPUS_DIR_OVERRIDE_ENV_VAR, CorpusConfig, DUMP_ENV_VAR, build_golden, dump_with,
        golden_path, redact_string_value, redact_value, should_write,
    };
    use crate::testsupport::{EnvVarGuard, lock_env};
    use actix_web::{App, HttpResponse, web};
    use std::path::PathBuf;

    // ── Env discipline ─────────────────────────────────────────────────────
    //
    // The module-local `CORPUS_ENV_LOCK` + `EnvVarGuard` that used to live
    // here were **not enough**, and that was the bug: they serialized the
    // *writers* in this module against each other, but the ~60 corpus
    // scenario tests in the sibling `*_scenarios` modules read
    // `BASTION_DUMP_CORPUS` / `BASTION_CONTRACT_CORPUS_DIR` (via `dump`)
    // while holding no lock at all. A harness test's temp-dir override was
    // therefore routinely observed by a scenario test, which then looked for
    // its golden in the harness's scratch directory and failed with
    // "missing checked-in golden" — reproducible in roughly half of all full
    // `cargo test` runs at `--test-threads=24`, and never once under
    // `cargo nextest` (process-per-test).
    //
    // Both halves are now fixed, and the two halves are independent:
    //
    // 1. **Readers**: `dump` snapshots its config through
    //    `CorpusConfig::from_env`, which takes `crate::testsupport::lock_env`
    //    for the duration of the read.
    // 2. **Writers**: the three end-to-end `dump*` tests below no longer
    //    mutate env at all — they pass a `CorpusConfig` by parameter. Only
    //    the four tests that exist *to* exercise the env plumbing still
    //    mutate it, and they do so through the crate-wide
    //    `crate::testsupport::EnvVarGuard` under `lock_env()`.
    //
    // Any new test here that touches env must take `lock_env()` once at the
    // top and mutate only via `EnvVarGuard`. See `crate::testsupport`'s
    // module docs for the full discipline (it is not reentrant).

    #[test]
    fn golden_path_builds_expected_route_scenario_filename() {
        let env_lock = lock_env();
        let _dir_guard = EnvVarGuard::set(
            &env_lock,
            CORPUS_DIR_OVERRIDE_ENV_VAR,
            "types/contract-corpus",
        );
        let path = golden_path("runs", "empty");
        assert_eq!(
            path,
            PathBuf::from("types/contract-corpus/runs__empty.json")
        );
    }

    #[test]
    fn golden_path_respects_corpus_dir_override() {
        let env_lock = lock_env();
        let _dir_guard = EnvVarGuard::set(
            &env_lock,
            CORPUS_DIR_OVERRIDE_ENV_VAR,
            "/tmp/some-override-dir",
        );
        let path = golden_path("board", "hq");
        assert_eq!(path, PathBuf::from("/tmp/some-override-dir/board__hq.json"));
    }

    /// `CorpusConfig::from_env` is the single place the `BASTION_*` knobs are
    /// read, so it carries the env-wiring coverage the three end-to-end
    /// `dump*` tests used to provide before they moved to config-by-parameter.
    #[test]
    fn corpus_config_from_env_snapshots_both_knobs() {
        let env_lock = lock_env();
        let _dir = EnvVarGuard::set(&env_lock, CORPUS_DIR_OVERRIDE_ENV_VAR, "/tmp/cfg-probe");
        let _dump = EnvVarGuard::set(&env_lock, DUMP_ENV_VAR, "1");
        // `from_env_locked`, not `from_env`: we already hold the lock, and it
        // is not reentrant.
        let config = CorpusConfig::from_env_locked(&env_lock);
        assert_eq!(config.dir, PathBuf::from("/tmp/cfg-probe"));
        assert!(
            config.write,
            "BASTION_DUMP_CORPUS=1 must mean GENERATE mode"
        );
        assert_eq!(
            config.golden_path("runs", "empty"),
            PathBuf::from("/tmp/cfg-probe/runs__empty.json")
        );
    }

    #[test]
    fn corpus_config_golden_path_is_pure() {
        let config = CorpusConfig {
            dir: PathBuf::from("some/dir"),
            write: false,
        };
        assert_eq!(
            config.golden_path("board", "hq"),
            PathBuf::from("some/dir/board__hq.json")
        );
    }

    #[test]
    fn build_golden_captures_non_200_status_code() {
        let golden = build_golden(404, br#"{"code":"C002","message":"not found"}"#);
        assert_eq!(golden["status_code"], serde_json::json!(404));
        assert_eq!(golden["body"]["code"], serde_json::json!("C002"));
    }

    #[test]
    fn build_golden_captures_200_status_with_array_body() {
        let golden = build_golden(200, b"[]");
        assert_eq!(golden["status_code"], serde_json::json!(200));
        assert_eq!(golden["body"], serde_json::json!([]));
    }

    #[test]
    #[should_panic(expected = "is not valid JSON")]
    fn build_golden_panics_on_non_json_body() {
        build_golden(200, b"not json at all");
    }

    #[test]
    fn should_write_is_false_when_env_var_unset() {
        let env_lock = lock_env();
        let result = {
            let _unset = EnvVarGuard::unset(&env_lock, DUMP_ENV_VAR);
            should_write()
        };
        assert!(
            !result,
            "should_write() must default to VERIFY (false) when unset"
        );
    }

    #[test]
    fn should_write_is_true_only_for_exact_value_one() {
        let env_lock = lock_env();
        {
            let _guard = EnvVarGuard::set(&env_lock, DUMP_ENV_VAR, "1");
            assert!(should_write());
        }
        {
            let _guard = EnvVarGuard::set(&env_lock, DUMP_ENV_VAR, "true");
            assert!(
                !should_write(),
                "only the literal value \"1\" should enable GENERATE mode"
            );
        }
    }

    /// End-to-end: dispatch a real request through a real (if minimal) actix
    /// `Service`, in GENERATE mode, and confirm the golden lands at the
    /// expected path with the expected `{status_code, body}` shape — proving
    /// `dump` itself (not just the pure helpers) writes to the right place.
    #[actix_web::test]
    async fn dump_writes_golden_at_expected_path_in_generate_mode() {
        // No env mutation and no lock: the config is passed by parameter, so
        // this test cannot race the scenario tests that read env via `dump`.
        let tmp_dir = crate::testsupport::unique_temp_dir("bastion-contract-corpus-test-generate");
        let config = CorpusConfig {
            dir: tmp_dir.clone(),
            write: true,
        };

        let app = actix_web::test::init_service(App::new().route(
            "/x",
            web::get().to(|| async {
                HttpResponse::NotFound().json(serde_json::json!({"code": "C002"}))
            }),
        ))
        .await;

        let req = actix_web::test::TestRequest::get().uri("/x");
        dump_with(&config, "widget", "not-found", req, &app).await;

        let written_path = tmp_dir.join("widget__not-found.json");
        assert!(written_path.exists(), "expected golden at {written_path:?}");
        let contents = std::fs::read_to_string(&written_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(parsed["status_code"], serde_json::json!(404));
        assert_eq!(parsed["body"]["code"], serde_json::json!("C002"));

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    /// VERIFY mode (the default for a normal `cargo test` run) must fail
    /// loudly — not silently skip — when the checked-in golden is missing.
    #[actix_web::test]
    #[should_panic(expected = "missing checked-in golden")]
    async fn dump_verify_mode_fails_loudly_when_golden_missing() {
        // No env mutation and no lock — see the GENERATE-mode sibling above.
        let tmp_dir =
            crate::testsupport::unique_temp_dir("bastion-contract-corpus-test-verify-missing");
        let _ = std::fs::create_dir_all(&tmp_dir);
        // `write: false` is VERIFY mode — the default a normal run gets.
        let config = CorpusConfig {
            dir: tmp_dir.clone(),
            write: false,
        };

        let app = actix_web::test::init_service(App::new().route(
            "/y",
            web::get().to(|| async { HttpResponse::Ok().json(serde_json::json!({})) }),
        ))
        .await;

        let req = actix_web::test::TestRequest::get().uri("/y");
        dump_with(&config, "widget", "no-golden-yet", req, &app).await;

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    // ---- Task 2: determinism / redaction ----------------------------------

    #[test]
    fn redact_string_value_redacts_rfc3339_timestamp() {
        assert_eq!(
            redact_string_value("2026-07-31T12:00:00Z"),
            Some("<TIMESTAMP>")
        );
        assert_eq!(
            redact_string_value("2026-07-31T12:00:00.123456+00:00"),
            Some("<TIMESTAMP>")
        );
    }

    #[test]
    fn redact_string_value_redacts_bare_calendar_date() {
        // Task 4: `AttentionDto::as_of` and the `Carryover`/`Backlog`
        // `created`/`reviewed` fields wire a bare `YYYY-MM-DD` date with no
        // time component — distinct from rule 1's original RFC3339 shape,
        // and must be redacted the same way or every Attention golden
        // drifts by one day, every day.
        assert_eq!(redact_string_value("2026-07-31"), Some("<TIMESTAMP>"));
    }

    #[test]
    fn redact_string_value_redacts_uuid() {
        assert_eq!(
            redact_string_value("550e8400-e29b-41d4-a716-446655440000"),
            Some("<UUID>")
        );
        // Case-insensitive.
        assert_eq!(
            redact_string_value("550E8400-E29B-41D4-A716-446655440000"),
            Some("<UUID>")
        );
    }

    #[test]
    fn redact_string_value_redacts_temp_dir_path() {
        let tmp_dir = std::env::temp_dir();
        let path_under_tmp = tmp_dir.join("some-fixture-workspace").join("file.txt");
        let path_str = path_under_tmp.to_str().unwrap();
        assert_eq!(redact_string_value(path_str), Some("<TMP_PATH>"));
    }

    /// Regression (`ticket-contract-corpus-uncovered-routes`): a path under
    /// the **canonicalized** temp root must redact too. On macOS
    /// `std::env::temp_dir()` is `/var/folders/…` while the canonical form is
    /// `/private/var/folders/…`; `BlockGraphDto::root` wires the canonical
    /// spelling, so the original single-prefix check let a live per-run temp
    /// path into a checked-in golden.
    #[test]
    fn redact_string_value_redacts_canonicalized_temp_dir_path() {
        let canonical = std::fs::canonicalize(std::env::temp_dir())
            .expect("the system temp dir must be canonicalizable");
        let path_under_tmp = canonical.join("some-fixture-workspace");
        assert_eq!(
            redact_string_value(path_under_tmp.to_str().unwrap()),
            Some("<TMP_PATH>"),
            "a path under the canonicalized temp root must redact — see redaction rule 3"
        );
    }

    /// Both spellings must be covered simultaneously, and the un-canonicalized
    /// one must not have regressed while fixing the canonical one.
    #[test]
    fn temp_dir_prefixes_covers_both_spellings_longest_first() {
        let prefixes = super::temp_dir_prefixes();
        assert!(
            !prefixes.is_empty(),
            "there must always be at least the raw temp_dir() prefix"
        );
        let raw = std::env::temp_dir().to_str().unwrap().to_owned();
        assert!(
            prefixes.contains(&raw),
            "the raw temp_dir() spelling must always be present; got {prefixes:?}"
        );
        assert!(
            prefixes.windows(2).all(|w| w[0].len() >= w[1].len()),
            "prefixes must be ordered longest-first so the most specific wins; got {prefixes:?}"
        );
    }

    #[test]
    fn redact_string_value_leaves_run_status_strings_untouched() {
        // These are the exact strings this corpus exists to freeze — they
        // must never be redacted, no matter how similar-looking a future
        // redaction rule might become.
        for status in [
            "pending",
            "running",
            "success",
            "failed",
            "cancelled",
            "budget_halted",
            "suspended",
        ] {
            assert_eq!(
                redact_string_value(status),
                None,
                "run-status string {status:?} must never be redacted"
            );
        }
    }

    #[test]
    fn redact_string_value_leaves_ordinary_strings_untouched() {
        assert_eq!(redact_string_value("hello world"), None);
        assert_eq!(redact_string_value(""), None);
        assert_eq!(redact_string_value("not-quite-a-uuid-1234"), None);
    }

    #[test]
    fn redact_value_touches_nested_values_but_never_keys() {
        let mut value = serde_json::json!({
            "run_id": "550e8400-e29b-41d4-a716-446655440000",
            "status": "budget_halted",
            "nested": {
                "started_at": "2026-07-31T12:00:00Z",
                "550e8400-e29b-41d4-a716-446655440000": "a key that looks like a uuid",
            },
            "items": ["2026-07-31T12:00:00Z", "budget_halted"],
        });
        redact_value(&mut value);

        assert_eq!(value["run_id"], serde_json::json!("<UUID>"));
        assert_eq!(value["status"], serde_json::json!("budget_halted"));
        assert_eq!(
            value["nested"]["started_at"],
            serde_json::json!("<TIMESTAMP>")
        );
        assert_eq!(value["items"][0], serde_json::json!("<TIMESTAMP>"));
        assert_eq!(value["items"][1], serde_json::json!("budget_halted"));
        // The object KEY that happens to look like a UUID must survive
        // untouched — redaction only ever rewrites values.
        assert!(
            value["nested"]
                .as_object()
                .unwrap()
                .contains_key("550e8400-e29b-41d4-a716-446655440000"),
            "redaction must never rewrite object keys"
        );
    }

    #[test]
    fn redact_value_redacts_age_days_key_regardless_of_magnitude() {
        // Redaction rule 5 (Task 4): `age_days` is redacted by KEY name, not
        // by value pattern-matching, since a plain integer can't be told
        // apart from any other legitimate numeric field by its value alone.
        let mut value = serde_json::json!({
            "age_days": 23,
            "threshold_days": 3,
        });
        redact_value(&mut value);
        assert_eq!(value["age_days"], serde_json::json!(0));
        // The AUTHORED threshold must survive untouched — only the
        // live-clock-derived field is in `VOLATILE_NUMERIC_KEYS`.
        assert_eq!(value["threshold_days"], serde_json::json!(3));
    }

    #[test]
    fn build_golden_redacts_volatile_fields_in_the_body() {
        let body = br#"{"run_id":"550e8400-e29b-41d4-a716-446655440000","status":"budget_halted","started_at":"2026-07-31T12:00:00Z"}"#;
        let golden = build_golden(200, body);
        assert_eq!(golden["body"]["run_id"], serde_json::json!("<UUID>"));
        assert_eq!(golden["body"]["status"], serde_json::json!("budget_halted"));
        assert_eq!(
            golden["body"]["started_at"],
            serde_json::json!("<TIMESTAMP>")
        );
    }

    /// This crate's dependency graph pulls in `tree-sitter` as a
    /// build-dependency, which in principle could unify the `preserve_order`
    /// feature onto `serde_json` for the whole build (it enables
    /// `serde_json/preserve_order` for its own build script). If that ever
    /// happened, `serde_json::Value::Object` would iterate/serialize in
    /// *insertion* order instead of sorted order, silently breaking
    /// determinism across two dumps whose handler happened to build the
    /// object's fields in a different order. This test proves — for the
    /// actual `bastion` binary build, not a synthetic crate — that key order
    /// is sorted, per module docs rule 4. If this test ever starts failing,
    /// the redaction/determinism story here needs to be revisited, not just
    /// this assertion.
    #[test]
    fn object_keys_serialize_in_sorted_order_not_insertion_order() {
        let value = serde_json::json!({"zebra": 1, "apple": 2, "middle": 3});
        let serialized = serde_json::to_string(&value).unwrap();
        assert_eq!(
            serialized, r#"{"apple":2,"middle":3,"zebra":1}"#,
            "serde_json::Value::Object must serialize keys in sorted order in this build \
             (verified, not assumed) — if this fails, `preserve_order` has been unified onto \
             serde_json and corpus determinism across differently-ordered handler output can no \
             longer be assumed"
        );
    }

    /// End-to-end determinism proof (the spec's headline acceptance
    /// criterion): dispatch the *same* request through the *same* app twice,
    /// in GENERATE mode, into the same target file, and assert the two
    /// writes produce byte-identical file contents. If any timestamp/UUID
    /// fixture, HashMap-sourced ordering, or other volatile value leaked
    /// through unredacted, this test would flap.
    #[actix_web::test]
    async fn dump_generates_byte_identical_golden_across_two_runs() {
        // No env mutation and no lock — see the GENERATE-mode sibling above.
        let tmp_dir =
            crate::testsupport::unique_temp_dir("bastion-contract-corpus-test-determinism");
        let config = CorpusConfig {
            dir: tmp_dir.clone(),
            write: true,
        };

        // A handler whose response embeds exactly the volatile shapes this
        // task's redaction rules must neutralize: a UUID and an RFC3339
        // timestamp, sourced fresh (i.e. genuinely different) on each call.
        let app = actix_web::test::init_service(App::new().route(
            "/z",
            web::get().to(|| async {
                let run_id = uuid::Uuid::new_v4().to_string();
                let started_at = chrono::Utc::now().to_rfc3339();
                HttpResponse::Ok().json(serde_json::json!({
                    "run_id": run_id,
                    "started_at": started_at,
                    "status": "budget_halted",
                }))
            }),
        ))
        .await;

        let req1 = actix_web::test::TestRequest::get().uri("/z");
        dump_with(&config, "widget", "determinism", req1, &app).await;
        let path = tmp_dir.join("widget__determinism.json");
        let first_write = std::fs::read(&path).unwrap();

        let req2 = actix_web::test::TestRequest::get().uri("/z");
        dump_with(&config, "widget", "determinism", req2, &app).await;
        let second_write = std::fs::read(&path).unwrap();

        assert_eq!(
            first_write, second_write,
            "two dumps of logically-identical scenario data must produce byte-identical goldens"
        );

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
}

/// Task 3: `GET /api/runs` corpus scenarios — the empty store, one active
/// run, and one scenario per `RunStatus` variant, so every wire status
/// string is frozen (in particular `budget_halted`, which
/// `handlers::runs::run_status_str` hand-writes rather than derives).
///
/// Every scenario here dispatches through the **real** `handlers::runs::list_runs`
/// handler and the real `LiveStateStore`, wired into a minimal actix app that
/// mirrors `src/serve/mod.rs`'s `build_app` routing for just this resource
/// (`web::resource("/runs") -> list_runs`) — scoped down so these tests don't
/// need the hub/auth machinery the full app factory carries, while still
/// going through real `actix_web` dispatch and real `serde` serialization,
/// never a hand-authored `serde_json::Value`.
///
/// `run_id` (A1, `ticket-serve-run-join-and-handoff-types`, contract v0.18)
/// has already landed in this branch's history (see `git log` /
/// `RunSummaryDto`) before this task ran, so nothing is omitted here — every
/// golden below carries a real `run_id`.
#[cfg(test)]
mod runs_scenarios {
    use std::collections::{BTreeSet, HashMap};

    use actix_web::web;
    use chrono::{TimeZone, Utc};
    use engine_contract::task_context::{NodeRun, NodeRunStatus, TaskContext};
    use engine_serve::live_state::LiveStateStore;
    use uuid::Uuid;

    use super::dump;
    use crate::db::workflows::RunStatus;
    use crate::serve::handlers::runs::list_runs;

    /// Build a minimal actix `Service` wired to the real `list_runs` handler
    /// over `$store`, matching `src/serve/mod.rs`'s production
    /// `web::resource("/runs").route(web::get().to(handlers::runs::list_runs))`
    /// mapping. A macro (not a fn returning `impl Service<..>`) because the
    /// body/service associated types `actix_web::test::init_service` produces
    /// are opaque per-call-site; inlining avoids naming them.
    macro_rules! runs_service {
        ($store:expr) => {
            actix_web::test::init_service(
                actix_web::App::new()
                    .app_data(web::Data::new($store))
                    .app_data(web::Data::new(crate::config::FileConfig::default()))
                    .service(web::resource("/api/runs").route(web::get().to(list_runs))),
            )
            .await
        };
        ($store:expr, $registry:expr) => {
            actix_web::test::init_service(
                actix_web::App::new()
                    .app_data(web::Data::new($store))
                    .app_data(web::Data::new($registry))
                    .service(web::resource("/api/runs").route(web::get().to(list_runs))),
            )
            .await
        };
    }

    /// Fixed, non-random run ids (determinism rule 2 — never
    /// `Uuid::new_v4()` in a fixture; [`super::redact_value`]'s UUID
    /// redaction is the backstop, not the primary source of determinism).
    fn fixed_run_id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn node_run(status: NodeRunStatus) -> NodeRun {
        NodeRun {
            status,
            started_at: None,
            completed_at: None,
            error: None,
            input: None,
            usage: None,
        }
    }

    /// A run with a single node in the given `NodeRunStatus` and no metadata
    /// annotations — exercises `derive_run_status`'s node-aggregate fallback
    /// (priority order step 4: `pending`/`running`/`failed`/`success`).
    fn ctx_with_node(status: NodeRunStatus) -> TaskContext {
        let mut node_runs = HashMap::new();
        node_runs.insert("Node".to_string(), node_run(status));
        TaskContext {
            event: serde_json::json!({}),
            nodes: HashMap::new(),
            metadata: serde_json::json!({}),
            node_runs,
        }
    }

    /// A run carrying a v1.1.0 metadata annotation (cancellation / budget /
    /// suspension) and no tracked nodes — exercises `derive_run_status`'s
    /// metadata-priority path (steps 1-3), which wins over node state.
    fn ctx_with_metadata(metadata: serde_json::Value) -> TaskContext {
        TaskContext {
            event: serde_json::json!({}),
            nodes: HashMap::new(),
            metadata,
            node_runs: HashMap::new(),
        }
    }

    /// The seven `RunStatus` variants, each paired with the fixture that
    /// drives `derive_run_status` to produce it and the scenario name its
    /// golden is filed under. **Exhaustive by construction**: adding an
    /// eighth `RunStatus` variant breaks [`expected_status_str`]'s match
    /// (no wildcard arm) at *compile* time — a stronger guarantee than a
    /// runtime test failure — forcing this list to be extended too before
    /// the crate builds again.
    fn variant_scenarios() -> Vec<(&'static str, TaskContext, RunStatus)> {
        vec![
            (
                "pending",
                ctx_with_node(NodeRunStatus::Pending),
                RunStatus::Pending,
            ),
            (
                "running",
                ctx_with_node(NodeRunStatus::Running),
                RunStatus::Running,
            ),
            (
                "success",
                ctx_with_node(NodeRunStatus::Success),
                RunStatus::Success,
            ),
            (
                "failed",
                ctx_with_node(NodeRunStatus::Failed),
                RunStatus::Failed,
            ),
            (
                "cancelled",
                ctx_with_metadata(serde_json::json!({
                    "cancellation": { "cancelled": true }
                })),
                RunStatus::Cancelled,
            ),
            (
                "budget_halted",
                ctx_with_metadata(serde_json::json!({
                    "budget": {
                        "halted": true,
                        "reason": {
                            "cap": "max_total_tokens",
                            "spent": 120_000,
                            "limit": 100_000,
                        }
                    }
                })),
                RunStatus::BudgetHalted,
            ),
            (
                "suspended",
                ctx_with_metadata(serde_json::json!({
                    "suspension": { "suspended": true }
                })),
                RunStatus::Suspended,
            ),
        ]
    }

    /// Exhaustive `RunStatus` -> wire string map. Deliberately independent of
    /// `handlers::runs::run_status_str` (private to that module) so this
    /// module's coverage guarantee does not depend on reaching into that
    /// function — the two are instead kept honest by
    /// `observed_status_strings_equal_expected_seven_variant_set`, which
    /// compares this map's expectations against what the real handler
    /// actually emits.
    fn expected_status_str(status: RunStatus) -> &'static str {
        match status {
            RunStatus::Pending => "pending",
            RunStatus::Running => "running",
            RunStatus::Success => "success",
            RunStatus::Failed => "failed",
            RunStatus::Cancelled => "cancelled",
            RunStatus::BudgetHalted => "budget_halted",
            RunStatus::Suspended => "suspended",
        }
    }

    // ---- run_id -> repo join fixtures (A7, ?with_repo=1) ---------------

    /// The `run_id` `FLOW_JSON_WITH_RUN_ID` carries — the join key the
    /// `with-repo` golden below resolves against.
    const FLOW_RUN_ID: &str = "a1b2c3d4-e5f6-4789-a012-3456789abcde";

    const FLOW_JSON_WITH_RUN_ID: &str = include_str!("status/fixtures/flow_state_with_run_id.json");

    /// A single-repo registry (`"repo-x"`) whose flow state carries
    /// [`FLOW_RUN_ID`] — used to exercise a repo-resolvable run id without
    /// pulling in `workflows_scenarios`' private helpers (a different
    /// `#[cfg(test)] mod`, so its `TempDir`/`registry`/`repo_with_flow` are
    /// not visible here).
    fn registry_with_run_id_flow() -> (super::fixtures::TempDir, crate::config::FileConfig) {
        let tmp = super::fixtures::TempDir::new("runs-with-repo");
        super::fixtures::write(
            &tmp.path()
                .join("planning")
                .join("phase6-blockA")
                .join("sdlc")
                .join("sdlc-flow-state.json"),
            FLOW_JSON_WITH_RUN_ID,
        );
        let registry = crate::config::FileConfig {
            workspaces: Some(HashMap::from([(
                "repo-x".to_owned(),
                tmp.path().to_path_buf(),
            )])),
            ..Default::default()
        };
        (tmp, registry)
    }

    // ---- corpus goldens -----------------------------------------------

    /// `?with_repo=1` scenario (A7): the active run's id equals
    /// [`FLOW_RUN_ID`], so the join resolves `repo: "repo-x"` — freezing the
    /// one new field this ticket adds. All nine pre-existing `runs__*`
    /// goldens (the empty store + `runs_corpus_active_run` + the seven
    /// `RunStatus` variant scenarios) must stay byte-identical, since `repo`
    /// uses `skip_serializing_if = "Option::is_none"` and none of those
    /// scenarios pass `?with_repo=1`.
    #[actix_web::test]
    async fn runs_corpus_with_repo() {
        let (_tmp, registry) = registry_with_run_id_flow();
        let store = LiveStateStore::new();
        let run_id = Uuid::parse_str(FLOW_RUN_ID).expect("fixture run_id must parse as a UUID");
        store.record(
            run_id,
            &TaskContext {
                event: serde_json::json!({ "spec_slug": "contract-corpus-with-repo-fixture" }),
                nodes: HashMap::new(),
                metadata: serde_json::json!({}),
                node_runs: HashMap::new(),
            },
        );

        let app = runs_service!(store, registry);
        let req = actix_web::test::TestRequest::get().uri("/api/runs?with_repo=1");
        dump("runs", "with-repo", req, &app).await;
    }

    #[actix_web::test]
    async fn runs_corpus_empty_store() {
        let app = runs_service!(LiveStateStore::new());
        let req = actix_web::test::TestRequest::get().uri("/api/runs");
        dump("runs", "empty", req, &app).await;
    }

    #[actix_web::test]
    async fn runs_corpus_active_run() {
        let store = LiveStateStore::new();
        let mut node_runs = HashMap::new();
        node_runs.insert(
            "PlanNode".to_string(),
            NodeRun {
                status: NodeRunStatus::Running,
                started_at: Some(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()),
                completed_at: None,
                error: None,
                input: None,
                usage: None,
            },
        );
        store.record(
            fixed_run_id(1),
            &TaskContext {
                event: serde_json::json!({ "spec_slug": "contract-corpus-active-fixture" }),
                nodes: HashMap::new(),
                metadata: serde_json::json!({}),
                node_runs,
            },
        );

        let app = runs_service!(store);
        let req = actix_web::test::TestRequest::get().uri("/api/runs");
        dump("runs", "active", req, &app).await;
    }

    #[actix_web::test]
    async fn runs_corpus_all_run_status_variants() {
        for (i, (scenario, ctx, _expected)) in variant_scenarios().into_iter().enumerate() {
            let store = LiveStateStore::new();
            // `100 + i` keeps ids disjoint from `runs_corpus_active_run`'s
            // fixed id and from each other; the exact value is irrelevant
            // once redacted, but must stay fixed (never `Uuid::new_v4()`).
            store.record(fixed_run_id(100 + i as u128), &ctx);
            let app = runs_service!(store);
            let req = actix_web::test::TestRequest::get().uri("/api/runs");
            dump("runs", scenario, req, &app).await;
        }
    }

    /// The coverage guarantee this task exists for: the set of `status`
    /// strings actually observed by dispatching [`variant_scenarios`]
    /// through the real handler must equal the expected seven-element set.
    /// Combined with [`variant_scenarios`]'s exhaustive-match construction,
    /// an eighth `RunStatus` variant added without a matching scenario here
    /// either fails the crate to compile (if `expected_status_str` is left
    /// un-updated) or fails this test's set-equality assertion — never
    /// silently ships an unfrozen status string.
    #[actix_web::test]
    async fn observed_status_strings_equal_expected_seven_variant_set() {
        let mut observed: BTreeSet<String> = BTreeSet::new();

        for (scenario, ctx, expected_status) in variant_scenarios() {
            let store = LiveStateStore::new();
            store.record(Uuid::new_v4(), &ctx);
            let resp = list_runs(
                web::Query(crate::serve::handlers::runs::RunsQuery::default()),
                web::Data::new(store),
                web::Data::new(crate::config::FileConfig::default()),
            )
            .await;
            assert_eq!(resp.status(), 200, "scenario {scenario} must return 200");

            let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let arr = json.as_array().expect("array body");
            assert_eq!(
                arr.len(),
                1,
                "scenario {scenario} must produce exactly one run"
            );
            assert!(
                arr[0]
                    .get("run_id")
                    .and_then(serde_json::Value::as_str)
                    .is_some(),
                "scenario {scenario} must carry a run_id (A1 has landed)"
            );

            let status = arr[0]
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_else(|| panic!("scenario {scenario} response missing status field"));
            let expected = expected_status_str(expected_status);
            assert_eq!(
                status, expected,
                "scenario {scenario} produced status {status:?}, expected {expected:?}"
            );
            observed.insert(status.to_owned());
        }

        let expected: BTreeSet<String> = [
            "pending",
            "running",
            "success",
            "failed",
            "cancelled",
            "budget_halted",
            "suspended",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();

        assert_eq!(
            observed, expected,
            "observed RunStatus wire strings must exactly equal the expected seven-element set \
             — if this fails after adding a RunStatus variant, add its scenario to \
             `variant_scenarios()` (and a golden) rather than only updating this set"
        );

        // The single highest-value assertion this corpus exists for (see
        // module docs + task spec): `run_status_str` is a hand-written match,
        // not a serde derive, and would emit `budgethalted` under
        // `rename_all = "lowercase"` — pin the underscore spelling
        // explicitly, not just via set membership.
        assert!(
            observed.contains("budget_halted"),
            "budget_halted must be frozen with its exact underscore spelling"
        );
    }
}

/// Task 4 shared test-only fixture plumbing: a minimal on-disk HQ brain
/// corpus for the `board`/`attention` scenarios, plus the small `TempDir`
/// helper every fixture-backed scenario module below uses (mirroring the
/// existing pattern in `handlers/pipeline.rs`/`handlers/block_graph.rs`'s own
/// test modules — reused here by name/shape, not by import, since each
/// lives in its own private test module).
#[cfg(test)]
mod fixtures {
    use std::path::{Path, PathBuf};

    /// Minimal self-cleaning temp-dir helper. Mirrors
    /// `handlers/pipeline.rs`'s / `handlers/block_graph.rs`'s own test-only
    /// `TempDir` — duplicated rather than shared across modules because each
    /// lives behind its own private `#[cfg(test)] mod`.
    pub(super) struct TempDir(PathBuf);

    impl TempDir {
        /// Path construction is delegated to
        /// [`crate::testsupport::unique_temp_dir`] — the one helper whose
        /// `<prefix>-<pid>-<nanos>-<counter>` shape is collision-proof across
        /// both threads (`cargo test`) and processes (`cargo nextest`). The
        /// hand-rolled `{name}_{pid}_{id}` idiom this replaced was only
        /// collision-free *within* this module's counter; a sibling module
        /// choosing the same `name` could still land on the same path.
        pub(super) fn new(name: &str) -> Self {
            let dir =
                crate::testsupport::unique_temp_dir(&format!("bastion-contract-corpus-{name}"));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        pub(super) fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Write `content` to `path`, creating parent directories as needed.
    pub(super) fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    /// `today - n` as `YYYY-MM-DD`, computed at fixture-build time from the
    /// same wall clock `get_attention` itself reads
    /// (`chrono::Local::now().date_naive()`) — NOT a hardcoded calendar
    /// date. Because both this fixture-build-time subtraction and the
    /// handler's own `today - anchor` subtraction happen against the same
    /// live "now" a moment apart, the resulting `age_days` is a fixed
    /// constant (`n`, give or take a same-day rounding of zero) regardless
    /// of which real-world day the test runs on — see module docs'
    /// redaction rule 5 for why `age_days` itself is *also* redacted as a
    /// backstop, not a replacement for this.
    pub(super) fn days_ago(n: i64) -> String {
        let today = chrono::Local::now().date_naive();
        (today - chrono::Duration::days(n))
            .format("%Y-%m-%d")
            .to_string()
    }
}

/// Task 4: `GET /api/board` corpus scenarios — `scope=hq`/`tier`/`epic`, the
/// epic-404 registry-miss, and the `last_touched`/`dependent_count`/`ready`
/// enrichment fields (`plan-board-graph-enrichment`, A5, already landed on
/// this branch — see `tasks.md` Notes).
///
/// Every scenario here dispatches through the **real**
/// `handlers::board::get_board` handler, wired into a minimal actix app that
/// mirrors `src/serve/mod.rs`'s production `web::resource("/api/board")`
/// mapping over a real, on-disk fixture brain corpus (`fixture_hq_corpus`) —
/// scoped down so these tests don't need the hub/auth machinery the full app
/// factory carries, while still going through real `actix_web` dispatch and
/// real `mev`/`serde` brain-walk + serialization, never a hand-authored
/// `serde_json::Value` (mirrors `runs_scenarios`' own doc comment).
#[cfg(test)]
mod board_scenarios {
    use std::collections::HashMap;

    use actix_web::web;

    use super::dump;
    use super::fixtures::{TempDir, write};
    use crate::config::FileConfig;
    use crate::serve::handlers::board::get_board;

    /// Build a fixture HQ brain corpus:
    /// - `brain.toml`: one `[[repos]]` entry, `bastion`, tier `core`.
    /// - `planning/state.json` (HQ, `kind: "brain"`): one HQ `epics[]` entry
    ///   (`epic-alpha`) — the registry `scope=epic` validates `&epic=`
    ///   against.
    /// - `bastion/planning/state.json` (`kind: "project"`): `BA.1`
    ///   (`in_progress`, tagged `epic-alpha`, wave 1 — lands in the `now`
    ///   lane), `BA.2` (`open`, `depends_on` the still-open `BA.1` — lands
    ///   in the `blocked` lane with an unmet dep, and gives `BA.1` a
    ///   corpus-wide `dependent_count` of 1 under `?graph=1`), and `BA.9`
    ///   (`closed` — lands in the `finished` lane).
    /// - `bastion/planning/BA.1-in-progress/sdlc/sdlc-flow-state.json`: a
    ///   resolvable SDLC spec folder for `BA.1` only, so
    ///   `derive_last_touched` (BA.11.S) has exactly one resolvable block —
    ///   mirrors `handlers/block_graph.rs`'s own cross-check fixture.
    ///
    /// `bastion`'s `status_file`/`cache_doc` are deliberately never created,
    /// so `check_sync` reports every in-scope repo stale — a static,
    /// non-live-clock-derived `stale: true` on every board golden below.
    fn fixture_hq_corpus() -> TempDir {
        let tmp = TempDir::new("board");
        let root = tmp.path();

        write(
            &root.join("brain.toml"),
            r#"
[[repos]]
slug = "bastion"
tier = "core"
repo_path = "bastion"
status_file = "docs/status.md"
cache_doc = "docs/projects/bastion.md"
heading = "Bastion"
"#,
        );

        write(
            &root.join("planning").join("state.json"),
            r#"{
  "repo": "hq",
  "kind": "brain",
  "updated": "2026-07-28",
  "epics": [
    {"slug": "epic-alpha", "title": "Epic Alpha", "description": "The alpha initiative"}
  ]
}"#,
        );

        write(
            &root.join("bastion").join("planning").join("state.json"),
            r#"{
  "repo": "bastion",
  "kind": "project",
  "updated": "2026-07-28",
  "tracks": [
    {
      "title": "Phase 1",
      "blocks": [
        {"id": "BA.1", "title": "In progress", "status": "in_progress", "epics": ["epic-alpha"], "wave": 1},
        {"id": "BA.2", "title": "Waiting on BA.1", "status": "open", "depends_on": [{"type": "block", "repo": "bastion", "id": "BA.1"}]},
        {"id": "BA.9", "title": "Closed", "status": "closed"}
      ]
    }
  ]
}"#,
        );

        write(
            &root
                .join("bastion")
                .join("planning")
                .join("BA.1-in-progress")
                .join("sdlc")
                .join("sdlc-flow-state.json"),
            r#"{"updated_at": "2026-07-28T12:00:00Z"}"#,
        );

        tmp
    }

    fn registry(tmp: &TempDir) -> FileConfig {
        FileConfig {
            workspaces: Some(HashMap::from([("hq".to_owned(), tmp.path().to_path_buf())])),
            default_workspace: Some("hq".to_owned()),
            ..Default::default()
        }
    }

    macro_rules! board_service {
        ($registry:expr) => {
            actix_web::test::init_service(
                actix_web::App::new()
                    .app_data(web::Data::new($registry))
                    .service(web::resource("/api/board").route(web::get().to(get_board))),
            )
            .await
        };
    }

    #[actix_web::test]
    async fn board_corpus_hq() {
        let tmp = fixture_hq_corpus();
        let app = board_service!(registry(&tmp));
        let req = actix_web::test::TestRequest::get().uri("/api/board?scope=hq");
        dump("board", "hq", req, &app).await;
    }

    #[actix_web::test]
    async fn board_corpus_tier() {
        let tmp = fixture_hq_corpus();
        let app = board_service!(registry(&tmp));
        let req = actix_web::test::TestRequest::get().uri("/api/board?scope=tier&tier=core");
        dump("board", "tier", req, &app).await;
    }

    #[actix_web::test]
    async fn board_corpus_epic() {
        let tmp = fixture_hq_corpus();
        let app = board_service!(registry(&tmp));
        let req = actix_web::test::TestRequest::get().uri("/api/board?scope=epic&epic=epic-alpha");
        dump("board", "epic", req, &app).await;
    }

    #[actix_web::test]
    async fn board_corpus_epic_404() {
        let tmp = fixture_hq_corpus();
        let app = board_service!(registry(&tmp));
        let req =
            actix_web::test::TestRequest::get().uri("/api/board?scope=epic&epic=no-such-epic");
        dump("board", "epic-404", req, &app).await;
    }

    /// `last_touched` is populated unconditionally (no `?graph=1` gate) —
    /// this scenario exists as its own named golden per the task spec's
    /// scenario list, even though `board__hq` above already exercises the
    /// same field on the same fixture's `BA.1`.
    #[actix_web::test]
    async fn board_corpus_last_touched() {
        let tmp = fixture_hq_corpus();
        let app = board_service!(registry(&tmp));
        let req = actix_web::test::TestRequest::get().uri("/api/board?scope=hq");
        dump("board", "last_touched", req, &app).await;
    }

    /// `?graph=1` gates the A5 `dependent_count`/`ready` enrichment
    /// (`plan-board-graph-enrichment`, already landed) — `BA.1` has `BA.2`
    /// depending on it, so `BA.1`'s `now`-lane entry carries a nonzero
    /// `dependent_count` here (`None` on every other board golden above,
    /// which omit `?graph=1`).
    ///
    /// This is also the sole golden that exercises `effective_priority`
    /// (BA.19.B). The shared `fixture_hq_corpus()` authors no `priority` on
    /// any block, so mev's min-propagation (`../mev/src/brain/state.rs`'s
    /// `effective_priorities`) lands every node at the unpropagated
    /// `u8::MAX` sentinel and omits all three keys from its map — the golden
    /// would carry no `effective_priority` field at all, an evidence gap
    /// caught only at BA.19.B task 4's fixture-evidence check, not by task
    /// 3's own drift regeneration. Overwriting `BA.2`'s authored `priority`
    /// to `1` here (isolated to this scenario's own corpus copy, not the
    /// shared fixture the other board goldens above reuse) makes `BA.1`
    /// inherit that hotness through the reverse `depends_on` walk — "a block
    /// inherits the hotness of what it gates" — giving both `BA.1` and
    /// `BA.2` a real, in-range `effective_priority` without touching any
    /// other board golden's output.
    #[actix_web::test]
    async fn board_corpus_dependent_count() {
        let tmp = fixture_hq_corpus();
        write(
            &tmp.path()
                .join("bastion")
                .join("planning")
                .join("state.json"),
            r#"{
  "repo": "bastion",
  "kind": "project",
  "updated": "2026-07-28",
  "tracks": [
    {
      "title": "Phase 1",
      "blocks": [
        {"id": "BA.1", "title": "In progress", "status": "in_progress", "epics": ["epic-alpha"], "wave": 1},
        {"id": "BA.2", "title": "Waiting on BA.1", "status": "open", "priority": 1, "depends_on": [{"type": "block", "repo": "bastion", "id": "BA.1"}]},
        {"id": "BA.9", "title": "Closed", "status": "closed"}
      ]
    }
  ]
}"#,
        );
        let app = board_service!(registry(&tmp));
        let req = actix_web::test::TestRequest::get().uri("/api/board?scope=hq&graph=true");
        dump("board", "dependent_count", req, &app).await;
    }
}

/// Task 4: `GET /api/attention` corpus scenarios — populated (stale
/// carryover + aging backlog + orphaned capture) and empty.
///
/// See `board_scenarios`' doc comment for the shared "minimal app wired to
/// the real handler over a real on-disk fixture" approach — identical here,
/// against `handlers::attention::get_attention`.
///
/// # The live-clock problem this module's fixtures solve
/// `get_attention` computes `as_of`/`age_days` from
/// `chrono::Local::now().date_naive()` at request time — there is no
/// fixture-clock override. Every `created`/`reviewed` date below is
/// therefore built with [`super::fixtures::days_ago`], **not** a hardcoded
/// calendar date, so the resulting `age_days` is a fixed constant across
/// every real-world day the test runs on; [`super::redact_value`]'s bare
/// calendar-date + `age_days` rules (redaction rules 1 and 5) are the
/// backstop for the remaining `as_of` field, which is always today's literal
/// date regardless of fixture design.
#[cfg(test)]
mod attention_scenarios {
    use std::collections::HashMap;

    use actix_web::web;

    use super::dump;
    use super::fixtures::{TempDir, days_ago, write};
    use crate::config::FileConfig;
    use crate::serve::handlers::attention::get_attention;

    /// Build a fixture HQ brain corpus with one stale `carryover[]` entry
    /// (`bastion`, `known_issue`, threshold 10 days — created 15 days ago,
    /// so it trips), one aging `backlog[]` idea (threshold 7 days — created
    /// 10 days ago), and one `origin.type == "capture"` backlog idea aged
    /// the same way (lands in `orphaned_captures`, not `aging_backlog`).
    fn fixture_populated() -> TempDir {
        let tmp = TempDir::new("attention-populated");
        let root = tmp.path();

        write(
            &root.join("brain.toml"),
            r#"
[[repos]]
slug = "bastion"
tier = "core"
repo_path = "bastion"
status_file = "docs/status.md"
cache_doc = "docs/projects/bastion.md"
heading = "Bastion"
"#,
        );

        write(
            &root.join("planning").join("state.json"),
            &format!(
                r#"{{
  "repo": "hq",
  "kind": "brain",
  "updated": "2026-07-28",
  "backlog": [
    {{
      "slug": "aging-idea",
      "title": "Aging idea",
      "repo": "bastion",
      "type": "improvement",
      "status": "idea",
      "created": "{aging}"
    }},
    {{
      "slug": "orphan-capture",
      "title": "Orphaned capture",
      "repo": "bastion",
      "type": "chore",
      "status": "idea",
      "created": "{aging}",
      "origin": {{"type": "capture", "notes": "planning/orphan-capture/notes.md"}}
    }}
  ]
}}"#,
                aging = days_ago(10),
            ),
        );

        write(
            &root.join("bastion").join("planning").join("state.json"),
            &format!(
                r#"{{
  "repo": "bastion",
  "kind": "project",
  "updated": "2026-07-28",
  "carryover": [
    {{
      "slug": "stale-known-issue",
      "scope": {{"repo": "bastion"}},
      "kind": "known_issue",
      "text": "some known issue that has gone stale",
      "created": "{created}"
    }}
  ]
}}"#,
                created = days_ago(15),
            ),
        );

        tmp
    }

    /// A brain with a registered repo but zero `carryover[]`/`backlog[]`
    /// entries anywhere — every Attention lane comes back empty.
    fn fixture_empty() -> TempDir {
        let tmp = TempDir::new("attention-empty");
        let root = tmp.path();

        write(
            &root.join("brain.toml"),
            r#"
[[repos]]
slug = "bastion"
tier = "core"
repo_path = "bastion"
status_file = "docs/status.md"
cache_doc = "docs/projects/bastion.md"
heading = "Bastion"
"#,
        );

        write(
            &root.join("planning").join("state.json"),
            r#"{
  "repo": "hq",
  "kind": "brain",
  "updated": "2026-07-28"
}"#,
        );

        tmp
    }

    fn registry(tmp: &TempDir) -> FileConfig {
        FileConfig {
            workspaces: Some(HashMap::from([("hq".to_owned(), tmp.path().to_path_buf())])),
            default_workspace: Some("hq".to_owned()),
            ..Default::default()
        }
    }

    macro_rules! attention_service {
        ($registry:expr) => {
            actix_web::test::init_service(
                actix_web::App::new()
                    .app_data(web::Data::new($registry))
                    .service(web::resource("/api/attention").route(web::get().to(get_attention))),
            )
            .await
        };
    }

    #[actix_web::test]
    async fn attention_corpus_populated() {
        let tmp = fixture_populated();
        let app = attention_service!(registry(&tmp));
        let req = actix_web::test::TestRequest::get().uri("/api/attention?scope=hq");
        dump("attention", "populated", req, &app).await;
    }

    #[actix_web::test]
    async fn attention_corpus_empty() {
        let tmp = fixture_empty();
        let app = attention_service!(registry(&tmp));
        let req = actix_web::test::TestRequest::get().uri("/api/attention?scope=hq");
        dump("attention", "empty", req, &app).await;
    }
}

/// Task 4: `GET /api/pipeline` corpus scenarios — populated (a real
/// `business/docs/pipeline.md` + opportunities/leads) and the keyless-`{}`
/// case (no `business/` tree at all, so both `stages`/`opportunities`
/// degrade to their `#[serde(default)]` empty values).
///
/// See `board_scenarios`' doc comment for the shared fixture-app approach —
/// identical here, against `handlers::pipeline::get_pipeline`. Every
/// frontmatter field here is authored, static test data (never derived from
/// a live clock), so — unlike `attention_scenarios` — no
/// [`super::fixtures::days_ago`] trick is needed; any literal dates in the
/// fixture content are incidentally redacted by
/// [`super::redact_value`]'s bare-calendar-date rule, which is harmless here
/// (the corpus exists to freeze *shape*, not these fixture literals).
#[cfg(test)]
mod pipeline_scenarios {
    use std::collections::HashMap;

    use actix_web::web;

    use super::dump;
    use super::fixtures::{TempDir, write};
    use crate::config::FileConfig;
    use crate::serve::handlers::pipeline::get_pipeline;

    const STAGES_MD: &str = "# Pipeline\n\n## Stages\n\n`identified` → `researching` → `contacted` → `conversation` → `proposal-sent` → `closed-won` → `closed-lost`\n\n---\n\n## Active Leads\n";

    const OPPORTUNITY_FIXTURE: &str = "---\ntype: Opportunity\ntitle: Anthropic\nkind: company\nstage: identified\nsource: RESEARCH_AGENT test run\nurl: https://www.anthropic.com\ncontacts: []\nactions:\n  - { at: 2026-07-25, kind: research, note: \"Generated CompanyBrief\" }\n---\n\n# Anthropic\n\n## Research Brief\n```json\n{ \"company_name\": \"Anthropic\", \"summary\": \"An AI lab.\" }\n```\n";

    /// A brain root with `brain.toml` (so `find_brain_root` resolves) plus a
    /// real `business/docs/pipeline.md` and one opportunity file.
    fn fixture_populated() -> TempDir {
        let tmp = TempDir::new("pipeline-populated");
        let root = tmp.path();

        write(&root.join("brain.toml"), "");

        write(
            &root.join("business").join("docs").join("pipeline.md"),
            STAGES_MD,
        );
        write(
            &root
                .join("business")
                .join("docs")
                .join("opportunities")
                .join("anthropic.md"),
            OPPORTUNITY_FIXTURE,
        );

        tmp
    }

    /// A brain root with `brain.toml` present but NO `business/` tree at
    /// all — the keyless-`{}` case: `stages`/`opportunities` both degrade to
    /// their `#[serde(default)]` empty values rather than erroring (D25:
    /// this route never treats a missing business tree as a failure).
    fn fixture_keyless() -> TempDir {
        let tmp = TempDir::new("pipeline-keyless");
        write(&tmp.path().join("brain.toml"), "");
        tmp
    }

    fn registry(tmp: &TempDir) -> FileConfig {
        FileConfig {
            workspaces: Some(HashMap::from([("hq".to_owned(), tmp.path().to_path_buf())])),
            default_workspace: Some("hq".to_owned()),
            ..Default::default()
        }
    }

    macro_rules! pipeline_service {
        ($registry:expr) => {
            actix_web::test::init_service(
                actix_web::App::new()
                    .app_data(web::Data::new($registry))
                    .service(web::resource("/api/pipeline").route(web::get().to(get_pipeline))),
            )
            .await
        };
    }

    #[actix_web::test]
    async fn pipeline_corpus_populated() {
        let tmp = fixture_populated();
        let app = pipeline_service!(registry(&tmp));
        let req = actix_web::test::TestRequest::get().uri("/api/pipeline");
        dump("pipeline", "populated", req, &app).await;
    }

    #[actix_web::test]
    async fn pipeline_corpus_keyless() {
        let tmp = fixture_keyless();
        let app = pipeline_service!(registry(&tmp));
        let req = actix_web::test::TestRequest::get().uri("/api/pipeline");
        dump("pipeline", "keyless", req, &app).await;
    }
}

/// Task 4: `GET /api/docs/{repo}/tree` corpus scenarios — a directory path,
/// a **file** path (the exact shape of shipped bug 1: `resolve_allowlisted_path`
/// never calls `is_dir()`, so a file resolves successfully through the
/// allowlist and `crawl_brain` walks it as a root, producing `200` +
/// `entries: []` rather than the `404` the old bastion-web stub assumed —
/// this is the single most valuable golden in the whole corpus, per the
/// task spec), and an unknown/nonexistent path (`404`).
///
/// See `board_scenarios`' doc comment for the shared fixture-app approach —
/// identical here, against `handlers::docs::get_docs_tree`. No live-clock
/// concern: `DocTreeDto` carries no `modified`/timestamp field (unlike
/// `DocFileDto`, out of this task's scenario list).
#[cfg(test)]
mod docs_scenarios {
    use std::collections::HashMap;

    use actix_web::web;

    use super::dump;
    use super::fixtures::{TempDir, write};
    use crate::config::FileConfig;
    use crate::serve::handlers::docs::get_docs_tree;

    /// A repo root with one allowlisted `docs/` directory containing one
    /// markdown file — enough to exercise both the directory-listing and
    /// (via `?path=docs/guide.md`) the file-path-as-tree-root scenarios.
    fn fixture_repo() -> TempDir {
        let tmp = TempDir::new("docs-tree");
        write(
            &tmp.path().join("docs").join("guide.md"),
            "# Guide\n\nSome content.\n",
        );
        tmp
    }

    fn registry(tmp: &TempDir) -> FileConfig {
        FileConfig {
            workspaces: Some(HashMap::from([(
                "myrepo".to_owned(),
                tmp.path().to_path_buf(),
            )])),
            ..Default::default()
        }
    }

    macro_rules! docs_tree_service {
        ($registry:expr) => {
            actix_web::test::init_service(
                actix_web::App::new()
                    .app_data(web::Data::new($registry))
                    .service(
                        web::resource("/api/docs/{repo}/tree").route(web::get().to(get_docs_tree)),
                    ),
            )
            .await
        };
    }

    #[actix_web::test]
    async fn docs_tree_corpus_dir() {
        let tmp = fixture_repo();
        let app = docs_tree_service!(registry(&tmp));
        let req = actix_web::test::TestRequest::get().uri("/api/docs/myrepo/tree?path=docs");
        dump("docs-tree", "dir", req, &app).await;
    }

    /// Bug 1's exact shape: a **file** path through the tree endpoint must
    /// come back `200` with `entries: []`, never `404` — the old
    /// bastion-web stub 404'd here because it (incorrectly) assumed a tree
    /// request always targets a directory. See `docs::resolve_allowlisted_path`
    /// and `handlers/docs.rs`'s own `assemble_md_paths`: the file resolves
    /// through the allowlist fine, then `crawl_brain`'s `WalkDir` walks the
    /// single-file root, and `build_doc_tree` drops the resulting entry
    /// (its relative path equals the tree `root` itself, so `strip_root`
    /// yields no child) — leaving `entries: []`.
    #[actix_web::test]
    async fn docs_tree_corpus_file() {
        let tmp = fixture_repo();
        let app = docs_tree_service!(registry(&tmp));
        let req =
            actix_web::test::TestRequest::get().uri("/api/docs/myrepo/tree?path=docs/guide.md");
        dump("docs-tree", "file", req, &app).await;
    }

    #[actix_web::test]
    async fn docs_tree_corpus_unknown() {
        let tmp = fixture_repo();
        let app = docs_tree_service!(registry(&tmp));
        let req = actix_web::test::TestRequest::get()
            .uri("/api/docs/myrepo/tree?path=docs/does-not-exist.md");
        dump("docs-tree", "unknown", req, &app).await;
    }
}

/// `GET /api/workflows` corpus scenarios (`ticket-contract-corpus-uncovered-routes`,
/// task 1) — the cross-repo flow-state aggregate added by A2 (serve-api
/// v0.20), which the original A4 corpus never froze.
///
/// See `board_scenarios`' doc comment for the shared "minimal app wired to the
/// real handler over a real on-disk fixture" approach — identical here, against
/// `handlers::status::list_all_workflows`.
///
/// Both goldens are deterministic without any redaction trick: the two
/// `sdlc-flow-state.json` fixtures reused here (`status/fixtures/`, the same
/// files `src/serve/mod.rs`'s A2 route tests seed with) carry *authored*
/// `started_at`/`updated_at`/`run_id` literals, never live-clock or
/// `Uuid::new_v4()` values — [`super::redact_value`]'s timestamp/UUID rules
/// then neutralize them a second time, as a backstop rather than as the source
/// of determinism.
#[cfg(test)]
mod workflows_scenarios {
    use std::collections::HashMap;

    use actix_web::web;

    use super::dump;
    use super::fixtures::{TempDir, write};
    use crate::config::FileConfig;
    use crate::serve::handlers::status::list_all_workflows;

    /// Flow state **without** a `run_id` key — freezes the
    /// `skip_serializing_if = "Option::is_none"` branch of
    /// `RepoWorkflowStateDto::run_id` (the key is absent, not `null`).
    const FLOW_JSON: &str = include_str!("status/fixtures/flow_state_valid.json");
    /// Flow state **with** an authored `run_id` — freezes the present-key
    /// branch of the same field (A1's `run_id` join key).
    const FLOW_JSON_WITH_RUN_ID: &str = include_str!("status/fixtures/flow_state_with_run_id.json");

    macro_rules! workflows_service {
        ($registry:expr) => {
            actix_web::test::init_service(
                actix_web::App::new()
                    .app_data(web::Data::new($registry))
                    .service(
                        web::resource("/api/workflows").route(web::get().to(list_all_workflows)),
                    ),
            )
            .await
        };
    }

    fn registry(entries: &[(&str, &TempDir)]) -> FileConfig {
        FileConfig {
            workspaces: Some(
                entries
                    .iter()
                    .map(|(name, tmp)| ((*name).to_owned(), tmp.path().to_path_buf()))
                    .collect::<HashMap<_, _>>(),
            ),
            ..Default::default()
        }
    }

    /// Seed one repo's `planning/<spec>/sdlc/sdlc-flow-state.json`.
    fn repo_with_flow(name: &str, flow_json: &str) -> TempDir {
        let tmp = TempDir::new(name);
        write(
            &tmp.path()
                .join("planning")
                .join("phase6-blockA")
                .join("sdlc")
                .join("sdlc-flow-state.json"),
            flow_json,
        );
        tmp
    }

    /// An empty workspace registry is explicitly **not** a 404 — the aggregate
    /// degrades to `200 []` (`list_all_workflows`' documented contract). That
    /// is exactly the kind of "does an absent thing 404 or empty-200?"
    /// question a Surface's stub gets wrong, so it gets its own golden.
    #[actix_web::test]
    async fn workflows_corpus_empty() {
        let app = workflows_service!(FileConfig::default());
        let req = actix_web::test::TestRequest::get().uri("/api/workflows");
        dump("workflows", "empty", req, &app).await;
    }

    /// Two registered repos, each carrying one flow state — freezes the
    /// per-repo `repo` tagging, the (repo, spec_slug) sort order the handler
    /// imposes over the `HashMap`-sourced registry (without which this golden
    /// would flap run to run), and both `run_id` serialization branches in one
    /// response.
    #[actix_web::test]
    async fn workflows_corpus_populated() {
        let tmp_a = repo_with_flow("workflows-alpha", FLOW_JSON);
        let tmp_b = repo_with_flow("workflows-beta", FLOW_JSON_WITH_RUN_ID);
        let app = workflows_service!(registry(&[("repo-alpha", &tmp_a), ("repo-beta", &tmp_b)]));
        let req = actix_web::test::TestRequest::get().uri("/api/workflows");
        dump("workflows", "populated", req, &app).await;
    }

    /// `?with_skipped=1` — freezes the `{entries, skipped}` envelope
    /// (ask A9, `ticket-report-skipped-workspaces`) across all three skip
    /// reasons plus a healthy repo in one response, so the golden pins both
    /// the full `reason` vocabulary and the "skipped ordered by repo, healthy
    /// repos never appear" rule at once.
    ///
    /// The `unreadable_root` fixture is a path joined onto a real `TempDir`
    /// but never created — deterministic and machine-independent (no
    /// dependency on any real absent path on the host), and its filesystem
    /// path never appears in the response body (the DTO carries only the
    /// registry key, `"repo-missing"`, never the underlying path), so the
    /// golden stays host-independent.
    #[actix_web::test]
    async fn workflows_corpus_skipped() {
        let tmp_unreadable = TempDir::new("workflows-unreadable-root");
        let missing_root = tmp_unreadable.path().join("does-not-exist");

        let tmp_no_planning = TempDir::new("workflows-no-planning-dir");

        let tmp_malformed = TempDir::new("workflows-malformed");
        write(
            &tmp_malformed
                .path()
                .join("planning")
                .join("bad-spec")
                .join("sdlc")
                .join("sdlc-flow-state.json"),
            "{ not json",
        );

        let tmp_healthy = repo_with_flow("workflows-healthy", FLOW_JSON);

        let app = workflows_service!(FileConfig {
            workspaces: Some(
                [
                    ("repo-missing".to_owned(), missing_root),
                    (
                        "repo-no-planning".to_owned(),
                        tmp_no_planning.path().to_path_buf(),
                    ),
                    (
                        "repo-malformed".to_owned(),
                        tmp_malformed.path().to_path_buf(),
                    ),
                    ("repo-healthy".to_owned(), tmp_healthy.path().to_path_buf(),),
                ]
                .into_iter()
                .collect::<HashMap<_, _>>(),
            ),
            ..Default::default()
        });
        let req = actix_web::test::TestRequest::get().uri("/api/workflows?with_skipped=1");
        dump("workflows", "skipped", req, &app).await;
    }
}

/// `GET /api/repos/{name}/handoff` corpus scenarios
/// (`ticket-contract-corpus-uncovered-routes`, task 2) — A3's `HandoffInfoDto`
/// (serve-api v0.18), plus **both** of the route's 404s.
///
/// The two 404s are the point of this module. `get_repo_handoff` fails
/// differently depending on *why* it failed — an unregistered workspace name is
/// `C005` (registry/config miss) while a registered repo whose `handoff.md` is
/// simply absent is `C002` (resource miss) — and a Surface that collapses the
/// two into one "not found" branch is wrong in a way only a frozen error
/// *code* catches. `src/serve/mod.rs`'s
/// `get_repo_handoff_unknown_repo_vs_missing_handoff_have_distinct_codes`
/// asserts the distinction; these goldens freeze the wire bodies carrying it.
///
/// See `board_scenarios`' doc comment for the shared fixture-app approach.
#[cfg(test)]
mod handoff_scenarios {
    use std::collections::HashMap;

    use actix_web::web;

    use super::dump;
    use super::fixtures::{TempDir, write};
    use crate::config::FileConfig;
    use crate::serve::handlers::status::get_repo_handoff;

    /// The same OKF-frontmatter handoff fixture `src/serve/mod.rs`'s A3 route
    /// tests seed (`src/serve/mod.rs:1124`) — authored, static content, so the
    /// frozen `title`/`body` pair is stable by construction.
    const HANDOFF_MD: &str = include_str!("status/fixtures/handoff_minimal.md");

    macro_rules! handoff_service {
        ($registry:expr) => {
            actix_web::test::init_service(
                actix_web::App::new()
                    .app_data(web::Data::new($registry))
                    .service(
                        web::resource("/api/repos/{name}/handoff")
                            .route(web::get().to(get_repo_handoff)),
                    ),
            )
            .await
        };
    }

    fn registry(name: &str, tmp: &TempDir) -> FileConfig {
        FileConfig {
            workspaces: Some(HashMap::from([(name.to_owned(), tmp.path().to_path_buf())])),
            ..Default::default()
        }
    }

    /// A registered repo whose `planning/handoff.md` exists.
    fn fixture_with_handoff() -> TempDir {
        let tmp = TempDir::new("handoff-populated");
        write(&tmp.path().join("planning").join("handoff.md"), HANDOFF_MD);
        tmp
    }

    /// A registered repo with a `planning/` tree but **no** `handoff.md` —
    /// resolves through the registry fine, then 404s on the file.
    fn fixture_without_handoff() -> TempDir {
        let tmp = TempDir::new("handoff-missing");
        write(&tmp.path().join("planning").join("status.md"), "# Status\n");
        tmp
    }

    #[actix_web::test]
    async fn handoff_corpus_populated() {
        let tmp = fixture_with_handoff();
        let app = handoff_service!(registry("repo-x", &tmp));
        let req = actix_web::test::TestRequest::get().uri("/api/repos/repo-x/handoff");
        dump("handoff", "populated", req, &app).await;
    }

    /// Registered repo, absent `handoff.md` → 404 + `C002`.
    #[actix_web::test]
    async fn handoff_corpus_missing() {
        let tmp = fixture_without_handoff();
        let app = handoff_service!(registry("repo-no-handoff", &tmp));
        let req = actix_web::test::TestRequest::get().uri("/api/repos/repo-no-handoff/handoff");
        dump("handoff", "missing", req, &app).await;
    }

    /// Unregistered workspace name → 404 + `C005`, a *different* code from the
    /// scenario above. The registry is deliberately non-empty so this golden
    /// pins "the name isn't in the registry", not "the registry is empty".
    #[actix_web::test]
    async fn handoff_corpus_unknown_repo() {
        let tmp = fixture_with_handoff();
        let app = handoff_service!(registry("repo-x", &tmp));
        let req = actix_web::test::TestRequest::get().uri("/api/repos/no-such-repo/handoff");
        dump("handoff", "unknown-repo", req, &app).await;
    }
}

/// Shared on-disk fixture brain corpus for the `epics` and `blocks-graph`
/// scenarios (`ticket-contract-corpus-uncovered-routes`, task 3).
///
/// Deliberately *one* corpus for both routes: `/api/epics` and
/// `/api/blocks/graph` project two different views of the same
/// `brain.toml` + `state.json` tree, and freezing them against a shared
/// fixture means a fixture edit shows up as a coordinated diff across both
/// routes' goldens rather than letting them silently describe different
/// worlds.
///
/// Shape mirrors `src/serve/mod.rs`'s `make_temp_epics_brain_root`:
/// - `brain.toml` — one `[[repos]]` entry, `bastion`, tier `core`,
///   `repo_path = "bastion"` (deliberately not `"."`, so `tier_scope_for`
///   can't mistake the HQ file for a tier-container self-entry).
/// - `planning/state.json` (`kind: "brain"`) — the HQ `epics[]` registry:
///   `bastion-surfaces` fully authored (**including `weight: 85`**, the field
///   serve-api v0.21 added and which these goldens exist to freeze) and
///   `brain-rag` carrying only the two required fields, so both the
///   all-optionals-present and all-optionals-absent `EpicDto` shapes are
///   frozen in one response.
/// - `bastion/planning/state.json` (`kind: "project"`) — four blocks covering
///   every `epic_progress` tally and every block-graph edge case: `BA.11.R`
///   (in_progress, epic member), `BA.11.S` (open, unmet dep on the
///   still-open `BA.11.R`), `BA.11.T` (open, met dep on the closed
///   `BA.11.K`), `BA.11.K` (closed).
///
/// Every date in the fixture is an authored literal (never live-clock), so
/// the pre-redaction values are already deterministic;
/// [`super::redact_value`]'s bare-calendar-date rule then neutralizes them
/// again as a backstop.
#[cfg(test)]
mod brain_fixture {
    use std::collections::HashMap;

    use super::fixtures::{TempDir, write};
    use crate::config::FileConfig;

    const BRAIN_TOML: &str = r#"[vocab]
layer = ["console"]
status = ["active"]

[crawl]
skip_dirs = ["target", ".git"]

[[repos]]
slug = "bastion"
tier = "core"
repo_path = "bastion"
status_file = "planning/status.md"
cache_doc = "docs/projects/bastion.md"
heading = "bastion"
"#;

    const HQ_STATE_JSON: &str = r#"{
  "repo": "hq",
  "kind": "brain",
  "updated": "2026-07-25",
  "focus": { "now": [], "next": [], "blocked": [] },
  "tracks": [],
  "epics": [
    {
      "slug": "bastion-surfaces",
      "title": "Bastion Surfaces",
      "description": "Cross-surface work",
      "status": "active",
      "weight": 85,
      "plan": "core/planning/master-plan.md",
      "repos": ["bastion"]
    },
    {
      "slug": "brain-rag",
      "title": "Brain RAG"
    }
  ]
}"#;

    /// The same tree with **no** `epics[]` key at all — the absent-HQ-registry
    /// case, which `get_epics` answers `200 []` rather than treating as an
    /// error.
    const HQ_STATE_JSON_NO_EPICS: &str = r#"{
  "repo": "hq",
  "kind": "brain",
  "updated": "2026-07-25",
  "focus": { "now": [], "next": [], "blocked": [] },
  "tracks": []
}"#;

    const LEAF_STATE_JSON: &str = r#"{
  "repo": "bastion",
  "kind": "project",
  "updated": "2026-07-25",
  "focus": { "now": [], "next": [], "blocked": [] },
  "tracks": [
    {
      "title": "Phase 11",
      "blocks": [
        {
          "id": "BA.11.R",
          "title": "Epic ranking enrichment",
          "status": "in_progress",
          "epics": ["bastion-surfaces"],
          "wave": 3,
          "priority": 1,
          "due": "2026-07-15"
        },
        {
          "id": "BA.11.S",
          "title": "Downstream consumer",
          "status": "open",
          "depends_on": [{ "type": "block", "repo": "bastion", "id": "BA.11.R" }],
          "epics": ["bastion-surfaces"]
        },
        {
          "id": "BA.11.T",
          "title": "Ready block",
          "status": "open",
          "depends_on": [{ "type": "block", "repo": "bastion", "id": "BA.11.K" }],
          "epics": ["brain-rag"]
        },
        {
          "id": "BA.11.K",
          "title": "Prior closed block",
          "status": "closed",
          "epics": ["bastion-surfaces"]
        }
      ]
    }
  ]
}"#;

    fn build(name: &str, hq_state: &str) -> TempDir {
        let tmp = TempDir::new(name);
        let root = tmp.path();
        write(&root.join("brain.toml"), BRAIN_TOML);
        write(&root.join("planning").join("state.json"), hq_state);
        write(
            &root.join("bastion").join("planning").join("state.json"),
            LEAF_STATE_JSON,
        );
        tmp
    }

    /// Brain corpus with a populated HQ `epics[]` registry.
    pub(super) fn populated(name: &str) -> TempDir {
        build(name, HQ_STATE_JSON)
    }

    /// Identical corpus minus the `epics[]` registry.
    pub(super) fn without_epics(name: &str) -> TempDir {
        build(name, HQ_STATE_JSON_NO_EPICS)
    }

    /// Registry pointing `default_workspace` at the fixture brain root —
    /// mirrors `src/serve/mod.rs`'s `registry_with_epics_fixture`.
    pub(super) fn registry(tmp: &TempDir) -> FileConfig {
        FileConfig {
            workspaces: Some(HashMap::from([(
                "brain-root".to_owned(),
                tmp.path().to_path_buf(),
            )])),
            default_workspace: Some("brain-root".to_owned()),
            ..Default::default()
        }
    }
}

/// `GET /api/epics` corpus scenarios (`ticket-contract-corpus-uncovered-routes`,
/// task 3) — the HQ cross-repo initiative registry.
///
/// **v0.21 `weight`**: `EpicDto` gained `weight: Option<u8>` in serve-api
/// v0.21 (merged hours before this ticket ran). Freezing the *post*-v0.21
/// shape is a headline requirement, so [`super::brain_fixture`]'s registry
/// authors `weight: 85` on `bastion-surfaces` and omits it entirely on
/// `brain-rag` — pinning both the present-value and the `null` serialization
/// in a single golden.
///
/// The route exposes no per-epic 404 (it takes no path/query params at all —
/// see `handlers/epics.rs`'s route docs), so the second scenario is the
/// *empty-registry* shape the task spec names as the alternative: an absent HQ
/// `epics[]` is `200 []`, not an error.
///
/// See `board_scenarios`' doc comment for the shared fixture-app approach.
#[cfg(test)]
mod epics_scenarios {
    use actix_web::web;

    use super::brain_fixture;
    use super::dump;
    use crate::serve::handlers::epics::get_epics;

    macro_rules! epics_service {
        ($registry:expr) => {
            actix_web::test::init_service(
                actix_web::App::new()
                    .app_data(web::Data::new($registry))
                    .service(web::resource("/api/epics").route(web::get().to(get_epics))),
            )
            .await
        };
    }

    #[actix_web::test]
    async fn epics_corpus_populated() {
        let tmp = brain_fixture::populated("epics-populated");
        let app = epics_service!(brain_fixture::registry(&tmp));
        let req = actix_web::test::TestRequest::get().uri("/api/epics");
        dump("epics", "populated", req, &app).await;
    }

    #[actix_web::test]
    async fn epics_corpus_empty() {
        let tmp = brain_fixture::without_epics("epics-empty");
        let app = epics_service!(brain_fixture::registry(&tmp));
        let req = actix_web::test::TestRequest::get().uri("/api/epics");
        dump("epics", "empty", req, &app).await;
    }

    /// Guard rail, not a golden: assert directly that the populated response
    /// actually carries the v0.21 `weight` field with the authored value. The
    /// golden freezes it too, but a golden can be regenerated into agreement
    /// with a *regression* — this assertion cannot, so it is what actually
    /// prevents the corpus from quietly re-freezing a pre-v0.21 shape.
    #[actix_web::test]
    async fn epics_corpus_carries_the_v0_21_weight_field() {
        let tmp = brain_fixture::populated("epics-weight-probe");
        let app = epics_service!(brain_fixture::registry(&tmp));
        let req = actix_web::test::TestRequest::get()
            .uri("/api/epics")
            .to_request();
        let resp = actix_web::test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = actix_web::test::read_body_json(resp).await;
        let entries = body.as_array().expect("body must be an array");
        assert_eq!(
            entries.len(),
            2,
            "expected two registry entries; got {body}"
        );
        assert_eq!(entries[0]["slug"], "bastion-surfaces");
        assert_eq!(
            entries[0]["weight"], 85,
            "the v0.21 `weight` field must reach the wire verbatim; got {body}"
        );
        assert!(
            entries[1]
                .as_object()
                .expect("entry must be an object")
                .contains_key("weight"),
            "an unauthored weight must still wire the key (as null), not omit it; got {body}"
        );
        assert!(
            entries[1]["weight"].is_null(),
            "an unauthored weight must wire as null; got {body}"
        );
    }
}

/// `GET /api/blocks/graph` corpus scenarios
/// (`ticket-contract-corpus-uncovered-routes`, task 3) — mev's enriched
/// block-graph export (serve-api v0.13).
///
/// **Fixed `max_nodes`**: the request pins `max_nodes=50` explicitly rather
/// than relying on the route's absent-param default (400). The default is a
/// *tunable* — changing it would silently rewrite this golden's `truncated`
/// flag and node set for reasons that have nothing to do with the wire
/// contract — so the corpus pins the input instead, exactly as the task spec
/// requires. 50 comfortably exceeds the fixture's four blocks, so the export
/// is complete (`truncated: false`) and the golden freezes the full
/// nodes/edges/topo shape rather than an arbitrary truncation.
///
/// `root` is the fixture's own temp path and is neutralized to `<TMP_PATH>` by
/// [`super::redact_value`]'s rule 3 — the one volatile value on this route.
///
/// See `board_scenarios`' doc comment for the shared fixture-app approach.
#[cfg(test)]
mod blocks_graph_scenarios {
    use actix_web::web;

    use super::brain_fixture;
    use super::dump;
    use crate::serve::handlers::block_graph::get_block_graph;

    macro_rules! block_graph_service {
        ($registry:expr) => {
            actix_web::test::init_service(
                actix_web::App::new()
                    .app_data(web::Data::new($registry))
                    .service(
                        web::resource("/api/blocks/graph").route(web::get().to(get_block_graph)),
                    ),
            )
            .await
        };
    }

    #[actix_web::test]
    async fn blocks_graph_corpus_populated() {
        let tmp = brain_fixture::populated("blocks-graph-populated");
        let app = block_graph_service!(brain_fixture::registry(&tmp));
        let req =
            actix_web::test::TestRequest::get().uri("/api/blocks/graph?scope=hq&max_nodes=50");
        dump("blocks-graph", "populated", req, &app).await;
    }
}

/// `GET /api/costs` corpus scenarios (`ticket-contract-corpus-uncovered-routes`,
/// task 4) — the **DB-free error shapes only** (serve-api v0.15).
///
/// # The populated `200`, budget-breached, empty, windowed, and `C009` shapes
/// `ticket-costs-200-contract-golden` added the missing seam:
/// [`crate::serve::handlers::costs::get_costs_with`] takes the runs-fetching
/// function as a compile-time generic parameter, so this module can dispatch
/// through a real actix `Service` — real window resolution, real
/// `Config::load`, real budget-gate evaluation, real `serde_json`
/// serialization — while supplying fixture `WorkflowRun`s in place of
/// `db::costs::fetch_all_runs`, with **no live Postgres**. The injection is
/// reachable only from this test module (`#[cfg(test)]`, linking directly
/// against `get_costs_with`); `get_costs` itself — the route production
/// mounts — has no env var, config key, or `app_data` slot that could swap in
/// a fixture at runtime. See `get_costs`'s own module docs for the full
/// argument.
///
/// The `400`/`C006` (`bad-window`) and `503`/`C005` (`no-database-url`)
/// scenarios below still dispatch through the real `get_costs` (they need no
/// fetch at all, or fail before ever calling it) — they are unaffected by the
/// seam and are unchanged from before this ticket.
///
/// # Env discipline (the reason this module is not like its siblings)
/// Every scenario below that reaches `Config::load` — which is every one of
/// them except `bad-window` — follows `crate::testsupport`'s lock discipline
/// to the letter:
/// - `lock_env()` **once**, at the top, held for the whole test;
/// - every mutation through `EnvVarGuard` (which takes the lock as a witness —
///   an unlocked mutation is a compile error), never a bare `remove_var`/`set_var`;
/// - `DotenvShadow` for the `.env` file, because `Config::load` calls
///   `dotenvy::dotenv()`, which walks *upward* from the cwd and would restore a
///   working `DATABASE_URL` (and, just as importantly, a developer's own
///   `BASTION_MAX_TOTAL_TOKENS`/`BASTION_MAX_COST_USD`) from an ancestor
///   checkout's `.env` the instant this module's guard unsets it;
/// - `XDG_CONFIG_HOME` pointed at an empty, unique temp dir so
///   `~/.config/bastion/config.toml` can never supply a `database_url` or a
///   budget cap either — the populated/breached/empty/windowed/db-error
///   goldens all read `Config::load`'s budget fields, so a developer's real
///   config file leaking a cap through would silently change `budget.breached`;
/// - `DATABASE_URL` set to an unroutable dummy
///   (`postgres://corpus@127.0.0.1:1/corpus`) so `Config::load` succeeds —
///   it is never actually connected to, since the fetch function is faked;
/// - **both** `BASTION_MAX_TOTAL_TOKENS` and `BASTION_MAX_COST_USD` are
///   explicitly pinned (unset for the no-cap scenarios, set for
///   `budget-breached`) rather than left to whatever the invoking shell
///   happens to export — proven by regenerating with both vars exported to
///   arbitrary values in the invoking shell and confirming no golden changes
///   (recorded in `tasks.md`'s `## Notes`);
/// - `dump_with(&CorpusConfig::from_env_locked(&env_lock), …)` rather than
///   `dump`, because `dump` → `CorpusConfig::from_env` would take the same
///   **non-reentrant** mutex a second time on this thread and deadlock.
#[cfg(test)]
mod costs_scenarios {
    use actix_web::web;
    use chrono::{Duration, Utc};

    use super::{CorpusConfig, dump, dump_with};
    use crate::db::workflows::{NodeState, RunStatus, WorkflowRun};
    use crate::serve::handlers::costs::{CostsQuery, get_costs, get_costs_with};
    use crate::testsupport::{DotenvShadow, EnvVarGuard, lock_env, unique_temp_dir};

    macro_rules! costs_service {
        () => {
            actix_web::test::init_service(
                actix_web::App::new()
                    .service(web::resource("/api/costs").route(web::get().to(get_costs))),
            )
            .await
        };
    }

    /// A fixture-backed `costs_service!` equivalent: routes `/api/costs`
    /// through the given async handler fn instead of the real `get_costs`, so
    /// the corpus can dispatch through real actix `Service`/`serde_json`
    /// machinery while injecting fixture rows.
    macro_rules! costs_fixture_service {
        ($handler:expr) => {
            actix_web::test::init_service(
                actix_web::App::new()
                    .service(web::resource("/api/costs").route(web::get().to($handler))),
            )
            .await
        };
    }

    // ── Fixture builders ────────────────────────────────────────────────────

    /// Build a single-node [`NodeState`] with reported (not tiktoken-derived)
    /// token counts — `input`/`output` stay `None` so
    /// `costs::exact_or_reported_tokens` falls back to `tokens_in`/`tokens_out`
    /// verbatim, giving exact, hand-checkable USD figures instead of a
    /// tokenizer-derived count.
    fn node(model: &str, tokens_in: u64, tokens_out: u64) -> NodeState {
        NodeState {
            id: "n1".to_owned(),
            name: "n1".to_owned(),
            status: RunStatus::Success,
            depends_on: vec![],
            input: None,
            output: None,
            error: None,
            tokens_in: Some(tokens_in),
            tokens_out: Some(tokens_out),
            model: Some(model.to_owned()),
            started_at: Some("2026-06-20T09:00:00Z".to_owned()),
            elapsed_secs: None,
        }
    }

    /// Build a [`WorkflowRun`] with an explicit `started_at`.
    fn run_at(
        id: &str,
        workflow_name: &str,
        started_at: &str,
        nodes: Vec<NodeState>,
    ) -> WorkflowRun {
        WorkflowRun {
            id: id.to_owned(),
            workflow_name: workflow_name.to_owned(),
            status: RunStatus::Success,
            budget_halt: None,
            nodes,
            started_at: Some(started_at.to_owned()),
            elapsed_secs: None,
        }
    }

    /// Three runs across two workflow names, mixed priced (`claude-haiku-4-5`,
    /// $1.00 in / $5.00 out per MTok) and unpriced (`some-unpriced-model`)
    /// models. Token counts are whole-MTok multiples so every USD figure is
    /// an exact `f64` (no `0.1 + 0.2`-style artifacts):
    /// - `content-pipeline` run 1: 1,000,000 in / 200,000 out → $1.00 + $1.00 = $2.00
    /// - `content-pipeline` run 2: 500,000 in / 0 out → $0.50
    ///   → `content-pipeline` totals: 2 runs, $2.50
    /// - `research-pipeline` run 1: unpriced model → $0.00, surfaces in
    ///   `unpriced_models`
    ///
    /// Row order (usd descending): `content-pipeline` ($2.50), then
    /// `research-pipeline` ($0.00). Grand total: $2.50.
    fn populated_runs() -> Vec<WorkflowRun> {
        vec![
            run_at(
                "run-content-1",
                "content-pipeline",
                "2026-06-20T09:00:00Z",
                vec![node("claude-haiku-4-5", 1_000_000, 200_000)],
            ),
            run_at(
                "run-content-2",
                "content-pipeline",
                "2026-06-20T10:00:00Z",
                vec![node("claude-haiku-4-5", 500_000, 0)],
            ),
            run_at(
                "run-research-1",
                "research-pipeline",
                "2026-06-20T11:00:00Z",
                vec![node("some-unpriced-model", 1_000, 1_000)],
            ),
        ]
    }

    /// Two runs relative to `Utc::now()` at fixture-build time (Notes point
    /// 5: no fixed calendar date, so the in/out-of-window decision is stable
    /// at any wall clock) — one 1 day old (inside a `7d` window), one 40 days
    /// old (outside it). Same priced model as [`populated_runs`] so the USD
    /// figures stay exact.
    fn windowed_runs() -> Vec<WorkflowRun> {
        let now = Utc::now();
        let in_window = (now - Duration::days(1)).to_rfc3339();
        let out_of_window = (now - Duration::days(40)).to_rfc3339();
        vec![
            run_at(
                "run-in-window",
                "in-window-pipeline",
                &in_window,
                vec![node("claude-haiku-4-5", 1_000_000, 0)],
            ),
            run_at(
                "run-out-of-window",
                "out-of-window-pipeline",
                &out_of_window,
                vec![node("claude-haiku-4-5", 1_000_000, 0)],
            ),
        ]
    }

    // ── Fixture-backed handlers ──────────────────────────────────────────────

    async fn populated_handler(query: web::Query<CostsQuery>) -> actix_web::HttpResponse {
        let runs = populated_runs();
        get_costs_with(query, move |_db_url| async move { Ok(runs) }).await
    }

    async fn budget_breached_handler(query: web::Query<CostsQuery>) -> actix_web::HttpResponse {
        let runs = populated_runs();
        get_costs_with(query, move |_db_url| async move { Ok(runs) }).await
    }

    async fn empty_handler(query: web::Query<CostsQuery>) -> actix_web::HttpResponse {
        get_costs_with(
            query,
            |_db_url| async move { Ok(Vec::<WorkflowRun>::new()) },
        )
        .await
    }

    async fn windowed_handler(query: web::Query<CostsQuery>) -> actix_web::HttpResponse {
        let runs = windowed_runs();
        get_costs_with(query, move |_db_url| async move { Ok(runs) }).await
    }

    async fn db_error_handler(query: web::Query<CostsQuery>) -> actix_web::HttpResponse {
        get_costs_with(query, |_db_url| async move {
            Err::<Vec<WorkflowRun>, _>(anyhow::anyhow!("simulated database failure"))
        })
        .await
    }

    /// Common env pinning every scenario below (except `bad-window`) shares:
    /// `DATABASE_URL` (dummy, unroutable), `XDG_CONFIG_HOME` (empty temp
    /// dir), and `DotenvShadow`. Caller still owns pinning the two
    /// `BASTION_MAX_*` vars explicitly (unset or set) per scenario — that
    /// choice is scenario-specific and must not have a shared default.
    struct PinnedCorpusEnv {
        _dotenv: DotenvShadow,
        _database_url: EnvVarGuard,
        _xdg: EnvVarGuard,
        empty_config_home: std::path::PathBuf,
    }

    impl PinnedCorpusEnv {
        fn new(lock: &crate::testsupport::EnvLock, label: &str) -> Self {
            let dotenv = DotenvShadow::new(lock, label);
            let database_url =
                EnvVarGuard::set(lock, "DATABASE_URL", "postgres://corpus@127.0.0.1:1/corpus");
            let empty_config_home = unique_temp_dir(&format!("bastion-corpus-costs-{label}"));
            std::fs::create_dir_all(&empty_config_home).unwrap();
            let xdg = EnvVarGuard::set(
                lock,
                "XDG_CONFIG_HOME",
                empty_config_home.to_str().expect("temp path must be UTF-8"),
            );
            Self {
                _dotenv: dotenv,
                _database_url: database_url,
                _xdg: xdg,
                empty_config_home,
            }
        }
    }

    impl Drop for PinnedCorpusEnv {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.empty_config_home);
        }
    }

    /// Unparseable `?window=` → 400 + `C006`.
    ///
    /// No env handling needed, and that is itself part of the contract:
    /// `resolve_window` runs *before* `Config::load`, so this response is
    /// produced with no database access and regardless of whatever
    /// `DATABASE_URL` happens to hold. Plain `dump` is therefore correct here —
    /// this test holds no lock.
    #[actix_web::test]
    async fn costs_corpus_bad_window() {
        let app = costs_service!();
        let req = actix_web::test::TestRequest::get().uri("/api/costs?window=nonsense");
        dump("costs", "bad-window", req, &app).await;
    }

    /// `DATABASE_URL` genuinely absent → 503 + `C005`.
    #[actix_web::test]
    async fn costs_corpus_no_database_url() {
        // ONE acquisition, at the top, held for the whole test.
        let env_lock = lock_env();

        // Neutralize `dotenvy`'s upward `.env` walk before unsetting the var.
        let _dotenv_shadow = DotenvShadow::new(&env_lock, "corpus_costs_c005");
        let _database_url = EnvVarGuard::unset(&env_lock, "DATABASE_URL");

        // `Config::load` also consults `~/.config/bastion/config.toml` (via
        // XDG_CONFIG_HOME, then HOME) as a lower-precedence source for
        // `database_url`. Point XDG_CONFIG_HOME at an empty temp dir so a
        // developer's real config file can never supply one — without this the
        // golden would be machine-dependent, which is precisely what a checked-in
        // corpus must never be.
        let empty_config_home = unique_temp_dir("bastion-corpus-costs-xdg");
        std::fs::create_dir_all(&empty_config_home).unwrap();
        let _xdg = EnvVarGuard::set(
            &env_lock,
            "XDG_CONFIG_HOME",
            empty_config_home.to_str().expect("temp path must be UTF-8"),
        );

        // `from_env_locked`, not `from_env`/`dump`: we already hold the lock and
        // it is not reentrant.
        let config = CorpusConfig::from_env_locked(&env_lock);

        let app = costs_service!();
        let req = actix_web::test::TestRequest::get().uri("/api/costs");
        dump_with(&config, "costs", "no-database-url", req, &app).await;

        let _ = std::fs::remove_dir_all(&empty_config_home);
    }

    /// Populated `200` — 3 runs across 2 workflow names, mixed priced/unpriced
    /// models, `?window=all`, both budget caps unset. Freezes `rows[]`
    /// ordering (usd descending), the `"TOTAL"` totals sentinel,
    /// `unpriced_models[]`, and `budget` with `null` caps and
    /// `breached: false` / `breach: null`.
    #[actix_web::test]
    async fn costs_corpus_populated() {
        let env_lock = lock_env();
        let _pinned = PinnedCorpusEnv::new(&env_lock, "populated");
        let _max_total_tokens = EnvVarGuard::unset(&env_lock, "BASTION_MAX_TOTAL_TOKENS");
        let _max_cost_usd = EnvVarGuard::unset(&env_lock, "BASTION_MAX_COST_USD");

        let config = CorpusConfig::from_env_locked(&env_lock);
        let app = costs_fixture_service!(populated_handler);
        let req = actix_web::test::TestRequest::get().uri("/api/costs?window=all");
        dump_with(&config, "costs", "populated", req, &app).await;
    }

    /// Budget-breached `200` — same rows as [`costs_corpus_populated`], but
    /// `BASTION_MAX_COST_USD` is pinned below the fixture total ($2.50), so
    /// `budget.breached == true` and `budget.breach.cap == "max_cost_usd"`
    /// (the exact `Cap::as_str` wire string consumers switch on).
    #[actix_web::test]
    async fn costs_corpus_budget_breached() {
        let env_lock = lock_env();
        let _pinned = PinnedCorpusEnv::new(&env_lock, "budget-breached");
        let _max_total_tokens = EnvVarGuard::unset(&env_lock, "BASTION_MAX_TOTAL_TOKENS");
        let _max_cost_usd = EnvVarGuard::set(&env_lock, "BASTION_MAX_COST_USD", "2.0");

        let config = CorpusConfig::from_env_locked(&env_lock);
        let app = costs_fixture_service!(budget_breached_handler);
        let req = actix_web::test::TestRequest::get().uri("/api/costs?window=all");
        dump_with(&config, "costs", "budget-breached", req, &app).await;
    }

    /// Empty `200` — the fetch fn returns `vec![]`. Zero rows is `200` with
    /// `rows: []` and a zeroed `"TOTAL"` totals row, never a 404 — mirrors the
    /// `workflows__empty` contract.
    #[actix_web::test]
    async fn costs_corpus_empty() {
        let env_lock = lock_env();
        let _pinned = PinnedCorpusEnv::new(&env_lock, "empty");
        let _max_total_tokens = EnvVarGuard::unset(&env_lock, "BASTION_MAX_TOTAL_TOKENS");
        let _max_cost_usd = EnvVarGuard::unset(&env_lock, "BASTION_MAX_COST_USD");

        let config = CorpusConfig::from_env_locked(&env_lock);
        let app = costs_fixture_service!(empty_handler);
        let req = actix_web::test::TestRequest::get().uri("/api/costs?window=all");
        dump_with(&config, "costs", "empty", req, &app).await;
    }

    /// Windowed `200` — `?window=7d`, one fixture row inside the window (1
    /// day old), one outside it (40 days old). Freezes that the window filter
    /// reaches the wire (only the in-window row's workflow appears) and that
    /// `window` echoes `"7d"`, not `"all"`.
    #[actix_web::test]
    async fn costs_corpus_windowed() {
        let env_lock = lock_env();
        let _pinned = PinnedCorpusEnv::new(&env_lock, "windowed");
        let _max_total_tokens = EnvVarGuard::unset(&env_lock, "BASTION_MAX_TOTAL_TOKENS");
        let _max_cost_usd = EnvVarGuard::unset(&env_lock, "BASTION_MAX_COST_USD");

        let config = CorpusConfig::from_env_locked(&env_lock);
        let app = costs_fixture_service!(windowed_handler);
        let req = actix_web::test::TestRequest::get().uri("/api/costs?window=7d");
        dump_with(&config, "costs", "windowed", req, &app).await;
    }

    /// `503` + `C009` — the fetch fn returns `Err`, exercising the branch
    /// that had no test (and no corpus golden) before this ticket. The
    /// message text is fixture-derived, not from a real Postgres failure; the
    /// golden pins the `{code, message}` shape, and consumers are documented
    /// to switch on `code` only.
    #[actix_web::test]
    async fn costs_corpus_db_error() {
        let env_lock = lock_env();
        let _pinned = PinnedCorpusEnv::new(&env_lock, "db-error");
        let _max_total_tokens = EnvVarGuard::unset(&env_lock, "BASTION_MAX_TOTAL_TOKENS");
        let _max_cost_usd = EnvVarGuard::unset(&env_lock, "BASTION_MAX_COST_USD");

        let config = CorpusConfig::from_env_locked(&env_lock);
        let app = costs_fixture_service!(db_error_handler);
        let req = actix_web::test::TestRequest::get().uri("/api/costs?window=all");
        dump_with(&config, "costs", "db-error", req, &app).await;
    }
}

/// `GET /health` corpus scenarios (`BA.22.A`, task 1) — the liveness probe.
///
/// The handler takes no `app_data` — no fixture/registry is needed, so this
/// is the simplest scenario module in the corpus. `health()` is widened to
/// `pub(crate)` in `src/serve/mod.rs` (the one permitted production-adjacent
/// edit for this block) so this module can dispatch the real handler rather
/// than a duplicate — the module doc's "real dispatch only" rule.
#[cfg(test)]
mod health_scenarios {
    use actix_web::web;

    use super::dump;
    use crate::serve::health;

    macro_rules! health_service {
        () => {
            actix_web::test::init_service(
                actix_web::App::new()
                    .service(web::resource("/health").route(web::get().to(health))),
            )
            .await
        };
    }

    #[actix_web::test]
    async fn health_corpus_ok() {
        let app = health_service!();
        let req = actix_web::test::TestRequest::get().uri("/health");
        dump("health", "ok", req, &app).await;
    }
}

/// `GET /api/repos` corpus scenarios (`BA.22.A`, task 1) — the workspace
/// registry summary (`handlers::status::list_repos`).
///
/// `repos__populated` seeds one workspace entry with a `planning/status.md`
/// so the golden freezes the full `{name, now, has_handoff}` shape rather
/// than an all-default row; `repos__empty` uses a registry with no
/// workspaces at all, exercising the route's documented `[]` degrade path.
///
/// Neither scenario's response body embeds the fixture's own temp-dir
/// path — `RepoSummaryDto` only wires `name`/`now`/`has_handoff`, none of
/// which is derived from the workspace root string itself — so no
/// [`super::redact_value`] rule applies to this route.
#[cfg(test)]
mod repos_scenarios {
    use std::collections::HashMap;

    use actix_web::web;

    use super::dump;
    use super::fixtures::{TempDir, write};
    use crate::config::FileConfig;
    use crate::serve::handlers::status::list_repos;

    macro_rules! repos_service {
        ($registry:expr) => {
            actix_web::test::init_service(
                actix_web::App::new()
                    .app_data(web::Data::new($registry))
                    .service(web::resource("/api/repos").route(web::get().to(list_repos))),
            )
            .await
        };
    }

    const STATUS_MD: &str = r#"---
type: ProjectStatus
title: Fixture Status
description: Contract-corpus fixture status.md for the repos__populated golden.
now: "BA.22.A in progress — contract-corpus goldens"
next: "n/a"
blocked: []
---

# Status
"#;

    fn registry_with_one(tmp: &TempDir) -> FileConfig {
        write(&tmp.path().join("planning/status.md"), STATUS_MD);
        let mut workspaces = HashMap::new();
        workspaces.insert("bastion".to_string(), tmp.path().to_path_buf());
        FileConfig {
            workspaces: Some(workspaces),
            ..Default::default()
        }
    }

    #[actix_web::test]
    async fn repos_corpus_populated() {
        let tmp = TempDir::new("repos-populated");
        let registry = registry_with_one(&tmp);
        let app = repos_service!(registry);
        let req = actix_web::test::TestRequest::get().uri("/api/repos");
        dump("repos", "populated", req, &app).await;
    }

    #[actix_web::test]
    async fn repos_corpus_empty() {
        let registry = FileConfig::default();
        let app = repos_service!(registry);
        let req = actix_web::test::TestRequest::get().uri("/api/repos");
        dump("repos", "empty", req, &app).await;
    }
}
