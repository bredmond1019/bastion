// TUI application state: selected run, selected node cursor, poll interval.

use crate::db::workflows::WorkflowRun;

pub struct App {
    pub runs: Vec<WorkflowRun>,
    pub selected_run: usize,
    pub selected_node: usize,
    pub should_quit: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            runs: vec![],
            selected_run: 0,
            selected_node: 0,
            should_quit: false,
        }
    }

    pub fn selected_run(&self) -> Option<&WorkflowRun> {
        self.runs.get(self.selected_run)
    }
}
