// TUI application state: selected run, selected node cursor, poll interval.
// Pure state only — no ratatui draw calls, no tokio, no I/O.

use crate::db::workflows::{NodeState, WorkflowRun};
use crate::monitor::graph::GraphLayout;
use crate::sessions::model::Session;

#[derive(Debug, Clone)]
pub enum MissionItem {
    Session(Session),
    Run(WorkflowRun),
}

pub fn build_mission_items(sessions: &[Session], runs: &[WorkflowRun]) -> Vec<MissionItem> {
    let mut items = Vec::new();
    items.extend(sessions.iter().cloned().map(MissionItem::Session));
    items.extend(runs.iter().cloned().map(MissionItem::Run));

    items.sort_by_key(|item| match item {
        MissionItem::Session(s) => {
            if s.agent_state == crate::detect::AgentState::Blocked {
                0
            } else if s.agent_state == crate::detect::AgentState::Working
                || s.state == crate::sessions::model::SessionState::Running
            {
                1
            } else {
                2
            }
        }
        MissionItem::Run(r) => {
            if r.status == crate::db::workflows::RunStatus::Running
                || r.status == crate::db::workflows::RunStatus::Pending
            {
                1
            } else {
                2
            }
        }
    });

    items
}

pub struct App {
    pub items: Vec<MissionItem>,
    /// Graph layout for the currently selected run (rebuilt on each poll tick).
    pub layout: Option<GraphLayout>,
    pub selected: usize,
    pub selected_node: usize,
    pub should_quit: bool,
    /// Optional banner message shown at the bottom of the TUI (errors, status).
    pub banner: Option<String>,
}

impl App {
    pub fn new() -> Self {
        Self {
            items: vec![],
            layout: None,
            selected: 0,
            selected_node: 0,
            should_quit: false,
            banner: None,
        }
    }

    /// Signal the event loop to exit cleanly.
    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    // ── Navigation ─────────────────────────────────────────────────────────────

    /// Move the node cursor forward by one, clamped to the last node.
    pub fn next_node(&mut self) {
        let node_count = self.selected_run().map(|r| r.nodes.len()).unwrap_or(0);
        if node_count > 0 {
            self.selected_node = (self.selected_node + 1).min(node_count - 1);
        }
    }

    /// Move the node cursor backward by one, clamped to zero.
    pub fn prev_node(&mut self) {
        self.selected_node = self.selected_node.saturating_sub(1);
    }

    /// Switch to the next run, clamping at the last run. Resets the node cursor.
    pub fn next_item(&mut self) {
        if !self.items.is_empty() {
            self.selected = (self.selected + 1).min(self.items.len() - 1);
            self.selected_node = 0;
        }
    }

    /// Switch to the previous run, clamping at zero. Resets the node cursor.
    pub fn prev_item(&mut self) {
        self.selected = self.selected.saturating_sub(1);
        self.selected_node = 0;
    }

    // ── Accessors ──────────────────────────────────────────────────────────────

    /// The currently selected `WorkflowRun`, or `None` when `runs` is empty.
    pub fn selected_item(&self) -> Option<&MissionItem> {
        self.items.get(self.selected)
    }

    pub fn selected_run(&self) -> Option<&WorkflowRun> {
        match self.selected_item() {
            Some(MissionItem::Run(r)) => Some(r),
            _ => None,
        }
    }

    pub fn selected_session(&self) -> Option<&Session> {
        match self.selected_item() {
            Some(MissionItem::Session(s)) => Some(s),
            _ => None,
        }
    }

    /// The currently selected `NodeState`, or `None` when the run or its node
    /// list is empty.
    pub fn selected_node(&self) -> Option<&NodeState> {
        self.selected_run()
            .and_then(|r| r.nodes.get(self.selected_node))
    }

    // ── State update ───────────────────────────────────────────────────────────

    /// Swap in freshly-polled runs while keeping the selection valid.
    ///
    /// - If the new run count is shorter, `selected_run` is clamped to the
    ///   last available run and `selected_node` is reset to 0.
    /// - If the new node count for the selected run is shorter,
    ///   `selected_node` is clamped to the last available node.
    /// - If `new_items` is empty both cursors are reset to 0.
    pub fn replace_items(&mut self, new_items: Vec<MissionItem>) {
        self.items = new_items;
        if self.items.is_empty() {
            self.selected = 0;
            self.selected_node = 0;
            return;
        }
        // Clamp run cursor.
        if self.selected >= self.items.len() {
            self.selected = self.items.len() - 1;
            self.selected_node = 0;
        }
        // Clamp node cursor within the (possibly new) selected run.
        let node_count = match &self.items[self.selected] {
            MissionItem::Run(r) => r.nodes.len(),
            MissionItem::Session(_) => 0,
        };
        if node_count == 0 {
            self.selected_node = 0;
        } else if self.selected_node >= node_count {
            self.selected_node = node_count - 1;
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::workflows::{NodeState, RunStatus, WorkflowRun};

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn make_node(name: &str) -> NodeState {
        NodeState {
            id: name.to_string(),
            name: name.to_string(),
            status: RunStatus::Pending,
            depends_on: vec![],
            input: None,
            output: None,
            error: None,
            tokens_in: None,
            tokens_out: None,
            model: None,
            started_at: None,
            elapsed_secs: None,
        }
    }

    fn make_run(id: &str, node_names: &[&str]) -> WorkflowRun {
        WorkflowRun {
            id: id.to_string(),
            workflow_name: "test_workflow".to_string(),
            status: RunStatus::Running,
            nodes: node_names.iter().map(|n| make_node(n)).collect(),
            started_at: None,
            elapsed_secs: None,
        }
    }

    // ── quit ─────────────────────────────────────────────────────────────────

    #[test]
    fn quit_sets_should_quit() {
        let mut app = App::new();
        assert!(!app.should_quit);
        app.quit();
        assert!(app.should_quit);
    }

    // ── empty state ───────────────────────────────────────────────────────────

    #[test]
    fn empty_items_selected_run_returns_none() {
        let app = App::new();
        assert!(app.selected_run().is_none());
    }

    #[test]
    fn empty_items_selected_node_returns_none() {
        let app = App::new();
        assert!(app.selected_node().is_none());
    }

    #[test]
    fn run_with_no_nodes_selected_node_returns_none() {
        let mut app = App::new();
        app.items = vec![MissionItem::Run(make_run("r1", &[]))];
        assert!(app.selected_node().is_none());
    }

    // ── next_node / prev_node ─────────────────────────────────────────────────

    #[test]
    fn next_node_advances_cursor() {
        let mut app = App::new();
        app.items = vec![MissionItem::Run(make_run("r1", &["A", "B", "C"]))];
        assert_eq!(app.selected_node, 0);
        app.next_node();
        assert_eq!(app.selected_node, 1);
    }

    #[test]
    fn next_node_clamps_at_last() {
        let mut app = App::new();
        app.items = vec![MissionItem::Run(make_run("r1", &["A", "B"]))];
        app.selected_node = 1; // already at last
        app.next_node();
        assert_eq!(app.selected_node, 1, "must not advance past the last node");
        app.next_node();
        assert_eq!(app.selected_node, 1, "still clamped");
    }

    #[test]
    fn prev_node_retreats_cursor() {
        let mut app = App::new();
        app.items = vec![MissionItem::Run(make_run("r1", &["A", "B", "C"]))];
        app.selected_node = 2;
        app.prev_node();
        assert_eq!(app.selected_node, 1);
    }

    #[test]
    fn prev_node_clamps_at_zero() {
        let mut app = App::new();
        app.items = vec![MissionItem::Run(make_run("r1", &["A", "B"]))];
        app.selected_node = 0;
        app.prev_node();
        assert_eq!(app.selected_node, 0, "must not go below zero");
        app.prev_node();
        assert_eq!(app.selected_node, 0, "still clamped");
    }

    #[test]
    fn next_node_on_empty_items_does_not_panic() {
        let mut app = App::new();
        app.next_node(); // empty runs → selected_run() is None
        assert_eq!(app.selected_node, 0);
    }

    #[test]
    fn prev_node_on_empty_items_does_not_panic() {
        let mut app = App::new();
        app.prev_node();
        assert_eq!(app.selected_node, 0);
    }

    // ── next_run / prev_run ───────────────────────────────────────────────────

    #[test]
    fn next_run_advances_cursor() {
        let mut app = App::new();
        app.items = vec![
            MissionItem::Run(make_run("r1", &["A"])),
            MissionItem::Run(make_run("r2", &["B"])),
        ];
        app.next_item();
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn next_run_clamps_at_last_run() {
        let mut app = App::new();
        app.items = vec![
            MissionItem::Run(make_run("r1", &["A"])),
            MissionItem::Run(make_run("r2", &["B"])),
        ];
        app.selected = 1;
        app.next_item();
        assert_eq!(app.selected, 1, "must not advance past the last run");
        app.next_item();
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn prev_run_retreats_cursor() {
        let mut app = App::new();
        app.items = vec![
            MissionItem::Run(make_run("r1", &["A"])),
            MissionItem::Run(make_run("r2", &["B"])),
        ];
        app.selected = 1;
        app.prev_item();
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn prev_run_clamps_at_first_run() {
        let mut app = App::new();
        app.items = vec![
            MissionItem::Run(make_run("r1", &["A"])),
            MissionItem::Run(make_run("r2", &["B"])),
        ];
        app.selected = 0;
        app.prev_item();
        assert_eq!(app.selected, 0, "must not go below zero");
    }

    #[test]
    fn next_run_resets_node_cursor() {
        let mut app = App::new();
        app.items = vec![
            MissionItem::Run(make_run("r1", &["A", "B", "C"])),
            MissionItem::Run(make_run("r2", &["X"])),
        ];
        app.selected_node = 2; // non-zero cursor on run 0
        app.next_item();
        assert_eq!(app.selected, 1);
        assert_eq!(
            app.selected_node, 0,
            "node cursor must reset when switching runs"
        );
    }

    #[test]
    fn prev_run_resets_node_cursor() {
        let mut app = App::new();
        app.items = vec![
            MissionItem::Run(make_run("r1", &["A"])),
            MissionItem::Run(make_run("r2", &["X", "Y", "Z"])),
        ];
        app.selected = 1;
        app.selected_node = 2;
        app.prev_item();
        assert_eq!(app.selected, 0);
        assert_eq!(
            app.selected_node, 0,
            "node cursor must reset when switching runs"
        );
    }

    #[test]
    fn next_run_on_empty_items_does_not_panic() {
        let mut app = App::new();
        app.next_item();
        assert_eq!(app.selected, 0);
    }

    // ── selected_node accessor ────────────────────────────────────────────────

    #[test]
    fn selected_node_returns_correct_node() {
        let mut app = App::new();
        app.items = vec![MissionItem::Run(make_run(
            "r1",
            &["Alpha", "Beta", "Gamma"],
        ))];
        app.selected_node = 1;
        let node = app.selected_node().expect("should return Beta");
        assert_eq!(node.name, "Beta");
    }

    // ── replace_runs ──────────────────────────────────────────────────────────

    #[test]
    fn replace_runs_swaps_runs() {
        let mut app = App::new();
        app.items = vec![MissionItem::Run(make_run("r1", &["A"]))];
        app.replace_items(vec![MissionItem::Run(make_run("r2", &["B", "C"]))]);
        assert_eq!(app.items.len(), 1);
        assert_eq!(app.selected_run().unwrap().id, "r2");
    }

    #[test]
    fn replace_runs_preserves_valid_selection() {
        let mut app = App::new();
        app.items = vec![
            MissionItem::Run(make_run("r1", &["A", "B"])),
            MissionItem::Run(make_run("r2", &["X", "Y"])),
        ];
        app.selected = 1;
        app.selected_node = 1;
        // New runs still have 2 runs, each with 2 nodes
        app.replace_items(vec![
            MissionItem::Run(make_run("r1", &["A", "B"])),
            MissionItem::Run(make_run("r2", &["X", "Y"])),
        ]);
        assert_eq!(app.selected, 1, "run cursor preserved");
        assert_eq!(app.selected_node, 1, "node cursor preserved");
    }

    #[test]
    fn replace_runs_clamps_run_cursor_when_fewer_runs() {
        let mut app = App::new();
        app.items = vec![
            MissionItem::Run(make_run("r1", &["A"])),
            MissionItem::Run(make_run("r2", &["B"])),
        ];
        app.selected = 1;
        // New runs has only one entry
        app.replace_items(vec![MissionItem::Run(make_run("r1", &["A"]))]);
        assert_eq!(app.selected, 0, "run cursor must be clamped to 0");
        assert_eq!(app.selected_node, 0, "node cursor reset after run clamp");
    }

    #[test]
    fn replace_runs_clamps_node_cursor_when_fewer_nodes() {
        let mut app = App::new();
        app.items = vec![MissionItem::Run(make_run("r1", &["A", "B", "C"]))];
        app.selected_node = 2;
        // Same run id but now only 1 node
        app.replace_items(vec![MissionItem::Run(make_run("r1", &["A"]))]);
        assert_eq!(app.selected, 0);
        assert_eq!(
            app.selected_node, 0,
            "node cursor must clamp when run shrinks"
        );
    }

    #[test]
    fn replace_runs_empty_resets_both_cursors() {
        let mut app = App::new();
        app.items = vec![MissionItem::Run(make_run("r1", &["A", "B"]))];
        app.selected = 0;
        app.selected_node = 1;
        app.replace_items(vec![]);
        assert!(app.items.is_empty());
        assert_eq!(app.selected, 0);
        assert_eq!(app.selected_node, 0);
    }

    #[test]
    fn replace_runs_with_run_having_no_nodes_clamps_node_to_zero() {
        let mut app = App::new();
        app.items = vec![MissionItem::Run(make_run("r1", &["A", "B"]))];
        app.selected_node = 1;
        app.replace_items(vec![MissionItem::Run(make_run("r1", &[]))]);
        assert_eq!(app.selected_node, 0);
    }
}
