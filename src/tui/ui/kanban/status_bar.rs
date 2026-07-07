//! Status bar at the bottom of the kanban board.
//!
//! Renders one of three flavours depending on app state:
//! * a transient status message,
//! * archive-mode hints, or
//! * mode-specific hints (Normal mode delegates to `action_hints` /
//!   `epic_action_hints` / `batch_action_hints`).

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::super::palette::{CYAN, MUTED, YELLOW};
use super::super::shared::push_hint_spans;
use super::{action_hints, epic_action_hints};
use crate::tui::{App, ColumnItem, InputMode};

pub(super) fn render_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    if let Some(msg) = &app.status.message {
        let bar = Paragraph::new(msg.as_str()).style(Style::default().fg(Color::Yellow));
        frame.render_widget(bar, area);
        return;
    }

    // Archive mode status bar
    if app.show_archived() {
        let key_color = MUTED;
        let label_style = Style::default().fg(MUTED);
        let spans = vec![
            Span::styled(
                "[x]",
                Style::default().fg(key_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" delete  ", label_style),
            Span::styled(
                "[e]",
                Style::default().fg(key_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" edit  ", label_style),
            Span::styled(
                "[H]",
                Style::default().fg(key_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" close  ", label_style),
            Span::styled(
                "[q]",
                Style::default().fg(key_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" quit  ", label_style),
        ];
        let bar = Paragraph::new(Line::from(spans));
        frame.render_widget(bar, area);
        return;
    }

    match &app.input.mode {
        InputMode::Normal => {
            let key_color = CYAN;
            let mut spans = if app.has_selection() {
                let count = app.selected_tasks().len() + app.selected_epics().len();
                let has_tasks = !app.selected_tasks().is_empty();
                batch_action_hints(count, key_color, has_tasks)
            } else if let Some(ColumnItem::Epic(epic)) = app.selected_column_item() {
                epic_action_hints(epic, key_color)
            } else {
                action_hints(app.selected_task(), app.selected_column(), key_color)
            };
            if app.split_active() {
                let mut prefix = vec![
                    Span::styled(
                        "[S]",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("plit ", Style::default().fg(Color::Green)),
                ];
                prefix.append(&mut spans);
                spans = prefix;
            }
            if app.board.flattened {
                let mut prefix = vec![Span::styled(
                    "[flat] ",
                    Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
                )];
                prefix.append(&mut spans);
                spans = prefix;
            }
            if app.filter_only_active() {
                let mut prefix = vec![Span::styled(
                    "[active] ",
                    Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
                )];
                prefix.append(&mut spans);
                spans = prefix;
            }
            if app.needs_review_count > 0 {
                let mut prefix = vec![Span::styled(
                    format!("[KB:{}] ", app.needs_review_count),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )];
                prefix.append(&mut spans);
                spans = prefix;
            }
            if app.search_active() {
                let mut prefix = vec![Span::styled(
                    format!("[/{}] ", app.search.query),
                    Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
                )];
                prefix.append(&mut spans);
                spans = prefix;
            }
            if app.board.todo_open_count > 0 {
                spans.push(Span::styled(
                    format!(" ({}) ", app.board.todo_open_count),
                    Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
                ));
            }
            let bar = Paragraph::new(Line::from(spans));
            frame.render_widget(bar, area);
        }
        InputMode::SearchTasks => {
            let text = format!(
                "Search tasks: {}_   [Enter] keep  [Esc] cancel",
                app.search.query
            );
            let bar = Paragraph::new(text).style(Style::default().fg(Color::Cyan));
            frame.render_widget(bar, area);
        }
        InputMode::InputTitle => {
            let bar = Paragraph::new("Creating task: enter title")
                .style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::InputDescription => {
            let bar = Paragraph::new("Creating task: opening $EDITOR for description")
                .style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::InputRepoPath => {
            let bar = Paragraph::new("Creating task: enter repo path")
                .style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::InputTag => {
            let text = app
                .status
                .message
                .as_deref()
                .unwrap_or("Tag: [b]ug  [f]eature  [c]hore  [e]pic  [Enter] none");
            let bar = Paragraph::new(text).style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmDelete => {
            let text = app.status.message.as_deref().unwrap_or("Delete? [y/n]");
            let bar = Paragraph::new(text).style(Style::default().fg(Color::Red));
            frame.render_widget(bar, area);
        }
        InputMode::QuickDispatch => {
            let bar = Paragraph::new("Quick dispatch: select repo path")
                .style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmRetry(_) => {
            let bar = Paragraph::new("[r] Resume  [f] Fresh start  [Esc] Cancel")
                .style(Style::default().fg(Color::Red));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmArchive(_) => {
            let bar =
                Paragraph::new("Archive task? [y/n]").style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmDone(_) => {
            let text = app
                .status
                .message
                .as_deref()
                .unwrap_or("Move to Done? [y/n]");
            let bar = Paragraph::new(text).style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::InputEpicTitle => {
            let bar = Paragraph::new("Creating epic: enter title")
                .style(Style::default().fg(Color::Magenta));
            frame.render_widget(bar, area);
        }
        InputMode::InputEpicDescription => {
            let bar = Paragraph::new("Creating epic: opening $EDITOR for description")
                .style(Style::default().fg(Color::Magenta));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmDeleteEpic => {
            let text = app
                .status
                .message
                .as_deref()
                .unwrap_or("Delete epic and subtasks? [y/n]");
            let bar = Paragraph::new(text).style(Style::default().fg(Color::Red));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmArchiveEpic => {
            let bar = Paragraph::new("Archive epic and subtasks? [y/n]")
                .style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::Help => {
            let bar = Paragraph::new("[?] or [Esc] to close help")
                .style(Style::default().fg(Color::Cyan));
            frame.render_widget(bar, area);
        }
        InputMode::RepoFilter => {
            let bar = Paragraph::new("Filter repos: [1-9] toggle  [a] all  [q/Esc] close")
                .style(Style::default().fg(Color::Cyan));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmWrapUp(_) => {
            let text = app
                .status
                .message
                .as_deref()
                .unwrap_or("Wrap up: [r] rebase  [p] create PR  [Esc] cancel");
            let bar = Paragraph::new(text).style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::InputPresetName => {
            let bar = Paragraph::new("Enter preset name, [Enter] save, [Esc] cancel")
                .style(Style::default().fg(Color::Cyan));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmDeletePreset => {
            let bar = Paragraph::new("[A-Z] delete preset  [Esc] cancel")
                .style(Style::default().fg(Color::Cyan));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmDeleteRepoPath => {
            let bar = Paragraph::new("Delete repo path? y to confirm, any key to cancel")
                .style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmEpicWrapUp(_) => {
            let text = app
                .status
                .message
                .as_deref()
                .unwrap_or("Epic wrap up: [r] rebase all  [p] PR all  [Esc] cancel");
            let bar = Paragraph::new(text).style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmDetachTmux(_) => {
            let text = app
                .status
                .message
                .as_deref()
                .unwrap_or("Detach tmux panel? [y/n]");
            let bar = Paragraph::new(text).style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmQuit => {
            let bar =
                Paragraph::new("Quit dispatch? [y/n]").style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::InputBaseBranch => {
            let text = app.status.message.as_deref().unwrap_or("Base branch: ");
            let bar = Paragraph::new(text).style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::InputWrapUpMode => {
            let text = app
                .status
                .message
                .as_deref()
                .unwrap_or("Wrap-up: [r]ebase  [p]r  [d]one  [Enter] skip");
            let bar = Paragraph::new(text).style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
        InputMode::MainSessionDir => {
            let text = format!(
                "Main session directory:  {}  [Enter] open  [Esc] cancel",
                app.input.buffer
            );
            let bar = Paragraph::new(text).style(Style::default().fg(Color::Cyan));
            frame.render_widget(bar, area);
        }
        InputMode::ReparentEpic(_) => {
            let bar = Paragraph::new("Select new parent: navigate tree above, Enter to select")
                .style(Style::default().fg(Color::Magenta));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmReparentEpic { .. } => {
            let text = app
                .status
                .message
                .as_deref()
                .unwrap_or("Reparent epic? [y/n]");
            let bar = Paragraph::new(text).style(Style::default().fg(Color::Magenta));
            frame.render_widget(bar, area);
        }
        InputMode::MoveTaskToEpic(_) => {
            let bar = Paragraph::new("Select target epic: navigate tree above, Enter to select")
                .style(Style::default().fg(Color::Magenta));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmMoveTaskToEpic { .. } => {
            let text = app
                .status
                .message
                .as_deref()
                .unwrap_or("Move task to epic? [y/n]");
            let bar = Paragraph::new(text).style(Style::default().fg(Color::Magenta));
            frame.render_widget(bar, area);
        }
        InputMode::ManagedFeedConfig => {
            let text =
                app.status.message.as_deref().unwrap_or(
                    "Managed feed config: Tab/arrows to move, Enter to save, Esc to cancel",
                );
            let bar = Paragraph::new(text).style(Style::default().fg(Color::Cyan));
            frame.render_widget(bar, area);
        }
        InputMode::TodoTitle | InputMode::TodoQuickAdd => {
            let label = if matches!(app.input.mode, InputMode::TodoTitle) {
                "New todo"
            } else {
                "Quick add"
            };
            let style = Style::default().fg(Color::Yellow);
            let prefix = format!("{label}: ");
            let suffix = "  [Enter] save  [Esc] cancel";
            let value_width = (area.width as usize)
                .saturating_sub(prefix.chars().count() + suffix.chars().count())
                .max(1);
            let mut line = crate::tui::ui::caret_line(
                prefix,
                &app.input.buffer,
                app.input.caret,
                value_width,
                style,
            );
            line.spans.push(Span::styled(suffix, style));
            let bar = Paragraph::new(line);
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmDeleteTodo => {
            let bar = Paragraph::new("Delete todo? [y/n]").style(Style::default().fg(Color::Red));
            frame.render_widget(bar, area);
        }
        InputMode::LinkTodoToTask(_) => {
            let text =
                app.status.message.as_deref().unwrap_or(
                    "Navigate to a task or epic and press Enter to link — Esc to cancel",
                );
            let bar = Paragraph::new(text).style(Style::default().fg(Color::Cyan));
            frame.render_widget(bar, area);
        }
        InputMode::ConfirmTrustRepo { .. } => {
            let text = app
                .status
                .message
                .as_deref()
                .unwrap_or("Repo not trusted — trust it? [y/N]");
            let bar = Paragraph::new(text).style(Style::default().fg(Color::Yellow));
            frame.render_widget(bar, area);
        }
    }
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
    push_hint("x", "archive");
    push_hint("a", "select all");
    push_hint("F", "flat");
    push_hint("Space", "toggle");
    push_hint("Esc", "clear");
    spans
}
