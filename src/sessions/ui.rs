// sessions/ui.rs — ratatui session dashboard.
//
// This is the thin I/O shell over the pure `SessionApp` state model.
// Synchronous event loop (Decision D5 — no tokio coupling).
// DB-free (Decision D4 — no Config::load, no Postgres pool).

use crate::sessions::app::{Action, InputKind, Mode, SessionApp};
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
    style::{Modifier, Style},
    text::Line,
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
    format!("{:<20} {:<20} {}", s.name, format_state_col(s), last)
}

/// Render the footer key legend (Normal mode) or the active input prompt.
pub fn footer_hint(mode: &Mode) -> String {
    match mode {
        Mode::Normal => "[a]ttach [n]ew [s]end [k]ill [q]uit  ↑/j move".to_string(),
        Mode::Input(InputKind::New) => "new session name (Enter=create, Esc=cancel): ".to_string(),
        Mode::Input(InputKind::Send) => "send to selected (Enter=send, Esc=cancel): ".to_string(),
    }
}

/// Return the footer/status line content shown in the bottom bar.
/// In Normal mode: the transient status (or empty when none).
/// In Input mode: the prompt prepended to the live input buffer.
pub fn status_line(app: &SessionApp) -> String {
    match &app.mode {
        Mode::Normal => app.status.clone().unwrap_or_default(),
        Mode::Input(_) => format!("{}{}", footer_hint(&app.mode), app.input),
    }
}

// ── Frame builder (I/O — not unit-tested) ─────────────────────────────────────

fn draw(frame: &mut Frame, app: &SessionApp, list_state: &mut ListState) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());

    if app.sessions.is_empty() {
        let msg = Paragraph::new("no sessions — press [n] to create one")
            .block(Block::default().title("sessions").borders(Borders::ALL));
        frame.render_widget(msg, areas[0]);
    } else {
        let items: Vec<ListItem> = app
            .sessions
            .iter()
            .map(|s| ListItem::new(Line::from(session_row(s))))
            .collect();

        let list = List::new(items)
            .block(Block::default().title("sessions").borders(Borders::ALL))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        list_state.select(Some(app.selected));
        frame.render_stateful_widget(list, areas[0], list_state);
    }

    let footer = Paragraph::new(status_line(app));
    frame.render_widget(footer, areas[1]);
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
            s.last_line = Pane::new(&s.name, out).last_line().to_string();
        }
    }
    sessions
}

// ── Action execution helper ───────────────────────────────────────────────────

fn set_tmux_status(app: &mut SessionApp, verb: &str, name: &str, e: anyhow::Error) {
    if let Some(te) = e.downcast_ref::<TmuxError>() {
        let msg = match degrade_tmux_error(verb, name, te) {
            Degraded::Graceful(m) | Degraded::Fatal(m) => m,
        };
        app.status = Some(msg);
    } else {
        app.status = Some(e.to_string());
    }
}

fn execute_action(action: Action, app: &mut SessionApp) {
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
    app: &mut SessionApp,
) -> Result<()> {
    let mut list_state = ListState::default();

    loop {
        terminal.draw(|f| draw(f, app, &mut list_state))?;

        if event::poll(Duration::from_millis(REFRESH_MS))? {
            if let Event::Key(k) = event::read()? {
                let action = app.on_key(k.code);

                if let Action::Attach(ref name) = action {
                    // Suspend the TUI, hand the terminal to tmux, then restore.
                    let name = name.clone();
                    disable_raw_mode()?;
                    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

                    let res = tmux::attach_session(&name);

                    enable_raw_mode()?;
                    execute!(terminal.backend_mut(), EnterAlternateScreen)?;
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

/// Launch the interactive session dashboard (synchronous; no tokio).
pub fn run() -> Result<()> {
    let mut stdout = io::stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let mut app = SessionApp::new(poll_sessions());
    let result = run_inner(&mut terminal, &mut app);

    // Always tear down — even on the error path — so the terminal is never left
    // in raw mode or on the alternate screen.
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);

    result
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
        }
    }

    fn make_app(sessions: &[Session]) -> SessionApp {
        SessionApp::new(sessions.to_vec())
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
    fn status_line_empty_when_no_status_normal() {
        let app = make_app(&[]);
        let line = status_line(&app);
        assert_eq!(line, "");
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
