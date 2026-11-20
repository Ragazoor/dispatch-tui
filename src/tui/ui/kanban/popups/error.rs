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

/// The rows a line takes when wrapped at word boundaries to `width`, as the
/// popup's paragraph wraps it. A word longer than the width is split.
fn wrapped_rows(line: &str, width: usize) -> usize {
    let mut rows = 1;
    let mut used = 0;
    for word in line.split_whitespace() {
        let len = word.chars().count();
        let needed = if used == 0 { len } else { used + 1 + len };
        if needed <= width {
            used = needed;
        } else if len <= width {
            rows += usize::from(used > 0);
            used = len;
        } else {
            rows += usize::from(used > 0) + (len - 1) / width;
            used = (len - 1) % width + 1;
        }
    }
    rows
}

pub(in crate::tui::ui::kanban) fn render_error_popup(frame: &mut Frame, app: &App, area: Rect) {
    let Some(error_msg) = app.error_popup() else {
        return;
    };

    // A message of several lines (a key-table load's refusals) is a list:
    // each line its own row, left-aligned, the popup as tall as it needs.
    let multi_line = error_msg.contains('\n');
    let popup_width = if multi_line {
        area.width.saturating_sub(4).clamp(30, 100)
    } else {
        (area.width * 60 / 100).clamp(30, 60)
    };
    let inner_width = usize::from(popup_width.saturating_sub(2)).max(1);
    let message_rows: usize = error_msg
        .lines()
        .map(|l| wrapped_rows(l, inner_width))
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
