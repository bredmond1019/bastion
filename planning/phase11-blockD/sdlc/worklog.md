# Worklog — phase11-blockD

## Task 1 — PASSED (1 attempt)
What: Added a pure status.md parser (parse_status/RepoStatus) extracting D30 frontmatter scalars and the five Momentum queue lines, with fixtures and exhaustive unit tests, registered in src/serve/status/mod.rs.
Decisions: RepoStatus.name and has_handoff default to empty/false from parse_status since they aren't derivable from file content alone — Task 4 handlers fill them in from the workspace registry and a separate handoff.md existence check, matching the spec's 'passed in by caller' note.; Reused the existing pub(crate) crate::validate::frontmatter::parse_frontmatter for the YAML-ish frontmatter scan instead of adding a new yaml dependency, keeping it consistent with src/brain/okf.rs's approach.; parse_bullet accepts both em dash (—) and plain hyphen (-) as the key/text separator to tolerate loose authoring style across status.md files.
Validated: gating checks (fast tripwire)

## Task 2 — PASSED (1 attempt)
What: Added pure read_handoff() (handoff.md title+body parser) and FlowState/parse_flow_state/is_terminal/detect_transition (sdlc-flow-state.json parser + terminal-transition detection) with fixtures and 23 new unit tests; all gates (fmt, clippy, test, release build) pass with 931 tests total.
Decisions: read_handoff() body field returns the full raw file content (including frontmatter) rather than stripping frontmatter, since the spec only specifies title+body and Task 4's GET /repos/{name}/handoff handler can decide presentation; heading_title() falls back to the literal string "Handoff" for a bare "# Handoff" heading with no trailing dash text, rather than empty string, to avoid losing all title signal; FlowState intentionally models only the 6 fields the status surface needs (not the full tasks/review/docs/pr shape) — serde ignores unknown fields by default since deny_unknown_fields is not set
Validated: gating checks (fast tripwire)

## Task 3 — PASSED (1 attempt)
What: Added RepoSummaryDto/RepoStatusDto/WorkflowStateDto/WorkflowDonePayload DTOs (with From impls bridging Task 1/2 parser structs) and a pure stateful FlowWatcher in poll.rs that detects non-terminal-to-terminal sdlc-flow-state.json transitions per (repo, spec_slug) and emits workflow_done payloads.
Decisions: Did not add a new WsFrameKind::WorkflowDone variant — per the spec's stated preference, reused the existing Event/EventPayload pattern; WorkflowDonePayload carries the extra repo/spec_slug/status fields for the caller (Task 4) to flatten into the event frame.; FlowWatcher keys its last-known-status map by (repo, spec_slug) tuple rather than spec_slug alone, so identically-named specs across different repos are tracked independently.
Validated: gating checks (fast tripwire)

## Task 4 — PASSED (1 attempt)
What: Added GET /api/repos, /repos/{name}/status, /repos/{name}/handoff, /repos/{name}/workflows REST handlers (thin I/O shells over the Task 1-2 pure parsers), wired them into the bearer-protected /api scope with a shared web::Data<FileConfig> workspace registry, and bumped docs/serve-api.md to v0.3 documenting all four endpoints plus the workflow_done WS event.
Decisions: Loaded the workspace registry once at server startup and shared it via web::Data<FileConfig> (rather than re-reading env/config per request) — this also makes the handlers' 404/200 paths directly unit-testable by injecting a FileConfig in tests, avoiding fragile real-env-var test setup.; build_app() test helper now takes a FileConfig parameter (all 16 existing call sites updated to build_app(FileConfig::default())) so the new repo/workflow tests can inject a fixture registry without touching process env vars.; GET /repos/{name}/workflows returns 404 only for an unknown workspace name; a known workspace with no planning/ tree or no matching sdlc-flow-state.json files returns 200 with [] (malformed flow-state files are skipped individually, not treated as a route failure).; Reused the project's existing no-tempfile-dependency TempDir test helper pattern (from src/validate/mod.rs) in both src/serve/handlers/status.rs and src/serve/mod.rs test modules instead of adding the tempfile crate.; Did not wire FlowWatcher into the Hub actor for an actual live workflow_done push — Task 4's scope (per tasks.md) was REST handlers + routing + docs only; the WS event is documented in docs/serve-api.md as driven by FlowWatcher::observe() (Task 3), with live Hub wiring left to a later block if needed.
Validated: gating checks (fast tripwire)

## Task 5 — PASSED (1 attempt)
What: Confirmed all gated checks (fmt, clippy, test, release build) pass for phase11-blockD; test count is 973 (>908 baseline), docs/serve-api.md confirmed at v0.3 with all four new endpoints and the workflow_done event documented.
Decisions: No code changes needed for Task 5 — it is a pure validation/confirmation step over Tasks 1-4 already committed on this branch.; Recorded validation results in the spec's Notes section rather than creating a separate report file.
Validated: gating checks (fast tripwire)

## Docs
Patched: docs/index.md

## Wrap-up — PASS
Next: Open PR for phase11-blockD; wire FlowWatcher into the live Hub actor for an actual workflow_done WS push (deferred from BA.11.D); then check master-plan for next Phase 11 block or BA.7.B
