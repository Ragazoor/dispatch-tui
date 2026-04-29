#![allow(unused_imports)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Modifier, Style},
    text::Span,
    Terminal,
};
use std::time::{Duration, Instant};

use super::*;
use crate::models::{
    DispatchMode, Epic, EpicId, SubStatus, TaskId, TaskStatus, TaskTag, DEFAULT_QUICK_TASK_TITLE,
};
use crate::tui::ViewMode;

pub(in crate::tui) const TEST_TIMEOUT: Duration = Duration::from_secs(300);

/// Check whether a rendered buffer contains the given text anywhere.
pub(in crate::tui) fn buffer_contains(buf: &Buffer, text: &str) -> bool {
    let area = buf.area();
    for y in area.top()..area.bottom() {
        let mut line = String::new();
        for x in area.left()..area.right() {
            line.push_str(buf[(x, y)].symbol());
        }
        if line.contains(text) {
            return true;
        }
    }
    false
}

/// Helper: render the app into a test terminal and return the buffer.
pub(in crate::tui) fn render_to_buffer(app: &mut App, width: u16, height: u16) -> Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| ui::render(f, app)).unwrap();
    terminal.backend().buffer().clone()
}

pub(in crate::tui) fn make_key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

pub(in crate::tui) fn make_task(id: i64, status: TaskStatus) -> Task {
    let now = chrono::Utc::now();
    Task {
        id: TaskId(id),
        title: format!("Task {id}"),
        description: String::new(),
        repo_path: String::from("/repo"),
        status,
        worktree: None,
        tmux_window: None,
        plan_path: None,
        epic_id: None,
        sub_status: SubStatus::default_for(status),
        pr_url: None,
        tag: None,
        sort_order: None,
        base_branch: "main".to_string(),
        external_id: None,
        created_at: now,
        updated_at: now,
        project_id: 1,
    }
}

pub(in crate::tui) fn make_app() -> App {
    App::new(
        vec![
            make_task(1, TaskStatus::Backlog),
            make_task(2, TaskStatus::Backlog),
            make_task(3, TaskStatus::Running),
            make_task(4, TaskStatus::Done),
        ],
        1,
        TEST_TIMEOUT,
    )
}

pub(in crate::tui) fn make_shift_key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::SHIFT)
}

/// Extract bold key spans (like "[d]", "[Tab]") from hint spans.
pub(in crate::tui) fn hint_keys<'a>(hints: &'a [ratatui::text::Span<'static>]) -> Vec<&'a str> {
    hints
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
        .map(|s| s.content.as_ref())
        .collect()
}

pub(in crate::tui) fn make_epic(id: i64) -> Epic {
    let now = chrono::Utc::now();
    Epic {
        id: EpicId(id),
        title: format!("Epic {id}"),
        description: String::new(),
        repo_path: "/repo".to_string(),
        status: TaskStatus::Backlog,
        plan_path: None,
        sort_order: None,
        auto_dispatch: true,
        parent_epic_id: None,
        feed_command: None,
        feed_interval_secs: None,
        created_at: now,
        updated_at: now,
        project_id: 1,
    }
}

pub(in crate::tui) fn make_epic_with_title(id: i64, title: &str) -> Epic {
    Epic {
        title: title.to_string(),
        ..make_epic(id)
    }
}

pub(in crate::tui) fn make_app_with_archived_task() -> App {
    let mut app = make_app();
    let mut t = make_task(10, TaskStatus::Archived);
    t.title = "archived task".to_string();
    app.board.tasks.push(t);
    app
}

/// Helper: create an app with one task + one epic in Backlog, cursor on the epic.
pub(in crate::tui) fn make_app_with_epic_selected() -> App {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    // Same priority (5), task (id=1) at row 0, epic (id=10) at row 1
    app.selection_mut().set_column(1); // Backlog = nav col 1
    app.selection_mut().set_row(1, 1);
    app
}

pub(in crate::tui) fn make_app_confirm_archive_epic() -> App {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], 1, TEST_TIMEOUT);
    app.board.epics = vec![make_epic(10)];
    app.selection_mut().set_column(1); // Backlog = nav col 1
    app.selection_mut().set_row(1, 1); // cursor on epic (same priority as task, sorts after by id)
    app.input.mode = InputMode::ConfirmArchiveEpic;
    app.status.message = Some("Archive epic and all subtasks? [y/n]".to_string());
    app
}

pub(in crate::tui) fn make_review_subtask(id: i64, epic_id: i64, sort_order: i64) -> Task {
    let mut task = make_task(id, TaskStatus::Review);
    task.epic_id = Some(EpicId(epic_id));
    task.worktree = Some(format!("/repo/.worktrees/{id}-task-{id}"));
    task.sort_order = Some(sort_order);
    task
}

/// Find a text span in the buffer and return the style of its first character.
pub(in crate::tui) fn find_style_of(buf: &Buffer, text: &str) -> Option<ratatui::style::Style> {
    let area = buf.area();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let remaining = (area.right() - x) as usize;
            if remaining < text.len() {
                break;
            }
            let segment: String = (0..text.len() as u16)
                .map(|dx| buf[(x + dx, y)].symbol().to_string())
                .collect();
            if segment == text {
                return Some(buf[(x, y)].style());
            }
        }
    }
    None
}

/// Extract the foreground color of the first `[` bracket in the given row.
pub(in crate::tui) fn first_bracket_fg(buf: &Buffer, row: u16) -> Option<Color> {
    let area = buf.area();
    for x in area.left()..area.right() {
        if buf[(x, row)].symbol() == "[" {
            return Some(buf[(x, row)].fg);
        }
    }
    None
}

pub(in crate::tui) fn make_tips() -> Vec<crate::tips::Tip> {
    vec![
        crate::tips::Tip {
            id: 1,
            title: "Tip One".into(),
            body: "Body one".into(),
        },
        crate::tips::Tip {
            id: 2,
            title: "Tip Two".into(),
            body: "Body two".into(),
        },
        crate::tips::Tip {
            id: 3,
            title: "Tip Three".into(),
            body: "Body three".into(),
        },
    ]
}

pub(in crate::tui) fn make_tip_with_id(id: u32) -> crate::tips::Tip {
    crate::tips::Tip {
        id,
        title: format!("Tip {id}"),
        body: format!("Body {id}"),
    }
}

pub(in crate::tui) fn determine_tips_start(
    tips: &[crate::tips::Tip],
    seen_up_to: u32,
    show_mode: crate::models::TipsShowMode,
) -> Option<usize> {
    crate::runtime::tips_starting_index(tips, seen_up_to, show_mode)
}
