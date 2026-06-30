# Worklog — phase11-blockD

## Task 1 — PASSED (1 attempt)
What: Added a pure status.md parser (parse_status/RepoStatus) extracting D30 frontmatter scalars and the five Momentum queue lines, with fixtures and exhaustive unit tests, registered in src/serve/status/mod.rs.
Decisions: RepoStatus.name and has_handoff default to empty/false from parse_status since they aren't derivable from file content alone — Task 4 handlers fill them in from the workspace registry and a separate handoff.md existence check, matching the spec's 'passed in by caller' note.; Reused the existing pub(crate) crate::validate::frontmatter::parse_frontmatter for the YAML-ish frontmatter scan instead of adding a new yaml dependency, keeping it consistent with src/brain/okf.rs's approach.; parse_bullet accepts both em dash (—) and plain hyphen (-) as the key/text separator to tolerate loose authoring style across status.md files.
Validated: gating checks (fast tripwire)

## Task 2 — PASSED (1 attempt)
What: Added pure read_handoff() (handoff.md title+body parser) and FlowState/parse_flow_state/is_terminal/detect_transition (sdlc-flow-state.json parser + terminal-transition detection) with fixtures and 23 new unit tests; all gates (fmt, clippy, test, release build) pass with 931 tests total.
Decisions: read_handoff() body field returns the full raw file content (including frontmatter) rather than stripping frontmatter, since the spec only specifies title+body and Task 4's GET /repos/{name}/handoff handler can decide presentation; heading_title() falls back to the literal string "Handoff" for a bare "# Handoff" heading with no trailing dash text, rather than empty string, to avoid losing all title signal; FlowState intentionally models only the 6 fields the status surface needs (not the full tasks/review/docs/pr shape) — serde ignores unknown fields by default since deny_unknown_fields is not set
Validated: gating checks (fast tripwire)
