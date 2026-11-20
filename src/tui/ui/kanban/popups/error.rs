//! Error popup overlay.

use ratatui::{
    layout::{Alignment, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{BorderType, Paragraph, Wrap},
    Frame,
};

use crate::palette::{FG, MUTED, RED};
use crate::tui::ui::shared::{centered_rect, open_overlay, titled_block};
use crate::tui::App;

pub(in crate::tui::ui::kanban) fn render_error_popup(frame: &mut Frame, app: &App, area: Rect) {
    let Some(error_msg) = &app.status.error_popup else {
        return;
    };

    // A message of several lines (a key-table load's refusals) is a list:
    // each line its own row, left-aligned, the popup as tall as it needs.
    let multi_line = error_msg.contains('\n');
    let popup_width = if multi_line {
        (area.width * 80 / 100).clamp(30, 100)
    } else {
        (area.width * 60 / 100).clamp(30, 60)
    };
    let inner_width = usize::from(popup_width.saturating_sub(2)).max(1);
    let message_rows: usize = error_msg
        .lines()
        .map(|l| l.chars().count().div_ceil(inner_width).max(1))
        .sum();
    // Border (2), blank lines (2), the dismiss hint (1).
    let wanted = u16::try_from(message_rows + 5).unwrap_or(u16::MAX);
    let popup_height = wanted.clamp(7, area.height.max(7));
    let popup_area = centered_rect(area, popup_width, popup_height);

    let block = titled_block(RED, BorderType::Thick, " Error ".to_string());

    let mut text = vec![Line::from("")];
    text.extend(
        error_msg
            .lines()
            .map(|l| Line::from(Span::styled(l.to_string(), Style::default().fg(FG)))),
    );
    text.extend([
        Line::from(""),
        Line::from(Span::styled(
            "Press any key to dismiss",
            Style::default().fg(MUTED),
        )),
    ]);

    let inner = open_overlay(frame, popup_area, block);

    let paragraph = Paragraph::new(text)
        .wrap(Wrap { trim: true })
        .alignment(if multi_line {
            Alignment::Left
        } else {
            Alignment::Center
        });

    frame.render_widget(paragraph, inner);
}
