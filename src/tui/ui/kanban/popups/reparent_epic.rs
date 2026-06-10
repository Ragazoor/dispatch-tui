//! Reparent-epic tree picker overlay.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Text,
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};
use std::collections::HashSet;

use crate::models::{descendant_epic_ids, Epic, EpicId};
use crate::tui::{App, InputMode};

const NO_PARENT_ID: &str = "__no_parent__";

pub(in crate::tui::ui::kanban) fn render_reparent_epic_overlay(
    frame: &mut Frame,
    app: &App,
    area: Rect,
) {
    let picker = match &app.reparent_picker {
        Some(p)
            if matches!(
                app.input.mode,
                InputMode::ReparentEpic(_) | InputMode::ConfirmReparentEpic { .. }
            ) =>
        {
            p
        }
        _ => return,
    };

    let excluded = descendant_epic_ids(picker.epic_id, &app.board.epics);
    let items = build_reparent_tree(&app.board.epics, &excluded);

    let overlay_width = (area.width * 50 / 100).clamp(30, 80);
    let overlay_height = area.height.saturating_sub(4).max(10);
    let x = area.x + (area.width.saturating_sub(overlay_width)) / 2;
    let y = area.y + 2;
    let overlay_area = Rect::new(x, y, overlay_width, overlay_height);

    frame.render_widget(Clear, overlay_area);

    let title = app
        .board
        .epics
        .iter()
        .find(|e| e.id == picker.epic_id)
        .map(|e| format!(" Reparent epic: \"{}\" ", e.title))
        .unwrap_or_else(|| " Reparent epic ".to_string());

    let block = Block::default()
        .title(title)
        .title_style(
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        )
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Magenta));

    let inner_area = block.inner(overlay_area);
    frame.render_widget(block, overlay_area);

    // Split inner area: tree above, hint line at bottom
    let footer_area = Rect {
        y: inner_area.y + inner_area.height.saturating_sub(1),
        height: 1,
        ..inner_area
    };
    let tree_area = Rect {
        height: inner_area.height.saturating_sub(1),
        ..inner_area
    };

    let hints = Paragraph::new(
        " j/k:navigate  l/Space/\u{2192}:expand  h/\u{2190}:collapse  Enter:select  q/Esc:cancel",
    )
    .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hints, footer_area);

    let tree = match tui_tree_widget::Tree::new(&items) {
        Ok(t) => t
            .block(Block::default())
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        Err(_) => return,
    };

    frame.render_stateful_widget(tree, tree_area, &mut picker.tree_state.borrow_mut());
}

fn build_reparent_tree(
    epics: &[Epic],
    excluded: &HashSet<EpicId>,
) -> Vec<tui_tree_widget::TreeItem<'static, String>> {
    let no_parent = tui_tree_widget::TreeItem::new_leaf(
        NO_PARENT_ID.to_string(),
        Text::raw("— no parent —"),
    );
    let valid: Vec<&Epic> = epics.iter().filter(|e| !excluded.contains(&e.id)).collect();
    let mut items = vec![no_parent];
    items.extend(build_epic_nodes(&valid, None));
    items
}

fn build_epic_nodes(
    epics: &[&Epic],
    parent_id: Option<EpicId>,
) -> Vec<tui_tree_widget::TreeItem<'static, String>> {
    epics
        .iter()
        .filter(|e| e.parent_epic_id == parent_id)
        .filter_map(|e| {
            let children = build_epic_nodes(epics, Some(e.id));
            let id = format!("epic:{}", e.id.0);
            if children.is_empty() {
                Some(tui_tree_widget::TreeItem::new_leaf(
                    id,
                    Text::from(e.title.clone()),
                ))
            } else {
                tui_tree_widget::TreeItem::new(id, Text::from(e.title.clone()), children).ok()
            }
        })
        .collect()
}
