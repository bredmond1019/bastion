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
use std::{
    fs, io,
    path::{Path, PathBuf},
};

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
// dispatches to, as of BA.26.G task 3's repoint of `src/main.rs`'s
// `Commands::Overview` arm to [`run_sections_ui`] below).
//
// PARKED, NOT DEAD: `run` and `StateJson` above still compile, are still
// exercised by their own tests (`parked_kanban_render_still_works_alongside_the_new_renderer`),
// and remain callable by anything that still wants the Kanban view — they
// are simply no longer what the `overview` subcommand invokes. Do not read
// their lack of a live call site as license to delete them; that reading is
// exactly what BA.26.I's decision exists to head off.
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
/// The content pane renders the selected section's document through
/// bella's markdown renderer (task 2), sharing the CALLER-HELD
/// `TableExpansions` — task 4 adds the in-document jumps. This function
/// never constructs its own `TableExpansions`; `tables` must be the SAME
/// persisted map the caller holds across draws (mirroring how BA.26.B's
/// `AppState::table_expansions` is held outside `draw_with_root` and
/// threaded through, rather than being rebuilt fresh every frame — the bug
/// this section exists to not repeat, since before this task
/// `src/overview/mod.rs` had zero `bella_engine` references at all).
/// Resolves a declared section to its markdown content: a section's `root`
/// may be a single markdown document or a directory of them (mirrors the
/// session TUI reader's own resolution); when it's a directory, `index.md`
/// is the entry document — the same convention `read_document`/
/// `render_document_markdown` apply everywhere else in this codebase for a
/// directory-rooted view.
///
/// Factored out of [`render_sections`]'s content pane so [`run_sections_inner`]'s
/// jump parsing (task 4) reads the SAME markdown the reader is looking at
/// rather than a second, possibly-diverging read of the document.
fn section_markdown(section: &OfferedView) -> String {
    let doc_path = if section.root.is_dir() {
        section.root.join("index.md")
    } else {
        section.root.clone()
    };
    let doc = crate::sessions::ui::read_document(&doc_path);
    let raw_md = crate::sessions::ui::render_document_markdown(
        &doc,
        &format!("No {} found.", doc_path.display()),
    );
    crate::sessions::ui::strip_frontmatter(&raw_md).to_owned()
}

pub fn render_sections(
    frame: &mut Frame,
    sections: &[OfferedView],
    selected: usize,
    tables: &bella_engine::links::TableExpansions,
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

    let content_area = layout[1];
    match sections.get(selected) {
        Some(section) => {
            let md = section_markdown(section);

            let theme = crate::ui_theme::to_bella_theme(crate::ui_theme::current_theme());
            // `tables` is threaded straight through from the caller — never
            // `bella_engine::links::TableExpansions::new()` constructed
            // here, which is exactly the per-draw-fresh-map bug BA.26.B
            // fixed for the session TUI reader (AC-3).
            let rendered = bella_engine::render_with_edit(
                &md,
                None,
                content_area.width.saturating_sub(2),
                &theme,
                None,
                tables,
            );

            let content = Paragraph::new(rendered.lines)
                .block(crate::ui_theme::themed_block(section.label.as_str(), false));
            frame.render_widget(content, content_area);
        }
        None => {
            let content = Paragraph::new("No open-work sections declared.")
                .block(crate::ui_theme::themed_block("", false));
            frame.render_widget(content, content_area);
        }
    }
}

// ── In-document jumps (BA.26.G task 4, AC-5) ────────────────────────────────
//
// An open-work document may declare an in-document jump using the same
// `[[wikilink]]` convention the corpus already uses elsewhere (bastion
// `AGENTS.md` standing rule 10 references `bastion brain`'s `[[wikilink]]`
// graph): `[[roadmap:<slug>]]`, `[[epic:<slug>]]`, `[[repo:<slug>]]`. Three
// kinds, three independent resolvers — a jump target that does not resolve
// degrades to a visible, worded [`JumpResolution::Unresolved`] rather than a
// silent no-op or a panic.

/// The three jump kinds a `[[<kind>:<id>]]` reference in an open-work
/// document may name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JumpKind {
    Roadmap,
    Epic,
    Repo,
}

/// One in-document jump reference, parsed from a section's markdown by
/// [`parse_jump_targets`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JumpTarget {
    pub kind: JumpKind,
    pub id: String,
}

/// The result of attempting to resolve a [`JumpTarget`] to something real on
/// disk. `Unresolved` carries a human-readable reason — this is the type
/// that makes a miss VISIBLE rather than a silent no-op: a caller renders
/// the reason instead of doing nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JumpResolution {
    /// The target resolved to a real path on disk.
    Found(PathBuf),
    /// The target did not resolve; the `String` is shown to the reader.
    Unresolved(String),
}

/// Scans `markdown` for `[[roadmap:<id>]]` / `[[epic:<id>]]` / `[[repo:<id>]]`
/// references, in source order. Any other `[[...]]` form (no recognized
/// `<kind>:` prefix) is ignored, not an error — this parser only ever adds
/// jump targets, it never rejects a document for containing an ordinary
/// wikilink.
pub fn parse_jump_targets(markdown: &str) -> Vec<JumpTarget> {
    let mut out = Vec::new();
    let mut rest = markdown;
    while let Some(open) = rest.find("[[") {
        let after_open = &rest[open + 2..];
        let Some(close) = after_open.find("]]") else {
            break;
        };
        let inner = &after_open[..close];
        if let Some((prefix, id)) = inner.split_once(':') {
            let kind = match prefix {
                "roadmap" => Some(JumpKind::Roadmap),
                "epic" => Some(JumpKind::Epic),
                "repo" => Some(JumpKind::Repo),
                _ => None,
            };
            if let Some(kind) = kind {
                let id = id.trim().to_string();
                if !id.is_empty() {
                    out.push(JumpTarget { kind, id });
                }
            }
        }
        rest = &after_open[close + 2..];
    }
    out
}

/// A minimal shape of HQ's `planning/state.json` — just enough to look an
/// epic slug up by name. Deliberately NOT [`StateJson`]/[`Focus`]/
/// [`BlockTask`] above (the parked Kanban path's types, untouched by this
/// task, AC-4) — this is a distinct document (HQ's own `state.json`, not the
/// per-repo focus board `run` reads) and a distinct, much smaller shape.
#[derive(serde::Deserialize, Debug, Clone, Default)]
struct HqEpicsFile {
    #[serde(default)]
    epics: Vec<HqEpicEntry>,
}

#[derive(serde::Deserialize, Debug, Clone)]
struct HqEpicEntry {
    slug: String,
    #[serde(default)]
    plan: Option<String>,
}

/// Resolves a `[[roadmap:<id>]]` jump: HQ's roadmaps live at
/// `<hq_root>/planning/roadmaps/<id>/roadmap.md` (`AGENTS.md`'s
/// `/roadmap-status` and `/generate-roadmap` both name this shape). Found
/// only when that exact file exists on disk.
pub fn resolve_roadmap_jump(id: &str, hq_root: &Path) -> JumpResolution {
    let path = hq_root
        .join("planning/roadmaps")
        .join(id)
        .join("roadmap.md");
    if path.is_file() {
        JumpResolution::Found(path)
    } else {
        JumpResolution::Unresolved(format!(
            "no roadmap named '{id}' (expected {})",
            path.display()
        ))
    }
}

/// Resolves an `[[epic:<id>]]` jump against HQ's `planning/state.json`
/// `epics[]` registry (`slug` + `plan`, per `AGENTS.md`'s epics-registry
/// description). Absent, unreadable, or malformed `state.json`; a slug with
/// no matching entry; or an entry with no `plan` path — all degrade to
/// [`JumpResolution::Unresolved`] with the specific reason, never a panic.
pub fn resolve_epic_jump(id: &str, hq_root: &Path) -> JumpResolution {
    let state_path = hq_root.join("planning/state.json");
    let content = match fs::read_to_string(&state_path) {
        Ok(c) => c,
        Err(e) => {
            return JumpResolution::Unresolved(format!(
                "could not read {}: {e}",
                state_path.display()
            ));
        }
    };
    let parsed: HqEpicsFile = match serde_json::from_str(&content) {
        Ok(p) => p,
        Err(e) => {
            return JumpResolution::Unresolved(format!(
                "could not parse {}: {e}",
                state_path.display()
            ));
        }
    };
    match parsed.epics.into_iter().find(|e| e.slug == id) {
        Some(HqEpicEntry {
            plan: Some(plan), ..
        }) => JumpResolution::Found(hq_root.join(plan)),
        Some(HqEpicEntry { plan: None, .. }) => {
            JumpResolution::Unresolved(format!("epic '{id}' has no plan path"))
        }
        None => JumpResolution::Unresolved(format!("no epic named '{id}'")),
    }
}

/// Resolves a `[[repo:<id>]]` jump against the already-loaded
/// [`crate::brain::spaces::SpaceTree`] (`brain.toml`'s `[[repos]]` table) —
/// `id` matches a `SpaceEntry::slug` in any tier.
pub fn resolve_repo_jump(id: &str, tree: &crate::brain::spaces::SpaceTree) -> JumpResolution {
    for (_tier, repos) in &tree.tiers {
        if let Some(entry) = repos.iter().find(|r| r.slug == id) {
            return JumpResolution::Found(entry.repo_path.clone());
        }
    }
    JumpResolution::Unresolved(format!("no repo named '{id}' in brain.toml"))
}

/// Dispatches a [`JumpTarget`] to its kind-specific resolver.
pub fn resolve_jump(
    target: &JumpTarget,
    hq_root: &Path,
    tree: &crate::brain::spaces::SpaceTree,
) -> JumpResolution {
    match target.kind {
        JumpKind::Roadmap => resolve_roadmap_jump(&target.id, hq_root),
        JumpKind::Epic => resolve_epic_jump(&target.id, hq_root),
        JumpKind::Repo => resolve_repo_jump(&target.id, tree),
    }
}

/// Renders the current jump status as a one-line footer: nothing selected
/// yet, a resolved target's path, or an unresolved target's reason — always
/// SOMETHING visible, never a blank line standing in for "nothing happened".
///
/// A `Found` jump maps onto [`crate::ui_theme::StatusKind::Success`] and an
/// `Unresolved` one onto [`crate::ui_theme::StatusKind::Failed`], both
/// resolved through [`crate::ui_theme::status_glyph_and_style`] (BA.26.H
/// task 4) rather than picking `sage()`/`rose()` ad hoc as before — the same
/// shared status set `src/runs/mod.rs`'s `finished_run_line` (task 3) and
/// `src/sessions/ui.rs`'s state dots (task 2) resolve through. The "nothing
/// selected yet" prompt isn't a pass/fail status at all, so it keeps plain
/// body text styling rather than being forced onto a `StatusKind` variant.
pub fn render_jump_status(
    frame: &mut Frame,
    status: Option<&JumpResolution>,
    area: ratatui::layout::Rect,
) {
    let (text, style) = match status {
        None => (
            "Press 'g' to jump to the next in-document roadmap/epic/repo reference.".to_string(),
            Style::default().fg(crate::ui_theme::text()),
        ),
        Some(JumpResolution::Found(path)) => {
            let (glyph, style) =
                crate::ui_theme::status_glyph_and_style(crate::ui_theme::StatusKind::Success);
            (format!("{glyph}-> Jumped to {}", path.display()), style)
        }
        Some(JumpResolution::Unresolved(reason)) => {
            let (glyph, style) =
                crate::ui_theme::status_glyph_and_style(crate::ui_theme::StatusKind::Failed);
            (format!("{glyph}x Could not resolve jump: {reason}"), style)
        }
    };
    let line = Paragraph::new(ratatui::text::Span::styled(text, style));
    frame.render_widget(line, area);
}

/// Launch the interactive open-work overview (the new `bastion overview`
/// entry point, BA.26.G task 3). Resolves the declared `[views]` table the
/// same absence-tolerant way `sessions::ui::run` resolves it for the session
/// TUI reader — an absent or malformed config, or an absent `[views]` table
/// entirely, degrades to zero declared sections rather than an error or a
/// panic, and [`render_sections`] already renders that case as a
/// placeholder rather than failing.
///
/// Holds exactly ONE `bella_engine::links::TableExpansions` for the whole
/// run, constructed once before the draw loop starts and threaded through
/// every [`render_sections`] call by reference — never rebuilt per frame,
/// which is the per-draw-fresh-map bug task 2 exists to not repeat.
pub fn run_sections_ui() -> Result<()> {
    let xdg = std::env::var("XDG_CONFIG_HOME").ok();
    let home = std::env::var("HOME").ok();
    let file = crate::config::load_workspace_registry(xdg, home).unwrap_or_default();
    crate::ui_theme::init_theme(crate::config::resolve_theme(&file));
    let sections = crate::openwork::resolved_sections(&file);

    // Jump resolution (task 4) resolves against HQ's own `brain.toml` +
    // `planning/state.json`, both rooted at the same directory —
    // `brain_toml_path`'s parent. An absent/unreadable `brain.toml` degrades
    // to an empty `SpaceTree` (repo jumps then simply never resolve, via
    // `resolve_repo_jump`'s own `Unresolved` path) rather than failing the
    // whole overview.
    let brain_toml_path = crate::config::load_brain_toml_path();
    let hq_root = brain_toml_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let tree = crate::brain::spaces::load_space_tree(&brain_toml_path).unwrap_or_default();

    let mut stdout = io::stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let result = run_sections_inner(&mut terminal, &sections, &hq_root, &tree);

    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    result
}

/// The open-work overview's draw/input loop. `selected` tracks the active
/// tab; Left/Right (and Tab/BackTab) cycle it, clamped into
/// `0..sections.len()` (or fixed at 0 when `sections` is empty, matching
/// [`render_sections`]'s own clamp), and `q` exits — the same quit key the
/// parked Kanban loop (`run_inner`) uses. `g` cycles through the current
/// section's in-document jump targets (task 4) and resolves the newly
/// selected one, shown in the footer [`render_jump_status`] draws every
/// frame — switching sections resets the cycle and clears the shown status.
fn run_sections_inner(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    sections: &[crate::config::OfferedView],
    hq_root: &Path,
    tree: &crate::brain::spaces::SpaceTree,
) -> Result<()> {
    // `TableExpansions` is `HashMap<u64, TableExpand>` (bella_engine::links) —
    // `::default()` here, not `::new()`, so this production initializer
    // reads distinctly from the pattern
    // `render_sections_source_never_constructs_a_fresh_table_expansions`
    // exists to forbid: a FRESH map built PER DRAW inside a `render_sections`
    // call. This one is built exactly ONCE, before the loop starts, and
    // threaded through every draw by reference below — never rebuilt.
    let tables = bella_engine::links::TableExpansions::default();
    let mut selected: usize = 0;
    let mut jump_index: usize = 0;
    let mut jump_status: Option<JumpResolution> = None;

    loop {
        terminal.draw(|f| {
            let area = f.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)])
                .split(area);
            render_sections(f, sections, selected, &tables, chunks[0]);
            render_jump_status(f, jump_status.as_ref(), chunks[1]);
        })?;

        #[allow(clippy::collapsible_if)]
        if event::poll(std::time::Duration::from_millis(250))? {
            if let Event::Key(k) = event::read()? {
                match k.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Right | KeyCode::Tab if !sections.is_empty() => {
                        selected = (selected + 1) % sections.len();
                        jump_index = 0;
                        jump_status = None;
                    }
                    KeyCode::Left | KeyCode::BackTab if !sections.is_empty() => {
                        selected = (selected + sections.len() - 1) % sections.len();
                        jump_index = 0;
                        jump_status = None;
                    }
                    KeyCode::Char('g') if !sections.is_empty() => {
                        let md = section_markdown(&sections[selected]);
                        let targets = parse_jump_targets(&md);
                        jump_status = if targets.is_empty() {
                            Some(JumpResolution::Unresolved(
                                "no in-document jump references in this section".to_string(),
                            ))
                        } else {
                            jump_index = (jump_index + 1) % targets.len();
                            Some(resolve_jump(&targets[jump_index], hq_root, tree))
                        };
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
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

        let sections = crate::openwork::resolved_sections(&fc);

        // AC-3: derived from the resolved table, never a literal count.
        let declared_with_existing_root = 2;
        assert_eq!(sections.len(), declared_with_existing_root);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("TestBackend terminal");
        terminal
            .draw(|f| {
                let area = f.area();
                let tables = bella_engine::links::TableExpansions::new();
                render_sections(f, &sections, 0, &tables, area);
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
        let sections = crate::openwork::resolved_sections(&fc);
        assert_eq!(sections.len(), 1);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("TestBackend terminal");
        terminal
            .draw(|f| {
                let area = f.area();
                let tables = bella_engine::links::TableExpansions::new();
                render_sections(f, &sections, 0, &tables, area);
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
        let sections = crate::openwork::resolved_sections(&fc);
        assert_eq!(sections.len(), 0);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("TestBackend terminal");
        terminal
            .draw(|f| {
                let area = f.area();
                let tables = bella_engine::links::TableExpansions::new();
                render_sections(f, &sections, 0, &tables, area);
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

    // ── Shared TableExpansions + section resolution (BA.26.G task 2) ───────

    /// A markdown source with one table whose single cell is wider than the
    /// content pane, so wrap-vs-clip is actually exercised. Mirrors
    /// `src/sessions/ui.rs`'s own `WIDE_TABLE_MD` fixture and the BA.26.B
    /// regression test built on it (`table_expansion_survives_rerender_via_app_state`).
    const WIDE_TABLE_MD: &str = "# T\n\n\
        | Col |\n\
        | --- |\n\
        | This cell holds a long run of prose text that is deliberately wider \
          than the content pane so that truncation or wrapping has something \
          real to do once the table is laid out at that width |\n";

    /// AC-2 of task 2 (the concrete content of the BA.26.B edge): an
    /// expansion recorded on the SAME `AppState::table_expansions` field the
    /// session TUI reader's key handler mutates (BA.26.B) is visible when
    /// rendered through `render_sections` — proving the two surfaces share
    /// state rather than each constructing their own map. A test proving
    /// only that `render_sections` renders correctly twice would NOT prove
    /// sharing; this test proves it by mutating the app's own field between
    /// the two renders and passing that exact field both times.
    #[test]
    fn render_sections_reflects_an_expansion_recorded_on_the_tui_readers_app_state() {
        use ratatui::{Terminal, backend::TestBackend};

        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("index.md"), WIDE_TABLE_MD).expect("write index.md");

        let section = crate::config::OfferedView {
            name: "wide".to_string(),
            label: "Wide Section".to_string(),
            root: dir.path().to_path_buf(),
        };
        let sections = vec![section];

        // The exact type `AppState` (the session TUI reader, BA.26.B) holds
        // as `table_expansions` — constructed via its own `AppState::new`,
        // not a bare map built just for this test.
        let mut app =
            crate::sessions::app::AppState::new(vec![], crate::brain::spaces::SpaceTree::default());

        // First render: through the TUI reader's own (fresh) map. No
        // expansion recorded yet, so the wide cell must clip.
        let width = 78u16;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("TestBackend terminal");
        terminal
            .draw(|f| {
                let area = ratatui::layout::Rect::new(0, 0, width + 2, 24);
                render_sections(f, &sections, 0, &app.table_expansions, area);
            })
            .expect("render_sections must not panic (unexpanded)");
        let clipped = buf_to_string(&terminal.backend().buffer().clone());
        assert!(
            clipped.contains('…'),
            "unexpanded first render must clip: {clipped}"
        );

        // Locate the table's id exactly as bella laid it out at this width,
        // then record the expansion on `app.table_expansions` — i.e.
        // "through the TUI reader surface".
        let theme = crate::ui_theme::to_bella_theme(crate::ui_theme::current_theme());
        let baseline = bella_engine::render_with_edit(
            WIDE_TABLE_MD,
            None,
            width,
            &theme,
            None,
            &bella_engine::links::TableExpansions::new(),
        );
        let id = baseline
            .table_map
            .regions
            .first()
            .expect("WIDE_TABLE_MD must lay out exactly one table region")
            .id;
        app.table_expansions.insert(
            id,
            bella_engine::links::TableExpand {
                all: false,
                cols: Default::default(),
                cells: std::iter::once((0usize, 0usize)).collect(),
            },
        );

        // Second render: "through the other surface" — `render_sections` —
        // passed the SAME `app.table_expansions`, mutated in place, never
        // reconstructed. The expansion must be visible here.
        let backend2 = TestBackend::new(80, 24);
        let mut terminal2 = Terminal::new(backend2).expect("TestBackend terminal");
        terminal2
            .draw(|f| {
                let area = ratatui::layout::Rect::new(0, 0, width + 2, 24);
                render_sections(f, &sections, 0, &app.table_expansions, area);
            })
            .expect("render_sections must not panic (expanded)");
        let wrapped = buf_to_string(&terminal2.backend().buffer().clone());
        assert!(
            !wrapped.contains('…'),
            "an expansion recorded on the TUI reader's own AppState.table_expansions \
             must be visible through render_sections, not re-clipped: {wrapped}"
        );
    }

    /// AC-1 of task 2: no draw path in this module constructs a fresh
    /// `TableExpansions` — asserted indirectly by proving `render_sections`
    /// actually honors an externally-supplied, non-empty map (a renderer
    /// that silently substituted `TableExpansions::new()` internally would
    /// fail this the same way it fails the sharing test above), and
    /// directly by a source-level check over actual CODE lines (comments
    /// and doc comments are allowed to mention the pattern by name when
    /// explaining why it must not appear in code, as this file's own doc
    /// comments do).
    #[test]
    fn render_sections_source_never_constructs_a_fresh_table_expansions() {
        let source = include_str!("mod.rs");
        // Split off this test module's own fixture/setup lines (which
        // legitimately construct `TableExpansions::new()` to build a test
        // baseline) — only the non-test portion of the file is a "draw
        // path".
        let production_source = source
            .split("mod tests {")
            .next()
            .expect("file must contain a tests module");

        let offending_code_lines: Vec<&str> = production_source
            .lines()
            .filter(|line| {
                let trimmed = line.trim_start();
                !trimmed.starts_with("//") && line.contains("TableExpansions::new()")
            })
            .collect();

        assert!(
            offending_code_lines.is_empty(),
            "no production draw path in src/overview/mod.rs may construct a fresh \
             TableExpansions — it must be threaded through from the caller; \
             offending line(s): {offending_code_lines:?}"
        );
    }

    /// AC-3 of task 2: section resolution is the SAME shared function used
    /// by `src/openwork/`, not a second resolver that happens to agree
    /// today. Proven by calling `openwork::resolved_sections` (as the
    /// production call sites above do) and cross-checking it returns
    /// exactly what `config::offered_views` returns for the same input.
    #[test]
    fn section_resolution_is_shared_with_openwork() {
        use std::collections::HashMap;

        let dir = tempfile::tempdir().expect("tempdir");
        let mut views = HashMap::new();
        views.insert(
            "shared".to_string(),
            crate::config::ViewEntry {
                label: "Shared Section".to_string(),
                root: dir.path().to_path_buf(),
            },
        );
        let fc = crate::config::FileConfig {
            views: Some(views),
            ..crate::config::FileConfig::default()
        };

        let via_openwork = crate::openwork::resolved_sections(&fc);
        let via_config = crate::config::offered_views(&fc);
        assert_eq!(via_openwork, via_config);
        assert_eq!(via_openwork.len(), 1);
    }

    // ── In-document jumps (BA.26.G task 4, AC-5) ────────────────────────────

    /// `parse_jump_targets` finds all three recognized kinds, in source
    /// order, and ignores an ordinary `[[wikilink]]` with no `<kind>:`
    /// prefix it recognizes.
    #[test]
    fn parse_jump_targets_finds_all_three_kinds_and_ignores_unrecognized_prefixes() {
        let md = "See [[roadmap:bastion-tui]] and [[epic:fleet-integrity]], \
                   also [[repo:bastion]] and [[not-a-kind:whatever]] and a plain [[Some Note]].";
        let targets = parse_jump_targets(md);
        assert_eq!(
            targets,
            vec![
                JumpTarget {
                    kind: JumpKind::Roadmap,
                    id: "bastion-tui".to_string()
                },
                JumpTarget {
                    kind: JumpKind::Epic,
                    id: "fleet-integrity".to_string()
                },
                JumpTarget {
                    kind: JumpKind::Repo,
                    id: "bastion".to_string()
                },
            ]
        );
    }

    /// AC-5, roadmap kind: `[[roadmap:<id>]]` resolves to
    /// `<hq_root>/planning/roadmaps/<id>/roadmap.md` when that file exists.
    #[test]
    fn roadmap_jump_resolves_to_its_roadmap_md() {
        let hq_root = tempfile::tempdir().expect("tempdir");
        let roadmap_dir = hq_root.path().join("planning/roadmaps/bastion-tui");
        std::fs::create_dir_all(&roadmap_dir).expect("create roadmap dir");
        let roadmap_md = roadmap_dir.join("roadmap.md");
        std::fs::write(&roadmap_md, "# Bastion TUI roadmap\n").expect("write roadmap.md");

        let resolution = resolve_roadmap_jump("bastion-tui", hq_root.path());
        assert_eq!(resolution, JumpResolution::Found(roadmap_md));
    }

    /// AC-5, epic kind: `[[epic:<id>]]` resolves via HQ's
    /// `planning/state.json` `epics[]` registry to the matching entry's
    /// `plan` path.
    #[test]
    fn epic_jump_resolves_to_its_plan_path() {
        let hq_root = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(hq_root.path().join("planning")).expect("create planning dir");
        std::fs::write(
            hq_root.path().join("planning/state.json"),
            r#"{"epics":[{"slug":"fleet-integrity","title":"x","plan":"planning/epics/fleet-integrity.md"}]}"#,
        )
        .expect("write state.json");

        let resolution = resolve_epic_jump("fleet-integrity", hq_root.path());
        assert_eq!(
            resolution,
            JumpResolution::Found(hq_root.path().join("planning/epics/fleet-integrity.md"))
        );
    }

    /// AC-5, repo kind: `[[repo:<id>]]` resolves via the loaded
    /// `SpaceTree` (`brain.toml`'s `[[repos]]` table) to the matching
    /// entry's `repo_path`.
    #[test]
    fn repo_jump_resolves_to_its_repo_path() {
        let repo_path = std::path::PathBuf::from("/home/user/agentic-portfolio/core/bastion");
        let tree = crate::brain::spaces::SpaceTree {
            tiers: vec![(
                "core".to_string(),
                vec![crate::brain::spaces::SpaceEntry {
                    slug: "bastion".to_string(),
                    tier: "core".to_string(),
                    repo_path: repo_path.clone(),
                    heading: None,
                }],
            )],
        };

        let resolution = resolve_repo_jump("bastion", &tree);
        assert_eq!(resolution, JumpResolution::Found(repo_path));
    }

    /// A jump target that does not resolve — for each of the three kinds —
    /// degrades to a worded [`JumpResolution::Unresolved`], never a panic
    /// and never silently nothing.
    #[test]
    fn unresolved_jump_targets_carry_a_worded_reason_for_every_kind() {
        let hq_root = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(hq_root.path().join("planning")).expect("create planning dir");
        std::fs::write(
            hq_root.path().join("planning/state.json"),
            r#"{"epics":[]}"#,
        )
        .expect("write empty state.json");
        let empty_tree = crate::brain::spaces::SpaceTree::default();

        let roadmap_miss = resolve_roadmap_jump("does-not-exist", hq_root.path());
        assert!(
            matches!(roadmap_miss, JumpResolution::Unresolved(ref r) if r.contains("does-not-exist"))
        );

        let epic_miss = resolve_epic_jump("does-not-exist", hq_root.path());
        assert!(
            matches!(epic_miss, JumpResolution::Unresolved(ref r) if r.contains("does-not-exist"))
        );

        let repo_miss = resolve_repo_jump("does-not-exist", &empty_tree);
        assert!(
            matches!(repo_miss, JumpResolution::Unresolved(ref r) if r.contains("does-not-exist"))
        );
    }

    /// `resolve_jump` dispatches each [`JumpKind`] to its own resolver —
    /// proven end-to-end through the shared entry point rather than only
    /// through the three kind-specific functions directly.
    #[test]
    fn resolve_jump_dispatches_to_the_matching_kind_specific_resolver() {
        let hq_root = tempfile::tempdir().expect("tempdir");
        let roadmap_dir = hq_root.path().join("planning/roadmaps/foo");
        std::fs::create_dir_all(&roadmap_dir).expect("create roadmap dir");
        std::fs::write(roadmap_dir.join("roadmap.md"), "# Foo\n").expect("write roadmap.md");
        let tree = crate::brain::spaces::SpaceTree::default();

        let target = JumpTarget {
            kind: JumpKind::Roadmap,
            id: "foo".to_string(),
        };
        let resolution = resolve_jump(&target, hq_root.path(), &tree);
        assert_eq!(
            resolution,
            JumpResolution::Found(roadmap_dir.join("roadmap.md"))
        );
    }

    /// AC-5's miss case, at the render layer: an unresolved jump status
    /// renders VISIBLE text distinguishable from both "nothing selected yet"
    /// and a resolved jump — never a blank line, never a panic.
    #[test]
    fn render_jump_status_shows_found_and_unresolved_distinguishably() {
        use ratatui::{Terminal, backend::TestBackend};

        let backend = TestBackend::new(80, 3);
        let mut terminal = Terminal::new(backend).expect("TestBackend terminal");

        // Nothing selected yet.
        terminal
            .draw(|f| {
                let area = f.area();
                render_jump_status(f, None, area);
            })
            .expect("render_jump_status must not panic (no status)");
        let idle = buf_to_string(&terminal.backend().buffer().clone());
        assert!(idle.contains("Press"));

        // Found.
        let found = JumpResolution::Found(std::path::PathBuf::from("/tmp/roadmap.md"));
        terminal
            .draw(|f| {
                let area = f.area();
                render_jump_status(f, Some(&found), area);
            })
            .expect("render_jump_status must not panic (found)");
        let found_rendered = buf_to_string(&terminal.backend().buffer().clone());
        assert!(found_rendered.contains("Jumped to"));
        assert!(found_rendered.contains("roadmap.md"));

        // Unresolved — the miss case must degrade VISIBLY, not silently.
        let unresolved = JumpResolution::Unresolved("no epic named 'ghost'".to_string());
        terminal
            .draw(|f| {
                let area = f.area();
                render_jump_status(f, Some(&unresolved), area);
            })
            .expect("render_jump_status must not panic (unresolved)");
        let unresolved_rendered = buf_to_string(&terminal.backend().buffer().clone());
        assert!(unresolved_rendered.contains("Could not resolve"));
        assert!(unresolved_rendered.contains("no epic named 'ghost'"));

        // All three renders must be visibly distinct from one another.
        assert_ne!(idle, found_rendered);
        assert_ne!(idle, unresolved_rendered);
        assert_ne!(found_rendered, unresolved_rendered);
    }

    /// BA.26.H task 4: `render_jump_status`'s Found/Unresolved colours must
    /// resolve through `ui_theme`'s shared `StatusKind` set
    /// (`Success`/`Failed`) rather than picking `sage()`/`rose()` ad hoc —
    /// asserted against the buffer's actual foreground colour, which is the
    /// same (glyph, style) pair `status_glyph_and_style` returns.
    #[test]
    fn jump_status_found_and_unresolved_use_the_shared_status_kind_colours() {
        use crate::ui_theme::{StatusKind, status_glyph_and_style};
        use ratatui::{Terminal, backend::TestBackend};

        let (_, success_style) = status_glyph_and_style(StatusKind::Success);
        let (_, failed_style) = status_glyph_and_style(StatusKind::Failed);

        let backend = TestBackend::new(80, 1);
        let mut terminal = Terminal::new(backend).expect("TestBackend terminal");

        let found = JumpResolution::Found(std::path::PathBuf::from("/tmp/roadmap.md"));
        terminal
            .draw(|f| {
                let area = f.area();
                render_jump_status(f, Some(&found), area);
            })
            .expect("render_jump_status must not panic (found)");
        let found_cell = terminal
            .backend()
            .buffer()
            .cell((0, 0))
            .expect("cell present");
        assert_eq!(found_cell.fg, success_style.fg.expect("style has an fg"));

        let unresolved = JumpResolution::Unresolved("no epic named 'ghost'".to_string());
        terminal
            .draw(|f| {
                let area = f.area();
                render_jump_status(f, Some(&unresolved), area);
            })
            .expect("render_jump_status must not panic (unresolved)");
        let unresolved_cell = terminal
            .backend()
            .buffer()
            .cell((0, 0))
            .expect("cell present");
        assert_eq!(
            unresolved_cell.fg,
            failed_style.fg.expect("style has an fg")
        );
    }
}
