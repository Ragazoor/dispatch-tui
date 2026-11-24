//! The orchestrator's band in an epic view (`epics.allium:
//! OrchestratorHeader`).

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::models::{descendant_epic_ids, Task, TaskStatus};
use crate::palette::{CYAN, MUTED, PURPLE, RED, YELLOW};
use crate::text::truncate;
use crate::tui::App;

/// The band's rows for the epic view showing, at most `max_rows` of them,
/// each cut to `width`. Empty outside the view of a covered epic.
pub(in crate::tui) fn orchestrator_header_lines(
    app: &App,
    width: u16,
    max_rows: usize,
) -> Vec<Line<'static>> {
    let Some(view) = app.current_epic_id() else {
        return vec![];
    };
    let Some(epic) = app.covering_epic(Some(view)) else {
        return vec![];
    };
    if max_rows == 0 {
        return vec![];
    }
    let width = usize::from(width);
    let board = &app.board.orchestration;
    let host = app.local_host_id.clone().unwrap_or_default();
    let ids = descendant_epic_ids(epic, &app.board.epics);
    let subtree: Vec<&Task> = app
        .board
        .tasks
        .iter()
        .filter(|t| t.epic_id.is_some_and(|e| e == epic || ids.contains(&e)))
        .collect();
    let items = board.open_items_of(epic);

    let mut first: Vec<Span<'static>> = Vec::new();
    match board.orchestrator_on(epic, &host) {
        Some(o) => {
            let state = crate::models::orchestrator_state(o, &subtree);
            let state_style = match state {
                "down" => Style::default().fg(RED).add_modifier(Modifier::BOLD),
                "working" => Style::default().fg(CYAN),
                _ => Style::default().fg(MUTED),
            };
            first.push(Span::styled("orchestrator ", Style::default().fg(MUTED)));
            first.push(Span::styled(o.name.clone(), Style::default().fg(PURPLE)));
            first.push(Span::styled(" \u{00b7} ", Style::default().fg(MUTED)));
            first.push(Span::styled(state.to_string(), state_style));
        }
        None => first.push(Span::styled(
            "orchestrator: resting",
            Style::default().fg(MUTED),
        )),
    }
    if let Some(on) = board.orchestration_of(epic) {
        let running = subtree
            .iter()
            .filter(|t| {
                t.status == TaskStatus::Running
                    && t.host.as_deref() == Some(host.as_str())
                    && !t.is_detached()
            })
            .count();
        first.push(Span::styled(
            format!(
                " \u{00b7} {} \u{00b7} {running}/{} running",
                on.start_rule,
                on.limit()
            ),
            Style::default().fg(MUTED),
        ));
    }
    if !items.is_empty() {
        first.push(Span::styled(
            format!(" \u{00b7} {} waiting", items.len()),
            Style::default().fg(YELLOW).add_modifier(Modifier::BOLD),
        ));
    }
    if epic != view {
        first.push(Span::styled(
            format!(" (from {})", app.board.epic_label(epic)),
            Style::default().fg(MUTED),
        ));
    }

    let item_rows: Vec<String> = items
        .iter()
        .map(|i| match i.task_id {
            Some(task) => format!("  #{} task #{}: {}", i.id, task.0, i.about),
            None => format!("  #{} the epic: {}", i.id, i.about),
        })
        .collect();
    let log_rows: Vec<String> = board
        .log_of(epic)
        .iter()
        .map(|e| {
            let task = e
                .task_id
                .map(|t| format!(" task #{}", t.0))
                .unwrap_or_default();
            format!(
                "  {} {}{task}: {}",
                e.at.with_timezone(&chrono::Local).format("%m-%d %H:%M"),
                e.kind,
                e.text
            )
        })
        .collect();

    let mut lines = vec![Line::from(first)];
    let room = max_rows - 1;
    let total = item_rows.len() + log_rows.len();
    let (shown_items, shown_log) = if total <= room {
        (item_rows.len(), log_rows.len())
    } else {
        // One row goes to "+N more"; log rows are cut first.
        let keep = room.saturating_sub(1);
        let items = item_rows.len().min(keep);
        (items, keep - items)
    };
    let cut = |s: &str| truncate(s, width.saturating_sub(1));
    for row in &item_rows[..shown_items] {
        lines.push(Line::from(Span::styled(
            cut(row),
            Style::default().fg(YELLOW),
        )));
    }
    for row in &log_rows[..shown_log] {
        lines.push(Line::from(Span::styled(
            cut(row),
            Style::default().fg(MUTED),
        )));
    }
    let hidden = total - shown_items - shown_log;
    if hidden > 0 && room > 0 {
        lines.push(Line::from(Span::styled(
            format!("  +{hidden} more"),
            Style::default().fg(MUTED),
        )));
    }
    lines
}
