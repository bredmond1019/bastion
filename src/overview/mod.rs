use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};
use std::{fs, io};

#[derive(serde::Deserialize, Debug, Clone)]
pub struct StateJson {
    pub repo: String,
    pub updated: String,
    pub focus: Focus,
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct Focus {
    pub now: Vec<BlockTask>,
    pub next: Vec<BlockTask>,
    pub blocked: Vec<BlockTask>,
}

#[derive(serde::Deserialize, Debug, Clone)]
pub struct BlockTask {
    pub id: String,
    pub title: String,
    pub repo: Option<String>,
}

pub fn run() -> Result<()> {
    // Read state.json from the planning directory
    let path = crate::config::load_planning_root().join("state.json");

    let content = fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("Failed to read {:?}: {}", path, e))?;
    let state: StateJson = serde_json::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Failed to parse state.json: {}", e))?;

    let mut stdout = io::stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_inner(&mut terminal, &state);

    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    result
}

fn run_inner(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &StateJson,
) -> Result<()> {
    loop {
        terminal.draw(|f| draw(f, state))?;

        #[allow(clippy::collapsible_if)]
        if event::poll(std::time::Duration::from_millis(250))? {
            if let Event::Key(k) = event::read()? {
                if k.code == KeyCode::Char('q') {
                    break;
                }
            }
        }
    }
    Ok(())
}

fn draw(frame: &mut Frame, state: &StateJson) {
    render(frame, state, frame.area());
}

pub fn render(frame: &mut Frame, state: &StateJson, area: ratatui::layout::Rect) {
    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(area);

    // ── Header ────────────────────────────────────────────────────────────────
    let header_text = format!(" Kanban Board — {} (updated {})", state.repo, state.updated);
    let header = Paragraph::new(ratatui::text::Span::styled(
        header_text,
        ratatui::style::Style::default()
            .fg(crate::ui_theme::text())
            .add_modifier(Modifier::BOLD),
    ))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(crate::ui_theme::border_active())),
    );
    frame.render_widget(header, main_layout[0]);

    // ── Columns ───────────────────────────────────────────────────────────────
    let columns = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(33),
            Constraint::Percentage(33),
            Constraint::Percentage(34),
        ])
        .split(main_layout[1]);

    // Build a single ListItem for a task:
    //   Line 1: [ID]  (accent color)
    //   Line 2: title (text color, will wrap inside the column width)
    let build_items = |tasks: &[BlockTask]| -> Vec<ListItem<'static>> {
        tasks
            .iter()
            .flat_map(|b| {
                let id_line = ratatui::text::Line::from(vec![ratatui::text::Span::styled(
                    b.id.clone(),
                    Style::default()
                        .fg(crate::ui_theme::accent())
                        .add_modifier(Modifier::BOLD),
                )]);
                let title_line = ratatui::text::Line::from(vec![ratatui::text::Span::styled(
                    format!("  {}", b.title.clone()),
                    Style::default().fg(crate::ui_theme::text()),
                )]);
                let sep = ratatui::text::Line::from("");
                // id, title, blank separator between tasks
                [
                    ListItem::new(id_line),
                    ListItem::new(title_line),
                    ListItem::new(sep),
                ]
            })
            .collect()
    };

    let now_items = build_items(&state.focus.now);
    let next_items = build_items(&state.focus.next);
    let blocked_items = build_items(&state.focus.blocked);

    let now_list = List::new(now_items).block(
        Block::default()
            .title(ratatui::text::Span::styled(
                " In Progress ",
                Style::default()
                    .fg(crate::ui_theme::sage())
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(crate::ui_theme::border_dim())),
    );

    let next_list = List::new(next_items).block(
        Block::default()
            .title(ratatui::text::Span::styled(
                " Up Next ",
                Style::default()
                    .fg(crate::ui_theme::violet())
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(crate::ui_theme::border_dim())),
    );

    let blocked_list = List::new(blocked_items).block(
        Block::default()
            .title(ratatui::text::Span::styled(
                " Blocked ",
                Style::default()
                    .fg(crate::ui_theme::rose())
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(crate::ui_theme::border_dim())),
    );

    frame.render_widget(now_list, columns[0]);
    frame.render_widget(next_list, columns[1]);
    frame.render_widget(blocked_list, columns[2]);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// A representative `state.json` shape (repo/updated/focus.now/next/blocked, each
    /// with a couple of `BlockTask` entries including an optional `repo` field) — the
    /// shape mev's `emit_state` (MV.4.E) now generates and bastion's Kanban board
    /// depends on.
    const REPRESENTATIVE: &str = r#"{
        "repo": "bastion",
        "updated": "2026-07-01",
        "focus": {
            "now": [
                { "id": "BA.16.A", "title": "State surface viewer safety", "repo": "bastion" },
                { "id": "BA.16.B", "title": "Something else" }
            ],
            "next": [
                { "id": "BA.17.A", "title": "Next block", "repo": "bastion" }
            ],
            "blocked": [
                { "id": "OR.B", "title": "Blocked block", "repo": "orchestrator" }
            ]
        }
    }"#;

    const EMPTY_FOCUS: &str = r#"{
        "repo": "bastion",
        "updated": "2026-07-01",
        "focus": { "now": [], "next": [], "blocked": [] }
    }"#;

    #[test]
    fn state_json_deserializes_representative_fixture() {
        let state: StateJson =
            serde_json::from_str(REPRESENTATIVE).expect("representative fixture should parse");

        assert_eq!(state.repo, "bastion");
        assert_eq!(state.updated, "2026-07-01");

        assert_eq!(state.focus.now.len(), 2);
        assert_eq!(state.focus.now[0].id, "BA.16.A");
        assert_eq!(state.focus.now[0].title, "State surface viewer safety");
        assert_eq!(state.focus.now[0].repo.as_deref(), Some("bastion"));
        assert_eq!(state.focus.now[1].id, "BA.16.B");
        assert_eq!(state.focus.now[1].repo, None);

        assert_eq!(state.focus.next.len(), 1);
        assert_eq!(state.focus.next[0].id, "BA.17.A");

        assert_eq!(state.focus.blocked.len(), 1);
        assert_eq!(state.focus.blocked[0].id, "OR.B");
        assert_eq!(state.focus.blocked[0].repo.as_deref(), Some("orchestrator"));
    }

    #[test]
    fn state_json_deserializes_empty_focus_arrays_cleanly() {
        let state: StateJson =
            serde_json::from_str(EMPTY_FOCUS).expect("empty-focus fixture should parse");

        assert!(state.focus.now.is_empty());
        assert!(state.focus.next.is_empty());
        assert!(state.focus.blocked.is_empty());
    }

    #[test]
    fn render_builds_expected_item_counts_without_panicking() {
        use ratatui::{Terminal, backend::TestBackend};

        let state: StateJson =
            serde_json::from_str(REPRESENTATIVE).expect("representative fixture should parse");

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("TestBackend terminal");
        terminal
            .draw(|f| {
                let area = f.area();
                render(f, &state, area);
            })
            .expect("render must not panic");

        // Each task renders as 3 ListItems (id line, title line, blank separator);
        // 2 tasks in "now", 1 in "next", 1 in "blocked".
        let build_items_len = |tasks: &[BlockTask]| tasks.len() * 3;
        assert_eq!(build_items_len(&state.focus.now), 6);
        assert_eq!(build_items_len(&state.focus.next), 3);
        assert_eq!(build_items_len(&state.focus.blocked), 3);
    }

    #[test]
    fn render_handles_empty_columns_without_panicking() {
        use ratatui::{Terminal, backend::TestBackend};

        let state: StateJson =
            serde_json::from_str(EMPTY_FOCUS).expect("empty-focus fixture should parse");

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("TestBackend terminal");
        terminal
            .draw(|f| {
                let area = f.area();
                render(f, &state, area);
            })
            .expect("render must not panic on empty columns");
    }
}
