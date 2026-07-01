// sessions/app.rs — state model for the session TUI.
//
// Pure (no I/O, no DB — D4/D5); the event loop in ui.rs owns all I/O.
// This module holds every state transition and key→action mapping,
// tested exhaustively without spawning any process.

use crate::sessions::model::Session;
use crossterm::event::KeyCode;

use ratatui::layout::{Constraint, Direction, Layout, Rect};

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum TabState {
    SpaceOverview,
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
}

// ── Constructor + navigation ───────────────────────────────────────────────────

impl AppState {
    pub fn new(sessions: Vec<Session>) -> Self {
        Self {
            tabs: vec![TabState::SpaceOverview, TabState::MissionControl],
            active_tab_index: 0,
            sessions,
            selected: 0,
            mode: Mode::Normal,
            input: String::new(),
            status: Option::None,
            should_quit: false,
            monitor_app: crate::monitor::app::App::new(),
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

    pub fn compute_view(&self, area: Rect) -> (Rect, Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(30), Constraint::Min(0)])
            .split(area);
        (chunks[0], chunks[1])
    }

    /// Move selection to the next session (wraps around).
    pub fn select_next(&mut self) {
        if self.sessions.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.sessions.len();
    }

    /// Move selection to the previous session (wraps around).
    pub fn select_prev(&mut self) {
        if self.sessions.is_empty() {
            return;
        }
        if self.selected == 0 {
            self.selected = self.sessions.len() - 1;
        } else {
            self.selected -= 1;
        }
    }

    pub fn selected_session(&self) -> Option<&Session> {
        self.sessions.get(self.selected)
    }

    /// Replace the session list and clamp `selected` to the new length.
    pub fn set_sessions(&mut self, sessions: Vec<Session>) {
        if sessions.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(sessions.len() - 1);
        }
        self.sessions = sessions;
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
                        Action::None
                    }
                    KeyCode::Up => {
                        self.status = Option::None;
                        self.select_prev();
                        Action::None
                    }
                    KeyCode::Char('a') => {
                        self.status = Option::None;
                        if let Some(s) = self.selected_session() {
                            Action::Attach(s.name.clone())
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
                        if let Some(s) = self.selected_session() {
                            let _name = s.name.clone(); // ensure borrow ends
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
                        if let Some(s) = self.selected_session() {
                            Action::Kill(s.name.clone())
                        } else {
                            Action::None
                        }
                    }
                    KeyCode::Char('q') => {
                        self.should_quit = true;
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
                                if let Some(s) = self.selected_session() {
                                    Action::Send {
                                        session: s.name.clone(),
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

    // ── Constructor ───────────────────────────────────────────────────────────

    #[test]
    fn new_starts_at_zero_normal_mode() {
        let app = AppState::new(make_sessions(&["alpha", "beta"]));
        assert_eq!(app.selected, 0);
        assert_eq!(app.mode, Mode::Normal);
        assert!(!app.should_quit);
        assert!(app.status.is_none());
    }

    // ── Navigation ────────────────────────────────────────────────────────────

    #[test]
    fn select_next_wraps() {
        let mut app = AppState::new(make_sessions(&["a", "b", "c"]));
        app.selected = 2;
        app.select_next();
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn select_prev_wraps() {
        let mut app = AppState::new(make_sessions(&["a", "b", "c"]));
        app.selected = 0;
        app.select_prev();
        assert_eq!(app.selected, 2);
    }

    #[test]
    fn select_next_empty_is_noop() {
        let mut app = AppState::new(vec![]);
        app.select_next();
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn select_prev_empty_is_noop() {
        let mut app = AppState::new(vec![]);
        app.select_prev();
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn single_session_next_prev_stay_at_zero() {
        let mut app = AppState::new(make_sessions(&["only"]));
        app.select_next();
        assert_eq!(app.selected, 0);
        app.select_prev();
        assert_eq!(app.selected, 0);
    }

    // ── set_sessions ─────────────────────────────────────────────────────────

    #[test]
    fn set_sessions_clamps_selected_when_list_shrinks() {
        let mut app = AppState::new(make_sessions(&["a", "b", "c"]));
        app.selected = 2;
        app.set_sessions(make_sessions(&["x"]));
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn set_sessions_empty_resets_to_zero() {
        let mut app = AppState::new(make_sessions(&["a", "b"]));
        app.selected = 1;
        app.set_sessions(vec![]);
        assert_eq!(app.selected, 0);
    }

    // ── selected_session ─────────────────────────────────────────────────────

    #[test]
    fn selected_session_returns_none_when_empty() {
        let app = AppState::new(vec![]);
        assert!(app.selected_session().is_none());
    }

    // ── Input-buffer editing ──────────────────────────────────────────────────

    #[test]
    fn push_backspace_take_input_roundtrip() {
        let mut app = AppState::new(vec![]);
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
        let mut app = AppState::new(make_sessions(&["a", "b", "c"]));
        let a1 = app.on_key(KeyCode::Char('j'));
        assert_eq!(a1, Action::None);
        assert_eq!(app.selected, 1);
        let a2 = app.on_key(KeyCode::Down);
        assert_eq!(a2, Action::None);
        assert_eq!(app.selected, 2);
    }

    #[test]
    fn on_key_up_retreats() {
        let mut app = AppState::new(make_sessions(&["a", "b", "c"]));
        app.selected = 2;
        let a = app.on_key(KeyCode::Up);
        assert_eq!(a, Action::None);
        assert_eq!(app.selected, 1);
    }

    // ── on_key: Normal mode actions ───────────────────────────────────────────

    #[test]
    fn on_key_a_returns_attach_with_selected_name() {
        let mut app = AppState::new(make_sessions(&["my-session"]));
        let action = app.on_key(KeyCode::Char('a'));
        assert_eq!(action, Action::Attach("my-session".into()));
    }

    #[test]
    fn on_key_a_empty_list_is_none() {
        let mut app = AppState::new(vec![]);
        let action = app.on_key(KeyCode::Char('a'));
        assert_eq!(action, Action::None);
    }

    #[test]
    fn on_key_n_enters_new_input_mode() {
        let mut app = AppState::new(vec![]);
        let action = app.on_key(KeyCode::Char('n'));
        assert_eq!(action, Action::None);
        assert_eq!(app.mode, Mode::Input(InputKind::New));
    }

    #[test]
    fn on_key_s_enters_send_input_mode_when_selected() {
        let mut app = AppState::new(make_sessions(&["alpha"]));
        let action = app.on_key(KeyCode::Char('s'));
        assert_eq!(action, Action::None);
        assert_eq!(app.mode, Mode::Input(InputKind::Send));
    }

    #[test]
    fn on_key_s_no_selection_sets_status() {
        let mut app = AppState::new(vec![]);
        let action = app.on_key(KeyCode::Char('s'));
        assert_eq!(action, Action::None);
        assert!(app.status.is_some());
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn on_key_k_returns_kill_with_selected_name() {
        let mut app = AppState::new(make_sessions(&["victim"]));
        let action = app.on_key(KeyCode::Char('k'));
        assert_eq!(action, Action::Kill("victim".into()));
    }

    #[test]
    fn on_key_q_sets_should_quit() {
        let mut app = AppState::new(vec![]);
        let action = app.on_key(KeyCode::Char('q'));
        assert_eq!(action, Action::None);
        assert!(app.should_quit);
    }

    // ── on_key: Input mode ────────────────────────────────────────────────────

    #[test]
    fn input_mode_char_appends() {
        let mut app = AppState::new(vec![]);
        app.mode = Mode::Input(InputKind::New);
        app.on_key(KeyCode::Char('a'));
        app.on_key(KeyCode::Char('b'));
        assert_eq!(app.input, "ab");
    }

    #[test]
    fn input_mode_backspace_pops() {
        let mut app = AppState::new(vec![]);
        app.mode = Mode::Input(InputKind::New);
        app.input = "abc".into();
        app.on_key(KeyCode::Backspace);
        assert_eq!(app.input, "ab");
    }

    #[test]
    fn input_mode_esc_cancels_to_normal() {
        let mut app = AppState::new(vec![]);
        app.mode = Mode::Input(InputKind::New);
        app.input = "partial".into();
        let action = app.on_key(KeyCode::Esc);
        assert_eq!(action, Action::None);
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.input.is_empty());
    }

    #[test]
    fn input_mode_enter_new_returns_new_action() {
        let mut app = AppState::new(vec![]);
        app.mode = Mode::Input(InputKind::New);
        app.input = "fresh".into();
        let action = app.on_key(KeyCode::Enter);
        assert_eq!(action, Action::New("fresh".into()));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn input_mode_enter_new_empty_sets_status() {
        let mut app = AppState::new(vec![]);
        app.mode = Mode::Input(InputKind::New);
        let action = app.on_key(KeyCode::Enter);
        assert_eq!(action, Action::None);
        assert!(app.status.is_some());
    }

    #[test]
    fn input_mode_enter_send_returns_send_action() {
        let mut app = AppState::new(make_sessions(&["target"]));
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
        let mut app = AppState::new(vec![]);
        app.mode = Mode::Input(InputKind::Send);
        app.input = "whatever".into();
        let action = app.on_key(KeyCode::Enter);
        assert_eq!(action, Action::None);
    }

    // ── Tab Management ────────────────────────────────────────────────────────

    #[test]
    fn push_tab_updates_index() {
        let mut app = AppState::new(vec![]);
        app.push_tab(TabState::MissionControl);
        assert_eq!(app.tabs.len(), 3);
        assert_eq!(app.active_tab_index, 2);
        assert_eq!(app.tabs[1], TabState::MissionControl);
    }

    #[test]
    fn close_tab_updates_index() {
        let mut app = AppState::new(vec![]);
        app.push_tab(TabState::MissionControl);
        assert_eq!(app.tabs.len(), 3);
        assert_eq!(app.active_tab_index, 2);

        app.close_tab();
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.active_tab_index, 1); // shifted back

        app.close_tab();
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.active_tab_index, 0); // shifted back
    }

    #[test]
    fn compute_view_math() {
        let app = AppState::new(vec![]);
        let area = Rect::new(0, 0, 100, 50);
        let (sidebar, main) = app.compute_view(area);
        assert_eq!(sidebar.width, 30);
        assert_eq!(sidebar.height, 50);
        assert_eq!(main.width, 70);
        assert_eq!(main.height, 50);
    }
}
