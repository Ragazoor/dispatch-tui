//! `board-visuals.allium`, "Bookmark", and
//! `tasks.allium: ToggleBookmarkFromBoard`.
use super::*;
use crate::models::{EpicBuilder, EpicId};
use crate::palette::{BOOKMARK, PURPLE, SIGNAL};
use crate::tui::commands::TaskCommand;
use crate::tui::types::Command;

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

fn stripe_fg_on_line(buf: &Buffer, y: u16) -> Color {
    (buf.area.left()..buf.area.right())
        .find(|&x| buf[(x, y)].symbol() == STRIPE)
        .map(|x| buf[(x, y)].fg)
        .expect("the card's stripe is on its title line")
}

fn set_bookmark_sent(cmds: &[Command]) -> Option<(TaskId, bool)> {
    cmds.iter().find_map(|c| match c {
        Command::Task(TaskCommand::SetBookmark { id, bookmarked }) => Some((*id, *bookmarked)),
        _ => None,
    })
}

#[tokio::test]
async fn bookmarked_task_card_shows_bookmark_glyph() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.bookmarked_tasks.insert(TaskId(1));
    let buf = render_to_buffer(&mut app, 120, 30);

    let cells = glyph_cells(&buf);
    assert_eq!(cells.len(), 1);
    assert_eq!(buf[cells[0]].fg, BOOKMARK);
}

#[tokio::test]
async fn epic_on_the_path_to_a_bookmarked_task_is_marked() {
    let mut app = app_with_task_two_epics_deep();
    app.board.bookmarked_tasks.insert(TaskId(1));
    assert!(app.board.epic_has_bookmark(EpicId(10)));
    assert!(app.board.epic_has_bookmark(EpicId(11)));

    let buf = render_to_buffer(&mut app, 120, 30);

    assert!(buffer_contains(&buf, "Root epic"));
    let cells = glyph_cells(&buf);
    assert_eq!(cells.len(), 1, "the root epic card");
    assert_eq!(buf[cells[0]].fg, BOOKMARK);
}

#[tokio::test]
async fn bookmarked_epic_stripe_stays_purple() {
    let mut app = app_with_task_two_epics_deep();
    app.board.bookmarked_tasks.insert(TaskId(1));
    let buf = render_to_buffer(&mut app, 120, 30);

    let (_, y) = glyph_cells(&buf)[0];
    assert_eq!(stripe_fg_on_line(&buf, y), PURPLE);
}

#[tokio::test]
async fn unseen_wins_the_glyph_slot_over_bookmark() {
    // A task card.
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.bookmarked_tasks.insert(TaskId(1));
    app.board.unseen_tasks.insert(TaskId(1));
    let buf = render_to_buffer(&mut app, 120, 30);
    let cells = glyph_cells(&buf);
    assert_eq!(cells.len(), 1, "one diamond, never two");
    assert_eq!(buf[cells[0]].fg, SIGNAL);

    // An epic card.
    let mut app = app_with_task_two_epics_deep();
    app.board.bookmarked_tasks.insert(TaskId(1));
    app.board.unseen_tasks.insert(TaskId(1));
    let buf = render_to_buffer(&mut app, 120, 30);
    let cells = glyph_cells(&buf);
    assert_eq!(cells.len(), 1, "one diamond, never two");
    assert_eq!(buf[cells[0]].fg, SIGNAL);
}

#[tokio::test]
async fn cursor_on_a_bookmarked_task_clears_nothing() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::BookmarksRefreshed(vec![TaskId(1)]),
    ));

    app.update(Message::NavigateRowFirst);

    assert!(app.board.bookmarked_tasks.contains(&TaskId(1)));
}

#[tokio::test]
async fn b_on_a_task_card_toggles_its_bookmark() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);

    let cmds = app.handle_key(make_key(KeyCode::Char('b')));
    assert_eq!(set_bookmark_sent(&cmds), Some((TaskId(1), true)));
    assert!(app.board.bookmarked_tasks.contains(&TaskId(1)));
    assert_eq!(app.status.message.as_deref(), Some("Bookmarked Task 1"));

    let cmds = app.handle_key(make_key(KeyCode::Char('b')));
    assert_eq!(set_bookmark_sent(&cmds), Some((TaskId(1), false)));
    assert!(app.board.bookmarked_tasks.is_empty());
    assert_eq!(
        app.status.message.as_deref(),
        Some("Bookmark removed from Task 1")
    );
}

#[tokio::test]
async fn b_on_a_done_task_is_refused() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Done)]);
    app.update(Message::NavigateColumn(3));

    let cmds = app.handle_key(make_key(KeyCode::Char('b')));

    assert_eq!(set_bookmark_sent(&cmds), None);
    assert!(app.board.bookmarked_tasks.is_empty());
    assert_eq!(
        app.status.message.as_deref(),
        Some("Task 1 is done and cannot be bookmarked")
    );
}

#[tokio::test]
async fn b_on_an_epic_card_does_nothing() {
    let mut app = app_with_task_two_epics_deep();
    assert!(
        app.selected_task().is_none(),
        "the cursor is on the epic card"
    );

    let cmds = app.handle_key(make_key(KeyCode::Char('b')));

    assert_eq!(set_bookmark_sent(&cmds), None);
    assert!(app.board.bookmarked_tasks.is_empty());
}
