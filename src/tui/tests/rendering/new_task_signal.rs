//! `board-visuals.allium`, "New-task signal", and
//! `tasks.allium: SeeTaskByLandingOnIt`.
use super::*;
use crate::models::{EpicBuilder, EpicId};
use crate::palette::SIGNAL;

const GLYPH: &str = "\u{25c6}";
const STRIPE: &str = "\u{258e}";

/// Two epics, `10` (a root) holding `11`, and a task in `11`.
fn app_with_task_two_epics_deep() -> App {
    let mut app = App::new(vec![]);
    app.board.epics = vec![
        EpicBuilder::new(10).title("Root epic").build(),
        EpicBuilder::new(11)
            .title("Child epic")
            .parent(Some(10))
            .build(),
    ];
    let mut task = make_task(1, TaskStatus::Backlog);
    task.epic_id = Some(EpicId(11));
    app.board.tasks = vec![task];
    app
}

fn glyph_cells(buf: &Buffer) -> Vec<(u16, u16)> {
    let mut found = vec![];
    for y in buf.area.top()..buf.area.bottom() {
        for x in buf.area.left()..buf.area.right() {
            if buf[(x, y)].symbol() == GLYPH {
                found.push((x, y));
            }
        }
    }
    found
}

#[tokio::test]
async fn unseen_task_card_shows_signal_glyph() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    let quiet = render_to_buffer(&mut app, 120, 30);
    assert!(!buffer_contains(&quiet, GLYPH), "no mark, no glyph");

    app.board.unseen_tasks.insert(TaskId(1));
    let buf = render_to_buffer(&mut app, 120, 30);

    let cells = glyph_cells(&buf);
    assert_eq!(cells.len(), 1, "exactly the one marked card");
    assert_eq!(buf[cells[0]].fg, SIGNAL);
}

#[tokio::test]
async fn unseen_task_card_stripe_keeps_column_hue() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.unseen_tasks.insert(TaskId(1));
    let buf = render_to_buffer(&mut app, 120, 30);

    let (_, y) = glyph_cells(&buf)[0];
    let stripe_fg = (buf.area.left()..buf.area.right())
        .find(|&x| buf[(x, y)].symbol() == STRIPE)
        .map(|x| buf[(x, y)].fg)
        .expect("the card's stripe is on its title line");
    assert_ne!(stripe_fg, SIGNAL, "a task stripe carries column identity");
}

#[tokio::test]
async fn epic_on_the_path_to_an_unseen_task_is_marked() {
    let mut app = app_with_task_two_epics_deep();
    app.board.unseen_tasks.insert(TaskId(1));

    // The root epic's card is on the main board; the task sits two levels down.
    let buf = render_to_buffer(&mut app, 120, 30);

    assert!(buffer_contains(&buf, "Root epic"));
    assert!(
        !glyph_cells(&buf).is_empty(),
        "an epic whose subtree holds an unseen task draws the glyph"
    );
}

#[tokio::test]
async fn marked_epic_stripe_is_signal_not_purple() {
    let mut app = app_with_task_two_epics_deep();
    app.board.unseen_tasks.insert(TaskId(1));
    let buf = render_to_buffer(&mut app, 120, 30);

    let (_, y) = glyph_cells(&buf)[0];
    let stripe_fg = (buf.area.left()..buf.area.right())
        .find(|&x| buf[(x, y)].symbol() == STRIPE)
        .map(|x| buf[(x, y)].fg)
        .expect("the epic card's stripe is on its title line");
    assert_eq!(stripe_fg, SIGNAL);
}

#[tokio::test]
async fn epic_mark_clears_when_the_last_unseen_task_below_it_is_seen() {
    let mut app = app_with_task_two_epics_deep();
    app.board.unseen_tasks.insert(TaskId(1));
    assert!(app.board.epic_has_unseen(EpicId(10)));
    assert!(app.board.epic_has_unseen(EpicId(11)));

    app.board.unseen_tasks.clear();

    assert!(!app.board.epic_has_unseen(EpicId(10)));
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(!buffer_contains(&buf, GLYPH));
}

#[tokio::test]
async fn signal_never_colours_the_card_frame() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.unseen_tasks.insert(TaskId(1));
    let buf = render_to_buffer(&mut app, 120, 30);

    let (_, y) = glyph_cells(&buf)[0];
    // The frame's rails flank the title line the glyph sits on.
    let rail = (buf.area.left()..buf.area.right())
        .find(|&x| buf[(x, y)].symbol() == "\u{2502}")
        .expect("the card's left rail");
    assert_ne!(buf[(rail, y)].fg, SIGNAL);
}

fn mark_seen_sent(cmds: &[crate::tui::types::Command], id: TaskId) -> bool {
    cmds.iter().any(|c| {
        matches!(
            c,
            crate::tui::types::Command::Task(crate::tui::commands::TaskCommand::MarkSeen(t))
                if *t == id
        )
    })
}

fn any_mark_seen_sent(cmds: &[crate::tui::types::Command]) -> bool {
    cmds.iter().any(|c| {
        matches!(
            c,
            crate::tui::types::Command::Task(crate::tui::commands::TaskCommand::MarkSeen(_))
        )
    })
}

/// One backlog task under the cursor, which a refresh then marked unseen.
fn app_with_one_task_a_refresh_marked_unseen() -> App {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::UnseenRefreshed(vec![TaskId(1)]),
    ));
    app
}

/// Epic `10` holding two backlog tasks, `2` and `3`, both unseen.
fn app_with_two_new_tasks_in_one_epic() -> App {
    let mut app = App::new(vec![]);
    app.board.epics = vec![EpicBuilder::new(10).title("Root epic").build()];
    for id in [2, 3] {
        let mut task = make_task(id, TaskStatus::Backlog);
        task.epic_id = Some(EpicId(10));
        app.board.tasks.push(task);
    }
    app.board.unseen_tasks = [TaskId(2), TaskId(3)].into_iter().collect();
    app
}

#[tokio::test]
async fn cursor_landing_on_an_unseen_task_clears_only_it() {
    let mut app = app_with_two_new_tasks_in_one_epic();

    // Space on the epic card enters it.
    let cmds = app.handle_key(make_key(KeyCode::Char(' ')));

    let landed = app.selected_task().expect("the cursor is on a task").id;
    let other = if landed == TaskId(2) {
        TaskId(3)
    } else {
        TaskId(2)
    };
    assert!(!app.board.unseen_tasks.contains(&landed));
    assert!(
        app.board.unseen_tasks.contains(&other),
        "entering the epic does not see the card the cursor is not on"
    );
    assert!(app.board.epic_has_unseen(EpicId(10)));
    assert!(mark_seen_sent(&cmds, landed));
    assert!(!mark_seen_sent(&cmds, other));

    // The sibling still draws its own glyph inside the epic.
    let buf = render_to_buffer(&mut app, 120, 30);
    assert_eq!(glyph_cells(&buf).len(), 1);
}

#[tokio::test]
async fn moving_the_cursor_onto_an_unseen_task_clears_it() {
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Backlog),
    ]);
    let first = app.selected_task().expect("a task card").id;
    let second = if first == TaskId(1) {
        TaskId(2)
    } else {
        TaskId(1)
    };
    app.board.unseen_tasks.insert(second);

    let cmds = app.handle_key(make_key(KeyCode::Char('j')));

    assert_eq!(app.selected_task().map(|t| t.id), Some(second));
    assert!(app.board.unseen_tasks.is_empty());
    assert!(mark_seen_sent(&cmds, second));
}

/// Task #99089: the board may sit in a tmux window nobody is looking at. A
/// new PR that a pull sorts into the row under a still cursor must keep its
/// mark until the operator presses a key.
#[tokio::test]
async fn a_refresh_that_slides_an_unseen_task_under_the_cursor_keeps_it() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    assert_eq!(app.selected_task().map(|t| t.id), Some(TaskId(1)));

    let cmds = app.update(Message::Task(
        crate::tui::messages::TaskMessage::UnseenRefreshed(vec![TaskId(1)]),
    ));

    assert!(app.board.unseen_tasks.contains(&TaskId(1)));
    assert!(!any_mark_seen_sent(&cmds));
}

#[tokio::test]
async fn a_key_with_the_cursor_on_an_unseen_task_clears_it() {
    let mut app = app_with_one_task_a_refresh_marked_unseen();

    // `[` on the only card does not move the cursor; the operator is at the
    // board all the same.
    let cmds = app.handle_key(make_key(KeyCode::Char('[')));

    assert!(app.board.unseen_tasks.is_empty());
    assert!(mark_seen_sent(&cmds, TaskId(1)));
}

#[tokio::test]
async fn a_key_in_an_overlay_sees_nothing() {
    let mut app = app_with_one_task_a_refresh_marked_unseen();
    app.input.mode = InputMode::Help;

    let cmds = app.handle_key(make_key(KeyCode::Char('j')));

    assert!(matches!(app.input.mode, InputMode::Help));
    assert!(app.board.unseen_tasks.contains(&TaskId(1)));
    assert!(!any_mark_seen_sent(&cmds));
}

#[tokio::test]
async fn cursor_on_a_marked_epic_clears_nothing() {
    let mut app = app_with_task_two_epics_deep();
    app.board.unseen_tasks.insert(TaskId(1));
    assert!(
        app.selected_task().is_none(),
        "the cursor is on the epic card"
    );

    let cmds = app.handle_key(make_key(KeyCode::Char('[')));

    assert!(app.board.unseen_tasks.contains(&TaskId(1)));
    assert!(!any_mark_seen_sent(&cmds));
}

#[tokio::test]
async fn entering_an_epic_whose_new_task_is_deeper_clears_nothing() {
    let mut app = app_with_task_two_epics_deep();
    app.board.unseen_tasks.insert(TaskId(1));

    let cmds = app.handle_key(make_key(KeyCode::Char(' ')));

    assert!(matches!(
        app.board.view_mode,
        ViewMode::Epic {
            epic_id: EpicId(10),
            ..
        }
    ));
    assert!(app.board.unseen_tasks.contains(&TaskId(1)));
    assert!(app.board.epic_has_unseen(EpicId(11)));
    assert!(!any_mark_seen_sent(&cmds));
}

#[tokio::test]
async fn a_card_already_seen_sends_nothing() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);

    let cmds = app.handle_key(make_key(KeyCode::Char('[')));

    assert!(!any_mark_seen_sent(&cmds));
}
