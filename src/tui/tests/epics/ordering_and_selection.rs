use super::*;

#[test]
fn space_key_on_epic_enters_epic_view() {
    let mut app = App::new(vec![]);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Review;
    app.board.epics = vec![epic];

    // Even with subtasks that have tmux windows, space enters epic view
    let mut subtask = make_task(1, TaskStatus::Review);
    subtask.epic_id = Some(EpicId(10));
    subtask.tmux_window = Some(test_tmux_window("win-1"));
    app.board.tasks = vec![subtask];

    app.selection_mut().set_column(3);
    app.selection_mut().set_row(3, 0);

    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(matches!(app.board.view_mode, ViewMode::Epic { epic_id, .. } if epic_id == EpicId(10)));
}

#[test]
fn shift_g_on_single_item_column_with_epic_is_noop() {
    // Only one selectable item (the epic itself) in the column: G jumping to
    // the "last" row lands on the same row it started on, and must never
    // enter epic view (that's still `space`'s job).
    let mut app = App::new(vec![]);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Running;
    app.board.epics = vec![epic];

    let mut subtask = make_task(1, TaskStatus::Running);
    subtask.epic_id = Some(EpicId(10));
    subtask.sub_status = SubStatus::NeedsInput;
    subtask.tmux_window = Some(test_tmux_window("win-1"));
    app.board.tasks = vec![subtask];

    app.selection_mut().set_column(2);
    app.selection_mut().set_row(2, 0);

    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('G'))));
    assert!(
        cmds.is_empty(),
        "G on a single-item column emits no commands"
    );
    assert!(
        !matches!(app.board.view_mode, ViewMode::Epic { .. }),
        "G on an epic must not enter epic view"
    );
    assert_eq!(app.selection().row(2), 0);
}

#[test]
fn shift_g_jumps_to_last_row_in_column_with_an_epic() {
    // A column with an epic plus a plain task: G should jump to the last
    // selectable row (the same NavigateRowLast logic as `]`), not enter
    // epic view.
    // Task 3 is the epic's, so the epic holds a card in Running; it renders
    // inside that card rather than as a third row.
    let mut owned = make_task(3, TaskStatus::Running);
    owned.epic_id = Some(EpicId(10));
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Running),
        make_task(2, TaskStatus::Running),
        owned,
    ]);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Running;
    app.board.epics = vec![epic];

    app.selection_mut().set_column(2);
    app.selection_mut().set_row(2, 0);

    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('G'))));
    assert!(cmds.is_empty());
    assert!(
        !matches!(app.board.view_mode, ViewMode::Epic { .. }),
        "G on an epic-containing column must not enter epic view"
    );
    assert_eq!(
        app.selection().row(2),
        2,
        "G should jump to the last row (epic + 2 tasks)"
    );
}

#[test]
fn column_items_sorted_by_sort_order() {
    let mut app = make_app();
    let mut t1 = make_task(1, TaskStatus::Backlog);
    t1.title = "First".to_string();
    t1.sort_order = Some(200);
    let mut t2 = make_task(2, TaskStatus::Backlog);
    t2.title = "Second".to_string();
    t2.sort_order = Some(100);
    app.board.tasks = vec![t1, t2];

    let items = app.view().column_items_for_status(TaskStatus::Backlog);
    assert_eq!(items.len(), 2);
    match &items[0] {
        ColumnItem::Task(t) => assert_eq!(t.title, "Second"),
        _ => panic!("expected task"),
    }
    match &items[1] {
        ColumnItem::Task(t) => assert_eq!(t.title, "First"),
        _ => panic!("expected task"),
    }
}

#[test]
fn column_items_null_sort_order_uses_id() {
    let mut app = make_app();
    let mut t1 = make_task(10, TaskStatus::Backlog);
    t1.title = "High ID".to_string();
    t1.sort_order = None;
    let mut t2 = make_task(5, TaskStatus::Backlog);
    t2.title = "Low ID".to_string();
    t2.sort_order = None;
    app.board.tasks = vec![t1, t2];

    let items = app.view().column_items_for_status(TaskStatus::Backlog);
    match &items[0] {
        ColumnItem::Task(t) => assert_eq!(t.title, "Low ID"),
        _ => panic!("expected task"),
    }
}

#[test]
fn done_column_sorts_by_completion_recency_via_completed_at() {
    let mut app = make_app();
    // completed_at as the service layer stamps it. The column reads this
    // descending, so the later timestamp renders first.
    let mut older = make_task(1, TaskStatus::Done);
    older.title = "Completed first".to_string();
    older.completed_at = chrono::DateTime::from_timestamp(1_700_000_000, 0);
    let mut newer = make_task(2, TaskStatus::Done);
    newer.title = "Completed second".to_string();
    newer.completed_at = chrono::DateTime::from_timestamp(1_700_000_100, 0);
    app.board.tasks = vec![older, newer];

    let items = app.view().column_items_for_status(TaskStatus::Done);
    assert_eq!(items.len(), 2);
    match &items[0] {
        ColumnItem::Task(t) => assert_eq!(
            t.title, "Completed second",
            "the more recently completed task must render first"
        ),
        _ => panic!("expected task"),
    }
    match &items[1] {
        ColumnItem::Task(t) => assert_eq!(t.title, "Completed first"),
        _ => panic!("expected task"),
    }
}

#[test]
fn handle_key_normal_new_epic() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('E')));
    assert_eq!(*app.mode(), InputMode::InputEpicTitle);
}

#[test]
fn handle_key_normal_tab_is_noop_without_feed_epics() {
    // Tab from Board is a no-op when there are no feed epics.
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Tab));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn handle_key_epic_text_input_char_and_enter() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('E'))); // start epic creation
    assert_eq!(*app.mode(), InputMode::InputEpicTitle);

    app.handle_key(make_key(KeyCode::Char('X')));
    assert_eq!(app.input.buffer, "X");

    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(*app.mode(), InputMode::InputEpicDescription);
}

#[test]
fn handle_key_epic_text_input_esc_cancels() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicTitle;
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(*app.mode(), InputMode::Normal);
}

#[test]
fn v_toggles_epic_selection() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    // Epic is at row 0 in Backlog column (no standalone tasks)
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    app.handle_key(make_key(KeyCode::Char('v')));
    assert!(app.select.epics.contains(&EpicId(10)));
}

#[test]
fn v_on_epic_toggle_off() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    // Select
    app.handle_key(make_key(KeyCode::Char('v')));
    assert!(app.select.epics.contains(&EpicId(10)));

    // Deselect
    app.handle_key(make_key(KeyCode::Char('v')));
    assert!(!app.select.epics.contains(&EpicId(10)));
}

#[test]
fn v_on_empty_column_no_epics_is_noop() {
    let mut app = App::new(vec![]);
    // Navigate to Review column (empty)
    app.update(Message::NavigateColumn(2));
    app.handle_key(make_key(KeyCode::Char('v')));
    assert!(app.select.epics.is_empty());
    assert!(app.select.tasks.is_empty());
}

#[test]
fn select_all_column_includes_epics() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.epics = vec![make_epic(10)];

    app.update(Message::SelectAllColumn);
    assert!(app.select.tasks.contains(&TaskId(1)));
    assert!(app.select.epics.contains(&EpicId(10)));
}

#[test]
fn select_all_deselects_all_including_epics() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.epics = vec![make_epic(10)];

    // Select all
    app.update(Message::SelectAllColumn);
    assert_eq!(app.select.tasks.len(), 1);
    assert_eq!(app.select.epics.len(), 1);

    // Deselect all
    app.update(Message::SelectAllColumn);
    assert!(app.select.tasks.is_empty());
    assert!(app.select.epics.is_empty());
}

#[test]
fn select_all_column_with_only_epics() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10), make_epic(20)];

    app.update(Message::SelectAllColumn);
    assert!(app.select.tasks.is_empty());
    assert_eq!(app.select.epics.len(), 2);
    assert!(app.select.epics.contains(&EpicId(10)));
    assert!(app.select.epics.contains(&EpicId(20)));
}

#[test]
fn esc_clears_epic_selection() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::Epic(
        crate::tui::messages::EpicMessage::ToggleSelect(EpicId(10)),
    ));
    assert_eq!(app.select.epics.len(), 1);

    app.handle_key(make_key(KeyCode::Esc));
    assert!(app.select.epics.is_empty());
}

#[test]
fn x_key_with_epic_selection_shows_count_in_confirm() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10), make_epic(20)];
    app.update(Message::Epic(
        crate::tui::messages::EpicMessage::ToggleSelect(EpicId(10)),
    ));
    app.update(Message::Epic(
        crate::tui::messages::EpicMessage::ToggleSelect(EpicId(20)),
    ));

    app.handle_key(make_key(KeyCode::Char('x')));
    assert!(matches!(app.input.mode, InputMode::ConfirmBatchDelete));
    assert_eq!(app.status.message.as_deref(), Some("Delete 2 items? [y/n]"));
}

#[test]
fn shift_l_on_epic_moves_status_forward() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    // Cursor on Backlog column, row 0 (the epic)
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    let cmds = app.handle_key(make_key(KeyCode::Char('L')));
    assert_eq!(app.board.epics[0].status, TaskStatus::Running);
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Epic(crate::tui::commands::EpicCommand::Persist {
            id: EpicId(10),
            status: Some(TaskStatus::Running),
            ..
        })
    )));
}
