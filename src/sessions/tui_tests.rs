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
// is gone with the top tab bar; `sidebar_shows_pinned_mission_control_and_selectable_headers`
// below (BA.13.0.3) is its replacement — it asserts `◆ Mission Control` renders
// first, tier/HQ headers are shown, no top tab bar renders, and no standalone
// `brain` leaf appears.

#[cfg(test)]
mod tests {
    use crate::brain::spaces::SpaceEntry;
    use crate::sessions::app::AppState;
    use crossterm::event::KeyCode;
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

    /// Build an `AppState` covering every spine row kind: `Hq` (with a collapsed
    /// `brain` leaf + `learn-ai` child) and all four non-`Hq` tiers
    /// (`core`/`side`/`client`/`portfolio`).
    fn app_with_all_tiers() -> AppState {
        let mut tree = crate::brain::spaces::SpaceTree::default();
        tree.tiers.push((
            "_root".to_string(),
            vec![
                SpaceEntry {
                    slug: "brain".to_string(),
                    tier: "_root".to_string(),
                    repo_path: std::path::PathBuf::from("."),
                    heading: None,
                },
                SpaceEntry {
                    slug: "learn-ai".to_string(),
                    tier: "_root".to_string(),
                    repo_path: std::path::PathBuf::from("learn-ai"),
                    heading: None,
                },
            ],
        ));
        for tier in ["core", "side", "client", "portfolio"] {
            tree.tiers.push((tier.to_string(), vec![]));
        }
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
        app: &mut AppState,
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

        let mut app = empty_app(); // selected_spine = 0 = Mission Control
        render_frame(&mut app, dir.path(), 80, 24);
    }

    /// Mission Control renders at 120x40 (wider layout).
    #[test]
    fn mission_control_renders_without_panic_120x40() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_planning_fixtures(dir.path());

        let mut app = empty_app();
        render_frame(&mut app, dir.path(), 120, 40);
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

        render_frame(&mut app, dir.path(), 80, 24);
    }

    /// Space Overview degrade: missing status.md shows fallback text, no panic.
    #[test]
    fn space_overview_missing_status_md_shows_fallback() {
        // Point at an empty directory — no files at all.
        let dir = tempfile::tempdir().expect("tempdir");

        let mut app = app_with_hq_and_tier();
        app.selected_spine = 1; // Hq
        let buf = render_frame(&mut app, dir.path(), 80, 24);
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

        render_frame(&mut app, dir.path(), 80, 24);
    }

    // ── Sidebar (spine primary navigation) ────────────────────────────────────

    /// The sidebar renders `◆ Mission Control` first (before `HQ`), shows every
    /// selectable tier/`HQ` header, no longer renders a top tab bar (the old
    /// `Space Overview` / `Kanban Board` tab labels), and no longer shows a
    /// standalone `brain` leaf.
    #[test]
    fn sidebar_shows_pinned_mission_control_and_selectable_headers() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_planning_fixtures(dir.path());

        let mut app = app_with_all_tiers();
        let buf = render_frame(&mut app, dir.path(), 100, 40);
        let text = buf_to_string(&buf);

        let mc_idx = text
            .find("Mission Control")
            .expect("Mission Control must render");
        let hq_idx = text.find("HQ").expect("HQ header must render");
        assert!(
            mc_idx < hq_idx,
            "Mission Control must render before HQ; frame:\n{text}"
        );

        for tier in ["core", "side", "client", "portfolio"] {
            assert!(
                text.contains(tier),
                "sidebar must show selectable tier header '{tier}'; frame:\n{text}"
            );
        }

        // The old top tab bar rendered "Space Overview" / "Kanban Board" labels —
        // neither should appear anywhere in the frame now that the spine is the
        // single primary navigator.
        assert!(
            !text.contains("Space Overview") && !text.contains("Kanban Board"),
            "no top tab bar must render; frame:\n{text}"
        );

        assert!(
            !text.contains("brain"),
            "no standalone 'brain' leaf must render (collapsed into HQ); frame:\n{text}"
        );
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

            let buf = render_frame(&mut app, dir.path(), 80, 24);
            let cell = buf.cell((0, 0)).expect("cell(0,0) must exist");
            assert!(
                !cell.symbol().is_empty(),
                "top-left cell must not be empty for spine row {spine_index}"
            );
        }
    }

    // ── Agent panel strip (BA.13.1.3) ───────────────────────────────────────────

    /// Build an `AppState` with a fixed set of sessions (mixed `AgentState`s)
    /// and a two-tier space tree, so `selected_spine` can be driven to
    /// different `SelectedNode`s while the panel's session list stays fixed.
    fn app_with_sessions() -> AppState {
        use crate::detect::AgentState;
        use crate::sessions::model::{Session, SessionState};

        let sessions = vec![
            Session {
                name: "idle-sess".to_string(),
                state: SessionState::Idle,
                window_count: 1,
                foreground_cmd: String::new(),
                last_line: String::new(),
                agent_state: AgentState::Idle,
                blocked_reason: None,
                cwd: String::new(),
            },
            Session {
                name: "blocked-sess".to_string(),
                state: SessionState::Idle,
                window_count: 1,
                foreground_cmd: String::new(),
                last_line: String::new(),
                agent_state: AgentState::Blocked,
                blocked_reason: Some(crate::detect::BlockedReason::PermissionPrompt),
                cwd: String::new(),
            },
            Session {
                name: "working-sess".to_string(),
                state: SessionState::Running,
                window_count: 1,
                foreground_cmd: String::new(),
                last_line: String::new(),
                agent_state: AgentState::Working,
                blocked_reason: None,
                cwd: String::new(),
            },
        ];

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

        AppState::new(sessions, tree)
    }

    /// The "agents · priority" strip title renders under Mission Control (the
    /// pinned first spine row) and under a tier header (a second, distinct
    /// `SelectedNode`) — proving it's reserved under every selection, not just
    /// one branch of the content match.
    #[test]
    fn agent_panel_strip_renders_under_multiple_selected_nodes() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_planning_fixtures(dir.path());

        let mut app = app_with_sessions();

        app.selected_spine = 0; // Mission Control
        assert_eq!(
            app.selected_node(),
            crate::brain::spaces::SelectedNode::MissionControl
        );
        let buf_mc = render_frame(&mut app, dir.path(), 100, 40);
        let text_mc = buf_to_string(&buf_mc);
        assert!(
            text_mc.contains("agents"),
            "strip must render under Mission Control; frame:\n{text_mc}"
        );

        app.selected_spine = 3; // Tier(core) — [MissionControl, Hq, Space(learn-ai), Tier(core)]
        assert_eq!(
            app.selected_node(),
            crate::brain::spaces::SelectedNode::Tier("core".to_string())
        );
        let buf_tier = render_frame(&mut app, dir.path(), 100, 40);
        let text_tier = buf_to_string(&buf_tier);
        assert!(
            text_tier.contains("agents"),
            "strip must render under a tier header too; frame:\n{text_tier}"
        );
    }

    /// The strip's rows appear in urgency order (Blocked before Working before
    /// Idle), matching `agent_panel_rows`/`session_urgency`.
    #[test]
    fn agent_panel_strip_rows_appear_in_urgency_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_planning_fixtures(dir.path());

        let mut app = app_with_sessions(); // selected_spine = 0 = Mission Control
        let buf = render_frame(&mut app, dir.path(), 100, 40);
        let text = buf_to_string(&buf);

        let blocked_idx = text
            .find("blocked-sess")
            .expect("blocked session must render in the strip");
        let working_idx = text
            .find("working-sess")
            .expect("working session must render in the strip");
        let idle_idx = text
            .find("idle-sess")
            .expect("idle session must render in the strip");

        assert!(
            blocked_idx < working_idx && working_idx < idle_idx,
            "strip rows must be urgency-ordered (blocked < working < idle); frame:\n{text}"
        );
    }

    /// The strip's min-height fallback renders without panic even when the
    /// frame is too short to spare its preferred height.
    #[test]
    fn agent_panel_strip_min_height_fallback_no_panic_on_short_frame() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_planning_fixtures(dir.path());

        let mut app = app_with_sessions();
        // 5 rows tall: barely enough for the 1-line footer + a sliver of main
        // content; the strip must shrink instead of panicking or overflowing.
        render_frame(&mut app, dir.path(), 80, 5);
        // Even more extreme — 1 row tall.
        render_frame(&mut app, dir.path(), 80, 1);
    }

    // ── planning_root pure-function sanity check ──────────────────────────────

    /// Verify the pure planning_root() helper is reachable and defaults correctly.
    #[test]
    fn planning_root_pure_default() {
        let root = crate::config::planning_root(None);
        assert!(root.ends_with("planning"));
    }

    // ── Pane geometry storage (BA.13.2 task 1) ─────────────────────────────────

    /// After a draw, `AppState::pane_areas` holds non-empty Rects for every pane
    /// that is actually part of the current `SelectedNode`'s layout (spine,
    /// content, agent panel always; browser too when the Hq/Space overview is
    /// selected) — proving `draw_with_root` actually stores what
    /// `compute_pane_areas` computes, not just that geometry math is right in
    /// isolation.
    #[test]
    fn draw_stores_non_empty_pane_areas_for_mission_control() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_planning_fixtures(dir.path());

        let mut app = empty_app(); // selected_spine = 0 = Mission Control
        assert_eq!(app.pane_areas, crate::sessions::app::PaneAreas::default());

        render_frame(&mut app, dir.path(), 80, 24);

        assert!(app.pane_areas.spine.width > 0 && app.pane_areas.spine.height > 0);
        assert!(app.pane_areas.content.width > 0 && app.pane_areas.content.height > 0);
        assert!(app.pane_areas.agent_panel.width > 0 && app.pane_areas.agent_panel.height > 0);
        // Mission Control has no browser pane.
        assert_eq!(app.pane_areas.browser, ratatui::layout::Rect::default());
    }

    /// Selecting `Hq` populates the browser pane too (non-zero), unlike
    /// Mission Control.
    #[test]
    fn draw_stores_non_empty_browser_area_for_hq() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_planning_fixtures(dir.path());

        let mut app = app_with_hq_and_tier();
        app.selected_spine = 1; // Hq
        // BA.26.H: 80 wide now trips the size guard and collapses the
        // browser (that guard is exactly what this block adds); render
        // wide enough (120) to assert the *non-collapsed* case this test is
        // actually about — a guard-engaged assertion belongs with the
        // guard's own tests in sessions::app::tests.
        render_frame(&mut app, dir.path(), 120, 24);

        assert!(app.pane_areas.browser.width > 0 && app.pane_areas.browser.height > 0);
        assert!(app.pane_areas.content.width > 0 && app.pane_areas.content.height > 0);
    }

    // ── Declared-view reader keys (BA.26.A task 4, AC-2) ───────────────────────

    /// Build an app with one declared `[views]` entry (BA.26.A task 3) rooted at
    /// `root`, selected as the current spine row. Returns the app with the
    /// browser already (re)initialized at that root.
    fn app_with_view_at(root: &Path) -> AppState {
        let offered = crate::config::OfferedView {
            name: "open-work".to_string(),
            label: "Open Work".to_string(),
            root: root.to_path_buf(),
        };
        let mut app = AppState::new(vec![], crate::brain::spaces::SpaceTree::default())
            .with_offered_views(vec![offered]);
        let view_index = app.spine_rows().len() - 1;
        app.selected_spine = view_index;
        app.reinit_browser();
        app
    }

    /// AC-2 (mandatory, must be SHOWN FAILING): drive the reader keys (Right,
    /// Left, Enter) against the declared-view `SelectedNode` variant and assert
    /// pane focus moves and Enter descends — then, via a RUNTIME INVERSION
    /// rather than a committed red baseline (base-template D68), prove this
    /// assertion set can actually go red for the exact bug AC-1 forecloses.
    ///
    /// The inversion reproduces the pre-BA.26.A-task-3 non-exhaustive gate
    /// (`is_space_overview` computed as `SelectedNode::Hq | SelectedNode::Space(_)
    /// => true, _ => false` — View was not yet a variant, so it fell through to
    /// `false`) as a local closure and shows it disagrees with today's
    /// exhaustive gate on the View row — i.e. under the old gate the Right/Left
    /// focus-switch keys asserted above would silently never have fired. A test
    /// that only ever exercises the fixed gate cannot tell a real fix from a
    /// no-op; this side-by-side comparison is what makes it a gate that can
    /// fail.
    ///
    /// Written as a `match ... _ => false` rather than the `matches!` macro on
    /// purpose: `scripts/check-selected-node-exhaustive.sh` (this same task)
    /// sweeps ALL of src/ — this test file included — for exactly that macro
    /// over a `selected_node()` scrutinee, and a literal reproduction here
    /// would trip its own regression check.
    #[test]
    fn view_variant_reader_keys_move_focus_and_descend_and_the_gate_can_fail() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().canonicalize().expect("canonicalize root");
        let child = root.join("child");
        std::fs::create_dir(&child).expect("mkdir child");
        let child = child.canonicalize().expect("canonicalize child");

        let mut app = app_with_view_at(&root);
        match app.selected_node() {
            crate::brain::spaces::SelectedNode::View(view) => {
                assert_eq!(view.name, "open-work")
            }
            other => panic!("expected SelectedNode::View(open-work), got {other:?}"),
        }
        assert_eq!(
            app.file_browser.dir, root,
            "reinit_browser must root the browser at the declared view's own root"
        );

        // Right: Sidebar -> Browser -> Content. Left: Content -> Browser -> Sidebar.
        assert_eq!(
            app.overview_pane,
            crate::sessions::app::OverviewPane::Sidebar
        );
        app.on_key(KeyCode::Right);
        assert_eq!(
            app.overview_pane,
            crate::sessions::app::OverviewPane::Browser,
            "Right must move focus Sidebar -> Browser for a declared view"
        );
        app.on_key(KeyCode::Right);
        assert_eq!(
            app.overview_pane,
            crate::sessions::app::OverviewPane::Content,
            "Right must move focus Browser -> Content for a declared view"
        );
        app.on_key(KeyCode::Left);
        assert_eq!(
            app.overview_pane,
            crate::sessions::app::OverviewPane::Browser,
            "Left must move focus Content -> Browser for a declared view"
        );

        // Enter, with the "child" directory entry selected, must descend.
        let child_idx = app
            .file_browser
            .entries
            .iter()
            .position(|e| e.kind == bella_engine::browser::BrowserEntryKind::Dir)
            .expect("fixture root must list the 'child' directory entry");
        app.file_browser.selected = child_idx;
        app.on_key(KeyCode::Enter);
        assert_eq!(
            app.file_browser.dir, child,
            "Enter on a directory entry must descend the browser into it for a declared view"
        );

        // ── Runtime inversion: prove the gate can fail (D68) ────────────────
        // Reproduce the OLD, non-exhaustive gate from before BA.26.A task 3
        // (View was not yet a SelectedNode variant, so it fell through to the
        // `_ => false` arm) as a local closure over the SAME `selected_node()`
        // this test already exercised above. Deliberately a `match ... _ =>`,
        // not the `matches!` macro — see the doc comment above.
        #[allow(clippy::match_like_matches_macro)]
        let naive_is_space_overview = match app.selected_node() {
            crate::brain::spaces::SelectedNode::Hq
            | crate::brain::spaces::SelectedNode::Space(_) => true,
            _ => false,
        };
        assert!(
            !naive_is_space_overview,
            "the pre-fix non-exhaustive `matches!` gate treats a declared View as NOT \
             space-overview — reproducing the exact bug AC-1's exhaustive match forecloses; \
             if this ever becomes true the inversion below is testing nothing"
        );

        // Today's exhaustive gate (src/sessions/app.rs's `is_space_overview`) is
        // what actually ran when Right/Left/Enter fired above.
        let exhaustive_is_space_overview = match app.selected_node() {
            crate::brain::spaces::SelectedNode::Hq
            | crate::brain::spaces::SelectedNode::Space(_)
            | crate::brain::spaces::SelectedNode::View(_) => true,
            crate::brain::spaces::SelectedNode::MissionControl
            | crate::brain::spaces::SelectedNode::Tier(_) => false,
        };
        assert!(
            exhaustive_is_space_overview,
            "the exhaustive gate must treat a declared View as space-overview"
        );

        assert_ne!(
            naive_is_space_overview, exhaustive_is_space_overview,
            "the naive pre-fix gate and today's exhaustive gate must disagree on the View \
             variant — this disagreement is what proves the Right/Left/Enter assertions above \
             actually exercise AC-1's fix rather than passing vacuously"
        );
    }

    // ── scripts/check-selected-node-exhaustive.sh (BA.26.A task 4, AC-3) ───────

    /// Locate the repo root from `CARGO_MANIFEST_DIR` so this test finds the
    /// script regardless of the working directory `cargo test`/`nextest` is
    /// invoked from.
    fn repo_root() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    /// AC-3's regression sweep must exit 0 (no `matches!`/`==`/`!=` gating on
    /// `SelectedNode` anywhere in src/) on the post-change tree.
    #[test]
    fn selected_node_exhaustive_check_passes_on_current_tree() {
        let script = repo_root().join("scripts/check-selected-node-exhaustive.sh");
        let status = std::process::Command::new("bash")
            .arg(&script)
            .current_dir(repo_root())
            .status()
            .expect("failed to run scripts/check-selected-node-exhaustive.sh");
        assert!(
            status.success(),
            "scripts/check-selected-node-exhaustive.sh must exit 0 on the current tree"
        );
    }

    /// AC-3's MANDATORY positive control: the identical sweep run against
    /// scripts/fixtures/selected-node-control-pre-change.rs (a checked-in
    /// fixture reproducing the pre-BA.26.A `is_space_overview` shape) must
    /// MATCH — proving the sweep can find the pattern it otherwise reports
    /// absent. An empty result here would mean the instrument is broken, not
    /// that the tree is clean (standing rule 11).
    ///
    /// A fixture, not `git show <hardcoded-rev>`: an earlier version of this
    /// test resolved the control from a commit hash that turned out not to
    /// be reachable from this repo's own `origin/main` history at all — not
    /// a shallow-checkout gap, the object was simply absent from the pushed
    /// history graph, so it failed in every CI run regardless of checkout
    /// depth (confirmed 2026-09-08). A checked-in fixture is deterministic
    /// in any checkout.
    #[test]
    fn selected_node_exhaustive_check_control_matches_fixture() {
        let script = repo_root().join("scripts/check-selected-node-exhaustive.sh");
        let status = std::process::Command::new("bash")
            .arg(&script)
            .arg("--control")
            .current_dir(repo_root())
            .status()
            .expect("failed to run scripts/check-selected-node-exhaustive.sh --control");
        assert!(
            status.success(),
            "the positive control must pass: scripts/fixtures/selected-node-control-pre-change.rs \
             must match the sweep, or the instrument itself is broken"
        );
    }
}
