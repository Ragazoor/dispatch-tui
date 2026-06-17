//! Managed-feed config popup (the `C` key).
//!
//! A small centered form with four fields (reviews command/interval, CVE
//! command/interval). The focused field is highlighted; intervals show a
//! "(default)" hint when blank. Rendered only while
//! `InputMode::ManagedFeedConfig` is active.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::tui::messages::ManagedFeedField;
use crate::tui::{App, InputMode};

pub(in crate::tui::ui::kanban) fn render_managed_feed_config_overlay(
    frame: &mut Frame,
    app: &App,
    area: Rect,
) {
    if !matches!(app.mode(), InputMode::ManagedFeedConfig) {
        return;
    }
    let Some(state) = app.managed_feed_config() else {
        return;
    };

    let popup_width = (area.width * 70 / 100).clamp(40, 80);
    let popup_height = 11u16.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" Managed feed config ")
        .title_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let field_line = |label: &str, value: &str, field: ManagedFeedField, is_interval: bool| {
        let focused = state.field == field;
        let marker = if focused { "> " } else { "  " };
        let shown = if value.is_empty() {
            if is_interval {
                "(default)".to_string()
            } else {
                "(unset)".to_string()
            }
        } else {
            value.to_string()
        };
        let label_style = if focused {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let value_style = if value.is_empty() {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::White)
        };
        Line::from(vec![
            Span::styled(format!("{marker}{label}: "), label_style),
            Span::styled(shown, value_style),
        ])
    };

    let lines = vec![
        field_line(
            "Reviews command",
            &state.reviews_command,
            ManagedFeedField::ReviewsCommand,
            false,
        ),
        field_line(
            "Reviews interval (s)",
            &state.reviews_interval,
            ManagedFeedField::ReviewsInterval,
            true,
        ),
        field_line(
            "CVE command",
            &state.cve_command,
            ManagedFeedField::CveCommand,
            false,
        ),
        field_line(
            "CVE interval (s)",
            &state.cve_interval,
            ManagedFeedField::CveInterval,
            true,
        ),
        Line::from(""),
        Line::styled(
            " Tab/\u{2191}\u{2193}:field  Enter:save  Esc:cancel  (empty command = unset)",
            Style::default().fg(Color::DarkGray),
        ),
    ];

    frame.render_widget(Paragraph::new(lines), inner);
}
