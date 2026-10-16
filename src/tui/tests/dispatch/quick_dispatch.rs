use super::*;

#[test]
fn dispatch_only_backlog_tasks() {
    let mut app = make_app();

    // Task 1 is Backlog — should dispatch
    let cmds = app.update(Message::Task(TaskMessage::Dispatch(
        TaskId(1),
        DispatchMode::Dispatch,
    )));
    assert!(matches!(
        cmds[0],
        Command::Task(TaskCommand::DispatchAgent { .. })
    ));

    // Task 3 is Running — should not dispatch
    let cmds = app.update(Message::Task(TaskMessage::Dispatch(
        TaskId(3),
        DispatchMode::Dispatch,
    )));
    assert!(cmds.is_empty());

    // Task 4 is Done — should not dispatch
    let cmds = app.update(Message::Task(TaskMessage::Dispatch(
        TaskId(4),
        DispatchMode::Dispatch,
    )));
    assert!(cmds.is_empty());
}

#[test]
fn dispatch_from_running_is_noop() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)]);
    let cmds = app.update(Message::Task(TaskMessage::Dispatch(
        TaskId(4),
        DispatchMode::Dispatch,
    )));
    assert!(cmds.is_empty());
}

#[test]
fn dispatch_from_review_is_noop() {
    let mut app = App::new(vec![make_task(5, TaskStatus::Review)]);
    let cmds = app.update(Message::Task(TaskMessage::Dispatch(
        TaskId(5),
        DispatchMode::Dispatch,
    )));
    assert!(cmds.is_empty());
}

#[test]
fn shift_d_with_one_repo_emits_quick_dispatch() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.repo_paths = vec!["/repo".to_string()];
    let cmds = without_usage(app.handle_key(make_shift_key(KeyCode::Char('D'))));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::Task(TaskCommand::QuickDispatch { ref draft, epic_id: None }) if draft.repo_path == "/repo")
    );
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn shift_d_with_no_repos_opens_picker() {
    // With no saved repos, D should open the picker so the user can type a new
    // repo path rather than showing a "no saved paths" error.
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.repo_paths = vec![];
    let cmds = without_usage(app.handle_key(make_shift_key(KeyCode::Char('D'))));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::QuickDispatch);
}

#[test]
fn shift_d_with_multiple_repos_enters_quick_dispatch_mode() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.repo_paths = vec!["/repo1".to_string(), "/repo2".to_string()];
    let cmds = without_usage(app.handle_key(make_shift_key(KeyCode::Char('D'))));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::QuickDispatch);
}

#[test]
fn quick_dispatch_mode_typed_digit_filters_not_selects() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.repo_paths = vec!["/repo1".to_string(), "/repo2".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    let cmds = app.handle_key(make_key(KeyCode::Char('2')));
    assert!(cmds.is_empty(), "digit must not produce any command");
    assert_eq!(app.input.mode, InputMode::QuickDispatch);
    assert_eq!(app.input.buffer, "2");
}

#[test]
fn quick_dispatch_mode_invalid_number_appends_to_buffer() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.repo_paths = vec!["/repo1".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    let cmds = app.handle_key(make_key(KeyCode::Char('3')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::QuickDispatch);
    assert_eq!(app.input.buffer, "3");
}

#[test]
fn quick_dispatch_mode_esc_cancels() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.repo_paths = vec!["/repo1".to_string(), "/repo2".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Esc)));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn quick_dispatch_message_emits_command() {
    let mut app = App::new(vec![]);
    let cmds = app.update(Message::Task(TaskMessage::QuickDispatch {
        repo_path: "/my/repo".to_string(),
        epic_id: None,
    }));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::Task(TaskCommand::QuickDispatch { ref draft, epic_id: None })
        if draft.title == DEFAULT_QUICK_TASK_TITLE && draft.repo_path == "/my/repo")
    );
}

#[test]
fn shift_d_in_epic_view_quick_dispatches_subtask_single_repo() {
    let mut app = App::new(vec![]);
    let epic = make_epic(10);
    app.board.epics = vec![epic];
    app.board.repo_paths = vec!["/my/repo".to_string()];
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };
    let cmds = without_usage(app.handle_key(make_shift_key(KeyCode::Char('D'))));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0],
        Command::Task(TaskCommand::QuickDispatch { ref draft, epic_id: Some(EpicId(10)) })
        if draft.repo_path == "/my/repo"
    ));
}

#[test]
fn shift_d_in_epic_view_shows_repo_selection_with_multiple_repos() {
    let mut app = App::new(vec![]);
    let epic = make_epic(10);
    app.board.epics = vec![epic];
    app.board.repo_paths = vec!["/repo/a".to_string(), "/repo/b".to_string()];
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };
    let cmds = without_usage(app.handle_key(make_shift_key(KeyCode::Char('D'))));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::QuickDispatch);
    assert_eq!(app.input.pending_epic_id, Some(EpicId(10)));
}

#[test]
fn shift_d_in_epic_view_repo_selection_dispatches_with_epic_id() {
    let mut app = App::new(vec![]);
    let epic = make_epic(10);
    app.board.epics = vec![epic];
    app.board.repo_paths = vec!["/repo/a".to_string(), "/repo/b".to_string()];
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };
    // Enter selection mode
    app.handle_key(make_shift_key(KeyCode::Char('D')));
    // Move cursor to second repo, then Enter to select.
    app.handle_key(make_key(KeyCode::Down));
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Enter)));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0],
        Command::Task(TaskCommand::QuickDispatch { ref draft, epic_id: Some(EpicId(10)) })
        if draft.repo_path == "/repo/b"
    ));
}

#[test]
fn quick_dispatch_zero_is_noop() {
    let mut app = App::new(vec![]);
    app.board.repo_paths = vec!["/repo".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    let cmds = app.handle_key(make_key(KeyCode::Char('0')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::QuickDispatch);
}

#[test]
fn quick_dispatch_non_digit_is_noop() {
    let mut app = App::new(vec![]);
    app.board.repo_paths = vec!["/repo".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    let cmds = app.handle_key(make_key(KeyCode::Char('a')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::QuickDispatch);
}

#[test]
fn dispatch_is_noop_when_on_select_all() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('k')));
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('d'))));
    assert!(cmds.is_empty());
}

#[test]
fn quick_dispatch_down_arrow_moves_cursor_down() {
    let mut app = App::new(vec![]);
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string(), "/c".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    app.input.repo_cursor = 0;
    app.handle_key(make_key(KeyCode::Down));
    assert_eq!(app.input.repo_cursor, 1);
}

#[test]
fn quick_dispatch_j_typed_into_filter_buffer() {
    let mut app = App::new(vec![]);
    app.board.repo_paths = vec!["/jkl".to_string(), "/abc".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    app.input.repo_cursor = 0;
    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.input.buffer, "j");
    assert_eq!(app.input.repo_cursor, 0);
}

#[test]
fn quick_dispatch_enter_selects_cursor_repo() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.repo_paths = vec![
        "/repo1".to_string(),
        "/repo2".to_string(),
        "/repo3".to_string(),
    ];
    app.input.mode = InputMode::QuickDispatch;
    app.input.repo_cursor = 2; // third repo
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Enter)));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::Task(TaskCommand::QuickDispatch { ref draft, epic_id: None }) if draft.repo_path == "/repo3")
    );
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn quick_dispatch_clears_buffer_on_entry() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.repo_paths = vec!["/repo1".to_string(), "/repo2".to_string()];
    app.input.set_buffer("leftover".to_string());
    app.update(Message::Input(InputMessage::StartQuickDispatchSelection));
    assert_eq!(app.input.buffer, "");
    assert_eq!(app.input.mode, InputMode::QuickDispatch);
}

#[test]
fn quick_dispatch_typing_updates_buffer() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.repo_paths = vec!["/api-service".to_string(), "/frontend".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    app.handle_key(make_key(KeyCode::Char('a')));
    assert_eq!(app.input.buffer, "a");
}

#[test]
fn quick_dispatch_typing_resets_cursor() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.repo_paths = vec![
        "/api".to_string(),
        "/backend".to_string(),
        "/core".to_string(),
    ];
    app.input.mode = InputMode::QuickDispatch;
    app.input.repo_cursor = 2;
    app.handle_key(make_key(KeyCode::Char('a')));
    assert_eq!(app.input.repo_cursor, 0);
}

#[test]
fn quick_dispatch_backspace_shrinks_buffer_and_resets_cursor() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.repo_paths = vec!["/api".to_string(), "/backend".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    app.input.set_buffer("ap".to_string());
    app.input.repo_cursor = 1;
    app.handle_key(make_key(KeyCode::Backspace));
    assert_eq!(app.input.buffer, "a");
    assert_eq!(app.input.repo_cursor, 0);
}

#[test]
fn quick_dispatch_enter_selects_from_filtered_list() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.repo_paths = vec![
        "/api-service".to_string(),
        "/backend".to_string(),
        "/api-gateway".to_string(),
    ];
    app.input.mode = InputMode::QuickDispatch;
    app.input.set_buffer("api".to_string()); // matches index 0 and 2 of full list
    app.input.repo_cursor = 1; // second item in filtered list → "/api-gateway"
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Enter)));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::Task(TaskCommand::QuickDispatch { ref draft, .. }) if draft.repo_path == "/api-gateway")
    );
}

#[test]
fn quick_dispatch_enter_uses_buffer_as_new_repo_when_no_match() {
    // When the typed path matches no existing repos, Enter should dispatch
    // to the literal buffer value as a brand-new repo path.
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.repo_paths = vec!["/api-service".to_string(), "/backend".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    app.input
        .set_buffer("/home/user/brand-new-project".to_string());
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Enter)));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::Task(TaskCommand::QuickDispatch { ref draft, .. })
            if draft.repo_path == "/home/user/brand-new-project")
    );
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn quick_dispatch_enter_uses_buffer_when_cursor_on_new_entry() {
    // When the buffer fuzzy-matches an existing repo but the cursor is on the
    // trailing "new path" entry, Enter dispatches with the raw buffer value.
    // repos = ["/home/code/project-work"]; buffer = "/home/code/work"
    // filtered = ["/home/code/project-work"] (fuzzy match), new entry at idx 1
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.repo_paths = vec!["/home/code/project-work".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    app.input.set_buffer("/home/code/work".to_string());
    app.input.repo_cursor = 1; // cursor on new entry (past the 1 filtered result)
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Enter)));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::Task(TaskCommand::QuickDispatch { ref draft, .. })
            if draft.repo_path == "/home/code/work")
    );
}

#[test]
fn quick_dispatch_cursor_navigates_to_new_entry() {
    // Down arrow should be able to move the cursor past the filtered list to
    // the new-path entry when the buffer is non-empty and not an exact match.
    let mut app = App::new(vec![]);
    app.board.repo_paths = vec!["/home/code/project-work".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    app.input.set_buffer("/home/code/work".to_string()); // fuzzy-matches repo, but not exact
    app.input.repo_cursor = 0;
    app.handle_key(make_key(KeyCode::Down));
    // Should have moved to index 1 (the new-path entry)
    assert_eq!(app.input.repo_cursor, 1);
}

#[test]
fn quick_dispatch_no_new_entry_when_buffer_exactly_matches_repo() {
    // When the buffer is an exact match for an existing repo path, there
    // is no new-entry slot, so Down wraps within the filtered list.
    let mut app = App::new(vec![]);
    app.board.repo_paths = vec!["/repo/a".to_string(), "/repo/b".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    app.input.set_buffer("/repo/a".to_string()); // exact match → no new entry
                                                 // filtered = ["/repo/a"] only (only exact match on the query chars)
    app.input.repo_cursor = 0;
    app.handle_key(make_key(KeyCode::Down));
    // filtered has 1 item, no new entry → wraps back to 0
    assert_eq!(app.input.repo_cursor, 0);
}

#[test]
fn quick_dispatch_enter_uses_cursor_within_filtered_list() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.repo_paths = vec![
        "/api-service".to_string(),
        "/backend".to_string(),
        "/api-gateway".to_string(),
    ];
    app.input.mode = InputMode::QuickDispatch;
    app.input.set_buffer("api".to_string()); // filtered: ["/api-service", "/api-gateway"]
    app.input.repo_cursor = 1; // pick second filtered item → "/api-gateway"
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Enter)));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::Task(TaskCommand::QuickDispatch { ref draft, .. }) if draft.repo_path == "/api-gateway")
    );
}

#[test]
fn quick_dispatch_status_uses_freshly_created_title() {
    // Quick dispatch sends TaskCreated *then* MarkDispatching. The status
    // helper must look up the freshly-created task by ID — silently
    // reordering those two messages would silently break the title.
    let mut app = App::new(vec![]);
    let task = Task {
        title: "Quick task".to_string(),
        ..make_task(42, TaskStatus::Backlog)
    };

    app.update(Message::Task(TaskMessage::Created {
        task: Box::new(task),
    }));
    app.update(Message::Task(TaskMessage::MarkDispatching(TaskId(42))));

    let msg = app.status.message.as_deref().expect("status set");
    assert!(msg.contains("Quick task"), "got: {msg}");
    assert!(app.is_dispatching(TaskId(42)));
}

#[test]
fn d_key_on_backlog_is_inert() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.tag = Some(TaskTag::Feature);
    let mut app = App::new(vec![task]);
    app.selection_mut().set_column(1); // Backlog column
    app.selection_mut().set_row(1, 0);
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('d'))));
    // `d` is unbound: it dispatches nothing and opens no confirmation.
    assert!(cmds.is_empty(), "`d` should emit no commands, got {cmds:?}");
    assert!(
        !matches!(app.input.mode, InputMode::ConfirmTrustRepo { .. }),
        "`d` should not open the trust prompt, got {:?}",
        app.input.mode
    );
}
