use super::*;

#[test]
fn move_epic_status_forward() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)]; // starts as Backlog
    let cmds = app.update(Message::Epic(
        crate::tui::messages::EpicMessage::MoveStatus(EpicId(10), MoveDirection::Forward),
    ));
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

#[test]
fn move_epic_status_backward() {
    let mut app = App::new(vec![]);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Done;
    app.board.epics = vec![epic];
    let cmds = app.update(Message::Epic(
        crate::tui::messages::EpicMessage::MoveStatus(EpicId(10), MoveDirection::Backward),
    ));
    assert_eq!(app.board.epics[0].status, TaskStatus::Review);
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Epic(crate::tui::commands::EpicCommand::Persist {
            id: EpicId(10),
            status: Some(TaskStatus::Review),
            ..
        })
    )));
}

#[test]
fn shift_l_key_on_epic_moves_status_forward() {
    let mut app = make_app_with_epic_selected();
    let cmds = app.handle_key(make_key(KeyCode::Char('L')));
    assert_eq!(app.board.epics[0].status, TaskStatus::Running);
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Epic(crate::tui::commands::EpicCommand::Persist { .. })
    )));
}

#[test]
fn shift_h_key_on_backlog_epic_stays_backlog() {
    let mut app = make_app_with_epic_selected();
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('H'))));
    // Already at Backlog, can't go backward
    assert_eq!(app.board.epics[0].status, TaskStatus::Backlog);
    assert!(cmds.is_empty());
}

#[test]
fn shift_h_on_done_epic_moves_to_review() {
    let mut app = App::new(vec![{
        let mut t = make_task(1, TaskStatus::Done);
        t.epic_id = Some(EpicId(10));
        t
    }]);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Done;
    app.board.epics = vec![epic];
    // Done epic → column 4
    app.selection_mut().set_column(4);
    app.selection_mut().set_row(4, 0);
    let cmds = app.handle_key(make_key(KeyCode::Char('H')));
    assert_eq!(app.board.epics[0].status, TaskStatus::Review);
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Epic(crate::tui::commands::EpicCommand::Persist {
            id: EpicId(10),
            status: Some(TaskStatus::Review),
            ..
        })
    )));
}

#[test]
fn shift_e_key_starts_new_epic() {
    let mut app = make_app();
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('E'))));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::InputEpicTitle);
}

#[test]
fn space_key_on_epic_from_board_enters_epic_view() {
    let mut app = make_app_with_epic_selected();
    app.handle_key(make_key(KeyCode::Char(' ')));
    assert!(matches!(
        app.board.view_mode,
        ViewMode::Epic {
            epic_id: EpicId(10),
            ..
        }
    ));
}

#[test]
fn e_key_in_epic_view_edits_epic() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('e'))));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::Editor(crate::tui::commands::EditorCommand::PopOut(EditKind::EpicEdit(e))) if e.id == EpicId(10))
    );
}

#[test]
fn e_key_on_task_in_epic_view_edits_task_not_epic() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    let mut subtask = make_task(1, TaskStatus::Backlog);
    subtask.epic_id = Some(EpicId(10));
    app.board.tasks = vec![subtask];
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(10),
    )));

    // Cursor on the subtask in the Backlog column (col 1, row 0)
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('e'))));
    assert_eq!(cmds.len(), 1, "expected exactly one command");
    assert!(
        matches!(&cmds[0], Command::Editor(crate::tui::commands::EditorCommand::PopOut(EditKind::TaskEdit(t))) if t.id == TaskId(1)),
        "expected PopOutEditor(TaskEdit(task 1), got {:?}",
        cmds
    );
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn esc_in_epic_view_exits_to_board() {
    let mut app = App::new(vec![]);
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };
    app.handle_key(make_key(KeyCode::Esc));
    assert!(matches!(app.board.view_mode, ViewMode::Board(_)));
}

#[test]
fn shift_u_in_epic_view_toggles_auto_dispatch() {
    let mut app = App::new(vec![]);
    let mut epic = make_epic(42);
    epic.auto_dispatch = true;
    app.board.epics = vec![epic];

    // Enter epic view
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(42),
    )));

    // Press Shift+U — should return ToggleEpicAutoDispatch command with auto_dispatch = false
    let cmds = app.handle_key(make_key(KeyCode::Char('U')));
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Epic(crate::tui::commands::EpicCommand::ToggleAutoDispatch {
            id: EpicId(42),
            auto_dispatch: false
        })
    )));

    // Also verify in-memory state was updated
    assert!(!app.board.epics[0].auto_dispatch);
}

#[test]
fn epic_title_esc_cancels() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputEpicTitle;
    app.input.set_buffer("partial".to_string());
    app.handle_key(make_key(KeyCode::Esc));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.input.buffer.is_empty());
}

#[test]
fn epic_title_enter_with_text_advances_to_description() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputEpicTitle;
    app.input.set_buffer("My Epic".to_string());
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::InputEpicDescription);
    assert!(app.input.buffer.is_empty());
    assert_eq!(app.input.epic_draft.as_ref().unwrap().title, "My Epic");
}

#[test]
fn epic_title_enter_empty_cancels() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputEpicTitle;
    app.input.buffer.clear();
    app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn epic_description_submit_completes_creation() {
    let mut app = make_app();
    app.input.mode = InputMode::InputEpicDescription;
    app.input.epic_draft = Some(EpicDraft {
        title: "My Epic".to_string(),
        description: String::new(),
        parent_epic_id: None,
    });
    let cmds = app.update(Message::Epic(
        crate::tui::messages::EpicMessage::SubmitDescription("Some description".to_string()),
    ));
    // Should immediately emit Insert command
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Epic(crate::tui::commands::EpicCommand::Insert(_))
    )));
    // Mode should be reset to Normal
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn epic_text_input_char_appends() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputEpicTitle;
    app.handle_key(make_key(KeyCode::Char('A')));
    app.handle_key(make_key(KeyCode::Char('b')));
    assert_eq!(app.input.buffer, "Ab");
}

#[test]
fn epic_text_input_backspace_removes() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputEpicTitle;
    app.input.set_buffer("abc".to_string());
    app.handle_key(make_key(KeyCode::Backspace));
    assert_eq!(app.input.buffer, "ab");
}

#[test]
fn epic_text_input_unrecognized_key_is_noop() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputEpicTitle;
    app.input.set_buffer("x".to_string());
    let cmds = app.handle_key(make_key(KeyCode::Tab));
    assert!(cmds.is_empty());
    assert_eq!(app.input.buffer, "x");
    assert_eq!(app.input.mode, InputMode::InputEpicTitle);
}

fn make_app_confirm_delete_epic() -> App {
    let mut app = make_app_with_epic_selected();
    app.input.mode = InputMode::ConfirmDeleteEpic;
    app.status.message = Some("Delete epic \"Epic 10\" and subtasks? [y/n]".to_string());
    app
}

#[test]
fn confirm_delete_epic_enters_mode_with_title() {
    let mut app = make_app_with_epic_selected();
    app.update(Message::Epic(
        crate::tui::messages::EpicMessage::ConfirmDelete,
    ));
    assert_eq!(app.input.mode, InputMode::ConfirmDeleteEpic);
    assert_eq!(
        app.status.message.as_deref(),
        Some("Delete epic \"Epic 10\" and subtasks? [y/n]")
    );
}

#[test]
fn confirm_delete_epic_y_deletes() {
    let mut app = make_app_confirm_delete_epic();
    let cmds = app.handle_key(make_key(KeyCode::Char('y')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.status.message.is_none());
    assert!(app.board.epics.is_empty());
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::Epic(crate::tui::commands::EpicCommand::Delete(id)) if *id == EpicId(10))));
}

#[test]
fn confirm_delete_epic_uppercase_y_deletes() {
    let mut app = make_app_confirm_delete_epic();
    let cmds = app.handle_key(make_key(KeyCode::Char('Y')));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.board.epics.is_empty());
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::Epic(crate::tui::commands::EpicCommand::Delete(id)) if *id == EpicId(10))));
}

#[test]
fn confirm_delete_epic_other_key_cancels() {
    let mut app = make_app_confirm_delete_epic();
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('n'))));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.status.message.is_none());
    assert_eq!(app.board.epics.len(), 1); // not deleted
    assert!(cmds.is_empty());
}

#[test]
fn confirm_delete_epic_no_epic_selected_is_noop() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.selection_mut().set_column(1); // cursor on task, not epic
    app.input.mode = InputMode::ConfirmDeleteEpic;
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('y'))));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds.is_empty()); // no deletion happened
}
