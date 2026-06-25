# Worklog — phase6-blockB

## Task 1 — PASSED (1 attempt)
What: Added workspace registry + pure resolver to config.rs: FileConfig extended with workspaces/default_workspace fields, ConfigError::UnknownWorkspace variant, pure resolve_workspace_root() with explicit-root > named-workspace > default-workspace > built-in-dot precedence, and DB-free load_workspace_registry() loader; 13 new tests covering all resolver paths and TOML round-trip.
Decisions: Used ConfigError::UnknownWorkspace(String) variant (not a separate WorkspaceError type) to keep the single error enum pattern consistent with the existing codebase; resolve_workspace_root returns PathBuf::from('.') as built-in default to preserve Block A behavior exactly; load_workspace_registry silently degrades to FileConfig::default() on absent/unreadable config file (same contract as Config::load) but propagates MalformedFile on parse errors
Validated: gating checks (fast tripwire)

## Task 2 — PASSED (1 attempt)
What: Added portable OKF fixture corpus (client/project domain) and 6 portability tests in okf.rs proving build_node_edge_lists works over any conforming corpus, not just the bastion decision graph.
Decisions: Used include_str! macros in the test module to embed fixture files at compile time, keeping tests self-contained without filesystem I/O; Chose a client/project knowledge domain (proj-overview, team-roster, req-doc, tech-spec, stale-note) to maximally differentiate from the Block A decision-graph domain (d3, d20, d21, d4); No production code changes required — build_node_edge_lists was already pure and corpus-agnostic as stated in the task spec
Validated: gating checks (fast tripwire)

## Task 3 — PASSED (1 attempt)
What: Wire --workspace/--knowledge-dir flag through CLI and brain::run for named workspace selection; --root changed to Option<PathBuf>; workspace registry loaded DB-free in main.rs
Decisions: Changed --root from PathBuf with default_value='.' to Option<PathBuf> so the resolver can distinguish 'unset' from an explicit path — required for correct precedence (explicit > workspace > default > builtin); Used visible_alias for --knowledge-dir so it appears in --help output as documented in the spec; load_workspace_registry errors (malformed file) propagate as anyhow errors rather than silently degrading, matching the existing config.rs contract where malformed TOML is always an error
Validated: gating checks (fast tripwire)

## Docs
Patched: /Users/brandon/Dev/agentic-portfolio/bastion/trees/phase6-blockB-flow/docs/brain.md, /Users/brandon/Dev/agentic-portfolio/bastion/trees/phase6-blockB-flow/docs/config.md

## Wrap-up — PASS
Next: phase6-blockC (Structural code navigation — code-as-graph, program Block Q)
