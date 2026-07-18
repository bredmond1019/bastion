use crate::sessions::model::Session;
use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SpaceEntry {
    pub slug: String,
    pub tier: String,
    pub repo_path: PathBuf,
    pub heading: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpaceTree {
    pub tiers: Vec<(String, Vec<SpaceEntry>)>,
}

impl SpaceTree {
    /// Flatten the tree into an ordered list of items for UI navigation
    /// The resulting `Vec` contains `(is_tier_header, label, Option<&SpaceEntry>)`.
    pub fn flatten(&self) -> Vec<(bool, String, Option<&SpaceEntry>)> {
        let mut list = Vec::new();
        for (tier_name, repos) in &self.tiers {
            list.push((true, tier_name.clone(), None));
            for repo in repos {
                list.push((false, repo.slug.clone(), Some(repo)));
            }
        }
        list
    }
}

/// A single row in the primary-navigation spine, as rendered top-to-bottom in the sidebar.
///
/// This is a presentation layer over [`SpaceTree`] / [`parse_space_tree`] — it does not change
/// how the tree is parsed, only how it is flattened for navigation. The `"_root"` tier is
/// renamed `HQ` here and its `brain` leaf is collapsed into the `Hq` row itself (the brain
/// repo's root, `.`, is the implicit data source for `Hq`); `learn-ai`/`base-template` remain
/// as `Space` rows directly beneath `Hq`.
#[derive(Debug, Clone, PartialEq)]
pub enum SpineRow {
    /// Pinned first: the global cross-space session view.
    MissionControl,
    /// The renamed `"_root"` tier header; its `brain` leaf is collapsed into this row.
    Hq,
    /// A selectable tier header (`core` / `side` / `client` / `portfolio` / any other tier name).
    Tier(String),
    /// A single space (repo) entry, nested under its `Hq` or `Tier` header.
    Space(SpaceEntry),
}

impl SpineRow {
    /// Convert this row into the [`SelectedNode`] it routes to when selected.
    pub fn as_selected_node(&self) -> SelectedNode {
        match self {
            SpineRow::MissionControl => SelectedNode::MissionControl,
            SpineRow::Hq => SelectedNode::Hq,
            SpineRow::Tier(name) => SelectedNode::Tier(name.clone()),
            SpineRow::Space(entry) => SelectedNode::Space(entry.clone()),
        }
    }
}

/// The main-area routing target derived from the selected spine row.
///
/// Shares its shape with [`SpineRow`] by design — `app.rs` derives this from the currently
/// selected row, and `ui.rs` matches on it to decide what to render in the main area.
#[derive(Debug, Clone, PartialEq)]
pub enum SelectedNode {
    MissionControl,
    Hq,
    Tier(String),
    Space(SpaceEntry),
}

/// Flatten a [`SpaceTree`] into the ordered, primary-navigation spine row list:
/// `◆ Mission Control` first, then `HQ` and its children (`learn-ai`/`base-template`,
/// with the redundant `brain` leaf collapsed into the `Hq` row), then the remaining tiers
/// (`core` / `side` / `client` / `portfolio` / any other) with their spaces.
///
/// Does not modify `tree` or [`parse_space_tree`]'s output — purely a presentation-layer view.
pub fn spine_rows(tree: &SpaceTree) -> Vec<SpineRow> {
    let mut rows = vec![SpineRow::MissionControl];

    for (tier_name, entries) in &tree.tiers {
        if tier_name == "_root" {
            rows.push(SpineRow::Hq);
            for entry in entries {
                if entry.slug == "brain" {
                    // Collapsed into the `Hq` row itself (data source = brain root `.`).
                    continue;
                }
                rows.push(SpineRow::Space(entry.clone()));
            }
        } else {
            rows.push(SpineRow::Tier(tier_name.clone()));
            for entry in entries {
                rows.push(SpineRow::Space(entry.clone()));
            }
        }
    }

    rows
}

/// Resolve the [`SpaceEntry`] whose `repo_path` is the longest path-prefix of `cwd`.
///
/// Comparison is component-wise (via [`Path::starts_with`]), not raw string prefix, so a cwd of
/// `/a/foo-bar` does **not** match a repo at `/a/foo`. Among entries whose `repo_path` is a
/// prefix of `cwd`, the one with the most path components wins (deeper repo beats shallower).
///
/// The brain root entry (`repo_path == "."`) is treated as the fallback bucket: it only wins
/// when no other entry's `repo_path` is a prefix of `cwd`. If `cwd` is empty, unparsable, or
/// matches no entry (including no `.` fallback present), returns `None`.
pub fn session_space<'a>(cwd: &str, tree: &'a SpaceTree) -> Option<&'a SpaceEntry> {
    if cwd.is_empty() {
        return None;
    }

    let cwd_path = Path::new(cwd);
    let mut best: Option<&SpaceEntry> = None;
    let mut best_len = 0usize;
    let mut fallback: Option<&SpaceEntry> = None;

    for (_tier, entries) in &tree.tiers {
        for entry in entries {
            if entry.repo_path == Path::new(".") {
                fallback.get_or_insert(entry);
                continue;
            }

            if cwd_path.starts_with(&entry.repo_path) {
                let len = entry.repo_path.components().count();
                if len > best_len || best.is_none() {
                    best = Some(entry);
                    best_len = len;
                }
            }
        }
    }

    best.or(fallback)
}

/// Map each session to the [`SpaceEntry`] its `cwd` resolves to via [`session_space`].
///
/// Returned as a `Vec` of `(&Session, Option<&SpaceEntry>)` pairs, preserving input order, so
/// callers (TUI + `src/serve/*` WS handlers alike) can render or key the mapping however they
/// need without this pure core taking a stance on presentation shape.
pub fn map_sessions_to_spaces<'a>(
    sessions: &'a [Session],
    tree: &'a SpaceTree,
) -> Vec<(&'a Session, Option<&'a SpaceEntry>)> {
    sessions
        .iter()
        .map(|session| (session, session_space(&session.cwd, tree)))
        .collect()
}

/// Same mapping as [`map_sessions_to_spaces`], keyed by session name for O(1) lookup — a
/// convenience shape for callers (e.g. WS handlers) that need to look up a space by session
/// name rather than iterate pairs.
pub fn map_sessions_to_spaces_by_name<'a>(
    sessions: &'a [Session],
    tree: &'a SpaceTree,
) -> HashMap<&'a str, Option<&'a SpaceEntry>> {
    sessions
        .iter()
        .map(|session| (session.name.as_str(), session_space(&session.cwd, tree)))
        .collect()
}

#[derive(Deserialize)]
struct BrainToml {
    #[serde(default)]
    repos: Vec<SpaceEntry>,
}

pub fn load_space_tree(brain_toml_path: &Path) -> Result<SpaceTree> {
    let content = std::fs::read_to_string(brain_toml_path)?;
    let mut tree = parse_space_tree(&content)?;

    if let Some(parent) = brain_toml_path.parent()
        && !parent.as_os_str().is_empty()
    {
        for tier in &mut tree.tiers {
            for repo in &mut tier.1 {
                if repo.repo_path.is_relative() {
                    repo.repo_path = parent.join(&repo.repo_path);
                }
            }
        }
    }

    Ok(tree)
}

pub fn parse_space_tree(content: &str) -> Result<SpaceTree> {
    let doc: BrainToml = toml::from_str(content)?;

    let mut root = Vec::new();
    let mut core = Vec::new();
    let mut side = Vec::new();
    let mut client = Vec::new();
    let mut portfolio = Vec::new();
    let mut other = Vec::new();

    for repo in doc.repos {
        match repo.tier.as_str() {
            "_root" => root.push(repo),
            "core" => core.push(repo),
            "side" => side.push(repo),
            "client" => client.push(repo),
            "portfolio" => portfolio.push(repo),
            _ => other.push(repo),
        }
    }

    let mut tiers = Vec::new();
    if !root.is_empty() {
        tiers.push(("_root".to_string(), root));
    }
    if !core.is_empty() {
        tiers.push(("core".to_string(), core));
    }
    if !side.is_empty() {
        tiers.push(("side".to_string(), side));
    }
    if !client.is_empty() {
        tiers.push(("client".to_string(), client));
    }
    if !portfolio.is_empty() {
        tiers.push(("portfolio".to_string(), portfolio));
    }
    if !other.is_empty() {
        tiers.push(("other".to_string(), other));
    }

    Ok(SpaceTree { tiers })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::model::SessionState;

    fn fixture_tree() -> SpaceTree {
        let toml = r#"
[[repos]]
slug = "brain"
tier = "_root"
repo_path = "."
heading = "Company Brain"

[[repos]]
slug = "bastion"
tier = "core"
repo_path = "/home/user/agentic-portfolio/core/bastion"

[[repos]]
slug = "mev"
tier = "core"
repo_path = "/home/user/agentic-portfolio/core/mev"

[[repos]]
slug = "bella"
tier = "core"
repo_path = "/home/user/agentic-portfolio/core/bastion/bella"
"#;
        parse_space_tree(toml).unwrap()
    }

    fn fixture_session(name: &str, cwd: &str) -> Session {
        Session {
            name: name.to_string(),
            state: SessionState::Idle,
            window_count: 1,
            foreground_cmd: String::new(),
            last_line: String::new(),
            agent_state: crate::detect::AgentState::Unknown,
            cwd: cwd.to_string(),
        }
    }

    #[test]
    fn session_space_deeper_repo_wins_over_shallower() {
        let tree = fixture_tree();
        // Nested under both "core/bastion" and "core/bastion/bella" — the deeper repo must win.
        let entry = session_space("/home/user/agentic-portfolio/core/bastion/bella/src", &tree)
            .expect("expected a match");
        assert_eq!(entry.slug, "bella");
    }

    #[test]
    fn session_space_matches_shallower_repo_when_not_under_deeper_one() {
        let tree = fixture_tree();
        let entry = session_space(
            "/home/user/agentic-portfolio/core/bastion/src/main.rs",
            &tree,
        )
        .expect("expected a match");
        assert_eq!(entry.slug, "bastion");
    }

    #[test]
    fn session_space_does_not_match_sibling_with_shared_prefix() {
        let tree = fixture_tree();
        // "/home/user/agentic-portfolio/core/bastion-ui" shares a string prefix with
        // ".../core/bastion" but is not actually nested under it — component-wise comparison
        // must reject it, falling back to the "." brain-root entry.
        let entry = session_space("/home/user/agentic-portfolio/core/bastion-ui", &tree)
            .expect("expected the brain-root fallback");
        assert_eq!(entry.slug, "brain");
    }

    #[test]
    fn session_space_falls_back_to_brain_root() {
        let tree = fixture_tree();
        let entry = session_space("/home/user/agentic-portfolio/docs/index.md", &tree)
            .expect("expected the brain-root fallback");
        assert_eq!(entry.slug, "brain");
    }

    #[test]
    fn session_space_returns_none_when_no_fallback_present() {
        let toml = r#"
[[repos]]
slug = "bastion"
tier = "core"
repo_path = "/home/user/agentic-portfolio/core/bastion"
"#;
        let tree = parse_space_tree(toml).unwrap();
        assert!(session_space("/somewhere/else", &tree).is_none());
    }

    #[test]
    fn session_space_returns_none_for_empty_cwd() {
        let tree = fixture_tree();
        assert!(session_space("", &tree).is_none());
    }

    #[test]
    fn map_sessions_to_spaces_preserves_order_and_maps_each_session() {
        let tree = fixture_tree();
        let sessions = vec![
            fixture_session("s1", "/home/user/agentic-portfolio/core/bastion/bella/src"),
            fixture_session("s2", "/home/user/agentic-portfolio/core/bastion/src"),
            fixture_session("s3", "/outside/nowhere"),
        ];
        let mapped = map_sessions_to_spaces(&sessions, &tree);

        assert_eq!(mapped.len(), 3);
        assert_eq!(mapped[0].0.name, "s1");
        assert_eq!(mapped[0].1.map(|e| e.slug.as_str()), Some("bella"));
        assert_eq!(mapped[1].0.name, "s2");
        assert_eq!(mapped[1].1.map(|e| e.slug.as_str()), Some("bastion"));
        assert_eq!(mapped[2].0.name, "s3");
        assert_eq!(mapped[2].1.map(|e| e.slug.as_str()), Some("brain"));
    }

    #[test]
    fn map_sessions_to_spaces_by_name_keys_by_session_name() {
        let tree = fixture_tree();
        let sessions = vec![fixture_session(
            "my-session",
            "/home/user/agentic-portfolio/core/mev/src",
        )];
        let mapped = map_sessions_to_spaces_by_name(&sessions, &tree);

        assert_eq!(mapped.len(), 1);
        let entry = mapped.get("my-session").expect("session present").as_ref();
        assert_eq!(entry.map(|e| e.slug.as_str()), Some("mev"));
    }

    #[test]
    fn parses_and_groups_by_tier() {
        let toml = r#"
[[repos]]
slug = "brain"
tier = "_root"
repo_path = "."
heading = "Company Brain"

[[repos]]
slug = "bastion"
tier = "core"
repo_path = "core/bastion"

[[repos]]
slug = "rag-engine-rs"
tier = "portfolio"
repo_path = "portfolio/rag-engine-rs"
"#;
        let tree = parse_space_tree(toml).unwrap();
        assert_eq!(tree.tiers.len(), 3);
        assert_eq!(tree.tiers[0].0, "_root");
        assert_eq!(tree.tiers[0].1[0].slug, "brain");
        assert_eq!(tree.tiers[1].0, "core");
        assert_eq!(tree.tiers[2].0, "portfolio");
    }

    #[test]
    fn flatten_tree() {
        let toml = r#"
[[repos]]
slug = "brain"
tier = "_root"
repo_path = "."
"#;
        let tree = parse_space_tree(toml).unwrap();
        let flat = tree.flatten();
        assert_eq!(flat.len(), 2);
        assert!(flat[0].0);
        assert_eq!(flat[0].1, "_root");
        assert!(!flat[1].0);
        assert_eq!(flat[1].1, "brain");
        assert!(flat[1].2.is_some());
    }

    fn full_toml() -> &'static str {
        r#"
[[repos]]
slug = "brain"
tier = "_root"
repo_path = "."
heading = "Company Brain"

[[repos]]
slug = "learn-ai"
tier = "_root"
repo_path = "learn-ai"

[[repos]]
slug = "base-template"
tier = "_root"
repo_path = "base-template"

[[repos]]
slug = "bastion"
tier = "core"
repo_path = "core/bastion"

[[repos]]
slug = "mev"
tier = "core"
repo_path = "core/mev"

[[repos]]
slug = "amistad"
tier = "side"
repo_path = "side/amistad"

[[repos]]
slug = "rag-engine-rs"
tier = "portfolio"
repo_path = "portfolio/rag-engine-rs"
"#
    }

    #[test]
    fn spine_rows_pins_mission_control_first() {
        let tree = parse_space_tree(full_toml()).unwrap();
        let rows = spine_rows(&tree);
        assert_eq!(rows[0], SpineRow::MissionControl);
    }

    #[test]
    fn spine_rows_renames_root_to_hq_and_collapses_brain_leaf() {
        let tree = parse_space_tree(full_toml()).unwrap();
        let rows = spine_rows(&tree);

        // The Hq row appears right after Mission Control.
        assert_eq!(rows[1], SpineRow::Hq);

        // No standalone `brain` leaf appears anywhere in the spine.
        assert!(!rows.iter().any(|row| matches!(
            row,
            SpineRow::Space(entry) if entry.slug == "brain"
        )));

        // No `_root`-named tier header row appears — it was replaced by `Hq`.
        assert!(
            !rows
                .iter()
                .any(|row| matches!(row, SpineRow::Tier(name) if name == "_root"))
        );
    }

    #[test]
    fn spine_rows_places_learn_ai_and_base_template_under_hq() {
        let tree = parse_space_tree(full_toml()).unwrap();
        let rows = spine_rows(&tree);

        // learn-ai and base-template directly follow the Hq row (before any Tier row).
        assert_eq!(
            rows[2],
            SpineRow::Space(SpaceEntry {
                slug: "learn-ai".to_string(),
                tier: "_root".to_string(),
                repo_path: PathBuf::from("learn-ai"),
                heading: None,
            })
        );
        assert_eq!(
            rows[3],
            SpineRow::Space(SpaceEntry {
                slug: "base-template".to_string(),
                tier: "_root".to_string(),
                repo_path: PathBuf::from("base-template"),
                heading: None,
            })
        );
    }

    #[test]
    fn spine_rows_orders_remaining_tiers_after_hq() {
        let tree = parse_space_tree(full_toml()).unwrap();
        let rows = spine_rows(&tree);

        let tier_headers: Vec<&String> = rows
            .iter()
            .filter_map(|row| match row {
                SpineRow::Tier(name) => Some(name),
                _ => None,
            })
            .collect();
        assert_eq!(tier_headers, vec!["core", "side", "portfolio"]);

        // Spaces under each tier appear after their header.
        let core_idx = rows
            .iter()
            .position(|r| matches!(r, SpineRow::Tier(name) if name == "core"))
            .unwrap();
        assert_eq!(
            rows[core_idx + 1],
            SpineRow::Space(SpaceEntry {
                slug: "bastion".to_string(),
                tier: "core".to_string(),
                repo_path: PathBuf::from("core/bastion"),
                heading: None,
            })
        );
    }

    #[test]
    fn spine_rows_handles_empty_tree() {
        let tree = SpaceTree::default();
        let rows = spine_rows(&tree);
        assert_eq!(rows, vec![SpineRow::MissionControl]);
    }

    #[test]
    fn spine_rows_handles_root_only_tree_without_learn_ai_or_base_template() {
        let toml = r#"
[[repos]]
slug = "brain"
tier = "_root"
repo_path = "."
"#;
        let tree = parse_space_tree(toml).unwrap();
        let rows = spine_rows(&tree);
        assert_eq!(rows, vec![SpineRow::MissionControl, SpineRow::Hq]);
    }

    #[test]
    fn spine_row_as_selected_node_maps_each_variant() {
        let space = SpaceEntry {
            slug: "bastion".to_string(),
            tier: "core".to_string(),
            repo_path: PathBuf::from("core/bastion"),
            heading: None,
        };
        assert_eq!(
            SpineRow::MissionControl.as_selected_node(),
            SelectedNode::MissionControl
        );
        assert_eq!(SpineRow::Hq.as_selected_node(), SelectedNode::Hq);
        assert_eq!(
            SpineRow::Tier("core".to_string()).as_selected_node(),
            SelectedNode::Tier("core".to_string())
        );
        assert_eq!(
            SpineRow::Space(space.clone()).as_selected_node(),
            SelectedNode::Space(space)
        );
    }
}
