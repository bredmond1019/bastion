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
