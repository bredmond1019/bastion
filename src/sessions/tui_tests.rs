// sessions/tui_tests.rs — integration smoke tests for the unified console TUI.
//
// Uses `ratatui::backend::TestBackend` to render the main area at a fixed terminal
// size and asserts no panic occurs and the output buffer contains expected
// content markers.
//
// The planning root path is injected directly into `draw_for_test` so no
// process-environment mutation is needed — these tests are safe to run in
// parallel.
//
// NOTE: navigation is keyed off `selected_spine` / `selected_node()` (the spine
// model — BA.13.0), not the old tab machinery. `tab_bar_contains_all_tab_names`
// is gone with the top tab bar; a spine-sidebar assertion (`draw_for_test`
// asserting `◆ Mission Control` first, selectable tier/HQ headers, no tab bar)
// belongs to BA.13.0.3 (`src/sessions/ui.rs` sidebar rewrite).

#[cfg(test)]
mod tests {
    use crate::brain::spaces::SpaceEntry;
    use crate::sessions::app::AppState;
    use ratatui::{Terminal, backend::TestBackend};
    use std::path::Path;

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Create a fresh `AppState` with no sessions (DB-free, tmux-free) and an
    /// empty space tree — `selected_spine = 0` routes to Mission Control (pinned
    /// first row) in this configuration.
    fn empty_app() -> AppState {
        AppState::new(vec![], crate::brain::spaces::SpaceTree::default())
    }

    /// Build an `AppState` whose spine is `[MissionControl, Hq, Space(learn-ai), Tier(core)]`.
    fn app_with_hq_and_tier() -> AppState {
        let mut tree = crate::brain::spaces::SpaceTree::default();
        tree.tiers.push((
            "_root".to_string(),
            vec![SpaceEntry {
                slug: "learn-ai".to_string(),
                tier: "_root".to_string(),
                repo_path: std::path::PathBuf::from("learn-ai"),
                heading: None,
            }],
        ));
        tree.tiers.push(("core".to_string(), vec![]));
        AppState::new(vec![], tree)
    }

    /// Extract all cell text from the terminal buffer as a single flat string.
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

    /// Build a temporary directory containing minimal planning fixtures.
    ///
    /// Files created:
    ///   `status.md`  — minimal OKF markdown
    ///   `state.json` — minimal valid state.json
    fn write_planning_fixtures(dir: &Path) {
        std::fs::write(
            dir.join("status.md"),
            "# Status\n\n- **now** — smoke test\n",
        )
        .expect("write status.md");

        std::fs::write(
            dir.join("state.json"),
            r#"{"repo":"bastion","kind":"project","updated":"2026-07-01","focus":{"now":[],"next":[],"blocked":[]},"tracks":[],"repos":[],"cross_repo":[],"tiers":[],"note":null,"backlog":[],"carryover":[]}"#,
        )
        .expect("write state.json");
    }

    /// Render a single frame with the given app into a `TestBackend` and return
    /// the terminal so the caller can inspect the buffer.
    fn render_frame(
        app: &AppState,
        planning_root: &Path,
        width: u16,
        height: u16,
    ) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let mut list_state = ratatui::widgets::ListState::default();
                crate::sessions::ui::draw_for_test(f, app, &mut list_state, planning_root);
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    // ── Mission Control (pinned first spine row) ────────────────────────────────

    /// Mission Control renders (no active run) without panic at 80x24.
    #[test]
    fn mission_control_renders_without_panic_80x24() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_planning_fixtures(dir.path());

        let app = empty_app(); // selected_spine = 0 = Mission Control
        render_frame(&app, dir.path(), 80, 24);
    }

    /// Mission Control renders at 120x40 (wider layout).
    #[test]
    fn mission_control_renders_without_panic_120x40() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_planning_fixtures(dir.path());

        let app = empty_app();
        render_frame(&app, dir.path(), 120, 40);
    }

    // ── Hq / Space overview ──────────────────────────────────────────────────────

    /// Selecting the `Hq` spine row renders the Space Overview pane without panic.
    #[test]
    fn hq_space_overview_renders_without_panic() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_planning_fixtures(dir.path());

        let mut app = app_with_hq_and_tier();
        app.selected_spine = 1; // [MissionControl, Hq, Space(learn-ai), Tier(core)]
        assert_eq!(app.selected_node(), crate::brain::spaces::SelectedNode::Hq);

        render_frame(&app, dir.path(), 80, 24);
    }

    /// Space Overview degrade: missing status.md shows fallback text, no panic.
    #[test]
    fn space_overview_missing_status_md_shows_fallback() {
        // Point at an empty directory — no files at all.
        let dir = tempfile::tempdir().expect("tempdir");

        let mut app = app_with_hq_and_tier();
        app.selected_spine = 1; // Hq
        let buf = render_frame(&app, dir.path(), 80, 24);
        let text = buf_to_string(&buf);
        assert!(
            text.contains("planning/status.md"),
            "fallback text must appear for missing status.md; frame:\n{text}"
        );
    }

    // ── Tier header (empty-state degrade) ───────────────────────────────────────

    /// Selecting a tier header renders without panic even when the tier has no
    /// spaces and no `planning/status.md` (graceful empty-state degrade).
    #[test]
    fn tier_header_renders_without_panic() {
        let dir = tempfile::tempdir().expect("tempdir");

        let mut app = app_with_hq_and_tier();
        app.selected_spine = 3; // [MissionControl, Hq, Space(learn-ai), Tier(core)]
        assert_eq!(
            app.selected_node(),
            crate::brain::spaces::SelectedNode::Tier("core".to_string())
        );

        render_frame(&app, dir.path(), 80, 24);
    }

    // ── Layout invariants ─────────────────────────────────────────────────────

    /// At 80x24, the top-left corner must always be occupied (border or content)
    /// regardless of which spine row is selected.
    #[test]
    fn frame_top_left_is_always_occupied() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_planning_fixtures(dir.path());

        let mut app = app_with_hq_and_tier();
        let row_count = app.spine_rows().len();
        for spine_index in 0..row_count {
            app.selected_spine = spine_index;

            let buf = render_frame(&app, dir.path(), 80, 24);
            let cell = buf.cell((0, 0)).expect("cell(0,0) must exist");
            assert!(
                !cell.symbol().is_empty(),
                "top-left cell must not be empty for spine row {spine_index}"
            );
        }
    }

    // ── planning_root pure-function sanity check ──────────────────────────────

    /// Verify the pure planning_root() helper is reachable and defaults correctly.
    #[test]
    fn planning_root_pure_default() {
        let root = crate::config::planning_root(None);
        assert!(root.ends_with("planning"));
    }
}
