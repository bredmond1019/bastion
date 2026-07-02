use anyhow::Result;
use serde::Deserialize;
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
        assert_eq!(flat[0].0, true);
        assert_eq!(flat[0].1, "_root");
        assert_eq!(flat[1].0, false);
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
