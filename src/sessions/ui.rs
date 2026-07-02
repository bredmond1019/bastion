// sessions/ui.rs — ratatui session dashboard.
//
// This is the thin I/O shell over the pure `SessionApp` state model.
// Synchronous event loop (Decision D5 — no tokio coupling).
// DB-free (Decision D4 — no Config::load, no Postgres pool).

use crate::sessions::app::{Action, AppState, InputKind, Mode};
use crate::sessions::commands::{Degraded, degrade_tmux_error};
use crate::sessions::model::{Pane, Session, parse_sessions};
use crate::sessions::tmux::{self, TmuxError};
use anyhow::Result;

use crossterm::{
    event::{self, Event},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};
use std::{io, time::Duration};

// Refresh cadence — poll tmux on timeout (matches the 2 s interval used elsewhere).
const REFRESH_MS: u64 = 2000;

// ── Pure render-string helpers (unit-testable, no Frame) ─────────────────────

/// Format a single session as a display row string.
/// Running sessions show "running (cmd)"; idle sessions show "idle".
pub fn session_row(s: &Session) -> String {
    use crate::sessions::commands::format_state_col;
    let last = if s.last_line.is_empty() {
        "(no output)"
    } else {
        s.last_line.as_str()
    };

    let state = if s.agent_state != crate::detect::AgentState::Unknown {
        s.agent_state.as_str().to_string()
    } else {
        format_state_col(s)
    };

    format!("{:<20} {:<20} {}", s.name, state, last)
}

/// Render the footer key legend (Normal mode) or the active input prompt.
pub fn footer_hint(mode: &Mode) -> String {
    match mode {
        Mode::Normal => {
            "[a]ttach [n]ew [s]end [k]ill [q]uit  ↑/j ↓/k move  Tab/Shift+Tab switch tab"
                .to_string()
        }
        Mode::Input(InputKind::New) => "new session name (Enter=create, Esc=cancel): ".to_string(),
        Mode::Input(InputKind::Send) => "send to selected (Enter=send, Esc=cancel): ".to_string(),
    }
}

/// Return the footer/status line content shown in the bottom bar.
/// In Normal mode: the transient status (or the key hint when none).
/// In Input mode: the prompt prepended to the live input buffer.
pub fn status_line(app: &AppState) -> String {
    match &app.mode {
        Mode::Normal => app.status.clone().unwrap_or_else(|| footer_hint(&app.mode)),
        Mode::Input(_) => format!("{}{}", footer_hint(&app.mode), app.input),
    }
}

/// Strip YAML frontmatter (`---` delimited block) from a markdown string.
/// If no frontmatter is found the original string is returned unchanged.
pub fn strip_frontmatter(md: &str) -> &str {
    let trimmed = md.trim_start();
    if !trimmed.starts_with("---") {
        return md;
    }
    // Skip the opening `---` line.
    let after_fence = &trimmed[3..];
    // Find the closing `---`.
    if let Some(pos) = after_fence.find("\n---") {
        // Skip past `\n---` plus the newline that follows it.
        let end = 3 + pos + 4; // 3 (opening) + pos + 4 ("\n---")
        let rest = &trimmed[end..];
        // Consume one optional newline after the closing fence.
        rest.trim_start_matches('\n')
    } else {
        md
    }
}

// ── Frame builder (I/O — not unit-tested) ─────────────────────────────────────

/// Build a colored sidebar list from the SpaceTree. Each repo gets a state dot and name.
fn build_sidebar_items(app: &AppState) -> Vec<ListItem<'static>> {
    let mut items = Vec::new();
    let flat_tree = app.space_tree.flatten();

    for (is_header, label, _repo_opt) in flat_tree {
        if is_header {
            let span = Span::styled(format!(" ▾ {}", label), crate::ui_theme::muted());
            items.push(ListItem::new(Line::from(vec![span])));
        } else {
            let mut dot = "  ○ ";
            let mut dot_style = crate::ui_theme::state_idle_style();

            if let Some(s) = app.sessions.iter().find(|s| s.name == label) {
                use crate::detect::AgentState;
                use crate::sessions::model::SessionState;
                match s.agent_state {
                    AgentState::Working => {
                        dot = "  ● ";
                        dot_style = crate::ui_theme::state_working_style();
                    }
                    AgentState::Blocked => {
                        dot = "  ● ";
                        dot_style = crate::ui_theme::state_blocked_style();
                    }
                    AgentState::Idle => {
                        dot = "  ○ ";
                        dot_style = crate::ui_theme::state_idle_style();
                    }
                    AgentState::Unknown => match s.state {
                        SessionState::Running => {
                            dot = "  ● ";
                            dot_style = crate::ui_theme::state_running_style();
                        }
                        SessionState::Idle => {
                            dot = "  ○ ";
                            dot_style = crate::ui_theme::state_idle_style();
                        }
                    },
                }
            }

            let name_style = Style::default().fg(crate::ui_theme::text());
            let spans = vec![
                Span::styled(dot, dot_style),
                Span::styled(label, name_style),
            ];
            items.push(ListItem::new(Line::from(spans)));
        }
    }
    items
}

/// Core frame-builder. Takes an explicit `planning_root` so tests can inject a
/// tempdir path without touching the process environment.
fn draw_with_root(
    frame: &mut Frame,
    app: &AppState,
    list_state: &mut ListState,
    planning_root: &std::path::Path,
) {
    let th = &crate::ui_theme::border_dim();
    let th_active = &crate::ui_theme::border_active();

    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());

    let (sidebar_area, main_area) = app.compute_view(areas[0]);

    // ── Sidebar ───────────────────────────────────────────────────────────────
    let sidebar_block = Block::default()
        .title(Span::styled(" spaces ", crate::ui_theme::title_style()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(*th));

    if app.space_tree.tiers.is_empty() {
        let msg = Paragraph::new(Span::styled(
            "  no spaces",
            Style::default().fg(crate::ui_theme::muted()),
        ))
        .block(sidebar_block);
        frame.render_widget(msg, sidebar_area);
    } else {
        let items = build_sidebar_items(app);
        let list = List::new(items)
            .block(sidebar_block)
            .highlight_style(crate::ui_theme::list_selected_style())
            .highlight_symbol("  ");

        list_state.select(Some(app.selected_space));
        frame.render_stateful_widget(list, sidebar_area, list_state);
    }

    // ── Main area: tab bar + content ──────────────────────────────────────────
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(main_area);

    // Tab bar — styled spans; active tab gets accent color + underline.
    use crate::sessions::app::TabState;
    let mut tab_spans: Vec<Span> = Vec::new();
    // Leading spacer
    tab_spans.push(Span::raw(" "));
    for (i, tab) in app.tabs.iter().enumerate() {
        let title = match tab {
            TabState::SpaceOverview => "Space Overview",
            TabState::Kanban => "Kanban Board",
            TabState::MissionControl => "Mission Control",
            TabState::MarkdownDocument(p) => p.to_str().unwrap_or("Doc"),
        };
        let label = format!(" {} ", title);
        let style = if i == app.active_tab_index {
            crate::ui_theme::tab_active_style()
        } else {
            crate::ui_theme::tab_inactive_style()
        };
        tab_spans.push(Span::styled(label, style));
        // Separator between tabs
        if i + 1 < app.tabs.len() {
            tab_spans.push(Span::styled(
                "  ",
                Style::default().fg(crate::ui_theme::muted()),
            ));
        }
    }

    let tabs_bar = Paragraph::new(Line::from(tab_spans)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(*th_active)),
    );
    frame.render_widget(tabs_bar, main_chunks[0]);

    // Content block shared border style
    let content_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(*th));

    match &app.tabs[app.active_tab_index] {
        TabState::MissionControl => {
            crate::monitor::ui::render(frame, &app.monitor_app, main_chunks[1]);
        }
        TabState::SpaceOverview => {
            let overview_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(30), Constraint::Min(0)])
                .split(main_chunks[1]);

            // Browser Pane
            let browser_border = if app.overview_pane == crate::sessions::app::OverviewPane::Browser
            {
                *th_active
            } else {
                *th
            };
            let browser_block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(browser_border))
                .title(Span::styled(
                    " file browser ",
                    crate::ui_theme::title_style(),
                ));

            let mut list_items = Vec::new();
            for entry in &app.file_browser.entries {
                let prefix = match entry.kind {
                    bella_engine::browser::BrowserEntryKind::ParentDir => " ⇧ ",
                    bella_engine::browser::BrowserEntryKind::Dir => " 📁 ",
                    bella_engine::browser::BrowserEntryKind::Markdown => " 📄 ",
                };
                let span = Span::raw(format!("{}{}", prefix, entry.display));
                list_items.push(ListItem::new(Line::from(vec![span])));
            }
            let mut list_state = ListState::default();
            list_state.select(Some(app.file_browser.selected));
            // Apply browser scroll offset manually if List doesn't do it automatically, wait List handles scroll implicitly via state!
            let browser_list = List::new(list_items)
                .block(browser_block)
                .highlight_style(crate::ui_theme::list_selected_style())
                .highlight_symbol(">>");

            frame.render_stateful_widget(browser_list, overview_chunks[0], &mut list_state);

            // Content Pane
            let content_border = if app.overview_pane == crate::sessions::app::OverviewPane::Content
            {
                *th_active
            } else {
                *th
            };
            let content_block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(content_border))
                .title(Span::styled(" content ", crate::ui_theme::title_style()));

            let file_path = match &app.space_overview_file {
                Some(p) => p.clone(),
                None => planning_root.join("status.md"),
            };

            let raw_md = std::fs::read_to_string(&file_path)
                .unwrap_or_else(|_| "No planning/status.md found.".to_string());
            // Strip YAML frontmatter before handing to bella.
            let status_md = strip_frontmatter(&raw_md).to_owned();
            let theme = bella_engine::Theme::bastion();
            let tables = bella_engine::links::TableExpansions::new();
            let rendered = bella_engine::render_with_edit(
                &status_md,
                None,
                overview_chunks[1].width.saturating_sub(2), // account for borders
                &theme,
                None,
                &tables,
            );
            let paragraph = Paragraph::new(rendered.lines)
                .block(content_block)
                .scroll((app.space_overview_scroll, 0));
            frame.render_widget(paragraph, overview_chunks[1]);
        }
        TabState::Kanban => {
            let state_json_path = planning_root.join("state.json");
            if let Ok(content) = std::fs::read_to_string(&state_json_path) {
                if let Ok(state) = serde_json::from_str::<crate::overview::StateJson>(&content) {
                    crate::overview::render(frame, &state, main_chunks[1]);
                } else {
                    let p = Paragraph::new(Span::styled(
                        "Failed to parse state.json",
                        Style::default().fg(crate::ui_theme::rose()),
                    ))
                    .block(content_block);
                    frame.render_widget(p, main_chunks[1]);
                }
            } else {
                let p = Paragraph::new(Span::styled(
                    "No planning/state.json found.",
                    Style::default().fg(crate::ui_theme::muted()),
                ))
                .block(content_block);
                frame.render_widget(p, main_chunks[1]);
            }
        }
        _ => {
            frame.render_widget(content_block, main_chunks[1]);
        }
    }

    // ── Footer ────────────────────────────────────────────────────────────────
    let footer_text = status_line(app);
    let footer_style = if app.status.is_some() && matches!(app.mode, Mode::Normal) {
        crate::ui_theme::footer_status_style()
    } else {
        crate::ui_theme::footer_style()
    };
    let footer = Paragraph::new(Span::styled(footer_text, footer_style));
    frame.render_widget(footer, areas[1]);
}

/// Thin real-world wrapper: resolves the planning root from the environment,
/// then delegates to `draw_with_root`.
fn draw(frame: &mut Frame, app: &AppState, list_state: &mut ListState) {
    draw_with_root(frame, app, list_state, &crate::config::load_planning_root());
}

// ── tmux poll → Vec<Session> ──────────────────────────────────────────────────

fn poll_sessions() -> Vec<Session> {
    let raw = match tmux::list_sessions_raw() {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let mut sessions = parse_sessions(&raw);
    for s in sessions.iter_mut() {
        if let Ok(out) = tmux::capture_pane_raw(&s.name) {
            s.last_line = Pane::new(&s.name, &out).last_line().to_string();
            s.agent_state = crate::serve::status::detect::detect_state(&out);
        }
    }
    sessions
}

// ── Action execution helper ───────────────────────────────────────────────────

fn set_tmux_status(app: &mut AppState, verb: &str, name: &str, e: anyhow::Error) {
    if let Some(te) = e.downcast_ref::<TmuxError>() {
        let msg = match degrade_tmux_error(verb, name, te) {
            Degraded::Graceful(m) | Degraded::Fatal(m) => m,
        };
        app.status = Some(msg);
    } else {
        app.status = Some(e.to_string());
    }
}

fn execute_action(action: Action, app: &mut AppState) {
    match action {
        Action::None | Action::Attach(_) => {
            // Attach is handled in the event loop (needs terminal suspension).
        }
        Action::SelectTab(i) => {
            app.active_tab_index = i;
        }
        Action::New(name) => match tmux::new_session(&name, None) {
            Ok(()) => app.status = Some(format!("created '{name}'")),
            Err(e) => set_tmux_status(app, "new", &name, e),
        },
        Action::Send { session, keys } => match tmux::send_keys(&session, &keys) {
            Ok(()) => app.status = Some(format!("sent to '{session}'")),
            Err(e) => set_tmux_status(app, "send", &session, e),
        },
        Action::Kill(name) => match tmux::kill_session(&name) {
            Ok(()) => {
                app.status = Some(format!("killed '{name}'"));
                app.set_sessions(poll_sessions());
            }
            Err(e) => set_tmux_status(app, "kill", &name, e),
        },
    }
}

// ── Event loop ────────────────────────────────────────────────────────────────

fn run_inner(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut AppState,
) -> Result<()> {
    let mut list_state = ListState::default();

    loop {
        terminal.draw(|f| draw(f, app, &mut list_state))?;

        if event::poll(Duration::from_millis(REFRESH_MS))? {
            match event::read()? {
                Event::Key(k) => {
                    let action = app.on_key(k.code);

                    if let Action::Attach(ref name) = action {
                        // Suspend the TUI, hand the terminal to tmux, then restore.
                        let name = name.clone();
                        disable_raw_mode()?;
                        execute!(
                            terminal.backend_mut(),
                            LeaveAlternateScreen,
                            event::DisableMouseCapture
                        )?;

                        let res = tmux::suspend_and_attach(&name);

                        enable_raw_mode()?;
                        execute!(
                            terminal.backend_mut(),
                            EnterAlternateScreen,
                            event::EnableMouseCapture
                        )?;
                        terminal.clear()?;

                        if let Err(e) = res {
                            set_tmux_status(app, "attach", &name, e);
                        }
                        app.set_sessions(poll_sessions());
                        continue;
                    }

                    execute_action(action, app);
                }
                Event::Mouse(m)
                    if m.kind == event::MouseEventKind::Down(event::MouseButton::Left) =>
                {
                    let areas = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Min(1), Constraint::Length(1)])
                        .split(terminal.size()?.into());
                    let (_, main_area) = app.compute_view(areas[0]);
                    let main_chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Length(3), Constraint::Min(0)])
                        .split(main_area);

                    let action = app.on_mouse(m.column, m.row, main_chunks[0]);
                    execute_action(action, app);
                }
                _ => {}
            }
        } else {
            // Timeout: refresh session list.
            app.set_sessions(poll_sessions());
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

/// Launch the interactive session dashboard (synchronous; no tokio).
pub fn run() -> Result<()> {
    let mut stdout = io::stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, event::EnableMouseCapture)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let space_tree = crate::brain::spaces::load_space_tree(&crate::config::load_brain_toml_path())
        .unwrap_or_default();
    let mut app = AppState::new(poll_sessions(), space_tree);
    let result = run_inner(&mut terminal, &mut app);

    // Always tear down — even on the error path — so the terminal is never left
    // in raw mode or on the alternate screen.
    let _ = disable_raw_mode();
    let _ = execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        event::DisableMouseCapture
    );

    result
}

// ── Test-only surface ────────────────────────────────────────────────────────

/// Thin wrapper over `draw_with_root`, exposed only in test builds so that
/// `tui_tests.rs` can drive a `TestBackend` frame with an injected planning root
/// without touching the process environment.
#[cfg(test)]
pub fn draw_for_test(
    frame: &mut ratatui::Frame,
    app: &AppState,
    list_state: &mut ratatui::widgets::ListState,
    planning_root: &std::path::Path,
) {
    draw_with_root(frame, app, list_state, planning_root);
}

// ── Unit tests for pure helpers ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::model::SessionState;

    fn make_session(name: &str, state: SessionState, last_line: &str) -> Session {
        Session {
            name: name.to_string(),
            state,
            window_count: 1,
            foreground_cmd: String::new(),
            last_line: last_line.to_string(),
            agent_state: crate::detect::AgentState::Unknown,
        }
    }

    fn make_session_with_cmd(
        name: &str,
        state: SessionState,
        foreground_cmd: &str,
        last_line: &str,
    ) -> Session {
        Session {
            name: name.to_string(),
            state,
            window_count: 1,
            foreground_cmd: foreground_cmd.to_string(),
            last_line: last_line.to_string(),
            agent_state: crate::detect::AgentState::Unknown,
        }
    }

    fn make_app(sessions: &[Session]) -> AppState {
        let mut tree = crate::brain::spaces::SpaceTree::default();
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

    #[test]
    fn session_row_running_with_cmd_shows_command() {
        let s = make_session_with_cmd("main", SessionState::Running, "claude", "some output");
        let row = session_row(&s);
        assert!(row.contains("main"), "row: {row}");
        assert!(row.contains("running (claude)"), "row: {row}");
        assert!(row.contains("some output"), "row: {row}");
    }

    #[test]
    fn session_row_idle_shows_idle() {
        let s = make_session_with_cmd("scratch", SessionState::Idle, "zsh", "");
        let row = session_row(&s);
        assert!(row.contains("idle"), "row: {row}");
        assert!(!row.contains("running"), "row must not say running: {row}");
    }

    #[test]
    fn session_row_empty_lastline_shows_placeholder() {
        let s = make_session("scratch", SessionState::Idle, "");
        let row = session_row(&s);
        assert!(row.contains("(no output)"), "row: {row}");
    }

    #[test]
    fn footer_hint_normal_lists_all_keys() {
        let hint = footer_hint(&Mode::Normal);
        assert!(hint.contains("[a]"), "hint: {hint}");
        assert!(hint.contains("[n]"), "hint: {hint}");
        assert!(hint.contains("[s]"), "hint: {hint}");
        assert!(hint.contains("[k]"), "hint: {hint}");
        assert!(hint.contains("[q]"), "hint: {hint}");
        assert!(hint.contains("Tab"), "hint: {hint}");
    }

    #[test]
    fn footer_hint_input_new_and_send_differ() {
        let new_hint = footer_hint(&Mode::Input(InputKind::New));
        let send_hint = footer_hint(&Mode::Input(InputKind::Send));
        assert_ne!(new_hint, send_hint);
        assert!(new_hint.contains("name"), "new_hint: {new_hint}");
        assert!(send_hint.contains("send"), "send_hint: {send_hint}");
    }

    #[test]
    fn status_line_shows_key_hint_when_no_status_normal() {
        let app = make_app(&[]);
        let line = status_line(&app);
        assert_eq!(line, footer_hint(&Mode::Normal));
    }

    #[test]
    fn status_line_input_mode_composes_prompt_and_buffer() {
        let mut app = make_app(&[]);
        app.mode = Mode::Input(InputKind::New);
        app.input = "my-session".into();
        let line = status_line(&app);
        // Must contain the prompt from footer_hint and the typed text.
        assert!(
            line.contains("my-session"),
            "status_line missing input: {line}"
        );
        assert!(
            line.contains("Enter=create"),
            "status_line missing prompt: {line}"
        );
    }
}
