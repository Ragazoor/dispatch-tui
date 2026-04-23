use super::palette::{BORDER, CYAN, DIM_META, FG, MUTED, MUTED_LIGHT, YELLOW};
use super::review::{build_dependabot_pr_item, review_column_bg_color, review_column_color};
use super::shared::{
    push_hint_spans, refresh_status, render_substatus_header, render_tab_bar, staleness_color,
    truncate,
};

use crate::models::{
    format_age, AlertKind, AlertSeverity, ReviewDecision, ReviewPr, SecurityAlert,
    SecurityWorkflowColumn, Staleness,
};
use crate::tui::{App, InputMode, SecurityBoardMode, ViewMode};
use chrono::Utc;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

// ---------------------------------------------------------------------------
// Security board rendering
// ---------------------------------------------------------------------------

fn security_column_color(severity: AlertSeverity) -> Color {
    match severity {
        AlertSeverity::Critical => Color::Red,
        AlertSeverity::High => YELLOW,
        AlertSeverity::Medium => Color::Rgb(86, 152, 194),
        AlertSeverity::Low => Color::DarkGray,
    }
}

fn security_cursor_bg_color(severity: AlertSeverity) -> Color {
    match severity {
        AlertSeverity::Critical => Color::Rgb(56, 28, 28),
        AlertSeverity::High => Color::Rgb(52, 44, 20),
        AlertSeverity::Medium => Color::Rgb(24, 40, 52),
        AlertSeverity::Low => Color::Rgb(34, 34, 40),
    }
}

pub fn render_security_board(frame: &mut Frame, app: &mut App, area: Rect) {
    let mode = match app.view_mode() {
        ViewMode::SecurityBoard { mode, .. } => *mode,
        _ => SecurityBoardMode::Dependabot,
    };
    match mode {
        SecurityBoardMode::Dependabot => render_dependabot_board(frame, app, area),
        SecurityBoardMode::Alerts => render_security_alerts_board(frame, app, area),
    }
}

fn render_security_alerts_board(frame: &mut Frame, app: &mut App, area: Rect) {
    let detail_height = if app.security_detail_visible() { 8 } else { 0 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),             // tab bar
            Constraint::Length(1),             // mode header
            Constraint::Length(1),             // summary row
            Constraint::Length(1),             // refresh status row
            Constraint::Min(1),                // board
            Constraint::Length(detail_height), // detail panel
            Constraint::Length(1),             // status bar
        ])
        .split(area);

    render_tab_bar(frame, app, chunks[0]);
    render_security_mode_header(frame, SecurityBoardMode::Alerts, chunks[1]);

    if app.security.unconfigured {
        let prompt = "No repositories configured — press [e] to set up security alert queries";
        frame.render_widget(
            Paragraph::new(prompt)
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::DarkGray)),
            chunks[4],
        );
        // Status bar: show transient message or an unconfigured-specific hint line
        if let Some(msg) = app.status.message.as_deref() {
            frame.render_widget(
                Paragraph::new(msg.to_string()).style(Style::default().fg(Color::Yellow)),
                chunks[6],
            );
        } else {
            let key_color = Color::Cyan;
            let label_style = Style::default().fg(MUTED);
            let mut hints: Vec<Span<'static>> = Vec::new();
            push_hint_spans(&mut hints, "e", "edit queries", key_color, label_style);
            push_hint_spans(&mut hints, "Tab", "tasks", key_color, label_style);
            push_hint_spans(&mut hints, "q", "quit", key_color, label_style);
            frame.render_widget(Paragraph::new(Line::from(hints)), chunks[6]);
        }
        // Filter overlay still available when unconfigured
        if matches!(app.mode(), InputMode::SecurityRepoFilter) {
            render_security_repo_filter_overlay(frame, app, area);
        }
        return;
    }

    render_security_summary_row(frame, app, chunks[2]);

    // Refresh status row
    let (status_text, status_color) = refresh_status(
        app.security_last_fetch(),
        app.security_loading(),
        crate::tui::SECURITY_POLL_INTERVAL,
    );
    frame.render_widget(
        Paragraph::new(status_text).style(Style::default().fg(status_color)),
        chunks[3],
    );

    let filtered = app.filtered_security_alerts();
    if filtered.is_empty() {
        let msg = if app.filtered_security_alerts().is_empty() && !app.security.alerts.is_empty() {
            "All alerts filtered out."
        } else {
            "No security alerts found"
        };
        let p = Paragraph::new(msg)
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(p, chunks[4]);
    } else {
        render_security_columns(frame, app, chunks[4]);
    }

    render_security_detail(frame, app, chunks[5]);

    // Status bar
    if let Some(msg) = app.status.message.as_deref() {
        let status = Paragraph::new(msg.to_string()).style(Style::default().fg(Color::Yellow));
        frame.render_widget(status, chunks[6]);
    } else if let Some(err) = app.last_security_error() {
        let status = Paragraph::new(format!("Error: {err}")).style(Style::default().fg(Color::Red));
        frame.render_widget(status, chunks[6]);
    } else {
        let has_alert = app.selected_security_alert().is_some();
        let agent_status = app
            .selected_security_alert()
            .and_then(|a| app.alert_agent(a).map(|h| h.status));
        let hints = Paragraph::new(Line::from(security_action_hints(
            app,
            has_alert,
            agent_status,
        )));
        frame.render_widget(hints, chunks[6]);
    }

    // Filter overlay
    if matches!(app.mode(), InputMode::SecurityRepoFilter) {
        render_security_repo_filter_overlay(frame, app, area);
    }
}

fn render_security_mode_header(frame: &mut Frame, current_mode: SecurityBoardMode, area: Rect) {
    let active_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let inactive_style = Style::default().fg(MUTED);

    let (dep_style, alerts_style) = match current_mode {
        SecurityBoardMode::Dependabot => (active_style, inactive_style),
        SecurityBoardMode::Alerts => (inactive_style, active_style),
    };

    let line = Line::from(vec![
        Span::styled("[1] Dependabot", dep_style),
        Span::styled("  ", Style::default()),
        Span::styled("[2] Alerts", alerts_style),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_dependabot_board(frame: &mut Frame, app: &mut App, area: Rect) {
    let detail_height = if app.security.dependabot.detail_visible {
        8
    } else {
        0
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),             // tab bar
            Constraint::Length(1),             // mode header
            Constraint::Length(1),             // column summary row
            Constraint::Length(1),             // refresh status row
            Constraint::Min(1),                // board
            Constraint::Length(detail_height), // detail panel (placeholder)
            Constraint::Length(1),             // status bar
        ])
        .split(area);

    render_tab_bar(frame, app, chunks[0]);
    render_security_mode_header(frame, SecurityBoardMode::Dependabot, chunks[1]);
    render_dependabot_summary_row(frame, app, chunks[2]);

    // Refresh status row
    let (status_text, status_color) = refresh_status(
        app.bot_prs_last_fetch(),
        app.bot_prs_loading(),
        crate::tui::REVIEW_REFRESH_INTERVAL,
    );
    frame.render_widget(
        Paragraph::new(status_text).style(Style::default().fg(status_color)),
        chunks[3],
    );

    let bot_prs = app.filtered_bot_prs();
    if bot_prs.is_empty() {
        let msg = if app.security.dependabot.prs.last_error.is_some() {
            "Failed to load PRs."
        } else {
            "No Dependabot PRs found"
        };
        let p = Paragraph::new(msg)
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(p, chunks[4]);
    } else {
        render_dependabot_columns(frame, app, chunks[4]);
    }

    // Detail panel placeholder (currently empty)
    let detail_block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(BORDER));
    frame.render_widget(detail_block, chunks[5]);

    // Status bar
    if let Some(msg) = app.status.message.as_deref() {
        let status = Paragraph::new(msg.to_string()).style(Style::default().fg(Color::Yellow));
        frame.render_widget(status, chunks[6]);
    } else if let Some(err) = app.security.dependabot.prs.last_error.as_deref() {
        let status = Paragraph::new(format!("Error: {err}")).style(Style::default().fg(Color::Red));
        frame.render_widget(status, chunks[6]);
    } else {
        let has_selected = !app.selected_bot_prs().is_empty();
        let col = match app.view_mode() {
            ViewMode::SecurityBoard {
                dependabot_selection,
                ..
            } => dependabot_selection.selected_column,
            _ => 0,
        };
        let row = match app.view_mode() {
            ViewMode::SecurityBoard {
                dependabot_selection,
                ..
            } => dependabot_selection.selected_row[col],
            _ => 0,
        };
        let selected_pr = app
            .filtered_bot_prs()
            .into_iter()
            .filter(|pr| crate::tui::bot_pr_column(pr, app.pr_agent(pr).map(|h| h.status)) == col)
            .nth(row);
        let pr_agent_status = selected_pr.and_then(|pr| app.pr_agent(pr).map(|h| h.status));
        let hints = Paragraph::new(Line::from(dependabot_action_hints(
            has_selected,
            selected_pr,
            pr_agent_status,
        )));
        frame.render_widget(hints, chunks[6]);
    }

    if matches!(app.mode(), InputMode::BotPrRepoFilter) {
        render_bot_pr_repo_filter_overlay(frame, app, area);
    }
}

fn render_dependabot_summary_row(frame: &mut Frame, app: &App, area: Rect) {
    let col_count = 3usize;
    let col_labels = ["Backlog", "In Review", "Approved"];
    let col_colors = [
        review_column_color(ReviewDecision::ReviewRequired),
        review_column_color(ReviewDecision::ChangesRequested),
        review_column_color(ReviewDecision::Approved),
    ];

    let sel_col = match app.view_mode() {
        ViewMode::SecurityBoard {
            dependabot_selection,
            ..
        } => dependabot_selection.selected_column,
        _ => 0,
    };

    let prs = app.filtered_bot_prs();

    let segments = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(vec![Constraint::Ratio(1, col_count as u32); col_count])
        .split(area);

    for i in 0..col_count {
        let count = prs
            .iter()
            .filter(|pr| crate::tui::bot_pr_column(pr, app.pr_agent(pr).map(|h| h.status)) == i)
            .count();
        let is_focused = i == sel_col;
        let prefix = if is_focused { "\u{25b8} " } else { "\u{25e6} " };
        let label = format!("{prefix}{} ({count})", col_labels[i]);

        let style = if is_focused {
            Style::default()
                .fg(col_colors[i])
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        frame.render_widget(Paragraph::new(label).style(style), segments[i]);
    }
}

fn render_dependabot_columns(frame: &mut Frame, app: &mut App, area: Rect) {
    let col_count = 3usize;
    let col_decisions = [
        ReviewDecision::ReviewRequired,
        ReviewDecision::ChangesRequested,
        ReviewDecision::Approved,
    ];

    let (sel_col, sel_rows) = match app.view_mode() {
        ViewMode::SecurityBoard {
            dependabot_selection,
            ..
        } => (
            dependabot_selection.selected_column,
            dependabot_selection.selected_row,
        ),
        _ => (0, [0; ReviewDecision::COLUMN_COUNT]),
    };

    let col_areas = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(vec![Constraint::Ratio(1, col_count as u32); col_count])
        .split(area);

    let selected_prs: std::collections::HashSet<String> =
        app.selected_bot_prs().iter().cloned().collect();

    for i in 0..col_count {
        let is_focused = i == sel_col;
        let decision_for_color = col_decisions[i];
        let selected_row = sel_rows[i];

        let mut prs: Vec<&ReviewPr> = app
            .filtered_bot_prs()
            .into_iter()
            .filter(|pr| crate::tui::bot_pr_column(pr, app.pr_agent(pr).map(|h| h.status)) == i)
            .collect();
        crate::tui::sort_prs_for_display(&mut prs);

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

            let is_selected = selected_prs.contains(&pr.url);
            list_items.push(build_dependabot_pr_item(
                pr,
                decision_for_color,
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

        // Write back list state for scroll tracking
        if let ViewMode::SecurityBoard {
            dependabot_selection,
            ..
        } = &mut app.board.view_mode
        {
            dependabot_selection.list_states[i] = list_state;
        }
    }
}

pub(in crate::tui) fn dependabot_action_hints(
    has_selected: bool,
    selected_pr: Option<&crate::models::ReviewPr>,
    agent_status: Option<crate::models::ReviewAgentStatus>,
) -> Vec<Span<'static>> {
    use crate::models::ReviewAgentStatus;
    let key_color = Color::Cyan;
    let label_style = Style::default().fg(MUTED);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let push_hint = |spans: &mut Vec<Span<'static>>, key: &'static str, label: String| {
        push_hint_spans(spans, key, &label, key_color, label_style);
    };

    if has_selected {
        push_hint(&mut spans, "a", "approve".into());
        push_hint(&mut spans, "m", "merge".into());
        push_hint(&mut spans, "Esc", "clear".into());
    } else if selected_pr.is_some() {
        push_hint(&mut spans, "Space", "select".into());
        match agent_status {
            Some(ReviewAgentStatus::Idle) => {
                push_hint(&mut spans, "g", "go to".into());
                push_hint(&mut spans, "d", "resume".into());
                push_hint(&mut spans, "T", "detach".into());
            }
            Some(_) => {
                push_hint(&mut spans, "g", "go to".into());
                push_hint(&mut spans, "T", "detach".into());
            }
            None => {
                push_hint(&mut spans, "d", "dispatch".into());
            }
        }
        push_hint(&mut spans, "p", "open".into());
    }
    push_hint(&mut spans, "Tab", "tasks".into());
    push_hint(&mut spans, "?", "help".into());
    push_hint(&mut spans, "q", "quit".into());
    spans
}

pub(in crate::tui) fn security_action_hints(
    app: &App,
    has_alert: bool,
    agent_status: Option<crate::models::ReviewAgentStatus>,
) -> Vec<Span<'static>> {
    use crate::models::ReviewAgentStatus;
    let key_color = Color::Cyan;
    let label_style = Style::default().fg(MUTED);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let push_hint = |spans: &mut Vec<Span<'static>>, key: &'static str, label: String| {
        push_hint_spans(spans, key, &label, key_color, label_style);
    };
    if has_alert {
        push_hint(&mut spans, "Enter", "detail".into());
        match agent_status {
            Some(ReviewAgentStatus::Idle) => {
                push_hint(&mut spans, "g", "go to".into());
                push_hint(&mut spans, "d", "resume".into());
                push_hint(&mut spans, "T", "detach".into());
            }
            Some(_) => {
                push_hint(&mut spans, "g", "go to".into());
                push_hint(&mut spans, "T", "detach".into());
            }
            None => {
                push_hint(&mut spans, "d", "dispatch".into());
            }
        }
        push_hint(&mut spans, "p", "open".into());
    }
    push_hint(&mut spans, "f", "filter".into());
    let kind_label = match app.security_kind_filter() {
        None => "all",
        Some(AlertKind::Dependabot) => "deps",
        Some(AlertKind::CodeScanning) => "code",
    };
    push_hint(&mut spans, "t", format!("kind:{kind_label}"));
    push_hint(&mut spans, "Tab", "tasks".into());
    push_hint(&mut spans, "?", "help".into());
    push_hint(&mut spans, "q", "quit".into());
    spans
}

fn render_security_summary_row(frame: &mut Frame, app: &App, area: Rect) {
    let sel = app.security_selection();
    let selected_col = sel.map(|s| s.column()).unwrap_or(0);
    let col_count = SecurityWorkflowColumn::COLUMN_COUNT;

    let segments = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(vec![Constraint::Ratio(1, col_count as u32); col_count])
        .split(area);

    for (i, workflow_col) in SecurityWorkflowColumn::ALL.iter().enumerate() {
        let count = app.security_alerts_for_column(i).len();
        let is_focused = i == selected_col;
        let prefix = if is_focused { "\u{25b8} " } else { "\u{25e6} " };
        let label = format!("{prefix}{} ({count})", workflow_col.label());

        let style = if is_focused {
            Style::default().fg(FG).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let p = Paragraph::new(label).style(style);
        frame.render_widget(p, segments[i]);
    }
}

fn render_security_columns(frame: &mut Frame, app: &mut App, area: Rect) {
    let sel_col = app.security_selection().map(|s| s.column()).unwrap_or(0);
    let col_count = SecurityWorkflowColumn::COLUMN_COUNT;

    let col_areas = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(vec![Constraint::Ratio(1, col_count as u32); col_count])
        .split(area);

    for i in 0..col_count {
        let is_focused = i == sel_col;
        let alerts: Vec<&SecurityAlert> = app.security_alerts_for_column(i);
        let is_in_progress_col = i == SecurityWorkflowColumn::InProgress.column_index();

        let selected_row = app.security_selection().map(|s| s.row(i)).unwrap_or(0);
        let mut list_items: Vec<ListItem> = Vec::new();
        let mut list_selection_idx: Option<usize> = None;
        let mut current_repo: Option<&str> = None;

        for (item_idx, alert) in alerts.iter().enumerate() {
            if current_repo != Some(alert.repo.as_str()) {
                current_repo = Some(alert.repo.as_str());
                let repo_short = alert.repo.split('/').next_back().unwrap_or(&alert.repo);
                list_items.push(render_substatus_header(repo_short, list_items.is_empty()));
            }

            if item_idx == selected_row {
                list_selection_idx = Some(list_items.len());
            }

            list_items.push(build_security_alert_item(
                alert,
                is_focused && item_idx == selected_row,
                col_areas[i].width,
                is_in_progress_col,
            ));
        }

        let bg = if is_focused {
            Color::Rgb(24, 26, 32)
        } else {
            Color::Reset
        };

        let list = List::new(list_items).block(Block::default().style(Style::default().bg(bg)));

        let mut list_state = ListState::default();
        if is_focused {
            list_state.select(list_selection_idx);
        }

        frame.render_stateful_widget(list, col_areas[i], &mut list_state);

        if let Some(sel) = app.security_selection_mut() {
            sel.list_states[i] = list_state;
        }
    }
}

/// Test helper: returns concatenated text content of the card for assertions.
#[cfg(test)]
pub(in crate::tui) fn build_security_alert_item_for_test(
    alert: &SecurityAlert,
    is_cursor: bool,
    col_width: u16,
    is_running: bool,
) -> String {
    let _item = build_security_alert_item(alert, is_cursor, col_width, is_running);
    // Extract text via rendering to Text — ListItem wraps a Text internally.
    // We reconstruct text by converting via the widget's Text representation.
    // Simplest: build the same lines as the function and concatenate spans.
    let now = chrono::Utc::now();
    let age = format_age(alert.created_at, now);
    let severity_color = security_column_color(alert.severity);
    let stripe = if is_cursor { "\u{258c} " } else { "\u{258e} " };
    let running_badge = if is_running { "\u{25c9} " } else { "" };
    let header = format!("#{} {}", alert.number, alert.title);
    let reserved = 2 + if is_running { 2 } else { 0 };
    let max_header = (col_width as usize).saturating_sub(reserved);
    let header_truncated = truncate(&header, max_header);
    let (sev_badge, _) = match alert.severity {
        AlertSeverity::Critical => ("[CRIT]", Color::Red),
        AlertSeverity::High => ("[HIGH]", YELLOW),
        AlertSeverity::Medium => ("[MED] ", Color::Rgb(86, 152, 194)),
        AlertSeverity::Low => ("[LOW] ", Color::DarkGray),
    };
    let kind_label = match alert.kind {
        AlertKind::Dependabot => "\u{2b21} Dependabot",
        AlertKind::CodeScanning => "\u{2b21} CodeScanning",
    };
    let pkg = alert.package.as_deref().unwrap_or("-");
    let cvss_str = alert
        .cvss_score
        .map(|s| format!(" \u{b7} CVSS:{s:.1}"))
        .unwrap_or_default();
    format!(
        "{stripe}{running_badge}{header_truncated}  {sev_badge} {kind_label} \u{b7} {pkg}{cvss_str} \u{b7} {age}",
    )
}

pub(in crate::tui::ui) fn build_security_alert_item(
    alert: &SecurityAlert,
    is_cursor: bool,
    col_width: u16,
    is_running: bool,
) -> ListItem<'static> {
    let severity_color = security_column_color(alert.severity);
    let now = Utc::now();
    let age = format_age(alert.created_at, now);

    // Line 1: severity-colored stripe + optional ◉ running badge + #number + title
    let stripe = if is_cursor { "\u{258c} " } else { "\u{258e} " };
    let running_badge = if is_running { "\u{25c9} " } else { "" };
    let header = format!("#{} {}", alert.number, alert.title);
    // stripe(2) + running_badge(2 if present) + header
    let reserved = 2 + if is_running { 2 } else { 0 };
    let max_header = (col_width as usize).saturating_sub(reserved);
    let header_truncated = truncate(&header, max_header);

    let line1_style = if is_cursor {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(severity_color)
    };

    let mut line1_spans = vec![Span::styled(stripe, Style::default().fg(severity_color))];
    if is_running {
        line1_spans.push(Span::styled(running_badge, Style::default().fg(CYAN)));
    }
    line1_spans.push(Span::styled(header_truncated, line1_style));
    let line1 = Line::from(line1_spans);

    // Line 2: [severity] ⬡ kind · package · CVSS · age
    let staleness = Staleness::from_age(alert.created_at, now);
    let age_color = staleness_color(staleness);

    let (sev_badge, sev_color) = match alert.severity {
        AlertSeverity::Critical => ("[CRIT]", Color::Red),
        AlertSeverity::High => ("[HIGH]", YELLOW),
        AlertSeverity::Medium => ("[MED] ", Color::Rgb(86, 152, 194)),
        AlertSeverity::Low => ("[LOW] ", Color::DarkGray),
    };

    let (kind_color, kind_label) = match alert.kind {
        AlertKind::Dependabot => (YELLOW, "\u{2b21} Dependabot"),
        AlertKind::CodeScanning => (CYAN, "\u{2b21} CodeScanning"),
    };
    let pkg = alert.package.as_deref().unwrap_or("-");
    let cvss_str = alert
        .cvss_score
        .map(|s| format!(" \u{b7} CVSS:{s:.1}"))
        .unwrap_or_default();

    let meta_style = Style::default().fg(DIM_META);

    let line2 = Line::from(vec![
        Span::raw("  "),
        Span::styled(sev_badge, Style::default().fg(sev_color)),
        Span::raw(" "),
        Span::styled(kind_label, Style::default().fg(kind_color)),
        Span::styled(format!(" \u{b7} {pkg}{cvss_str} \u{b7} "), meta_style),
        Span::styled(age, Style::default().fg(age_color)),
    ]);

    let bg = if is_cursor {
        security_cursor_bg_color(alert.severity)
    } else {
        Color::Reset
    };

    ListItem::new(vec![line1, line2]).style(Style::default().bg(bg))
}

fn render_security_detail(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(BORDER));

    if !app.security_detail_visible() {
        let paragraph = Paragraph::new("").block(block);
        frame.render_widget(paragraph, area);
        return;
    }

    let Some(alert) = app.selected_security_alert() else {
        let paragraph = Paragraph::new("").block(block);
        frame.render_widget(paragraph, area);
        return;
    };

    let color = security_column_color(alert.severity);
    let now = Utc::now();
    let age = format_age(alert.created_at, now);

    // Line 1: title
    let line1 = Line::from(vec![Span::styled(
        format!("{}#{} {}", alert.repo, alert.number, alert.title),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )]);

    // Line 2: kind + severity + CVSS
    let cvss_str = alert
        .cvss_score
        .map(|s| format!(" CVSS:{s:.1}"))
        .unwrap_or_default();
    let line2 = Line::from(Span::styled(
        format!(
            "{} \u{00b7} {}{} \u{00b7} {} \u{00b7} {}",
            alert.kind.as_str(),
            alert.severity.as_str(),
            cvss_str,
            alert.repo,
            age,
        ),
        Style::default().fg(MUTED),
    ));

    // Line 3: package info or location
    let pkg_line = if let Some(pkg) = &alert.package {
        let range = alert.vulnerable_range.as_deref().unwrap_or("");
        let fix = alert
            .fixed_version
            .as_ref()
            .map(|v| format!(" \u{2192} {v}"))
            .unwrap_or_default();
        format!("Package: {pkg} {range}{fix}")
    } else {
        "No package info".to_string()
    };
    let line3 = Line::from(Span::styled(pkg_line, Style::default().fg(MUTED_LIGHT)));

    // Lines 4+: description (truncated)
    let desc_lines: Vec<Line> = alert
        .description
        .lines()
        .take(4)
        .map(|l| {
            Line::from(Span::styled(
                l.to_string(),
                Style::default().fg(Color::DarkGray),
            ))
        })
        .collect();

    let mut lines = vec![line1, line2, line3];
    lines.extend(desc_lines);

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn render_security_repo_filter_overlay(frame: &mut Frame, app: &App, area: Rect) {
    let repos = app.active_security_repos();
    let mode_str = app.security.repo_filter_mode.as_str();
    render_security_filter_overlay_inner(frame, area, repos, mode_str, |r| {
        app.security.repo_filter.contains(r)
    });
}

fn render_bot_pr_repo_filter_overlay(frame: &mut Frame, app: &App, area: Rect) {
    let repos = app.active_bot_pr_repos();
    let mode_str = app.security.dependabot.prs.repo_filter_mode.as_str();
    render_security_filter_overlay_inner(frame, area, repos, mode_str, |r| {
        app.security.dependabot.prs.repo_filter.contains(r)
    });
}

fn render_security_filter_overlay_inner(
    frame: &mut Frame,
    area: Rect,
    repos: &[String],
    mode_str: &str,
    is_selected: impl Fn(&str) -> bool,
) {
    if repos.is_empty() {
        return;
    }

    let height = (repos.len() as u16 + 4).min(area.height.saturating_sub(2));
    let width = 50.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let popup_area = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(format!(" Filter Repos ({mode_str}) "))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(CYAN));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!(" Mode: {mode_str} [Tab] toggle"),
        Style::default().fg(MUTED_LIGHT),
    )));
    lines.push(Line::from(Span::styled(
        " [a]ll toggle",
        Style::default().fg(MUTED),
    )));

    for (i, repo) in repos.iter().enumerate() {
        let selected = is_selected(repo);
        let marker = if selected { "\u{25c9}" } else { "\u{25cb}" };
        let num = i + 1;
        let line = Line::from(vec![
            Span::styled(
                format!(" {num}"),
                Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {marker} {repo}"),
                if selected {
                    Style::default().fg(FG)
                } else {
                    Style::default().fg(MUTED)
                },
            ),
        ]);
        lines.push(line);
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}
