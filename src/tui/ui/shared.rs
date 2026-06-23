use super::palette::{FG, MUTED};

use crate::models::{FeedRole, Staleness};
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

/// Compact indicator for an epic's feed routing role, shown in the epic header.
/// `None` for ordinary epics (`FeedRole::None`); `Some("role:<role>  ")` for
/// managed feed epics so the routing parent and its role sub-epics are
/// identifiable at a glance.
pub(in crate::tui::ui) fn feed_role_label(role: FeedRole) -> Option<String> {
    match role {
        FeedRole::None => None,
        other => Some(format!("role:{}  ", other.as_str())),
    }
}

pub(in crate::tui::ui) fn render_top_indicators(frame: &mut Frame, app: &App, area: Rect) {
    let mut parts: Vec<Span> = Vec::new();
    // Auto dispatch indicator — only in epic view
    if let ViewMode::Epic { epic_id, .. } = app.view_mode() {
        if let Some(epic) = app.epics().iter().find(|e| e.id == *epic_id) {
            let (label, style) = if epic.auto_dispatch {
                ("auto dispatch [U]  ", Style::default().fg(Color::Green))
            } else {
                ("manual dispatch [U]  ", Style::default().fg(MUTED))
            };
            parts.push(Span::styled(label, style));

            // Feed routing role — for managed feed epics (parent + role sub-epics)
            if let Some(role_label) = feed_role_label(epic.feed_role) {
                parts.push(Span::styled(role_label, Style::default().fg(MUTED)));
            }

            // Group-by-repo indicator — shown for all epics
            let (label, style) = if epic.group_by_repo {
                ("group:on [R]  ", Style::default().fg(Color::Green))
            } else {
                ("group:off [R]  ", Style::default().fg(MUTED))
            };
            parts.push(Span::styled(label, style));
        }
    }
    if !app.repo_filter().is_empty() {
        let active = app.repo_filter().len();
        let total = app.board.repo_paths.len();
        let label = match app.repo_filter_mode() {
            RepoFilterMode::Include => format!("[{active}/{total} repos]  "),
            RepoFilterMode::Exclude => format!("[excl {active}/{total} repos]  "),
        };
        parts.push(Span::styled(label, Style::default().fg(MUTED)));
    }
    if app.notifications_enabled() {
        parts.push(Span::styled(
            "\u{1F514} [N]",
            Style::default().fg(Color::Yellow),
        ));
    } else {
        parts.push(Span::styled("\u{1F515} [N]", Style::default().fg(MUTED)));
    }
    let line = Line::from(parts);
    frame.render_widget(Paragraph::new(line).alignment(Alignment::Right), area);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_role_label_none_is_hidden() {
        assert_eq!(feed_role_label(FeedRole::None), None);
    }

    #[test]
    fn feed_role_label_shows_kebab_role() {
        assert_eq!(
            feed_role_label(FeedRole::ReviewsParent).as_deref(),
            Some("role:reviews-parent  ")
        );
        assert_eq!(
            feed_role_label(FeedRole::MyReviews).as_deref(),
            Some("role:my-reviews  ")
        );
        assert_eq!(
            feed_role_label(FeedRole::Cve).as_deref(),
            Some("role:cve  ")
        );
    }
}
