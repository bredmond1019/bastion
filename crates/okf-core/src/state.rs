//! `state.json` serde schema + block-dependency graph model.
//!
//! Ported (pure model + primitives only) from `mev`'s `brain/state.rs` — the
//! serde structs mirroring `planning/state-schema.md`, plus a loader and a
//! block-dependency graph builder. mev's validation/derivation logic
//! (`check_*`/`derive_*`, `discover_state_files`, `build_graph`, `check_graph`)
//! depends on mev's `Corpus`/`BrainConfig`/`Diagnostic` types and stays in mev;
//! it consumes these shared types instead of duplicating them.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur when loading a `planning/state.json` file.
#[derive(Debug, Error)]
pub enum StateLoadError {
    /// The file could not be read from disk.
    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The file contents are not valid JSON (or do not match [`StateFile`]'s shape).
    #[error("could not parse {path} as JSON: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

// ---------------------------------------------------------------------------
// BlockedBy — internally tagged enum on `type`
// ---------------------------------------------------------------------------

/// A single entry in a `blocked_by[]` / `depends_on[]` / `related[]` array.
///
/// Tagged by the `"type"` field. Unknown `type` values are rejected by serde
/// (no `#[serde(other)]`), surfaced as `StateLoadError::Parse`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BlockedBy {
    /// A dependency on another block (may be cross-repo).
    Block {
        /// Slug of the owning repo.
        repo: String,
        /// Canonical block ID (e.g. `BA.11.C`).
        id: String,
        /// Optional gloss explaining the dependency.
        #[serde(default)]
        what: Option<String>,
    },
    /// An environmental / external dependency (not a tracked block).
    External {
        /// Human description of the external dependency.
        what: String,
    },
}

// ---------------------------------------------------------------------------
// Block — lenient superset across now/next/blocked variants
// ---------------------------------------------------------------------------

/// One entry in a `focus.now`, `focus.next`, or `focus.blocked` array.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Block {
    /// Canonical block ID. `#[serde(alias)]` keeps v1 `"block"`-keyed files readable.
    #[serde(alias = "block")]
    pub id: String,
    /// Brief human description.
    pub title: String,
    /// Lifecycle status (present on `now` and `blocked` entries).
    #[serde(default)]
    pub status: Option<String>,
    /// Optional in-flight context note.
    #[serde(default)]
    pub note: Option<String>,
    /// Cross-repo source repo slug (used in brain `focus` entries).
    #[serde(default)]
    pub repo: Option<String>,
    /// What this block is waiting on (present on `blocked` entries).
    #[serde(default)]
    pub blocked_by: Vec<BlockedBy>,
}

// ---------------------------------------------------------------------------
// Focus
// ---------------------------------------------------------------------------

/// The `focus` object — what's now, next, and blocked in a repo.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Focus {
    /// Blocks currently in progress.
    #[serde(default)]
    pub now: Vec<Block>,
    /// Blocks queued for next (ordered).
    #[serde(default)]
    pub next: Vec<Block>,
    /// Blocks waiting on something.
    #[serde(default)]
    pub blocked: Vec<Block>,
}

// ---------------------------------------------------------------------------
// Track / TrackBlock — leaf roadmap catalog
// ---------------------------------------------------------------------------

/// One block entry inside a `tracks[]` phase/wave.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TrackBlock {
    /// Canonical block ID.
    pub id: String,
    /// Brief human description.
    pub title: String,
    /// Lifecycle status (authored: `open`/`in_progress`/`closed`).
    #[serde(default)]
    pub status: Option<String>,
    /// The block's full dependency edges (the authoritative DAG).
    #[serde(default)]
    pub depends_on: Vec<BlockedBy>,
    /// Execution-order rank for "what's next" (orthogonal to track grouping).
    #[serde(default)]
    pub wave: Option<i64>,
    /// Backlog-promotion provenance, when this block came from a backlog item.
    #[serde(default)]
    pub origin: Option<Origin>,
}

/// One phase/wave entry in a leaf repo's `tracks[]`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Track {
    /// Phase or wave name.
    pub title: String,
    /// Ordered blocks in this phase.
    #[serde(default)]
    pub blocks: Vec<TrackBlock>,
}

// ---------------------------------------------------------------------------
// RepoRollup — brain `repos[]` child headline cache
// ---------------------------------------------------------------------------

/// One child repo's cached headline in a brain `repos[]` entry.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RepoRollup {
    /// Child repo slug.
    pub repo: String,
    /// Tier classification (e.g. `"core"`, `"portfolio"`).
    #[serde(default)]
    pub tier: Option<String>,
    /// Cached `focus.now` from the child.
    #[serde(default)]
    pub now: Vec<Block>,
    /// Cached `focus.next` from the child.
    #[serde(default)]
    pub next: Vec<Block>,
    /// Cached `focus.blocked` from the child.
    #[serde(default)]
    pub blocked: Vec<Block>,
}

// ---------------------------------------------------------------------------
// CrossRepoEdge / Endpoint — brain `cross_repo[]`
// ---------------------------------------------------------------------------

/// One endpoint of a cross-repo dependency edge.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Endpoint {
    /// Repo slug.
    pub repo: String,
    /// Canonical block ID.
    #[serde(alias = "block")]
    pub id: String,
}

/// A directed cross-repo dependency edge in a brain `cross_repo[]`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CrossRepoEdge {
    /// Source endpoint (the dependent block).
    pub from: Endpoint,
    /// Target endpoint (the dependency).
    pub to: Endpoint,
    /// Optional explanation of why this edge exists.
    #[serde(default)]
    pub note: Option<String>,
}

// ---------------------------------------------------------------------------
// TierEntry — HQ `tiers[]`
// ---------------------------------------------------------------------------

/// One tier pointer in the HQ brain `tiers[]`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TierEntry {
    /// Tier name (e.g. `"core"`).
    pub tier: String,
    /// Path or slug to the tier sub-brain, or `null`.
    #[serde(default)]
    pub rollup: Option<String>,
    /// One-line summary of the tier's current state.
    #[serde(default)]
    pub summary: Option<String>,
}

// ---------------------------------------------------------------------------
// Origin — backlog→block promotion provenance (v2)
// ---------------------------------------------------------------------------

/// Provenance pointer on a block that was promoted from a backlog item.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Origin {
    /// Origin kind — `"backlog"` today.
    #[serde(rename = "type")]
    pub kind: String,
    /// The originating backlog node's stable `slug` key.
    pub slug: String,
}

// ---------------------------------------------------------------------------
// Backlog — HQ queued-ideas graph node (v2)
// ---------------------------------------------------------------------------

/// One entry in the HQ brain `backlog[]` — a queued idea as a graph node.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Backlog {
    /// Stable node key (the notes-dir slug).
    pub slug: String,
    /// Human description.
    pub title: String,
    /// Repo the item will land in when promoted (or `"cross-repo"`).
    pub repo: String,
    /// Item kind (`improvement` / `feature` / `chore` / `decision` / …).
    #[serde(rename = "type")]
    pub kind: String,
    /// Lifecycle status: `idea` / `ready` / `promoted`.
    pub status: String,
    /// What the idea is gated on — same edge forms as a block's `depends_on`.
    #[serde(default)]
    pub depends_on: Vec<BlockedBy>,
    /// Set only when `status == "promoted"`: the ID of the block it became.
    #[serde(default)]
    pub block: Option<String>,
    /// Path to the pre-plan notes doc.
    #[serde(default)]
    pub notes: Option<String>,
}

// ---------------------------------------------------------------------------
// Carryover — durable caveats / follow-ons (v3)
// ---------------------------------------------------------------------------

/// The scope of a `carryover[]` entry.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct CarryoverScope {
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub tier: Option<String>,
    #[serde(default)]
    pub cross_repo: Option<bool>,
}

/// A durable caveat, known issue, environmental note, or deferred follow-on.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Carryover {
    /// Stable node key.
    pub slug: String,
    /// Where it applies.
    pub scope: CarryoverScope,
    /// Item kind (`constraint`, `known_issue`, `env`, `deferred`).
    pub kind: String,
    /// The caveat / follow-on text.
    pub text: String,
    /// Optional related edges (same forms as blocked_by).
    #[serde(default)]
    pub related: Vec<BlockedBy>,
    /// Human-readable condition under which this entry should be deleted.
    #[serde(default)]
    pub clears_when: Option<String>,
    /// Date recorded (YYYY-MM-DD).
    pub created: String,
}

// ---------------------------------------------------------------------------
// StateFile — top-level structure
// ---------------------------------------------------------------------------

/// The deserialized contents of a `planning/state.json` file.
///
/// Both leaf (`kind:"project"`) and brain (`kind:"brain"`) variants are covered.
/// All optional collections default to empty; extra unknown fields are
/// tolerated (no `deny_unknown_fields`).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StateFile {
    /// Repo slug identifying this file's owner.
    pub repo: String,
    /// File variant: `"project"` or `"brain"`.
    pub kind: String,
    /// Freshness date string (presence only).
    pub updated: String,
    /// Current work status snapshot.
    #[serde(default)]
    pub focus: Focus,
    /// Roadmap catalog (leaf repos).
    #[serde(default)]
    pub tracks: Vec<Track>,
    /// Child-repo headline cache (brain files).
    #[serde(default)]
    pub repos: Vec<RepoRollup>,
    /// Directed cross-repo dependency edges (brain files).
    #[serde(default)]
    pub cross_repo: Vec<CrossRepoEdge>,
    /// Tier pointers (HQ brain only).
    #[serde(default)]
    pub tiers: Vec<TierEntry>,
    /// Optional top-level annotation note (seen in HQ state.json).
    #[serde(default)]
    pub note: Option<String>,
    /// HQ queued-ideas graph (brain HQ only; empty elsewhere).
    #[serde(default)]
    pub backlog: Vec<Backlog>,
    /// Durable caveats and follow-ons.
    #[serde(default)]
    pub carryover: Vec<Carryover>,
}

// ---------------------------------------------------------------------------
// Loader
// ---------------------------------------------------------------------------

/// Read `path` and deserialize it as a [`StateFile`].
///
/// Returns [`StateLoadError::Io`] if the file cannot be read, or
/// [`StateLoadError::Parse`] if the contents are not valid JSON or do not
/// match the [`StateFile`] schema.
pub fn load_state(path: &Path) -> Result<StateFile, StateLoadError> {
    let contents = std::fs::read_to_string(path).map_err(|e| StateLoadError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    serde_json::from_str(&contents).map_err(|e| StateLoadError::Parse {
        path: path.to_path_buf(),
        source: e,
    })
}

// ---------------------------------------------------------------------------
// StateSource — discovery record (pure data; discovery itself stays in mev)
// ---------------------------------------------------------------------------

/// Metadata about a discovered `planning/state.json` file.
///
/// mev's `discover_state_files` (which *produces* these, walking the
/// filesystem against `BrainConfig`) stays in mev; this is just the pure
/// record that [`build_state_graph`] consumes.
#[derive(Debug, Clone)]
pub struct StateSource {
    /// Identifying slug for this source (e.g. `"hq"`, `"core"`, `"mev"`).
    pub repo_slug: String,
    /// Absolute path to the `planning/state.json` file.
    pub abs_path: PathBuf,
    /// Expected `kind` field value: `"brain"`, `"project"`, or `"portfolio"`.
    pub expected_kind: &'static str,
}

// ---------------------------------------------------------------------------
// State graph model (D4 serializable, emittable artifact)
// ---------------------------------------------------------------------------

/// The kind of a directed edge in the state block graph.
///
/// `BlockedBy` edges come from `tracks[].blocks[].depends_on[]{type:"block"}`
/// entries. `CrossRepo` edges come from brain-file `cross_repo[]` arrays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StateEdgeKind {
    /// A `blocked_by` dependency (a block is waiting on another block).
    BlockedBy,
    /// An explicit cross-repo dependency declared in a brain file's `cross_repo[]`.
    CrossRepo,
}

/// A directed edge in the state block graph.
///
/// `from` and `to_ref` are canonical `"repo:id"` keys. `source_path` is kept
/// for diagnostic generation but **skipped** in serialization.
#[derive(Debug, Clone, Serialize)]
pub struct StateEdge {
    /// `"repo:id"` key of the source block (the dependent / blocked block).
    pub from: String,
    /// `"repo:id"` key of the target block (the dependency / blocker).
    pub to_ref: String,
    /// Edge discriminant.
    pub kind: StateEdgeKind,
    /// Absolute path of the file that authored this edge (skipped in JSON).
    #[serde(skip)]
    pub source_path: PathBuf,
}

/// A graph node — a block registered in a repo's `tracks[]`.
#[derive(Debug, Clone, Serialize)]
pub struct StateNode {
    /// Canonical key: `"repo:id"`.
    pub key: String,
    /// Repo slug that owns this block.
    pub repo: String,
    /// Canonical block ID (e.g. `"MV.3.P"`).
    pub id: String,
    /// Brief human description.
    pub title: String,
    /// Absolute path of the file that registered this block (skipped in JSON).
    #[serde(skip)]
    pub source_path: PathBuf,
}

/// The serializable, emittable state block graph.
///
/// Produced by [`build_state_graph`]. The graph is authored-only — no node or
/// edge is inferred.
#[derive(Debug, Default, Serialize)]
pub struct StateGraph {
    /// All blocks registered in any repo's `tracks[]`.
    pub nodes: Vec<StateNode>,
    /// All `blocked_by` block edges and brain `cross_repo[]` edges.
    pub edges: Vec<StateEdge>,
}

/// Build a [`StateGraph`] from the loaded state files.
///
/// # Nodes
/// One [`StateNode`] per `tracks[].blocks[]` entry across all files (keyed
/// `"repo:id"`).
///
/// # Edges
/// - One [`StateEdge`] with `kind: BlockedBy` per `{type:"block"}` entry in
///   any file's `tracks[].blocks[].depends_on[]`. External entries are
///   skipped — they are leaf constraints, not graph edges.
/// - One [`StateEdge`] with `kind: CrossRepo` per brain-file `cross_repo[]`
///   entry.
pub fn build_state_graph(files: &[(StateSource, StateFile)]) -> StateGraph {
    let mut nodes: Vec<StateNode> = Vec::new();
    let mut edges: Vec<StateEdge> = Vec::new();

    for (src, file) in files {
        let path = &src.abs_path;

        // --- Nodes + BlockedBy edges: from tracks[].blocks[] ---
        for track in &file.tracks {
            for block in &track.blocks {
                let from_key = format!("{}:{}", src.repo_slug, block.id);

                nodes.push(StateNode {
                    key: from_key.clone(),
                    repo: src.repo_slug.clone(),
                    id: block.id.clone(),
                    title: block.title.clone(),
                    source_path: path.clone(),
                });

                // BlockedBy edges: one per {type:block} depends_on entry.
                // External entries are leaf constraints, not graph edges — skip.
                for dep in &block.depends_on {
                    if let BlockedBy::Block { repo, id, .. } = dep {
                        edges.push(StateEdge {
                            from: from_key.clone(),
                            to_ref: format!("{repo}:{id}"),
                            kind: StateEdgeKind::BlockedBy,
                            source_path: path.clone(),
                        });
                    }
                }
            }
        }

        // --- CrossRepo edges: from brain cross_repo[] ---
        for edge in &file.cross_repo {
            edges.push(StateEdge {
                from: format!("{}:{}", edge.from.repo, edge.from.id),
                to_ref: format!("{}:{}", edge.to.repo, edge.to.id),
                kind: StateEdgeKind::CrossRepo,
                source_path: path.clone(),
            });
        }
    }

    StateGraph { nodes, edges }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn project_fixture() -> &'static str {
        r#"{
            "repo": "bastion",
            "kind": "project",
            "updated": "2026-07-01",
            "focus": {
                "now": [
                    {"id": "BA.15.12", "title": "okf-core convergence", "status": "in_progress"}
                ],
                "next": [
                    {"id": "BA.15.13", "title": "next thing"}
                ],
                "blocked": [
                    {
                        "id": "BA.16.A",
                        "title": "blocked thing",
                        "blocked_by": [
                            {"type": "block", "repo": "mev", "id": "MV.3.P"},
                            {"type": "external", "what": "waiting on infra"}
                        ]
                    }
                ]
            },
            "tracks": [
                {
                    "title": "Phase 15",
                    "blocks": [
                        {
                            "id": "BA.15.12",
                            "title": "okf-core convergence",
                            "status": "in_progress",
                            "depends_on": [
                                {"type": "block", "repo": "mev", "id": "MV.3.P"},
                                {"type": "external", "what": "waiting on infra"}
                            ],
                            "wave": 1
                        }
                    ]
                }
            ],
            "carryover": [
                {
                    "slug": "ba15-12-mev-context-seed",
                    "scope": {"repo": "bastion"},
                    "kind": "deferred",
                    "text": "seed mev context",
                    "created": "2026-06-20"
                }
            ]
        }"#
    }

    fn brain_fixture() -> &'static str {
        r#"{
            "repo": "hq",
            "kind": "brain",
            "updated": "2026-07-01",
            "repos": [
                {"repo": "bastion", "tier": "core", "now": [], "next": [], "blocked": []},
                {"repo": "mev", "tier": "core", "now": [], "next": [], "blocked": []}
            ],
            "cross_repo": [
                {
                    "from": {"repo": "bastion", "id": "BA.15.12"},
                    "to": {"repo": "mev", "id": "MV.3.P"},
                    "note": "okf-core convergence"
                }
            ],
            "tiers": [
                {"tier": "core", "rollup": "core/status.md", "summary": "on track"}
            ]
        }"#
    }

    #[test]
    fn load_state_roundtrip_real_fixture() {
        let file: StateFile = serde_json::from_str(project_fixture()).unwrap();
        let round: serde_json::Value = serde_json::to_value(&file).unwrap();
        let original: serde_json::Value = serde_json::from_str(project_fixture()).unwrap();

        // Re-parse the round-tripped value back into a StateFile and re-serialize
        // to confirm fidelity of the model (field-for-field), rather than a raw
        // string diff (key order / whitespace are not part of the contract).
        let reparsed: StateFile = serde_json::from_value(round.clone()).unwrap();
        let reserialized = serde_json::to_value(&reparsed).unwrap();
        assert_eq!(round, reserialized);

        assert_eq!(file.repo, "bastion");
        assert_eq!(file.kind, "project");
        assert_eq!(file.focus.now.len(), 1);
        assert_eq!(file.focus.now[0].id, "BA.15.12");
        assert_eq!(file.tracks[0].blocks[0].depends_on.len(), 2);
        assert_eq!(file.carryover[0].slug, "ba15-12-mev-context-seed");

        // Sanity: original parses too (both are valid StateFile JSON).
        let _: StateFile = serde_json::from_value(original).unwrap();
    }

    #[test]
    fn load_state_brain_fixture() {
        let file: StateFile = serde_json::from_str(brain_fixture()).unwrap();
        assert_eq!(file.repos.len(), 2);
        assert_eq!(file.cross_repo.len(), 1);
        assert_eq!(file.tiers.len(), 1);
    }

    #[test]
    fn blocked_by_unknown_type_is_rejected() {
        let bad = r#"{
            "repo": "bastion",
            "kind": "project",
            "updated": "2026-07-01",
            "tracks": [
                {
                    "title": "Phase 1",
                    "blocks": [
                        {
                            "id": "X",
                            "title": "x",
                            "depends_on": [{"type": "bogus", "repo": "a", "id": "b"}]
                        }
                    ]
                }
            ]
        }"#;
        let result: Result<StateFile, _> = serde_json::from_str(bad);
        assert!(result.is_err());
    }

    #[test]
    fn block_id_alias_reads_v1_key() {
        let v1 = r#"{"block": "BA.1.A", "title": "legacy"}"#;
        let block: Block = serde_json::from_str(v1).unwrap();
        assert_eq!(block.id, "BA.1.A");
    }

    #[test]
    fn load_state_missing_file_is_io_error() {
        let err = load_state(Path::new("/nonexistent/path/state.json")).unwrap_err();
        match err {
            StateLoadError::Io { .. } => {}
            other => panic!("expected Io error, got {other:?}"),
        }
    }

    #[test]
    fn load_state_malformed_json_is_parse_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, "{ not json").unwrap();

        let err = load_state(&path).unwrap_err();
        match err {
            StateLoadError::Parse { .. } => {}
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    fn state_source(repo_slug: &str, path: &Path) -> StateSource {
        StateSource {
            repo_slug: repo_slug.to_string(),
            abs_path: path.to_path_buf(),
            expected_kind: "project",
        }
    }

    #[test]
    fn build_state_graph_nodes_and_edges() {
        let repo_a_json = r#"{
            "repo": "a",
            "kind": "project",
            "updated": "2026-07-01",
            "tracks": [
                {
                    "title": "Phase 1",
                    "blocks": [
                        {
                            "id": "X",
                            "title": "x",
                            "depends_on": [
                                {"type": "block", "repo": "b", "id": "Y"},
                                {"type": "external", "what": "infra"}
                            ]
                        }
                    ]
                }
            ]
        }"#;
        let repo_b_json = r#"{
            "repo": "b",
            "kind": "project",
            "updated": "2026-07-01",
            "tracks": [
                {
                    "title": "Phase 1",
                    "blocks": [
                        {"id": "Y", "title": "y"}
                    ]
                }
            ]
        }"#;

        let path_a = PathBuf::from("/repos/a/planning/state.json");
        let path_b = PathBuf::from("/repos/b/planning/state.json");
        let file_a: StateFile = serde_json::from_str(repo_a_json).unwrap();
        let file_b: StateFile = serde_json::from_str(repo_b_json).unwrap();

        let files = vec![
            (state_source("a", &path_a), file_a),
            (state_source("b", &path_b), file_b),
        ];

        let graph = build_state_graph(&files);

        assert_eq!(graph.nodes.len(), 2);
        assert!(graph.nodes.iter().any(|n| n.key == "a:X"));
        assert!(graph.nodes.iter().any(|n| n.key == "b:Y"));

        let blocked_by_edges: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == StateEdgeKind::BlockedBy)
            .collect();
        assert_eq!(blocked_by_edges.len(), 1);
        assert_eq!(blocked_by_edges[0].from, "a:X");
        assert_eq!(blocked_by_edges[0].to_ref, "b:Y");
    }

    #[test]
    fn build_state_graph_cross_repo_edge() {
        let brain_json = r#"{
            "repo": "hq",
            "kind": "brain",
            "updated": "2026-07-01",
            "cross_repo": [
                {
                    "from": {"repo": "bastion", "id": "BA.15.12"},
                    "to": {"repo": "mev", "id": "MV.3.P"}
                }
            ]
        }"#;
        let path = PathBuf::from("/repos/hq/planning/state.json");
        let file: StateFile = serde_json::from_str(brain_json).unwrap();
        let files = vec![(state_source("hq", &path), file)];

        let graph = build_state_graph(&files);

        assert_eq!(graph.nodes.len(), 0);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].kind, StateEdgeKind::CrossRepo);
        assert_eq!(graph.edges[0].from, "bastion:BA.15.12");
        assert_eq!(graph.edges[0].to_ref, "mev:MV.3.P");
    }
}
