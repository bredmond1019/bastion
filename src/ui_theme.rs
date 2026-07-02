// ui_theme.rs — bastion TUI color palette.
//
// Single source of truth for every color used in the session dashboard,
// Kanban board, and Mission Control renders. Import this module wherever
// ratatui styles need to be built.
//
// Palette philosophy: deep navy background, periwinkle/violet as primary
// accents, cyan as highlight, sage green for success/active states.
// All colors are produced through `bella_engine::palette::rgb()` so they
// degrade gracefully to xterm-256 on terminals that lack truecolor.
//
// Config-driven theme (BA.14.0): every named color/style function below
// reads from a process-wide runtime `Theme` (see `current_theme()` /
// `init_theme()`) rather than baked `rgb()` literals. The only place fixed
// `rgb(...)` calls are allowed is inside a `Theme` preset constructor
// (e.g. `Theme::bastion()`).

use bella_engine::palette::rgb;
use ratatui::style::{Color, Modifier, Style};
use std::sync::OnceLock;

// ── Runtime theme ───────────────────────────────────────────────────────────

/// A named palette of the colors bastion's TUI chrome is built from.
///
/// Presets are selected by name (see `theme_by_name`); `bastion` is both the
/// name of the default preset and the fallback used for an absent/unknown
/// config selection.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub name: &'static str,
    /// Periwinkle blue — primary accent, active tab indicator, selected borders.
    pub accent: Color,
    /// Violet — secondary accent, idle session dots, Kanban "Up Next" header.
    pub violet: Color,
    /// Cyan — links, "running" session dots, highlights.
    pub cyan: Color,
    /// Sage green — success/active states, "In Progress" Kanban header.
    pub sage: Color,
    /// Soft red/rose — blocked/error states, "Blocked" Kanban header.
    pub rose: Color,
    /// Muted text — secondary info, sub-labels, last-line output.
    pub muted: Color,
    /// Body text — primary readable foreground.
    pub text: Color,
    /// Dim border — inactive panel borders.
    pub border_dim: Color,
    /// Active border — focused panel borders.
    pub border_active: Color,
    /// Deep navy — code / block backgrounds.
    pub surface: Color,
}

impl Theme {
    /// The default bastion Mission Control palette (deep navy / periwinkle / cyan / sage).
    pub fn bastion() -> Self {
        Theme {
            name: "bastion",
            accent: rgb(0x88, 0x99, 0xff),
            violet: rgb(0xb0, 0x7f, 0xff),
            cyan: rgb(0x00, 0xd2, 0xff),
            sage: rgb(0x5c, 0xce, 0x94),
            rose: rgb(0xff, 0x6b, 0x8a),
            muted: rgb(0x6b, 0x70, 0x99),
            text: rgb(0xd0, 0xd4, 0xf0),
            border_dim: rgb(0x3d, 0x40, 0x58),
            border_active: rgb(0x58, 0x65, 0xd6),
            surface: rgb(0x1e, 0x20, 0x3a),
        }
    }
}

/// Resolve a preset by name, case-insensitively. Falls back to the `bastion`
/// default for an absent (empty) or unrecognized name — never panics.
pub fn theme_by_name(name: &str) -> Theme {
    match name.trim().to_ascii_lowercase().as_str() {
        "bastion" => Theme::bastion(),
        _ => Theme::bastion(),
    }
}

static ACTIVE_THEME: OnceLock<Theme> = OnceLock::new();

/// Initialize the process-wide active theme. Intended to be called once at
/// startup (e.g. from the resolved `FileConfig` `[theme]` section). A second
/// call is a no-op — the first-set theme wins, matching `OnceLock` semantics.
pub fn init_theme(theme: Theme) {
    let _ = ACTIVE_THEME.set(theme);
}

/// The currently active theme, defaulting to `Theme::bastion()` if
/// `init_theme` has not yet been called.
pub fn current_theme() -> &'static Theme {
    ACTIVE_THEME.get_or_init(Theme::bastion)
}

/// Map a bastion `Theme` onto the shared `bella_engine::Theme` so chrome and
/// the markdown view (`render_with_edit`) render from the same palette.
pub fn to_bella_theme(theme: &Theme) -> bella_engine::Theme {
    bella_engine::Theme {
        name: theme.name.to_string(),
        fg: theme.text,
        bg: None,
        muted: theme.muted,
        heading: [
            theme.accent,
            theme.violet,
            theme.cyan,
            theme.sage,
            theme.text,
            theme.muted,
        ],
        heading_modifier: Modifier::BOLD,
        emphasis: Modifier::ITALIC,
        strong: Modifier::BOLD,
        code_fg: theme.sage,
        code_bg: Some(theme.surface),
        link: theme.cyan,
        link_focused: theme.violet,
        link_modifier: Modifier::UNDERLINED,
        quote: theme.muted,
        list_marker: theme.violet,
        rule: theme.border_dim,
        strikethrough: Modifier::CROSSED_OUT,
        status_fg: theme.text,
        status_bg: theme.border_active,
        syntect_theme: "base16-ocean.dark",
    }
}

// ── Named colors ──────────────────────────────────────────────────────────────

/// Periwinkle blue — primary accent, active tab indicator, selected borders.
pub fn accent() -> Color {
    current_theme().accent
}

/// Violet — secondary accent, idle session dots, Kanban "Up Next" header.
pub fn violet() -> Color {
    current_theme().violet
}

/// Cyan — links, "running" session dots, highlights.
pub fn cyan() -> Color {
    current_theme().cyan
}

/// Sage green — success/active states, "In Progress" Kanban header.
pub fn sage() -> Color {
    current_theme().sage
}

/// Soft red/rose — blocked/error states, "Blocked" Kanban header.
pub fn rose() -> Color {
    current_theme().rose
}

/// Muted text — secondary info, sub-labels, last-line output.
pub fn muted() -> Color {
    current_theme().muted
}

/// Body text — primary readable foreground.
pub fn text() -> Color {
    current_theme().text
}

/// Dim border — inactive panel borders.
pub fn border_dim() -> Color {
    current_theme().border_dim
}

/// Active border — focused panel borders.
pub fn border_active() -> Color {
    current_theme().border_active
}

/// Deep navy — code / block backgrounds.
pub fn surface() -> Color {
    current_theme().surface
}

// ── Composed styles ───────────────────────────────────────────────────────────

/// Style for the panel title text (e.g. "Spaces", "Kanban Board").
pub fn title_style() -> Style {
    Style::default().fg(accent()).add_modifier(Modifier::BOLD)
}

/// Style for the active tab label.
pub fn tab_active_style() -> Style {
    Style::default()
        .fg(accent())
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
}

/// Style for inactive tab labels.
pub fn tab_inactive_style() -> Style {
    Style::default().fg(muted())
}

/// Style for the selected item in a list (sidebar session highlight).
pub fn list_selected_style() -> Style {
    Style::default()
        .fg(surface())
        .bg(accent())
        .add_modifier(Modifier::BOLD)
}

/// Style for a "running" session state indicator.
pub fn state_running_style() -> Style {
    Style::default().fg(cyan()).add_modifier(Modifier::BOLD)
}

/// Style for an "idle" session state indicator.
pub fn state_idle_style() -> Style {
    Style::default().fg(muted())
}

/// Style for an "agent working" state indicator.
pub fn state_working_style() -> Style {
    Style::default().fg(sage()).add_modifier(Modifier::BOLD)
}

/// Style for an "agent blocked" state indicator.
pub fn state_blocked_style() -> Style {
    Style::default().fg(rose()).add_modifier(Modifier::BOLD)
}

/// Style for the footer status bar.
pub fn footer_style() -> Style {
    Style::default().fg(muted())
}

/// Style for a transient status/error message in the footer.
pub fn footer_status_style() -> Style {
    Style::default().fg(cyan())
}

/// Kanban "In Progress" column header style.
pub fn kanban_now_style() -> Style {
    Style::default().fg(sage()).add_modifier(Modifier::BOLD)
}

/// Kanban "Up Next" column header style.
pub fn kanban_next_style() -> Style {
    Style::default().fg(violet()).add_modifier(Modifier::BOLD)
}

/// Kanban "Blocked" column header style.
pub fn kanban_blocked_style() -> Style {
    Style::default().fg(rose()).add_modifier(Modifier::BOLD)
}

/// Kanban item ID label style.
pub fn kanban_id_style() -> Style {
    Style::default().fg(accent())
}

/// Kanban item title style.
pub fn kanban_title_style() -> Style {
    Style::default().fg(text())
}

/// Border style for inactive panels.
pub fn border_dim_style() -> Style {
    Style::default().fg(border_dim())
}

/// Border style for the active / focused panel.
pub fn border_active_style() -> Style {
    Style::default().fg(border_active())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_by_name_resolves_known_preset() {
        let theme = theme_by_name("bastion");
        assert_eq!(theme.name, "bastion");
        assert_eq!(theme, Theme::bastion());
    }

    #[test]
    fn theme_by_name_is_case_insensitive() {
        let theme = theme_by_name("Bastion");
        assert_eq!(theme.name, "bastion");
    }

    #[test]
    fn theme_by_name_falls_back_for_absent_name() {
        let theme = theme_by_name("");
        assert_eq!(theme, Theme::bastion());
    }

    #[test]
    fn theme_by_name_falls_back_for_unknown_name() {
        let theme = theme_by_name("nonexistent-preset");
        assert_eq!(theme, Theme::bastion());
    }

    #[test]
    fn bastion_preset_matches_expected_accent_color() {
        let theme = Theme::bastion();
        assert_eq!(theme.accent, rgb(0x88, 0x99, 0xff));
        assert_eq!(theme.surface, rgb(0x1e, 0x20, 0x3a));
    }

    #[test]
    fn to_bella_theme_maps_roles_from_bastion_theme() {
        let theme = Theme::bastion();
        let bella = to_bella_theme(&theme);

        assert_eq!(bella.name, "bastion");
        assert_eq!(bella.fg, theme.text);
        assert_eq!(bella.muted, theme.muted);
        assert_eq!(
            bella.heading,
            [
                theme.accent,
                theme.violet,
                theme.cyan,
                theme.sage,
                theme.text,
                theme.muted,
            ]
        );
        assert_eq!(bella.link, theme.cyan);
        assert_eq!(bella.link_focused, theme.violet);
        assert_eq!(bella.quote, theme.muted);
        assert_eq!(bella.list_marker, theme.violet);
        assert_eq!(bella.rule, theme.border_dim);
        assert_eq!(bella.code_fg, theme.sage);
        assert_eq!(bella.code_bg, Some(theme.surface));
        assert_eq!(bella.status_fg, theme.text);
        assert_eq!(bella.status_bg, theme.border_active);
    }
}
