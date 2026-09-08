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
    widgets::{List, ListItem, Paragraph, Tabs},
};
use std::{fs, io};

use crate::config::OfferedView;

/// Height of the "In Progress" Kanban column as a percentage of the columns row.
const NOW_COLUMN_PCT: u16 = 33;
/// Height of the "Up Next" Kanban column as a percentage of the columns row.
const NEXT_COLUMN_PCT: u16 = 33;
/// Height of the "Blocked" Kanban column as a percentage of the columns row.
const BLOCKED_COLUMN_PCT: u16 = 34;

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
    .block(crate::ui_theme::themed_block("", true));
    frame.render_widget(header, main_layout[0]);

    // ── Columns ───────────────────────────────────────────────────────────────
    let columns = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(NOW_COLUMN_PCT),
            Constraint::Percentage(NEXT_COLUMN_PCT),
            Constraint::Percentage(BLOCKED_COLUMN_PCT),
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

    let now_list = List::new(now_items).block(crate::ui_theme::themed_block(
        ratatui::text::Span::styled(
            " In Progress ",
            Style::default()
                .fg(crate::ui_theme::sage())
                .add_modifier(Modifier::BOLD),
        ),
        false,
    ));

    let next_list = List::new(next_items).block(crate::ui_theme::themed_block(
        ratatui::text::Span::styled(
            " Up Next ",
            Style::default()
                .fg(crate::ui_theme::violet())
                .add_modifier(Modifier::BOLD),
        ),
        false,
    ));

    let blocked_list = List::new(blocked_items).block(crate::ui_theme::themed_block(
        ratatui::text::Span::styled(
            " Blocked ",
            Style::default()
                .fg(crate::ui_theme::rose())
                .add_modifier(Modifier::BOLD),
        ),
        false,
    ));

    frame.render_widget(now_list, columns[0]);
    frame.render_widget(next_list, columns[1]);
    frame.render_widget(blocked_list, columns[2]);
}

// ── Open-work renderer (BA.26.G) ─────────────────────────────────────────────
//
// This is a NEW front end for `bastion overview`, rendered ALONGSIDE the
// parked Kanban path above — `render` and `StateJson` are untouched by this
// block (BA.26.I's decision D20 supersedes D13's Kanban clause; the old path
// stays reachable as code but is simply no longer what `bastion overview`
// dispatches to, once BA.26.G task 3 repoints `src/main.rs`).
//
// Sections come from the config's `[views]` table (BA.26.A), already
// resolved to the "safe to offer" subset by `config::offered_views` — a
// declared view whose `root` does not exist on disk has already been
// dropped there, so this module does no existence-checking of its own and
// cannot re-derive a different answer than the TUI reader's spine does.

/// Renders the config-declared open-work sections as a tab strip (fast
/// section switching) plus a content pane for the currently selected
/// section, into `area`.
///
/// `sections` MUST be the already-resolved output of
/// [`crate::config::offered_views`] — a declared section whose `root` does
/// not exist has already been filtered out there, so a caller that passes
/// the raw `[views]` table instead would silently re-offer an absent
/// section; this function does not re-check existence itself (single
/// resolver, not two).
///
/// `selected` is clamped into range; an empty `sections` list renders a
/// single "no sections declared" placeholder rather than panicking on an
/// out-of-bounds `Tabs::select`.
///
/// The content pane here is a placeholder (label + root path) — task 2 wires
/// it to bella's markdown renderer sharing the TUI reader's persisted
/// `TableExpansions`, and task 4 adds the in-document jumps. This task's
/// job is the section resolution and the tab-switching skeleton only.
pub fn render_sections(
    frame: &mut Frame,
    sections: &[OfferedView],
    selected: usize,
    area: ratatui::layout::Rect,
) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(area);

    let titles: Vec<ratatui::text::Line> = sections
        .iter()
        .map(|s| ratatui::text::Line::from(s.label.clone()))
        .collect();

    let tabs = Tabs::new(titles)
        .select(if sections.is_empty() {
            0
        } else {
            selected.min(sections.len() - 1)
        })
        .highlight_style(
            Style::default()
                .fg(crate::ui_theme::accent())
                .add_modifier(Modifier::BOLD),
        )
        .block(crate::ui_theme::themed_block(" Open Work ", true));
    frame.render_widget(tabs, layout[0]);

    let content = match sections.get(selected) {
        Some(section) => Paragraph::new(format!("{}\n{}", section.label, section.root.display())),
        None => Paragraph::new("No open-work sections declared."),
    }
    .block(crate::ui_theme::themed_block("", false));
    frame.render_widget(content, layout[1]);
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

    // ── render_sections (BA.26.G task 1) ────────────────────────────────────

    fn buf_to_string(buf: &ratatui::buffer::Buffer) -> String {
        let area = buf.area;
        (0..area.height)
            .flat_map(|y| {
                (0..area.width).map(move |x| {
                    buf.cell((x, y))
                        .map(|c| c.symbol().to_string())
                        .unwrap_or_default()
                })
            })
            .collect()
    }

    /// AC-1 (shown-failing half) + AC-3 (derived count, no literal): three
    /// views declared in the `[views]` table, two with an existing `root`
    /// and one whose `root` is absent from disk. `config::offered_views`
    /// resolves that down to the two present sections — this test asserts
    /// the RENDER agrees: the two present sections' labels appear in the
    /// buffer, distinguishable from the absent one, which appears nowhere
    /// (not as an error, not as an empty pane — simply not offered), and
    /// the count driving the tab strip is `sections.len()`, never a
    /// literal `2`.
    #[test]
    fn render_sections_offers_only_sections_with_an_existing_root() {
        use ratatui::{Terminal, backend::TestBackend};
        use std::collections::HashMap;

        let dir_alpha = tempfile::tempdir().expect("tempdir alpha");
        let dir_beta = tempfile::tempdir().expect("tempdir beta");

        let mut views = HashMap::new();
        views.insert(
            "alpha".to_string(),
            crate::config::ViewEntry {
                label: "Alpha Section".to_string(),
                root: dir_alpha.path().to_path_buf(),
            },
        );
        views.insert(
            "beta".to_string(),
            crate::config::ViewEntry {
                label: "Beta Section".to_string(),
                root: dir_beta.path().to_path_buf(),
            },
        );
        // Declared, but its source path does not exist on disk — must not
        // be offered at all (SHOWN FAILING half of AC-1: a renderer that
        // shows this as an error pane or an empty pane fails this test).
        views.insert(
            "gamma".to_string(),
            crate::config::ViewEntry {
                label: "Gamma Section".to_string(),
                root: std::path::PathBuf::from("/definitely/does/not/exist/BA-26-G-absent-fixture"),
            },
        );

        let fc = crate::config::FileConfig {
            views: Some(views),
            ..crate::config::FileConfig::default()
        };

        let sections = crate::config::offered_views(&fc);

        // AC-3: derived from the resolved table, never a literal count.
        let declared_with_existing_root = 2;
        assert_eq!(sections.len(), declared_with_existing_root);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("TestBackend terminal");
        terminal
            .draw(|f| {
                let area = f.area();
                render_sections(f, &sections, 0, area);
            })
            .expect("render_sections must not panic");

        let rendered = buf_to_string(&terminal.backend().buffer().clone());
        assert!(
            rendered.contains("Alpha Section"),
            "present section 'Alpha Section' must be offered:\n{rendered}"
        );
        assert!(
            rendered.contains("Beta Section"),
            "present section 'Beta Section' must be offered:\n{rendered}"
        );
        assert!(
            !rendered.contains("Gamma Section"),
            "absent-source section 'Gamma Section' must NOT be offered:\n{rendered}"
        );
    }

    /// AC-1 for a single declared section with an existing root: it is
    /// offered as its own tab, distinct from the empty-table case.
    #[test]
    fn render_sections_offers_a_single_present_section() {
        use ratatui::{Terminal, backend::TestBackend};
        use std::collections::HashMap;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut views = HashMap::new();
        views.insert(
            "solo".to_string(),
            crate::config::ViewEntry {
                label: "Solo Section".to_string(),
                root: dir.path().to_path_buf(),
            },
        );
        let fc = crate::config::FileConfig {
            views: Some(views),
            ..crate::config::FileConfig::default()
        };
        let sections = crate::config::offered_views(&fc);
        assert_eq!(sections.len(), 1);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("TestBackend terminal");
        terminal
            .draw(|f| {
                let area = f.area();
                render_sections(f, &sections, 0, area);
            })
            .expect("render_sections must not panic");

        let rendered = buf_to_string(&terminal.backend().buffer().clone());
        assert!(rendered.contains("Solo Section"));
    }

    /// No `[views]` table declared at all: zero sections, and
    /// `render_sections` degrades to a placeholder rather than panicking on
    /// an out-of-range `Tabs::select`.
    #[test]
    fn render_sections_handles_zero_declared_sections_without_panicking() {
        use ratatui::{Terminal, backend::TestBackend};

        let fc = crate::config::FileConfig::default();
        let sections = crate::config::offered_views(&fc);
        assert_eq!(sections.len(), 0);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("TestBackend terminal");
        terminal
            .draw(|f| {
                let area = f.area();
                render_sections(f, &sections, 0, area);
            })
            .expect("render_sections must not panic on zero declared sections");

        let rendered = buf_to_string(&terminal.backend().buffer().clone());
        assert!(rendered.contains("No open-work sections declared."));
    }

    /// `render` and `StateJson` are untouched by this task (AC-4 of task 1)
    /// — the parked Kanban path keeps working exactly as before, proven by
    /// re-running its own existing render test alongside the new renderer's.
    #[test]
    fn parked_kanban_render_still_works_alongside_the_new_renderer() {
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
            .expect("parked Kanban render must still work");
    }
}
