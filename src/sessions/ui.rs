// sessions/ui.rs — ratatui session dashboard.
//
// This is the thin I/O shell over the pure `SessionApp` state model.
// Synchronous event loop (Decision D5 — no tokio coupling).
// DB-free (Decision D4 — no Config::load, no Postgres pool).

use crate::brain::spaces::{SelectedNode, SpineRow};
use crate::detect::AgentState;
use crate::sessions::agent_panel::{AgentPanelRow, agent_panel_rows};
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
            "[a]ttach [n]ew [s]end [k]ill [q]uit  ↑/j ↓/k move spine (wraps)".to_string()
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

/// Compute the path to a tier's `planning/status.md`, rooted at the brain repo root
/// (e.g. `<brain_root>/core/planning/status.md`). Pure — no I/O.
pub fn tier_status_path(brain_root: &std::path::Path, tier: &str) -> std::path::PathBuf {
    brain_root.join(tier).join("planning").join("status.md")
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

/// Preferred (`MIN_HEIGHT`..=`MAX_HEIGHT`) total height (borders + rows) for the
/// always-on bottom "agents · priority" strip, given how many sessions it needs
/// to show and the full frame height. Grows with `row_count` up to `MAX_HEIGHT`,
/// but shrinks (down to `0`, never panics/underflows) once the frame is too
/// short to spare `MIN_HEIGHT` after reserving room for the main content area
/// and the one-line footer — the "min-height fallback" for tight terminals.
/// Pure — no `Frame`/I/O, unit-tested directly.
pub fn agent_panel_strip_height(row_count: usize, frame_height: u16) -> u16 {
    const MIN_HEIGHT: u16 = 3; // 1 content row + 2 border lines
    const MAX_HEIGHT: u16 = 7; // 5 content rows + 2 border lines

    let desired = (row_count as u16)
        .saturating_add(2)
        .clamp(MIN_HEIGHT, MAX_HEIGHT);

    // Always leave at least 1 line for the main content area and 1 for the
    // footer; when the frame can't spare that, shrink toward 0 rather than
    // producing a layout that overflows the frame.
    let available = frame_height.saturating_sub(2);
    desired.min(available)
}

/// Dot glyph + themed style for one `AgentState`, used by the sidebar space
/// dots and the agent panel strip. Reads the live runtime theme (BA.14.0) —
/// never a literal color.
fn agent_state_dot(state: AgentState) -> (&'static str, Style) {
    match state {
        AgentState::Blocked => ("● ", crate::ui_theme::state_blocked_style()),
        AgentState::Working => ("● ", crate::ui_theme::state_working_style()),
        AgentState::Idle => ("○ ", crate::ui_theme::state_idle_style()),
        AgentState::Unknown => ("○ ", crate::ui_theme::state_idle_style()),
    }
}

/// Build the list rows for the agent panel strip from already-urgency-sorted
/// `AgentPanelRow`s, applying the themed state dot to each.
fn build_agent_panel_items(rows: &[AgentPanelRow]) -> Vec<ListItem<'static>> {
    rows.iter()
        .map(|row| {
            let (dot, dot_style) = agent_state_dot(row.agent_state);
            let name_style = Style::default().fg(crate::ui_theme::text());
            let spans = vec![
                Span::styled(dot, dot_style),
                Span::styled(row.label.clone(), name_style),
            ];
            ListItem::new(Line::from(spans))
        })
        .collect()
}

// ── Frame builder (I/O — not unit-tested) ─────────────────────────────────────

/// Build the sidebar item for a single `Space` row: a state dot + the space's slug,
/// colored by the matching session's detected `AgentState` (falling back to the raw
/// tmux `SessionState` when the agent state is unknown, and to idle when no matching
/// session exists at all).
fn build_space_item(app: &AppState, label: &str) -> ListItem<'static> {
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
        Span::styled(label.to_string(), name_style),
    ];
    ListItem::new(Line::from(spans))
}

/// Build the primary-navigation sidebar from `spine_rows()` — the pinned
/// `◆ Mission Control` row, the `HQ` header + its `learn-ai`/`base-template`
/// children, then the remaining tier headers (`core`/`side`/`client`/`portfolio`/
/// any other) with their space rows. Every row is selectable (headers included),
/// matching `AppState::select_next`/`select_prev`'s wrap-over-all-rows behaviour.
fn build_sidebar_items(app: &AppState) -> Vec<ListItem<'static>> {
    let mut items = Vec::new();

    for row in app.spine_rows() {
        match row {
            SpineRow::MissionControl => {
                let span = Span::styled(" ◆ Mission Control", crate::ui_theme::title_style());
                items.push(ListItem::new(Line::from(vec![span])));
            }
            SpineRow::Hq => {
                let span = Span::styled(" ▾ HQ", crate::ui_theme::muted());
                items.push(ListItem::new(Line::from(vec![span])));
            }
            SpineRow::Tier(name) => {
                let span = Span::styled(format!(" ▾ {name}"), crate::ui_theme::muted());
                items.push(ListItem::new(Line::from(vec![span])));
            }
            SpineRow::Space(entry) => {
                items.push(build_space_item(app, &entry.slug));
            }
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

    // The bottom "agents · priority" strip (BA.13.1.3) is always reserved,
    // regardless of `SelectedNode` — it renders under Mission Control, HQ,
    // every tier, and every space.
    let panel_rows = agent_panel_rows(&app.sessions);
    let strip_height = agent_panel_strip_height(panel_rows.len(), frame.area().height);

    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(strip_height),
            Constraint::Length(1),
        ])
        .split(frame.area());

    let (sidebar_area, main_area) = app.compute_view(areas[0]);

    // ── Sidebar ───────────────────────────────────────────────────────────────
    let sidebar_block = Block::default()
        .title(Span::styled(" spaces ", crate::ui_theme::title_style()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(*th));

    // `◆ Mission Control` is pinned first by `spine_rows()` regardless of whether
    // `space_tree` has any tiers, so the sidebar always has at least one row —
    // no "no spaces" empty-state branch is needed here anymore.
    let items = build_sidebar_items(app);
    let list = List::new(items)
        .block(sidebar_block)
        .highlight_style(crate::ui_theme::list_selected_style())
        .highlight_symbol("  ");

    list_state.select(Some(app.selected_spine));
    frame.render_stateful_widget(list, sidebar_area, list_state);

    // ── Main area: content ──────────────────────────────────────────────────
    // NOTE: the top tab bar is gone (spine is now the single primary navigator);
    // routing below keys off `selected_node()`.
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0)])
        .split(main_area);

    // Content block shared border style
    let content_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(*th));

    match app.selected_node() {
        SelectedNode::MissionControl => {
            crate::monitor::ui::render(frame, &app.monitor_app, main_chunks[0]);
        }
        SelectedNode::Tier(tier_name) => {
            // Rooted at `<brain_root>/<tier>/planning/status.md`; missing tier/file
            // degrades gracefully to a placeholder instead of panicking.
            let brain_root = crate::config::load_brain_toml_path()
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            let file_path = tier_status_path(&brain_root, &tier_name);

            let raw_md = std::fs::read_to_string(&file_path)
                .unwrap_or_else(|_| format!("No {} found.", file_path.display()));
            let status_md = strip_frontmatter(&raw_md).to_owned();
            let theme = crate::ui_theme::to_bella_theme(crate::ui_theme::current_theme());
            let tables = bella_engine::links::TableExpansions::new();
            let rendered = bella_engine::render_with_edit(
                &status_md,
                None,
                main_chunks[0].width.saturating_sub(2), // account for borders
                &theme,
                None,
                &tables,
            );
            let tier_block = content_block.clone().title(Span::styled(
                format!(" {tier_name} "),
                crate::ui_theme::title_style(),
            ));
            let paragraph = Paragraph::new(rendered.lines).block(tier_block);
            frame.render_widget(paragraph, main_chunks[0]);
        }
        SelectedNode::Hq | SelectedNode::Space(_) => {
            let overview_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(30), Constraint::Min(0)])
                .split(main_chunks[0]);

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
            let theme = crate::ui_theme::to_bella_theme(crate::ui_theme::current_theme());
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
    }

    // ── Agent panel strip ────────────────────────────────────────────────────
    // Always-on cross-space "agents · priority" strip, sorted by urgency
    // (Blocked/needs-input first — `session_urgency`/`agent_panel_rows`,
    // BA.13.1.1/.2). Renders under every `SelectedNode`.
    let strip_block = Block::default()
        .title(Span::styled(
            " agents · priority ",
            crate::ui_theme::title_style(),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(*th));
    let strip_list = List::new(build_agent_panel_items(&panel_rows)).block(strip_block);
    frame.render_widget(strip_list, areas[1]);

    // ── Footer ────────────────────────────────────────────────────────────────
    let footer_text = status_line(app);
    let footer_style = if app.status.is_some() && matches!(app.mode, Mode::Normal) {
        crate::ui_theme::footer_status_style()
    } else {
        crate::ui_theme::footer_style()
    };
    let footer = Paragraph::new(Span::styled(footer_text, footer_style));
    frame.render_widget(footer, areas[2]);
}

/// Thin real-world wrapper: resolves the planning root from the environment,
/// then delegates to `draw_with_root`.
fn draw(frame: &mut Frame, app: &AppState, list_state: &mut ListState) {
    let root = app.current_space_planning_root();
    draw_with_root(frame, app, list_state, &root);
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
            // Mouse handling (click-to-select on the tab bar) is out of scope for
            // this block (BA.13.2 — the top tab bar itself is gone) and deferred; only
            // key events are handled here.
            if let Event::Key(k) = event::read()? {
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

/// Resolve the active theme from the on-disk config (DB-free — see D4) and
/// initialize the process-wide runtime theme so chrome and the markdown view
/// (`render_with_edit`) share one palette. A missing/unreadable/malformed
/// config degrades gracefully to the `bastion` default rather than panicking.
fn init_theme_from_config() {
    let file = crate::config::load_workspace_registry(
        std::env::var("XDG_CONFIG_HOME").ok(),
        std::env::var("HOME").ok(),
    )
    .unwrap_or_default();
    crate::ui_theme::init_theme(crate::config::resolve_theme(&file));
}

/// Launch the interactive session dashboard (synchronous; no tokio).
pub fn run() -> Result<()> {
    init_theme_from_config();

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
        // The top tab bar + Tab/Shift+Tab cycling is gone (spine is now the single
        // primary navigator) — the hint must not reference it.
        assert!(!hint.contains("Tab"), "hint: {hint}");
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

    // ── tier_status_path ─────────────────────────────────────────────────────

    #[test]
    fn tier_status_path_joins_tier_planning_status() {
        let root = std::path::Path::new("/brain");
        let path = tier_status_path(root, "core");
        assert_eq!(
            path,
            std::path::PathBuf::from("/brain/core/planning/status.md")
        );
    }

    #[test]
    fn tier_status_path_differs_per_tier() {
        let root = std::path::Path::new("/brain");
        assert_ne!(
            tier_status_path(root, "core"),
            tier_status_path(root, "side")
        );
    }

    // ── agent_panel_strip_height (pure, BA.13.1.3) ──────────────────────────────

    #[test]
    fn strip_height_grows_with_row_count_up_to_max() {
        assert_eq!(agent_panel_strip_height(0, 24), 3);
        assert_eq!(agent_panel_strip_height(1, 24), 3);
        assert_eq!(agent_panel_strip_height(3, 24), 5);
        // 5 sessions -> desired 7, capped at MAX_HEIGHT (7).
        assert_eq!(agent_panel_strip_height(5, 24), 7);
        // Growing further does not exceed the cap.
        assert_eq!(agent_panel_strip_height(50, 24), 7);
    }

    #[test]
    fn strip_height_shrinks_toward_zero_on_tiny_frames() {
        // No room to spare beyond main(1) + footer(1) -> strip collapses to 0.
        assert_eq!(agent_panel_strip_height(0, 2), 0);
        assert_eq!(agent_panel_strip_height(0, 1), 0);
        assert_eq!(agent_panel_strip_height(0, 0), 0);
        // A little more room than the reserved main+footer lines, but still
        // below MIN_HEIGHT -> never underflows/panics, just yields what's left.
        assert_eq!(agent_panel_strip_height(0, 4), 2);
    }

    #[test]
    fn strip_height_never_exceeds_available_frame_space() {
        for frame_height in 0..30u16 {
            let h = agent_panel_strip_height(10, frame_height);
            assert!(
                h <= frame_height.saturating_sub(2),
                "strip height {h} must not exceed frame_height({frame_height}) - 2"
            );
        }
    }

    // ── Runtime theme drives chrome + the render_with_edit seam (BA.14.0.3) ────

    /// A "working" session's sidebar dot must be colored from the live
    /// `current_theme()` (the same runtime theme `state_working_style()` reads),
    /// not a baked literal — proving chrome tracks the runtime theme instead of a
    /// fixed color.
    #[test]
    fn build_space_item_working_dot_tracks_runtime_theme() {
        use ratatui::{Terminal, backend::TestBackend};

        let session = Session {
            name: "core".to_string(),
            state: SessionState::Running,
            window_count: 1,
            foreground_cmd: String::new(),
            last_line: String::new(),
            agent_state: crate::detect::AgentState::Working,
        };
        let app = make_app(&[session]);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("TestBackend terminal");
        let dir = std::env::temp_dir();
        terminal
            .draw(|f| {
                let mut list_state = ratatui::widgets::ListState::default();
                draw_with_root(f, &app, &mut list_state, &dir);
            })
            .expect("draw must not panic");
        let buf = terminal.backend().buffer().clone();

        let expected = crate::ui_theme::current_theme().sage;
        let mut found_dot = false;
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                if let Some(cell) = buf.cell((x, y)) {
                    if cell.symbol() == "●" {
                        found_dot = true;
                        assert_eq!(
                            cell.fg, expected,
                            "working-state dot must render with the runtime theme's sage color"
                        );
                    }
                }
            }
        }
        assert!(found_dot, "expected a working-state session dot to render");
    }

    /// `draw_with_root` hands `render_with_edit` the theme produced by
    /// `to_bella_theme(current_theme())` — assert that seam stays in lock-step
    /// with the live runtime theme (rather than asserting on opaque rendered
    /// pixel colors, which `render_with_edit`'s markdown layout makes brittle).
    #[test]
    fn render_with_edit_receives_theme_mapped_from_current_theme() {
        let live = crate::ui_theme::current_theme();
        let mapped = crate::ui_theme::to_bella_theme(live);

        assert_eq!(mapped.fg, live.text);
        assert_eq!(mapped.muted, live.muted);
        assert_eq!(mapped.link, live.cyan);
        assert_eq!(mapped.link_focused, live.violet);
        assert_eq!(mapped.code_fg, live.sage);
        assert_eq!(mapped.code_bg, Some(live.surface));
        assert_eq!(mapped.rule, live.border_dim);
        assert_eq!(mapped.status_bg, live.border_active);
    }
}
