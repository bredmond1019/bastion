# Task Breakdown — Phase 0, Block A: Foundation setup

## Source Spec
`planning/phase0-blockA/tasks.md`

## Goal
Verify the Rust toolchain compiles the scaffold and implement `bastion status` end-to-end — probe PostgreSQL and the FastAPI health endpoint, print a reachability summary, and ship a `.env.example`.

## How to Use
Work top to bottom. Each sub-step is a single atomic action. Run the inline **Verify**
checks as you go — do not batch them at the end. Each check must pass before continuing.

Standing-rule reminders baked into the steps below: **no emoji in source/docs** (the master-plan's `✓`
glyphs are illustrative, not literal output — render words like `reachable`/`unreachable`); **no
`panic!`/`unwrap` on missing env** (typed errors only); **every block ships with tests**; all four
gated checks (`fmt`, `clippy`, `test`, `build`) must be left green and must pass **offline**
(live `reachable` output is a manual, non-gating acceptance step).

---

## Steps

### Step 1: Toolchain + config plumbing

#### 1.1 Record the baseline build/test state
**Action:** run command (record results in the spec's Notes section afterward)
```
cargo build && cargo test
```
Capture pass/fail + any warnings as the pre-change baseline. Do not fix unrelated scaffold issues yet.

**Verify:** `cargo build` → exit 0 (scaffold compiles; `todo!()` stubs are allowed — they only panic at runtime, not compile time).

#### 1.2 Check for existing `Config::load()` call sites before changing its return type
**File:** (search only)
**Action:** run command
```
grep -rn "Config::load\|config::Config" src/
```
Step 1.3 changes `Config::load`'s return type from `anyhow::Result<Self>` to `Result<Self, ConfigError>`. Confirm the only consumer is `run::status()` (added in 3.2). If other call sites exist, they must use `?` inside an `anyhow`-returning fn (anyhow absorbs `ConfigError` automatically) — note any that don't.

#### 1.3 Rewrite `src/config.rs` with a typed error and a hermetically-testable parser
**File:** `src/config.rs`
**Action:** replace the whole file
Replace the current `anyhow`-based `load()` with a typed `ConfigError` (via `thiserror`, already a dependency) and split parsing into a pure `from_vars` function so tests never mutate process env:
```rust
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum ConfigError {
    #[error("{0} must be set (point to the Python orchestrator's PostgreSQL)")]
    MissingVar(&'static str),
}

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub api_base_url: String,
    pub poll_interval_secs: u64,
}

impl Config {
    /// Default FastAPI base URL — orchestrator `/health` lives on port 8080
    /// (recon 2026-06-18; the scaffold's old 8000 default was wrong).
    const DEFAULT_API_URL: &'static str = "http://localhost:8080";

    pub fn load() -> Result<Self, ConfigError> {
        dotenvy::dotenv().ok();
        Self::from_vars(
            std::env::var("DATABASE_URL").ok(),
            std::env::var("BASTION_API_URL").ok(),
            std::env::var("BASTION_POLL_INTERVAL").ok(),
        )
    }

    /// Pure parser — no env access, so unit tests can call it directly.
    pub fn from_vars(
        database_url: Option<String>,
        api_base_url: Option<String>,
        poll_interval: Option<String>,
    ) -> Result<Self, ConfigError> {
        let database_url = database_url.ok_or(ConfigError::MissingVar("DATABASE_URL"))?;
        let api_base_url =
            api_base_url.unwrap_or_else(|| Self::DEFAULT_API_URL.to_string());
        let poll_interval_secs = poll_interval
            .and_then(|s| s.parse().ok())
            .unwrap_or(2);
        Ok(Self { database_url, api_base_url, poll_interval_secs })
    }
}
```
Note: `DATABASE_URL` is required (typed error when absent, never `unwrap`); `BASTION_API_URL` and `BASTION_POLL_INTERVAL` fall back to defaults. The config unit tests are added in 4.1 (same file).

#### 1.4 Add `.env.example` at the repo root
**File:** `.env.example` (repo root, alongside `Cargo.toml`)
**Action:** create
Use the recon-corrected real values (port 8080, db name `postgres`), one comment line per var:
```
# PostgreSQL connection for the Python orchestrator (bastion reads it READ-ONLY).
DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres

# Base URL of the orchestrator's FastAPI service (its /health is on port 8080).
BASTION_API_URL=http://localhost:8080

# DB/keyboard poll interval in seconds for `bastion monitor` (Phase 1; default 2).
BASTION_POLL_INTERVAL=2
```

**Verify:** `cargo build` → exit 0 (config.rs compiles with the new `ConfigError` type).

---

### Step 2: Service health probes

#### 2.1 Replace `ApiClient::health()` with a typed reachable/unreachable result
**File:** `src/api/client.rs`
**Action:** edit — add types + rewrite `health()`; leave `trigger_workflow`/`rerun_node` `todo!()` stubs untouched (Phase 3/4)
Add imports at the top of the file:
```rust
use serde::Deserialize;
use std::time::Duration;
```
Add these types above the `impl ApiClient` block:
```rust
/// Outcome of probing the orchestrator's `/health` endpoint.
/// Unreachable is a normal outcome (not an `Err`) so `bastion status` never fails on it.
#[derive(Debug, Clone, PartialEq)]
pub enum ApiStatus {
    Reachable { status: String, version: String },
    Unreachable(String),
}

/// Orchestrator `/health` body — `{ "status": ..., "version": ... }` (recon 2026-06-18).
#[derive(Debug, Deserialize)]
struct HealthBody {
    status: String,
    version: String,
}
```
Replace the current `async fn health(&self) -> Result<bool>` stub with:
```rust
    pub async fn health(&self) -> ApiStatus {
        let url = format!("{}/health", self.base_url.trim_end_matches('/'));
        let resp = self
            .client
            .get(&url)
            .timeout(Duration::from_secs(2))
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => match r.json::<HealthBody>().await {
                Ok(body) => ApiStatus::Reachable {
                    status: body.status,
                    version: body.version,
                },
                Err(e) => ApiStatus::Unreachable(format!("invalid health body: {e}")),
            },
            Ok(r) => ApiStatus::Unreachable(format!("HTTP {}", r.status())),
            Err(e) => ApiStatus::Unreachable(e.to_string()),
        }
    }
```
The short 2s timeout makes an absent service fail fast instead of hanging. `reqwest`'s `json`
feature is already enabled in `Cargo.toml`. If `use anyhow::Result;` becomes unused after this
edit (the two remaining stubs still return `Result`), keep it.

#### 2.2 Create `src/db/health.rs` — a read-only connection probe
**File:** `src/db/health.rs`
**Action:** create
Honors D2 (observer, never writer): opens a pool and runs `SELECT 1` only. Worker-count /
queue-depth are deferred (they live in Redis, out of bastion's configured scope per D2) — so the
probe reports DB reachability only.
```rust
// Read-only PostgreSQL reachability probe for `bastion status`.
// Observer only (D2): opens a pool and runs `SELECT 1`. No writes.

use sqlx::postgres::PgPoolOptions;
use std::time::Duration;

/// Outcome of probing PostgreSQL. Unreachable is a normal outcome, not an `Err`.
#[derive(Debug, Clone, PartialEq)]
pub enum DbStatus {
    Reachable,
    Unreachable(String),
}

pub async fn probe(db_url: &str) -> DbStatus {
    let pool = match PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(2))
        .connect(db_url)
        .await
    {
        Ok(p) => p,
        Err(e) => return DbStatus::Unreachable(e.to_string()),
    };

    match sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&pool)
        .await
    {
        Ok(_) => DbStatus::Reachable,
        Err(e) => DbStatus::Unreachable(e.to_string()),
    }
}
```

#### 2.3 Register the new `health` module
**File:** `src/db/mod.rs`
**Action:** add a line
Current contents are `pub mod workflows;` / `pub mod costs;`. Add:
```rust
pub mod health;
```

**Verify:** `cargo build` → exit 0 (both probes compile; `ApiStatus` / `DbStatus` resolve).

---

### Step 3: `bastion status` command + output

#### 3.1 Confirm the `status` subcommand exists (no change needed)
**File:** `src/cli.rs`
**Action:** verify only
`Commands::Status` already exists at `src/cli.rs:48` (`/// Quick stack health check (non-TUI)` →
`Status,`) with no args. No edit required — note this in passing; do not add a duplicate.

#### 3.2 Implement `run::status()` and a pure `render_status()` renderer
**File:** `src/run/mod.rs`
**Action:** edit — replace the `status()` `todo!()` body and add `render_status`; leave
`trigger()` `todo!()` untouched (Phase 3)
Replace the top-of-file imports/comment block's `use anyhow::Result;` region so it reads:
```rust
// `bastion run <workflow>` — trigger a workflow via FastAPI.
// `bastion status`         — quick stack health check (non-TUI).

use anyhow::Result;

use crate::api::client::{ApiClient, ApiStatus};
use crate::config::Config;
use crate::db::health::{self, DbStatus};
```
Replace the `status()` stub with:
```rust
pub async fn status() -> Result<()> {
    let config = Config::load()?;
    let db = health::probe(&config.database_url).await;
    let api = ApiClient::new(&config.api_base_url).health().await;
    println!("{}", render_status(&db, &api));
    Ok(())
}

/// Pure renderer — one row per service. Kept side-effect-free so it is unit-testable
/// without a live DB/API. No emoji (words only) per the project's source/docs rule.
fn render_status(db: &DbStatus, api: &ApiStatus) -> String {
    let db_line = match db {
        DbStatus::Reachable => "DB   reachable".to_string(),
        DbStatus::Unreachable(_) => "DB   unreachable".to_string(),
    };
    let api_line = match api {
        ApiStatus::Reachable { version, .. } => {
            format!("API  reachable (version {version})")
        }
        ApiStatus::Unreachable(_) => "API  unreachable".to_string(),
    };
    format!("{db_line}\n{api_line}")
}
```
`Config::load()` returns `Result<_, ConfigError>`; the `?` here converts into `anyhow::Error`
automatically (so `status()` keeps its `anyhow::Result<()>` signature, matching `main.rs`).
Worker-count / queue-depth rows are intentionally omitted (deferred per D2).

#### 3.3 Confirm clap dispatch wires `status` (no change needed)
**File:** `src/main.rs`
**Action:** verify only
`Commands::Status => run::status().await,` already exists at `src/main.rs:26`. No edit required.

**Verify:** `cargo run -- status` → exits cleanly (exit 0) printing `DB   unreachable` and
`API  unreachable` when no DB/API is running locally (it must NOT panic). Also run
`cargo run -- --help` → exit 0, lists the `status` subcommand.

---

### Step 4: Unit tests (offline-passing)

#### 4.1 Add config parsing tests to `src/config.rs`
**File:** `src/config.rs`
**Action:** append a test module at the end of the file
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_when_all_vars_present() {
        let c = Config::from_vars(
            Some("postgres://localhost/db".into()),
            Some("http://localhost:9000".into()),
            Some("5".into()),
        )
        .expect("should parse");
        assert_eq!(c.database_url, "postgres://localhost/db");
        assert_eq!(c.api_base_url, "http://localhost:9000");
        assert_eq!(c.poll_interval_secs, 5);
    }

    #[test]
    fn applies_defaults_for_optional_vars() {
        let c = Config::from_vars(Some("postgres://localhost/db".into()), None, None)
            .expect("should parse");
        assert_eq!(c.api_base_url, "http://localhost:8080");
        assert_eq!(c.poll_interval_secs, 2);
    }

    #[test]
    fn missing_database_url_is_typed_error_not_panic() {
        let err = Config::from_vars(None, None, None).unwrap_err();
        assert_eq!(err, ConfigError::MissingVar("DATABASE_URL"));
    }
}
```
These call `from_vars` directly, so they touch no process env and run hermetically in parallel.

#### 4.2 Add status-render tests to `src/run/mod.rs`
**File:** `src/run/mod.rs`
**Action:** append a test module at the end of the file
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_reachable_services_with_version() {
        let out = render_status(
            &DbStatus::Reachable,
            &ApiStatus::Reachable {
                status: "ok".into(),
                version: "1.2.3".into(),
            },
        );
        assert!(out.contains("DB   reachable"), "got: {out}");
        assert!(out.contains("API  reachable (version 1.2.3)"), "got: {out}");
    }

    #[test]
    fn renders_unreachable_services_without_panicking() {
        let out = render_status(
            &DbStatus::Unreachable("connection refused".into()),
            &ApiStatus::Unreachable("connection refused".into()),
        );
        assert!(out.contains("DB   unreachable"), "got: {out}");
        assert!(out.contains("API  unreachable"), "got: {out}");
    }
}
```
This covers the spec's required `unreachable` path with no live service.

**Verify:** `cargo test` → exit 0, all five new tests pass (3 config + 2 render).

---

### Step 5: Validate

#### 5.1 Run the full gated suite
**Action:** run each command; all must pass offline
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```
If `cargo fmt --check` reports diffs, run `cargo fmt` and re-run the check. If clippy flags an
unused `use anyhow::Result;` in `api/client.rs`, confirm whether the remaining `todo!()` stubs
still use it before removing.

#### 5.2 Record the baseline + any deviations in the spec Notes
**File:** `planning/phase0-blockA/tasks.md`
**Action:** fill in the `## Notes` section
Record the 1.1 baseline result and note the recon-driven default corrections (API port 8080, db
name `postgres`) and that worker/queue rows are deferred per D2.

**Verify:** all four commands in 5.1 → exit 0.

---

## Acceptance Criteria
- `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, and `cargo build --release` all pass.
- `config.rs` reads `DATABASE_URL` + `BASTION_API_URL` from env and surfaces a missing var as a typed error (no panic).
- `.env.example` exists at the repo root documenting both vars.
- `bastion status` runs against unreachable services and prints `unreachable` per service, exiting cleanly (no panic) — covered by a unit test.
- Health-probe and status-render logic are unit-tested with hermetic tests.
- (Manual, non-gating) With the Python orchestrator + DB live, `bastion status` prints real reachable health data.

## Validation Commands
```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

## Notes
- **Signature changes (intentional):** `Config::load()` now returns `Result<Self, ConfigError>`
  (was `anyhow::Result<Self>`); `ApiClient::health()` now returns `ApiStatus` (was
  `Result<bool>`). Both are absorbed by the `?` operator in callers that return `anyhow::Result`.
  `health()` and `db::health::probe()` deliberately return their own status enums (not `Result`)
  so an unreachable service is a normal outcome, never an error that aborts `status`.
- **Hermetic config tests:** parsing is split into a pure `Config::from_vars(...)` so tests never
  read or mutate `std::env` (which `dotenvy` also loads) — keeps `cargo test` deterministic under
  parallel execution.
- **Recon-corrected defaults (status.md, 2026-06-18):** default `BASTION_API_URL` is
  `http://localhost:8080` (orchestrator `/health` port) — the scaffold's old `8000` was wrong;
  `.env.example` uses db name `postgres` (not `orchestrator_db`). The stale `8000`/`orchestrator_db`
  values still in `CLAUDE.md`'s Environment block are out of scope for this task; flag for a later
  docs pass.
- **D2 scope:** worker-count / queue-depth live in Redis (out of bastion's configured scope), so
  `status` reports DB + API reachability only; the master-plan's `worker count / queue depth`
  columns are deferred, not implemented here.
- **No-emoji rule:** the master-plan's `DB ✓ / API ✓` is illustrative; rendered output uses the
  words `reachable` / `unreachable`.
- `cli.rs` (`Commands::Status`) and `main.rs` dispatch already exist from the scaffold — Steps 3.1
  and 3.3 are verification-only, no edits.
