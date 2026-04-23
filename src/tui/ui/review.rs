use super::palette::{BLUE, BORDER, DIM_META, GREEN, MUTED, MUTED_LIGHT, RED_DIM, YELLOW};
use super::shared::{
    push_hint_spans, refresh_status, render_substatus_header, render_tab_bar, staleness_color,
    truncate,
};

use crate::models::{format_age, CiStatus, ReviewDecision, ReviewPr, Staleness};
use crate::tui::{App, InputMode, ReviewBoardMode, ViewMode};
use chrono::Utc;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

fn render_review_repo_filter_overlay(frame: &mut Frame, app: &App, area: Rect) {
    let repos = app.active_review_repos();
    let repo_count = repos.len();
    let popup_height = (repo_count as u16 + 5).clamp(7, area.height.saturating_sub(4));
    let popup_width = (area.width * 70 / 100).clamp(30, 60);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    let review_mode_label = app.review_repo_filter_mode().as_str();
    let block = Block::default()
        .title(format!(" Review Repo Filter ({review_mode_label}) "))
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(Color::Cyan))
        .title_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

    let key_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(Color::Gray);
    let note_style = Style::default().fg(Color::DarkGray);

    let mut lines = vec![Line::from("")];

    for (i, repo) in repos.iter().enumerate() {
        let num = i + 1;
        let checked = if app.review_repo_filter().contains(repo) {
            "x"
        } else {
            " "
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {num}"), key_style),
            Span::styled(format!(". [{checked}] {repo}"), desc_style),
        ]));
    }

    lines.push(Line::from(""));

    let all_selected = !repos.is_empty() && app.review_repo_filter().len() == repos.len();
    let a_label = if all_selected {
        "clear all"
    } else {
        "select all"
    };
    lines.push(Line::from(vec![
        Span::styled("  [a]", key_style),
        Span::styled(format!(" {a_label}  "), note_style),
        Span::styled("[Tab]", key_style),
        Span::styled(" incl/excl  ", note_style),
        Span::styled("[Enter/Esc]", key_style),
        Span::styled(" close", note_style),
    ]));

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, popup_area);
}

// ---------------------------------------------------------------------------
// Review board rendering
// ---------------------------------------------------------------------------

pub(in crate::tui) fn review_action_hints(
    has_pr: bool,
    is_author_mode: bool,
    agent_status: Option<crate::models::ReviewAgentStatus>,
) -> Vec<Span<'static>> {
    use crate::models::ReviewAgentStatus;
    let key_color = Color::Cyan;
    let label_style = Style::default().fg(MUTED);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut push_hint = |key: &'static str, label: &'static str| {
        push_hint_spans(&mut spans, key, label, key_color, label_style);
    };
    if has_pr {
        push_hint("Enter", "open PR");
    }
    match agent_status {
        Some(ReviewAgentStatus::Idle) => {
            push_hint("g", "go to");
            push_hint("d", "resume");
            push_hint("T", "detach");
        }
        Some(_) => {
            push_hint("g", "go to");
            push_hint("T", "detach");
        }
        None => {
            if has_pr {
                push_hint("d", "dispatch");
            }
        }
    }
    push_hint("f", "filter");
    push_hint("e", "edit queries");
    if is_author_mode {
        push_hint("D", "dispatch filter");
    }
    if has_pr {
        if !is_author_mode {
            push_hint("a", "approve");
        }
        push_hint("m", "merge");
    }
    push_hint("1/2", "mode");
    push_hint("Tab", "task board");
    push_hint("?", "help");
    push_hint("q", "quit");
    spans
}

pub(in crate::tui) fn review_column_color(decision: ReviewDecision) -> Color {
    match decision {
        ReviewDecision::ReviewRequired => BLUE,
        ReviewDecision::WaitingForResponse => YELLOW,
        ReviewDecision::ChangesRequested => RED_DIM,
        ReviewDecision::Approved => GREEN,
    }
}

pub(in crate::tui) fn review_cursor_bg_color(decision: ReviewDecision) -> Color {
    match decision {
        ReviewDecision::ReviewRequired => Color::Rgb(34, 38, 66),
        ReviewDecision::WaitingForResponse => Color::Rgb(52, 44, 20),
        ReviewDecision::ChangesRequested => Color::Rgb(56, 32, 32),
        ReviewDecision::Approved => Color::Rgb(32, 52, 36),
    }
}

pub(in crate::tui) fn review_column_bg_color(decision: ReviewDecision) -> Color {
    match decision {
        ReviewDecision::ReviewRequired => Color::Rgb(28, 30, 44),
        ReviewDecision::WaitingForResponse => Color::Rgb(36, 34, 26),
        ReviewDecision::ChangesRequested => Color::Rgb(36, 28, 28),
        ReviewDecision::Approved => Color::Rgb(27, 36, 30),
    }
}

/// Render the review board view.
pub fn render_review_board(frame: &mut Frame, app: &mut App, area: Rect) {
    let detail_height = if app.review_detail_visible() { 8 } else { 0 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),             // tab bar
            Constraint::Length(1),             // summary row
            Constraint::Length(1),             // refresh status row
            Constraint::Min(1),                // board
            Constraint::Length(detail_height), // detail panel
            Constraint::Length(1),             // status bar
        ])
        .split(area);

    render_tab_bar(frame, app, chunks[0]);
    render_review_summary_row(frame, app, chunks[1]);

    // Refresh status row
    let (last_fetch, loading) = match app.view_mode() {
        ViewMode::ReviewBoard {
            mode: ReviewBoardMode::Dependabot,
            ..
        } => (app.review_bot_prs_last_fetch(), app.review_bot_prs_loading()),
        _ => (app.review_last_fetch(), app.review_board_loading()),
    };
    let (status_text, status_color) =
        refresh_status(last_fetch, loading, crate::tui::REVIEW_REFRESH_INTERVAL);
    frame.render_widget(
        Paragraph::new(status_text).style(Style::default().fg(status_color)),
        chunks[2],
    );

    let filtered = app.active_review_prs();
    if filtered.is_empty() {
        let is_empty = match app.view_mode() {
            ViewMode::ReviewBoard {
                mode: ReviewBoardMode::Dependabot,
                ..
            } => app.review_bot_prs().is_empty(),
            _ => app.review_prs().is_empty(),
        };
        let msg = if is_empty {
            "No PRs found"
        } else {
            "All PRs filtered out."
        };
        let p = Paragraph::new(msg)
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(p, chunks[3]);
    } else {
        render_review_columns(frame, app, chunks[3]);
    }

    render_review_detail(frame, app, chunks[4]);

    // Status bar: transient message takes priority; fall back to persistent error
    if let Some(msg) = app.status.message.as_deref() {
        let status = Paragraph::new(msg.to_string()).style(Style::default().fg(Color::Yellow));
        frame.render_widget(status, chunks[5]);
    } else if let Some(err) = app.last_review_error() {
        let status = Paragraph::new(format!("Error: {err}")).style(Style::default().fg(Color::Red));
        frame.render_widget(status, chunks[5]);
    } else if app.has_bot_pr_selection() {
        let count = app.selected_bot_prs().len();
        let text = format!("{count} selected  [A] approve  [m] merge  [Esc] clear");
        let status = Paragraph::new(text).style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_widget(status, chunks[5]);
    } else {
        let has_pr = app.selected_review_pr().is_some();
        let is_author_mode = false; // Author mode removed in v2
        let agent_status = app
            .selected_review_pr()
            .and_then(|pr| app.pr_agent(pr).map(|h| h.status));
        let hints = Paragraph::new(Line::from(review_action_hints(
            has_pr,
            is_author_mode,
            agent_status,
        )));
        frame.render_widget(hints, chunks[5]);
    }

    // Filter overlay (on top of everything)
    if matches!(app.mode(), InputMode::ReviewRepoFilter) {
        render_review_repo_filter_overlay(frame, app, area);
    }
}

fn render_review_detail(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(BORDER));

    if !app.review_detail_visible() {
        let paragraph = Paragraph::new("").block(block);
        frame.render_widget(paragraph, area);
        return;
    }

    let Some(pr) = app.selected_review_pr() else {
        let paragraph = Paragraph::new("").block(block);
        frame.render_widget(paragraph, area);
        return;
    };

    let decision_color = review_column_color(pr.review_decision);
    let now = Utc::now();
    let age = format_age(pr.created_at, now);

    // Line 1: title + CI status
    let ci_color = match pr.ci_status {
        CiStatus::Success => Color::Green,
        CiStatus::Failure => Color::Red,
        CiStatus::Pending => Color::Yellow,
        CiStatus::None => Color::DarkGray,
    };
    let ci_label = match pr.ci_status {
        CiStatus::Success => "passing",
        CiStatus::Failure => "failing",
        CiStatus::Pending => "pending",
        CiStatus::None => "no checks",
    };
    let line1 = Line::from(vec![
        Span::styled(
            format!("{}#{} {}", pr.repo, pr.number, pr.title),
            Style::default()
                .fg(decision_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  CI: {} {ci_label}", pr.ci_status.symbol()),
            Style::default().fg(ci_color),
        ),
    ]);

    // Line 2: metadata
    let line2 = Line::from(Span::styled(
        format!(
            "@{} \u{00b7} {} \u{00b7} +{}/-{}",
            pr.author, age, pr.additions, pr.deletions
        ),
        Style::default().fg(MUTED),
    ));

    // Line 3: reviewer list
    let reviewer_spans: Vec<String> = pr
        .reviewers
        .iter()
        .map(|r| {
            let icon = match r.decision {
                Some(ReviewDecision::Approved) => "\u{2713}",
                Some(ReviewDecision::ChangesRequested) => "\u{2717}",
                _ => "\u{23f3}",
            };
            format!("@{} {icon}", r.login)
        })
        .collect();
    let reviewer_line = if reviewer_spans.is_empty() {
        "No reviewers".to_string()
    } else {
        format!("Reviews: {}", reviewer_spans.join(" \u{00b7} "))
    };
    let line3 = Line::from(Span::styled(
        reviewer_line,
        Style::default().fg(MUTED_LIGHT),
    ));

    // Lines 4+: PR body (truncated to fit remaining space)
    let body_lines: Vec<Line> = pr
        .body
        .lines()
        .take(5)
        .map(|l| {
            Line::from(Span::styled(
                l.to_string(),
                Style::default().fg(Color::DarkGray),
            ))
        })
        .collect();

    let mut lines = vec![line1, line2, line3];
    lines.extend(body_lines);

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn render_review_summary_row(frame: &mut Frame, app: &App, area: Rect) {
    use crate::models::ReviewWorkflowState;
    let sel = app.review_selection();
    let selected_col = sel.map(|s| s.column()).unwrap_or(0);
    let filtered = app.active_review_prs();
    let col_count = ReviewBoardMode::column_count();
    let workflow_states = [
        ReviewWorkflowState::Backlog,
        ReviewWorkflowState::Ongoing,
        ReviewWorkflowState::ActionRequired,
        ReviewWorkflowState::Done,
    ];

    let segments = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(vec![Constraint::Ratio(1, col_count as u32); col_count])
        .split(area);

    for i in 0..col_count {
        let count = filtered.iter().filter(|pr| pr.review_decision.column_index() == i).count();
        let is_focused = i == selected_col;
        let prefix = if is_focused { "\u{25b8} " } else { "\u{25e6} " };
        let col_label = workflow_states.get(i).map(|s| ReviewBoardMode::column_label(*s)).unwrap_or("");
        let label = format!("{prefix}{col_label} ({count})");

        // Map column index to ReviewDecision for coloring
        let decision_for_color =
            ReviewDecision::from_column_index(i).unwrap_or(ReviewDecision::ReviewRequired);

        let color = review_column_color(decision_for_color);
        let style = if is_focused {
            Style::default().fg(color).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let p = Paragraph::new(label).style(style);
        frame.render_widget(p, segments[i]);
    }
}

fn render_review_columns(frame: &mut Frame, app: &mut App, area: Rect) {
    let sel_col = app.review_selection().map(|s| s.column()).unwrap_or(0);
    let col_count = ReviewBoardMode::column_count();

    let col_areas = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(vec![Constraint::Ratio(1, col_count as u32); col_count])
        .split(area);

    for i in 0..col_count {
        let is_focused = i == sel_col;
        let prs: Vec<&ReviewPr> = app.active_prs_for_column(i);

        let decision_for_color =
            ReviewDecision::from_column_index(i).unwrap_or(ReviewDecision::ReviewRequired);
        let selected_row = app.review_selection().map(|s| s.row(i)).unwrap_or(0);
        let mut list_items: Vec<ListItem> = Vec::new();
        let mut list_selection_idx: Option<usize> = None;
        let mut current_repo: Option<&str> = None;

        for (item_idx, pr) in prs.iter().enumerate() {
            if current_repo != Some(pr.repo.as_str()) {
                current_repo = Some(pr.repo.as_str());
                let repo_short = pr.repo.split('/').next_back().unwrap_or(&pr.repo);
                list_items.push(render_substatus_header(repo_short, list_items.is_empty()));
            }

            if item_idx == selected_row {
                list_selection_idx = Some(list_items.len());
            }

            let is_selected = false;
            list_items.push(build_review_pr_item(
                pr,
                i,
                is_focused && item_idx == selected_row,
                app.pr_agent(pr).map(|h| h.status),
                is_selected,
                col_areas[i].width,
            ));
        }

        let bg = if is_focused {
            review_column_bg_color(decision_for_color)
        } else {
            Color::Reset
        };

        let list = List::new(list_items).block(Block::default().style(Style::default().bg(bg)));

        let mut list_state = ListState::default();
        if is_focused {
            list_state.select(list_selection_idx);
        }

        frame.render_stateful_widget(list, col_areas[i], &mut list_state);

        // Write back the list state for scroll tracking
        if let Some(sel) = app.review_selection_mut() {
            sel.list_states[i] = list_state;
        }
    }
}

fn build_pr_line1(
    pr: &ReviewPr,
    color: Color,
    is_selected: bool,
    is_cursor: bool,
    agent_status: Option<crate::models::ReviewAgentStatus>,
    col_width: u16,
) -> Line<'static> {
    let select_prefix = if is_selected { "* " } else { "" };
    let stripe = if is_cursor { "\u{258c} " } else { "\u{258e} " };

    let (badge_text, badge_is_running) = match agent_status {
        Some(crate::models::ReviewAgentStatus::Reviewing) => ("\u{25c9} ", true),
        Some(crate::models::ReviewAgentStatus::FindingsReady) => ("\u{2714} ", false),
        Some(crate::models::ReviewAgentStatus::Idle) => ("\u{25cb} ", false),
        None => ("", false),
    };

    let header = format!("{select_prefix}#{} {}", pr.number, pr.title);
    // stripe(2) + badge(0 or 2) + header + " ●"(2)
    let badge_w = if badge_text.is_empty() { 0 } else { 2 };
    let max_header = (col_width as usize).saturating_sub(4 + badge_w);
    let header_truncated = truncate(&header, max_header);

    let line1_style = if is_selected || is_cursor {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(color)
    };
    let badge_style = if badge_is_running {
        Style::default().fg(super::palette::CYAN)
    } else {
        line1_style
    };

    let mut spans = vec![Span::styled(stripe, Style::default().fg(color))];
    if !badge_text.is_empty() {
        spans.push(Span::styled(badge_text.to_string(), badge_style));
    }
    spans.push(Span::styled(header_truncated, line1_style));
    spans.push(Span::styled(
        " \u{25cf}",
        Style::default().fg(ci_dot_color(pr.ci_status)),
    ));
    Line::from(spans)
}

pub(in crate::tui::ui) fn build_review_pr_item(
    pr: &ReviewPr,
    col: usize,
    is_cursor: bool,
    agent_status: Option<crate::models::ReviewAgentStatus>,
    is_selected: bool,
    col_width: u16,
) -> ListItem<'static> {
    let decision_for_color =
        ReviewDecision::from_column_index(col).unwrap_or(ReviewDecision::ReviewRequired);
    let color = review_column_color(decision_for_color);
    let now = Utc::now();
    let age = format_age(pr.created_at, now);

    let line1 = build_pr_line1(pr, color, is_selected, is_cursor, agent_status, col_width);

    // Line 2: ● ci_state · @author · +/-lines · age
    let staleness = Staleness::from_age(pr.created_at, now);
    let age_color = staleness_color(staleness);

    let (ci_color, ci_label) = ci_state_prefix(pr.ci_status);

    let meta_style = Style::default().fg(DIM_META);

    let line2 = Line::from(vec![
        Span::raw("  "),
        Span::styled(ci_label, Style::default().fg(ci_color)),
        Span::styled(
            format!(
                " \u{b7} @{} \u{b7} +{}/-{} \u{b7} ",
                pr.author, pr.additions, pr.deletions
            ),
            meta_style,
        ),
        Span::styled(age, Style::default().fg(age_color)),
    ]);

    let bg = if is_cursor {
        review_cursor_bg_color(decision_for_color)
    } else {
        Color::Reset
    };

    ListItem::new(vec![line1, line2]).style(Style::default().bg(bg))
}

pub(in crate::tui::ui) fn build_dependabot_pr_item(
    pr: &ReviewPr,
    decision: ReviewDecision,
    is_cursor: bool,
    agent_status: Option<crate::models::ReviewAgentStatus>,
    is_selected: bool,
    col_width: u16,
) -> ListItem<'static> {
    let color = review_column_color(decision);
    let now = Utc::now();
    let age = format_age(pr.created_at, now);

    let line1 = build_pr_line1(pr, color, is_selected, is_cursor, agent_status, col_width);

    // Line 2: ● ci_state · +/-lines · age (author omitted — always "dependabot")
    let staleness = Staleness::from_age(pr.created_at, now);
    let age_color = staleness_color(staleness);

    let (ci_prefix_color, ci_label) = ci_state_prefix(pr.ci_status);
    let meta_style = Style::default().fg(DIM_META);

    let line2 = Line::from(vec![
        Span::raw("  "),
        Span::styled(ci_label, Style::default().fg(ci_prefix_color)),
        Span::styled(
            format!(" \u{b7} +{}/-{} \u{b7} ", pr.additions, pr.deletions),
            meta_style,
        ),
        Span::styled(age, Style::default().fg(age_color)),
    ]);

    let bg = if is_cursor {
        review_cursor_bg_color(decision)
    } else {
        Color::Reset
    };

    ListItem::new(vec![line1, line2]).style(Style::default().bg(bg))
}

fn ci_state_prefix(status: CiStatus) -> (Color, &'static str) {
    match status {
        CiStatus::Success => (Color::Green, "\u{25cf} passing"),
        CiStatus::Failure => (Color::Red, "\u{25cf} failing"),
        CiStatus::Pending => (Color::Yellow, "\u{25cf} pending"),
        CiStatus::None => (Color::DarkGray, "\u{25cf} \u{2013}"),
    }
}

fn ci_dot_color(status: CiStatus) -> Color {
    ci_state_prefix(status).0
}
