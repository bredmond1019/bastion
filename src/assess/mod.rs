//! `bastion assess <path> [--json]` — read-only repo diagnostic (Phase 15, Block BA.15.9).
//!
//! Computes three check families over `okf-core` alone (OKF coverage, graph readiness,
//! state readiness) and prints either a human summary or a mev-convention `--json`
//! envelope. Writes nothing to the filesystem.
//!
//! The ID-convention family is explicitly out of scope until `BA.15.6` lands and must be
//! *absent* from both output modes — never reported as a zero-count entry.
//!
//! Module layout (this task lays the skeleton; tasks 3-7 fill in `okf`/`graph`/`state`/
//! `render`/`run`):
//! - `discover` — context resolution (`brain.toml` / `planning/` / markdown discovery).
//! - `okf` (task 3) — OKF-coverage family.
//! - `graph` (task 4) — graph-readiness family.
//! - `state` (task 5) — state-readiness family.
//! - `render` (task 6) — human + `--json` renderers.
//! - `run` (task 7) — the CLI-facing read-only shell.

mod discover;
mod graph;
mod okf;
mod render;
mod state;

// Re-exported for `assess::run` (task 7) to consume; unused until that shell is wired up.
#[allow(unused_imports)]
pub use discover::{AssessContext, resolve_context};
#[allow(unused_imports)]
pub use graph::{DanglingEdgeFinding, GraphFamily, assess_graph};
#[allow(unused_imports)]
pub use okf::{InvalidFrontmatterFinding, MissingRequiredFinding, OkfFamily, assess_okf};
#[allow(unused_imports)]
pub use render::{Gap, render_human, render_json, top_gaps};
#[allow(unused_imports)]
pub use state::{StateFamily, StateFinding, assess_state, classify_state};

use serde::{Deserialize, Serialize};

/// Top-level `bastion assess` report, following the mev envelope convention
/// (`assessor` + `root` + counts + per-family objects) rather than reusing
/// `mev::JsonReport` directly, since `assess` emits a report shape, not a diagnostic list.
///
/// Deliberately carries **no** id-convention field of any kind: that family is deferred
/// to `BA.15.6` and must be absent from serialized output, never present as a zero.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssessReport {
    pub assessor: String,
    pub root: String,
    pub files_scanned: usize,
    pub okf: OkfFamily,
    pub graph: GraphFamily,
    pub state: StateFamily,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_report() -> AssessReport {
        AssessReport {
            assessor: "assess".to_string(),
            root: "/tmp/example-repo".to_string(),
            files_scanned: 3,
            okf: OkfFamily::default(),
            graph: GraphFamily::default(),
            state: StateFamily::default(),
        }
    }

    #[test]
    fn assess_report_serde_round_trip_preserves_all_fields() {
        let report = sample_report();
        let json = serde_json::to_string(&report).expect("serialize");
        let round_tripped: AssessReport = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(round_tripped, report);
    }

    #[test]
    fn assess_report_json_carries_no_id_convention_key() {
        let json = serde_json::to_string_pretty(&sample_report()).expect("serialize");
        for needle in ["id_convention", "id-convention", "idConvention", "\"ids\""] {
            assert!(
                !json.contains(needle),
                "unexpected id-convention key `{needle}` in AssessReport JSON:\n{json}"
            );
        }
    }

    #[test]
    fn assess_report_json_carries_the_expected_top_level_keys() {
        let json = serde_json::to_string(&sample_report()).expect("serialize");
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let obj = value.as_object().expect("top-level object");
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec!["assessor", "files_scanned", "graph", "okf", "root", "state"]
        );
    }
}
