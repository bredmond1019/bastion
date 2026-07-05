//! `bastion view` / `bastion edit` — thin pass-throughs over bella-engine's terminal markdown
//! viewer (Phase 15, Block BA.15.2 — see D15, D14). No `bella` or `bella-engine` source is
//! touched.
//!
//! ## Resolved entrypoint (see tasks.md §Notes)
//! `bella-engine`'s public surface (`bella_engine::markdown::render_with_edit`) returns a
//! `Rendered` buffer — a one-shot layout of the document, not an interactive loop. The
//! interactive Reader/Browser event loop lives in the `bella` app crate (`../../../bella`,
//! `crates/bella`), but that crate builds a binary only (`[[bin]] name = "bella"`, no `[lib]`
//! target) — its `app`/`events`/`ui` modules are private to that binary and are not part of
//! `bella-engine`'s public API, so they cannot be imported from bastion without modifying
//! bella's `Cargo.toml` (out of scope — no bella source changes).
//!
//! Per the task spec's fallback ("mirror bella's own binary app loop" when no higher-level
//! interactive open exists), the thin-pass-through-preserving choice is to launch the `bella`
//! binary itself as a subprocess with `<path>` as its argument and inherit the controlling
//! terminal — the same "construction vs. execution" shape already used for tmux
//! (`sessions/tmux.rs`): argument-vector construction is pure and unit-tested; the actual
//! spawn is the thin I/O shell, smoke-tested and recorded in tasks.md §Notes.
//!
//! `bella`'s own `Mode` enum (`crates/bella/src/app.rs`) currently has only two interactive
//! modes — `Reader` and `Browser` — with no separate edit-mode CLI flag or keybinding, despite
//! `render_with_edit`'s naming (that API prepares an edit-aware `Rendered` buffer used
//! internally for cursor/click bookkeeping, not a full text editor). `view` and `edit` therefore
//! both resolve to the same `bella <path>` invocation today; they are kept as distinct bastion
//! subcommands/modules so a future bella edit-mode flag has a home here without another CLI
//! shape change.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Name of the bella binary this module shells out to. Resolved via `PATH` (D14 contract:
/// bella is consumed, never vendored/forked).
pub const BELLA_BIN: &str = "bella";

/// Errors from the pure path-validation step, before any process is spawned.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DocViewError {
    /// The requested path does not exist on disk.
    #[error("file not found: {0}")]
    NotFound(PathBuf),
    /// The requested path exists but is a directory, not a file.
    #[error("expected a file, got a directory: {0}")]
    IsDirectory(PathBuf),
}

/// Validate that `path` exists and is a file (pure aside from the two `Path` metadata probes;
/// no process is spawned and no terminal state is touched). Called before either `view` or
/// `edit` shells out, so a missing/invalid path degrades cleanly with a typed error instead of
/// spawning `bella` and letting it fail on its own.
pub fn validate_path(path: &Path) -> Result<(), DocViewError> {
    if !path.exists() {
        return Err(DocViewError::NotFound(path.to_path_buf()));
    }
    if path.is_dir() {
        return Err(DocViewError::IsDirectory(path.to_path_buf()));
    }
    Ok(())
}

/// Returns the argument list for opening `path` in bella's viewer:
///   bella <path>
/// The first element is the `bella` binary name.
pub fn view_args(path: &Path) -> Vec<String> {
    vec![BELLA_BIN.to_string(), path.to_string_lossy().into_owned()]
}

/// Returns the argument list for opening `path` in bella's editor.
///
/// Identical to [`view_args`] today — see the module doc for why: bella has no distinct
/// edit-mode flag yet. Kept as its own function so the two bastion subcommands stay
/// independently wired (and independently testable) if that changes upstream.
pub fn edit_args(path: &Path) -> Vec<String> {
    view_args(path)
}

/// Spawn `bella` with `args` (first element is the binary name), inheriting stdio so the
/// interactive terminal app takes over the current terminal, and block until it exits.
///
/// Thin I/O shell — not unit-tested directly (it replaces the calling process's terminal);
/// smoke-tested manually and recorded in tasks.md §Notes.
fn spawn_bella(args: &[String]) -> Result<()> {
    let (bin, rest) = args
        .split_first()
        .context("bella argument vector must have at least the binary name")?;

    let status = Command::new(bin).args(rest).status().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow::anyhow!("[C001] bella binary not found in PATH — is `../bella` built?")
        } else {
            anyhow::anyhow!("[C010] failed to spawn bella: {e}")
        }
    })?;

    if !status.success() {
        bail!(
            "[C010] bella exited with a non-zero status: {}",
            status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".to_string())
        );
    }

    Ok(())
}

/// `bastion view <path>` — open `path` in bella's terminal markdown viewer.
///
/// Validates `path` first (pure, no process spawned on a bad path), then shells out to the
/// `bella` binary and inherits the terminal for the duration of the interactive session.
pub fn view(path: PathBuf) -> Result<()> {
    validate_path(&path)?;
    spawn_bella(&view_args(&path))
}

/// `bastion edit <path>` — open `path` in bella's editor.
///
/// See the module doc: currently equivalent to [`view`] since bella has no distinct edit mode.
pub fn edit(path: PathBuf) -> Result<()> {
    validate_path(&path)?;
    spawn_bella(&edit_args(&path))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── validate_path ──────────────────────────────────────────────────────────

    #[test]
    fn validate_path_missing_file_errors_not_found() {
        let path = PathBuf::from("/definitely/does/not/exist/doc.md");
        let err = validate_path(&path).unwrap_err();
        assert_eq!(err, DocViewError::NotFound(path));
    }

    #[test]
    fn validate_path_directory_errors_is_directory() {
        let dir = std::env::temp_dir();
        let err = validate_path(&dir).unwrap_err();
        assert_eq!(err, DocViewError::IsDirectory(dir));
    }

    #[test]
    fn validate_path_existing_file_ok() {
        let mut path = std::env::temp_dir();
        path.push(format!("bastion-docview-test-{}.md", std::process::id()));
        std::fs::write(&path, "# hello\n").unwrap();
        let result = validate_path(&path);
        let _ = std::fs::remove_file(&path);
        assert!(result.is_ok());
    }

    #[test]
    fn docview_error_display_not_found() {
        let err = DocViewError::NotFound(PathBuf::from("/tmp/x.md"));
        assert_eq!(err.to_string(), "file not found: /tmp/x.md");
    }

    #[test]
    fn docview_error_display_is_directory() {
        let err = DocViewError::IsDirectory(PathBuf::from("/tmp"));
        assert_eq!(err.to_string(), "expected a file, got a directory: /tmp");
    }

    // ── view_args / edit_args (pure command construction) ──────────────────────

    #[test]
    fn view_args_builds_expected_vector() {
        let path = PathBuf::from("notes.md");
        assert_eq!(
            view_args(&path),
            vec!["bella".to_string(), "notes.md".to_string()]
        );
    }

    #[test]
    fn view_args_preserves_absolute_path() {
        let path = PathBuf::from("/abs/path/doc.md");
        assert_eq!(
            view_args(&path),
            vec!["bella".to_string(), "/abs/path/doc.md".to_string()]
        );
    }

    #[test]
    fn edit_args_matches_view_args_today() {
        let path = PathBuf::from("planning/status.md");
        assert_eq!(edit_args(&path), view_args(&path));
    }

    #[test]
    fn edit_args_builds_expected_vector() {
        let path = PathBuf::from("doc.md");
        assert_eq!(
            edit_args(&path),
            vec!["bella".to_string(), "doc.md".to_string()]
        );
    }

    // ── view()/edit() degrade cleanly on a bad path without spawning bella ─────

    #[test]
    fn view_missing_path_returns_error_without_spawning() {
        let path = PathBuf::from("/definitely/does/not/exist/doc.md");
        let result = view(path);
        assert!(result.is_err());
    }

    #[test]
    fn edit_missing_path_returns_error_without_spawning() {
        let path = PathBuf::from("/definitely/does/not/exist/doc.md");
        let result = edit(path);
        assert!(result.is_err());
    }

    #[test]
    fn view_directory_path_returns_error_without_spawning() {
        let dir = std::env::temp_dir();
        let result = view(dir);
        assert!(result.is_err());
    }
}
