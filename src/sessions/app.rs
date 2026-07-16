// sessions/app.rs — state model for the session TUI.
//
// Pure (no I/O, no DB — D4/D5); the event loop in ui.rs owns all I/O.
// This module holds every state transition and key→action mapping,
// tested exhaustively without spawning any process.

use crate::brain::spaces::{SelectedNode, SpaceTree, SpineRow};
use crate::sessions::agent_panel::agent_panel_rows;
use crate::sessions::model::Session;
use crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind};

use ratatui::layout::{Constraint, Direction, Layout, Rect};

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Default)]
pub enum OverviewPane {
    #[default]
    Sidebar,
    Browser,
    Content,
}

/// Per-pane viewport `Rect`s captured during the most recent draw, so the pure
/// mouse dispatcher (BA.13.2) can hit-test clicks/scrolls without a `Frame`.
/// All-zero (`Rect::default()`) before the first draw and for panes that are
/// not part of the current `SelectedNode`'s layout (e.g. `browser`/`content`
/// stay separate, but a pane not rendered this frame is zeroed so `point_in`
/// never matches it).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PaneAreas {
    pub spine: Rect,
    pub browser: Rect,
    pub content: Rect,
    pub agent_panel: Rect,
}

/// Pure layout mirror of `draw_with_root`'s `Layout` splits — computes every
/// pane's viewport `Rect` from a frame size, the already-computed agent-strip
/// height, and the current `SelectedNode`, without touching a `Frame`. Single
/// source of truth for pane geometry: `draw_with_root` consumes this output
/// directly instead of re-deriving the same splits inline.
///
/// - Outer vertical split: main content + the agent-panel strip + a 1-line footer.
/// - `main_chunks`: the primary-nav sidebar (`compute_view`'s `Constraint::Length(30)`)
///   + the remaining main area.
/// - `overview_chunks`: only for `Hq`/`Space` (browser + content, 30-col browser);
///   `Tier`/`MissionControl` route their content to the whole main area and have
///   no browser pane, so `browser` is zero-sized there (`point_in` never matches).
pub fn compute_pane_areas(
    frame_area: Rect,
    agent_strip_height: u16,
    selected_node: &SelectedNode,
) -> PaneAreas {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(agent_strip_height),
            Constraint::Length(1),
        ])
        .split(frame_area);
    let main_area_outer = outer[0];
    let agent_panel = outer[1];

    let main_chunks_outer = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(30), Constraint::Min(0)])
        .split(main_area_outer);
    let spine = main_chunks_outer[0];
    let main_area = main_chunks_outer[1];

    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0)])
        .split(main_area);

    let (browser, content) = match selected_node {
        SelectedNode::Hq | SelectedNode::Space(_) => {
            let overview_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(30), Constraint::Min(0)])
                .split(main_chunks[0]);
            (overview_chunks[0], overview_chunks[1])
        }
        SelectedNode::MissionControl | SelectedNode::Tier(_) => (Rect::default(), main_chunks[0]),
    };

    PaneAreas {
        spine,
        browser,
        content,
        agent_panel,
    }
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
    None,
}

/// State for the interactive session dashboard.
pub struct AppState {
    pub sessions: Vec<Session>,
    pub selected: usize,
    pub mode: Mode,
    pub input: String,
    /// Transient status/error line shown in the footer. Cleared on the next action.
    pub status: Option<String>,
    pub should_quit: bool,
    pub monitor_app: crate::monitor::app::App,
    pub space_tree: SpaceTree,
    /// Index into `spine_rows()` — the single primary-navigation selection cursor.
    pub selected_spine: usize,
    pub file_browser: bella_engine::browser::Browser,
    pub space_overview_scroll: u16,
    pub overview_pane: OverviewPane,
    pub space_overview_file: Option<std::path::PathBuf>,
    /// Transient full-screen markdown overlay flag, set by the `t` ("open") key in the
    /// file browser. Replaces the old tab-push behaviour; overlay rendering/close-key
    /// polish is deferred to a later block.
    pub markdown_overlay: Option<std::path::PathBuf>,
    /// Per-pane viewport `Rect`s from the most recent draw (BA.13.2). Zeroed
    /// (all-default) until the first draw runs.
    pub pane_areas: PaneAreas,
}

// ── Constructor + navigation ───────────────────────────────────────────────────

impl AppState {
    pub fn new(sessions: Vec<Session>, space_tree: SpaceTree) -> Self {
        let mut app = Self {
            sessions,
            selected: 0,
            mode: Mode::Normal,
            input: String::new(),
            status: Option::None,
            should_quit: false,
            monitor_app: crate::monitor::app::App::new(),
            space_tree,
            selected_spine: 0,
            file_browser: bella_engine::browser::Browser::new(std::path::PathBuf::from(".")),
            space_overview_scroll: 0,
            overview_pane: OverviewPane::Sidebar,
            space_overview_file: None,
            markdown_overlay: None,
            pane_areas: PaneAreas::default(),
        };
        // `spine_rows()` always pins Mission Control first, so index 0 is always a
        // valid selection — no header-skip initialization needed.
        app.reinit_browser();
        app
    }

    /// The ordered, flattened primary-navigation spine for the current `space_tree`.
    pub fn spine_rows(&self) -> Vec<SpineRow> {
        crate::brain::spaces::spine_rows(&self.space_tree)
    }

    /// The main-area routing target for the currently selected spine row.
    /// Falls back to `MissionControl` (the pinned first row) if the index is out of
    /// range, which cannot happen in practice since `spine_rows()` is never empty.
    pub fn selected_node(&self) -> SelectedNode {
        self.spine_rows()
            .get(self.selected_spine)
            .map(SpineRow::as_selected_node)
            .unwrap_or(SelectedNode::MissionControl)
    }

    pub fn reinit_browser(&mut self) {
        match self.selected_node() {
            SelectedNode::Space(entry) => {
                let path = entry.repo_path.clone();
                let mut browser = bella_engine::browser::Browser::new(path.clone());
                browser.root_boundary = Some(path);
                self.file_browser = browser;
                self.space_overview_scroll = 0;
                self.space_overview_file = None; // Reset the file view when changing spaces
            }
            SelectedNode::Hq => {
                // The brain repo's root ("."), collapsed from the old `brain` leaf.
                let path = std::path::PathBuf::from(".");
                let mut browser = bella_engine::browser::Browser::new(path.clone());
                browser.root_boundary = Some(path);
                self.file_browser = browser;
                self.space_overview_scroll = 0;
                self.space_overview_file = None;
            }
            SelectedNode::MissionControl | SelectedNode::Tier(_) => {}
        }
    }

    pub fn current_space_planning_root(&self) -> std::path::PathBuf {
        match self.selected_node() {
            SelectedNode::Space(entry) => entry.repo_path.join("planning"),
            SelectedNode::Hq => std::path::PathBuf::from(".").join("planning"),
            SelectedNode::MissionControl | SelectedNode::Tier(_) => {
                crate::config::load_planning_root()
            }
        }
    }

    pub fn compute_view(&self, area: Rect) -> (Rect, Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(30), Constraint::Min(0)])
            .split(area);
        (chunks[0], chunks[1])
    }

    /// Move selection to the next spine row, wrapping around. Every row (including
    /// Mission Control and tier headers) is selectable now, so there is no
    /// header-skip logic — unlike the old space-only navigation.
    pub fn select_next(&mut self) {
        let len = self.spine_rows().len();
        if len == 0 {
            return;
        }
        self.selected_spine = (self.selected_spine + 1) % len;
        self.reinit_browser();
    }

    /// Move selection to the previous spine row, wrapping around.
    pub fn select_prev(&mut self) {
        let len = self.spine_rows().len();
        if len == 0 {
            return;
        }
        self.selected_spine = if self.selected_spine == 0 {
            len - 1
        } else {
            self.selected_spine - 1
        };
        self.reinit_browser();
    }

    pub fn selected_session(&self) -> Option<&Session> {
        if let SelectedNode::Space(entry) = self.selected_node() {
            return self.sessions.iter().find(|s| s.name == entry.slug);
        }
        None
    }

    pub fn selected_space_slug(&self) -> Option<String> {
        if let SelectedNode::Space(entry) = self.selected_node() {
            return Some(entry.slug);
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
        if self.selected_node() == SelectedNode::MissionControl {
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
                // Space Overview renders for both `Hq` and `Space` nodes; Mission
                // Control and tier headers route elsewhere (ui.rs).
                let is_space_overview = matches!(
                    self.selected_node(),
                    SelectedNode::Hq | SelectedNode::Space(_)
                );
                self.status = Option::None;

                // Handle pane focus switching in SpaceOverview
                if is_space_overview {
                    match key {
                        KeyCode::Right => {
                            self.overview_pane = match self.overview_pane {
                                OverviewPane::Sidebar => OverviewPane::Browser,
                                OverviewPane::Browser => OverviewPane::Content,
                                OverviewPane::Content => OverviewPane::Content,
                            };
                            return Action::None;
                        }
                        KeyCode::Left => {
                            self.overview_pane = match self.overview_pane {
                                OverviewPane::Sidebar => OverviewPane::Sidebar,
                                OverviewPane::Browser => OverviewPane::Sidebar,
                                OverviewPane::Content => OverviewPane::Browser,
                            };
                            return Action::None;
                        }
                        _ => {}
                    }
                }

                if is_space_overview {
                    match self.overview_pane {
                        OverviewPane::Browser => {
                            match key {
                                KeyCode::Up | KeyCode::Char('k') => {
                                    self.file_browser.move_cursor(-1, 20);
                                    return Action::None;
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    self.file_browser.move_cursor(1, 20);
                                    return Action::None;
                                }
                                KeyCode::Enter => {
                                    if let Some(target) = self.file_browser.descend() {
                                        let mut b = bella_engine::browser::Browser::new(target);
                                        b.root_boundary = self.file_browser.root_boundary.clone();
                                        self.file_browser = b;
                                    } else if let Some(entry) = self.file_browser.selected_entry() {
                                        let is_md = entry.kind
                                            == bella_engine::browser::BrowserEntryKind::Markdown;
                                        if is_md {
                                            self.space_overview_file = Some(entry.path.clone());
                                            self.space_overview_scroll = 0;
                                        }
                                    }
                                    return Action::None;
                                }
                                KeyCode::Char('t') => {
                                    if let Some(entry) = self.file_browser.selected_entry() {
                                        let is_md = entry.kind
                                            == bella_engine::browser::BrowserEntryKind::Markdown;
                                        if is_md {
                                            // Transient full-screen overlay flag — replaces
                                            // the old tab-push. Overlay rendering/close-key
                                            // polish is deferred.
                                            self.markdown_overlay = Some(entry.path.clone());
                                        }
                                    }
                                    return Action::None;
                                }
                                KeyCode::Backspace => {
                                    if let Some(parent) = self.file_browser.ascend_target() {
                                        let mut b = bella_engine::browser::Browser::new(parent);
                                        b.root_boundary = self.file_browser.root_boundary.clone();
                                        self.file_browser = b;
                                    }
                                    return Action::None;
                                }
                                _ => {} // let it fall through? No, wait, if we are in Browser, we probably shouldn't fall through to global keys like 'q', etc unless we want to. Let's allow global keys for anything not explicitly handled.
                            }
                        }
                        OverviewPane::Content => match key {
                            KeyCode::PageUp => {
                                self.space_overview_scroll =
                                    self.space_overview_scroll.saturating_sub(10);
                                return Action::None;
                            }
                            KeyCode::PageDown => {
                                self.space_overview_scroll =
                                    self.space_overview_scroll.saturating_add(10);
                                return Action::None;
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                self.space_overview_scroll =
                                    self.space_overview_scroll.saturating_sub(1);
                                return Action::None;
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                self.space_overview_scroll =
                                    self.space_overview_scroll.saturating_add(1);
                                return Action::None;
                            }
                            _ => {}
                        },
                        OverviewPane::Sidebar => {
                            // Sidebar uses global Up/Down/j to navigate spaces
                        }
                    }
                }

                match key {
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.select_next();
                        if self.selected_node() == SelectedNode::MissionControl {
                            self.monitor_app.next_item();
                        }
                        Action::None
                    }
                    KeyCode::Up => {
                        self.select_prev();
                        if self.selected_node() == SelectedNode::MissionControl {
                            self.monitor_app.prev_item();
                        }
                        Action::None
                    }
                    KeyCode::Char('a') => {
                        if let Some(s) = self.selected_session_for_actions() {
                            Action::Attach(s.name.clone())
                        } else if let Some(slug) = self.selected_space_slug() {
                            Action::Attach(slug)
                        } else {
                            Action::None
                        }
                    }
                    KeyCode::Char('n') => {
                        self.input.clear();
                        self.mode = Mode::Input(InputKind::New);
                        Action::None
                    }
                    KeyCode::Char('s') => {
                        if let Some(s) = self.selected_session_for_actions() {
                            let _name = s.name.clone();
                            self.input.clear();
                            self.mode = Mode::Input(InputKind::Send);
                        } else if self.selected_space_slug().is_some() {
                            self.input.clear();
                            self.mode = Mode::Input(InputKind::Send);
                        } else {
                            self.status = Some("no session selected".into());
                        }
                        Action::None
                    }
                    KeyCode::Char('k') => {
                        // Global 'k' is Kill, but if in SpaceOverview Content/Browser, it was already handled above.
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

    // ── Mouse→action mapping (BA.13.2) ────────────────────────────────────────

    /// Map a mouse event to an `Action`, routing purely off `self.pane_areas`
    /// (captured by the most recent draw — see `compute_pane_areas`) via
    /// `bella_engine::geometry::point_in`. No `Frame`/terminal involved, so
    /// this is exhaustively unit-testable with synthetic `PaneAreas`.
    ///
    /// Kept as a single flat match-per-pane (click dispatch delegates to
    /// `handle_click`, scroll dispatch to `handle_scroll`) so BA.13.4 can add
    /// a `subtab` arm later without a rewrite. Anything not a left-click or a
    /// wheel event (moves, drags, other buttons) is a no-op.
    pub fn on_mouse(&mut self, ev: MouseEvent) -> Action {
        match ev.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.handle_click(ev.column, ev.row);
            }
            MouseEventKind::ScrollUp => self.handle_scroll(ev.column, ev.row, true),
            MouseEventKind::ScrollDown => self.handle_scroll(ev.column, ev.row, false),
            _ => {}
        }
        Action::None
    }

    /// Route a left-click by hit-testing `self.pane_areas` in a fixed order
    /// (spine → browser → agent panel → content). Panes never overlap in the
    /// real layout, so ordering only matters for the zero-sized default case
    /// before the first draw, where every `point_in` check fails and the
    /// click is correctly a no-op.
    fn handle_click(&mut self, col: u16, row: u16) {
        if bella_engine::geometry::point_in(self.pane_areas.spine, col, row) {
            let len = self.spine_rows().len();
            // No independent scroll offset is tracked for the spine `List`
            // (its `ListState` is local to the draw loop, not stored on
            // `AppState`), so row-mapping assumes an unscrolled viewport —
            // matching ratatui's behavior whenever the spine fits on screen.
            if let Some(idx) = row_index_in_pane(self.pane_areas.spine, row, 0, len) {
                self.selected_spine = idx;
                self.reinit_browser();
            }
        } else if bella_engine::geometry::point_in(self.pane_areas.browser, col, row) {
            let len = self.file_browser.entries.len();
            let scroll = self.file_browser.scroll as usize;
            if let Some(idx) = row_index_in_pane(self.pane_areas.browser, row, scroll, len) {
                self.file_browser.selected = idx;
                self.overview_pane = OverviewPane::Browser;
            }
        } else if bella_engine::geometry::point_in(self.pane_areas.agent_panel, col, row) {
            let rows = agent_panel_rows(&self.sessions);
            if let Some(idx) = row_index_in_pane(self.pane_areas.agent_panel, row, 0, rows.len()) {
                let name = rows[idx].label.clone();
                let target = self
                    .spine_rows()
                    .iter()
                    .position(|r| matches!(r, SpineRow::Space(entry) if entry.slug == name));
                if let Some(pos) = target {
                    self.selected_spine = pos;
                    self.reinit_browser();
                }
                // No matching space for this session name: no-op (v1 slug-equality
                // rule; see spec Context Pointers).
            }
        } else if bella_engine::geometry::point_in(self.pane_areas.content, col, row) {
            self.overview_pane = OverviewPane::Content;
        }
        // Click outside every stored pane (including all-zero default areas
        // before the first draw): no-op.
    }

    /// Route a wheel event by which pane the pointer is hovering over.
    /// Content scrolls `space_overview_scroll`; browser moves the file-browser
    /// cursor; spine moves the primary-navigation selection. Anywhere else is
    /// a no-op.
    fn handle_scroll(&mut self, col: u16, row: u16, up: bool) {
        if bella_engine::geometry::point_in(self.pane_areas.content, col, row) {
            self.space_overview_scroll = if up {
                self.space_overview_scroll.saturating_sub(1)
            } else {
                self.space_overview_scroll.saturating_add(1)
            };
        } else if bella_engine::geometry::point_in(self.pane_areas.browser, col, row) {
            // Two rows of border consumed top+bottom of the browser block.
            let viewport_h = self.pane_areas.browser.height.saturating_sub(2);
            self.file_browser
                .move_cursor(if up { -1 } else { 1 }, viewport_h);
        } else if bella_engine::geometry::point_in(self.pane_areas.spine, col, row) {
            if up {
                self.select_prev();
            } else {
                self.select_next();
            }
        }
    }
}

/// Map a click row to an in-list index, accounting for the enclosing block's
/// top/bottom border (one row each) and a vertical `scroll` offset (index of
/// the first visible entry). Returns `None` when the row lands on a border,
/// the pane has no content rows (height <= 2), or the mapped index is out of
/// bounds for a list of `len` entries — covering both out-of-range clicks and
/// the "nothing to click" empty-list case.
fn row_index_in_pane(area: Rect, row: u16, scroll: usize, len: usize) -> Option<usize> {
    if len == 0 || area.height <= 2 {
        return None;
    }
    let top = area.y + 1; // skip the top border row
    let bottom = area.y + area.height - 1; // one-past-last content row (bottom border starts here)
    if row < top || row >= bottom {
        return None;
    }
    let local = (row - top) as usize;
    let idx = scroll + local;
    if idx >= len {
        return None;
    }
    Some(idx)
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

    /// A truly empty spine: no tiers at all, so `spine_rows()` is just
    /// `[MissionControl]` (len 1).
    fn make_empty_app() -> AppState {
        AppState::new(vec![], SpaceTree::default())
    }

    /// A spine covering every row kind: Mission Control, Hq (+ a collapsed
    /// `brain` leaf + a `learn-ai` child), and a `core` tier with one space.
    /// `spine_rows()` == `[MC, Hq, Space(learn-ai), Tier(core), Space(bastion)]`.
    fn make_full_app() -> AppState {
        let tree = SpaceTree {
            tiers: vec![
                (
                    "_root".to_string(),
                    vec![
                        crate::brain::spaces::SpaceEntry {
                            slug: "brain".to_string(),
                            tier: "_root".to_string(),
                            repo_path: std::path::PathBuf::from("."),
                            heading: None,
                        },
                        crate::brain::spaces::SpaceEntry {
                            slug: "learn-ai".to_string(),
                            tier: "_root".to_string(),
                            repo_path: std::path::PathBuf::from("learn-ai"),
                            heading: None,
                        },
                    ],
                ),
                (
                    "core".to_string(),
                    vec![crate::brain::spaces::SpaceEntry {
                        slug: "bastion".to_string(),
                        tier: "core".to_string(),
                        repo_path: std::path::PathBuf::from("core/bastion"),
                        heading: None,
                    }],
                ),
            ],
        };
        AppState::new(vec![], tree)
    }

    // ── Constructor ───────────────────────────────────────────────────────────

    #[test]
    fn new_starts_at_mission_control_normal_mode() {
        let app = make_app(&make_sessions(&["alpha", "beta"]));
        assert_eq!(app.selected_spine, 0);
        assert_eq!(app.selected_node(), SelectedNode::MissionControl);
        assert_eq!(app.mode, Mode::Normal);
        assert!(!app.should_quit);
        assert!(app.status.is_none());
    }

    // ── Navigation ────────────────────────────────────────────────────────────

    #[test]
    fn select_next_wraps_at_end_of_spine() {
        // spine = [MC, Tier(core), a, b, c] — len 5.
        let mut app = make_app(&make_sessions(&["a", "b", "c"]));
        app.selected_spine = 4;
        app.select_next();
        assert_eq!(app.selected_spine, 0);
    }

    #[test]
    fn select_prev_wraps_at_start_of_spine() {
        let mut app = make_app(&make_sessions(&["a", "b", "c"]));
        app.selected_spine = 0;
        app.select_prev();
        assert_eq!(app.selected_spine, 4);
    }

    #[test]
    fn select_next_visits_every_row_including_headers() {
        // spine = [MC, Tier(core), a, b] — len 4. Mission Control and the tier
        // header are now selectable, unlike the old header-skip logic.
        let mut app = make_app(&make_sessions(&["a", "b"]));
        let mut seen = vec![app.selected_spine];
        for _ in 0..4 {
            app.select_next();
            seen.push(app.selected_spine);
        }
        assert_eq!(seen, vec![0, 1, 2, 3, 0]);
    }

    #[test]
    fn select_next_on_single_row_spine_is_noop() {
        let mut app = make_empty_app();
        app.select_next();
        assert_eq!(app.selected_spine, 0);
    }

    #[test]
    fn select_prev_on_single_row_spine_is_noop() {
        let mut app = make_empty_app();
        app.select_prev();
        assert_eq!(app.selected_spine, 0);
    }

    // ── selected_node ────────────────────────────────────────────────────────

    #[test]
    fn selected_node_maps_every_row_kind() {
        // spine = [MC, Hq, Space(learn-ai), Tier(core), Space(bastion)].
        let mut app = make_full_app();
        assert_eq!(app.selected_node(), SelectedNode::MissionControl);

        app.selected_spine = 1;
        assert_eq!(app.selected_node(), SelectedNode::Hq);

        app.selected_spine = 3;
        assert_eq!(app.selected_node(), SelectedNode::Tier("core".to_string()));

        app.selected_spine = 4;
        match app.selected_node() {
            SelectedNode::Space(entry) => assert_eq!(entry.slug, "bastion"),
            other => panic!("expected Space(bastion), got {other:?}"),
        }
    }

    #[test]
    fn selected_node_out_of_range_falls_back_to_mission_control() {
        let mut app = make_empty_app();
        app.selected_spine = 99;
        assert_eq!(app.selected_node(), SelectedNode::MissionControl);
    }

    // ── set_sessions ─────────────────────────────────────────────────────────

    #[test]
    fn set_sessions_does_not_change_selected_spine() {
        let mut app = make_app(&make_sessions(&["a", "b", "c"]));
        app.selected_spine = 2;
        app.set_sessions(make_sessions(&["x"]));
        assert_eq!(app.selected_spine, 2);
    }

    #[test]
    fn set_sessions_empty_does_not_change_selected_spine() {
        let mut app = make_app(&make_sessions(&["a", "b"]));
        app.selected_spine = 1;
        app.set_sessions(vec![]);
        assert_eq!(app.selected_spine, 1);
    }

    // ── selected_session ─────────────────────────────────────────────────────

    #[test]
    fn selected_session_returns_none_when_not_on_a_space_row() {
        let app = make_empty_app();
        assert!(app.selected_session().is_none());
    }

    #[test]
    fn selected_session_returns_session_when_on_a_space_row() {
        // spine = [MC, Tier(core), alpha] — len 3.
        let mut app = make_app(&make_sessions(&["alpha"]));
        app.selected_spine = 2;
        app.reinit_browser();
        assert_eq!(
            app.selected_session().map(|s| s.name.as_str()),
            Some("alpha")
        );
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
    fn on_key_j_and_down_advance_through_headers_too() {
        // spine = [MC, Tier(core), a, b, c] — len 5, starting at MC (index 0).
        let mut app = make_app(&make_sessions(&["a", "b", "c"]));
        let a1 = app.on_key(KeyCode::Char('j'));
        assert_eq!(a1, Action::None);
        assert_eq!(app.selected_spine, 1); // Tier(core) header, now selectable
        let a2 = app.on_key(KeyCode::Down);
        assert_eq!(a2, Action::None);
        assert_eq!(app.selected_spine, 2); // first space row
    }

    #[test]
    fn on_key_up_retreats() {
        let mut app = make_app(&make_sessions(&["a", "b", "c"]));
        app.selected_spine = 3;
        app.reinit_browser();
        let a = app.on_key(KeyCode::Up);
        assert_eq!(a, Action::None);
        assert_eq!(app.selected_spine, 2);
    }

    // ── on_key: Normal mode actions ───────────────────────────────────────────

    #[test]
    fn on_key_a_returns_attach_with_selected_name() {
        // spine = [MC, Tier(core), Space(my-session)] — len 3.
        let mut app = make_app(&make_sessions(&["my-session"]));
        app.selected_spine = 2;
        app.reinit_browser();
        let action = app.on_key(KeyCode::Char('a'));
        assert_eq!(action, Action::Attach("my-session".into()));
    }

    #[test]
    fn on_key_a_on_mission_control_with_no_item_is_none() {
        let mut app = make_empty_app();
        let action = app.on_key(KeyCode::Char('a'));
        assert_eq!(action, Action::None);
    }

    #[test]
    fn on_key_n_enters_new_input_mode() {
        let mut app = make_empty_app();
        let action = app.on_key(KeyCode::Char('n'));
        assert_eq!(action, Action::None);
        assert_eq!(app.mode, Mode::Input(InputKind::New));
    }

    #[test]
    fn on_key_s_enters_send_input_mode_when_selected() {
        let mut app = make_app(&make_sessions(&["alpha"]));
        app.selected_spine = 2;
        app.reinit_browser();
        let action = app.on_key(KeyCode::Char('s'));
        assert_eq!(action, Action::None);
        assert_eq!(app.mode, Mode::Input(InputKind::Send));
    }

    #[test]
    fn on_key_s_no_selection_sets_status() {
        let mut app = make_empty_app();
        let action = app.on_key(KeyCode::Char('s'));
        assert_eq!(action, Action::None);
        assert!(app.status.is_some());
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn on_key_k_returns_kill_with_selected_name() {
        let mut app = make_app(&make_sessions(&["victim"]));
        app.selected_spine = 2;
        app.reinit_browser();
        let action = app.on_key(KeyCode::Char('k'));
        assert_eq!(action, Action::Kill("victim".into()));
    }

    #[test]
    fn on_key_q_sets_should_quit() {
        let mut app = make_empty_app();
        let action = app.on_key(KeyCode::Char('q'));
        assert_eq!(action, Action::None);
        assert!(app.should_quit);
    }

    #[test]
    fn t_key_sets_markdown_overlay_on_markdown_entry() {
        // Land on a Space row so `is_space_overview` gates the browser keys.
        let mut app = make_app(&make_sessions(&["alpha"]));
        app.selected_spine = 2;
        app.reinit_browser();
        app.overview_pane = OverviewPane::Browser;
        app.file_browser.entries = vec![bella_engine::browser::BrowserEntry {
            path: std::path::PathBuf::from("alpha/README.md"),
            display: "README.md".to_string(),
            kind: bella_engine::browser::BrowserEntryKind::Markdown,
        }];
        app.file_browser.selected = 0;
        assert!(app.markdown_overlay.is_none());

        let action = app.on_key(KeyCode::Char('t'));
        assert_eq!(action, Action::None);
        assert_eq!(
            app.markdown_overlay,
            Some(std::path::PathBuf::from("alpha/README.md"))
        );
    }

    // ── on_key: Input mode ────────────────────────────────────────────────────

    #[test]
    fn input_mode_char_appends() {
        let mut app = make_empty_app();
        app.mode = Mode::Input(InputKind::New);
        app.on_key(KeyCode::Char('a'));
        app.on_key(KeyCode::Char('b'));
        assert_eq!(app.input, "ab");
    }

    #[test]
    fn input_mode_backspace_pops() {
        let mut app = make_empty_app();
        app.mode = Mode::Input(InputKind::New);
        app.input = "abc".into();
        app.on_key(KeyCode::Backspace);
        assert_eq!(app.input, "ab");
    }

    #[test]
    fn input_mode_esc_cancels_to_normal() {
        let mut app = make_empty_app();
        app.mode = Mode::Input(InputKind::New);
        app.input = "partial".into();
        let action = app.on_key(KeyCode::Esc);
        assert_eq!(action, Action::None);
        assert_eq!(app.mode, Mode::Normal);
        assert!(app.input.is_empty());
    }

    #[test]
    fn input_mode_enter_new_returns_new_action() {
        let mut app = make_empty_app();
        app.mode = Mode::Input(InputKind::New);
        app.input = "fresh".into();
        let action = app.on_key(KeyCode::Enter);
        assert_eq!(action, Action::New("fresh".into()));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn input_mode_enter_new_empty_sets_status() {
        let mut app = make_empty_app();
        app.mode = Mode::Input(InputKind::New);
        let action = app.on_key(KeyCode::Enter);
        assert_eq!(action, Action::None);
        assert!(app.status.is_some());
    }

    #[test]
    fn input_mode_enter_send_returns_send_action() {
        let mut app = make_app(&make_sessions(&["target"]));
        app.selected_spine = 2;
        app.reinit_browser();
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
        let mut app = make_empty_app();
        app.mode = Mode::Input(InputKind::Send);
        app.input = "whatever".into();
        let action = app.on_key(KeyCode::Enter);
        assert_eq!(action, Action::None);
    }

    #[test]
    fn compute_view_math() {
        let app = make_empty_app();
        let area = Rect::new(0, 0, 100, 50);
        let (sidebar, main) = app.compute_view(area);
        assert_eq!(sidebar.width, 30);
        assert_eq!(sidebar.height, 50);
        assert_eq!(main.width, 70);
        assert_eq!(main.height, 50);
    }

    // ── compute_pane_areas (pure geometry, BA.13.2 task 1) ─────────────────────

    #[test]
    fn compute_pane_areas_mission_control_zeros_browser() {
        // 80x24 frame, a typical agent-strip height (7 rows, the max).
        let frame = Rect::new(0, 0, 80, 24);
        let areas = compute_pane_areas(frame, 7, &SelectedNode::MissionControl);

        // Outer vertical split: Min(1) main + Length(7) strip + Length(1) footer
        // over height 24 -> main gets 24 - 7 - 1 = 16.
        assert_eq!(areas.agent_panel, Rect::new(0, 16, 80, 7));

        // Horizontal split of the 16-tall main area: Length(30) spine + Min(0) main.
        assert_eq!(areas.spine, Rect::new(0, 0, 30, 16));

        // Mission Control has no browser/content split — the whole remaining
        // main area is content, and browser is zero-sized so `point_in` never
        // matches it.
        assert_eq!(areas.content, Rect::new(30, 0, 50, 16));
        assert_eq!(areas.browser, Rect::default());
    }

    #[test]
    fn compute_pane_areas_tier_zeros_browser() {
        let frame = Rect::new(0, 0, 80, 24);
        let areas = compute_pane_areas(frame, 7, &SelectedNode::Tier("core".to_string()));

        assert_eq!(areas.content, Rect::new(30, 0, 50, 16));
        assert_eq!(areas.browser, Rect::default());
    }

    #[test]
    fn compute_pane_areas_hq_splits_browser_and_content() {
        let frame = Rect::new(0, 0, 80, 24);
        let areas = compute_pane_areas(frame, 7, &SelectedNode::Hq);

        assert_eq!(areas.spine, Rect::new(0, 0, 30, 16));
        // overview_chunks: Length(30) browser + Min(0) content, split from the
        // 50-wide main area (which starts at x=30).
        assert_eq!(areas.browser, Rect::new(30, 0, 30, 16));
        assert_eq!(areas.content, Rect::new(60, 0, 20, 16));
    }

    #[test]
    fn compute_pane_areas_space_splits_browser_and_content_same_as_hq() {
        let frame = Rect::new(0, 0, 80, 24);
        let entry = crate::brain::spaces::SpaceEntry {
            slug: "learn-ai".to_string(),
            tier: "_root".to_string(),
            repo_path: std::path::PathBuf::from("learn-ai"),
            heading: None,
        };
        let areas = compute_pane_areas(frame, 7, &SelectedNode::Space(entry));

        assert_eq!(areas.browser, Rect::new(30, 0, 30, 16));
        assert_eq!(areas.content, Rect::new(60, 0, 20, 16));
    }

    #[test]
    fn compute_pane_areas_tiny_frame_agent_strip_collapses_to_zero() {
        // A tiny frame where the caller (mirroring `agent_panel_strip_height`'s
        // own min-height fallback) has already collapsed the strip height to 0.
        let frame = Rect::new(0, 0, 20, 5);
        let areas = compute_pane_areas(frame, 0, &SelectedNode::MissionControl);

        assert_eq!(areas.agent_panel.height, 0);
        // Main area still gets height - 0 - 1 (footer) = 4.
        assert_eq!(areas.spine.height, 4);
        assert_eq!(areas.content.height, 4);
    }

    #[test]
    fn compute_pane_areas_default_is_all_zero() {
        assert_eq!(
            PaneAreas::default(),
            PaneAreas {
                spine: Rect::default(),
                browser: Rect::default(),
                content: Rect::default(),
                agent_panel: Rect::default(),
            }
        );
    }

    // ── on_mouse (BA.13.2 task 2) ───────────────────────────────────────────────

    fn mouse_event(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: crossterm::event::KeyModifiers::empty(),
        }
    }

    fn left_click(column: u16, row: u16) -> MouseEvent {
        mouse_event(MouseEventKind::Down(MouseButton::Left), column, row)
    }

    /// A full spine (`[MC, Hq, Space(learn-ai), Tier(core), Space(bastion)]`,
    /// len 5) with `pane_areas` set as if a draw had already run: an 80x24
    /// frame, spine 30 wide down the left, agent panel at the bottom, and (on
    /// an `Hq`/`Space` row) a 30-wide browser + remaining content.
    fn make_full_app_with_panes() -> AppState {
        let mut app = make_full_app();
        app.pane_areas = compute_pane_areas(Rect::new(0, 0, 80, 24), 7, &SelectedNode::Hq);
        app
    }

    // -- row_index_in_pane -------------------------------------------------------

    #[test]
    fn row_index_in_pane_skips_top_and_bottom_border() {
        let area = Rect::new(0, 0, 30, 5); // 3 content rows: y=1,2,3
        assert_eq!(row_index_in_pane(area, 0, 0, 3), None); // top border
        assert_eq!(row_index_in_pane(area, 4, 0, 3), None); // bottom border
        assert_eq!(row_index_in_pane(area, 1, 0, 3), Some(0));
        assert_eq!(row_index_in_pane(area, 3, 0, 3), Some(2));
    }

    #[test]
    fn row_index_in_pane_applies_scroll_offset() {
        let area = Rect::new(0, 0, 30, 5);
        assert_eq!(row_index_in_pane(area, 1, 10, 20), Some(10));
        assert_eq!(row_index_in_pane(area, 3, 10, 20), Some(12));
    }

    #[test]
    fn row_index_in_pane_out_of_bounds_clamped_to_none() {
        let area = Rect::new(0, 0, 30, 5);
        // 3 visible rows starting at scroll=8 over a 10-entry list: only
        // indices 8/9 exist, so the third visible row (would-be idx 10) is None.
        assert_eq!(row_index_in_pane(area, 3, 8, 10), None);
    }

    #[test]
    fn row_index_in_pane_empty_list_is_none() {
        let area = Rect::new(0, 0, 30, 5);
        assert_eq!(row_index_in_pane(area, 1, 0, 0), None);
    }

    #[test]
    fn row_index_in_pane_too_short_for_content_is_none() {
        let area = Rect::new(0, 0, 30, 2); // just the two border rows, no content
        assert_eq!(row_index_in_pane(area, 0, 0, 5), None);
        assert_eq!(row_index_in_pane(area, 1, 0, 5), None);
    }

    // -- click: spine -------------------------------------------------------------

    #[test]
    fn click_in_spine_selects_row() {
        let mut app = make_full_app_with_panes();
        // spine area is Rect::new(0, 0, 30, 16); row 1 is the first content row.
        let action = app.on_mouse(left_click(5, 1));
        assert_eq!(action, Action::None);
        assert_eq!(app.selected_spine, 0);

        let action = app.on_mouse(left_click(5, 3));
        assert_eq!(action, Action::None);
        assert_eq!(app.selected_spine, 2);
    }

    #[test]
    fn click_in_spine_out_of_range_row_is_noop() {
        let mut app = make_full_app_with_panes();
        app.selected_spine = 1;
        // spine has only 5 rows (content rows y=1..=5); row 12 is well past the
        // last entry but still inside the bordered pane.
        app.on_mouse(left_click(5, 12));
        assert_eq!(app.selected_spine, 1);
    }

    #[test]
    fn click_on_spine_border_is_noop() {
        let mut app = make_full_app_with_panes();
        app.selected_spine = 1;
        app.on_mouse(left_click(5, 0)); // top border row of the spine block
        assert_eq!(app.selected_spine, 1);
    }

    // -- click: browser -------------------------------------------------------------

    #[test]
    fn click_in_browser_selects_entry_and_focuses_browser_pane() {
        let mut app = make_full_app_with_panes();
        app.selected_spine = 1; // Hq row -> browser/content split
        app.reinit_browser();
        app.file_browser.entries = vec![
            bella_engine::browser::BrowserEntry {
                path: std::path::PathBuf::from("a"),
                display: "a".into(),
                kind: bella_engine::browser::BrowserEntryKind::Dir,
            },
            bella_engine::browser::BrowserEntry {
                path: std::path::PathBuf::from("b.md"),
                display: "b.md".into(),
                kind: bella_engine::browser::BrowserEntryKind::Markdown,
            },
        ];
        app.file_browser.selected = 0;
        app.file_browser.scroll = 0;
        app.overview_pane = OverviewPane::Sidebar;

        // browser area is Rect::new(30, 0, 30, 16); row 2 -> content row index 1.
        let action = app.on_mouse(left_click(35, 2));
        assert_eq!(action, Action::None);
        assert_eq!(app.file_browser.selected, 1);
        assert_eq!(app.overview_pane, OverviewPane::Browser);
    }

    #[test]
    fn click_in_browser_accounts_for_scroll_offset() {
        let mut app = make_full_app_with_panes();
        app.selected_spine = 1;
        app.reinit_browser();
        app.file_browser.entries = (0..10)
            .map(|i| bella_engine::browser::BrowserEntry {
                path: std::path::PathBuf::from(format!("f{i}.md")),
                display: format!("f{i}.md"),
                kind: bella_engine::browser::BrowserEntryKind::Markdown,
            })
            .collect();
        app.file_browser.selected = 5;
        app.file_browser.scroll = 5;

        // row 1 is the first content row; with scroll=5, that's entry index 5.
        app.on_mouse(left_click(35, 1));
        assert_eq!(app.file_browser.selected, 5);

        // row 3 -> local row 2 -> entry index 5 + 2 = 7.
        app.on_mouse(left_click(35, 3));
        assert_eq!(app.file_browser.selected, 7);
    }

    #[test]
    fn click_in_browser_out_of_range_row_is_noop() {
        let mut app = make_full_app_with_panes();
        app.selected_spine = 1;
        app.reinit_browser();
        app.file_browser.entries = vec![bella_engine::browser::BrowserEntry {
            path: std::path::PathBuf::from("a"),
            display: "a".into(),
            kind: bella_engine::browser::BrowserEntryKind::Dir,
        }];
        app.file_browser.selected = 0;
        app.on_mouse(left_click(35, 10)); // well past the single entry
        assert_eq!(app.file_browser.selected, 0);
    }

    // -- click: agent panel -------------------------------------------------------------

    #[test]
    fn click_in_agent_panel_selects_matching_space_by_slug() {
        let mut app = make_full_app_with_panes();
        app.sessions = make_sessions(&["bastion"]);
        app.pane_areas.agent_panel = Rect::new(0, 16, 80, 7);
        app.selected_spine = 0;

        // agent_panel_rows sorts by urgency; with one Idle session it's row 0
        // -> content row at y = 17 (top border at y=16).
        let action = app.on_mouse(left_click(5, 17));
        assert_eq!(action, Action::None);
        match app.selected_node() {
            SelectedNode::Space(entry) => assert_eq!(entry.slug, "bastion"),
            other => panic!("expected Space(bastion), got {other:?}"),
        }
    }

    #[test]
    fn click_in_agent_panel_no_matching_space_is_noop() {
        let mut app = make_full_app_with_panes();
        app.sessions = make_sessions(&["no-such-space"]);
        app.pane_areas.agent_panel = Rect::new(0, 16, 80, 7);
        app.selected_spine = 0;

        app.on_mouse(left_click(5, 17));
        assert_eq!(app.selected_spine, 0);
    }

    // -- click: content -------------------------------------------------------------

    #[test]
    fn click_in_content_focuses_content_pane() {
        let mut app = make_full_app_with_panes();
        app.selected_spine = 1;
        app.reinit_browser();
        app.overview_pane = OverviewPane::Sidebar;

        // content area is Rect::new(60, 0, 20, 16).
        let action = app.on_mouse(left_click(65, 2));
        assert_eq!(action, Action::None);
        assert_eq!(app.overview_pane, OverviewPane::Content);
    }

    // -- click: outside every pane / before first draw -------------------------------

    #[test]
    fn click_outside_every_pane_is_noop() {
        let mut app = make_full_app_with_panes();
        app.selected_spine = 1;
        let before = app.selected_spine;
        // Row 23 falls in the 1-line footer, outside every stored pane.
        let action = app.on_mouse(left_click(5, 23));
        assert_eq!(action, Action::None);
        assert_eq!(app.selected_spine, before);
    }

    #[test]
    fn click_before_first_draw_default_areas_is_noop() {
        let mut app = make_full_app();
        assert_eq!(app.pane_areas, PaneAreas::default());
        let action = app.on_mouse(left_click(5, 5));
        assert_eq!(action, Action::None);
        assert_eq!(app.selected_spine, 0);
        assert_eq!(app.overview_pane, OverviewPane::Sidebar);
    }

    // -- scroll -------------------------------------------------------------

    #[test]
    fn scroll_over_content_adjusts_overview_scroll_saturating() {
        let mut app = make_full_app_with_panes();
        app.selected_spine = 1;
        app.reinit_browser();
        app.space_overview_scroll = 5;

        app.on_mouse(mouse_event(MouseEventKind::ScrollDown, 65, 2));
        assert_eq!(app.space_overview_scroll, 6);

        app.on_mouse(mouse_event(MouseEventKind::ScrollUp, 65, 2));
        assert_eq!(app.space_overview_scroll, 5);

        app.space_overview_scroll = 0;
        app.on_mouse(mouse_event(MouseEventKind::ScrollUp, 65, 2));
        assert_eq!(app.space_overview_scroll, 0); // saturating, not underflowing
    }

    #[test]
    fn scroll_over_browser_moves_cursor() {
        let mut app = make_full_app_with_panes();
        app.selected_spine = 1;
        app.reinit_browser();
        app.file_browser.entries = (0..5)
            .map(|i| bella_engine::browser::BrowserEntry {
                path: std::path::PathBuf::from(format!("f{i}.md")),
                display: format!("f{i}.md"),
                kind: bella_engine::browser::BrowserEntryKind::Markdown,
            })
            .collect();
        app.file_browser.selected = 2;

        app.on_mouse(mouse_event(MouseEventKind::ScrollDown, 35, 2));
        assert_eq!(app.file_browser.selected, 3);

        app.on_mouse(mouse_event(MouseEventKind::ScrollUp, 35, 2));
        assert_eq!(app.file_browser.selected, 2);
    }

    #[test]
    fn scroll_over_spine_moves_selection() {
        let mut app = make_full_app_with_panes();
        app.selected_spine = 1;

        app.on_mouse(mouse_event(MouseEventKind::ScrollDown, 5, 5));
        assert_eq!(app.selected_spine, 2);

        app.on_mouse(mouse_event(MouseEventKind::ScrollUp, 5, 5));
        assert_eq!(app.selected_spine, 1);
    }

    #[test]
    fn scroll_outside_every_pane_is_noop() {
        let mut app = make_full_app_with_panes();
        app.selected_spine = 1;
        app.on_mouse(mouse_event(MouseEventKind::ScrollDown, 5, 23)); // footer row
        assert_eq!(app.selected_spine, 1);
    }

    // -- non-click/scroll events -------------------------------------------------------------

    #[test]
    fn mouse_move_event_is_noop() {
        let mut app = make_full_app_with_panes();
        app.selected_spine = 1;
        let action = app.on_mouse(mouse_event(MouseEventKind::Moved, 5, 1));
        assert_eq!(action, Action::None);
        assert_eq!(app.selected_spine, 1);
    }

    #[test]
    fn right_click_is_noop() {
        let mut app = make_full_app_with_panes();
        app.selected_spine = 1;
        let action = app.on_mouse(mouse_event(MouseEventKind::Down(MouseButton::Right), 5, 1));
        assert_eq!(action, Action::None);
        assert_eq!(app.selected_spine, 1);
    }
}
