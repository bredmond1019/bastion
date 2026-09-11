//! Shared permission-profile resolution helper for `bastion sweep --once` and
//! `bastion drain --once` (`BA.25.D`).
//!
//! Moved out of `sweep_cli.rs` in task 2 so `drain_cli.rs` can share the exact same
//! resolution logic rather than duplicating it — the block record's own instruction
//! (`planning/BA.25.D/tasks.json`, task 2). No behavior changed from task 1's original.
//!
//! ## The profile is read here and passed through — never defaulted to `Unrestricted`
//!
//! [`resolve_requested_profile`] is deliberately NOT
//! `engine_core::policy::permission::resolve_permission_profile[_from_config]` — those always
//! fail closed to [`PermissionProfile::Locked`] on error, which is a silent SUBSTITUTION from
//! a caller's point of view (asking for a profile that turns out to be unresolvable gets a
//! different profile back, quietly). This block's AC requires REFUSAL — a `--profile` naming
//! an undeclared level is a loud `Err`, never a fallback to any profile, `Locked` included.
//! `None` (no flag given) is the one case that reuses
//! `resolve_permission_profile_from_config`'s resolution of `[permission_profiles].default` —
//! still refused if the default itself doesn't resolve.

use engine_core::policy::permission::{PermissionProfile, resolve_permission_profile_from_config};
use mev::brain::config::PermissionProfilesConfig;
use serde_json::Value;

/// Map a `[permission_profiles.levels.<id>]` wire identifier onto the closed
/// [`PermissionProfile`] enum via its own `#[serde(rename_all = "snake_case")]` `Deserialize`
/// impl — never a hand-rolled second copy of the closed vocabulary. `None` for anything outside
/// the three known ids.
fn parse_profile_id(id: &str) -> Option<PermissionProfile> {
    serde_json::from_value(Value::String(id.to_string())).ok()
}

/// Resolve the [`PermissionProfile`] a sweep or drain runs under.
///
/// `requested = None` (no `--profile` flag) reuses `[permission_profiles].default`'s already-
/// declared level, still validated against `config.levels` and still refused if `default` itself
/// is missing or dangling — reusing
/// [`resolve_permission_profile_from_config`]'s error messages for that case, since its fail-
/// closed diagnostics are exactly what a caller needs here too.
///
/// `requested = Some(name)` looks `name` up in `config.levels` directly and returns the mapped
/// `PermissionProfile`, or an `Err` naming the unknown/absent profile verbatim. **Never a
/// fallback to any profile** — an absent or unrecognized `--profile` is refused, not
/// substituted, even with `Locked` (the tightest level).
pub fn resolve_requested_profile(
    config: &PermissionProfilesConfig,
    requested: Option<&str>,
) -> Result<PermissionProfile, String> {
    match requested {
        Some(name) => {
            let level = config.levels.get(name).ok_or_else(|| {
                format!(
                    "--profile \"{name}\" is not declared in brain.toml's \
                     [permission_profiles.levels] table — refusing rather than substituting a \
                     different profile"
                )
            })?;
            parse_profile_id(&level.id).ok_or_else(|| {
                format!(
                    "--profile \"{name}\" resolves to level id \"{}\", which is outside the \
                     closed locked/standard/unrestricted vocabulary",
                    level.id
                )
            })
        }
        None => {
            let (profile, err) = resolve_permission_profile_from_config(config);
            match err {
                None => Ok(profile),
                Some(source) => Err(format!(
                    "no --profile given, and [permission_profiles].default could not be \
                     resolved: {source}"
                )),
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use mev::brain::config::PermissionProfileLevel;

    fn level(id: &str) -> PermissionProfileLevel {
        PermissionProfileLevel {
            id: id.to_string(),
            meaning: String::new(),
            mini_install: false,
            main_push: id != "locked",
            cross_repo_write: id != "locked",
        }
    }

    fn three_levels() -> BTreeMap<String, PermissionProfileLevel> {
        let mut levels = BTreeMap::new();
        levels.insert("locked".to_string(), level("locked"));
        levels.insert("standard".to_string(), level("standard"));
        levels.insert("unrestricted".to_string(), level("unrestricted"));
        levels
    }

    fn valid_config(default: &str) -> PermissionProfilesConfig {
        PermissionProfilesConfig {
            never_allowed: vec!["clear_operator_gate".to_string()],
            default: Some(default.to_string()),
            levels: three_levels(),
        }
    }

    /// AC-1: `--profile` naming a level absent from `[permission_profiles.levels]` is `Err` —
    /// never a fallback to `locked`, `standard`, or `unrestricted`.
    #[test]
    fn unknown_named_profile_is_refused_not_defaulted() {
        let config = valid_config("standard");
        let err = resolve_requested_profile(&config, Some("unrestricted_but_absent"))
            .expect_err("an undeclared --profile must be refused");
        assert!(err.contains("unrestricted_but_absent"));
    }

    /// The block's own headline case: `--profile unrestricted` when `unrestricted` genuinely
    /// isn't declared is refused, never silently substituted with any other profile.
    #[test]
    fn profile_unrestricted_absent_from_levels_is_refused() {
        let mut config = valid_config("standard");
        config.levels.remove("unrestricted");
        let err = resolve_requested_profile(&config, Some("unrestricted"))
            .expect_err("--profile unrestricted must be refused when undeclared");
        assert!(err.contains("unrestricted"));
    }

    /// A named profile that resolves cleanly returns the mapped `PermissionProfile`.
    #[test]
    fn known_named_profile_resolves() {
        let config = valid_config("standard");
        let profile = resolve_requested_profile(&config, Some("locked"))
            .expect("a declared level must resolve");
        assert_eq!(profile, PermissionProfile::Locked);
    }

    /// AC-2: no `--profile` given and `[permission_profiles].default` itself does not resolve
    /// (here: empty `levels`, so even a well-formed `default` string is dangling) is `Err` — an
    /// absent flag reuses the declared default, it never invents one.
    #[test]
    fn absent_flag_with_unresolvable_default_is_refused() {
        let config = PermissionProfilesConfig {
            never_allowed: vec!["clear_operator_gate".to_string()],
            default: Some("standard".to_string()),
            levels: BTreeMap::new(),
        };
        let err = resolve_requested_profile(&config, None)
            .expect_err("an unresolvable default must be refused, never defaulted to a profile");
        assert!(err.contains("default"));
    }

    /// No `--profile` given and `default` resolves cleanly: the CLI reuses that level.
    #[test]
    fn absent_flag_with_valid_default_resolves() {
        let config = valid_config("unrestricted");
        let profile =
            resolve_requested_profile(&config, None).expect("a well-formed default must resolve");
        assert_eq!(profile, PermissionProfile::Unrestricted);
    }
}
