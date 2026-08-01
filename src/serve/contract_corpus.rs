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

use std::path::PathBuf;

use actix_web::body::MessageBody;
use actix_web::dev::{Service, ServiceResponse};
use actix_web::test;

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

/// Parse a captured `(status_code, raw response body bytes)` pair into the
/// golden JSON value `{status_code, body}`.
///
/// Fails loudly (panics) rather than silently skipping when the body is not
/// valid JSON — a non-JSON body means either the route doesn't serialize to
/// JSON (out of scope for this corpus) or the handler under test returned
/// something unexpected. Either way it must never be swallowed into an
/// empty/null golden that looks like a passing scenario.
fn build_golden(status_code: u16, body_bytes: &[u8]) -> serde_json::Value {
    let body: serde_json::Value = serde_json::from_slice(body_bytes).unwrap_or_else(|e| {
        panic!(
            "contract corpus dump: response body for status {status_code} is not valid JSON \
             ({e}); refusing to write/verify a golden from a non-JSON body. Raw body: {:?}",
            String::from_utf8_lossy(body_bytes)
        )
    });
    serde_json::json!({
        "status_code": status_code,
        "body": body,
    })
}

/// Write the golden to disk (GENERATE mode) or verify it against the
/// checked-in copy (VERIFY mode — the default, so a normal `cargo test` run
/// enforces the corpus rather than rewriting it).
fn write_or_verify(route_name: &str, scenario: &str, golden: &serde_json::Value) {
    let path = golden_path(route_name, scenario);
    if should_write() {
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
    let resp = test::call_service(app, req.to_request()).await;
    let status_code = resp.status().as_u16();
    let body_bytes = test::read_body(resp).await;
    let golden = build_golden(status_code, &body_bytes);
    write_or_verify(route_name, scenario, &golden);
}

#[cfg(test)]
mod harness_tests {
    // NOTE: deliberately *not* `use super::*;` and deliberately *not*
    // `use actix_web::test;` — importing the `actix_web::test` module
    // unqualified shadows/collides with the std-prelude `#[test]` attribute
    // macro used below. `actix_web::test::*` items are referenced with their
    // full path (`actix_web::test::TestRequest`, etc.) instead.
    use super::{
        CORPUS_DIR_OVERRIDE_ENV_VAR, DUMP_ENV_VAR, build_golden, dump, golden_path, should_write,
    };
    use actix_web::{App, HttpResponse, web};
    use std::path::PathBuf;

    /// RAII guard that sets an env var for the duration of a test and always
    /// restores the previous value on drop (including on panic/unwind), so a
    /// `#[should_panic]` test never leaks its override into a sibling test
    /// within the same process.
    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            // SAFETY: test-only, single-threaded-per-test under `cargo nextest`
            // (each test runs in its own process); mirrors the existing
            // `unsafe { std::env::set_var(..) }` pattern already used by
            // `src/serve/mod.rs`'s test module for `DATABASE_URL`.
            unsafe { std::env::set_var(key, value) };
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.previous {
                    Some(v) => std::env::set_var(self.key, v),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }

    #[test]
    fn golden_path_builds_expected_route_scenario_filename() {
        let _dir_guard = EnvVarGuard::set(CORPUS_DIR_OVERRIDE_ENV_VAR, "types/contract-corpus");
        let path = golden_path("runs", "empty");
        assert_eq!(
            path,
            PathBuf::from("types/contract-corpus/runs__empty.json")
        );
    }

    #[test]
    fn golden_path_respects_corpus_dir_override() {
        let _dir_guard = EnvVarGuard::set(CORPUS_DIR_OVERRIDE_ENV_VAR, "/tmp/some-override-dir");
        let path = golden_path("board", "hq");
        assert_eq!(path, PathBuf::from("/tmp/some-override-dir/board__hq.json"));
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
        let previous = std::env::var(DUMP_ENV_VAR).ok();
        unsafe { std::env::remove_var(DUMP_ENV_VAR) };
        let result = should_write();
        unsafe {
            match previous {
                Some(v) => std::env::set_var(DUMP_ENV_VAR, v),
                None => std::env::remove_var(DUMP_ENV_VAR),
            }
        }
        assert!(
            !result,
            "should_write() must default to VERIFY (false) when unset"
        );
    }

    #[test]
    fn should_write_is_true_only_for_exact_value_one() {
        {
            let _guard = EnvVarGuard::set(DUMP_ENV_VAR, "1");
            assert!(should_write());
        }
        {
            let _guard = EnvVarGuard::set(DUMP_ENV_VAR, "true");
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
        let tmp_dir = std::env::temp_dir().join(format!(
            "bastion-contract-corpus-test-{}-{}",
            std::process::id(),
            "generate"
        ));
        let _ = std::fs::remove_dir_all(&tmp_dir);
        let _dump_guard = EnvVarGuard::set(DUMP_ENV_VAR, "1");
        let _dir_guard = EnvVarGuard::set(CORPUS_DIR_OVERRIDE_ENV_VAR, tmp_dir.to_str().unwrap());

        let app = actix_web::test::init_service(App::new().route(
            "/x",
            web::get().to(|| async {
                HttpResponse::NotFound().json(serde_json::json!({"code": "C002"}))
            }),
        ))
        .await;

        let req = actix_web::test::TestRequest::get().uri("/x");
        dump("widget", "not-found", req, &app).await;

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
        let tmp_dir = std::env::temp_dir().join(format!(
            "bastion-contract-corpus-test-{}-{}",
            std::process::id(),
            "verify-missing"
        ));
        let _ = std::fs::remove_dir_all(&tmp_dir);
        let _ = std::fs::create_dir_all(&tmp_dir);
        // BASTION_DUMP_CORPUS deliberately left unset: VERIFY mode.
        let _dump_guard = EnvVarGuard::set(DUMP_ENV_VAR, "0");
        let _dir_guard = EnvVarGuard::set(CORPUS_DIR_OVERRIDE_ENV_VAR, tmp_dir.to_str().unwrap());

        let app = actix_web::test::init_service(App::new().route(
            "/y",
            web::get().to(|| async { HttpResponse::Ok().json(serde_json::json!({})) }),
        ))
        .await;

        let req = actix_web::test::TestRequest::get().uri("/y");
        dump("widget", "no-golden-yet", req, &app).await;

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
}
