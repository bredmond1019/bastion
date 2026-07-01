// sessions/tui_tests.rs — integration smoke tests for the unified console TUI.
//
// Uses `ratatui::backend::TestBackend` to render each tab at a fixed terminal
// size and asserts no panic occurs and the output buffer contains expected
// content markers.
//
// The planning root path is injected directly into `draw_for_test` so no
// process-environment mutation is needed — these tests are safe to run in
// parallel.

#[cfg(test)]
mod tests {
    use crate::sessions::app::{AppState, TabState};
    use ratatui::{Terminal, backend::TestBackend};
    use std::path::Path;

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Create a fresh `AppState` with no sessions (DB-free, tmux-free).
    fn empty_app() -> AppState {
        AppState::new(vec![], crate::brain::spaces::SpaceTree::default())
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

    // ── SpaceOverview tab ─────────────────────────────────────────────────────

    /// SpaceOverview tab renders without panic at 80×24 when status.md exists.
    #[test]
    fn space_overview_renders_without_panic_80x24() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_planning_fixtures(dir.path());

        let app = empty_app(); // active_tab_index = 0 = SpaceOverview
        render_frame(&app, dir.path(), 80, 24);
    }

    /// SpaceOverview tab renders at 120×40 (wider layout).
    #[test]
    fn space_overview_renders_without_panic_120x40() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_planning_fixtures(dir.path());

        let app = empty_app();
        render_frame(&app, dir.path(), 120, 40);
    }

    /// SpaceOverview degrade: missing status.md shows fallback text, no panic.
    #[test]
    fn space_overview_missing_status_md_shows_fallback() {
        // Point at an empty directory — no files at all.
        let dir = tempfile::tempdir().expect("tempdir");

        let app = empty_app();
        let buf = render_frame(&app, dir.path(), 80, 24);
        let text = buf_to_string(&buf);
        assert!(
            text.contains("No planning"),
            "fallback text must appear for missing status.md; frame:\n{text}"
        );
    }

    // ── Kanban tab ────────────────────────────────────────────────────────────

    /// Kanban tab renders valid state.json without panic.
    #[test]
    fn kanban_renders_valid_state_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_planning_fixtures(dir.path());

        let mut app = empty_app();
        app.active_tab_index = 1; // Kanban is tab 1
        assert_eq!(app.tabs[1], TabState::Kanban);

        render_frame(&app, dir.path(), 80, 24);
    }

    /// Kanban degrade: missing state.json shows fallback text, no panic.
    #[test]
    fn kanban_missing_state_json_shows_fallback() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Only status.md present; no state.json.
        std::fs::write(dir.path().join("status.md"), "# Status\n").unwrap();

        let mut app = empty_app();
        app.active_tab_index = 1;

        let buf = render_frame(&app, dir.path(), 80, 24);
        let text = buf_to_string(&buf);
        assert!(
            text.contains("No planning"),
            "fallback must appear for missing state.json; frame:\n{text}"
        );
    }

    /// Kanban degrade: malformed state.json shows parse-error fallback, no panic.
    #[test]
    fn kanban_malformed_state_json_shows_parse_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("state.json"), "{ not valid json !!!").unwrap();

        let mut app = empty_app();
        app.active_tab_index = 1;

        let buf = render_frame(&app, dir.path(), 80, 24);
        let text = buf_to_string(&buf);
        assert!(
            text.contains("Failed"),
            "parse-error fallback must appear for malformed state.json; frame:\n{text}"
        );
    }

    // ── MissionControl tab ────────────────────────────────────────────────────

    /// MissionControl tab renders (no active run) without panic.
    #[test]
    fn mission_control_renders_without_panic() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_planning_fixtures(dir.path());

        let mut app = empty_app();
        app.active_tab_index = 2; // MissionControl is tab 2
        assert_eq!(app.tabs[2], TabState::MissionControl);

        render_frame(&app, dir.path(), 80, 24);
    }

    // ── Layout invariants ─────────────────────────────────────────────────────

    /// At 80×24, the top-left corner must always be occupied (border or content)
    /// regardless of which tab is active.
    #[test]
    fn frame_top_left_is_always_occupied() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_planning_fixtures(dir.path());

        for tab_index in 0..3usize {
            let mut app = empty_app();
            app.active_tab_index = tab_index;

            let buf = render_frame(&app, dir.path(), 80, 24);
            let cell = buf.cell((0, 0)).expect("cell(0,0) must exist");
            assert!(
                !cell.symbol().is_empty(),
                "top-left cell must not be empty for tab {tab_index}"
            );
        }
    }

    /// Tab bar must appear in the rendered output (contains all three tab titles).
    #[test]
    fn tab_bar_contains_all_tab_names() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_planning_fixtures(dir.path());

        let app = empty_app();
        let buf = render_frame(&app, dir.path(), 120, 30);
        let text = buf_to_string(&buf);

        assert!(
            text.contains("Space Overview"),
            "Space Overview tab must appear; frame:\n{text}"
        );
        assert!(
            text.contains("Kanban"),
            "Kanban tab must appear; frame:\n{text}"
        );
        assert!(
            text.contains("Mission Control"),
            "Mission Control tab must appear; frame:\n{text}"
        );
    }

    // ── planning_root pure-function sanity check ──────────────────────────────

    /// Verify the pure planning_root() helper is reachable and defaults correctly.
    #[test]
    fn planning_root_pure_default() {
        let root = crate::config::planning_root(None);
        assert_eq!(root, std::path::PathBuf::from("planning"));
    }
}
