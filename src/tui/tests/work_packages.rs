//! Work packages on the board: the package card, the package view, the link
//! picker, and rename/delete from a package card.
//!
//! Obligations from `docs/specs/work-packages.allium`.
use super::*;
use crate::models::{
    EpicId, TaskId, TaskStatus, WorkPackage, WorkPackageId, WorkPackageItem, WorkPackageLink,
};
use crate::tui::commands::WorkPackageCommand;
use crate::tui::messages::WorkPackageMessage;
use crossterm::event::KeyCode;

fn make_package(id: i64, title: &str) -> WorkPackage {
    let at = chrono::DateTime::from_timestamp(1_700_000_000, 0).expect("valid timestamp");
    WorkPackage {
        id: WorkPackageId(id),
        title: title.to_string(),
        description: String::new(),
        created_by: None,
        created_at: at,
        updated_at: at,
    }
}

fn link(package: i64, item: WorkPackageItem) -> WorkPackageLink {
    WorkPackageLink {
        work_package_id: WorkPackageId(package),
        item,
    }
}

fn task_item(id: i64) -> WorkPackageItem {
    WorkPackageItem::Task(TaskId(id))
}

fn epic_item(id: i64) -> WorkPackageItem {
    WorkPackageItem::Epic(EpicId(id))
}

fn epic_task(id: i64, epic_id: i64, status: TaskStatus) -> crate::models::Task {
    crate::models::Task {
        epic_id: Some(EpicId(epic_id)),
        ..make_task(id, status)
    }
}

/// The columns whose card list holds package `id`, in board order.
fn package_columns(app: &App, id: i64) -> Vec<TaskStatus> {
    TaskStatus::ALL
        .iter()
        .copied()
        .filter(|&status| {
            app.view()
                .column_items_for_status(status)
                .iter()
                .any(|i| matches!(i, ColumnItem::WorkPackage(p) if p.id == WorkPackageId(id)))
        })
        .collect()
}

/// Put the cursor on the first package card in `status`'s column.
fn cursor_on_package(app: &mut App, status: TaskStatus) {
    let col = TaskStatus::ALL
        .iter()
        .position(|s| *s == status)
        .expect("a board status")
        + 1;
    let row = app
        .view()
        .column_items_for_status(status)
        .iter()
        .filter(|i| i.is_selectable())
        .position(|i| matches!(i, ColumnItem::WorkPackage(_)))
        .expect("a package card in the column");
    app.selection_mut().set_column(col);
    app.selection_mut().set_row(col, row);
}

/// Put the cursor on the item matching `pred` in `status`'s column.
fn cursor_on(app: &mut App, status: TaskStatus, pred: impl Fn(&ColumnItem) -> bool) {
    let col = TaskStatus::ALL
        .iter()
        .position(|s| *s == status)
        .expect("a board status")
        + 1;
    let row = app
        .view()
        .column_items_for_status(status)
        .iter()
        .filter(|i| i.is_selectable())
        .position(pred)
        .expect("the item is in the column");
    app.selection_mut().set_column(col);
    app.selection_mut().set_row(col, row);
}

/// Board: epic #1 (task #1 Running, task #2 Review), a standalone Backlog task
/// #3, and package #7 linking task #3 and epic #1.
fn board_with_package() -> App {
    let mut app = App::new(vec![
        epic_task(1, 1, TaskStatus::Running),
        epic_task(2, 1, TaskStatus::Review),
        make_task(3, TaskStatus::Backlog),
    ]);
    app.board.epics.push(make_epic(1));
    app.board.work_packages = vec![make_package(7, "Window 1")];
    app.board.work_package_links = vec![link(7, task_item(3)), link(7, epic_item(1))];
    app
}

fn enter_package(app: &mut App, id: i64) {
    app.update(Message::WorkPackage(WorkPackageMessage::Enter(
        WorkPackageId(id),
    )));
}

// ---------------------------------------------------------------------------
// PackageCardPlacement
// ---------------------------------------------------------------------------

#[test]
fn package_card_appears_in_every_column_its_linked_work_has() {
    let app = board_with_package();
    assert_eq!(
        package_columns(&app, 7),
        vec![TaskStatus::Backlog, TaskStatus::Running, TaskStatus::Review]
    );
}

#[test]
fn a_linked_epics_subtree_places_the_package_card() {
    // The only work sits in sub-epic #2 under the linked epic #1.
    let mut app = App::new(vec![epic_task(1, 2, TaskStatus::Done)]);
    let mut sub = make_epic(2);
    sub.parent_epic_id = Some(EpicId(1));
    app.board.epics = vec![make_epic(1), sub];
    app.board.work_packages = vec![make_package(7, "W")];
    app.board.work_package_links = vec![link(7, epic_item(1))];

    assert_eq!(package_columns(&app, 7), vec![TaskStatus::Done]);
}

#[test]
fn an_empty_package_card_sits_in_backlog() {
    let mut app = App::new(vec![]);
    app.board.work_packages = vec![make_package(7, "W")];
    assert_eq!(package_columns(&app, 7), vec![TaskStatus::Backlog]);
}

#[test]
fn a_package_whose_work_is_all_filtered_away_falls_back_to_backlog() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Done)]);
    app.board.work_packages = vec![make_package(7, "W")];
    app.board.work_package_links = vec![link(7, task_item(3))];
    app.filter.only_active = true;

    assert_eq!(package_columns(&app, 7), vec![TaskStatus::Backlog]);
}

#[test]
fn package_cards_sort_above_every_other_card_in_package_id_order() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.epics.push(make_epic(2));
    // Delivered out of order: the board keeps its packages in id order.
    app.update(Message::WorkPackage(WorkPackageMessage::Refresh {
        packages: vec![make_package(9, "B"), make_package(8, "A")],
        links: vec![],
    }));

    let items = app.view().column_items_for_status(TaskStatus::Backlog);
    assert!(
        matches!(items[0], ColumnItem::WorkPackage(p) if p.id == WorkPackageId(8)),
        "{items:?}"
    );
    assert!(matches!(items[1], ColumnItem::WorkPackage(p) if p.id == WorkPackageId(9)));
    assert!(matches!(items[2], ColumnItem::Task(_)));
    assert!(matches!(items[3], ColumnItem::Epic(_)));
}

#[test]
fn package_cards_come_before_the_first_section_header() {
    let app = board_with_package();
    let items = app.view().column_items_for_status(TaskStatus::Running);
    assert!(
        matches!(items[0], ColumnItem::WorkPackage(_)),
        "package card leads the Running column: {items:?}"
    );
    assert!(matches!(items[1], ColumnItem::SubstatusLabel(_)));
}

#[test]
fn a_search_matching_the_package_title_keeps_its_card() {
    let mut app = board_with_package();
    app.search.query = "window".to_string();
    assert!(!package_columns(&app, 7).is_empty());
}

#[test]
fn a_search_matching_linked_work_keeps_the_card() {
    let mut app = board_with_package();
    let title = app.board.tasks[2].title.clone();
    app.search.query = title;
    assert_eq!(package_columns(&app, 7), vec![TaskStatus::Backlog]);
}

#[test]
fn a_search_matching_neither_title_nor_work_hides_the_card() {
    let mut app = board_with_package();
    app.search.query = "zzz-no-such-thing".to_string();
    assert!(package_columns(&app, 7).is_empty());
}

#[test]
fn no_package_card_is_drawn_inside_an_epic_view() {
    let mut app = board_with_package();
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(1),
    )));
    assert!(package_columns(&app, 7).is_empty());
}

// ---------------------------------------------------------------------------
// Package view
// ---------------------------------------------------------------------------

#[test]
fn space_on_a_package_card_opens_the_package_view() {
    let mut app = board_with_package();
    app.selection_mut().set_column(2);
    cursor_on_package(&mut app, TaskStatus::Backlog);

    app.handle_key(make_key(KeyCode::Char(' ')));

    match &app.board.view_mode {
        ViewMode::Package {
            package_id, parent, ..
        } => {
            assert_eq!(*package_id, WorkPackageId(7));
            match parent.as_ref() {
                ViewMode::Board(sel) => assert_eq!(sel.column(), 1),
                other => panic!("parent should be the board, got {other:?}"),
            }
        }
        other => panic!("expected the package view, got {other:?}"),
    }
}

#[test]
fn the_package_view_holds_the_linked_items_and_nothing_else() {
    let mut app = board_with_package();
    // An unlinked standalone task must not show.
    app.board.tasks.push(make_task(4, TaskStatus::Backlog));
    enter_package(&mut app, 7);

    let backlog = app.view().column_items_for_status(TaskStatus::Backlog);
    let task_ids: Vec<TaskId> = backlog
        .iter()
        .filter_map(|i| match i {
            ColumnItem::Task(t) => Some(t.id),
            _ => None,
        })
        .collect();
    assert_eq!(task_ids, vec![TaskId(3)]);

    // The linked epic is an epic card, placed by its subtree.
    let running = app.view().column_items_for_status(TaskStatus::Running);
    assert!(running
        .iter()
        .any(|i| matches!(i, ColumnItem::Epic(e) if e.id == EpicId(1))));
    // Its tasks are not cards of their own.
    assert!(!running.iter().any(|i| matches!(i, ColumnItem::Task(_))));
    // No package card inside a package view.
    assert!(package_columns(&app, 7).is_empty());
}

#[test]
fn a_linked_task_inside_an_epic_is_its_own_card() {
    let mut app = App::new(vec![epic_task(1, 1, TaskStatus::Running)]);
    app.board.epics.push(make_epic(1));
    app.board.work_packages = vec![make_package(7, "W")];
    app.board.work_package_links = vec![link(7, task_item(1)), link(7, epic_item(1))];
    enter_package(&mut app, 7);

    let running = app.view().column_items_for_status(TaskStatus::Running);
    assert!(running
        .iter()
        .any(|i| matches!(i, ColumnItem::Task(t) if t.id == TaskId(1))));
    assert!(running
        .iter()
        .any(|i| matches!(i, ColumnItem::Epic(e) if e.id == EpicId(1))));
}

#[test]
fn a_linked_sub_epic_is_an_epic_card_in_the_package_view() {
    let mut app = App::new(vec![epic_task(1, 2, TaskStatus::Backlog)]);
    let mut sub = make_epic(2);
    sub.parent_epic_id = Some(EpicId(1));
    app.board.epics = vec![make_epic(1), sub];
    app.board.work_packages = vec![make_package(7, "W")];
    app.board.work_package_links = vec![link(7, epic_item(2))];
    enter_package(&mut app, 7);

    let backlog = app.view().column_items_for_status(TaskStatus::Backlog);
    let epics: Vec<EpicId> = backlog
        .iter()
        .filter_map(|i| match i {
            ColumnItem::Epic(e) => Some(e.id),
            _ => None,
        })
        .collect();
    assert_eq!(epics, vec![EpicId(2)]);
}

#[test]
fn flattening_is_not_applied_in_a_package_view() {
    let mut app = board_with_package();
    app.board.flattened = true;
    enter_package(&mut app, 7);

    let running = app.view().column_items_for_status(TaskStatus::Running);
    assert!(running
        .iter()
        .any(|i| matches!(i, ColumnItem::Epic(e) if e.id == EpicId(1))));
}

#[test]
fn q_leaves_the_package_view_for_the_main_board() {
    let mut app = board_with_package();
    enter_package(&mut app, 7);
    app.handle_key(make_key(KeyCode::Char('q')));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn esc_leaves_the_package_view_when_no_search_is_active() {
    let mut app = board_with_package();
    enter_package(&mut app, 7);
    app.handle_key(make_key(KeyCode::Esc));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn an_epic_entered_from_a_package_returns_to_the_package_on_q() {
    let mut app = board_with_package();
    enter_package(&mut app, 7);
    cursor_on(
        &mut app,
        TaskStatus::Running,
        |i| matches!(i, ColumnItem::Epic(e) if e.id == EpicId(1)),
    );
    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(matches!(
        app.board.view_mode,
        ViewMode::Epic { epic_id, .. } if epic_id == EpicId(1)
    ));

    app.handle_key(make_key(KeyCode::Char('q')));
    assert!(matches!(
        app.board.view_mode,
        ViewMode::Package { package_id, .. } if package_id == WorkPackageId(7)
    ));
}

#[test]
fn shift_q_from_an_epic_below_a_package_returns_to_the_main_board() {
    let mut app = board_with_package();
    enter_package(&mut app, 7);
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(1),
    )));
    app.handle_key(make_key(KeyCode::Char('Q')));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn a_deepest_jump_from_a_package_starts_its_trail_at_the_package_view() {
    // Linked epic #2 is a sub-epic of #1; its work sits in its own sub-epic #3.
    let mut app = App::new(vec![epic_task(1, 3, TaskStatus::Backlog)]);
    let mut two = make_epic(2);
    two.parent_epic_id = Some(EpicId(1));
    let mut three = make_epic(3);
    three.parent_epic_id = Some(EpicId(2));
    app.board.epics = vec![make_epic(1), two, three];
    app.board.work_packages = vec![make_package(7, "W")];
    app.board.work_package_links = vec![link(7, epic_item(2))];
    enter_package(&mut app, 7);

    app.update(Message::Epic(
        crate::tui::messages::EpicMessage::JumpToDeepest(EpicId(2), TaskStatus::Backlog),
    ));

    // Trail: package -> #2 -> #3. Epic #1 (above the linked epic) is not on it.
    let ViewMode::Epic {
        epic_id, parent, ..
    } = &app.board.view_mode
    else {
        panic!("expected an epic view, got {:?}", app.board.view_mode);
    };
    assert_eq!(*epic_id, EpicId(3));
    let ViewMode::Epic {
        epic_id: mid,
        parent: top,
        ..
    } = parent.as_ref()
    else {
        panic!("expected epic #2 below the package");
    };
    assert_eq!(*mid, EpicId(2));
    assert!(matches!(top.as_ref(), ViewMode::Package { .. }));
}

#[test]
fn quick_dispatch_in_a_package_view_makes_a_plain_task() {
    let mut app = board_with_package();
    enter_package(&mut app, 7);
    app.handle_key(make_key(KeyCode::Char('D')));
    assert_eq!(app.input.pending_epic_id, None);
}

// ---------------------------------------------------------------------------
// Refresh
// ---------------------------------------------------------------------------

#[test]
fn a_refresh_replaces_the_boards_packages_and_links() {
    let mut app = App::new(vec![]);
    app.update(Message::WorkPackage(WorkPackageMessage::Refresh {
        packages: vec![make_package(1, "A")],
        links: vec![link(1, task_item(5))],
    }));
    assert_eq!(app.board.work_packages.len(), 1);
    assert_eq!(app.board.work_package_links, vec![link(1, task_item(5))]);
}

#[test]
fn a_refresh_that_drops_the_open_package_returns_to_the_board() {
    let mut app = board_with_package();
    enter_package(&mut app, 7);
    app.update(Message::WorkPackage(WorkPackageMessage::Refresh {
        packages: vec![],
        links: vec![],
    }));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

// ---------------------------------------------------------------------------
// WorkPackageLinkPicker
// ---------------------------------------------------------------------------

fn picker_cursor(app: &App) -> usize {
    match &app.input.mode {
        InputMode::LinkWorkPackage { cursor, .. } => *cursor,
        other => panic!("expected the link picker, got {other:?}"),
    }
}

/// Board with packages #7 (links task #3) and #8 (empty); cursor on task #3.
fn picker_board() -> App {
    let mut app = board_with_package();
    app.board.work_packages.push(make_package(8, "Window 2"));
    cursor_on(
        &mut app,
        TaskStatus::Backlog,
        |i| matches!(i, ColumnItem::Task(t) if t.id == TaskId(3)),
    );
    app
}

#[test]
fn w_on_a_task_card_opens_the_link_picker_for_it() {
    let mut app = picker_board();
    app.handle_key(make_key(KeyCode::Char('w')));
    assert_eq!(
        app.input.mode,
        InputMode::LinkWorkPackage {
            item: task_item(3),
            cursor: 0
        }
    );
}

#[test]
fn w_on_an_epic_card_opens_the_link_picker_for_it() {
    let mut app = board_with_package();
    cursor_on(
        &mut app,
        TaskStatus::Running,
        |i| matches!(i, ColumnItem::Epic(e) if e.id == EpicId(1)),
    );
    app.handle_key(make_key(KeyCode::Char('w')));
    assert!(matches!(
        app.input.mode,
        InputMode::LinkWorkPackage { item, .. } if item == epic_item(1)
    ));
}

#[test]
fn w_on_a_package_card_does_nothing() {
    let mut app = board_with_package();
    cursor_on_package(&mut app, TaskStatus::Backlog);
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('w'))));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn toggling_a_linked_row_unlinks_and_keeps_the_picker_open() {
    let mut app = picker_board();
    app.handle_key(make_key(KeyCode::Char('w')));
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char(' '))));
    assert!(
        matches!(
            cmds.as_slice(),
            [Command::WorkPackage(WorkPackageCommand::Unlink { id, item })]
                if *id == WorkPackageId(7) && *item == task_item(3)
        ),
        "{cmds:?}"
    );
    assert!(!app
        .board
        .work_package_links
        .contains(&link(7, task_item(3))));
    assert_eq!(picker_cursor(&app), 0);
}

#[test]
fn toggling_an_unlinked_row_links() {
    let mut app = picker_board();
    app.handle_key(make_key(KeyCode::Char('w')));
    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(picker_cursor(&app), 1);
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Enter)));
    assert!(
        matches!(
            cmds.as_slice(),
            [Command::WorkPackage(WorkPackageCommand::Link { id, item })]
                if *id == WorkPackageId(8) && *item == task_item(3)
        ),
        "{cmds:?}"
    );
    assert!(app
        .board
        .work_package_links
        .contains(&link(8, task_item(3))));
}

#[test]
fn the_picker_cursor_stops_at_the_new_package_row() {
    let mut app = picker_board();
    app.handle_key(make_key(KeyCode::Char('w')));
    for _ in 0..5 {
        app.handle_key(make_key(KeyCode::Char('j')));
    }
    assert_eq!(picker_cursor(&app), 2, "two packages + the new-package row");
    for _ in 0..5 {
        app.handle_key(make_key(KeyCode::Char('k')));
    }
    assert_eq!(picker_cursor(&app), 0);
}

#[test]
fn the_new_package_row_creates_a_package_linked_to_the_item() {
    let mut app = picker_board();
    app.handle_key(make_key(KeyCode::Char('w')));
    app.handle_key(make_key(KeyCode::Char('j')));
    app.handle_key(make_key(KeyCode::Char('j')));
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(
        app.input.mode,
        InputMode::InputWorkPackageTitle(task_item(3))
    );
    for c in "Window 3".chars() {
        app.handle_key(make_key(KeyCode::Char(c)));
    }
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Enter)));
    assert!(
        matches!(
            cmds.as_slice(),
            [Command::WorkPackage(WorkPackageCommand::Create { title, item })]
                if title == "Window 3" && *item == task_item(3)
        ),
        "{cmds:?}"
    );
    // Back in the picker, the cursor on the row the new package will fill.
    assert_eq!(picker_cursor(&app), 2);
}

#[test]
fn an_empty_new_package_title_creates_nothing_and_returns_to_the_picker() {
    let mut app = picker_board();
    app.handle_key(make_key(KeyCode::Char('w')));
    app.handle_key(make_key(KeyCode::Char('j')));
    app.handle_key(make_key(KeyCode::Char('j')));
    app.handle_key(make_key(KeyCode::Enter));
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Enter)));
    assert!(cmds.is_empty(), "{cmds:?}");
    assert!(matches!(app.input.mode, InputMode::LinkWorkPackage { .. }));
}

#[test]
fn esc_in_the_new_package_title_returns_to_the_picker() {
    let mut app = picker_board();
    app.handle_key(make_key(KeyCode::Char('w')));
    app.handle_key(make_key(KeyCode::Char('j')));
    app.handle_key(make_key(KeyCode::Char('j')));
    app.handle_key(make_key(KeyCode::Enter));
    app.handle_key(make_key(KeyCode::Esc));
    assert!(matches!(app.input.mode, InputMode::LinkWorkPackage { .. }));
}

#[test]
fn esc_closes_the_picker() {
    let mut app = picker_board();
    app.handle_key(make_key(KeyCode::Char('w')));
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn q_closes_the_picker() {
    let mut app = picker_board();
    app.handle_key(make_key(KeyCode::Char('w')));
    app.handle_key(make_key(KeyCode::Char('q')));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn the_picker_renders_every_package_with_its_check_mark() {
    let mut app = picker_board();
    app.handle_key(make_key(KeyCode::Char('w')));
    let buf = render_to_buffer(&mut app, 120, 40);
    assert!(buffer_contains(&buf, "[x] Window 1"));
    assert!(buffer_contains(&buf, "[ ] Window 2"));
    assert!(buffer_contains(&buf, "new package"));
}

// ---------------------------------------------------------------------------
// Rename / delete from a package card
// ---------------------------------------------------------------------------

#[test]
fn e_on_a_package_card_opens_a_prefilled_rename_input() {
    let mut app = board_with_package();
    cursor_on_package(&mut app, TaskStatus::Backlog);
    app.handle_key(make_key(KeyCode::Char('e')));
    assert_eq!(
        app.input.mode,
        InputMode::RenameWorkPackage(WorkPackageId(7))
    );
    assert_eq!(app.input.buffer, "Window 1");
}

#[test]
fn enter_in_the_rename_input_saves_the_new_title() {
    let mut app = board_with_package();
    cursor_on_package(&mut app, TaskStatus::Backlog);
    app.handle_key(make_key(KeyCode::Char('e')));
    app.handle_key(make_key(KeyCode::Char('!')));
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Enter)));
    assert!(
        matches!(
            cmds.as_slice(),
            [Command::WorkPackage(WorkPackageCommand::Rename { id, title })]
                if *id == WorkPackageId(7) && title == "Window 1!"
        ),
        "{cmds:?}"
    );
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn an_empty_rename_cancels() {
    let mut app = board_with_package();
    cursor_on_package(&mut app, TaskStatus::Backlog);
    app.handle_key(make_key(KeyCode::Char('e')));
    app.input.buffer.clear();
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Enter)));
    assert!(cmds.is_empty(), "{cmds:?}");
    assert_eq!(app.input.mode, InputMode::Normal);
}

// The status message carrying the prompt auto-clears after STATUS_MESSAGE_TTL
// while the confirm mode lives on, so the prompt must still name the package.
#[test]
fn the_delete_prompt_still_names_the_package_after_the_status_message_expires() {
    let mut app = board_with_package();
    cursor_on_package(&mut app, TaskStatus::Backlog);
    app.handle_key(make_key(KeyCode::Char('x')));
    app.status.message = None;
    let buf = render_to_buffer(&mut app, 160, 40);
    assert!(
        buffer_contains(&buf, "Delete work package \"Window 1\"?"),
        "the expired prompt must still name the package"
    );
}

#[test]
fn x_on_a_package_card_asks_before_deleting() {
    let mut app = board_with_package();
    cursor_on_package(&mut app, TaskStatus::Backlog);
    app.handle_key(make_key(KeyCode::Char('x')));
    assert_eq!(
        app.input.mode,
        InputMode::ConfirmDeleteWorkPackage(WorkPackageId(7))
    );
    assert!(app
        .status
        .message
        .as_deref()
        .is_some_and(|m| m.contains("Delete work package \"Window 1\"?")));

    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('y'))));
    assert!(
        matches!(
            cmds.as_slice(),
            [Command::WorkPackage(WorkPackageCommand::Delete(id))] if *id == WorkPackageId(7)
        ),
        "{cmds:?}"
    );
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn any_other_key_keeps_the_package() {
    let mut app = board_with_package();
    cursor_on_package(&mut app, TaskStatus::Backlog);
    app.handle_key(make_key(KeyCode::Char('x')));
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('n'))));
    assert!(cmds.is_empty(), "{cmds:?}");
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn moving_a_package_card_does_nothing() {
    let mut app = board_with_package();
    cursor_on_package(&mut app, TaskStatus::Backlog);
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('L'))));
    assert!(cmds.is_empty(), "{cmds:?}");
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

#[test]
fn the_package_card_renders_its_title() {
    let mut app = board_with_package();
    let buf = render_to_buffer(&mut app, 160, 40);
    assert!(buffer_contains(&buf, "Window 1"));
}

#[test]
fn the_package_view_names_the_package_in_its_border() {
    let mut app = board_with_package();
    enter_package(&mut app, 7);
    let buf = render_to_buffer(&mut app, 160, 40);
    assert!(buffer_contains(&buf, "Window 1"));
}

// ---------------------------------------------------------------------------
// Snapshots (120×40, as in `snapshots.rs`)
// ---------------------------------------------------------------------------

fn render_to_string(app: &mut App) -> String {
    let buf = render_to_buffer(app, 120, 40);
    (0..buf.area().height)
        .map(|y| buffer_line(&buf, y).trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn snapshot_main_board_with_a_package_card() {
    let mut app = board_with_package();
    insta::assert_snapshot!(render_to_string(&mut app));
}

#[test]
fn snapshot_package_view() {
    let mut app = board_with_package();
    enter_package(&mut app, 7);
    insta::assert_snapshot!(render_to_string(&mut app));
}

#[test]
fn snapshot_link_picker() {
    let mut app = picker_board();
    app.handle_key(make_key(KeyCode::Char('w')));
    insta::assert_snapshot!(render_to_string(&mut app));
}

#[test]
fn the_status_bar_offers_the_package_card_actions() {
    let mut app = board_with_package();
    cursor_on_package(&mut app, TaskStatus::Backlog);
    let buf = render_to_buffer(&mut app, 160, 40);
    let last = buffer_line(&buf, 39);
    for hint in ["open", "rename", "delete"] {
        assert!(last.contains(hint), "{hint} missing from {last:?}");
    }
}

#[test]
fn a_refresh_keeps_an_open_pickers_cursor_on_a_real_row() {
    let mut app = picker_board();
    app.handle_key(make_key(KeyCode::Char('w')));
    app.handle_key(make_key(KeyCode::Char('j')));
    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(picker_cursor(&app), 2);
    app.update(Message::WorkPackage(WorkPackageMessage::Refresh {
        packages: vec![make_package(7, "Window 1")],
        links: vec![],
    }));
    assert_eq!(picker_cursor(&app), 1, "one package + the new-package row");
}

// `U` and `R` act on the epic the user is inside, under InsideEpicView. A
// package view is not an epic, so that context must not hold there (the keys
// would claim the press and silently do nothing while help listed them as
// live). Leaving the view is a separate context that does hold.
#[test]
fn the_epic_only_context_does_not_hold_in_a_package_view() {
    use crate::keybindings::KeyContext as C;
    let mut app = board_with_package();
    enter_package(&mut app, 7);
    assert!(!app.context_holds(C::InsideEpicView));
    assert!(app.context_holds(C::InsideNestedView));
    assert!(app.context_holds(C::NestedViewNoSearch));
}
