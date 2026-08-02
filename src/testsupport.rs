//! Process-global test hygiene — the shared disciplines that keep the
//! authoritative `cargo test` gate deterministic.
//!
//! `cargo test` runs every test of a binary as a **thread** inside one
//! process. `cargo nextest` runs each test in its **own process**. That single
//! difference means every bug in this module's remit is *structurally
//! invisible to `cargo nextest`* — the harness `fastCommand`
//! (`cargo nextest run --lib --bins`) can never reproduce them, so a green
//! nextest run is not evidence that the authoritative gate is green. Two
//! classes of process-global state bit this suite (both diagnosed in
//! `planning/ticket-serve-env-test-race/`):
//!
//! 1. **Env vars are process-global.** `std::env::set_var` / `remove_var` from
//!    one test thread are observed by every other thread that reads env
//!    (directly, or via `Config::load` / `dotenvy::dotenv()` / the
//!    contract-corpus `BASTION_*` knobs). [`lock_env`] + [`EnvVarGuard`] make
//!    every mutation exclusive against every reader that opts in.
//!
//! 2. **Temp-fixture directory names must be unique.** The old idiom —
//!    `temp_dir().join(format!("prefix-{pid}-{nanos}"))` — is *not* unique
//!    under thread parallelism: `std::process::id()` is identical across
//!    threads and macOS's `SystemTime::now()` has only microsecond
//!    resolution, so two tests entering the helper in the same microsecond get
//!    **the same path**. The first to finish `remove_dir_all`s the fixture out
//!    from under the second, whose request then 500s (or whose `symlink(..)
//!    .unwrap()` panics with `AlreadyExists`). [`unique_temp_dir`] closes this
//!    with a process-wide atomic counter. Under nextest the pid differs per
//!    test, which is exactly why nextest never saw it.
//!
//! # Lock discipline (read before adding a lock anywhere in the test suite)
//!
//! There is **one** env lock for the whole crate: [`ENV_LOCK`], reached only
//! through [`lock_env`]. Deliberately one, not several — a single lock makes
//! ordering moot and deadlock impossible between modules.
//!
//! - **Acquire it exactly once per test, at the top of the test body**, and
//!   hold the returned [`EnvLock`] for the whole test. `std::sync::Mutex` is
//!   **not reentrant**: a second `lock_env()` on the same thread deadlocks.
//! - **Never call a helper that acquires the lock while already holding it.**
//!   In practice that means production-shaped helpers snapshot env under the
//!   lock and *release before* doing any further work (see
//!   `serve::contract_corpus::CorpusConfig::from_env`), while low-level env
//!   readers (`corpus_dir()`, `should_write()`) stay lock-free so a
//!   lock-holding test can call them directly.
//! - **Never hold it across an `.await`** — snapshot what you need, drop the
//!   guard, then await (also keeps clippy's `await_holding_lock` quiet).
//! - Mutating env without the lock is not possible through [`EnvVarGuard`]:
//!   its constructors take `&EnvLock` as a witness, so the type system records
//!   that the caller holds it. `grep -rn 'set_var\|remove_var' src/` should
//!   only ever hit this file.
//! - The lock recovers from poisoning (`unwrap_or_else(|e| e.into_inner())`)
//!   so one panicking test cannot cascade into a suite-wide failure.
//!
//! # Reusing this from another module
//!
//! ```ignore
//! let _env = crate::testsupport::lock_env();
//! let _db = crate::testsupport::EnvVarGuard::set(&_env, "DATABASE_URL", "postgres://…");
//! // … env is exclusively ours until both guards drop …
//! ```

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

// ── 1. The process-wide env lock ────────────────────────────────────────────

/// The single process-wide lock serializing every mutation of, and opted-in
/// read of, process-global environment state (env vars **and** the repo-cwd
/// `.env` file that `dotenvy` reads). Reached only via [`lock_env`].
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// RAII token proving the holder owns [`ENV_LOCK`].
///
/// Passed by reference to [`EnvVarGuard`]'s constructors so that mutating
/// process env without the lock is a compile error rather than a convention.
/// Keep it alive for the whole test — dropping it early releases the lock
/// while your env mutations are still in place.
pub struct EnvLock(#[allow(dead_code)] MutexGuard<'static, ()>);

/// Acquire the process-wide env lock, recovering from poisoning.
///
/// Poison recovery is deliberate: a test that panics while holding the lock
/// has already failed and reported; propagating the poison would convert one
/// real failure into an avalanche of unrelated ones.
///
/// **Not reentrant** — call at most once per test thread. See the module
/// docs' lock discipline.
#[must_use]
pub fn lock_env() -> EnvLock {
    EnvLock(ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner()))
}

/// RAII guard that sets or unsets one env var for the duration of a test and
/// always restores the previous value on drop — including on panic/unwind, so
/// a `#[should_panic]` test never leaks its override into a sibling.
///
/// Construction requires a `&`[`EnvLock`], so the guard cannot exist without
/// the process-wide lock being held.
pub struct EnvVarGuard {
    key: String,
    previous: Option<String>,
}

impl EnvVarGuard {
    /// Set `key` to `value` until the guard drops.
    pub fn set(_lock: &EnvLock, key: &str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        // SAFETY: `_lock` witnesses that this thread holds `ENV_LOCK`, so no
        // other test thread that participates in the discipline reads or
        // writes process env while this guard lives.
        unsafe { std::env::set_var(key, value) };
        Self {
            key: key.to_owned(),
            previous,
        }
    }

    /// Remove `key` from the environment until the guard drops.
    pub fn unset(_lock: &EnvLock, key: &str) -> Self {
        let previous = std::env::var(key).ok();
        // SAFETY: see [`EnvVarGuard::set`].
        unsafe { std::env::remove_var(key) };
        Self {
            key: key.to_owned(),
            previous,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        // SAFETY: the `EnvLock` that authorized construction outlives this
        // guard in every call site (it is bound first, so it drops last).
        unsafe {
            match &self.previous {
                Some(v) => std::env::set_var(&self.key, v),
                None => std::env::remove_var(&self.key),
            }
        }
    }
}

// ── 2. Collision-proof temp fixture directories ─────────────────────────────

/// A path under `std::env::temp_dir()` that is unique **across threads of one
/// process and across processes**: `<prefix>-<pid>-<nanos>-<counter>`.
///
/// The trailing `counter` is the load-bearing part. `pid` alone is constant
/// across `cargo test`'s test threads and `nanos` alone is not fine-grained
/// enough (macOS `SystemTime::now()` is microsecond-resolution), so the
/// pre-existing `format!("{prefix}-{pid}-{nanos}")` idiom handed the *same*
/// directory to two concurrent tests often enough to fail roughly one full
/// `cargo test` run in ten. `pid` and `nanos` are retained only so a stray
/// leftover directory is still attributable to a run.
///
/// Creates nothing — the caller does its own `create_dir_all`, as before.
#[must_use]
pub fn unique_temp_dir(prefix: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{}-{nanos}-{seq}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_temp_dir_never_repeats_within_a_process() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1_000 {
            assert!(
                seen.insert(unique_temp_dir("bastion-uniqueness-probe")),
                "unique_temp_dir must never hand out the same path twice — that \
                 is the exact bug it exists to prevent"
            );
        }
    }

    #[test]
    fn unique_temp_dir_is_under_the_system_temp_dir_and_carries_the_prefix() {
        let path = unique_temp_dir("bastion-prefix-probe");
        assert!(
            path.starts_with(std::env::temp_dir()),
            "fixtures must live under the system temp dir; got {}",
            path.display()
        );
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        assert!(
            name.starts_with("bastion-prefix-probe-"),
            "the caller's prefix must lead the directory name; got {name}"
        );
        assert_eq!(
            name.split('-').count(),
            6,
            "name shape must stay <prefix(3 parts)>-<pid>-<nanos>-<seq>; got {name}"
        );
    }

    #[test]
    fn unique_temp_dir_is_collision_free_across_threads() {
        // The regression this whole helper exists for: the old
        // `{pid}-{nanos}` idiom collided precisely when two threads entered
        // within one microsecond.
        let paths = std::sync::Arc::new(Mutex::new(Vec::new()));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let paths = std::sync::Arc::clone(&paths);
                std::thread::spawn(move || {
                    let batch: Vec<PathBuf> =
                        (0..200).map(|_| unique_temp_dir("bastion-race")).collect();
                    paths.lock().unwrap().extend(batch);
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        let all = paths.lock().unwrap();
        let unique: std::collections::HashSet<_> = all.iter().collect();
        assert_eq!(
            unique.len(),
            all.len(),
            "concurrent callers must never receive the same fixture path"
        );
    }

    #[test]
    fn env_var_guard_sets_then_restores_a_previously_unset_var() {
        let lock = lock_env();
        const KEY: &str = "BASTION_TESTSUPPORT_PROBE_UNSET";
        // SAFETY: under `lock`.
        let baseline = EnvVarGuard::unset(&lock, KEY);
        assert!(std::env::var(KEY).is_err());
        {
            let _g = EnvVarGuard::set(&lock, KEY, "value");
            assert_eq!(std::env::var(KEY).unwrap(), "value");
        }
        assert!(
            std::env::var(KEY).is_err(),
            "dropping the guard must restore the absent state, not leave an empty string"
        );
        drop(baseline);
    }

    #[test]
    fn env_var_guard_restores_the_previous_value_not_absence() {
        let lock = lock_env();
        const KEY: &str = "BASTION_TESTSUPPORT_PROBE_SET";
        let baseline = EnvVarGuard::set(&lock, KEY, "original");
        {
            let _g = EnvVarGuard::set(&lock, KEY, "overridden");
            assert_eq!(std::env::var(KEY).unwrap(), "overridden");
        }
        assert_eq!(
            std::env::var(KEY).unwrap(),
            "original",
            "the guard must restore the prior value, not unset the var"
        );
        drop(baseline);
    }

    #[test]
    fn env_var_guard_unset_restores_the_previous_value() {
        let lock = lock_env();
        const KEY: &str = "BASTION_TESTSUPPORT_PROBE_UNSET_RESTORE";
        let baseline = EnvVarGuard::set(&lock, KEY, "original");
        {
            let _g = EnvVarGuard::unset(&lock, KEY);
            assert!(std::env::var(KEY).is_err());
        }
        assert_eq!(std::env::var(KEY).unwrap(), "original");
        drop(baseline);
    }

    #[test]
    fn lock_env_recovers_from_a_poisoned_mutex() {
        // Poison the lock from a scoped thread, then prove `lock_env()` still
        // hands out a usable guard — one panicking test must not cascade.
        let poisoner = std::thread::spawn(|| {
            let _held = lock_env();
            panic!("deliberate poisoning");
        });
        assert!(poisoner.join().is_err(), "the poisoner must have panicked");

        let lock = lock_env();
        const KEY: &str = "BASTION_TESTSUPPORT_PROBE_POISON";
        let g = EnvVarGuard::set(&lock, KEY, "still-works");
        assert_eq!(std::env::var(KEY).unwrap(), "still-works");
        drop(g);
    }
}
