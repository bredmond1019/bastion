// ratatui render functions: two-pane layout (graph left, node detail right).
//
// Pure helpers (status_color, status_symbol, format_node_detail, build_graph_lines)
// are separated from the render call so they can be unit-tested without a live
// terminal. The render function wires them together against a real Frame.

use crate::db::workflows::{NodeState, RunStatus};
use crate::monitor::app::App;
use petgraph::graph::NodeIndex;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

// ── Pure helpers ───────────────────────────────────────────────────────────────

/// Map a `RunStatus` to a terminal color.
pub fn status_color(status: &RunStatus) -> Color {
    match status {
        RunStatus::Running => Color::Yellow,
        RunStatus::Success => Color::Green,
        RunStatus::Failed => Color::Red,
        RunStatus::Pending => Color::DarkGray,
    }
}

/// A single-character symbol representing a `RunStatus`.
pub fn status_symbol(status: &RunStatus) -> &'static str {
    match status {
        RunStatus::Running => "~",
        RunStatus::Success => "+",
        RunStatus::Failed => "!",
        RunStatus::Pending => ".",
    }
}

/// Render the detail pane text for a selected `NodeState`.
///
/// Returns owned `Line<'static>` values so the result can be unit-tested
/// without a `Frame`. All optional fields that are `None` produce a
/// placeholder rather than being omitted.
pub fn format_node_detail(node: &NodeState) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Status + symbol
    let sym = status_symbol(&node.status);
    let color = status_color(&node.status);
    lines.push(Line::from(vec![
        Span::raw("Status: "),
        Span::styled(
            format!("{sym} {:?}", node.status),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ]));

    lines.push(Line::from(""));

    // Timing
    lines.push(Line::from(format!(
        "Started:  {}",
        node.started_at.as_deref().unwrap_or("—")
    )));
    lines.push(Line::from(format!(
        "Elapsed:  {}",
        node.elapsed_secs
            .map(|s| format!("{s}s"))
            .as_deref()
            .unwrap_or("—")
    )));

    lines.push(Line::from(""));

    // Model + tokens
    lines.push(Line::from(format!(
        "Model:    {}",
        node.model.as_deref().unwrap_or("—")
    )));
    lines.push(Line::from(format!(
        "Tokens ↑: {}",
        node.tokens_in
            .map(|t| t.to_string())
            .as_deref()
            .unwrap_or("—")
    )));
    lines.push(Line::from(format!(
        "Tokens ↓: {}",
        node.tokens_out
            .map(|t| t.to_string())
            .as_deref()
            .unwrap_or("—")
    )));

    // Error (only shown when present)
    if let Some(err) = &node.error {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::raw("Error:    "),
            Span::styled(err.clone(), Style::default().fg(Color::Red)),
        ]));
    }

    lines.push(Line::from(""));

    // Input (truncated to avoid overwhelming the pane)
    let input_text = node
        .input
        .as_ref()
        .map(|v| {
            let s = v.to_string();
            if s.len() > 200 {
                format!("{}…", &s[..200])
            } else {
                s
            }
        })
        .unwrap_or_else(|| "—".to_string());
    lines.push(Line::from("Input:"));
    lines.push(Line::from(input_text));

    lines.push(Line::from(""));

    // Output (truncated)
    let output_text = node
        .output
        .as_ref()
        .map(|v| {
            let s = v.to_string();
            if s.len() > 200 {
                format!("{}…", &s[..200])
            } else {
                s
            }
        })
        .unwrap_or_else(|| "—".to_string());
    lines.push(Line::from("Output:"));
    lines.push(Line::from(output_text));

    lines
}

/// Build text lines for the graph pane from the current `App` state.
///
/// Lays out nodes in a grid: column = depth level, row = position within
/// the column (from `GraphLayout.positions`). The selected node is
/// highlighted with an inverted background. Nodes not yet run appear in
/// gray; live status colors are applied from `GraphLayout.node_states`.
///
/// Returns owned `Line<'static>` values for testability.
pub fn build_graph_lines(app: &App) -> Vec<Line<'static>> {
    let Some(layout) = &app.layout else {
        return vec![Line::from("Loading graph…")];
    };

    if layout.positions.is_empty() {
        return vec![Line::from("Graph is empty")];
    }

    let selected_name = app.selected_node().map(|n| n.name.clone());

    // Build lookup: (col, row) → (name, status)
    let mut grid: std::collections::HashMap<(u16, u16), (String, Option<RunStatus>)> =
        std::collections::HashMap::new();

    for &(node_idx, col, row) in &layout.positions {
        let idx = NodeIndex::new(node_idx);
        let name = layout.graph[idx].clone();
        let status = layout.node_states.get(&name).cloned();
        grid.insert((col, row), (name, status));
    }

    let max_col = layout
        .positions
        .iter()
        .map(|&(_, col, _)| col)
        .max()
        .unwrap_or(0);
    let max_row = layout
        .positions
        .iter()
        .map(|&(_, _, row)| row)
        .max()
        .unwrap_or(0);

    const CELL_WIDTH: usize = 20;
    let mut lines: Vec<Line<'static>> = Vec::new();

    for row in 0..=max_row {
        let mut spans: Vec<Span<'static>> = Vec::new();
        for col in 0..=max_col {
            if col > 0 {
                spans.push(Span::raw("  "));
            }
            if let Some((name, status)) = grid.get(&(col, row)) {
                let sym = status.as_ref().map(status_symbol).unwrap_or("?");
                let color = status.as_ref().map(status_color).unwrap_or(Color::DarkGray);
                let is_selected = selected_name.as_deref() == Some(name.as_str());

                // Pad / truncate to CELL_WIDTH
                let label = format!("{sym} {name}");
                let padded: String = if label.len() >= CELL_WIDTH {
                    label[..CELL_WIDTH].to_string()
                } else {
                    format!("{label:<width$}", width = CELL_WIDTH)
                };

                let style = if is_selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(color)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(color)
                };
                spans.push(Span::styled(padded, style));
            } else {
                // Empty cell
                spans.push(Span::raw(" ".repeat(CELL_WIDTH)));
            }
        }
        lines.push(Line::from(spans));
    }

    lines
}

// ── Render ─────────────────────────────────────────────────────────────────────

/// Top-level render function — splits the frame 50/50 into graph (left) and
/// detail (right) panes, then delegates to the pane-specific helpers.
pub fn render(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(frame.area());

    render_graph_pane(frame, chunks[0], app);
    render_detail_pane(frame, chunks[1], app);
}

fn render_graph_pane(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let run_title = app
        .selected_run()
        .map(|r| format!(" {} — {} ", r.workflow_name, r.id))
        .unwrap_or_else(|| " No active runs ".to_string());

    let block = Block::default().title(run_title).borders(Borders::ALL);

    let lines = build_graph_lines(app);
    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(para, area);
}

fn render_detail_pane(frame: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let title = app
        .selected_node()
        .map(|n| format!(" {} ", n.name))
        .unwrap_or_else(|| " Node detail ".to_string());

    let block = Block::default().title(title).borders(Borders::ALL);

    let lines = match app.selected_node() {
        Some(node) => format_node_detail(node),
        None => {
            // No node selected: show run input if available, else placeholder.
            let placeholder = app
                .selected_run()
                .map(|_| "Select a node with ↑ / ↓  or  j / k".to_string())
                .unwrap_or_else(|| "No active workflow runs found.".to_string());
            vec![Line::from(placeholder)]
        }
    };

    // Render banner (errors / status) in the bottom row of the detail pane.
    let inner = if let Some(banner) = &app.banner {
        let [content_area, banner_area] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .areas(block.inner(area));

        let banner_span = Span::styled(banner.clone(), Style::default().fg(Color::Yellow));
        frame.render_widget(Paragraph::new(Line::from(banner_span)), banner_area);
        content_area
    } else {
        block.inner(area)
    };

    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::workflows::{NodeState, RunStatus};

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn make_node_full(name: &str, status: RunStatus) -> NodeState {
        NodeState {
            id: name.to_string(),
            name: name.to_string(),
            status,
            depends_on: vec![],
            input: None,
            output: None,
            error: None,
            tokens_in: None,
            tokens_out: None,
            model: None,
            started_at: None,
            elapsed_secs: None,
        }
    }

    fn lines_to_string(lines: &[Line<'static>]) -> String {
        lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref().to_string()))
            .collect::<Vec<_>>()
            .join("")
    }

    // ── status_color ─────────────────────────────────────────────────────────

    #[test]
    fn status_color_running_is_yellow() {
        assert_eq!(status_color(&RunStatus::Running), Color::Yellow);
    }

    #[test]
    fn status_color_success_is_green() {
        assert_eq!(status_color(&RunStatus::Success), Color::Green);
    }

    #[test]
    fn status_color_failed_is_red() {
        assert_eq!(status_color(&RunStatus::Failed), Color::Red);
    }

    #[test]
    fn status_color_pending_is_dark_gray() {
        assert_eq!(status_color(&RunStatus::Pending), Color::DarkGray);
    }

    // ── status_symbol ─────────────────────────────────────────────────────────

    #[test]
    fn status_symbol_all_variants_are_nonempty() {
        for status in [
            RunStatus::Running,
            RunStatus::Success,
            RunStatus::Failed,
            RunStatus::Pending,
        ] {
            let sym = status_symbol(&status);
            assert!(!sym.is_empty(), "symbol for {status:?} must not be empty");
        }
    }

    #[test]
    fn status_symbol_all_variants_are_distinct() {
        let symbols: Vec<&str> = [
            RunStatus::Running,
            RunStatus::Success,
            RunStatus::Failed,
            RunStatus::Pending,
        ]
        .iter()
        .map(status_symbol)
        .collect();
        let unique: std::collections::HashSet<_> = symbols.iter().collect();
        assert_eq!(
            unique.len(),
            symbols.len(),
            "all status symbols must be distinct"
        );
    }

    // ── format_node_detail ────────────────────────────────────────────────────

    #[test]
    fn format_node_detail_includes_status() {
        let node = make_node_full("TestNode", RunStatus::Success);
        let lines = format_node_detail(&node);
        let text = lines_to_string(&lines);
        assert!(
            text.contains("Status:"),
            "detail must show Status label; got: {text}"
        );
        assert!(
            text.contains("Success"),
            "detail must show status value; got: {text}"
        );
    }

    #[test]
    fn format_node_detail_all_four_status_variants() {
        for (status, expected) in [
            (RunStatus::Running, "Running"),
            (RunStatus::Success, "Success"),
            (RunStatus::Failed, "Failed"),
            (RunStatus::Pending, "Pending"),
        ] {
            let node = make_node_full("N", status);
            let text = lines_to_string(&format_node_detail(&node));
            assert!(
                text.contains(expected),
                "detail for {expected} must contain its name"
            );
        }
    }

    #[test]
    fn format_node_detail_handles_all_none_optional_fields() {
        // All optional fields (started_at, elapsed_secs, model, tokens, input,
        // output, error) are None → must not panic, must render placeholders.
        let node = make_node_full("Empty", RunStatus::Pending);
        let lines = format_node_detail(&node);
        let text = lines_to_string(&lines);
        // All placeholders should appear (em dash)
        assert!(
            text.contains('—'),
            "placeholder dash must appear for None fields"
        );
    }

    #[test]
    fn format_node_detail_includes_tokens_when_present() {
        let mut node = make_node_full("LLMNode", RunStatus::Success);
        node.tokens_in = Some(512);
        node.tokens_out = Some(128);
        node.model = Some("claude-3-5-haiku-20241022".to_string());

        let text = lines_to_string(&format_node_detail(&node));
        assert!(text.contains("512"), "tokens_in must appear");
        assert!(text.contains("128"), "tokens_out must appear");
        assert!(
            text.contains("claude-3-5-haiku-20241022"),
            "model must appear"
        );
    }

    #[test]
    fn format_node_detail_includes_error_when_present() {
        let mut node = make_node_full("FailNode", RunStatus::Failed);
        node.error = Some("connection timeout".to_string());

        let text = lines_to_string(&format_node_detail(&node));
        assert!(
            text.contains("connection timeout"),
            "error message must appear in detail; got: {text}"
        );
    }

    #[test]
    fn format_node_detail_no_error_line_when_error_is_none() {
        let node = make_node_full("OkNode", RunStatus::Success);
        let text = lines_to_string(&format_node_detail(&node));
        assert!(
            !text.contains("Error:"),
            "Error: label must not appear when error is None"
        );
    }

    #[test]
    fn format_node_detail_includes_timing_when_present() {
        let mut node = make_node_full("TimedNode", RunStatus::Success);
        node.started_at = Some("2026-06-21T12:00:00Z".to_string());
        node.elapsed_secs = Some(42);

        let text = lines_to_string(&format_node_detail(&node));
        assert!(
            text.contains("2026-06-21T12:00:00Z"),
            "started_at must appear"
        );
        assert!(text.contains("42s"), "elapsed_secs must appear as Ns");
    }

    // ── build_graph_lines ─────────────────────────────────────────────────────

    #[test]
    fn build_graph_lines_no_layout_shows_loading() {
        let app = crate::monitor::app::App::new();
        let lines = build_graph_lines(&app);
        let text = lines_to_string(&lines);
        assert!(
            text.contains("Loading"),
            "should show loading message when no layout"
        );
    }

    #[test]
    fn build_graph_lines_empty_graph_shows_empty_message() {
        use crate::api::client::WorkflowGraph;
        use crate::db::workflows::WorkflowRun;
        use crate::monitor::graph::build_layout;

        let mut app = crate::monitor::app::App::new();
        let empty_graph = WorkflowGraph {
            nodes: vec![],
            edges: vec![],
        };
        app.layout = Some(build_layout(&empty_graph, &[]));
        app.runs = vec![WorkflowRun {
            id: "r1".into(),
            workflow_name: "wf".into(),
            status: RunStatus::Pending,
            nodes: vec![],
            started_at: None,
            elapsed_secs: None,
        }];

        let lines = build_graph_lines(&app);
        let text = lines_to_string(&lines);
        assert!(
            text.contains("empty"),
            "should show empty graph message; got: {text}"
        );
    }

    #[test]
    fn build_graph_lines_contains_node_names() {
        use crate::api::client::WorkflowGraph;
        use crate::db::workflows::WorkflowRun;
        use crate::monitor::graph::build_layout;

        let graph = WorkflowGraph {
            nodes: vec!["Alpha".into(), "Beta".into()],
            edges: vec![("Alpha".into(), "Beta".into())],
        };
        let nodes_state = vec![
            make_node_full("Alpha", RunStatus::Success),
            make_node_full("Beta", RunStatus::Running),
        ];

        let mut app = crate::monitor::app::App::new();
        app.layout = Some(build_layout(&graph, &nodes_state));
        app.runs = vec![WorkflowRun {
            id: "r1".into(),
            workflow_name: "wf".into(),
            status: RunStatus::Running,
            nodes: nodes_state,
            started_at: None,
            elapsed_secs: None,
        }];

        let lines = build_graph_lines(&app);
        let text = lines_to_string(&lines);
        assert!(text.contains("Alpha"), "Alpha must appear in graph lines");
        assert!(text.contains("Beta"), "Beta must appear in graph lines");
    }

    // ── render (TestBackend) ──────────────────────────────────────────────────

    #[test]
    fn render_produces_two_pane_split() {
        use ratatui::{Terminal, backend::TestBackend};

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let app = crate::monitor::app::App::new();

        terminal.draw(|f| render(f, &app)).unwrap();

        // The left and right pane borders occupy column 0 (left of left pane),
        // column ~40 (right of left pane / left of right pane), and column 79
        // (right of right pane). Verify we can draw without panic and the
        // terminal area is fully covered.
        let buf = terminal.backend().buffer().clone();
        // Top-left corner of the frame must have a border character.
        let top_left = buf.cell((0, 0)).expect("cell(0,0) must exist");
        assert!(
            !top_left.symbol().is_empty(),
            "frame top-left must not be empty"
        );
    }
}
