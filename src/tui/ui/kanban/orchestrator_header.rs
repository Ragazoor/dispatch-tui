//! The orchestrator's band in an epic view (`epics.allium:
//! OrchestratorHeader`).

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::models::{descendant_epic_ids, EpicId, Orchestrator, Task};
use crate::palette::{CYAN, MUTED, PURPLE, RED, YELLOW};
use crate::text::truncate;
use crate::tui::App;

/// What the epic view's header and the orchestrator view's card both say
/// about an orchestrated epic's orchestrator on this machine, from one
/// place so the two cannot disagree.
pub(super) struct OrchestratorSummary<'a> {
    /// This machine's orchestrator, `None` before it has a row here.
    pub(super) orchestrator: Option<&'a Orchestrator>,
    /// `orchestrators.allium: orchestrator_state`; resting with no row.
    pub(super) state: &'static str,
    /// The epic's subtree tasks.
    pub(super) subtree: Vec<&'a Task>,
    /// The subtree tasks that hold one of this machine's slots.
    pub(super) running: usize,
    /// The open items that wait for the operator.
    pub(super) waiting: usize,
}

pub(super) fn orchestrator_summary(app: &App, epic: EpicId) -> OrchestratorSummary<'_> {
    let host = app.local_host_id.as_deref().unwrap_or_default();
    let ids = descendant_epic_ids(epic, &app.board.epics);
    let subtree: Vec<&Task> = app
        .board
        .tasks
        .iter()
        .filter(|t| t.epic_id.is_some_and(|e| ids.contains(&e)))
        .collect();
    let orchestrator = app.board.orchestration.orchestrator_on(epic, host);
    let state = orchestrator.map_or("resting", |o| {
        crate::models::orchestrator_state(o, &subtree)
    });
    let running = subtree
        .iter()
        .filter(|t| crate::models::runs_here(t, host))
        .count();
    OrchestratorSummary {
        orchestrator,
        state,
        running,
        waiting: app.board.orchestration.open_items_of(epic).len(),
        subtree,
    }
}

/// The style an orchestrator's state is drawn in: down is an alarm.
pub(super) fn state_style(state: &str) -> Style {
    match state {
        "down" => Style::default().fg(RED).add_modifier(Modifier::BOLD),
        "working" => Style::default().fg(CYAN),
        _ => Style::default().fg(MUTED),
    }
}

/// The band's rows for the epic view showing, at most `max_rows` of them,
/// each cut to `width`. Empty outside the view of a covered epic.
pub(in crate::tui) fn orchestrator_header_lines(
    app: &App,
    width: u16,
    max_rows: usize,
) -> Vec<Line<'static>> {
    // The epic view under an open task detail keeps its band, so the
    // columns do not jump behind the panel.
    let crate::tui::BoardViewMode::Epic { epic_id: view, .. } = app.view().effective_view_mode()
    else {
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
    let summary = orchestrator_summary(app, epic);
    let items = board.open_items_of(epic);

    let mut first: Vec<Span<'static>> = Vec::new();
    match summary.orchestrator {
        Some(o) => {
            first.push(Span::styled("orchestrator ", Style::default().fg(MUTED)));
            first.push(Span::styled(o.name.clone(), Style::default().fg(PURPLE)));
            first.push(Span::styled(" \u{00b7} ", Style::default().fg(MUTED)));
            first.push(Span::styled(
                summary.state.to_string(),
                state_style(summary.state),
            ));
        }
        None => first.push(Span::styled(
            "orchestrator: resting",
            Style::default().fg(MUTED),
        )),
    }
    if let Some(on) = board.orchestration_of(epic) {
        let running = summary.running;
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

    let name = |task: crate::models::TaskId| {
        crate::models::task_name_or_id(
            task,
            app.board
                .find_task(task)
                .and_then(|t| t.agent_name.as_deref()),
        )
    };
    let item_rows: Vec<String> = items
        .iter()
        .map(|i| match i.task_id {
            Some(task) => format!("  #{} {}: {}", i.id, name(task), i.about),
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
