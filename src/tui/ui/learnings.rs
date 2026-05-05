use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use super::palette::{CYAN, GREEN, MUTED, PURPLE, RED, YELLOW};
use super::shared::truncate;
use crate::models::{Learning, LearningKind, LearningScope};
use crate::tui::types::LearningsView;
use crate::tui::{App, ViewMode};

fn kind_icon(kind: LearningKind) -> &'static str {
    match kind {
        LearningKind::Pitfall => "[!]",
        LearningKind::Convention => "[->]",
        LearningKind::Preference => "[h]",
        LearningKind::ToolRecommendation => "[T]",
        LearningKind::Procedural => "[P]",
        LearningKind::Episodic => "[E]",
    }
}

fn kind_color(kind: LearningKind) -> Style {
    match kind {
        LearningKind::Pitfall => Style::default().fg(RED),
        LearningKind::Convention => Style::default().fg(CYAN),
        LearningKind::Preference => Style::default().fg(PURPLE),
        LearningKind::ToolRecommendation => Style::default().fg(GREEN),
        LearningKind::Procedural => Style::default().fg(YELLOW),
        LearningKind::Episodic => Style::default().fg(MUTED),
    }
}

fn scope_badge(scope: LearningScope, scope_ref: Option<&str>) -> String {
    match scope {
        LearningScope::User => "global".to_string(),
        LearningScope::Project => format!("project:{}", scope_ref.unwrap_or("?")),
        LearningScope::Repo => {
            let basename = scope_ref
                .and_then(|p| std::path::Path::new(p).file_name()?.to_str())
                .unwrap_or("?");
            format!("repo:{basename}")
        }
        LearningScope::Epic => format!("epic:{}", scope_ref.unwrap_or("?")),
        LearningScope::Task => format!("task:{}", scope_ref.unwrap_or("?")),
    }
}

fn scope_label(scope: LearningScope, scope_ref: Option<&str>) -> String {
    scope_badge(scope, scope_ref)
}

pub fn render_learnings(frame: &mut Frame, app: &App, area: Rect) {
    let ViewMode::Learnings {
        selected,
        ref learnings,
        view,
        ref tree_state,
        ..
    } = app.board.view_mode
    else {
        return;
    };

    // Split: 70% top (list or tree), 30% bottom (detail pane)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(area);

    let top_area = chunks[0];
    let bottom_area = chunks[1];

    match view {
        LearningsView::List => render_list(frame, learnings, selected, top_area),
        LearningsView::Tree => render_tree(frame, app, learnings, tree_state, top_area),
    }

    let selected_learning = learnings.get(selected);
    render_detail(frame, selected_learning, bottom_area);
}

fn render_list(frame: &mut Frame, learnings: &[Learning], selected: usize, area: Rect) {
    let title = format!(" Learnings ({}) \u{2014} sorted by use ", learnings.len());
    let block = Block::default().borders(Borders::ALL).title(title);

    let items: Vec<ListItem> = learnings
        .iter()
        .map(|l| {
            let icon = kind_icon(l.kind);
            let badge = scope_badge(l.scope, l.scope_ref.as_deref());
            let line = Line::from(vec![
                Span::styled(icon, kind_color(l.kind)),
                Span::raw(" "),
                Span::raw(truncate(&l.summary, 55)),
                Span::raw("  "),
                Span::styled(
                    format!("\u{2713}{}", l.confirmed_count),
                    Style::default().fg(Color::Green),
                ),
                Span::raw("  "),
                Span::styled(badge, Style::default().fg(Color::DarkGray)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(selected));

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    frame.render_stateful_widget(list, area, &mut list_state);

    // Footer hints (last row inside the area)
    let footer_area = Rect {
        y: area.y + area.height.saturating_sub(1),
        height: 1,
        ..area
    };
    let hints = Paragraph::new(" Tab:tree  j/k:nav  e:edit  x:reject  A:archive  q:close")
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hints, footer_area);
}

fn render_detail(frame: &mut Frame, learning: Option<&Learning>, area: Rect) {
    let block = Block::default().borders(Borders::TOP).title(" Detail ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let text = match learning {
        None => Text::raw("No learning selected"),
        Some(l) => {
            let mut lines = vec![Line::from(vec![
                Span::styled(kind_icon(l.kind), kind_color(l.kind)),
                Span::raw(" "),
                Span::styled(
                    l.summary.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ])];
            if let Some(detail) = &l.detail {
                lines.push(Line::raw(""));
                lines.push(Line::raw(detail.clone()));
            }
            lines.push(Line::raw(""));
            lines.push(Line::from(vec![
                Span::styled(
                    format!("scope:{}", scope_label(l.scope, l.scope_ref.as_deref())),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("confirmed:{}", l.confirmed_count),
                    Style::default().fg(Color::Green),
                ),
            ]));
            if !l.tags.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("tags: {}", l.tags.join(", ")),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            Text::from(lines)
        }
    };

    frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), inner);
}

pub fn build_learning_tree(
    _learnings: &[Learning],
    _app: &App,
) -> Vec<tui_tree_widget::TreeItem<'static, String>> {
    vec![]
}

fn render_tree(
    frame: &mut Frame,
    _app: &App,
    _learnings: &[Learning],
    _tree_state: &std::cell::RefCell<tui_tree_widget::TreeState<String>>,
    area: Rect,
) {
    // Stub: will be implemented in Task 12
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Learnings \u{2014} tree view (coming soon) ");
    frame.render_widget(block, area);
}
