use super::palette::{BORDER, FG, MUTED, PURPLE};

use crate::models::{Epic, Staleness};
use crate::tui::{App, RepoFilterMode, ViewMode};
use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{ListItem, Paragraph},
    Frame,
};
use std::time::{Duration, Instant};

/// Returns (text, color) for the 1-line refresh status row shown on Review and Security boards.
///
/// Thresholds are relative to `interval` so both boards stay consistent regardless of their
/// different poll rates.
pub fn refresh_status(
    last_fetch: Option<Instant>,
    loading: bool,
    interval: Duration,
) -> (String, Color) {
    if loading {
        return ("Refreshing...  [r] refresh".to_string(), Color::DarkGray);
    }
    let Some(last) = last_fetch else {
        return ("Never fetched  [r] refresh".to_string(), Color::DarkGray);
    };
    let elapsed = last.elapsed();
    let elapsed_str = if elapsed.as_secs() < 60 {
        format!("{}s ago", elapsed.as_secs())
    } else {
        format!(
            "{}m {}s ago",
            elapsed.as_secs() / 60,
            elapsed.as_secs() % 60
        )
    };
    let text = format!("Updated {elapsed_str}  [r] refresh");
    let color = if elapsed >= interval * 4 {
        Color::Red
    } else if elapsed >= interval * 2 {
        Color::Yellow
    } else {
        Color::White
    };
    (text, color)
}

/// Map a staleness tier to a terminal color.
/// Uses indexed terminal colors (not palette constants) so these adapt to the
/// user's terminal theme rather than being locked to Tokyo Night RGB values.
pub(in crate::tui::ui) fn staleness_color(staleness: Staleness) -> Color {
    match staleness {
        Staleness::Fresh => Color::Green,
        Staleness::Aging => Color::Yellow,
        Staleness::Stale => Color::Red,
    }
}

/// Truncate a string to at most `max` characters, appending "…" if truncated.
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", truncated)
    }
}

const LOADING_GLYPH: &str = " \u{21bb}";
const FILTER_GLYPH: &str = " \u{25c6}";

/// Format a tab label with optional count, filter, and loading indicators.
fn tab_label(prefix: &str, name: &str, count: usize, filter: bool, loading: bool) -> String {
    let count_part = if count > 0 {
        format!(" ({count})")
    } else {
        String::new()
    };
    let filter_part = if filter { FILTER_GLYPH } else { "" };
    let loading_part = if loading { LOADING_GLYPH } else { "" };
    format!("{prefix}{name}{count_part}{filter_part}{loading_part} ")
}

pub(in crate::tui::ui) fn render_tab_bar(frame: &mut Frame, app: &App, area: Rect) {
    let active_style = Style::default().fg(FG).add_modifier(Modifier::BOLD);
    let inactive_style = Style::default().fg(MUTED);
    let feed_epics: Vec<&Epic> = app
        .epics()
        .iter()
        .filter(|e| e.feed_command.is_some())
        .collect();

    // Determine which feed epic index (if any) is active.
    let active_feed_idx: Option<usize> = match app.view_mode() {
        ViewMode::Epic { epic_id, .. } => feed_epics.iter().position(|e| e.id == *epic_id),
        ViewMode::Board(_) | ViewMode::TaskDetail { .. } => None,
    };

    let mut spans: Vec<Span> = Vec::new();

    // Active project prefix
    let active_project_name = app
        .projects()
        .iter()
        .find(|p| p.id == app.active_project())
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "Default".to_string());
    spans.push(Span::styled(
        format!("[{}]  ", active_project_name),
        Style::default().fg(PURPLE).add_modifier(Modifier::BOLD),
    ));

    // Tasks tab
    match app.view_mode() {
        ViewMode::Epic { epic_id, .. } if active_feed_idx.is_none() => {
            // Epic view for a non-feed epic
            let epic_title = app
                .epics()
                .iter()
                .find(|e| e.id == *epic_id)
                .map(|e| truncate(&e.title, 30))
                .unwrap_or_else(|| "Epic".to_string());
            spans.push(Span::styled(
                format!(" \u{25b8} Epic: {epic_title} "),
                active_style.fg(PURPLE),
            ));
        }
        ViewMode::Board(_) => {
            spans.push(Span::styled(" \u{25b8} Tasks ", active_style));
        }
        _ => {
            spans.push(Span::styled(" Tasks ", inactive_style));
        }
    }

    // Feed epic tabs
    for (idx, epic) in feed_epics.iter().enumerate() {
        spans.push(Span::styled(" \u{2502} ", Style::default().fg(BORDER)));
        let is_active = active_feed_idx == Some(idx);
        let label = if is_active {
            tab_label(" \u{25b8} ", &epic.title, 0, false, false)
        } else {
            tab_label(" ", &epic.title, 0, false, false)
        };
        let style = if is_active {
            active_style
        } else {
            inactive_style
        };
        spans.push(Span::styled(label, style));
    }

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);

    // Right-aligned indicators (filter, notifications)
    let mut right_parts: Vec<Span> = Vec::new();
    // Auto dispatch indicator — only in epic view
    if let ViewMode::Epic { epic_id, .. } = app.view_mode() {
        if let Some(epic) = app.epics().iter().find(|e| e.id == *epic_id) {
            let (label, style) = if epic.auto_dispatch {
                ("auto dispatch [U]  ", Style::default().fg(Color::Green))
            } else {
                ("manual dispatch [U]  ", Style::default().fg(MUTED))
            };
            right_parts.push(Span::styled(label, style));
        }
    }
    if !app.repo_filter().is_empty() {
        let active = app.repo_filter().len();
        let total = app.board.repo_paths.len();
        let label = match app.repo_filter_mode() {
            RepoFilterMode::Include => format!("[{active}/{total} repos]  "),
            RepoFilterMode::Exclude => format!("[excl {active}/{total} repos]  "),
        };
        right_parts.push(Span::styled(label, Style::default().fg(MUTED)));
    }
    if app.notifications_enabled() {
        right_parts.push(Span::styled(
            "\u{1F514} [N]",
            Style::default().fg(Color::Yellow),
        ));
    } else {
        right_parts.push(Span::styled("\u{1F515} [N]", Style::default().fg(MUTED)));
    }
    if !right_parts.is_empty() {
        let right_line = Line::from(right_parts);
        let p = Paragraph::new(right_line).alignment(Alignment::Right);
        frame.render_widget(p, area);
    }
}

/// Non-selectable section header injected between substatus groups.
/// `first` — when true, omits the leading blank line so the top of the column
/// doesn't have an awkward gap before the very first group.
pub(in crate::tui::ui) fn render_substatus_header(label: &str, first: bool) -> ListItem<'static> {
    let header = Line::from(Span::styled(
        format!("  \u{2500}\u{2500} {label} "),
        Style::default().fg(FG).add_modifier(Modifier::BOLD),
    ));
    if first {
        ListItem::new(vec![header])
    } else {
        ListItem::new(vec![Line::raw(""), header])
    }
}

/// Push a keybinding hint as styled spans.
///
/// When the key is a single char matching the label's first letter (e.g. `d` / `dispatch`),
/// renders the compact `[d]ispatch` form. Otherwise renders `[key] label`.
pub(in crate::tui::ui) fn push_hint_spans(
    spans: &mut Vec<Span<'static>>,
    key: &str,
    label: &str,
    key_color: Color,
    label_style: Style,
) {
    let mut key_chars = key.chars();
    let key_char = key_chars.next();
    let key_is_single = key_char.is_some() && key_chars.next().is_none();
    let can_embed = key_is_single
        && label
            .chars()
            .next()
            .zip(key_char)
            .is_some_and(|(l, k)| l.eq_ignore_ascii_case(&k));

    spans.push(Span::styled(
        format!("[{key}]"),
        Style::default().fg(key_color).add_modifier(Modifier::BOLD),
    ));
    let label_text = if can_embed {
        format!("{}  ", &label[1..])
    } else {
        format!(" {label}  ")
    };
    spans.push(Span::styled(label_text, label_style));
}
