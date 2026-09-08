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

use std::path::Path;

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
}
