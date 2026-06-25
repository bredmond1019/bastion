# Worklog — phase6-blockB

## Task 1 — PASSED (1 attempt)
What: Added workspace registry + pure resolver to config.rs: FileConfig extended with workspaces/default_workspace fields, ConfigError::UnknownWorkspace variant, pure resolve_workspace_root() with explicit-root > named-workspace > default-workspace > built-in-dot precedence, and DB-free load_workspace_registry() loader; 13 new tests covering all resolver paths and TOML round-trip.
Decisions: Used ConfigError::UnknownWorkspace(String) variant (not a separate WorkspaceError type) to keep the single error enum pattern consistent with the existing codebase; resolve_workspace_root returns PathBuf::from('.') as built-in default to preserve Block A behavior exactly; load_workspace_registry silently degrades to FileConfig::default() on absent/unreadable config file (same contract as Config::load) but propagates MalformedFile on parse errors
Validated: gating checks (fast tripwire)
