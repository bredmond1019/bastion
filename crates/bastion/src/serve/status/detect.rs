//! Needs-input detection adapter for the WebSocket hub.
//!
//! Wraps the Block C₀ `detect` engine with a lazy-loaded compiled manifest for
//! the Claude agent (embedded via `include_str!`). Exposes two pure public
//! functions:
//!
//! - [`needs_input`] — returns `true` when the pane shows a permission prompt
//!   (i.e. `AgentState::Blocked` **and** `visible_blocker`).
//! - [`detect_state`] — returns the raw [`AgentState`] for the debounce logic
//!   in the hub actor (Task 4).
//!
//! The compiled manifest is initialised once on first call via [`OnceLock`].
//! Panicking on a malformed seed manifest is intentional — the file ships
//! in-tree and a parse failure is a build-time bug caught by the unit test
//! below.

use std::sync::OnceLock;

use crate::detect::manifest::{CompiledManifest, parse_manifest};
use crate::detect::{self, AgentDetection, AgentState};

// ── Embedded manifest ─────────────────────────────────────────────────────────

/// The in-tree Claude agent manifest source, embedded at compile time.
const CLAUDE_TOML: &str = include_str!("../../detect/manifests/claude.toml");

/// Returns a reference to the once-compiled Claude manifest.
///
/// # Panics
///
/// Panics on first call if the embedded `claude.toml` fails to parse or
/// compile.  This is intentional — the manifest ships in-tree, so a failure
/// is a compile-time bug surfaced by the unit test `claude_toml_parses_and_compiles`.
fn compiled_manifest() -> &'static CompiledManifest {
    static MANIFEST: OnceLock<CompiledManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        parse_manifest(CLAUDE_TOML)
            .expect("embedded claude.toml failed to parse — this is a build-time bug")
            .compile()
            .expect("embedded claude.toml failed to compile — this is a build-time bug")
    })
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Run the Claude manifest against `pane` and return the full [`AgentDetection`].
fn run_detection(pane: &str) -> AgentDetection {
    detect::detect(pane, compiled_manifest())
}

/// Returns `true` when `pane` is in the `Blocked` state **and** `visible_blocker`
/// is set — i.e. Claude is waiting on a permission prompt that the user must
/// resolve.
///
/// This is the signal the WebSocket hub emits as `event{needs_input}`.
pub fn needs_input(pane: &str) -> bool {
    let d = run_detection(pane);
    d.state == AgentState::Blocked && d.visible_blocker
}

/// Returns the raw [`AgentState`] for the hub's needs-input rising-edge debounce.
///
/// One-line passthrough over the same compiled manifest as [`needs_input`].
pub fn detect_state(pane: &str) -> AgentState {
    run_detection(pane).state
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// The embedded `claude.toml` must parse and compile without error.
    /// This guards against future manifest edits breaking the adapter at runtime.
    #[test]
    fn claude_toml_parses_and_compiles() {
        // If this panics, the OnceLock initialiser would panic too — it's the
        // same codepath. Calling it here makes the failure message explicit.
        let result = parse_manifest(CLAUDE_TOML);
        assert!(
            result.is_ok(),
            "claude.toml failed to parse: {:?}",
            result.err()
        );
        let compiled = result.unwrap().compile();
        assert!(
            compiled.is_ok(),
            "claude.toml failed to compile: {:?}",
            compiled.err()
        );
    }

    /// A pane showing a Claude permission prompt → `needs_input` returns `true`.
    #[test]
    fn needs_input_true_for_permission_prompt() {
        let prompt = include_str!("fixtures/needs_input.txt");
        assert!(
            needs_input(prompt),
            "expected needs_input=true for a permission-prompt pane"
        );
    }

    /// A pane showing Claude actively working → `needs_input` returns `false`.
    #[test]
    fn needs_input_false_for_working_pane() {
        let working = include_str!("fixtures/no_input.txt");
        assert!(
            !needs_input(working),
            "expected needs_input=false for a working pane"
        );
    }

    /// An empty pane string → `needs_input` returns `false` (no rule matches).
    #[test]
    fn needs_input_false_for_empty_pane() {
        assert!(
            !needs_input(""),
            "expected needs_input=false for an empty pane"
        );
    }

    /// `detect_state` returns `Blocked` for a permission-prompt pane.
    #[test]
    fn detect_state_blocked_for_permission_prompt() {
        let prompt = include_str!("fixtures/needs_input.txt");
        assert_eq!(
            detect_state(prompt),
            AgentState::Blocked,
            "expected AgentState::Blocked for a permission-prompt pane"
        );
    }

    /// `detect_state` returns `Working` for an active-work pane.
    #[test]
    fn detect_state_working_for_active_pane() {
        let working = include_str!("fixtures/no_input.txt");
        assert_eq!(
            detect_state(working),
            AgentState::Working,
            "expected AgentState::Working for an active-work pane"
        );
    }
}
