//! Thin I/O shell over the pure momentum/metrics parse core: walks the
//! `[workspaces]` registry, reads each workspace's `planning/status.md`,
//! and assembles the pure [`super::parse::RepoRollup`]s.
//!
//! Degradation contract: a workspace whose `status.md` is missing,
//! unreadable, or fails to parse (no well-formed frontmatter) is skipped —
//! it is not a hard error for the whole rollup.

use std::collections::HashMap;
use std::path::PathBuf;

use super::parse::{RepoRollup, parse_repo_rollup};

/// Read `<root>/planning/status.md` for every `(name, root)` in `registry`,
/// parse each into a [`RepoRollup`], and return the successfully-parsed
/// ones sorted by workspace name.
///
/// A workspace is skipped (not a hard error) when its `status.md` is
/// missing/unreadable, or when [`parse_repo_rollup`] returns `None`
/// (no well-formed frontmatter).
pub fn collect_rollups(registry: &HashMap<String, PathBuf>) -> Vec<RepoRollup> {
    let mut rollups: Vec<RepoRollup> = registry
        .iter()
        .filter_map(|(name, root)| {
            let status_path = root.join("planning").join("status.md");
            let content = std::fs::read_to_string(&status_path).ok()?;
            parse_repo_rollup(&content, name)
        })
        .collect();

    rollups.sort_by(|a, b| a.name.cmp(&b.name));
    rollups
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const WELL_FORMED: &str = include_str!("../serve/status/fixtures/status_well_formed.md");

    /// Builds a temp dir with `<root>/planning/status.md` populated from `content`,
    /// or entirely absent when `content` is `None`.
    fn make_workspace(tmp: &std::path::Path, name: &str, content: Option<&str>) -> PathBuf {
        let root = tmp.join(name);
        let planning = root.join("planning");
        fs::create_dir_all(&planning).expect("create planning dir");
        if let Some(content) = content {
            fs::write(planning.join("status.md"), content).expect("write status.md");
        }
        root
    }

    #[test]
    fn skips_missing_and_malformed_but_keeps_well_formed() {
        let tmp = std::env::temp_dir().join(format!(
            "bastion-momentum-collect-test-{}",
            std::process::id()
        ));
        fs::create_dir_all(&tmp).expect("create tmp root");

        let good_root = make_workspace(&tmp, "good-repo", Some(WELL_FORMED));
        let missing_root = make_workspace(&tmp, "missing-repo", None);

        let mut registry = HashMap::new();
        registry.insert("good-repo".to_string(), good_root);
        registry.insert("missing-repo".to_string(), missing_root);

        let rollups = collect_rollups(&registry);

        assert_eq!(rollups.len(), 1, "only the well-formed repo should survive");
        assert_eq!(rollups[0].name, "good-repo");

        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn skips_repo_with_unparseable_status_md() {
        let tmp = std::env::temp_dir().join(format!(
            "bastion-momentum-collect-test-malformed-{}",
            std::process::id()
        ));
        fs::create_dir_all(&tmp).expect("create tmp root");

        let bad_root = make_workspace(&tmp, "bad-repo", Some("no frontmatter here at all"));

        let mut registry = HashMap::new();
        registry.insert("bad-repo".to_string(), bad_root);

        let rollups = collect_rollups(&registry);

        assert!(
            rollups.is_empty(),
            "malformed status.md should be skipped, not error"
        );

        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn empty_registry_returns_empty_vec() {
        let registry: HashMap<String, PathBuf> = HashMap::new();
        let rollups = collect_rollups(&registry);
        assert!(rollups.is_empty());
    }

    #[test]
    fn results_are_sorted_by_workspace_name() {
        let tmp = std::env::temp_dir().join(format!(
            "bastion-momentum-collect-test-sort-{}",
            std::process::id()
        ));
        fs::create_dir_all(&tmp).expect("create tmp root");

        let zebra_root = make_workspace(&tmp, "zebra", Some(WELL_FORMED));
        let alpha_root = make_workspace(&tmp, "alpha", Some(WELL_FORMED));

        let mut registry = HashMap::new();
        registry.insert("zebra".to_string(), zebra_root);
        registry.insert("alpha".to_string(), alpha_root);

        let rollups = collect_rollups(&registry);

        assert_eq!(rollups.len(), 2);
        assert_eq!(rollups[0].name, "alpha");
        assert_eq!(rollups[1].name, "zebra");

        fs::remove_dir_all(&tmp).ok();
    }
}
