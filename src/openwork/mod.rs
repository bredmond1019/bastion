//! openwork/ — refresh action for the six HQ open-work boards (BA.26.C).
//!
//! Invokes `planning/open-work/scripts/refresh.py` (HQ root, executed —
//! never written) as a subprocess. Follows standing rule 6's
//! construction-vs-execution split (established in
//! `../engine-rs/crates/term-core/src/tmux.rs`): the argv builder, the
//! exit-code classifier, and the should-we-run decision are pure functions,
//! exhaustively tested; the spawn is a thin shell (task 3+).
//!
//! # AC-1 — `--commit` and `--emit` are UNREACHABLE, not merely undefaulted
//!
//! `refresh.py --commit` commits into the HQ-git-tracked vault; `--emit` runs
//! `mev emit-state --write` fleet-wide (refresh.py:20-21). Either would let a
//! single TUI keypress commit or regenerate the whole corpus. This module
//! enforces that in the TYPE, not with a runtime guard: [`RefreshMode`] is a
//! closed enum whose variants are exactly the legal modes (current-check,
//! force-regenerate), so `--commit` and `--emit` have no representation to
//! construct — there is no branch, flag, or field anywhere in this module
//! that can produce either string.
//!
//! Shown-failing demonstration (recorded per the task spec, not committed):
//! adding a `RefreshMode::Commit` variant that pushed `"--commit".to_string()`
//! into [`refresh_args`]'s output, and running
//! `cargo nextest run --lib --bins openwork`, turned
//! `argv_never_contains_commit_or_emit` red (the two `assert!(!argv.contains(...))`
//! lines failed) while every other test in the module stayed green. The
//! variant was then removed; no red test was committed.

use std::io;
use std::path::Path;
use std::process::{Child, Command};

/// The path to HQ's open-work refresh script, relative to the HQ root
/// (`agentic-portfolio/`). Bastion executes this script; it never writes to
/// it or to any path under `planning/open-work/`.
pub const REFRESH_SCRIPT_RELATIVE_PATH: &str = "planning/open-work/scripts/refresh.py";

/// The Python interpreter used to run [`REFRESH_SCRIPT_RELATIVE_PATH`].
pub const PYTHON_BIN: &str = "python3";

/// The closed set of modes the refresh action can run in.
///
/// This is the entire legal input space for [`refresh_args`] — there is no
/// other constructor, no builder pattern, and no stringly-typed flag field
/// anywhere in this module. `--commit` and `--emit` are not variants of this
/// enum, so no value of this type can ever cause [`refresh_args`] to emit
/// either flag (AC-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefreshMode {
    /// `--check` — report staleness, write nothing. The safe default: this
    /// is also the only mode the hand smoke test (task 5) may use.
    CheckOnly,
    /// `--check --force` — report staleness as if every board were stale,
    /// i.e. force a full regenerate-and-recheck. Still never writes via
    /// `--commit` or emits via `--emit`.
    ForceCheck,
}

/// Builds the argument vector for invoking [`REFRESH_SCRIPT_RELATIVE_PATH`]
/// under `python3`, for the given [`RefreshMode`].
///
/// The first element is [`PYTHON_BIN`], the second is the script path
/// (resolved against `hq_root`), followed by mode-specific flags. Because
/// [`RefreshMode`]'s variant set is closed and contains no commit/emit
/// representation, no value of `mode` can make this function's output
/// contain `"--commit"` or `"--emit"` (AC-1, asserted exhaustively in
/// `tests::argv_never_contains_commit_or_emit`).
///
/// `hq_root` is the absolute path to the `agentic-portfolio/` root, from
/// which `refresh.py` must be invoked (its own relative imports resolve
/// against that cwd — see refresh.py's own `sys.path` bootstrap).
pub fn refresh_args(mode: RefreshMode, hq_root: &Path) -> Vec<String> {
    let script_path = hq_root
        .join(REFRESH_SCRIPT_RELATIVE_PATH)
        .to_string_lossy()
        .into_owned();

    let mut argv = vec![PYTHON_BIN.to_string(), script_path, "--check".to_string()];

    match mode {
        RefreshMode::CheckOnly => {}
        RefreshMode::ForceCheck => argv.push("--force".to_string()),
    }

    argv
}

/// The classified result of one `refresh.py` invocation, as reported to the
/// TUI (task 4).
///
/// Three exit-code-derived variants plus a fourth for the process never
/// having started at all. `SpawnFailed` is kept distinct from `Failed`
/// because the operator's next action differs: a spawn failure means
/// `python3` is missing or unreadable (fix the environment), while a
/// `Failed { code }` means `refresh.py` itself ran and rejected — its
/// stderr and the specific `code` are the useful diagnostic, not the PATH.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshOutcome {
    /// Exit code 0 — every board is current; refresh.py wrote nothing.
    Current,
    /// Exit code 2 — at least one board was stale or changed and was
    /// regenerated.
    StaleOrChanged,
    /// Any other exit code. Carries the REAL code (AC-2) — never collapsed
    /// to a generic "it failed" — because a caller deciding what to do next
    /// needs to know which non-zero it was, not just that it was non-zero.
    Failed { code: i32 },
    /// The child process could not be started at all — e.g. `python3` is
    /// not on `PATH`, or a permissions error on the interpreter or script.
    /// This is a different failure class from `Failed`: nothing of
    /// refresh.py ran, so there is no exit code and no refresh.py stderr to
    /// show, only the OS-level spawn error.
    SpawnFailed { reason: String },
}

/// Classifies the exit code of a `refresh.py` invocation that DID spawn and
/// DID run to completion (AC-2).
///
/// `0` maps to [`RefreshOutcome::Current`], `2` to
/// [`RefreshOutcome::StaleOrChanged`], and every other value to
/// [`RefreshOutcome::Failed`] carrying that exact code. There is no default
/// arm that swallows an unrecognized non-zero code into a generic failure —
/// `other` is threaded straight through.
pub fn classify_exit_code(code: i32) -> RefreshOutcome {
    match code {
        0 => RefreshOutcome::Current,
        2 => RefreshOutcome::StaleOrChanged,
        other => RefreshOutcome::Failed { code: other },
    }
}

/// Classifies a spawn error (the `std::io::Error` `Command::spawn` returns
/// when the child process could never be started) as
/// [`RefreshOutcome::SpawnFailed`], distinct from any exit-code-derived
/// [`RefreshOutcome::Failed`] (AC-2's "failure-to-spawn" requirement).
///
/// Kept as a separate function from [`classify_exit_code`] rather than
/// folded into one signature over `Result<i32, io::Error>`, so the thin I/O
/// shell (task 3+) can call whichever one matches what it actually got back
/// from `Command::spawn`/`Child::wait` without constructing a placeholder
/// exit code for the no-spawn case.
pub fn classify_spawn_error(err: &std::io::Error) -> RefreshOutcome {
    RefreshOutcome::SpawnFailed {
        reason: err.to_string(),
    }
}

/// The thin I/O shell (task 3, AC-3) over [`refresh_args`]: spawns the given
/// argv as a child process OFF the caller's thread of control.
///
/// `Command::spawn` itself is already non-blocking — it returns as soon as
/// the OS has forked/exec'd the child, without waiting for it to run or
/// exit — so this function adds no synchronization of its own; it exists so
/// callers (the ui.rs event loop, and this module's own non-blocking test)
/// have one narrow seam to call instead of constructing a `Command` inline.
///
/// Generic over any argv, not tied to [`refresh_args`]'s output, so a test
/// proving the non-blocking property can spawn a `sleep`-style stand-in
/// binary instead of the real (measured 21.7 s) `refresh.py`.
///
/// Returns the spawn `io::Error` on failure (e.g. the program is not on
/// `PATH`) rather than panicking, so a caller can classify it with
/// [`classify_spawn_error`].
pub fn spawn_argv(argv: &[String]) -> io::Result<Child> {
    let (program, args) = argv
        .split_first()
        .expect("argv must contain at least the program name");
    Command::new(program).args(args).spawn()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_modes() -> [RefreshMode; 2] {
        [RefreshMode::CheckOnly, RefreshMode::ForceCheck]
    }

    #[test]
    fn check_only_args_correct() {
        let argv = refresh_args(RefreshMode::CheckOnly, Path::new("/hq"));
        assert_eq!(
            argv,
            vec![
                "python3".to_string(),
                "/hq/planning/open-work/scripts/refresh.py".to_string(),
                "--check".to_string(),
            ]
        );
    }

    #[test]
    fn force_check_args_correct() {
        let argv = refresh_args(RefreshMode::ForceCheck, Path::new("/hq"));
        assert_eq!(
            argv,
            vec![
                "python3".to_string(),
                "/hq/planning/open-work/scripts/refresh.py".to_string(),
                "--check".to_string(),
                "--force".to_string(),
            ]
        );
    }

    /// AC-1, exhaustive over the closed input space: no `RefreshMode`
    /// variant produces `--commit` or `--emit` anywhere in the argv.
    #[test]
    fn argv_never_contains_commit_or_emit() {
        for mode in all_modes() {
            let argv = refresh_args(mode, Path::new("/hq"));
            assert!(
                !argv.iter().any(|a| a == "--commit"),
                "mode {mode:?} produced --commit: {argv:?}"
            );
            assert!(
                !argv.iter().any(|a| a == "--emit"),
                "mode {mode:?} produced --emit: {argv:?}"
            );
        }
    }

    /// AC-4 (standing rule 6): asserted element-by-element as a Vec<String>,
    /// never by asserting on a joined string.
    #[test]
    fn argv_first_element_is_python_binary() {
        for mode in all_modes() {
            let argv = refresh_args(mode, Path::new("/hq"));
            assert_eq!(argv[0], "python3");
        }
    }

    #[test]
    fn argv_second_element_is_script_path_resolved_against_hq_root() {
        for mode in all_modes() {
            let argv = refresh_args(mode, Path::new("/some/hq/root"));
            assert_eq!(
                argv[1],
                "/some/hq/root/planning/open-work/scripts/refresh.py"
            );
        }
    }

    #[test]
    fn check_only_has_exactly_three_elements() {
        let argv = refresh_args(RefreshMode::CheckOnly, Path::new("/hq"));
        assert_eq!(argv.len(), 3);
    }

    #[test]
    fn force_check_has_exactly_four_elements() {
        let argv = refresh_args(RefreshMode::ForceCheck, Path::new("/hq"));
        assert_eq!(argv.len(), 4);
    }

    // -- task 2: exit-code classifier (AC-2) ---------------------------

    #[test]
    fn exit_code_zero_is_current() {
        assert_eq!(classify_exit_code(0), RefreshOutcome::Current);
    }

    #[test]
    fn exit_code_two_is_stale_or_changed() {
        assert_eq!(classify_exit_code(2), RefreshOutcome::StaleOrChanged);
    }

    /// The failure variant must carry the REAL exit code. Asserted with two
    /// distinct non-zero codes so a classifier that hardcodes a single
    /// failure code cannot pass.
    #[test]
    fn exit_code_one_is_failed_with_code_preserved() {
        assert_eq!(classify_exit_code(1), RefreshOutcome::Failed { code: 1 });
    }

    #[test]
    fn exit_code_seventeen_is_failed_with_code_preserved() {
        assert_eq!(classify_exit_code(17), RefreshOutcome::Failed { code: 17 });
    }

    /// A negative code (as `wait_status`/signal-derived codes can surface
    /// as, depending on how the caller maps `ExitStatus`) is still threaded
    /// through unmodified rather than being special-cased.
    #[test]
    fn negative_exit_code_is_failed_with_code_preserved() {
        assert_eq!(classify_exit_code(-1), RefreshOutcome::Failed { code: -1 });
    }

    /// Failure-to-spawn is a distinct result from any exit-code-derived
    /// outcome, never collapsed into `Failed`.
    #[test]
    fn spawn_error_is_spawn_failed_not_failed() {
        let err = std::io::Error::new(std::io::ErrorKind::NotFound, "No such file or directory");
        let outcome = classify_spawn_error(&err);
        match outcome {
            RefreshOutcome::SpawnFailed { reason } => {
                assert!(reason.contains("No such file or directory"));
            }
            other => panic!("expected SpawnFailed, got {other:?}"),
        }
    }

    #[test]
    fn spawn_failed_is_distinct_variant_from_every_exit_code_outcome() {
        let spawn_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let spawn_outcome = classify_spawn_error(&spawn_err);
        assert_ne!(spawn_outcome, RefreshOutcome::Current);
        assert_ne!(spawn_outcome, RefreshOutcome::StaleOrChanged);
        assert_ne!(spawn_outcome, RefreshOutcome::Failed { code: 1 });
    }

    // -- task 3: thin spawn shell (AC-3) --------------------------------

    /// `spawn_argv` returns a live `Child` without waiting for it, and the
    /// classifier round-trips the eventual exit status correctly. Uses
    /// `true`/`false` (not `refresh.py`) as the stand-in — see the ui.rs
    /// non-blocking event-loop test for the "spawn returns before a
    /// long-lived child exits" ordering assertion this AC actually cares
    /// about.
    #[test]
    fn spawn_argv_runs_the_given_program_and_exit_code_round_trips() {
        let mut child = spawn_argv(&["true".to_string()]).expect("spawn 'true'");
        let status = child.wait().expect("wait on spawned child");
        assert_eq!(
            classify_exit_code(status.code().expect("exit code")),
            RefreshOutcome::Current
        );

        let mut child = spawn_argv(&["false".to_string()]).expect("spawn 'false'");
        let status = child.wait().expect("wait on spawned child");
        assert_eq!(
            classify_exit_code(status.code().expect("exit code")),
            RefreshOutcome::Failed { code: 1 }
        );
    }

    /// A program that does not exist on `PATH` is a spawn failure, not a
    /// process that ran and exited non-zero.
    #[test]
    fn spawn_argv_missing_program_is_a_spawn_error() {
        let err = spawn_argv(&["definitely-not-a-real-binary-xyz".to_string()])
            .expect_err("spawning a nonexistent program must error");
        let outcome = classify_spawn_error(&err);
        assert!(matches!(outcome, RefreshOutcome::SpawnFailed { .. }));
    }
}
