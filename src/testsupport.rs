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

/// RAII guard that neutralizes `dotenvy::dotenv()` for tests that need
/// `DATABASE_URL` (or any other `.env`-supplied var) to be genuinely absent —
/// or genuinely theirs.
///
/// `Config::load` calls `dotenvy::dotenv()`, which **searches upward from the
/// process cwd** and loads the first `.env` it finds, setting any var not
/// already present in the environment. Simply removing this crate's own `.env`
/// is therefore not enough: in a git worktree dotenvy walks past the deleted
/// file and picks up the main checkout's `core/bastion/.env` instead, silently
/// restoring a working `DATABASE_URL` and turning an expected 503 into a 200
/// against the dev Postgres.
///
/// So instead of *removing* `.env`, we **replace it with an empty one**:
/// dotenvy stops at the first file it finds, loads nothing from it, and never
/// reaches any ancestor `.env`.
///
/// **Guarantee actually provided, and its residual hazard**
/// (`planning/ticket-dotenv-shadow-data-loss/`): no code path here ever writes
/// to, truncates, or removes `.env` unless a backup of whatever was there a
/// moment ago is either freshly made or was already sitting, untouched, at
/// this guard's own backup path. Three things make that hold:
///
/// - **A failed backup panics instead of proceeding.** If the rename to
///   `.env.{suffix}.bak` fails (stale file/directory at that path, a full
///   disk, permissions), the guard does **not** write the empty stand-in over
///   an unbacked-up `.env` — it `panic!`s, naming both the original file and
///   the backup path a human should investigate, and leaves `.env`
///   byte-for-byte untouched. A loud, immediate failure here is deliberate:
///   the whole reason this ticket exists is that the prior silent-truncation
///   behavior went unnoticed until a human happened to look at the file by
///   chance.
/// - **A marker (`.env.dotenv-shadow.active`) claimed atomically, before
///   `.env` is touched, makes the guard nesting-aware — and this now covers
///   *every* branch of `new()`, including the adopt-a-stale-backup case
///   below, not just the common path.** The claim is `OpenOptions::create_new`
///   (`O_CREAT | O_EXCL`), so exactly one caller wins even across processes —
///   a real mutex, not an advisory check. A guard that loses the race treats
///   itself as *nested* under the winner and touches nothing at all, neither
///   on construction nor on `Drop`, leaving the winner's backup as the sole
///   source of truth. This is what keeps two overlapping guards (interleaved
///   `A.new, B.new, A.drop, B.drop`) from destroying each other's restores —
///   including the case where `B` happens to find a stale `.bak` sitting at
///   its own suffix while `A` is live under a different suffix: `B` still
///   loses the marker race and still becomes `Nested`, so it never clobbers
///   `A`'s claim or touches `.env` while `A` owns it. Claiming *before either
///   the rename or the adopt* is load-bearing: any ordering that lets `.env`
///   or the marker move first leaves a window where a second guard sees an
///   inconsistent world (e.g. no marker yet but `.env` already touched) and
///   takes a branch that ends up undoing the first guard's state on `Drop`.
/// - **A pre-existing backup at *this exact* suffix is adopted, never
///   overwritten — and adopting it never discards whatever is *currently* in
///   `.env` without saving a copy first.** If `.env.{suffix}.bak` already
///   exists when `new()` runs (a prior guard using the same suffix was killed
///   — Ctrl-C, a timeout, a crash — before its `Drop` ran), this guard
///   reaches that check only *after* it has already won the marker claim
///   above, so no live owner under another suffix can have its state
///   disturbed by an adopt. Having won the claim, the guard does not blindly
///   rename over the stale `.bak`: it adopts the existing backup as its own
///   and will restore *that* file on `Drop`, recovering the original content
///   the interrupted run captured. But whatever currently occupies `.env` at
///   this moment is not the same thing as that stale backup's content — it
///   may be live, more-recent content that arrived after the killed run wrote
///   its `.bak` — so before writing the empty stand-in over it, this guard
///   backs *that* up too, to a distinct path from the adopted `.bak` (so
///   neither copy overwrites the other). That second copy is not
///   auto-restored on `Drop` (the adopted `.bak` is the one `Drop` restores,
///   matching the recovery this branch exists for), but it is never left
///   irrecoverable: a human can always find it on disk next to `.env`.
///
/// **Residual hazard.** `ENV_LOCK` is only meaningful within one process; the
/// *marker* is what carries the guarantee across processes, and it does so
/// only for guards that go through this constructor. Two remaining cases are
/// therefore not covered, both non-destructive:
///
/// - A run killed between claiming the marker and its `Drop` leaves a stale
///   marker behind. Every later guard then reads it, returns `Nested`, and
///   stops shadowing — so `Config::load` starts seeing the real `.env` again
///   and the `C005` scenarios fail loudly. Recovery is to delete the stale
///   `.env.dotenv-shadow.active`; the real content is safe in whichever
///   `.env.{suffix}.bak` the killed run wrote, and that suffix's next run
///   adopts it.
/// - Anything that writes `.env` *without* going through this guard is
///   unsynchronized, as it always was.
///
/// The durable fix for both is to stop touching the repo-root `.env` at all —
/// point the process cwd at a disposable directory so `dotenvy`'s upward walk
/// starts somewhere throwaway (what [`CwdGuard`] does inside this module's own
/// tests). That was considered and deferred: cwd is process-global, so under
/// `cargo test`'s threaded model it is shared with sibling tests that resolve
/// relative paths, and auditing that blast radius is a larger change than this
/// guard's own hardening.
///
/// [`CwdGuard`]: self::tests::CwdGuard
///
/// Mutates a process-wide (indeed repo-wide) resource, so — exactly like an env
/// var — it may only be constructed while holding the crate-wide env lock;
/// [`DotenvShadow::new`] takes the [`EnvLock`] as a witness.
///
/// Lives here rather than in `serve::mod`'s test module (where it was
/// introduced) because `serve::contract_corpus`'s costs scenarios need the same
/// guarantee, and a *second* copy of a global-resource guard is precisely the
/// per-module-duplication mistake the crate-wide [`ENV_LOCK`] exists to undo.
pub struct DotenvShadow {
    state: DotenvShadowState,
}

/// What [`DotenvShadow::new`] actually did, and therefore what [`Drop`] must
/// undo (see the type-level doc for the full rationale of each variant).
enum DotenvShadowState {
    /// We renamed a real `.env` aside to `backup` and wrote the empty
    /// stand-in; restore `backup` on `Drop`.
    Owned(PathBuf),
    /// There was no `.env` to back up; remove the stand-in on `Drop`.
    NoOriginal,
    /// `backup` already existed at our suffix from an interrupted prior run;
    /// we adopted it without overwriting it and restore it on `Drop`.
    Adopted(PathBuf),
    /// Another guard's marker was already present — a guard is already
    /// live under a different suffix. We touched nothing; do nothing on
    /// `Drop` either, so we never race the guard that actually owns the
    /// backup.
    Nested,
}

/// Fixed (not suffix-scoped) marker recording that *some* guard currently has
/// `.env` shadowed. Its presence — regardless of which suffix owns the real
/// backup — is what lets a second, differently-suffixed guard recognize it
/// must not touch `.env` itself (see [`DotenvShadowState::Nested`]).
const DOTENV_SHADOW_MARKER: &str = ".env.dotenv-shadow.active";

impl DotenvShadow {
    /// `suffix` disambiguates the backup filename so two guards can never
    /// collide on it, even if the lock discipline is ever broken.
    ///
    /// `_lock` is the witness that the caller holds the crate-wide env lock —
    /// the `.env` swap is a global mutation and must be serialized against
    /// every reader of it (`dotenvy::dotenv()` inside `Config::load`) just as
    /// an env var would be.
    #[must_use]
    pub fn new(_lock: &EnvLock, suffix: &str) -> Self {
        let env_path = std::path::Path::new(".env");
        let marker_path = std::path::Path::new(DOTENV_SHADOW_MARKER);
        let backup_path = PathBuf::from(format!(".env.{suffix}.bak"));

        // Claim the marker **atomically, before touching `.env` at all —
        // including before checking whether a stale backup already sits at
        // our own suffix.** `create_new(true)` is `O_CREAT | O_EXCL`: exactly
        // one caller can win, even across processes, so this is a real mutex
        // rather than an advisory check.
        //
        // The ordering is load-bearing, for two distinct reasons. Claiming
        // *after* the rename (as the first cut of this guard did) leaves a
        // window in which `.env` has already been moved aside but no marker
        // exists yet — a second process arriving there sees no marker *and*
        // no `.env`, takes the `NoOriginal` branch, and on `Drop` removes the
        // `.env` the first guard had just restored. And claiming *after* the
        // adopt check (the bug this ordering fixes) let a guard that finds a
        // stale `.bak` at its own suffix truncate `.env` and clobber the
        // marker unconditionally, even while a *different* suffix's guard
        // currently owns it — destroying that live owner's state. Winning
        // this claim first means every branch below already knows no other
        // guard is live.
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(marker_path)
        {
            Ok(mut marker) => {
                use std::io::Write as _;
                let _ = marker.write_all(suffix.as_bytes());
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                // A marker is already on disk. Distinguish *whose* it is
                // before deciding what to do: the marker's own content is
                // the suffix that claimed it, so a marker holding **our**
                // suffix is not a live competing guard — it is the stale
                // remnant of an earlier run using this exact suffix that was
                // killed before its `Drop` released the claim (the recovery
                // case this whole type exists for). Reclaiming it is safe
                // because it is, in every sense that matters, the *same*
                // guard resuming: nothing else can be using this suffix
                // concurrently without itself already having lost this same
                // race. A marker holding any *other* suffix is a genuinely
                // live guard; its backup is the only one that matters, so we
                // touch nothing and undo nothing — including when we happen
                // to have a stale `.bak` sitting at our own suffix. That
                // stale copy is not going anywhere; it stays parked for our
                // next attempt.
                let existing_suffix = std::fs::read_to_string(marker_path).unwrap_or_default();
                if existing_suffix != suffix {
                    return Self {
                        state: DotenvShadowState::Nested,
                    };
                }
            }
            Err(err) => panic!(
                "DotenvShadow: failed to claim the shadow marker `{}` ({err}) \
                 — refusing to proceed, since shadowing `{}` without holding \
                 the claim is what allows two guards to destroy each other's \
                 restore. `{}` has NOT been modified.",
                marker_path.display(),
                env_path.display(),
                env_path.display(),
            ),
        }

        // A backup already sitting at *our own* suffix means a prior guard
        // using this exact suffix was interrupted before its `Drop` ran.
        // Adopt it rather than overwriting it — it is the only copy of the
        // real content that guard captured. We only reach this check after
        // winning the marker claim above, so no live owner under another
        // suffix can be disturbed by taking it.
        //
        // Whatever currently occupies `.env` is not necessarily the same
        // thing as that stale backup's content — it may be live, more-recent
        // content that arrived after the killed run wrote its `.bak` — so it
        // is backed up to a distinct, non-restored path before the empty
        // stand-in is written over it. That keeps the adopt path from ever
        // truncating unbacked-up content, without discarding the recovery
        // this branch exists for (the adopted `.bak` remains what `Drop`
        // restores).
        if backup_path.is_file() {
            if env_path.exists() {
                let preadopt_backup = PathBuf::from(format!(".env.{suffix}.pre-adopt.bak"));
                if let Err(err) = std::fs::rename(env_path, &preadopt_backup) {
                    // Release the claim first — see the matching rationale on
                    // the owned-path panic below.
                    let _ = std::fs::remove_file(marker_path);
                    panic!(
                        "DotenvShadow: found a stale backup at `{}` (from an \
                         interrupted prior run) but failed to back up the \
                         *current* `{}` to `{}` before adopting it ({err}) — \
                         refusing to proceed, since doing so would discard \
                         whatever is currently in `{}` with no copy saved. \
                         `{}` has NOT been modified.",
                        backup_path.display(),
                        env_path.display(),
                        preadopt_backup.display(),
                        env_path.display(),
                        env_path.display(),
                    );
                }
            }
            let _ = std::fs::write(env_path, "");
            return Self {
                state: DotenvShadowState::Adopted(backup_path),
            };
        }

        if env_path.exists() {
            match std::fs::rename(env_path, &backup_path) {
                Ok(()) => {
                    // The empty stand-in is what actually stops dotenvy's
                    // upward walk.
                    let _ = std::fs::write(env_path, "");
                    Self {
                        state: DotenvShadowState::Owned(backup_path),
                    }
                }
                Err(err) => {
                    // The backup failed — `.env` is still intact at this
                    // point (the rename never moved it), and it must stay
                    // that way: panic rather than proceed to the
                    // unconditional `write(env_path, "")` that used to run
                    // regardless of whether the backup actually succeeded.
                    //
                    // Release the claim first. Leaking it would make every
                    // subsequent guard in this working tree see the marker,
                    // return `Nested`, and silently stop shadowing at all —
                    // turning a loud failure into a quiet one.
                    let _ = std::fs::remove_file(marker_path);
                    panic!(
                        "DotenvShadow: failed to back up `{}` to `{}` before \
                         shadowing it ({err}) — refusing to proceed, since \
                         doing so would truncate the original with no copy \
                         saved. `{}` has NOT been modified. Investigate why \
                         the rename failed (a stale file or directory at the \
                         backup path, a permissions issue, or a full disk), \
                         clear whatever occupies `{}`, and retry. If `.env` \
                         is ever found empty despite this guard, restore it \
                         from your out-of-tree backup.",
                        env_path.display(),
                        backup_path.display(),
                        env_path.display(),
                        backup_path.display(),
                    );
                }
            }
        } else {
            let _ = std::fs::write(env_path, "");
            Self {
                state: DotenvShadowState::NoOriginal,
            }
        }
    }
}

impl Drop for DotenvShadow {
    fn drop(&mut self) {
        let env_path = std::path::Path::new(".env");
        let marker_path = std::path::Path::new(DOTENV_SHADOW_MARKER);

        match &self.state {
            DotenvShadowState::Owned(backup) | DotenvShadowState::Adopted(backup) => {
                let _ = std::fs::remove_file(env_path);
                let _ = std::fs::rename(backup, env_path);
                let _ = std::fs::remove_file(marker_path);
            }
            DotenvShadowState::NoOriginal => {
                let _ = std::fs::remove_file(env_path);
                let _ = std::fs::remove_file(marker_path);
            }
            DotenvShadowState::Nested => {
                // We never wrote anything; there is nothing to undo.
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
    fn dotenv_shadow_leaves_an_empty_env_and_restores_the_original() {
        let env_lock = lock_env();

        let env_path = std::path::Path::new(".env");
        let before = std::fs::read_to_string(env_path).ok();

        {
            let _shadow = DotenvShadow::new(&env_lock, "testsupport_unit_test");
            assert!(
                env_path.exists(),
                "DotenvShadow must leave an empty `.env` in place — an absent file \
                 lets dotenvy walk up to an ancestor checkout's `.env`"
            );
            assert_eq!(
                std::fs::read_to_string(env_path).unwrap(),
                "",
                "the stand-in `.env` must be empty so it sets no variables"
            );
        }

        let after = std::fs::read_to_string(env_path).ok();
        assert_eq!(
            after, before,
            "dropping the guard must restore the original `.env` byte-for-byte \
             (and leave none behind when there was none)"
        );
    }

    // ── `DotenvShadow` data-loss regression tests ──────────────────────────
    //
    // `planning/ticket-dotenv-shadow-data-loss/tasks.md` documents three
    // independent failure modes in `DotenvShadow` that have destroyed the
    // real, gitignored, unrecoverable `core/bastion/.env` at least twice.
    // Every test below operates on a **temp-dir fixture `.env`, never the
    // real repo-root file** — a test for this defect that itself risks the
    // developer's `.env` would be self-defeating.
    //
    // `DotenvShadow::new`/`Drop` hardcode cwd-relative paths (`.env`,
    // `.env.{suffix}.bak`), so the only way to exercise the *actual* guard
    // against a disposable fixture is to point the process cwd at the
    // fixture directory for the guard's lifetime. [`CwdGuard`] does that,
    // and — like [`EnvVarGuard`] — only while the caller holds [`EnvLock`],
    // so it is serialized against every other test that participates in the
    // crate's env-lock discipline. `cargo nextest run` (this module's own
    // validation command) additionally runs each test in its own process,
    // so cwd is not even shared with sibling tests there.

    /// RAII guard that points the process cwd at `dir` and restores the
    /// original cwd on drop — including on panic/unwind, so a failing
    /// assertion inside one of these tests can never leave a later test
    /// running from the wrong directory.
    struct CwdGuard {
        original: PathBuf,
    }

    impl CwdGuard {
        fn enter(_lock: &EnvLock, dir: &std::path::Path) -> Self {
            let original =
                std::env::current_dir().expect("current dir must be readable to save it");
            std::env::set_current_dir(dir)
                .expect("must be able to chdir into the disposable fixture dir");
            Self { original }
        }
    }

    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.original);
        }
    }

    /// Create a fresh, empty, disposable fixture directory under the system
    /// temp dir.
    fn fixture_dir(prefix: &str) -> PathBuf {
        let dir = unique_temp_dir(prefix);
        std::fs::create_dir_all(&dir).expect("create fixture dir");
        dir
    }

    const FIXTURE_ENV_CONTENT: &str =
        "DATABASE_URL=postgres://fixture-user:fixture-pass@localhost/fixture\n";

    #[test]
    fn dotenv_shadow_round_trip_preserves_content_on_a_fixture() {
        let lock = lock_env();
        let dir = fixture_dir("dotenv-shadow-roundtrip");
        let _cwd = CwdGuard::enter(&lock, &dir);

        std::fs::write(".env", FIXTURE_ENV_CONTENT).expect("seed fixture .env");

        {
            let shadow = DotenvShadow::new(&lock, "roundtrip_fixture");
            assert_eq!(
                std::fs::read_to_string(".env").unwrap(),
                "",
                "the stand-in must be empty while the guard is alive"
            );
            drop(shadow);
        }

        assert_eq!(
            std::fs::read_to_string(".env").unwrap(),
            FIXTURE_ENV_CONTENT,
            "the fixture .env must be byte-identical after new() -> drop() \
             — this is the baseline the current implementation already \
             passes and must not regress"
        );
    }

    #[test]
    fn dotenv_shadow_failed_backup_must_not_truncate() {
        // Failure mode 1: `DotenvShadow::new` used to discard the rename
        // error with `let _ =` and truncate `.env` regardless of whether the
        // backup actually succeeded. Force the rename to fail by
        // pre-creating a *directory* at the backup path, then assert (a) the
        // guard panics instead of proceeding and (b) the original content is
        // still intact afterward. Under the pre-fix implementation this
        // fails — `new()` returns normally and the file is truncated to ""
        // even though no backup exists.
        let lock = lock_env();
        let dir = fixture_dir("dotenv-shadow-failed-backup");
        let _cwd = CwdGuard::enter(&lock, &dir);

        std::fs::write(".env", FIXTURE_ENV_CONTENT).expect("seed fixture .env");
        std::fs::create_dir_all(".env.failed_backup_fixture.bak")
            .expect("pre-create a directory at the backup path so rename fails");

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            DotenvShadow::new(&lock, "failed_backup_fixture")
        }));

        let panic_payload = match result {
            Ok(_) => panic!(
                "a failed backup must panic naming the file and the recovery \
                 path instead of proceeding to truncate it"
            ),
            Err(payload) => payload,
        };
        let message = panic_payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic_payload.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default();
        assert!(
            message.contains(".env") && message.contains("failed_backup_fixture"),
            "panic message must name both the original file and the backup \
             (recovery) path; got: {message}"
        );

        assert_eq!(
            std::fs::read_to_string(".env").unwrap(),
            FIXTURE_ENV_CONTENT,
            "a failed backup must never truncate the original .env"
        );
    }

    #[test]
    fn dotenv_shadow_interrupted_run_does_not_poison_the_next_guard() {
        // Failure mode 2: the backup filename is a hardcoded literal, so a
        // process that dies between `new()` and `drop()` (Ctrl-C, a kill,
        // a timeout) leaves an empty `.env` and a real `.env.{suffix}.bak`.
        // `std::mem::forget` simulates that: the guard's value is leaked, so
        // its `Drop` never runs, exactly like a killed process. A second run
        // with the same (hardcoded) suffix must not let the empty stand-in
        // clobber the real backup left behind by the first.
        let lock = lock_env();
        let dir = fixture_dir("dotenv-shadow-interrupted");
        let _cwd = CwdGuard::enter(&lock, &dir);

        std::fs::write(".env", FIXTURE_ENV_CONTENT).expect("seed fixture .env");

        let first_run = DotenvShadow::new(&lock, "interrupted_fixture");
        std::mem::forget(first_run);
        assert_eq!(
            std::fs::read_to_string(".env").unwrap(),
            "",
            "sanity check: the simulated-killed first run must have left the \
             empty stand-in in place, with the real content parked in its backup"
        );

        let second_run = DotenvShadow::new(&lock, "interrupted_fixture");
        drop(second_run);

        assert_eq!(
            std::fs::read_to_string(".env").unwrap(),
            FIXTURE_ENV_CONTENT,
            "an interrupted first run must not let a second run over the same \
             fixture destroy the real content the first run backed up"
        );
    }

    #[test]
    fn dotenv_shadow_concurrent_guards_do_not_destroy_content() {
        // Failure mode 3: `ENV_LOCK` is process-local, so it gives no
        // protection across the separate OS processes `cargo nextest` uses
        // per test. Two guards with interleaved lifetimes — `A.new, B.new,
        // A.drop, B.drop` — reproduce, within a single process, the exact
        // sequence that destroys `.env` across two real processes: A's drop
        // restores the real content, then B's drop removes it again and
        // replaces it with its own (empty) backup.
        let lock = lock_env();
        let dir = fixture_dir("dotenv-shadow-concurrent");
        let _cwd = CwdGuard::enter(&lock, &dir);

        std::fs::write(".env", FIXTURE_ENV_CONTENT).expect("seed fixture .env");

        let guard_a = DotenvShadow::new(&lock, "concurrent_fixture_a");
        let guard_b = DotenvShadow::new(&lock, "concurrent_fixture_b");
        drop(guard_a);
        drop(guard_b);

        assert_eq!(
            std::fs::read_to_string(".env").unwrap(),
            FIXTURE_ENV_CONTENT,
            "two overlapping guards over the same fixture must not let the \
             later drop overwrite content an earlier drop already restored"
        );
    }

    #[test]
    fn dotenv_shadow_claims_its_marker_before_moving_env() {
        // Failure mode 3, the narrow residual left by the first fix: the
        // marker used to be written *after* the rename, so a second guard
        // arriving in that window saw neither a marker nor a `.env`, took the
        // `NoOriginal` branch, and on `Drop` deleted the `.env` the first
        // guard had just restored. Reachable with *distinct* suffixes — the
        // very case the marker exists to cover.
        //
        // **What this test does and does not prove.** It is an invariant
        // guard, *not* a regression test for the ordering change — and the
        // difference matters, so it is stated rather than left implied. The
        // fix is unobservable from a single process: both the old
        // (write-marker-after-rename) and new (claim-before-rename) forms
        // leave exactly the same on-disk state by the time `new()` returns,
        // so every assertion below passes against both. Catching the actual
        // interleaving would need two real processes suspended mid-`new()`,
        // or a fault-injection seam threaded through production code to widen
        // a microsecond window — more machinery than the window warrants.
        //
        // What it does earn: it pins the marker's *lifecycle* (claimed by
        // construction, released on drop, and never disturbed by a nested
        // guard), so a future refactor that leaks the marker — which would
        // silently turn every later guard into a no-op and stop `.env` being
        // shadowed at all — fails here loudly instead of in whichever
        // `C005` scenario happens to notice.
        let lock = lock_env();
        let dir = fixture_dir("dotenv-shadow-claim-order");
        let _cwd = CwdGuard::enter(&lock, &dir);

        std::fs::write(".env", FIXTURE_ENV_CONTENT).expect("seed fixture .env");

        let owner = DotenvShadow::new(&lock, "claim_order_fixture");
        assert!(
            std::path::Path::new(DOTENV_SHADOW_MARKER).exists(),
            "the marker must be claimed by the time `new` returns — if it is \
             written after the rename instead, a concurrent guard can slip \
             into the gap and delete the restored `.env`"
        );

        // A second guard arriving at any point after that must lose the race
        // and become a no-op, even though `.env` is currently the empty
        // stand-in rather than the real file.
        let latecomer = DotenvShadow::new(&lock, "claim_order_other_suffix");
        drop(latecomer);
        assert_eq!(
            std::fs::read_to_string(".env").unwrap(),
            "",
            "a nested guard's drop must not disturb the stand-in the owner \
             still relies on"
        );

        drop(owner);
        assert_eq!(
            std::fs::read_to_string(".env").unwrap(),
            FIXTURE_ENV_CONTENT,
            "the owner must still restore the real content after a latecomer \
             has come and gone"
        );
        assert!(
            !std::path::Path::new(DOTENV_SHADOW_MARKER).exists(),
            "the owner must release the marker on drop — a leaked marker makes \
             every later guard a silent no-op"
        );
    }

    #[test]
    fn adopted_branch_does_not_truncate_an_unbacked_env() {
        // The adopted-branch defect (`ticket-dotenv-shadow-adopted-branch`):
        // the pre-fix implementation checked for a stale backup at its own
        // suffix *before* claiming the marker, and truncated `.env` on the
        // spot with no fresh backup — on the assumption that the stale
        // `.bak` was still the authoritative copy of current content. Seed a
        // fixture where that assumption is false: content A is currently in
        // `.env`, but the stale `.bak` holds *different* content B (as if
        // captured by an interrupted run before A ever arrived). Constructing
        // and dropping a guard must never leave A irrecoverable — under the
        // pre-fix code A is destroyed outright and B is restored over it.
        let lock = lock_env();
        let dir = fixture_dir("dotenv-shadow-adopt-unbacked");
        let _cwd = CwdGuard::enter(&lock, &dir);

        const CONTENT_A: &str = "DATABASE_URL=postgres://current-a@localhost/a\n";
        const CONTENT_B: &str = "DATABASE_URL=postgres://stale-b@localhost/b\n";

        std::fs::write(".env", CONTENT_A).expect("seed fixture .env with content A");
        std::fs::write(".env.adopt_unbacked.bak", CONTENT_B)
            .expect("seed a stale backup at this guard's own suffix with content B");

        let guard = DotenvShadow::new(&lock, "adopt_unbacked");

        // Content A must have been preserved somewhere on disk the moment the
        // guard decided to overwrite `.env` — not silently discarded.
        let preadopt_backup = std::path::Path::new(".env.adopt_unbacked.pre-adopt.bak");
        assert!(
            preadopt_backup.is_file(),
            "adopting a stale backup must first save whatever is currently in \
             `.env` to a distinct, non-clobbered path — found no such file"
        );
        assert_eq!(
            std::fs::read_to_string(preadopt_backup).unwrap(),
            CONTENT_A,
            "the pre-adopt backup must hold content A byte-for-byte — it is \
             the only remaining copy of what was live in `.env`"
        );

        drop(guard);

        // `Drop` restores the adopted backup (content B, the recovery this
        // branch exists for) — but A must still be recoverable from the
        // pre-adopt backup left beside it, not gone.
        assert_eq!(
            std::fs::read_to_string(".env").unwrap(),
            CONTENT_B,
            "dropping an adopted guard must restore the adopted backup"
        );
        assert_eq!(
            std::fs::read_to_string(preadopt_backup).unwrap(),
            CONTENT_A,
            "content A must still be sitting, untouched, at the pre-adopt \
             backup path after the guard has dropped — it is never left \
             irrecoverable"
        );
    }

    #[test]
    fn adopted_branch_does_not_clobber_a_live_owners_marker() {
        // The second half of the adopted-branch defect: a latecomer whose own
        // suffix happens to have a stale `.bak` sitting next to it must not
        // be able to steal or overwrite a live owner's marker claim. Under
        // the pre-fix code the adopt check ran *before* the marker claim, so
        // the latecomer unconditionally `fs::write`'d the marker (clobbering
        // the owner's claim) and truncated `.env` — actively destroying the
        // owner's stand-in while the owner still believed it held the lock.
        let lock = lock_env();
        let dir = fixture_dir("dotenv-shadow-adopt-clobber");
        let _cwd = CwdGuard::enter(&lock, &dir);

        std::fs::write(".env", FIXTURE_ENV_CONTENT).expect("seed fixture .env");

        // Owner claims the marker under suffix X first.
        let owner = DotenvShadow::new(&lock, "adopt_clobber_owner_x");
        assert!(
            std::path::Path::new(DOTENV_SHADOW_MARKER).exists(),
            "sanity check: the owner must hold the marker"
        );
        let marker_contents_before =
            std::fs::read_to_string(DOTENV_SHADOW_MARKER).expect("read the claimed marker");
        assert_eq!(marker_contents_before, "adopt_clobber_owner_x");

        // A stale backup sitting at a *different* suffix Y, as if left behind
        // by some earlier interrupted run that used that suffix.
        std::fs::write(
            ".env.adopt_clobber_latecomer_y.bak",
            "stale content from suffix Y",
        )
        .expect("seed a stale backup at the latecomer's suffix");

        // The latecomer arrives while the owner is still live.
        let latecomer = DotenvShadow::new(&lock, "adopt_clobber_latecomer_y");

        // The owner's marker claim must survive untouched — not overwritten
        // with the latecomer's suffix.
        assert_eq!(
            std::fs::read_to_string(DOTENV_SHADOW_MARKER).unwrap(),
            "adopt_clobber_owner_x",
            "a latecomer with a stale backup at its own suffix must not \
             overwrite a live owner's marker claim"
        );

        // The owner's stand-in `.env` must be untouched by the latecomer's
        // construction.
        assert_eq!(
            std::fs::read_to_string(".env").unwrap(),
            "",
            "the latecomer must not touch `.env` while the owner is live"
        );

        drop(latecomer);
        assert_eq!(
            std::fs::read_to_string(".env").unwrap(),
            "",
            "a latecomer's drop must not disturb the owner's stand-in either"
        );
        assert_eq!(
            std::fs::read_to_string(DOTENV_SHADOW_MARKER).unwrap(),
            "adopt_clobber_owner_x",
            "the owner's marker must still be intact after the latecomer has \
             come and gone"
        );

        drop(owner);
        assert_eq!(
            std::fs::read_to_string(".env").unwrap(),
            FIXTURE_ENV_CONTENT,
            "the owner must still restore the real content after a latecomer \
             that lost the race has come and gone"
        );
        assert!(
            !std::path::Path::new(DOTENV_SHADOW_MARKER).exists(),
            "the owner must release the marker on drop"
        );
        // The latecomer's stale backup at suffix Y must have been left
        // completely alone — it was never adopted because the latecomer lost
        // the marker race before ever reaching the adopt check.
        assert_eq!(
            std::fs::read_to_string(".env.adopt_clobber_latecomer_y.bak").unwrap(),
            "stale content from suffix Y",
            "a latecomer that loses the marker race must never touch its own \
             stale backup either"
        );
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
