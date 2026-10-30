//! Status bar at the bottom of the kanban board.
//!
//! Renders one of two flavours depending on app state:
//! * a transient status message, or
//! * mode-specific hints (Normal mode delegates to `action_hints` /
//!   `epic_action_hints` / `batch_action_hints`).

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::super::shared::push_hint_spans;
use super::{action_hints, epic_action_hints};
use crate::palette::{CYAN, GREEN, MUTED, PURPLE, RED, YELLOW};
use crate::tui::{App, ColumnItem, InputMode};

pub(super) fn render_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let (line, style) = status_line(app);
    frame.render_widget(Paragraph::new(line).style(style), area);
}

/// Prepend `prefix` spans to `spans` in place, preserving order.
fn prepend(spans: &mut Vec<Span<'static>>, mut prefix: Vec<Span<'static>>) {
    prefix.append(spans);
    *spans = prefix;
}

/// A simple text status line with a single foreground colour, falling back to
/// `app.status.message` when set. (`app.status.message` being `Some` is already
/// short-circuited by `status_line`, so this always resolves to `default`; the
/// override arms retain this shape for clarity.)
fn hint_text(app: &App, default: &str, color: Color) -> (Line<'static>, Style) {
    let text = app.status.message.as_deref().unwrap_or(default).to_string();
    (Line::from(text), Style::default().fg(color))
}

/// A fixed text status line with a single foreground colour. Accepts anything
/// that converts into a `Line<'static>`, so string literals borrow without an
/// allocation while owned `String`s (e.g. a formatted search prompt) move in.
fn hint(text: impl Into<Line<'static>>, color: Color) -> (Line<'static>, Style) {
    (text.into(), Style::default().fg(color))
}

/// Compute the status bar content (a styled `Line` plus a base paragraph style)
/// for the current app state. Rendering happens once, in `render_status_bar`.
///
/// The composed Normal-mode hint line is the one structurally-heavier flavour;
/// it lives in its own builder (`normal_status_line`), and everything else is
/// a fixed per-mode hint.
fn status_line(app: &App) -> (Line<'static>, Style) {
    if let Some(msg) = &app.status.message {
        return (Line::from(msg.clone()), Style::default().fg(YELLOW));
    }

    match &app.input.mode {
        InputMode::Normal => normal_status_line(app),
        InputMode::SearchTasks => hint(
            format!(
                "Search board: {}_   [Enter] keep  [Esc] cancel",
                app.search.query
            ),
            CYAN,
        ),
        InputMode::InputTitle => hint("Creating task: enter title", YELLOW),
        InputMode::InputDescription => {
            hint("Creating task: opening $EDITOR for description", YELLOW)
        }
        InputMode::InputRepoPath => hint("Creating task: enter repo path", YELLOW),
        InputMode::InputTag => hint_text(
            app,
            crate::tui::ui::tag_prompt(app.input.phoenix_armed()),
            YELLOW,
        ),
        InputMode::ConfirmDeleteTask(_) => hint_text(app, "Delete? [y/n]", RED),
        InputMode::QuickDispatch => hint("Quick dispatch: select repo path", YELLOW),
        InputMode::ConfirmRetry(_) => hint("[r] Resume  [f] Fresh start  [Esc] Cancel", RED),
        InputMode::ConfirmBatchDelete => hint_text(app, "Delete items? [y/n]", RED),
        InputMode::ConfirmDone => hint_text(app, "Move to Done? [y/n]", YELLOW),
        InputMode::InputEpicTitle => hint("Creating epic: enter title", PURPLE),
        InputMode::InputEpicDescription => {
            hint("Creating epic: opening $EDITOR for description", PURPLE)
        }
        InputMode::ConfirmDeleteEpic => hint_text(app, "Delete epic and subtasks? [y/n]", RED),
        InputMode::Help => hint("[?] or [Esc] to close help", CYAN),
        InputMode::BoardFilter => hint(
            "Board filter: [Tab] next tab  [Space] toggle  [q/Esc] close",
            CYAN,
        ),
        InputMode::BoardFilterSearch => hint(
            format!(
                "Search epics: {}_   [Enter] keep  [Esc] clear",
                app.filter_overlay.epic_query
            ),
            CYAN,
        ),
        InputMode::ConfirmDeleteRepoPath => {
            hint("Delete repo path? y to confirm, any key to cancel", YELLOW)
        }
        InputMode::ConfirmDetachTmux(_) => hint_text(app, "Detach tmux panel? [y/n]", YELLOW),
        InputMode::ConfirmOverrideFeedOwner { .. } => {
            hint_text(app, "Take over polling for this feed? [y/N]", YELLOW)
        }
        InputMode::ConfirmQuit => hint("Quit dispatch? [y/n]", YELLOW),
        InputMode::InputBaseBranch => hint_text(app, "Base branch: ", YELLOW),
        InputMode::InputWrapUpMode => {
            hint_text(app, "Wrap-up: [r]ebase  [p]r  [d]one  [Enter] skip", YELLOW)
        }
        InputMode::InputPlacement => hint(
            "[Space] public/private  [j/k] workspace  [Enter] create  [Esc] cancel",
            YELLOW,
        ),
        InputMode::InputModel => {
            let seeded = app.input.task_draft.as_ref().and_then(|d| d.model);
            hint_text(app, &crate::tui::ui::model_prompt(seeded), YELLOW)
        }
        InputMode::ReparentEpic(_) => hint(
            "Select new parent: navigate tree above, Enter to select",
            PURPLE,
        ),
        InputMode::ConfirmReparentEpic { .. } => hint_text(app, "Reparent epic? [y/n]", PURPLE),
        InputMode::MoveTaskToEpic(_) => hint(
            "Select target epic: navigate tree above, Enter to select",
            PURPLE,
        ),
        InputMode::ConfirmMoveTaskToEpic { .. } => {
            hint_text(app, "Move task to epic? [y/n]", PURPLE)
        }
        InputMode::ConfirmTrustRepo { .. } | InputMode::ConfirmTrustRepoQuickDispatch { .. } => {
            hint_text(app, "Repo not trusted — trust it? [y/N]", YELLOW)
        }
        InputMode::LinkWorkPackage { .. } => hint(
            "Link to work package: [j/k] move  [Space/Enter] toggle  [Esc] close",
            CYAN,
        ),
        InputMode::InputWorkPackageTitle(_) => hint(
            "New work package: type a title  [Enter] create  [Esc] back",
            CYAN,
        ),
        InputMode::InputDisplayName => hint(
            "Display name: type a name  [Enter] save  [Esc] cancel",
            CYAN,
        ),
        InputMode::RenameWorkPackage(_) => {
            hint("Rename work package: [Enter] save  [Esc] cancel", CYAN)
        }
        InputMode::ConfirmDeleteWorkPackage(id) => {
            // The status message auto-clears after STATUS_MESSAGE_TTL while
            // the mode lives on, so rebuild the titled prompt.
            let fallback = app.board.find_work_package(*id).map_or_else(
                || "Delete work package? Its tasks and epics are kept [y/n]".to_string(),
                crate::tui::update::delete_work_package_prompt,
            );
            hint_text(app, &fallback, RED)
        }
    }
}

/// Normal-mode status bar: the base action hints (batch / epic / task) with the
/// active-mode badges (split, flat, active-filter, search) composed around
/// them.
fn normal_status_line(app: &App) -> (Line<'static>, Style) {
    let key_color = CYAN;
    let mut spans = if app.has_selection() {
        let count = app.selected_tasks().len() + app.selected_epics().len();
        let has_tasks = !app.selected_tasks().is_empty();
        batch_action_hints(count, key_color, has_tasks)
    } else {
        // One column build for the cursor's item, not one per question asked
        // of it.
        match app.selected_column_item() {
            Some(ColumnItem::Epic(epic)) => epic_action_hints(epic, key_color),
            Some(ColumnItem::WorkPackage(_)) => super::package_action_hints(key_color),
            item => {
                let task = match item {
                    Some(ColumnItem::Task(task)) => Some(task),
                    _ => None,
                };
                let now = chrono::Utc::now();
                let in_flight = task.is_some_and(|t| app.dispatch_may_be_in_flight(t, now));
                action_hints(task, in_flight, key_color)
            }
        }
    };
    if app.split_active() {
        prepend(
            &mut spans,
            vec![
                Span::styled(
                    "[s]",
                    Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
                ),
                Span::styled("plit ", Style::default().fg(GREEN)),
            ],
        );
    }
    let (hidden_epics, hidden_packages) = app.live_hidden_counts();
    if let Some(badge) = hidden_badge(hidden_epics, hidden_packages) {
        prepend(
            &mut spans,
            vec![Span::styled(
                badge,
                Style::default().fg(YELLOW).add_modifier(Modifier::BOLD),
            )],
        );
    }
    if app.filter.lens.flattened() {
        prepend(
            &mut spans,
            vec![Span::styled(
                "[flat] ",
                Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
            )],
        );
    }
    if app.filter.lens.only_active() {
        prepend(
            &mut spans,
            vec![Span::styled(
                "[active] ",
                Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
            )],
        );
    }
    if app.view().search_active() {
        prepend(
            &mut spans,
            vec![Span::styled(
                format!("[/{}] ", app.search.query),
                Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
            )],
        );
    }
    (Line::from(spans), Style::default())
}

/// `[hidden: 2 epics, 1 package] `, or `None` when nothing is hidden. Keeps a
/// forgotten hide from reading as missing work (`board-layout.allium`, "Hidden
/// Epics and Work Packages").
fn hidden_badge(epics: usize, packages: usize) -> Option<String> {
    let count = |n: usize, one: &str| match n {
        0 => None,
        1 => Some(format!("1 {one}")),
        n => Some(format!("{n} {one}s")),
    };
    let parts: Vec<String> = [count(epics, "epic"), count(packages, "package")]
        .into_iter()
        .flatten()
        .collect();
    (!parts.is_empty()).then(|| format!("[hidden: {}] ", parts.join(", ")))
}

/// Build status bar hints when tasks are batch-selected.
fn batch_action_hints(count: usize, key_color: Color, has_tasks: bool) -> Vec<Span<'static>> {
    let label_style = Style::default().fg(MUTED);
    let count_style = Style::default().fg(YELLOW).add_modifier(Modifier::BOLD);

    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::styled(format!("{count} selected  "), count_style));

    let mut push_hint = |key: &'static str, label: &'static str| {
        push_hint_spans(&mut spans, key, label, key_color, label_style);
    };

    if has_tasks {
        push_hint("L", "move");
        push_hint("H", "back");
    }
    // 'x' completes tasks that aren't Done yet and deletes the rest, so with
    // tasks selected the label can't commit to one verb. An epics-only
    // selection always deletes.
    push_hint("x", if has_tasks { "done/delete" } else { "delete" });
    push_hint("a", "select all");
    push_hint("F", "flat");
    push_hint("v", "toggle");
    push_hint("Esc", "clear");
    spans
}
