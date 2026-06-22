//! Personal TODO overlay — render function.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use super::palette::CYAN;
use crate::tui::{App, ViewMode};

/// Checkbox glyph for an open (not-done) todo.
const OPEN_GLYPH: &str = "▢";
/// Checkbox glyph for a completed (done) todo.
const DONE_GLYPH: &str = "▣";

pub fn render_todos(frame: &mut Frame, app: &App, area: Rect) {
    let ViewMode::Todos {
        ref todos,
        selected,
        ..
    } = app.board.view_mode
    else {
        return;
    };

    // ── Centered overlay (70% × 70%) ──────────────────────────────────────────
    let overlay_width = (area.width * 70 / 100).clamp(40, 100);
    let overlay_height = (area.height * 70 / 100).clamp(12, 35);
    let x = area.x + (area.width.saturating_sub(overlay_width)) / 2;
    let y = area.y + (area.height.saturating_sub(overlay_height)) / 2;
    let overlay_area = Rect::new(x, y, overlay_width, overlay_height);

    frame.render_widget(Clear, overlay_area);

    let open_count = todos.iter().filter(|t| !t.done).count();
    let title = format!(" TODO ({open_count} open) ");

    let outer_block = Block::default()
        .title(title)
        .title_style(Style::default().fg(CYAN).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(CYAN));

    let inner_area = outer_block.inner(overlay_area);
    frame.render_widget(outer_block, overlay_area);

    // Reserve one row for the footer hints.
    let list_height = inner_area.height.saturating_sub(1);
    let list_area = Rect {
        height: list_height,
        ..inner_area
    };

    let items: Vec<ListItem> = todos
        .iter()
        .map(|todo| {
            if todo.done {
                // Done items: dimmed + strikethrough
                ListItem::new(Line::from(vec![
                    Span::styled(
                        DONE_GLYPH,
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::DIM),
                    ),
                    Span::raw(" "),
                    Span::styled(
                        todo.title.clone(),
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::DIM | Modifier::CROSSED_OUT),
                    ),
                ]))
            } else {
                ListItem::new(Line::from(vec![
                    Span::styled(OPEN_GLYPH, Style::default().fg(CYAN)),
                    Span::raw(" "),
                    Span::raw(todo.title.clone()),
                ]))
            }
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(if todos.is_empty() {
        None
    } else {
        Some(selected)
    });

    let list = List::new(items).highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    frame.render_stateful_widget(list, list_area, &mut list_state);

    // Footer hints
    let footer_area = Rect {
        y: inner_area.y + inner_area.height.saturating_sub(1),
        height: 1,
        ..inner_area
    };
    let hints = Paragraph::new(" a add · space done · J/K order · e edit · c clear-done · d del · q back")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hints, footer_area);
}
