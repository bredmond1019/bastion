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
    style::{Color, Modifier, Style},
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
    let mut path = std::env::current_dir()?;
    // Up one directory if we are in core/bastion
    if path.ends_with("bastion") {
        path.pop();
    }
    path.push("planning");
    path.push("state.json");

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

    // Header
    let header = Paragraph::new(format!(
        " Kanban Board — {} (updated {})",
        state.repo, state.updated
    ))
    .style(Style::default().add_modifier(Modifier::BOLD))
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(header, main_layout[0]);

    // Columns
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(33),
            Constraint::Percentage(33),
            Constraint::Percentage(34),
        ])
        .split(main_layout[1]);

    let render_col = |title: String, items: &[BlockTask], color: Color| -> List<'static> {
        let list_items: Vec<ListItem> = items
            .iter()
            .map(|b| {
                let repo = b.repo.as_deref().unwrap_or("unknown");
                ListItem::new(format!("[{}] {}: {}", repo, b.id, b.title))
            })
            .collect();
        List::new(list_items).block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(color)),
        )
    };

    frame.render_widget(
        render_col(
            " In Progress (Now) ".to_string(),
            &state.focus.now,
            Color::Green,
        ),
        columns[0],
    );
    frame.render_widget(
        render_col(" Up Next ".to_string(), &state.focus.next, Color::Yellow),
        columns[1],
    );
    frame.render_widget(
        render_col(" Blocked ".to_string(), &state.focus.blocked, Color::Red),
        columns[2],
    );
}
