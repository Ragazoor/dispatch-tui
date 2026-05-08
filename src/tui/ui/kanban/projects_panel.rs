//! Left-side project filter panel.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
    Frame,
};

use crate::models::{Project, ProjectId, TaskStatus};
use crate::tui::App;

use super::super::palette::{FG, MUTED, PROJECTS_COL_BG, PROJECTS_CURSOR_BG, PURPLE};
use super::super::shared::truncate;
use super::cards::card_rule_line;

fn build_project_list_item<'a>(
    project: &Project,
    task_count: usize,
    is_cursor: bool,
    is_active: bool,
    col_width: u16,
) -> ListItem<'a> {
    let stripe_color = if is_active {
        PURPLE
    } else {
        Color::Rgb(120, 100, 160)
    };
    let stripe = if is_cursor { "▌ " } else { "▎ " };

    let name = truncate(&project.name, col_width.saturating_sub(4) as usize);
    let active_marker = if is_active { " ◉" } else { "" };
    let name_line = Line::from(vec![
        Span::styled(stripe, Style::default().fg(stripe_color)),
        Span::styled(
            format!("{}{}", name, active_marker),
            if is_cursor {
                Style::default()
                    .bg(PROJECTS_CURSOR_BG)
                    .fg(FG)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(FG)
            },
        ),
    ]);

    let meta_line = Line::from(Span::styled(
        format!("   {} tasks", task_count),
        Style::default().fg(MUTED),
    ));

    let rule_color = if is_cursor { stripe_color } else { MUTED };
    let rule_line = card_rule_line(rule_color, col_width);
    ListItem::new(vec![rule_line, name_line, meta_line])
}

pub(super) fn render_projects_column(frame: &mut Frame, app: &mut App, area: Rect) {
    let sel_row = app.selected_project_row();
    let active_project = app.active_project();

    let bg_block = Block::default().style(Style::default().bg(PROJECTS_COL_BG));
    frame.render_widget(bg_block, area);

    let task_counts: std::collections::HashMap<ProjectId, usize> = app
        .tasks()
        .iter()
        .filter(|t| t.status != TaskStatus::Archived)
        .fold(std::collections::HashMap::new(), |mut acc, t| {
            *acc.entry(t.project_id).or_insert(0) += 1;
            acc
        });

    let mut items: Vec<ListItem> = app
        .projects()
        .iter()
        .enumerate()
        .map(|(idx, project)| {
            let task_count = task_counts.get(&project.id).copied().unwrap_or(0);
            build_project_list_item(
                project,
                task_count,
                idx == sel_row,
                project.id == active_project,
                area.width,
            )
        })
        .collect();
    if !items.is_empty() {
        items.push(ListItem::new(card_rule_line(MUTED, area.width)));
    }

    let title = format!(" Projects ({}) ", app.projects().len());
    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(PURPLE).add_modifier(Modifier::BOLD))
        .borders(Borders::TOP)
        .border_style(Style::default().fg(PURPLE));
    let list = List::new(items).block(block);
    frame.render_stateful_widget(list, area, &mut app.projects_panel.list_state);
}
