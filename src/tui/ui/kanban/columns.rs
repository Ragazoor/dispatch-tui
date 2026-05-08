//! Kanban column layout and per-column rendering.

use chrono::{DateTime, Utc};
use ratatui::{
    layout::{Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, BorderType, Borders, List, ListItem},
    Frame,
};

use crate::models::{EpicId, TaskStatus};
use crate::tui::{App, ColumnItem, EpicStatsMap, ViewMode};

use super::super::palette::{ARCHIVE_COL_BG, ARCHIVE_STRIPE, MUTED, PURPLE};
use super::super::shared::{render_substatus_header, truncate};
use super::cards::{
    build_task_list_item, card_rule_line, render_epic_header_item, render_epic_item,
};
use super::projects_panel::render_projects_column;
use super::{board_column_constraints, column_bg_color, column_color, render_column_separator};

pub(super) fn render_columns(
    frame: &mut Frame,
    app: &mut App,
    epic_stats: &EpicStatsMap,
    area: Rect,
    now: DateTime<Utc>,
) {
    // In Epic mode, wrap the whole board in a purple rounded border with a
    // subtle purple background hint.
    let board_area = if let ViewMode::Epic {
        epic_id, parent, ..
    } = app.view_mode()
    {
        let title = {
            // Walk the parent chain to collect all ancestor IDs (innermost first),
            // then reverse for display order (root → … → current).
            let mut ids: Vec<EpicId> = vec![*epic_id];
            let mut cursor: &ViewMode = parent.as_ref();
            while let ViewMode::Epic {
                epic_id: pid,
                parent: grandparent,
                ..
            } = cursor
            {
                ids.push(*pid);
                cursor = grandparent.as_ref();
            }
            ids.reverse();
            let segments: Vec<String> = ids
                .iter()
                .map(|id| {
                    app.epics()
                        .iter()
                        .find(|e| e.id == *id)
                        .map(|e| truncate(&e.title, 30))
                        .unwrap_or_default()
                })
                .collect();
            format!(" {} ", segments.join(" > "))
        };
        let block = Block::default()
            .title(title)
            .title_style(Style::default().fg(PURPLE).add_modifier(Modifier::BOLD))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(PURPLE))
            .style(Style::default().bg(Color::Rgb(24, 20, 34)));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        inner
    } else {
        area
    };

    let sel = app.selected_column();
    let all_areas = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(board_column_constraints(sel))
        .split(board_area);

    // Odd indices are 1-char separator areas; even indices are content columns.
    for i in (1..all_areas.len()).step_by(2) {
        render_column_separator(frame, all_areas[i]);
    }

    // Content column areas at even indices: 0, 2, 4, ...
    let content_areas: Vec<Rect> = (0..all_areas.len())
        .step_by(2)
        .map(|i| all_areas[i])
        .collect();
    let mut content_idx = 0usize;

    if sel == 0 {
        render_projects_column(frame, app, content_areas[content_idx]);
        content_idx += 1;
    }

    for (task_col_idx, &status) in TaskStatus::ALL.iter().enumerate() {
        let nav_col = task_col_idx + 1;
        render_task_column(
            frame,
            app,
            content_areas[content_idx],
            now,
            status,
            nav_col,
            epic_stats,
        );
        content_idx += 1;
    }

    if sel == TaskStatus::COLUMN_COUNT + 1 {
        render_archive_column(frame, app, content_areas[content_idx], now);
    }
}

/// Render a single task-status column (Backlog/Running/Review/Done).
///
/// `nav_col` is the navigation column index (1–4) used for focus detection and
/// derives the 0-based array index (`nav_col - 1`) for `selected_row`/`list_states`.
fn render_task_column(
    frame: &mut Frame,
    app: &mut App,
    col_area: Rect,
    now: DateTime<Utc>,
    status: TaskStatus,
    nav_col: usize,
    epic_stats: &EpicStatsMap,
) {
    let col_idx = nav_col - 1;
    let is_focused = app.selected_column() == nav_col;
    let color = column_color(status);
    // In flat view the data layer pre-builds SubstatusLabel items; the renderer
    // must not also inject headers or they'd appear twice.
    let show_headers =
        !app.board.flattened && matches!(status, TaskStatus::Running | TaskStatus::Review);

    let column_items = app.column_items_for_status_with_stats(status, Some(epic_stats));
    let selected_row = app.selected_row()[col_idx];

    let mut list_items: Vec<ListItem> = Vec::new();
    let mut list_selection_idx: Option<usize> = None;
    let mut current_priority: Option<u8> = None;

    let mut selectable_idx: usize = 0;
    for item in column_items.iter() {
        // EpicHeader items are decorative — render immediately, don't affect
        // substatus grouping or cursor selection.
        if let ColumnItem::EpicHeader(epic) = item {
            list_items.push(render_epic_header_item(epic, col_area.width));
            continue;
        }

        if let ColumnItem::SubstatusLabel(label) = item {
            list_items.push(render_substatus_header(label, list_items.is_empty()));
            continue;
        }

        // Substatus grouping headers (Running / Review columns only).
        if show_headers {
            let priority = match item {
                ColumnItem::Task(t) => t.sub_status.column_priority_detached(t.is_detached()),
                ColumnItem::Epic(e) => epic_stats
                    .get(&e.id)
                    .map(|s| s.substatus.column_priority())
                    .unwrap_or(0),
                ColumnItem::EpicHeader(_) | ColumnItem::SubstatusLabel(_) => unreachable!(),
            };
            if Some(priority) != current_priority {
                current_priority = Some(priority);
                let label = match item {
                    ColumnItem::Task(t) => t
                        .sub_status
                        .header_label_detached(t.is_detached())
                        .to_string(),
                    ColumnItem::Epic(e) => epic_stats
                        .get(&e.id)
                        .map(|s| s.substatus.header_label())
                        .unwrap_or_default()
                        .to_string(),
                    ColumnItem::EpicHeader(_) | ColumnItem::SubstatusLabel(_) => unreachable!(),
                };
                list_items.push(render_substatus_header(&label, list_items.is_empty()));
            }
        }

        // Selection: cursor tracks selectable_idx, not the raw list position.
        if selectable_idx == selected_row {
            list_selection_idx = Some(list_items.len());
        }
        let is_cursor = is_focused && !app.on_select_all() && selectable_idx == selected_row;
        selectable_idx += 1;

        list_items.push(match item {
            ColumnItem::Task(task) => {
                build_task_list_item(task, status, app, now, is_cursor, color, col_area.width)
            }
            ColumnItem::Epic(epic) => {
                render_epic_item(epic, is_cursor, app, epic_stats, status, col_area.width)
            }
            ColumnItem::EpicHeader(_) | ColumnItem::SubstatusLabel(_) => unreachable!(),
        });
    }

    if !column_items.is_empty() {
        list_items.push(ListItem::new(card_rule_line(MUTED, col_area.width)));
    }

    let on_select_all = app.on_select_all();
    let sel = app.selection_mut();
    if is_focused {
        *sel.list_states[col_idx].selected_mut() = if on_select_all {
            None
        } else {
            list_selection_idx
        };
    }

    let border_color = if is_focused { color } else { MUTED };
    let block = if is_focused {
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(column_bg_color(status)))
    } else {
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(border_color))
    };
    let inner = block.inner(col_area);
    frame.render_widget(block, col_area);
    frame.render_stateful_widget(List::new(list_items), inner, &mut sel.list_states[col_idx]);
}

fn render_archive_column(frame: &mut Frame, app: &mut App, area: Rect, now: DateTime<Utc>) {
    let archived_epics = app.archived_epics();
    let archived_tasks = app.archived_tasks();
    let sel_row = app.selected_archive_row();
    let color = ARCHIVE_STRIPE;

    let bg_block = Block::default().style(Style::default().bg(ARCHIVE_COL_BG));
    frame.render_widget(bg_block, area);

    // Render epics first, then tasks. Selection still indexes archived tasks
    // (epics are non-interactive in the archive column for now).
    let epic_stats = app.compute_epic_stats();
    let mut items: Vec<ListItem> = archived_epics
        .iter()
        .map(|epic| {
            render_epic_item(
                epic,
                false,
                app,
                &epic_stats,
                TaskStatus::Archived,
                area.width,
            )
        })
        .collect();

    items.extend(archived_tasks.iter().enumerate().map(|(idx, task)| {
        let is_cursor = idx == sel_row;
        build_task_list_item(
            task,
            TaskStatus::Archived,
            app,
            now,
            is_cursor,
            color,
            area.width,
        )
    }));

    if !items.is_empty() {
        items.push(ListItem::new(card_rule_line(MUTED, area.width)));
    }

    let total = archived_epics.len() + archived_tasks.len();
    let title = format!(" Archive ({total}) ");
    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(color).add_modifier(Modifier::BOLD))
        .borders(Borders::TOP)
        .border_style(Style::default().fg(ARCHIVE_STRIPE));
    let list = List::new(items).block(block);
    frame.render_stateful_widget(list, area, &mut app.archive.list_state);
}
