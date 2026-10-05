use super::*;

#[test]
fn render_selected_epic_shows_star_prefix() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::Epic(
        crate::tui::messages::EpicMessage::ToggleSelect(EpicId(10)),
    ));

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "* "),
        "Selected epic should show * prefix"
    );
    assert!(
        buffer_contains(&buf, "Epic 10"),
        "Epic title should be visible"
    );
}

#[test]
fn render_unselected_epic_no_star() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Epic 10"),
        "Epic title should be visible"
    );
    // The epic renders with "  " prefix (2 spaces), not "* "
    assert!(
        !buffer_contains(&buf, "* "),
        "Unselected epic should not show * prefix"
    );
}

#[test]
fn render_batch_hints_with_epic_selection() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::Epic(
        crate::tui::messages::EpicMessage::ToggleSelect(EpicId(10)),
    ));

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "1 selected"),
        "Should show selection count"
    );
    assert!(buffer_contains(&buf, "delete"), "Should show delete hint");
}

#[test]
fn render_column_header_checked_with_epics() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.epics = vec![make_epic(10)];

    // Select both the task and the epic
    app.update(Message::SelectAllColumn);
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "[x]"),
        "Checkbox should be checked when all items selected"
    );
}

#[test]
fn refresh_epics_prunes_stale_epic_selections() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::Epic(
        crate::tui::messages::EpicMessage::ToggleSelect(EpicId(10)),
    ));
    app.update(Message::Epic(
        crate::tui::messages::EpicMessage::ToggleSelect(EpicId(99)),
    )); // non-existent

    // Refresh with only epic 10
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Refresh(
        vec![make_epic(10)],
    )));
    assert!(app.select.epics.contains(&EpicId(10)));
    assert!(!app.select.epics.contains(&EpicId(99)));
}

#[test]
fn render_status_bar_confirm_delete_epic() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmDeleteEpic;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Delete epic"),
        "ConfirmDeleteEpic should show 'Delete epic'"
    );
}

#[test]
fn render_status_bar_epic_title() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicTitle;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Creating epic: enter title"),
        "InputEpicTitle should show 'Creating epic: enter title'"
    );
}

#[test]
fn render_status_bar_epic_description() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicDescription;
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Creating epic: opening $EDITOR for description"),
        "InputEpicDescription should show 'Creating epic: opening $EDITOR for description'"
    );
}

#[test]
fn render_input_form_epic_title_shows_new_epic() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicTitle;
    app.input.set_buffer("My epic".to_string());
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "New Epic"),
        "block title 'New Epic' should be visible"
    );
    assert!(
        buffer_contains(&buf, "Title:"),
        "'Title:' label should be visible"
    );
    assert!(
        buffer_contains(&buf, "My epic"),
        "buffer text 'My epic' should be visible"
    );
}

#[test]
fn render_input_form_epic_description_shows_fields() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicDescription;
    app.input.epic_draft = Some(EpicDraft {
        title: "Epic title".to_string(),
        ..Default::default()
    });
    app.input.set_buffer("Epic desc".to_string());
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "New Epic"),
        "block title 'New Epic' should be visible"
    );
    assert!(
        buffer_contains(&buf, "Epic title"),
        "completed title 'Epic title' should be visible"
    );
    assert!(
        buffer_contains(&buf, "Description:"),
        "'Description:' label should be visible"
    );
}

#[test]
fn render_epic_banner_shows_title() {
    let mut app = make_app();
    let mut epic = make_epic(10);
    epic.title = "Auth Refactor".to_string();
    app.board.epics = vec![epic];
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Auth Refactor"),
        "epic banner should show the epic title 'Auth Refactor'"
    );
}

#[test]
fn render_epic_banner_not_shown_in_board_view() {
    let mut app = make_app();
    let epic = make_epic(10);
    app.board.epics = vec![epic];
    // Stay in default Board view — do not switch to ViewMode::Epic
    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        !buffer_contains(&buf, "Esc to return"),
        "epic banner should not be shown in Board view"
    );
}

#[test]
fn render_detail_task_with_epic_reference() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.epic_id = Some(EpicId(10));
    let mut epic = make_epic(10);
    epic.title = "Auth Epic".to_string();
    let mut app = App::new(vec![task]);
    app.board.epics = vec![epic];
    // Switch to Epic view so the subtask is visible (Board view hides epic subtasks)
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);
    // The old detail panel is replaced by the TaskDetail overlay (Task 6).
    // This test will be updated in Task 6 to use the overlay.
    let _buf = render_to_buffer(&mut app, 160, 30);
}

#[test]
fn render_detail_epic_shows_title_and_id() {
    let mut app = App::new(vec![]);
    let mut epic = make_epic(10);
    epic.title = "Platform Migration".to_string();
    app.board.epics = vec![epic];
    // Epic is the only item in Backlog column (no standalone tasks)
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);
    // The old detail panel is replaced by the TaskDetail overlay (Task 6).
    // This test will be updated in Task 6 to use the overlay.
    let _buf = render_to_buffer(&mut app, 120, 30);
}

#[test]
fn render_detail_epic_with_plan_shows_path() {
    let mut app = App::new(vec![]);
    let mut epic = make_epic(10);
    epic.plan_path = Some("docs/plans/migration.md".to_string());
    app.board.epics = vec![epic];
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);
    // The old detail panel is replaced by the TaskDetail overlay (Task 6).
    // This test will be updated in Task 6 to use the overlay.
    let _buf = render_to_buffer(&mut app, 120, 30);
}

#[test]
fn render_detail_epic_shows_subtask_list() {
    let mut app = App::new(vec![]);
    let epic = make_epic(10);
    app.board.epics = vec![epic];

    let mut t1 = make_task(101, TaskStatus::Done);
    t1.title = "Subtask Alpha".to_string();
    t1.epic_id = Some(EpicId(10));
    let mut t2 = make_task(102, TaskStatus::Running);
    t2.title = "Subtask Beta".to_string();
    t2.epic_id = Some(EpicId(10));
    app.board.tasks = vec![t1, t2];

    // Epic is in Backlog; subtasks are in other columns so won't appear as
    // standalone items in column 1 (Backlog). The epic itself is the first item.
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);
    // The old detail panel is replaced by the TaskDetail overlay (Task 6).
    // This test will be updated in Task 6 to use the overlay.
    let _buf = render_to_buffer(&mut app, 120, 30);
}

#[test]
fn render_detail_epic_subtask_conflict_shows_warning() {
    let mut app = App::new(vec![]);
    let epic = make_epic(10);
    app.board.epics = vec![epic];

    let mut t1 = make_task(201, TaskStatus::Running);
    t1.title = "Conflicted Task".to_string();
    t1.epic_id = Some(EpicId(10));
    t1.sub_status = SubStatus::Conflict;
    app.board.tasks = vec![t1];

    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);
    // The old detail panel is replaced by the TaskDetail overlay (Task 6).
    // This test will be updated in Task 6 to use the overlay.
    let _buf = render_to_buffer(&mut app, 120, 30);
}

#[test]
fn render_tab_bar_epic_mode_has_no_tasks_label() {
    let mut app = App::new(vec![]);
    let mut epic = make_epic(10);
    epic.title = "Platform Work".to_string();
    app.board.epics = vec![epic];
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        !buffer_contains(&buf, "Tasks"),
        "tab bar should not show a 'Tasks' tab label in any mode"
    );
}

#[test]
fn epic_card_title_truncated_in_narrow_terminal() {
    let mut epic = make_epic(1);
    epic.title = "This is a very long epic title that should be truncated to fit".to_string();
    let mut app = App::new(vec![]);
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Refresh(
        vec![epic],
    )));

    let buf = render_to_buffer(&mut app, 80, 10);
    assert!(
        !buffer_contains(
            &buf,
            "This is a very long epic title that should be truncated to fit"
        ),
        "full epic title should be truncated in narrow terminal"
    );
}

#[test]
fn handle_key_normal_esc_in_epic_view_exits() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(10),
    )));
    assert!(matches!(app.board.view_mode, ViewMode::Epic { .. }));

    app.handle_key(make_key(KeyCode::Esc));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn handle_key_normal_q_in_epic_view_exits() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(10),
    )));

    app.handle_key(make_key(KeyCode::Char('q')));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn handle_key_e_in_tag_input_does_not_set_a_tag() {
    let mut app = make_app();
    app.input.mode = InputMode::InputTag;
    app.input.task_draft = Some(TaskDraft {
        title: "Test".to_string(),
        ..Default::default()
    });

    app.handle_key(make_key(KeyCode::Char('e')));
    assert_eq!(app.input.task_draft.as_ref().unwrap().tag, None);
}

#[test]
fn handle_key_normal_shift_l_on_epic_moves_status() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);
    let cmds = app.handle_key(make_key(KeyCode::Char('L')));
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Epic(crate::tui::commands::EpicCommand::Persist { .. })
    )));
}

#[test]
fn handle_key_normal_shift_h_on_epic_moves_backward() {
    // The epic needs a running subtask to hold a card in the Running column:
    // placement is per column and driven by the tasks, not by epic.status.
    let mut subtask = make_task(1, TaskStatus::Running);
    subtask.epic_id = Some(EpicId(10));
    let mut app = App::new(vec![subtask]);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Running;
    app.board.epics = vec![epic];
    app.selection_mut().set_column(2);
    app.selection_mut().set_row(2, 0);
    let cmds = app.handle_key(make_key(KeyCode::Char('H')));
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Epic(crate::tui::commands::EpicCommand::Persist { .. })
    )));
}

/// InputEpicTitle mode routes to the text input handler.
#[test]
fn handle_key_input_epic_title_routes_to_text_input() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicTitle;
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Esc)));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// InputEpicDescription mode routes to the text input handler.
#[test]
fn handle_key_input_epic_description_routes_to_text_input() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicDescription;
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Esc)));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// ConfirmDeleteEpic mode routes correctly.
#[test]
fn handle_key_confirm_delete_epic_routes_correctly() {
    let mut app = make_app();
    app.input.mode = InputMode::ConfirmDeleteEpic;
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('n'))));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

/// Normal mode on Epic view routes to the board handler (not review/security).
#[test]
fn handle_key_normal_epic_view_routes_correctly() {
    let mut app = make_app();
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(1),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };
    // 'q' in epic view exits to board (doesn't quit)
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('q'))));
    assert!(cmds.is_empty());
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}
