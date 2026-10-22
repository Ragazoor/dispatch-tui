//! `board-visuals.allium`, "New-task signal", and
//! `tasks.allium: SeeTasksByEnteringTheirEpic`.
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

#[tokio::test]
async fn entering_an_epic_sees_its_own_tasks_and_keeps_the_child_marked() {
    let mut app = app_with_task_two_epics_deep();
    let mut shallow = make_task(2, TaskStatus::Backlog);
    shallow.epic_id = Some(EpicId(10));
    app.board.tasks.push(shallow);
    app.board.unseen_tasks = [TaskId(1), TaskId(2)].into_iter().collect();

    let cmds = app.handle_enter_epic(EpicId(10));

    assert!(!app.board.unseen_tasks.contains(&TaskId(2)));
    assert!(app.board.unseen_tasks.contains(&TaskId(1)));
    assert!(app.board.epic_has_unseen(EpicId(11)));
    assert!(cmds.iter().any(|c| matches!(
        c,
        crate::tui::types::Command::Epic(crate::tui::commands::EpicCommand::MarkSeen(EpicId(10)))
    )));
}

#[tokio::test]
async fn entering_an_epic_with_nothing_unseen_sends_nothing() {
    let mut app = app_with_task_two_epics_deep();

    let cmds = app.handle_enter_epic(EpicId(10));

    assert!(cmds.is_empty());
}

#[tokio::test]
async fn a_jump_sees_only_the_epic_that_opens() {
    let mut app = app_with_task_two_epics_deep();
    let mut shallow = make_task(2, TaskStatus::Backlog);
    shallow.epic_id = Some(EpicId(10));
    app.board.tasks.push(shallow);
    app.board.unseen_tasks = [TaskId(1), TaskId(2)].into_iter().collect();

    app.handle_jump_to_epic(EpicId(11));

    assert!(
        app.board.unseen_tasks.contains(&TaskId(2)),
        "the epic passed over was never looked at"
    );
    assert!(!app.board.unseen_tasks.contains(&TaskId(1)));
}
