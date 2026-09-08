// sessions/ui.rs — ratatui session dashboard.
//
// This is the thin I/O shell over the pure `SessionApp` state model.
// Synchronous event loop (Decision D5 — no tokio coupling).
// DB-free (Decision D4 — no Config::load, no Postgres pool).

use crate::brain::spaces::{SelectedNode, SpineRow};
use crate::detect::AgentState;
use crate::sessions::agent_panel::{AgentPanelRow, agent_panel_rows};
use crate::sessions::app::{
    Action, AppState, InputKind, Mode, NORMAL_KEY_BINDINGS, OpenWorkStatus,
};
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
    widgets::{List, ListItem, ListState, Paragraph},
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
///
/// Normal mode is rendered from `NORMAL_KEY_BINDINGS` (BA.26.B task 4) —
/// the single source of truth for which keys the footer advertises — rather
/// than a hand-maintained string, so a key can only appear here if it is
/// listed there. See `AppState`'s `footer_normal_key_bindings_each_resolve_to_a_bound_handler`
/// test for the other half of the contract: every listed key actually
/// resolves to a bound `on_key` handler.
pub fn footer_hint(mode: &Mode) -> String {
    match mode {
        Mode::Normal => {
            let legend: Vec<String> = NORMAL_KEY_BINDINGS
                .iter()
                .map(|b| {
                    let mut chars = b.label.chars();
                    let first = chars
                        .next()
                        .expect("KeyBinding.label must be non-empty");
                    debug_assert_eq!(
                        first, b.key,
                        "KeyBinding.label must start with its own key so `[x]abel` renders correctly: key={:?} label={:?}",
                        b.key, b.label
                    );
                    format!("[{}]{}", b.key, chars.as_str())
                })
                .collect();
            format!("{}  ↑/j ↓/k move spine (wraps)", legend.join(" "))
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
        // Open-work refresh progress/result (BA.26.C task 4) takes priority
        // over an ordinary `app.status` message while in Normal mode — it
        // reflects a subprocess actually in flight or just finished, which
        // is more current than whatever the last keypress set `app.status`
        // to. `Idle` returns `None` here, so this is a no-op before the
        // first refresh and existing footer behaviour is unchanged. Scoped
        // to `Mode::Normal` only — a refresh finishing mid-typed-input must
        // not blank out the operator's in-progress `Input` prompt/buffer.
        Mode::Normal => openwork_status_message(&app.openwork_status)
            .or_else(|| app.status.clone())
            .unwrap_or_else(|| footer_hint(&app.mode)),
        Mode::Input(_) => format!("{}{}", footer_hint(&app.mode), app.input),
    }
}

/// Render `AppState::openwork_status` (BA.26.C task 4) as footer text, or
/// `None` while `Idle` so callers fall through to their existing behaviour.
/// The `Failed`/`SpawnFailed` arms show the REAL exit code / OS error text
/// (AC-2) — never a generic "refresh failed" message, so an operator's next
/// action (retry vs. fix the environment) is legible from the footer alone.
fn openwork_status_message(status: &OpenWorkStatus) -> Option<String> {
    use crate::openwork::RefreshOutcome;
    match status {
        OpenWorkStatus::Idle => None,
        OpenWorkStatus::Refreshing => Some("refreshing open-work boards…".to_string()),
        OpenWorkStatus::Done(outcome) => Some(match outcome {
            RefreshOutcome::Current => "open-work boards refreshed — all current".to_string(),
            RefreshOutcome::StaleOrChanged => "open-work boards refreshed — updated".to_string(),
            RefreshOutcome::Failed { code } => format!("open-work refresh FAILED (exit {code})"),
            RefreshOutcome::SpawnFailed { reason } => {
                format!("open-work refresh failed to start: {reason}")
            }
        }),
    }
}

/// Compute the path to a tier's `planning/status.md`, rooted at the brain repo root
/// (e.g. `<brain_root>/core/planning/status.md`). Pure — no I/O.
pub fn tier_status_path(brain_root: &std::path::Path, tier: &str) -> std::path::PathBuf {
    brain_root.join(tier).join("planning").join("status.md")
}

/// Strip YAML frontmatter (`---` delimited block) from a markdown string.
/// If no frontmatter is found the original string is returned unchanged.
///
/// AC-7 (BA.26.B task 6) reconciliation: fence *detection* delegates to
/// `bella_engine::frontmatter::detect_fence` — the module bella owns since
/// BE.7.A — rather than re-implementing it by hand. bastion keeps exactly
/// two behaviours on top of that delegation, both real leniency this shell
/// depends on for files read straight off disk, not legacy accidents.
///
/// First, leading whitespace/blank lines before the opening fence are
/// trimmed before detection runs, so a file with a stray leading blank line
/// still has its frontmatter recognized. Second, every blank line
/// immediately after the closing fence is consumed
/// (`trim_start_matches('\n')`), not just one, so multiple trailing blank
/// lines in the frontmatter block don't leak into the rendered body.
///
/// `detect_fence` itself is stricter than the old hand-rolled search — it
/// requires the closing line to be *exactly* `---`, where the previous
/// bastion code matched the substring `"\n---"` anywhere (which could
/// false-match a closing fence followed immediately by more text on the
/// same line). That tightening is a correctness improvement inherited for
/// free. A future bella change to fence detection therefore surfaces here
/// as a test failure (see `strip_frontmatter_*` tests below) rather than as
/// a silently different render.
pub fn strip_frontmatter(md: &str) -> &str {
    let trimmed = md.trim_start();
    match bella_engine::frontmatter::detect_fence(trimmed) {
        Some(range) => trimmed[range.end..].trim_start_matches('\n'),
        None => md,
    }
}

/// Outcome of reading a markdown document off disk (BA.26.B AC "CONCURRENCY
/// WITH REFRESH").
///
/// Before this type, both content-reading call sites collapsed EVERY
/// `read_to_string` error into the same "No <path> found." placeholder,
/// which is indistinguishable from the file simply not existing yet. That
/// is wrong for a real, non-hypothetical race: BA.26.C's refresh performs
/// non-atomic truncating writes to exactly these files, and HQ's
/// `routine.sh` regenerates them by cron at 03:00 — so a reader can observe
/// a torn or momentarily-unreadable file with nobody at the keyboard. A
/// torn document must never render as if it were ordinary content, and it
/// must not be reported as absent either — both are misleading in
/// different ways.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentRead {
    /// The file does not exist — the ordinary, expected "nothing here yet"
    /// case (`io::ErrorKind::NotFound`).
    Absent,
    /// The file exists but the read did not succeed cleanly — permission
    /// error, or a read racing a concurrent non-atomic writer. Carries the
    /// path and the `io::ErrorKind` (as its `Debug` name) so the rendered
    /// state names the failure instead of silently degrading to "not
    /// found".
    Failed {
        path: std::path::PathBuf,
        kind: String,
    },
    /// The read succeeded; contents follow.
    Ok(String),
}

/// Read a markdown document, splitting `std::fs::read_to_string`'s single
/// `Result` into the three outcomes a reader must render differently: file
/// absent, read failed/torn, or read succeeded. Pure I/O shell — the
/// three-way split itself is the testable decision (see
/// `read_document_distinguishes_absent_from_failed` below).
pub fn read_document(path: &std::path::Path) -> DocumentRead {
    match std::fs::read_to_string(path) {
        Ok(contents) => DocumentRead::Ok(contents),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentRead::Absent,
        Err(e) => DocumentRead::Failed {
            path: path.to_path_buf(),
            kind: format!("{:?}", e.kind()),
        },
    }
}

/// Render a `DocumentRead` to the markdown string handed to `strip_frontmatter`
/// / `render_with_edit`. `absent_message` preserves each call site's existing
/// "No <path> found." wording for the ordinary absent case; a failed/torn
/// read renders its own named state instead, never the absent-file text and
/// never partially-read content.
pub fn render_document_markdown(read: &DocumentRead, absent_message: &str) -> String {
    match read {
        DocumentRead::Ok(contents) => contents.clone(),
        DocumentRead::Absent => absent_message.to_string(),
        DocumentRead::Failed { path, kind } => {
            format!("**Read failed:** `{}` ({kind})", path.display())
        }
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
            SpineRow::View(view) => {
                let span = Span::styled(
                    format!("  {} ", view.label),
                    Style::default().fg(crate::ui_theme::text()),
                );
                items.push(ListItem::new(Line::from(vec![span])));
            }
        }
    }
    items
}

/// Deterministic fingerprint of a `TableExpansions` map's expand/collapse
/// state, used as part of `RenderCacheKey` so a table toggle still
/// invalidates the cache even though `TableExpansions` (a bare
/// `HashMap<u64, TableExpand>` from bella) implements neither `Hash` nor
/// `PartialEq`. Iteration order over a `HashMap` is not stable, so the keys
/// are sorted before hashing — two maps with the same entries in a different
/// insertion/iteration order must fingerprint identically.
fn table_expansions_fingerprint(tables: &bella_engine::links::TableExpansions) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut keys: Vec<&u64> = tables.keys().collect();
    keys.sort_unstable();

    let mut hasher = DefaultHasher::new();
    for key in keys {
        let expand = &tables[key];
        key.hash(&mut hasher);
        expand.all.hash(&mut hasher);
        let mut cols: Vec<&usize> = expand.cols.iter().collect();
        cols.sort_unstable();
        cols.hash(&mut hasher);
        let mut cells: Vec<&(usize, usize)> = expand.cells.iter().collect();
        cells.sort_unstable();
        cells.hash(&mut hasher);
    }
    hasher.finish()
}

/// Identity of one cached `render_with_edit` output: which document (by
/// path), what its raw content was, the width it was laid out at, and the
/// table-expansion state it was rendered under. A pure scroll changes none
/// of these — only `AppState::space_overview_scroll`, which `draw_with_root`
/// applies via `Paragraph::scroll` after the fact — so a `RenderCacheKey`
/// unchanged between two frames means the previous frame's `Rendered` is
/// still exactly correct and does not need re-parsing.
#[derive(PartialEq, Eq)]
struct RenderCacheKey {
    path: std::path::PathBuf,
    content: String,
    width: u16,
    expansions_fingerprint: u64,
}

/// Single-slot cache for the markdown parse/layout pass
/// (`bella_engine::render_with_edit`) shared by both `draw_with_root` content
/// call sites (the Tier status pane and the Hq/Space overview content pane).
/// `read_document` + `strip_frontmatter` + `render_with_edit` walks the whole
/// document through `pulldown-cmark` and re-runs the full wrap/layout pass —
/// on every frame, that is a full re-parse of the entire document, even on a
/// frame whose only change is the scroll offset. This block replaced
/// table-cell clipping with wrapping (strictly more layout work) on exactly
/// the files this initiative makes one-keypress-reachable, so a pure scroll
/// paying for a full re-parse is the concern task 8 exists to close. Held by
/// `run_inner` and threaded through `draw`/`draw_with_root` so it survives
/// across frames — never reconstructed per-draw, which would defeat it the
/// same way task 1 found `TableExpansions::new()` defeated table expansion.
///
/// Only the Tier/Hq/Space branches use it — `SelectedNode::MissionControl`
/// has no markdown document to cache.
#[derive(Default)]
struct RenderCache {
    entry: Option<(RenderCacheKey, bella_engine::Rendered)>,
    /// Incremented only on an actual `render_with_edit` call (a cache miss).
    /// Exists so a test can assert *zero* re-parses across a pure scroll by
    /// counting, rather than inferring it from output shape alone.
    parses: usize,
}

impl RenderCache {
    /// Return the cached `Rendered` for `(path, content, width, tables)` if
    /// the previous call's key matches exactly, otherwise run
    /// `bella_engine::render_with_edit` (bumping `parses`) and cache the
    /// fresh result before returning it.
    fn get_or_render(
        &mut self,
        path: &std::path::Path,
        content: &str,
        width: u16,
        theme: &bella_engine::Theme,
        tables: &bella_engine::links::TableExpansions,
    ) -> bella_engine::Rendered {
        let key = RenderCacheKey {
            path: path.to_path_buf(),
            content: content.to_string(),
            width,
            expansions_fingerprint: table_expansions_fingerprint(tables),
        };

        if let Some((cached_key, cached)) = &self.entry
            && *cached_key == key
        {
            return cached.clone();
        }

        let rendered = bella_engine::render_with_edit(content, None, width, theme, None, tables);
        self.parses += 1;
        self.entry = Some((key, rendered.clone()));
        rendered
    }
}

/// Core frame-builder. Takes an explicit `planning_root` so tests can inject a
/// tempdir path without touching the process environment.
fn draw_with_root(
    frame: &mut Frame,
    app: &mut AppState,
    list_state: &mut ListState,
    planning_root: &std::path::Path,
    render_cache: &mut RenderCache,
) {
    // The bottom "agents · priority" strip (BA.13.1.3) is always reserved,
    // regardless of `SelectedNode` — it renders under Mission Control, HQ,
    // every tier, and every space.
    let panel_rows = agent_panel_rows(&app.sessions);
    let strip_height = agent_panel_strip_height(panel_rows.len(), frame.area().height);

    // Single source of truth for pane geometry (BA.13.2): computed once here
    // and stored on `AppState` so the pure mouse dispatcher can hit-test
    // against the exact same Rects this frame rendered into.
    let selected_node = app.selected_node();
    app.pane_areas =
        crate::sessions::app::compute_pane_areas(frame.area(), strip_height, &selected_node);

    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(strip_height),
            Constraint::Length(1),
        ])
        .split(frame.area());

    let sidebar_area = app.pane_areas.spine;

    // ── Sidebar ───────────────────────────────────────────────────────────────
    let sidebar_block = crate::ui_theme::themed_block(
        Span::styled(" spaces ", crate::ui_theme::title_style()),
        false,
    );

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
    // routing below keys off `selected_node()`. Pane Rects come from
    // `app.pane_areas` (computed once above via `compute_pane_areas`) rather
    // than re-deriving the same `Layout` splits here.
    let content_area = app.pane_areas.content;
    let browser_area = app.pane_areas.browser;

    match app.selected_node() {
        SelectedNode::MissionControl => {
            crate::monitor::ui::render(frame, &app.monitor_app, content_area);
        }
        SelectedNode::Tier(tier_name) => {
            // Rooted at `<brain_root>/<tier>/planning/status.md`; missing tier/file
            // degrades gracefully to a placeholder instead of panicking.
            let brain_root = crate::config::load_brain_toml_path()
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            let file_path = tier_status_path(&brain_root, &tier_name);

            let doc = read_document(&file_path);
            let raw_md =
                render_document_markdown(&doc, &format!("No {} found.", file_path.display()));
            let status_md = strip_frontmatter(&raw_md).to_owned();
            let theme = crate::ui_theme::to_bella_theme(crate::ui_theme::current_theme());
            let rendered = render_cache.get_or_render(
                &file_path,
                &status_md,
                content_area.width.saturating_sub(2), // account for borders
                &theme,
                &app.table_expansions,
            );
            // Feed the just-rendered table geometry back onto `AppState` so
            // `handle_click`'s `content_table_map.hit(line, col)` resolves
            // against what is actually on screen, not the empty default
            // (BA.26.B review fix — see task 2's deviation note).
            app.content_table_map = rendered.table_map.clone();
            let tier_block = crate::ui_theme::themed_block(
                Span::styled(format!(" {tier_name} "), crate::ui_theme::title_style()),
                false,
            );
            let paragraph = Paragraph::new(rendered.lines).block(tier_block);
            frame.render_widget(paragraph, content_area);
        }
        SelectedNode::Hq | SelectedNode::Space(_) | SelectedNode::View(_) => {
            // Browser Pane — a declared `[views]` entry (BA.26.A) shares this
            // rendering path: `app.file_browser`/`planning_root` are already
            // rooted at the view's declared `root` via `reinit_browser` /
            // `current_space_planning_root`.
            let browser_active = app.overview_pane == crate::sessions::app::OverviewPane::Browser;
            let browser_block = crate::ui_theme::themed_block(
                Span::styled(" file browser ", crate::ui_theme::title_style()),
                browser_active,
            );

            let mut list_items = Vec::new();
            for entry in &app.file_browser.entries {
                let prefix = match entry.kind {
                    bella_engine::browser::BrowserEntryKind::ParentDir => " ⇧ ",
                    bella_engine::browser::BrowserEntryKind::Dir => " 📁 ",
                    bella_engine::browser::BrowserEntryKind::ExpandedDir => " 📂 ",
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

            frame.render_stateful_widget(browser_list, browser_area, &mut list_state);

            // Content Pane
            //
            // The browser's `t` ("open") key formerly set a transient
            // `AppState::markdown_overlay` field with no reader anywhere in
            // this module — it was never drawn, never cleared, and the key
            // was never advertised in the footer legend (`NORMAL_KEY_BINDINGS`
            // has no 't' entry). BA.26.B task 5 removed the field, its setter
            // in `AppState::on_key`, and its test rather than finish the
            // wiring: finishing it would need a new close keybinding, which
            // this block's scope explicitly excludes ("New keybindings beyond
            // the expansion toggle..."). The single content pane below —
            // driven by `space_overview_file`, Enter to open — is the only
            // markdown-viewing path; there is no full-screen overlay.
            let content_active = app.overview_pane == crate::sessions::app::OverviewPane::Content;
            let content_block = crate::ui_theme::themed_block(
                Span::styled(" content ", crate::ui_theme::title_style()),
                content_active,
            );

            let file_path = match &app.space_overview_file {
                Some(p) => p.clone(),
                None => planning_root.join("status.md"),
            };

            let doc = read_document(&file_path);
            let raw_md = render_document_markdown(&doc, "No planning/status.md found.");
            // Strip YAML frontmatter before handing to bella.
            let status_md = strip_frontmatter(&raw_md).to_owned();
            let theme = crate::ui_theme::to_bella_theme(crate::ui_theme::current_theme());
            let rendered = render_cache.get_or_render(
                &file_path,
                &status_md,
                content_area.width.saturating_sub(2), // account for borders
                &theme,
                &app.table_expansions,
            );
            // See the matching Tier-branch comment above: without this,
            // `content_table_map` stays `TableMap::default()` for the life of
            // the app and click-to-expand can never resolve a real hit.
            app.content_table_map = rendered.table_map.clone();
            let paragraph = Paragraph::new(rendered.lines)
                .block(content_block)
                .scroll((app.space_overview_scroll, 0));
            frame.render_widget(paragraph, content_area);
        }
    }

    // ── Agent panel strip ────────────────────────────────────────────────────
    // Always-on cross-space "agents · priority" strip, sorted by urgency
    // (Blocked/needs-input first — `session_urgency`/`agent_panel_rows`,
    // BA.13.1.1/.2). Renders under every `SelectedNode`.
    let strip_block = crate::ui_theme::themed_block(
        Span::styled(" agents · priority ", crate::ui_theme::title_style()),
        false,
    );
    let strip_list = List::new(build_agent_panel_items(&panel_rows)).block(strip_block);
    frame.render_widget(strip_list, app.pane_areas.agent_panel);

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
/// then delegates to `draw_with_root`. `render_cache` is owned by
/// `run_inner` and threaded through here so it survives across the whole
/// event loop rather than being rebuilt every frame.
fn draw(
    frame: &mut Frame,
    app: &mut AppState,
    list_state: &mut ListState,
    render_cache: &mut RenderCache,
) {
    let root = app.current_space_planning_root();
    draw_with_root(frame, app, list_state, &root, render_cache);
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
            let detection = crate::serve::status::detect::detect(&out);
            s.agent_state = detection.state;
            s.blocked_reason = detection.blocked_reason;
        }
    }
    sessions
}

// ── Action execution helper ───────────────────────────────────────────────────

fn set_tmux_status(app: &mut AppState, verb: &str, name: &str, e: TmuxError) {
    let msg = match degrade_tmux_error(verb, name, e.root_cause()) {
        Degraded::Graceful(m) | Degraded::Fatal(m) => m,
    };
    app.status = Some(msg);
}

fn execute_action(action: Action, app: &mut AppState) {
    match action {
        Action::None | Action::Attach(_) => {
            // Attach is handled in the event loop (needs terminal suspension).
        }
        Action::RefreshOpenWork => {
            // Handled specially in the event loop (needs to own the spawned
            // `Child` across ticks — see `spawn_refresh`/`poll_refresh_child`
            // in `run_inner_with_events_and_refresh`).
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

/// Where `run_inner`'s event loop gets its next input event from.
///
/// Production wires [`CrosstermEvents`], which polls the real terminal.
/// Task 3's non-blocking-spawn test wires a synthetic queue instead, so it
/// can drive the ACTUAL `run_inner_with_events` loop — not a stand-in — while
/// a real child process runs alongside it, proving the non-blocking property
/// belongs to this event loop (AC-3) rather than only to `AppState::on_key`.
trait EventSource {
    /// Poll for the next event, waiting at most `timeout`. `Ok(None)` means
    /// the timeout elapsed with nothing available — the loop's tick path.
    fn poll_next(&mut self, timeout: Duration) -> io::Result<Option<Event>>;
}

/// The real event source: crossterm's terminal input.
struct CrosstermEvents;

impl EventSource for CrosstermEvents {
    fn poll_next(&mut self, timeout: Duration) -> io::Result<Option<Event>> {
        if event::poll(timeout)? {
            Ok(Some(event::read()?))
        } else {
            Ok(None)
        }
    }
}

fn run_inner(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut AppState,
) -> Result<()> {
    let mut refresh_child: Option<std::process::Child> = None;
    run_inner_with_events_and_refresh(terminal, app, &mut CrosstermEvents, &mut refresh_child)
}

/// Test/production-shared entry point that always starts with no refresh in
/// flight. `run_inner` (production) and the task-3 non-blocking test both
/// ultimately go through [`run_inner_with_events_and_refresh`] — there is no
/// separate "test loop" that could pass while the real one blocks.
fn run_inner_with_events<E: EventSource>(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut AppState,
    events: &mut E,
) -> Result<()> {
    let mut refresh_child: Option<std::process::Child> = None;
    run_inner_with_events_and_refresh(terminal, app, events, &mut refresh_child)
}

/// The actual event loop body, generic over its [`EventSource`] so it can be
/// driven by tests without a real terminal attached to stdin, and over an
/// externally-owned `refresh_child` slot (BA.26.C task 4) so a test can seed
/// it with a pre-spawned STAND-IN child and observe the SAME poll/re-read
/// machinery production drives, rather than a parallel test-only
/// implementation (see `openwork_refresh_completion_invalidates_and_rereads`
/// below).
fn run_inner_with_events_and_refresh<E: EventSource>(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut AppState,
    events: &mut E,
    refresh_child: &mut Option<std::process::Child>,
) -> Result<()> {
    let mut list_state = ListState::default();
    let mut render_cache = RenderCache::default();

    loop {
        // Poll any in-flight open-work refresh WITHOUT blocking, before this
        // tick's draw — so a refresh that finished between ticks is reflected
        // in the very next frame, both in the footer status (AC-2) and in the
        // content pane, which re-reads its document from disk on every draw
        // regardless (see `read_document` call sites above) and therefore
        // picks up the regenerated file the moment this poll observes the
        // child has exited.
        poll_refresh_child(app, refresh_child);

        terminal.draw(|f| draw(f, app, &mut list_state, &mut render_cache))?;

        if let Some(event) = events.poll_next(Duration::from_millis(REFRESH_MS))? {
            // Click-to-select and wheel-scroll routing (BA.13.2) share the same
            // action-handling path as key events below; sub-tab-bar click
            // routing is deferred to BA.13.4.
            let action = match event {
                Event::Key(k) => Some(app.on_key(k.code)),
                Event::Mouse(m) => Some(app.on_mouse(m)),
                _ => None,
            };

            if let Some(action) = action {
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

                if let Action::RefreshOpenWork = action {
                    // Non-blocking: `spawn_argv`/`Command::spawn` return as
                    // soon as the child is forked/exec'd (AC-3) — this adds no
                    // synchronization of its own, so the loop continues to
                    // its next tick immediately.
                    spawn_refresh(app, refresh_child);
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

/// Poll an in-flight open-work refresh child WITHOUT blocking
/// (`Child::try_wait`), classify its result on completion, and clear the
/// slot. Called once per event-loop tick regardless of whether an input
/// event arrived this tick (AC-3's "the loop must not have blocked" — a
/// non-blocking poll on a timeout tick is not blocking on the child).
fn poll_refresh_child(app: &mut AppState, refresh_child: &mut Option<std::process::Child>) {
    let Some(child) = refresh_child.as_mut() else {
        return;
    };
    match child.try_wait() {
        Ok(Some(status)) => {
            // `ExitStatus::code()` is `None` only on Unix when the process
            // was killed by a signal rather than exiting — there is no exit
            // code to classify in that case, so it is threaded through as
            // `-1` (matches `classify_exit_code`'s "negative code" test,
            // which documents this as a real signal-derived shape rather
            // than a made-up sentinel).
            let code = status.code().unwrap_or(-1);
            app.openwork_status = OpenWorkStatus::Done(crate::openwork::classify_exit_code(code));
            *refresh_child = None;
        }
        Ok(None) => {
            // Still running — `openwork_status` already reads `Refreshing`
            // (set by `AppState::on_key` the moment the key was pressed).
        }
        Err(e) => {
            app.openwork_status = OpenWorkStatus::Done(crate::openwork::classify_spawn_error(&e));
            *refresh_child = None;
        }
    }
}

/// Spawn the open-work refresh subprocess (BA.26.C task 4's key-bound
/// action) into `refresh_child`. Resolves the HQ root the same way every
/// other cross-tree read in this module does — `load_brain_toml_path`'s
/// parent (mirrors the `Tier` branch in `draw_with_root` above). Always
/// `RefreshMode::CheckOnly` — the only mode this action ever uses; AC-1's
/// argv builder has no representation for `--commit`/`--emit` regardless.
fn spawn_refresh(app: &mut AppState, refresh_child: &mut Option<std::process::Child>) {
    let hq_root = crate::config::load_brain_toml_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let argv = crate::openwork::refresh_args(crate::openwork::RefreshMode::CheckOnly, &hq_root);
    match crate::openwork::spawn_argv(&argv) {
        Ok(child) => *refresh_child = Some(child),
        Err(e) => {
            app.openwork_status = OpenWorkStatus::Done(crate::openwork::classify_spawn_error(&e));
        }
    }
}

/// Resolve the active theme from the on-disk config (DB-free — see D4) and
/// initialize the process-wide runtime theme so chrome and the markdown view
/// (`render_with_edit`) share one palette. A missing or unreadable config
/// degrades gracefully to the `bastion` default; a **malformed** config also
/// degrades to the default (never panics, never refuses to boot) but is no
/// longer silently indistinguishable from an absent one — the returned
/// `Some(message)` names the config path and the parser's own error so the
/// caller can surface it (BA.26.A task 2; previously `.unwrap_or_default()`
/// discarded `ConfigError::MalformedFile` here entirely).
fn init_theme_from_config() -> Option<String> {
    let xdg = std::env::var("XDG_CONFIG_HOME").ok();
    let home = std::env::var("HOME").ok();
    let path = crate::config::config_path(xdg.clone(), home.clone());
    match crate::config::load_workspace_registry(xdg, home) {
        Ok(file) => {
            crate::ui_theme::init_theme(crate::config::resolve_theme(&file));
            None
        }
        Err(e) => {
            // Still degrade to the default theme — a malformed config must
            // never panic or block boot — but, unlike before, do not throw
            // the error away: report it via `path` so the caller can surface
            // it to the operator instead of silently reverting.
            crate::ui_theme::init_theme(crate::config::resolve_theme(
                &crate::config::FileConfig::default(),
            ));
            let result: Result<crate::config::FileConfig, crate::config::ConfigError> = Err(e);
            path.and_then(|p| crate::config::describe_config_load_error(&p, &result))
        }
    }
}

/// Launch the interactive session dashboard (synchronous; no tokio).
pub fn run() -> Result<()> {
    let theme_degradation = init_theme_from_config();

    let mut stdout = io::stdout();
    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, event::EnableMouseCapture)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let space_tree = crate::brain::spaces::load_space_tree(&crate::config::load_brain_toml_path())
        .unwrap_or_default();
    // Resolve the declared `[views]` table (BA.26.A) the same way the theme was
    // just resolved above: absent/unreadable/malformed all degrade to an empty
    // offered-list — never an error, never a panic — since `init_theme_from_config`
    // already surfaced a malformed file via `theme_degradation`.
    let offered_views = crate::config::load_workspace_registry(
        std::env::var("XDG_CONFIG_HOME").ok(),
        std::env::var("HOME").ok(),
    )
    .map(|file| crate::config::offered_views(&file))
    .unwrap_or_default();
    let mut app = AppState::new(poll_sessions(), space_tree).with_offered_views(offered_views);
    // A malformed config file degrades to defaults above, but the operator
    // must still be told — surface it in the same footer status line other
    // degradations use (BA.26.A task 2), rather than leaving it silent.
    if let Some(msg) = theme_degradation {
        app.status = Some(msg);
    }
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
/// without touching the process environment. Builds a fresh `RenderCache` per
/// call — `tui_tests.rs` exercises single-frame draws, not the persist-across-
/// frames behaviour, which the `RenderCache` unit tests below cover directly
/// against `draw_with_root`.
#[cfg(test)]
pub fn draw_for_test(
    frame: &mut ratatui::Frame,
    app: &mut AppState,
    list_state: &mut ratatui::widgets::ListState,
    planning_root: &std::path::Path,
) {
    draw_with_root(
        frame,
        app,
        list_state,
        planning_root,
        &mut RenderCache::default(),
    );
}

// ── Unit tests for pure helpers ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::model::SessionState;

    // ── task 3: non-blocking spawn, asserted against the REAL ui.rs event
    // loop (AC-3) ────────────────────────────────────────────────────────

    /// A synthetic `EventSource` that yields a fixed queue of events. Every
    /// test using it queues a quitting key, so the loop it drives always
    /// terminates on its own rather than relying on the exhausted-queue
    /// (timeout) path looping forever.
    struct QueueEvents(std::collections::VecDeque<Event>);

    impl EventSource for QueueEvents {
        fn poll_next(&mut self, _timeout: Duration) -> io::Result<Option<Event>> {
            Ok(self.0.pop_front())
        }
    }

    /// AC-3, and the AC is explicit about how this must be tested: a faked
    /// in-flight boolean proves only that the key handler is reachable when
    /// a bool is set, and passes IDENTICALLY if the real spawn blocks. This
    /// test instead spawns a REAL stand-in child (`sleep`, never
    /// `refresh.py` — whose measured no-op `--check` path is 21.7 s) that
    /// outlives the render tick, and drives the ACTUAL
    /// `run_inner_with_events` event loop (not `app.rs`'s `on_key` in
    /// isolation — `run_inner` at src/sessions/ui.rs is the event loop this
    /// AC's non-blocking property belongs to) through a synthetic
    /// `EventSource` while that child is still alive.
    ///
    /// Two ordering assertions, never a wall-clock duration:
    /// 1. `spawn_argv` returns before the long-lived stand-in child has
    ///    exited.
    /// 2. The event loop processes a key (and quits) while that same child
    ///    is STILL running — proving the loop never blocked on the spawn.
    #[test]
    fn run_inner_event_loop_stays_responsive_while_a_real_child_is_in_flight() {
        // The child must outlive the render tick (`REFRESH_MS`); give it
        // comfortable headroom above it.
        let sleep_secs = (REFRESH_MS / 1000) + 2;
        let mut child = crate::openwork::spawn_argv(&["sleep".to_string(), sleep_secs.to_string()])
            .expect("spawn stand-in 'sleep' child");

        // Assertion 1: the spawning call already returned — prove the child
        // has not exited yet (it cannot have: it sleeps for several seconds).
        assert!(
            child
                .try_wait()
                .expect("try_wait on freshly spawned child")
                .is_none(),
            "stand-in child must still be running immediately after spawn_argv returns"
        );

        // Drive the REAL event loop with one synthetic key event that quits
        // — this exercises `run_inner_with_events` end to end, including
        // `terminal.draw`, exactly as `run_inner` does in production.
        let mut app = make_app(&[]);
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend).expect("crossterm terminal for test");
        let mut events = QueueEvents(std::collections::VecDeque::from([Event::Key(
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('q'),
                crossterm::event::KeyModifiers::NONE,
            ),
        )]));

        run_inner_with_events(&mut terminal, &mut app, &mut events)
            .expect("run_inner_with_events must not error");

        assert!(app.should_quit, "the 'q' key must have been processed");

        // Assertion 2: the app already finished processing the key (the
        // loop above returned) while the stand-in child is STILL running —
        // proving the loop never blocked waiting on the child.
        assert!(
            child
                .try_wait()
                .expect("try_wait after the event loop returns")
                .is_none(),
            "stand-in child must still be running after the event loop processed a key \
             and quit — the loop must not have blocked on the spawn"
        );

        // Clean up the stand-in child rather than leaving it sleeping out
        // its full duration as an orphan.
        let _ = child.kill();
        let _ = child.wait();
    }

    // ── task 4: refresh completion invalidates and re-reads (AC-5, first
    // half) ─────────────────────────────────────────────────────────────

    /// AC-5, first half: when a refresh FINISHES, the pane must show the NEW
    /// content — "the console stays responsive" (task 3) is satisfied
    /// perfectly by a build that never re-reads, which is why this is a
    /// separate assertion. Runs a REAL stand-in child (`sh`, never
    /// `refresh.py` — its measured no-op `--check` path is 21.7 s) that
    /// rewrites the displayed file, polls it to completion through the
    /// PRODUCTION `poll_refresh_child` (the exact function the ui.rs event
    /// loop calls once per tick), then redraws through the PRODUCTION
    /// `draw_with_root` and asserts the RENDERED CONTENT changed — not
    /// merely that `openwork_status` flipped to `Done`. (Driving the full
    /// `run_inner_with_events_and_refresh` loop here would need a
    /// `TestBackend`-typed terminal, but that loop is typed over the real
    /// `CrosstermBackend` so its `Attach` branch can suspend/restore an
    /// actual terminal — task 3's non-blocking test covers that loop's
    /// scheduling behaviour with a real `Terminal`; this test covers the
    /// poll+redraw content pipeline that loop calls into, directly.)
    #[test]
    fn openwork_refresh_completion_invalidates_and_rereads() {
        use ratatui::{Terminal, backend::TestBackend};

        let dir = crate::testsupport::unique_temp_dir("bastion-openwork-refresh-test");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let status_path = dir.join("status.md");
        std::fs::write(&status_path, "# OLD board content\n").expect("write initial status.md");

        // Hq-selected app — mirrors `hq_space_overview_render_hides_html_sentinel_comments`
        // above: a `"_root"`-tagged tier routes `selected_node()` to `Hq`,
        // whose content pane defaults to `<planning_root>/status.md` when
        // `space_overview_file` is `None`.
        let mut tree = crate::brain::spaces::SpaceTree::default();
        tree.tiers.push(("_root".to_string(), vec![]));
        let mut app = AppState::new(vec![], tree);
        app.selected_spine = 1;
        assert_eq!(
            app.selected_node(),
            crate::brain::spaces::SelectedNode::Hq,
            "selected_spine=1 must route to Hq"
        );

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("TestBackend terminal");
        let mut list_state = ListState::default();

        // Sanity: the pane shows the OLD content before any refresh runs.
        terminal
            .draw(|f| {
                draw_with_root(
                    f,
                    &mut app,
                    &mut list_state,
                    &dir,
                    &mut RenderCache::default(),
                )
            })
            .expect("first draw must not panic");
        let before = buf_to_string(&terminal.backend().buffer().clone());
        assert!(
            before.contains("OLD board content"),
            "expected OLD content before any refresh: {before}"
        );

        // Mirror `AppState::on_key`'s side effect of pressing 'r'.
        app.openwork_status = OpenWorkStatus::Refreshing;

        // A REAL stand-in child that rewrites `status.md` — simulating a
        // completed refresh's non-atomic truncating write of the exact file
        // this pane reads.
        let mut refresh_child = Some(
            crate::openwork::spawn_argv(&[
                "sh".to_string(),
                "-c".to_string(),
                format!(
                    "printf '# NEW board content\\n' > {}",
                    status_path.display()
                ),
            ])
            .expect("spawn stand-in refresh child"),
        );

        // Poll via the PRODUCTION `poll_refresh_child` — the exact function
        // `run_inner_with_events_and_refresh` calls once per tick — until
        // the stand-in child completes. `try_wait` never blocks, so this
        // loop only busy-polls with a short real sleep between attempts; it
        // does not depend on any particular render-tick cadence.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while refresh_child.is_some() {
            poll_refresh_child(&mut app, &mut refresh_child);
            if refresh_child.is_some() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "stand-in child never completed within 5s"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        assert_eq!(
            app.openwork_status,
            OpenWorkStatus::Done(crate::openwork::RefreshOutcome::Current),
            "the stand-in child exits 0, so the classified result must be Current"
        );

        // Redraw through the SAME production `draw_with_root` path and
        // assert the content pane picked up the rewritten file.
        terminal
            .draw(|f| {
                draw_with_root(
                    f,
                    &mut app,
                    &mut list_state,
                    &dir,
                    &mut RenderCache::default(),
                )
            })
            .expect("second draw must not panic");
        let after = buf_to_string(&terminal.backend().buffer().clone());
        assert!(
            after.contains("NEW board content"),
            "pane must show the NEW content once the refresh completes: {after}"
        );
        assert!(
            !after.contains("OLD board content"),
            "pane must not still show the stale OLD content after refresh completion: {after}"
        );
        assert!(
            after.contains("open-work boards refreshed"),
            "footer must render the classified result, not just the content pane: {after}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A spawn failure (program not found) must classify as `SpawnFailed`
    /// and render its OS-level reason in the footer, distinct from an
    /// exit-code-derived `Failed`. `spawn_refresh` (the production caller)
    /// always builds argv against the real HQ root/refresh.py — this
    /// exercises the exact classification path it applies on a spawn error
    /// (`classify_spawn_error`), and the exact rendering path
    /// (`openwork_status_message`) the footer uses for it, over a program
    /// guaranteed not to exist rather than depending on the test sandbox's
    /// HQ layout.
    #[test]
    fn spawn_refresh_failure_sets_spawn_failed_status_with_reason() {
        let mut app = AppState::new(vec![], crate::brain::spaces::SpaceTree::default());

        let err = crate::openwork::spawn_argv(&["definitely-not-a-real-binary-xyz".to_string()])
            .expect_err("spawning a nonexistent program must error");
        app.openwork_status = OpenWorkStatus::Done(crate::openwork::classify_spawn_error(&err));

        match &app.openwork_status {
            OpenWorkStatus::Done(crate::openwork::RefreshOutcome::SpawnFailed { reason }) => {
                assert!(!reason.is_empty());
            }
            other => panic!("expected Done(SpawnFailed), got {other:?}"),
        }
        let msg = openwork_status_message(&app.openwork_status)
            .expect("Done status must render a footer message");
        assert!(
            msg.contains("failed to start"),
            "spawn failure message must be distinguishable from an exit-code failure: {msg}"
        );
    }

    // ── read_document / render_document_markdown (BA.26.B task 7,
    // "CONCURRENCY WITH REFRESH") ───────────────────────────────────────────
    // Asserts on the `DocumentRead` state VALUE, not on rendered placeholder
    // text — a string assertion would keep passing after someone edits the
    // wording, which is exactly the failure mode this criterion exists to
    // prevent. Absent and failed are exercised in the SAME test so the two
    // are provably distinguishable rather than merely each individually
    // non-panicking.

    #[test]
    fn read_document_distinguishes_absent_from_failed() {
        let tmp = tempfile::tempdir().expect("tempdir");

        // Absent: the ordinary "nothing here yet" case.
        let absent_path = tmp.path().join("does-not-exist.md");
        assert_eq!(read_document(&absent_path), DocumentRead::Absent);

        // Failed/torn: a real io error that is NOT "not found". A directory
        // can't be read as a file, so `read_to_string` errors with a
        // platform io::ErrorKind other than NotFound — standing in for the
        // torn/racing-writer read this AC targets, without depending on
        // actually winning a real race in a unit test.
        let dir_path = tmp.path().join("a-directory");
        std::fs::create_dir(&dir_path).expect("create_dir");
        match read_document(&dir_path) {
            DocumentRead::Failed { path, kind } => {
                assert_eq!(path, dir_path);
                assert_ne!(kind, "NotFound");
            }
            other => panic!("expected DocumentRead::Failed for a directory path, got {other:?}"),
        }

        // The two outcomes must be distinct states, not the same value
        // reached two different ways.
        assert_ne!(read_document(&absent_path), read_document(&dir_path));
    }

    #[test]
    fn read_document_ok_on_successful_read() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let file_path = tmp.path().join("status.md");
        std::fs::write(&file_path, "# Hello\n").expect("write");
        assert_eq!(
            read_document(&file_path),
            DocumentRead::Ok("# Hello\n".to_string())
        );
    }

    #[test]
    fn render_document_markdown_absent_uses_the_absent_message_only() {
        let rendered = render_document_markdown(&DocumentRead::Absent, "No status.md found.");
        assert_eq!(rendered, "No status.md found.");
    }

    #[test]
    fn render_document_markdown_failed_never_uses_absent_message_or_content() {
        let failed = DocumentRead::Failed {
            path: std::path::PathBuf::from("/planning/status.md"),
            kind: "Other".to_string(),
        };
        let rendered = render_document_markdown(&failed, "No status.md found.");
        // Must not collapse to the absent-file placeholder...
        assert_ne!(rendered, "No status.md found.");
        // ...and must name the path and the error kind, so a torn read is
        // never silently rendered as ordinary content either.
        assert!(rendered.contains("/planning/status.md"));
        assert!(rendered.contains("Other"));
    }

    #[test]
    fn render_document_markdown_ok_renders_the_content_verbatim() {
        let ok = DocumentRead::Ok("# Real content\n".to_string());
        let rendered = render_document_markdown(&ok, "No status.md found.");
        assert_eq!(rendered, "# Real content\n");
    }

    // ── strip_frontmatter (AC-7, BA.26.B task 6) ──────────────────────────
    // Pins the two behaviours bastion keeps on top of delegating fence
    // detection to `bella_engine::frontmatter::detect_fence`: leading
    // blank-line tolerance before the opening fence, and multi-blank-line
    // consumption after the closing fence. A future bella change to fence
    // detection semantics should surface as a failure here.

    #[test]
    fn strip_frontmatter_removes_fenced_block() {
        let md = "---\ntype: Doc\ntitle: T\n---\n# Body\n";
        assert_eq!(strip_frontmatter(md), "# Body\n");
    }

    #[test]
    fn strip_frontmatter_no_fence_returns_unchanged() {
        let md = "# Body\nNo frontmatter here.\n";
        assert_eq!(strip_frontmatter(md), md);
    }

    #[test]
    fn strip_frontmatter_tolerates_leading_blank_lines() {
        // bastion-specific leniency: real files occasionally carry a stray
        // leading blank line before the fence; `detect_fence` alone would
        // require the very first line to be `---` and miss this.
        let md = "\n\n---\ntype: Doc\n---\nBody text\n";
        assert_eq!(strip_frontmatter(md), "Body text\n");
    }

    #[test]
    fn strip_frontmatter_consumes_multiple_trailing_blank_lines() {
        // bastion-specific leniency: consume every blank line right after
        // the closing fence, not just one, so it never leaks into the body.
        let md = "---\ntype: Doc\n---\n\n\n\nBody text\n";
        assert_eq!(strip_frontmatter(md), "Body text\n");
    }

    #[test]
    fn strip_frontmatter_requires_exact_closing_fence_line() {
        // detect_fence is stricter than the old hand-rolled search: a line
        // that merely CONTAINS "---" as a substring (not the whole line) is
        // not a closing fence, so nothing is stripped and the whole string
        // (unchanged) is returned.
        let md = "---\ntype: Doc\n---not-a-real-fence\nBody\n";
        assert_eq!(strip_frontmatter(md), md);
    }

    fn make_session(name: &str, state: SessionState, last_line: &str) -> Session {
        Session {
            name: name.to_string(),
            state,
            window_count: 1,
            foreground_cmd: String::new(),
            last_line: last_line.to_string(),
            agent_state: crate::detect::AgentState::Unknown,
            blocked_reason: None,
            cwd: String::new(),
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
            blocked_reason: None,
            cwd: String::new(),
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
        assert!(hint.contains("[v]"), "hint: {hint}");
        assert!(hint.contains("[q]"), "hint: {hint}");
        // The expand/collapse toggle (BA.26.B task 2) is the one new
        // keybinding this block introduces — the footer must stay honest
        // about it.
        assert!(hint.contains("[e]"), "hint: {hint}");
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

    // ── openwork_status_message / status_line integration (task 4) ─────────

    #[test]
    fn openwork_status_message_idle_is_none() {
        assert_eq!(openwork_status_message(&OpenWorkStatus::Idle), None);
    }

    #[test]
    fn openwork_status_message_refreshing_is_distinct_from_done() {
        let msg = openwork_status_message(&OpenWorkStatus::Refreshing)
            .expect("Refreshing must render a message");
        assert!(msg.contains("refreshing"));
    }

    /// Each `RefreshOutcome` variant renders a DISTINCT message, and the
    /// failure variants show the REAL exit code / OS error text (AC-2) —
    /// never a generic "it failed".
    #[test]
    fn openwork_status_message_per_outcome_is_distinct_and_carries_failure_detail() {
        use crate::openwork::RefreshOutcome;

        let current =
            openwork_status_message(&OpenWorkStatus::Done(RefreshOutcome::Current)).unwrap();
        let stale =
            openwork_status_message(&OpenWorkStatus::Done(RefreshOutcome::StaleOrChanged)).unwrap();
        let failed_17 =
            openwork_status_message(&OpenWorkStatus::Done(RefreshOutcome::Failed { code: 17 }))
                .unwrap();
        let failed_3 =
            openwork_status_message(&OpenWorkStatus::Done(RefreshOutcome::Failed { code: 3 }))
                .unwrap();
        let spawn_failed =
            openwork_status_message(&OpenWorkStatus::Done(RefreshOutcome::SpawnFailed {
                reason: "No such file or directory".to_string(),
            }))
            .unwrap();

        // All five distinct.
        let all = [&current, &stale, &failed_17, &failed_3, &spawn_failed];
        for (i, a) in all.iter().enumerate() {
            for (j, b) in all.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "messages at {i} and {j} must differ: {all:?}");
                }
            }
        }

        assert!(
            failed_17.contains("17"),
            "must carry the real exit code: {failed_17}"
        );
        assert!(
            failed_3.contains('3'),
            "must carry the real exit code: {failed_3}"
        );
        assert!(
            spawn_failed.contains("No such file or directory"),
            "must carry the real OS error reason: {spawn_failed}"
        );
    }

    /// `status_line` in Normal mode prefers the open-work refresh message
    /// over an ordinary `app.status` line — the refresh state is more
    /// current than a stale status set by an earlier keypress.
    #[test]
    fn status_line_normal_mode_prefers_openwork_status_over_app_status() {
        let mut app = make_app(&[]);
        app.status = Some("some other status".to_string());
        app.openwork_status = OpenWorkStatus::Refreshing;
        let line = status_line(&app);
        assert!(line.contains("refreshing"), "line: {line}");
        assert!(!line.contains("some other status"), "line: {line}");
    }

    /// A refresh finishing while the operator is mid-typed-input must NOT
    /// blank out the `Input` prompt/buffer — `status_line` only overrides in
    /// `Mode::Normal`.
    #[test]
    fn status_line_input_mode_ignores_openwork_status() {
        let mut app = make_app(&[]);
        app.mode = Mode::Input(InputKind::New);
        app.input = "my-session".into();
        app.openwork_status = OpenWorkStatus::Done(crate::openwork::RefreshOutcome::Current);
        let line = status_line(&app);
        assert!(line.contains("my-session"), "line: {line}");
        assert!(!line.contains("refreshed"), "line: {line}");
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
            blocked_reason: None,
            cwd: String::new(),
        };
        let mut app = make_app(&[session]);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("TestBackend terminal");
        let dir = std::env::temp_dir();
        terminal
            .draw(|f| {
                let mut list_state = ratatui::widgets::ListState::default();
                draw_with_root(
                    f,
                    &mut app,
                    &mut list_state,
                    &dir,
                    &mut RenderCache::default(),
                );
            })
            .expect("draw must not panic");
        let buf = terminal.backend().buffer().clone();

        let expected = crate::ui_theme::current_theme().sage;
        let mut found_dot = false;
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                if let Some(cell) = buf.cell((x, y))
                    && cell.symbol() == "●"
                {
                    found_dot = true;
                    assert_eq!(
                        cell.fg, expected,
                        "working-state dot must render with the runtime theme's sage color"
                    );
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

    // ── HTML sentinel comments must not leak into the rendered TUI buffer ──────

    /// Flatten a `TestBackend` buffer to plain text — local equivalent of the
    /// `buf_to_string` helper in `sessions/tui_tests.rs`.
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

    /// bella-engine's `render_with_edit` (BE.4.A) strips HTML comments — including the
    /// sentinel comments mev's `emit_state` now wraps generated sections in — at parse
    /// time. This test confirms that stripping is actually wired into bastion's own
    /// `Hq`/`Space` render path (via `draw_with_root`), not just covered upstream in
    /// bella's own test suite.
    #[test]
    fn hq_space_overview_render_hides_html_sentinel_comments() {
        use ratatui::{Terminal, backend::TestBackend};

        let dir = crate::testsupport::unique_temp_dir("bastion-ui-sentinel-test");
        std::fs::create_dir_all(&dir).expect("create temp dir");

        std::fs::write(
            dir.join("status.md"),
            "# Status\n\n\
             <!-- BEGIN generated:momentum -->\n\
             - **now** — sentinel-guarded momentum text\n\
             <!-- END generated:momentum -->\n",
        )
        .expect("write status.md");

        // A `"_root"`-tagged tier renders as the `Hq` spine row (mirrors the pattern
        // in `sessions/tui_tests.rs`'s `app_with_hq_and_tier`); the local `make_app`
        // helper above tags its tier `"core"`, which would select `Tier`, not `Hq`.
        let mut tree = crate::brain::spaces::SpaceTree::default();
        tree.tiers.push(("_root".to_string(), vec![]));
        let mut app = AppState::new(vec![], tree);
        app.selected_spine = 1;
        assert_eq!(
            app.selected_node(),
            crate::brain::spaces::SelectedNode::Hq,
            "selected_spine=1 must route to Hq"
        );

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("TestBackend terminal");
        terminal
            .draw(|f| {
                let mut list_state = ratatui::widgets::ListState::default();
                draw_with_root(
                    f,
                    &mut app,
                    &mut list_state,
                    &dir,
                    &mut RenderCache::default(),
                );
            })
            .expect("draw must not panic");

        let buf = terminal.backend().buffer().clone();
        let text = buf_to_string(&buf);

        assert!(
            !text.contains("<!--"),
            "buffer must not contain '<!--': {text}"
        );
        assert!(
            !text.contains("-->"),
            "buffer must not contain '-->': {text}"
        );
        assert!(
            !text.contains("generated:momentum"),
            "buffer must not contain the sentinel's inner text: {text}"
        );
        // Sanity: the ordinary Momentum content around the sentinels did render
        // (word-wrapping in the narrow content pane may split the bullet across
        // rendered lines, so check for an unbroken word rather than the full phrase).
        assert!(
            text.contains("sentinel-guarded"),
            "expected the surrounding Momentum bullet text to render: {text}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Table expansion persistence (BA.26.B task 1) ────────────────────────
    //
    // The defect: `bella_engine::links::TableExpansions::new()` was constructed
    // fresh inside the draw call at both `render_with_edit` call sites, so
    // `expanded(i)` was always false and every cell took the truncate branch
    // forever. bella already implements the wrap branch and hit-test geometry
    // (`render_table_row`, `TableMap::hit`) — the bug is entirely on bastion's
    // side: it never held the map anywhere that survives a redraw.

    /// Flatten a `Vec<Line>` (as returned by `bella_engine::render_with_edit`)
    /// to plain text for substring assertions, without needing a `Frame`.
    fn rendered_lines_to_string(lines: &[Line]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A markdown source with one table whose single cell is far wider than an
    /// 80-column pane can show untruncated, so the wrap-vs-clip branches are
    /// actually exercised at the width this repo renders at.
    const WIDE_TABLE_MD: &str = "# T\n\n\
        | Col |\n\
        | --- |\n\
        | This cell holds a long run of prose text that is deliberately wider \
          than an eighty column pane so that truncation or wrapping has \
          something real to do once the table is laid out at that width |\n";

    /// AC-1, SHOWN FAILING pattern: render the same table twice at the same
    /// 80-column-pane width — once through a freshly-constructed
    /// `TableExpansions` (the pre-change behaviour: always clips with an
    /// ellipsis) and once through a map with that table's one cell marked
    /// expanded (wraps to multiple lines instead). Asserting both directions
    /// in one test is what proves the test can actually tell them apart.
    #[test]
    fn expanded_cell_wraps_fresh_map_clips() {
        let theme = crate::ui_theme::to_bella_theme(crate::ui_theme::current_theme());
        let width: u16 = 78; // matches an 80-col pane minus the 2-col border allowance

        let fresh = bella_engine::links::TableExpansions::new();
        let clipped =
            bella_engine::render_with_edit(WIDE_TABLE_MD, None, width, &theme, None, &fresh);
        let clipped_text = rendered_lines_to_string(&clipped.lines);
        assert!(
            clipped_text.contains('…'),
            "a freshly-constructed TableExpansions (pre-change behaviour) must still clip \
             with an ellipsis at this width: {clipped_text}"
        );

        let id = clipped
            .table_map
            .regions
            .first()
            .expect("WIDE_TABLE_MD must lay out exactly one table region")
            .id;
        let mut expanded_map = bella_engine::links::TableExpansions::new();
        expanded_map.insert(
            id,
            bella_engine::links::TableExpand {
                all: false,
                cols: Default::default(),
                cells: std::iter::once((0usize, 0usize)).collect(),
            },
        );
        let wrapped =
            bella_engine::render_with_edit(WIDE_TABLE_MD, None, width, &theme, None, &expanded_map);
        let wrapped_text = rendered_lines_to_string(&wrapped.lines);
        assert!(
            !wrapped_text.contains('…'),
            "an expanded cell must wrap rather than clip: {wrapped_text}"
        );
        assert!(
            wrapped.lines.len() > clipped.lines.len(),
            "wrapping an expanded cell must add display lines versus the clipped render \
             (clipped={}, wrapped={})",
            clipped.lines.len(),
            wrapped.lines.len()
        );
    }

    /// AC-2 (the actual defect this task fixes): expand a cell through the
    /// `AppState`-held map, force `draw_with_root` to re-render twice, and
    /// assert the expansion is still in effect both times. A test that builds
    /// its own `TableExpansions` locally does not exercise this — bella's
    /// byte-offset keying already survives a re-render; what did not survive
    /// is bastion discarding the map inside the draw call.
    #[test]
    fn table_expansion_survives_rerender_via_app_state() {
        use ratatui::{Terminal, backend::TestBackend};

        let dir = crate::testsupport::unique_temp_dir("bastion-ui-table-expand-test");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        std::fs::write(dir.join("status.md"), WIDE_TABLE_MD).expect("write status.md");

        // A `"_root"`-tagged tier routes to the `Hq` spine row, whose content
        // pane reads `<planning_root>/status.md` (mirrors the sentinel test
        // above).
        let mut tree = crate::brain::spaces::SpaceTree::default();
        tree.tiers.push(("_root".to_string(), vec![]));
        let mut app = AppState::new(vec![], tree);
        app.selected_spine = 1;
        assert_eq!(
            app.selected_node(),
            crate::brain::spaces::SelectedNode::Hq,
            "selected_spine=1 must route to Hq"
        );

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("TestBackend terminal");

        // First draw: no expansion set yet — must clip (pre-change behaviour),
        // and it also gives us the exact content-pane width `draw_with_root`
        // computed, so the table id we key off matches production exactly.
        terminal
            .draw(|f| {
                let mut list_state = ratatui::widgets::ListState::default();
                draw_with_root(
                    f,
                    &mut app,
                    &mut list_state,
                    &dir,
                    &mut RenderCache::default(),
                );
            })
            .expect("draw must not panic");
        let first_buf = terminal.backend().buffer().clone();
        assert!(
            buf_to_string(&first_buf).contains('…'),
            "unexpanded first draw must clip"
        );

        let theme = crate::ui_theme::to_bella_theme(crate::ui_theme::current_theme());
        let content_width = app.pane_areas.content.width.saturating_sub(2);
        let stripped = strip_frontmatter(WIDE_TABLE_MD).to_owned();
        let baseline = bella_engine::render_with_edit(
            &stripped,
            None,
            content_width,
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

        // Re-render TWICE with no further mutation of `table_expansions` —
        // proving the map is read from persisted `AppState`, not rebuilt.
        for attempt in 0..2 {
            terminal
                .draw(|f| {
                    let mut list_state = ratatui::widgets::ListState::default();
                    draw_with_root(
                        f,
                        &mut app,
                        &mut list_state,
                        &dir,
                        &mut RenderCache::default(),
                    );
                })
                .expect("draw must not panic");
            let buf = terminal.backend().buffer().clone();
            let text = buf_to_string(&buf);
            assert!(
                !text.contains('…'),
                "expansion must still be in effect on re-render #{attempt}: {text}"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── RenderCache: pure scroll must not re-parse (BA.26.B task 8) ────────

    /// Two `TableExpansions` built with the same entries inserted in a
    /// different order must fingerprint identically — `HashMap` iteration
    /// order is not stable, so a naive "hash whatever order `.iter()` gives"
    /// implementation would flap between runs and defeat the cache on pure
    /// noise.
    #[test]
    fn table_expansions_fingerprint_is_order_independent() {
        let mut a = bella_engine::links::TableExpansions::new();
        a.insert(
            1,
            bella_engine::links::TableExpand {
                all: false,
                cols: [2usize, 5usize].into_iter().collect(),
                cells: Default::default(),
            },
        );
        a.insert(
            9,
            bella_engine::links::TableExpand {
                all: true,
                cols: Default::default(),
                cells: [(0usize, 0usize)].into_iter().collect(),
            },
        );

        let mut b = bella_engine::links::TableExpansions::new();
        b.insert(
            9,
            bella_engine::links::TableExpand {
                all: true,
                cols: Default::default(),
                cells: [(0usize, 0usize)].into_iter().collect(),
            },
        );
        b.insert(
            1,
            bella_engine::links::TableExpand {
                all: false,
                cols: [5usize, 2usize].into_iter().collect(),
                cells: Default::default(),
            },
        );

        assert_eq!(
            table_expansions_fingerprint(&a),
            table_expansions_fingerprint(&b),
            "same entries inserted in a different order must fingerprint the same"
        );
    }

    /// A fingerprint must actually change when the expansion state changes —
    /// otherwise `RenderCache` would silently serve a stale render across a
    /// table toggle, which is worse than never caching at all.
    #[test]
    fn table_expansions_fingerprint_changes_with_expansion_state() {
        let empty = bella_engine::links::TableExpansions::new();
        let mut expanded = bella_engine::links::TableExpansions::new();
        expanded.insert(
            1,
            bella_engine::links::TableExpand {
                all: true,
                cols: Default::default(),
                cells: Default::default(),
            },
        );

        assert_ne!(
            table_expansions_fingerprint(&empty),
            table_expansions_fingerprint(&expanded),
            "toggling a table's expansion must change the fingerprint"
        );
    }

    /// AC-8's gated stand-in: render the same document at the same width
    /// through the same `RenderCache` twice, with only
    /// `AppState::space_overview_scroll` different between the two draws — a
    /// pure scroll, exactly what `Paragraph::scroll` exists to handle without
    /// touching the underlying `Rendered` at all. Asserts `RenderCache::parses`
    /// is `1` after both draws: the second draw is a cache hit, not a second
    /// `bella_engine::render_with_edit` call, so the document is not
    /// re-parsed.
    #[test]
    fn pure_scroll_does_not_reparse_the_document() {
        use ratatui::{Terminal, backend::TestBackend};

        let dir = crate::testsupport::unique_temp_dir("bastion-ui-render-cache-scroll-test");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        // A document with enough lines that a scroll is a meaningful, distinct
        // viewport rather than a no-op against a document shorter than the pane.
        let long_md: String = (0..200)
            .map(|i| format!("- line {i}\n"))
            .collect::<String>();
        std::fs::write(dir.join("status.md"), &long_md).expect("write status.md");

        let mut tree = crate::brain::spaces::SpaceTree::default();
        tree.tiers.push(("_root".to_string(), vec![]));
        let mut app = AppState::new(vec![], tree);
        app.selected_spine = 1;
        assert_eq!(
            app.selected_node(),
            crate::brain::spaces::SelectedNode::Hq,
            "selected_spine=1 must route to Hq"
        );

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("TestBackend terminal");
        let mut render_cache = RenderCache::default();

        // First draw at scroll=0.
        terminal
            .draw(|f| {
                let mut list_state = ratatui::widgets::ListState::default();
                draw_with_root(f, &mut app, &mut list_state, &dir, &mut render_cache);
            })
            .expect("draw must not panic");
        assert_eq!(
            render_cache.parses, 1,
            "the first draw of a never-before-seen document must parse exactly once"
        );

        // A pure scroll: nothing else about the document, width, or table
        // expansion state changes.
        app.space_overview_scroll = 5;

        terminal
            .draw(|f| {
                let mut list_state = ratatui::widgets::ListState::default();
                draw_with_root(f, &mut app, &mut list_state, &dir, &mut render_cache);
            })
            .expect("draw must not panic");
        assert_eq!(
            render_cache.parses, 1,
            "a pure scroll (space_overview_scroll changed, nothing else) must be a cache \
             hit — the document must not be re-parsed a second time"
        );

        // Sanity: a genuine content change (a different document) DOES bump
        // the cache — proves `parses == 1` above is a real hit, not a broken
        // counter that never increments.
        std::fs::write(dir.join("status.md"), format!("{long_md}- one more line\n"))
            .expect("rewrite status.md");
        terminal
            .draw(|f| {
                let mut list_state = ratatui::widgets::ListState::default();
                draw_with_root(f, &mut app, &mut list_state, &dir, &mut render_cache);
            })
            .expect("draw must not panic");
        assert_eq!(
            render_cache.parses, 2,
            "a genuine content change must invalidate the cache and re-parse"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Review fix (AC-3): a real `draw_with_root` over a document containing
    /// a markdown table must leave `AppState::content_table_map` populated
    /// with that table's hit-test geometry — not the `TableMap::default()`
    /// it starts as. Before this fix, neither content-pane call site wrote
    /// `render_cache.get_or_render(...)`'s `Rendered::table_map` back onto
    /// `app.content_table_map`, so `handle_click`'s `content_table_map.hit`
    /// could never resolve a real hit in production (only in tests that hand
    /// it a synthetic `sample_table_map()` directly). Covers both content-pane
    /// call sites: the `Hq`/`Space`/`View` branch (asserted here) and the
    /// `Tier` branch (asserted via the second draw below).
    #[test]
    fn draw_populates_content_table_map_from_a_real_render() {
        use ratatui::{Terminal, backend::TestBackend};

        let dir = crate::testsupport::unique_temp_dir("bastion-ui-content-table-map-test");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let table_md = "| a | b |\n| --- | --- |\n| 1 | 2 |\n";
        std::fs::write(dir.join("status.md"), table_md).expect("write status.md");

        let mut tree = crate::brain::spaces::SpaceTree::default();
        tree.tiers.push(("_root".to_string(), vec![]));
        let mut app = AppState::new(vec![], tree);
        app.selected_spine = 1;
        assert_eq!(
            app.selected_node(),
            crate::brain::spaces::SelectedNode::Hq,
            "selected_spine=1 must route to Hq"
        );
        assert!(
            app.content_table_map.regions.is_empty(),
            "content_table_map must start empty (TableMap::default())"
        );

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("TestBackend terminal");
        let mut render_cache = RenderCache::default();

        terminal
            .draw(|f| {
                let mut list_state = ratatui::widgets::ListState::default();
                draw_with_root(f, &mut app, &mut list_state, &dir, &mut render_cache);
            })
            .expect("draw must not panic");

        assert!(
            !app.content_table_map.regions.is_empty(),
            "draw_with_root over a document with a real table must populate \
             app.content_table_map from Rendered::table_map, not leave it \
             at TableMap::default()"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
