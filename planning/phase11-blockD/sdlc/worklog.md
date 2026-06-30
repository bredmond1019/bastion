# Worklog — phase11-blockD

## Task 1 — PASSED (1 attempt)
What: Added a pure status.md parser (parse_status/RepoStatus) extracting D30 frontmatter scalars and the five Momentum queue lines, with fixtures and exhaustive unit tests, registered in src/serve/status/mod.rs.
Decisions: RepoStatus.name and has_handoff default to empty/false from parse_status since they aren't derivable from file content alone — Task 4 handlers fill them in from the workspace registry and a separate handoff.md existence check, matching the spec's 'passed in by caller' note.; Reused the existing pub(crate) crate::validate::frontmatter::parse_frontmatter for the YAML-ish frontmatter scan instead of adding a new yaml dependency, keeping it consistent with src/brain/okf.rs's approach.; parse_bullet accepts both em dash (—) and plain hyphen (-) as the key/text separator to tolerate loose authoring style across status.md files.
Validated: gating checks (fast tripwire)
