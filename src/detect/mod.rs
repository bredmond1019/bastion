// detect/mod.rs — pure, config-driven agent-state detection.
//
// Per-agent TOML manifests (see manifest.rs) compile into priority-ordered rules;
// `detect()` resolves each rule's screen region and evaluates its gate, returning
// the first matching rule's outcome. Clean-room reimplementation of the Herdr
// detect *pattern* (Herdr is AGPL-3.0 — reference only, no copied source).

pub mod manifest;

#[cfg(test)]
mod golden_tests; // slot owned by spec Task 2 (manifests + fixtures + golden tests)

use manifest::{CompiledManifest, resolve_region};
use serde::{Deserialize, Serialize};

// ── Core types ────────────────────────────────────────────────────────────────

/// Classified state of an agent session from its captured pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Idle,
    Working,
    Blocked,
    Unknown,
}

impl AgentState {
    /// Human-readable lowercase name for this state.
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentState::Idle => "idle",
            AgentState::Working => "working",
            AgentState::Blocked => "blocked",
            AgentState::Unknown => "unknown",
        }
    }
}

/// Narrower sub-classification of `AgentState::Blocked`. Deliberately not a
/// fifth `AgentState` variant — `AgentState` is matched exhaustively in 14+
/// files plus a hand-enumerated wire test, so a sub-classification is threaded
/// as an optional companion field instead of widening that enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockedReason {
    /// A tool-use permission dialog ("Do you want to proceed?") is on screen.
    PermissionPrompt,
    /// An `AskUserQuestion` prompt is on screen, waiting on a multiple-choice
    /// answer rather than a yes/no tool approval.
    AwaitingQuestion,
}

impl BlockedReason {
    /// Human-readable lowercase name for this reason.
    pub fn as_str(&self) -> &'static str {
        match self {
            BlockedReason::PermissionPrompt => "permission_prompt",
            BlockedReason::AwaitingQuestion => "awaiting_question",
        }
    }
}

/// Full detection outcome: the classified state plus the visibility and control
/// flags carried by the matching rule. On no match: `Unknown` with all flags `false`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDetection {
    pub state: AgentState,
    /// Show the "idle" UI indicator.
    pub visible_idle: bool,
    /// Show the "blocker / needs input" UI indicator.
    pub visible_blocker: bool,
    /// Show the "working" UI indicator.
    pub visible_working: bool,
    /// When `true`, the caller should not write a new state record.
    pub skip_state_update: bool,
    /// Sub-classification of `state == Blocked`. `None` for every other state,
    /// and `None` for `Blocked` when the matching rule declared no `reason`.
    pub blocked_reason: Option<BlockedReason>,
}

impl AgentDetection {
    /// The sentinel value returned when no rule in the manifest matches.
    pub fn unknown() -> Self {
        Self {
            state: AgentState::Unknown,
            visible_idle: false,
            visible_blocker: false,
            visible_working: false,
            skip_state_update: false,
            blocked_reason: None,
        }
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Evaluate `manifest`'s compiled rules (sorted descending by priority) against
/// `screen`. Returns the first matching rule's `AgentDetection`, or
/// `AgentDetection::unknown()` when no rule matches.
pub fn detect(screen: &str, manifest: &CompiledManifest) -> AgentDetection {
    for rule in &manifest.rules {
        let region = resolve_region(screen, &rule.region);
        if rule.gate.eval(&region) {
            return AgentDetection {
                state: rule.state,
                visible_idle: rule.visible_idle,
                visible_blocker: rule.visible_blocker,
                visible_working: rule.visible_working,
                skip_state_update: rule.skip_state_update,
                blocked_reason: rule.reason,
            };
        }
    }
    AgentDetection::unknown()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use manifest::parse_manifest;

    // ── AgentState::as_str round-trip ─────────────────────────────────────────

    #[test]
    fn as_str_idle() {
        assert_eq!(AgentState::Idle.as_str(), "idle");
    }

    #[test]
    fn as_str_working() {
        assert_eq!(AgentState::Working.as_str(), "working");
    }

    #[test]
    fn as_str_blocked() {
        assert_eq!(AgentState::Blocked.as_str(), "blocked");
    }

    #[test]
    fn as_str_unknown() {
        assert_eq!(AgentState::Unknown.as_str(), "unknown");
    }

    // ── BlockedReason::as_str round-trip ──────────────────────────────────────

    #[test]
    fn blocked_reason_as_str_permission_prompt() {
        assert_eq!(
            BlockedReason::PermissionPrompt.as_str(),
            "permission_prompt"
        );
    }

    #[test]
    fn blocked_reason_as_str_awaiting_question() {
        assert_eq!(
            BlockedReason::AwaitingQuestion.as_str(),
            "awaiting_question"
        );
    }

    // ── detect() — blocked_reason propagation ─────────────────────────────────

    #[test]
    fn detect_propagates_reason_when_rule_declares_it() {
        let src = r#"
name = "test"

[[rules]]
state = "blocked"
reason = "awaiting_question"
visible_blocker = true
gate = { contains = "question" }
"#;
        let manifest = parse_manifest(src)
            .expect("parse failed")
            .compile()
            .expect("compile failed");

        let detection = detect("a question is waiting", &manifest);
        assert_eq!(detection.state, AgentState::Blocked);
        assert_eq!(
            detection.blocked_reason,
            Some(BlockedReason::AwaitingQuestion)
        );
    }

    #[test]
    fn detect_reason_absent_when_rule_omits_it() {
        let src = r#"
name = "test"

[[rules]]
state = "blocked"
visible_blocker = true
gate = { contains = "proceed" }
"#;
        let manifest = parse_manifest(src)
            .expect("parse failed")
            .compile()
            .expect("compile failed");

        let detection = detect("do you want to proceed", &manifest);
        assert_eq!(detection.state, AgentState::Blocked);
        assert_eq!(detection.blocked_reason, None);
    }

    #[test]
    fn unknown_has_no_blocked_reason() {
        assert_eq!(AgentDetection::unknown().blocked_reason, None);
    }

    #[test]
    fn detect_higher_priority_reason_wins_over_lower_priority_match() {
        // Both rules match; the higher-priority (110) reason-bearing rule must win
        // over the lower-priority (100) rule that also matches and carries no reason.
        let src = r#"
name = "test"

[[rules]]
state = "blocked"
priority = 100
gate = { contains = "proceed" }

[[rules]]
state = "blocked"
priority = 110
reason = "awaiting_question"
gate = { contains = "select" }
"#;
        let manifest = parse_manifest(src)
            .expect("parse failed")
            .compile()
            .expect("compile failed");

        let detection = detect("proceed to select an option", &manifest);
        assert_eq!(
            detection.blocked_reason,
            Some(BlockedReason::AwaitingQuestion)
        );
    }

    // ── detect() — priority ordering ──────────────────────────────────────────

    /// A higher-priority rule must win even when a lower-priority rule also matches.
    #[test]
    fn detect_returns_first_matching_rule_by_priority() {
        // Both rules match "working idle"; the blocked rule (priority 100) should win.
        let src = r#"
name = "test"

[[rules]]
state = "idle"
priority = 1
visible_idle = true
gate = { contains = "idle" }

[[rules]]
state = "blocked"
priority = 100
visible_blocker = true
gate = { contains = "working" }
"#;
        let manifest = parse_manifest(src)
            .expect("parse failed")
            .compile()
            .expect("compile failed");

        let detection = detect("working idle session", &manifest);
        assert_eq!(detection.state, AgentState::Blocked);
        assert!(detection.visible_blocker);
        assert!(!detection.visible_idle);
    }

    // ── detect() — no-match → Unknown ────────────────────────────────────────

    #[test]
    fn detect_no_match_returns_unknown() {
        let src = r#"
name = "test"

[[rules]]
state = "idle"
gate = { contains = "NEVER_PRESENT_IN_SCREEN" }
"#;
        let manifest = parse_manifest(src)
            .expect("parse failed")
            .compile()
            .expect("compile failed");

        let detection = detect("some unrelated screen content", &manifest);
        assert_eq!(detection, AgentDetection::unknown());
    }

    // ── detect() — skip_state_update flag carry-through ──────────────────────

    #[test]
    fn detect_carries_skip_state_update_flag() {
        let src = r#"
name = "test"

[[rules]]
state = "working"
skip_state_update = true
gate = { contains = "spinner" }
"#;
        let manifest = parse_manifest(src)
            .expect("parse failed")
            .compile()
            .expect("compile failed");

        let detection = detect("spinner animation active", &manifest);
        assert_eq!(detection.state, AgentState::Working);
        assert!(detection.skip_state_update);
    }

    // ── detect() — empty manifest → Unknown ──────────────────────────────────

    #[test]
    fn detect_empty_manifest_returns_unknown() {
        let src = r#"name = "test""#;
        let manifest = parse_manifest(src)
            .expect("parse failed")
            .compile()
            .expect("compile failed");

        let detection = detect("any screen content here", &manifest);
        assert_eq!(detection, AgentDetection::unknown());
    }
}
