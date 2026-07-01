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

#[derive(Deserialize)]
struct BrainToml {
    #[serde(default)]
    repos: Vec<SpaceEntry>,
}

pub fn load_space_tree(brain_toml_path: &Path) -> Result<SpaceTree> {
    let content = std::fs::read_to_string(brain_toml_path)?;
    parse_space_tree(&content)
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
}
