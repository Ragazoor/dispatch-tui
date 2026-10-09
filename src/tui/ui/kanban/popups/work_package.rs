//! The work-package link picker and the package rename input
//! (`docs/specs/work-packages.allium`: `WorkPackageLinkPicker`,
//! `RenamePackageFromBoard`).

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{BorderType, Paragraph},
    Frame,
};

use crate::models::WorkPackageItem;
use crate::palette::{CYAN, FG, MUTED};
use crate::tui::ui::shared::{
    caret_line, centered_rect, open_overlay, scroll_offset, titled_block, truncate,
};
use crate::tui::{App, InputMode};

/// Rows the block's borders take.
const BORDER_ROWS: u16 = 2;

/// Both overlays' width: 60% of the board, kept between 30 and 70 columns.
fn overlay_width(area: Rect) -> u16 {
    (area.width * 60 / 100).clamp(30, 70)
}

/// The title field line, the caret drawn where `app.input.caret` puts it and
/// a long title scrolled to keep it visible (`shared::caret_line`).
fn title_field_line(app: &App, overlay_width: u16) -> Line<'static> {
    const LABEL: &str = "Title: ";
    let value_width = usize::from(overlay_width.saturating_sub(BORDER_ROWS))
        .saturating_sub(LABEL.len())
        .max(1);
    caret_line(
        LABEL.to_string(),
        &app.input.buffer,
        app.input.caret,
        value_width,
        Style::default().fg(FG),
    )
}

/// Draw the link picker while it, or its new-package title input, is open.
pub(in crate::tui::ui::kanban) fn render_work_package_picker_overlay(
    frame: &mut Frame,
    app: &App,
    area: Rect,
) {
    let (item, cursor, typing) = match app.input.mode {
        InputMode::LinkWorkPackage { item, cursor } => (item, cursor, false),
        InputMode::InputWorkPackageTitle(item) => (item, app.board.work_packages.len(), true),
        _ => return,
    };
    let item_title = match item {
        WorkPackageItem::Task(id) => app.board.find_task(id).map(|t| t.title.clone()),
        WorkPackageItem::Epic(id) => app.board.find_epic(id).map(|e| e.title.clone()),
    }
    .unwrap_or_default();

    let width = overlay_width(area);
    let text_width = usize::from(width.saturating_sub(BORDER_ROWS + 6));
    let mut rows: Vec<Line<'static>> = app
        .board
        .work_packages
        .iter()
        .map(|p| {
            let linked = app
                .board
                .work_package_links
                .iter()
                .any(|l| l.work_package_id == p.id && l.item == item);
            let mark = if linked { "[x] " } else { "[ ] " };
            Line::from(vec![
                Span::styled(mark, Style::default().fg(CYAN)),
                Span::styled(truncate(&p.title, text_width), Style::default().fg(FG)),
            ])
        })
        .collect();
    rows.push(Line::from(Span::styled(
        "+ new package",
        Style::default().fg(MUTED),
    )));

    // Rows below the list: the title field while typing, then the hint.
    let footer: Vec<Line<'static>> = if typing {
        vec![
            title_field_line(app, width),
            Line::from(Span::styled(
                "[Enter] create  [Esc] back",
                Style::default().fg(MUTED),
            )),
        ]
    } else {
        vec![Line::from(Span::styled(
            "[Space/Enter] toggle  [Esc] close",
            Style::default().fg(MUTED),
        ))]
    };

    let ceiling = area.height.saturating_sub(4).max(BORDER_ROWS + 2);
    let wanted = u16::try_from(rows.len() + footer.len() + 1)
        .unwrap_or(u16::MAX)
        .saturating_add(BORDER_ROWS);
    let height = wanted.min(ceiling);
    let list_rows = usize::from(height.saturating_sub(BORDER_ROWS))
        .saturating_sub(footer.len() + 1)
        .max(1);
    let scroll = scroll_offset(cursor, rows.len(), list_rows);

    let mut lines: Vec<Line<'static>> = rows
        .into_iter()
        .enumerate()
        .skip(scroll)
        .take(list_rows)
        .map(|(i, line)| {
            let mut spans = vec![Span::raw(if i == cursor { "\u{25b8} " } else { "  " })];
            spans.extend(line.spans);
            let line = Line::from(spans);
            if i == cursor {
                line.style(Style::default().add_modifier(Modifier::BOLD))
            } else {
                line
            }
        })
        .collect();
    lines.push(Line::raw(""));
    lines.extend(footer);

    let title = format!(" Work packages: \"{}\" ", truncate(&item_title, 40));
    let block = titled_block(CYAN, BorderType::Rounded, title);
    let inner = open_overlay(frame, centered_rect(area, width, height), block);
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Draw the one-line rename input over a package card.
pub(in crate::tui::ui::kanban) fn render_work_package_rename_overlay(
    frame: &mut Frame,
    app: &App,
    area: Rect,
) {
    if !matches!(app.input.mode, InputMode::RenameWorkPackage(_)) {
        return;
    }
    let width = overlay_width(area);
    let block = titled_block(
        CYAN,
        BorderType::Rounded,
        " Rename work package ".to_string(),
    );
    let inner = open_overlay(frame, centered_rect(area, width, 3 + BORDER_ROWS), block);
    let lines = vec![
        title_field_line(app, width),
        Line::raw(""),
        Line::from(Span::styled(
            "[Enter] save  [Esc] cancel",
            Style::default().fg(MUTED),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}
