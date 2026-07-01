// sessions/app.rs — state model for the session TUI.
//
// Pure (no I/O, no DB — D4/D5); the event loop in ui.rs owns all I/O.
// This module holds every state transition and key→action mapping,
// tested exhaustively without spawning any process.

use crate::brain::spaces::SpaceTree;
use crate::sessions::model::Session;
use crossterm::event::KeyCode;

use ratatui::layout::{Constraint, Direction, Layout, Rect};

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum TabState {
    SpaceOverview,
    Kanban,
    MissionControl,
    MarkdownDocument(std::path::PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
pub enum InputKind {
    New,
    Send,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Normal,
    Input(InputKind),
}

/// Actions the event loop must execute after `on_key` returns.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Attach(String),
    New(String),
    Send { session: String, keys: String },
    Kill(String),
    SelectTab(usize),
    None,
}

/// State for the interactive session dashboard.
pub struct AppState {
    pub tabs: Vec<TabState>,
    pub active_tab_index: usize,
    pub sessions: Vec<Session>,
    pub selected: usize,
    pub mode: Mode,
    pub input: String,
    /// Transient status/error line shown in the footer. Cleared on the next action.
    pub status: Option<String>,
    pub should_quit: bool,
    pub monitor_app: crate::monitor::app::App,
    pub space_tree: SpaceTree,
    pub selected_space: usize,
}

// ── Constructor + navigation ───────────────────────────────────────────────────

impl AppState {
    pub fn new(sessions: Vec<Session>, space_tree: SpaceTree) -> Self {
        let mut app = Self {
            tabs: vec![
                TabState::SpaceOverview,
                TabState::Kanban,
                TabState::MissionControl,
            ],
            active_tab_index: 0,
            sessions,
            selected: 0,
            mode: Mode::Normal,
            input: String::new(),
            status: Option::None,
            should_quit: false,
            monitor_app: crate::monitor::app::App::new(),
            space_tree,
            selected_space: 0,
        };
        // initialize selected_space to the first non-header if possible
        app.ensure_valid_selection();
        app
    }

    fn ensure_valid_selection(&mut self) {
        let flat = self.space_tree.flatten();
        if flat.is_empty() {
            return;
        }
        if flat.get(self.selected_space).map(|f| f.0).unwrap_or(true) {
            self.select_next();
        }
    }

    pub fn push_tab(&mut self, tab: TabState) {
        self.tabs.push(tab);
        self.active_tab_index = self.tabs.len() - 1;
    }

    pub fn close_tab(&mut self) {
        if self.tabs.len() <= 1 {
            return;
        }
        self.tabs.remove(self.active_tab_index);
        if self.active_tab_index >= self.tabs.len() {
            self.active_tab_index = self.tabs.len() - 1;
        }
    }

    /// Cycle to the next tab, wrapping around to the first.
    pub fn next_tab(&mut self) {
        self.active_tab_index = (self.active_tab_index + 1) % self.tabs.len();
    }

    /// Cycle to the previous tab, wrapping around to the last.
    pub fn prev_tab(&mut self) {
        self.active_tab_index = if self.active_tab_index == 0 {
            self.tabs.len() - 1
        } else {
            self.active_tab_index - 1
        };
    }

    pub fn compute_view(&self, area: Rect) -> (Rect, Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(30), Constraint::Min(0)])
            .split(area);
        (chunks[0], chunks[1])
    }

    /// Move selection to the next session (wraps around).
    pub fn select_next(&mut self) {
        let flat = self.space_tree.flatten();
        if flat.is_empty() {
            return;
        }
        let start = self.selected_space;
        for i in 1..=flat.len() {
            let next = (start + i) % flat.len();
            if !flat[next].0 {
                // not a header
                self.selected_space = next;
                return;
            }
        }
    }

    /// Move selection to the previous session (wraps around).
    pub fn select_prev(&mut self) {
        let flat = self.space_tree.flatten();
        if flat.is_empty() {
            return;
        }
        let start = self.selected_space;
        for i in 1..=flat.len() {
            let prev = if start < i {
                flat.len() - (i - start)
            } else {
                start - i
            };
            if !flat[prev].0 {
                // not a header
                self.selected_space = prev;
                return;
            }
        }
    }

    pub fn selected_session(&self) -> Option<&Session> {
        let flat = self.space_tree.flatten();
        if let Some((_, _, Some(repo))) = flat.get(self.selected_space) {
            return self.sessions.iter().find(|s| s.name == repo.slug);
        }
        None
    }

    pub fn selected_space_slug(&self) -> Option<String> {
        let flat = self.space_tree.flatten();
        if let Some((_, _, Some(repo))) = flat.get(self.selected_space) {
            return Some(repo.slug.clone());
        }
        None
    }

    /// Replace the session list
    pub fn set_sessions(&mut self, sessions: Vec<Session>) {
        self.sessions = sessions;

        let items = crate::monitor::app::build_mission_items(&self.sessions, &[]);
        self.monitor_app.replace_items(items);
    }

    pub fn selected_session_for_actions(&self) -> Option<&Session> {
        if self.tabs[self.active_tab_index] == TabState::MissionControl {
            if let Some(crate::monitor::app::MissionItem::Session(s)) =
                self.monitor_app.selected_item()
            {
                Some(s)
            } else {
                None
            }
        } else {
            self.selected_session()
        }
    }

    // ── Input-buffer editing ──────────────────────────────────────────────────

    pub fn push_input(&mut self, c: char) {
        self.input.push(c);
    }

    pub fn backspace_input(&mut self) {
        self.input.pop();
    }

    /// Take the current input buffer and clear it.
    pub fn take_input(&mut self) -> String {
        std::mem::take(&mut self.input)
    }

    // ── Key→action mapping ────────────────────────────────────────────────────

    /// Map a key event to an `Action`. Mutates mode / selection / input as needed.
    ///
    /// Key-binding decisions:
    /// - Navigation: `Up` and `Down` arrows, plus `j` for down. `k` is **not**
    ///   bound to up-nav; it is the Kill verb in Normal mode to avoid an
    ///   accidental kill on a vim-style up-press.
    pub fn on_key(&mut self, key: KeyCode) -> Action {
        match &self.mode.clone() {
            Mode::Normal => {
                // Clear transient status on any action key.
                match key {
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.status = Option::None;
                        self.select_next();
                        if self.tabs[self.active_tab_index] == TabState::MissionControl {
                            self.monitor_app.next_item();
                        }
                        Action::None
                    }
                    KeyCode::Up => {
                        self.status = Option::None;
                        self.select_prev();
                        if self.tabs[self.active_tab_index] == TabState::MissionControl {
                            self.monitor_app.prev_item();
                        }
                        Action::None
                    }
                    KeyCode::Char('a') => {
                        self.status = Option::None;
                        if let Some(s) = self.selected_session_for_actions() {
                            Action::Attach(s.name.clone())
                        } else if let Some(slug) = self.selected_space_slug() {
                            // Even if not running, we could theoretically try to attach or start, but action expects a name
                            Action::Attach(slug)
                        } else {
                            Action::None
                        }
                    }
                    KeyCode::Char('n') => {
                        self.status = Option::None;
                        self.input.clear();
                        self.mode = Mode::Input(InputKind::New);
                        Action::None
                    }
                    KeyCode::Char('s') => {
                        if let Some(s) = self.selected_session_for_actions() {
                            let _name = s.name.clone(); // ensure borrow ends
                            self.status = Option::None;
                            self.input.clear();
                            self.mode = Mode::Input(InputKind::Send);
                        } else if self.selected_space_slug().is_some() {
                            self.status = Option::None;
                            self.input.clear();
                            self.mode = Mode::Input(InputKind::Send);
                        } else {
                            self.status = Some("no session selected".into());
                        }
                        Action::None
                    }
                    KeyCode::Char('k') => {
                        self.status = Option::None;
                        if let Some(s) = self.selected_session_for_actions() {
                            Action::Kill(s.name.clone())
                        } else if let Some(slug) = self.selected_space_slug() {
                            Action::Kill(slug)
                        } else {
                            Action::None
                        }
                    }
                    KeyCode::Char('q') => {
                        self.should_quit = true;
                        Action::None
                    }
                    KeyCode::Tab => {
                        self.next_tab();
                        Action::None
                    }
                    KeyCode::BackTab => {
                        self.prev_tab();
                        Action::None
                    }
                    _ => Action::None,
                }
            }
            Mode::Input(kind) => {
                let kind = kind.clone();
                match key {
                    KeyCode::Char(c) => {
                        self.push_input(c);
                        Action::None
                    }
                    KeyCode::Backspace => {
                        self.backspace_input();
                        Action::None
                    }
                    KeyCode::Esc => {
                        self.mode = Mode::Normal;
                        self.input.clear();
                        Action::None
                    }
                    KeyCode::Enter => {
                        let text = self.take_input();
                        self.mode = Mode::Normal;
                        match kind {
                            InputKind::New => {
                                if text.is_empty() {
                                    self.status = Some("session name required".into());
                                    Action::None
                                } else {
                                    Action::New(text)
                                }
                            }
                            InputKind::Send => {
                                if let Some(s) = self.selected_session_for_actions() {
                                    Action::Send {
                                        session: s.name.clone(),
                                        keys: text,
                                    }
                                } else if let Some(slug) = self.selected_space_slug() {
                                    Action::Send {
                                        session: slug,
                                        keys: text,
                                    }
                                } else {
                                    Action::None
                                }
                            }
                        }
                    }
                    _ => Action::None,
                }
            }
        }
    }

    /// Map a mouse click to an `Action`.
    pub fn on_mouse(&mut self, col: u16, row: u16, tab_bar_area: Rect) -> Action {
        // Tab bar is rendered inside a block, so the actual text is at y + 1, x + 1
        if row == tab_bar_area.y + 1
            && col > tab_bar_area.x
            && col < tab_bar_area.x + tab_bar_area.width - 1
        {
            let tab_width = 20;
            let clicked_index = ((col - tab_bar_area.x - 1) / tab_width) as usize;
            if clicked_index < self.tabs.len() {
                return Action::SelectTab(clicked_index);
            }
        }
        Action::None
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::model::SessionState;

    fn make_sessions(names: &[&str]) -> Vec<Session> {
        names
            .iter()
            .map(|&name| Session {
                name: name.to_string(),
                state: SessionState::Idle,
                window_count: 1,
                foreground_cmd: String::new(),
                last_line: String::new(),
                agent_state: crate::detect::AgentState::Unknown,
            })
            .collect()
    }

    fn make_app(sessions: &[Session]) -> AppState {
        let mut tree = SpaceTree::default();
        tree.tiers.push((
            "core".to_string(),
            sessions
                .iter()
                .map(|s| crate::brain::spaces::SpaceEntry {
                    slug: s.name.clone(),
                    tier: "core".to_string(),
                    repo_path: std::path::PathBuf::from(s.name.clone()),
                    heading: None,
                })
                .collect(),
        ));
        AppState::new(sessions.to_vec(), tree)
    }

    // ── Constructor ───────────────────────────────────────────────────────────

    #[test]
    fn new_starts_at_first_non_header_normal_mode() {
        let app = make_app(&make_sessions(&["alpha", "beta"]));
        assert_eq!(app.selected_space, 1);
        assert_eq!(app.mode, Mode::Normal);
        assert!(!app.should_quit);
        assert!(app.status.is_none());
    }

    // ── Navigation ────────────────────────────────────────────────────────────

    #[test]
    fn select_next_wraps() {
        let mut app = make_app(&make_sessions(&["a", "b", "c"]));
        app.selected_space = 3;
        app.select_next();
        assert_eq!(app.selected_space, 1);
    }

    #[test]
    fn select_prev_wraps() {
        let mut app = make_app(&make_sessions(&["a", "b", "c"]));
        app.selected_space = 1;
        app.select_prev();
        assert_eq!(app.selected_space, 3);
    }

    #[test]
    fn select_next_empty_is_noop() {
        let mut app = make_app(&[]);
        app.select_next();
        assert_eq!(app.selected_space, 0);
    }

    #[test]
    fn select_prev_empty_is_noop() {
        let mut app = make_app(&[]);
        app.select_prev();
        assert_eq!(app.selected_space, 0);
    }

    #[test]
    fn single_session_next_prev_stay_at_one() {
        let mut app = make_app(&make_sessions(&["only"]));
        app.select_next();
        assert_eq!(app.selected_space, 1);
        app.select_prev();
        assert_eq!(app.selected_space, 1);
    }

    // ── set_sessions ─────────────────────────────────────────────────────────

    #[test]
    fn set_sessions_does_not_change_selected_space() {
        let mut app = make_app(&make_sessions(&["a", "b", "c"]));
        app.selected_space = 2;
        app.set_sessions(make_sessions(&["x"]));
        assert_eq!(app.selected_space, 2);
    }

    #[test]
    fn set_sessions_empty_does_not_change_selected_space() {
        let mut app = make_app(&make_sessions(&["a", "b"]));
        app.selected_space = 1;
        app.set_sessions(vec![]);
        assert_eq!(app.selected_space, 1);
    }

    // ── selected_session ─────────────────────────────────────────────────────

    #[test]
    fn selected_session_returns_none_when_empty() {
        let app = make_app(&[]);
        assert!(app.selected_session().is_none());
    }

    // ── Input-buffer editing ──────────────────────────────────────────────────

    #[test]
    fn push_backspace_take_input_roundtrip() {
        let mut app = make_app(&[]);
        app.push_input('h');
        app.push_input('i');
        app.backspace_input();
        app.push_input('e');
        let out = app.take_input();
        assert_eq!(out, "he");
        assert!(app.input.is_empty());
    }

    // ── on_key: Normal mode navigation ───────────────────────────────────────

    #[test]
    fn on_key_j_and_down_advance() {
        let mut app = make_app(&make_sessions(&["a", "b", "c"]));
        let a1 = app.on_key(KeyCode::Char('j'));
        assert_eq!(a1, Action::None);
        assert_eq!(app.selected_space, 2);
        let a2 = app.on_key(KeyCode::Down);
        assert_eq!(a2, Action::None);
        assert_eq!(app.selected_space, 3);
    }

    #[test]
    fn on_key_up_retreats() {
        let mut app = make_app(&make_sessions(&["a", "b", "c"]));
        app.selected_space = 3;
        let a = app.on_key(KeyCode::Up);
        assert_eq!(a, Action::None);
        assert_eq!(app.selected_space, 2);
    }

    // ── on_key: Normal mode actions ───────────────────────────────────────────

    #[test]
    fn on_key_a_returns_attach_with_selected_name() {
        let mut app = make_app(&make_sessions(&["my-session"]));
        let action = app.on_key(KeyCode::Char('a'));
        assert_eq!(action, Action::Attach("my-session".into()));
    }

    #[test]
    fn on_key_a_empty_list_is_none() {
        let mut app = make_app(&[]);
        let action = app.on_key(KeyCode::Char('a'));
        assert_eq!(action, Action::None);
    }

    #[test]
    fn on_key_n_enters_new_input_mode() {
        let mut app = make_app(&[]);
        let action = app.on_key(KeyCode::Char('n'));
        assert_eq!(action, Action::None);
        assert_eq!(app.mode, Mode::Input(InputKind::New));
    }

    #[test]
    fn on_key_s_enters_send_input_mode_when_selected() {
        let mut app = make_app(&make_sessions(&["alpha"]));
        let action = app.on_key(KeyCode::Char('s'));
        assert_eq!(action, Action::None);
        assert_eq!(app.mode, Mode::Input(InputKind::Send));
    }

    #[test]
    fn on_key_s_no_selection_sets_status() {
        let mut app = make_app(&[]);
        let action = app.on_key(KeyCode::Char('s'));
        assert_eq!(action, Action::None);
        assert!(app.status.is_some());
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn on_key_k_returns_kill_with_selected_name() {
        let mut app = make_app(&make_sessions(&["victim"]));
        let action = app.on_key(KeyCode::Char('k'));
        assert_eq!(action, Action::Kill("victim".into()));
    }

    #[test]
    fn on_key_q_sets_should_quit() {
        let mut app = make_app(&[]);
        let action = app.on_key(KeyCode::Char('q'));
        assert_eq!(action, Action::None);
        assert!(app.should_quit);
    }

    // ── on_key: Input mode ────────────────────────────────────────────────────

    #[test]
    fn input_mode_char_appends() {
        let mut app = make_app(&[]);
        app.mode = Mode::Input(InputKind::New);
        app.on_key(KeyCode::Char('a'));
        app.on_key(KeyCode::Char('b'));
        assert_eq!(app.input, "ab");
    }

    #[test]
    fn input_mode_backspace_pops() {
        let mut app = make_app(&[]);
        app.mode = Mode::Input(InputKind::New);
        app.input = "abc".into();
        app.on_key(KeyCode::Backspace);
        assert_eq!(app.input, "ab");
    }

    #[test]
    fn input_mode_esc_cancels_to_normal() {
        let mut app = make_app(&[]);
        app.mode = Mode::Input(InputKind::New);
        app.input = "partial".into();
        let action = app.on_key(KeyCode::Esc);
        assert_eq!(action, Action::None);
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.input.is_empty());
    }

    #[test]
    fn input_mode_enter_new_returns_new_action() {
        let mut app = make_app(&[]);
        app.mode = Mode::Input(InputKind::New);
        app.input = "fresh".into();
        let action = app.on_key(KeyCode::Enter);
        assert_eq!(action, Action::New("fresh".into()));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn input_mode_enter_new_empty_sets_status() {
        let mut app = make_app(&[]);
        app.mode = Mode::Input(InputKind::New);
        let action = app.on_key(KeyCode::Enter);
        assert_eq!(action, Action::None);
        assert!(app.status.is_some());
    }

    #[test]
    fn input_mode_enter_send_returns_send_action() {
        let mut app = make_app(&make_sessions(&["target"]));
        app.mode = Mode::Input(InputKind::Send);
        app.input = "cargo test".into();
        let action = app.on_key(KeyCode::Enter);
        assert_eq!(
            action,
            Action::Send {
                session: "target".into(),
                keys: "cargo test".into(),
            }
        );
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn input_mode_enter_send_no_selection_is_none() {
        let mut app = make_app(&[]);
        app.mode = Mode::Input(InputKind::Send);
        app.input = "whatever".into();
        let action = app.on_key(KeyCode::Enter);
        assert_eq!(action, Action::None);
    }

    // ── Tab Management ────────────────────────────────────────────────────────

    #[test]
    fn push_tab_updates_index() {
        let mut app = make_app(&[]);
        app.push_tab(TabState::MissionControl);
        assert_eq!(app.tabs.len(), 4);
        assert_eq!(app.active_tab_index, 3);
        assert_eq!(app.tabs[3], TabState::MissionControl);
    }

    #[test]
    fn close_tab_updates_index() {
        let mut app = make_app(&[]);
        app.push_tab(TabState::MissionControl);
        assert_eq!(app.tabs.len(), 4);
        assert_eq!(app.active_tab_index, 3);

        app.close_tab();
        assert_eq!(app.tabs.len(), 3);
        assert_eq!(app.active_tab_index, 2); // shifted back

        app.close_tab();
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.active_tab_index, 1); // shifted back
    }

    #[test]
    fn next_tab_advances_and_wraps() {
        let mut app = make_app(&[]);
        assert_eq!(app.active_tab_index, 0);
        app.next_tab();
        assert_eq!(app.active_tab_index, 1);
        app.next_tab();
        assert_eq!(app.active_tab_index, 2);
        app.next_tab();
        assert_eq!(app.active_tab_index, 0); // wraps to first
    }

    #[test]
    fn prev_tab_retreats_and_wraps() {
        let mut app = make_app(&[]);
        assert_eq!(app.active_tab_index, 0);
        app.prev_tab();
        assert_eq!(app.active_tab_index, 2); // wraps to last
        app.prev_tab();
        assert_eq!(app.active_tab_index, 1);
    }

    #[test]
    fn tab_key_advances_active_tab() {
        let mut app = make_app(&[]);
        let action = app.on_key(KeyCode::Tab);
        assert_eq!(action, Action::None);
        assert_eq!(app.active_tab_index, 1);
    }

    #[test]
    fn backtab_key_retreats_active_tab() {
        let mut app = make_app(&[]);
        let action = app.on_key(KeyCode::BackTab);
        assert_eq!(action, Action::None);
        assert_eq!(app.active_tab_index, 2);
    }

    #[test]
    fn compute_view_math() {
        let app = make_app(&[]);
        let area = Rect::new(0, 0, 100, 50);
        let (sidebar, main) = app.compute_view(area);
        assert_eq!(sidebar.width, 30);
        assert_eq!(sidebar.height, 50);
        assert_eq!(main.width, 70);
        assert_eq!(main.height, 50);
    }
}
