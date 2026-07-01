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

use bella_engine::palette::rgb;
use ratatui::style::{Color, Modifier, Style};

// ── Named colors ──────────────────────────────────────────────────────────────

/// Periwinkle blue — primary accent, active tab indicator, selected borders.
pub fn accent() -> Color {
    rgb(0x88, 0x99, 0xff)
}

/// Violet — secondary accent, idle session dots, Kanban "Up Next" header.
pub fn violet() -> Color {
    rgb(0xb0, 0x7f, 0xff)
}

/// Cyan — links, "running" session dots, highlights.
pub fn cyan() -> Color {
    rgb(0x00, 0xd2, 0xff)
}

/// Sage green — success/active states, "In Progress" Kanban header.
pub fn sage() -> Color {
    rgb(0x5c, 0xce, 0x94)
}

/// Soft red/rose — blocked/error states, "Blocked" Kanban header.
pub fn rose() -> Color {
    rgb(0xff, 0x6b, 0x8a)
}

/// Muted text — secondary info, sub-labels, last-line output.
pub fn muted() -> Color {
    rgb(0x6b, 0x70, 0x99)
}

/// Body text — primary readable foreground.
pub fn text() -> Color {
    rgb(0xd0, 0xd4, 0xf0)
}

/// Dim border — inactive panel borders.
pub fn border_dim() -> Color {
    rgb(0x3d, 0x40, 0x58)
}

/// Active border — focused panel borders.
pub fn border_active() -> Color {
    rgb(0x58, 0x65, 0xd6)
}

/// Deep navy — code / block backgrounds.
pub fn surface() -> Color {
    rgb(0x1e, 0x20, 0x3a)
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
