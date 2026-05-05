use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use super::palette::{CYAN, GREEN, MUTED, PURPLE, RED, YELLOW};
use super::shared::truncate;
use crate::models::{EpicId, Learning, LearningKind, LearningScope, ProjectId};
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

    // ── Centered overlay (80% × 80%) ──────────────────────────────────────────
    let overlay_width = (area.width * 80 / 100).clamp(40, 120);
    let overlay_height = (area.height * 80 / 100).clamp(16, 40);
    let x = area.x + (area.width.saturating_sub(overlay_width)) / 2;
    let y = area.y + (area.height.saturating_sub(overlay_height)) / 2;
    let overlay_area = Rect::new(x, y, overlay_width, overlay_height);

    frame.render_widget(Clear, overlay_area);

    let outer_block = Block::default()
        .title(" Learnings ")
        .title_style(
            Style::default()
                .fg(CYAN)
                .add_modifier(Modifier::BOLD),
        )
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(CYAN));

    let inner_area = outer_block.inner(overlay_area);
    frame.render_widget(outer_block, overlay_area);

    // Split inner area: 70% top (list or tree), 30% bottom (detail pane)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(inner_area);

    let top_area = chunks[0];
    let bottom_area = chunks[1];

    match view {
        LearningsView::List => render_list(frame, learnings, selected, top_area),
        LearningsView::Tree => render_tree(frame, app, learnings, tree_state, top_area),
    }

    // ── Detail pane: pick the selected learning based on the active view ──────
    // For Tree view, both selected_learning and scope_node_count are derived from a single borrow.
    let (selected_learning, scope_node_count): (Option<&Learning>, Option<usize>) = match view {
        LearningsView::List => (learnings.get(selected), None),
        LearningsView::Tree => {
            let state = tree_state.borrow();
            let selected_path = state.selected();

            let learning = selected_path
                .last()
                .and_then(|id| id.strip_prefix("learning:"))
                .and_then(|s| s.parse::<i64>().ok())
                .and_then(|id| learnings.iter().find(|l| l.id.0 == id));

            let count = selected_path.last().and_then(|last| {
                if last.starts_with("learning:") || last.is_empty() {
                    return None;
                }
                // Count learnings whose tree identifier matches this scope node.
                // Repo nodes use the full path as identifier (e.g. "repo:/home/user/dispatch"),
                // matching how build_learning_tree constructs node identifiers.
                let n = learnings
                    .iter()
                    .filter(|l| {
                        let identifier = match l.scope {
                            LearningScope::User => "user".to_string(),
                            LearningScope::Task => "tasks".to_string(),
                            LearningScope::Repo => l
                                .scope_ref
                                .as_ref()
                                .map(|r| format!("repo:{r}"))
                                .unwrap_or_default(),
                            LearningScope::Project => l
                                .scope_ref
                                .as_ref()
                                .map(|r| format!("project:{r}"))
                                .unwrap_or_default(),
                            LearningScope::Epic => l
                                .scope_ref
                                .as_ref()
                                .map(|r| format!("epic:{r}"))
                                .unwrap_or_default(),
                        };
                        identifier == *last
                    })
                    .count();
                if n > 0 { Some(n) } else { None }
            });

            (learning, count)
        }
    };

    render_detail(frame, selected_learning, scope_node_count, bottom_area);
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

fn render_detail(
    frame: &mut Frame,
    learning: Option<&Learning>,
    scope_node_count: Option<usize>,
    area: Rect,
) {
    let block = Block::default().borders(Borders::TOP).title(" Detail ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // When a scope node is selected in tree view, show a summary instead of learning detail.
    if let Some(count) = scope_node_count {
        let text = Text::from(Line::from(Span::styled(
            format!("{count} learning{} in this scope", if count == 1 { "" } else { "s" }),
            Style::default().fg(Color::DarkGray),
        )));
        frame.render_widget(Paragraph::new(text), inner);
        return;
    }

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
                    format!("scope:{}", scope_badge(l.scope, l.scope_ref.as_deref())),
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
    learnings: &[Learning],
    app: &App,
) -> Vec<tui_tree_widget::TreeItem<'static, String>> {
    use std::collections::HashMap;

    fn leaf(l: &Learning) -> tui_tree_widget::TreeItem<'static, String> {
        let text = format!(
            "{} {}  \u{2713}{}",
            kind_icon(l.kind),
            truncate(&l.summary, 55),
            l.confirmed_count
        );
        tui_tree_widget::TreeItem::new_leaf(format!("learning:{}", l.id), text)
    }

    // Build lookup maps once — O(N) instead of O(N²)
    let epic_project: HashMap<i64, ProjectId> = app
        .epics()
        .iter()
        .map(|e| (e.id.0, e.project_id))
        .collect();
    let epic_label_map: HashMap<i64, String> = app
        .epics()
        .iter()
        .map(|e| (e.id.0, e.title.clone()))
        .collect();
    let proj_label_map: HashMap<ProjectId, String> = app
        .projects()
        .iter()
        .map(|p| (p.id, p.name.clone()))
        .collect();

    let mut roots: Vec<tui_tree_widget::TreeItem<'static, String>> = Vec::new();

    // --- Global (user-scoped) ---
    let user_leaves: Vec<_> = learnings
        .iter()
        .filter(|l| l.scope == LearningScope::User)
        .map(leaf)
        .collect();
    if !user_leaves.is_empty() {
        roots.push(
            tui_tree_widget::TreeItem::new(
                "user".to_string(),
                format!("Global ({})", user_leaves.len()),
                user_leaves,
            )
            // identifiers are unique: "user", "project:{id}", "epic:{id}", "repo:{path}", "tasks"
            .unwrap(),
        );
    }

    // --- Per-project (project-scoped + epic-scoped nested under project) ---
    let mut project_ids: Vec<ProjectId> = Vec::new();
    for l in learnings {
        match l.scope {
            LearningScope::Project => {
                if let Some(id) = l.scope_ref.as_ref().and_then(|s| s.parse::<i64>().ok()) {
                    let pid = ProjectId(id);
                    if !project_ids.contains(&pid) {
                        project_ids.push(pid);
                    }
                }
            }
            LearningScope::Epic => {
                if let Some(ref sr) = l.scope_ref {
                    if let Ok(eid) = sr.parse::<i64>() {
                        if let Some(&proj_id) = epic_project.get(&eid) {
                            if !project_ids.contains(&proj_id) {
                                project_ids.push(proj_id);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    project_ids.sort_by_key(|p| p.0);

    for pid in project_ids {
        let proj_label = proj_label_map
            .get(&pid)
            .cloned()
            .unwrap_or_else(|| format!("Project {}", pid.0));
        let mut children: Vec<tui_tree_widget::TreeItem<'static, String>> = Vec::new();

        // Direct project-scoped leaves
        for l in learnings.iter().filter(|l| {
            l.scope == LearningScope::Project
                && l.scope_ref.as_deref() == Some(&pid.0.to_string())
        }) {
            children.push(leaf(l));
        }

        // Epic sub-nodes under this project
        let mut epic_ids: Vec<EpicId> = Vec::new();
        for l in learnings.iter().filter(|l| l.scope == LearningScope::Epic) {
            if let Some(ref sr) = l.scope_ref {
                if let Ok(eid) = sr.parse::<i64>() {
                    if epic_project.get(&eid) == Some(&pid) && !epic_ids.contains(&EpicId(eid)) {
                        epic_ids.push(EpicId(eid));
                    }
                }
            }
        }
        epic_ids.sort_by_key(|e| e.0);

        for eid in epic_ids {
            let epic_label = epic_label_map
                .get(&eid.0)
                .cloned()
                .unwrap_or_else(|| format!("Epic {}", eid.0));
            let epic_leaves: Vec<_> = learnings
                .iter()
                .filter(|l| {
                    l.scope == LearningScope::Epic
                        && l.scope_ref.as_deref() == Some(&eid.0.to_string())
                })
                .map(leaf)
                .collect();
            if !epic_leaves.is_empty() {
                children.push(
                    tui_tree_widget::TreeItem::new(
                        format!("epic:{}", eid.0),
                        format!("Epic: {} ({})", epic_label, epic_leaves.len()),
                        epic_leaves,
                    )
                    // identifiers are unique: "user", "project:{id}", "epic:{id}", "repo:{path}", "tasks"
                    .unwrap(),
                );
            }
        }

        if !children.is_empty() {
            roots.push(
                tui_tree_widget::TreeItem::new(
                    format!("project:{}", pid.0),
                    format!("Project: {} ({})", proj_label, children.len()),
                    children,
                )
                // identifiers are unique: "user", "project:{id}", "epic:{id}", "repo:{path}", "tasks"
                .unwrap(),
            );
        }
    }

    // --- Repos (top-level, scope = repo) ---
    let mut repo_paths: Vec<String> = learnings
        .iter()
        .filter(|l| l.scope == LearningScope::Repo)
        .filter_map(|l| l.scope_ref.clone())
        .collect();
    repo_paths.sort();
    repo_paths.dedup();

    for repo_path in repo_paths {
        let leaves: Vec<_> = learnings
            .iter()
            .filter(|l| {
                l.scope == LearningScope::Repo && l.scope_ref.as_deref() == Some(&repo_path)
            })
            .map(leaf)
            .collect();
        let basename = std::path::Path::new(&repo_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&repo_path);
        roots.push(
            tui_tree_widget::TreeItem::new(
                format!("repo:{repo_path}"),
                format!("Repo: {} ({})", basename, leaves.len()),
                leaves,
            )
            // identifiers are unique: "user", "project:{id}", "epic:{id}", "repo:{path}", "tasks"
            .unwrap(),
        );
    }

    // --- Tasks (rare, task-scoped) ---
    let task_leaves: Vec<_> = learnings
        .iter()
        .filter(|l| l.scope == LearningScope::Task)
        .map(leaf)
        .collect();
    if !task_leaves.is_empty() {
        roots.push(
            tui_tree_widget::TreeItem::new(
                "tasks".to_string(),
                format!("Tasks ({})", task_leaves.len()),
                task_leaves,
            )
            // identifiers are unique: "user", "project:{id}", "epic:{id}", "repo:{path}", "tasks"
            .unwrap(),
        );
    }

    roots
}

fn render_tree(
    frame: &mut Frame,
    app: &App,
    learnings: &[Learning],
    tree_state: &std::cell::RefCell<tui_tree_widget::TreeState<String>>,
    area: Rect,
) {
    let items = build_learning_tree(learnings, app);

    // Open all root nodes on first render (if none are open)
    {
        let mut state = tree_state.borrow_mut();
        if state.opened().is_empty() && !items.is_empty() {
            for item in &items {
                state.open(vec![item.identifier().clone()]);
            }
        }
    }

    let title = format!(" Learnings \u{2014} hierarchy view ({}) ", learnings.len());
    let block = Block::default().borders(Borders::ALL).title(title);

    let tree = tui_tree_widget::Tree::new(&items)
        .expect("all learning tree items have unique identifiers")
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    frame.render_stateful_widget(tree, area, &mut tree_state.borrow_mut());

    // Footer hints
    let footer_area = Rect {
        y: area.y + area.height.saturating_sub(1),
        height: 1,
        ..area
    };
    let hints =
        Paragraph::new(" Tab:list  h/l:collapse/expand  j/k:nav  e:edit  x:reject  A:archive  q:close")
            .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hints, footer_area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{LearningId, LearningStatus};
    use crate::tui::App;
    use chrono::Utc;
    use std::time::Duration;

    fn make_learning(id: i64, scope: LearningScope, scope_ref: Option<&str>) -> Learning {
        Learning {
            id: LearningId(id),
            kind: LearningKind::Convention,
            summary: format!("learning {id}"),
            detail: None,
            scope,
            scope_ref: scope_ref.map(|s| s.to_string()),
            tags: vec![],
            status: LearningStatus::Approved,
            source_task_id: None,
            confirmed_count: 0,
            last_confirmed_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_app() -> App {
        App::new(vec![], ProjectId(1), Duration::from_secs(300))
    }

    #[test]
    fn build_learning_tree_groups_user_under_global() {
        let learnings = vec![make_learning(1, LearningScope::User, None)];
        let app = make_app();
        let tree = build_learning_tree(&learnings, &app);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].identifier(), "user");
    }

    #[test]
    fn build_learning_tree_two_user_learnings_under_one_global_node() {
        let learnings = vec![
            make_learning(1, LearningScope::User, None),
            make_learning(2, LearningScope::User, None),
        ];
        let app = make_app();
        let tree = build_learning_tree(&learnings, &app);
        assert_eq!(tree.len(), 1, "both user learnings under one Global node");
        assert_eq!(tree[0].children().len(), 2);
    }

    #[test]
    fn build_learning_tree_repo_at_top_level() {
        let learnings = vec![make_learning(
            20,
            LearningScope::Repo,
            Some("/home/user/dispatch"),
        )];
        let app = make_app();
        let tree = build_learning_tree(&learnings, &app);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].identifier(), "repo:/home/user/dispatch");
    }

    #[test]
    fn build_learning_tree_empty_returns_empty() {
        let app = make_app();
        let tree = build_learning_tree(&[], &app);
        assert!(tree.is_empty());
    }

    #[test]
    fn build_learning_tree_task_scoped_in_tasks_node() {
        let learnings = vec![make_learning(5, LearningScope::Task, Some("42"))];
        let app = make_app();
        let tree = build_learning_tree(&learnings, &app);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].identifier(), "tasks");
        assert_eq!(tree[0].children().len(), 1);
    }

    #[test]
    fn build_learning_tree_multiple_repos_separate_nodes() {
        let learnings = vec![
            make_learning(1, LearningScope::Repo, Some("/repo/a")),
            make_learning(2, LearningScope::Repo, Some("/repo/b")),
        ];
        let app = make_app();
        let tree = build_learning_tree(&learnings, &app);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].identifier(), "repo:/repo/a");
        assert_eq!(tree[1].identifier(), "repo:/repo/b");
    }

    #[test]
    fn build_learning_tree_project_scoped_without_matching_epic() {
        let learnings = vec![make_learning(1, LearningScope::Project, Some("1"))];
        let app = make_app();
        let tree = build_learning_tree(&learnings, &app);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].identifier(), "project:1");
        assert_eq!(tree[0].children().len(), 1);
    }
}
