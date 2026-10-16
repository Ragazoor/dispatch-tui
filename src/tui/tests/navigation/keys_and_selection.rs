use super::*;

#[test]
fn typing_appends_to_input_buffer() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputTitle;
    app.handle_key(make_key(KeyCode::Char('H')));
    app.handle_key(make_key(KeyCode::Char('i')));
    assert_eq!(app.input.buffer, "Hi");
}

#[test]
fn any_key_clears_error_popup() {
    let mut app = App::new(vec![]);
    app.status.error_popup = Some("boom".to_string());
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('a'))));
    assert!(app.status.error_popup.is_none());
    assert!(cmds.is_empty());
}

#[test]
fn error_popup_blocks_normal_key_handling() {
    let mut app = make_app();
    app.status.error_popup = Some("boom".to_string());
    app.handle_key(make_key(KeyCode::Char('q'))); // would normally quit
    assert!(app.status.error_popup.is_none());
    assert!(!app.should_quit); // quit was NOT processed
}

#[test]
fn resumed_sets_tmux_window() {
    let mut task = make_task(4, TaskStatus::Running);
    task.worktree = Some("/wt".to_string());
    let mut app = App::new(vec![task]);
    let cmds = app.update(Message::Task(crate::tui::messages::TaskMessage::Resumed {
        id: TaskId(4),
        tmux_window: test_tmux_window("win-4"),
    }));
    assert_eq!(
        app.board.tasks[0].tmux_window.as_ref().map(|w| w.as_str()),
        Some("win-4")
    );
    assert_eq!(cmds.len(), 2);
    assert!(matches!(
        &cmds[0],
        Command::Task(crate::tui::commands::TaskCommand::Persist(_))
    ));
    assert!(matches!(
        &cmds[1],
        Command::Task(crate::tui::commands::TaskCommand::SeedActivity { id, .. }) if *id == TaskId(4)
    ));
}

#[test]
fn resumed_unknown_id_is_noop() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)]);
    let cmds = app.update(Message::Task(crate::tui::messages::TaskMessage::Resumed {
        id: TaskId(999),
        tmux_window: test_tmux_window("win"),
    }));
    assert!(cmds.is_empty());
}

#[test]
fn resumed_sets_status_to_running() {
    let mut task = make_task(4, TaskStatus::Review);
    task.tmux_window = None;
    let mut app = App::new(vec![task]);

    let cmds = app.update(Message::Task(crate::tui::messages::TaskMessage::Resumed {
        id: TaskId(4),
        tmux_window: test_tmux_window("task-4"),
    }));

    let task = app.board.tasks.iter().find(|t| t.id == TaskId(4)).unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(
        task.tmux_window.as_ref().map(|w| w.as_str()),
        Some("task-4")
    );
    assert_eq!(cmds.len(), 2);
    assert!(
        matches!(&cmds[0], Command::Task(crate::tui::commands::TaskCommand::Persist(t)) if t.status == TaskStatus::Running)
    );
    assert!(matches!(
        &cmds[1],
        Command::Task(crate::tui::commands::TaskCommand::SeedActivity { .. })
    ));
}

#[test]
fn refresh_tasks_replaces_and_clamps() {
    let mut app = make_app();
    app.selection_mut().set_row(1, 1); // row 1 of Backlog (nav col 1, has 2 items)
    app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        vec![make_task(10, TaskStatus::Backlog)],
    )));
    assert_eq!(app.board.tasks.len(), 1);
    assert_eq!(app.board.tasks[0].id, TaskId(10));
    assert_eq!(app.selection().row(1), 0); // clamped from 1 to 0
}

#[test]
fn refresh_tasks_empty_clamps_all_rows_to_zero() {
    let mut app = make_app();
    app.selection_mut().set_row(0, 1);
    app.selection_mut().set_row(1, 1);
    app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        vec![],
    )));
    assert!(app.board.tasks.is_empty());
    assert!(app.selection().selected_row.iter().all(|&r| r == 0));
}

#[test]
fn dismiss_error_clears_popup() {
    let mut app = App::new(vec![]);
    app.status.error_popup = Some("boom".to_string());
    app.update(Message::System(
        crate::tui::messages::SystemMessage::DismissError,
    ));
    assert!(app.status.error_popup.is_none());
}

#[test]
fn input_char_appends_to_buffer() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputTitle;
    app.update(Message::Input(
        crate::tui::messages::InputMessage::InputChar('H'),
    ));
    app.update(Message::Input(
        crate::tui::messages::InputMessage::InputChar('i'),
    ));
    assert_eq!(app.input.buffer, "Hi");
}

#[test]
fn input_backspace_removes_last_char() {
    let mut app = App::new(vec![]);
    app.input.set_buffer("abc".to_string());
    app.update(Message::Input(
        crate::tui::messages::InputMessage::InputBackspace,
    ));
    assert_eq!(app.input.buffer, "ab");
}

#[test]
fn status_info_sets_message() {
    let mut app = App::new(vec![]);
    app.update(Message::System(
        crate::tui::messages::SystemMessage::StatusInfo("hello".to_string()),
    ));
    assert_eq!(app.status.message.as_deref(), Some("hello"));
}

#[test]
fn start_quick_dispatch_selection_enters_mode() {
    let mut app = App::new(vec![]);
    app.update(Message::Input(
        crate::tui::messages::InputMessage::StartQuickDispatchSelection,
    ));
    assert_eq!(app.input.mode, InputMode::QuickDispatch);
    assert!(app.status.message.is_some());
}

#[test]
fn select_quick_dispatch_repo_dispatches() {
    let mut app = App::new(vec![]);
    app.board.repo_paths = vec!["/repo1".to_string(), "/repo2".to_string()];
    let cmds = app.update(Message::Input(
        crate::tui::messages::InputMessage::SelectQuickDispatchRepo(1),
    ));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds.iter().any(
        |c| matches!(c, Command::Task(crate::tui::commands::TaskCommand::QuickDispatch { ref draft, .. }) if draft.repo_path == "/repo2")
    ));
}

#[test]
fn select_quick_dispatch_repo_out_of_range_is_noop() {
    let mut app = App::new(vec![]);
    app.board.repo_paths = vec!["/repo1".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    let cmds = app.update(Message::Input(
        crate::tui::messages::InputMessage::SelectQuickDispatchRepo(5),
    ));
    assert!(cmds.is_empty());
    // Mode is not changed by the handler (stays as-is)
}

#[test]
fn v_toggles_task_selection() {
    let mut app = make_app();
    // Select task 1 in Backlog
    app.handle_key(make_key(KeyCode::Char('v')));
    assert!(app.select.tasks.contains(&TaskId(1)));

    // Toggle off
    app.handle_key(make_key(KeyCode::Char('v')));
    assert!(!app.select.tasks.contains(&TaskId(1)));
}

#[test]
fn v_on_empty_column_is_noop() {
    let mut app = make_app();
    // Navigate to Review column (empty)
    app.update(Message::NavigateColumn(2));
    app.handle_key(make_key(KeyCode::Char('v')));
    assert!(app.select.tasks.is_empty());
}

#[test]
fn batch_move_forward_moves_all_selected() {
    let mut app = make_app();
    // Select both Backlog tasks
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleSelect(TaskId(1)),
    ));
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleSelect(TaskId(2)),
    ));

    // Press m to batch move forward
    let cmds = app.handle_key(make_key(KeyCode::Char('L')));

    // Both should now be Running
    assert_eq!(
        app.board.find_task(TaskId(1)).unwrap().status,
        TaskStatus::Running
    );
    assert_eq!(
        app.board.find_task(TaskId(2)).unwrap().status,
        TaskStatus::Running
    );
    // Should have PersistTask commands
    let persist_count = cmds
        .iter()
        .filter(|c| {
            matches!(
                c,
                Command::Task(crate::tui::commands::TaskCommand::Persist(_))
            )
        })
        .count();
    assert_eq!(persist_count, 2);
}

#[test]
fn batch_move_clears_selection() {
    let mut app = make_app();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleSelect(TaskId(1)),
    ));
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleSelect(TaskId(2)),
    ));

    app.handle_key(make_key(KeyCode::Char('L')));

    assert!(app.select.tasks.is_empty());
}

#[test]
fn batch_move_backward() {
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Done),
        make_task(2, TaskStatus::Done),
        make_task(3, TaskStatus::Done),
    ]);

    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleSelect(TaskId(1)),
    ));
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleSelect(TaskId(2)),
    ));

    app.handle_key(make_key(KeyCode::Char('H')));

    assert_eq!(
        app.board.find_task(TaskId(1)).unwrap().status,
        TaskStatus::Review
    );
    assert_eq!(
        app.board.find_task(TaskId(2)).unwrap().status,
        TaskStatus::Review
    );
    // Task 3 not selected, should remain Done
    assert_eq!(
        app.board.find_task(TaskId(3)).unwrap().status,
        TaskStatus::Done
    );
}

#[test]
fn single_task_operations_work_without_selection() {
    let mut app = make_app();
    assert!(app.select.tasks.is_empty());

    // Single move should still work
    let cmds = app.handle_key(make_key(KeyCode::Char('L')));
    assert_eq!(
        app.board.find_task(TaskId(1)).unwrap().status,
        TaskStatus::Running
    );
    assert!(!cmds.is_empty());
}

#[test]
fn refresh_tasks_prunes_stale_selections() {
    let mut app = make_app();
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleSelect(TaskId(1)),
    ));
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleSelect(TaskId(99)),
    )); // non-existent

    // Refresh with only task 1
    app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        vec![make_task(1, TaskStatus::Backlog)],
    )));

    assert!(app.select.tasks.contains(&TaskId(1)));
    assert!(!app.select.tasks.contains(&TaskId(99)));
}

#[test]
fn e_on_task_directly_edits() {
    let mut app = make_app();
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('e'))));
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Editor(crate::tui::commands::EditorCommand::PopOut(
            EditKind::TaskEdit(_)
        ))
    )));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn shift_v_key_on_task_is_noop() {
    let mut app = make_app();
    app.selection_mut().set_column(0);
    let cmds = app.handle_key(make_key(KeyCode::Char('V')));
    assert!(cmds.is_empty());
}

#[test]
fn shift_v_key_on_empty_column_is_noop() {
    let mut app = App::new(vec![]);
    app.selection_mut().set_column(0);
    let cmds = app.handle_key(make_key(KeyCode::Char('V')));
    assert!(cmds.is_empty());
}

#[test]
fn focus_changed_ignored_when_split_inactive() {
    let mut app = make_app();
    assert!(!app.split_active());

    let cmds = app.update(Message::System(
        crate::tui::messages::SystemMessage::FocusChanged(false),
    ));
    assert!(cmds.is_empty());
    assert!(app.split_focused()); // still true — ignored
}

#[test]
fn normal_mode_unrecognized_key_is_noop() {
    let mut app = make_app();
    // `Q` is deliberately unbound.
    let cmds = app.handle_key(make_key(KeyCode::Char('Q')));
    assert!(cmds.is_empty());
    assert!(!app.should_quit);
}

#[test]
fn text_input_unrecognized_key_is_noop() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputTitle;
    app.input.set_buffer("x".to_string());
    let cmds = app.handle_key(make_key(KeyCode::Tab));
    assert!(cmds.is_empty());
    assert_eq!(app.input.buffer, "x");
    assert_eq!(app.input.mode, InputMode::InputTitle);
}

#[test]
fn question_mark_toggles_help_mode() {
    let mut app = make_app();
    assert_eq!(app.input.mode, InputMode::Normal);

    app.handle_key(make_key(KeyCode::Char('?')));
    assert_eq!(app.input.mode, InputMode::Help);
}

#[test]
fn question_mark_dismisses_help() {
    let mut app = make_app();
    app.input.mode = InputMode::Help;

    app.handle_key(make_key(KeyCode::Char('?')));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn help_mode_ignores_other_keys() {
    let mut app = make_app();
    app.input.mode = InputMode::Help;

    app.handle_key(make_key(KeyCode::Char('q')));
    assert_eq!(app.input.mode, InputMode::Help);
    assert!(!app.should_quit);
}

#[test]
fn help_overlay_hidden_in_normal_mode() {
    let mut app = make_app();
    let buf = render_to_buffer(&mut app, 80, 30);
    assert!(!buffer_contains(&buf, "Navigation"));
}

#[test]
fn move_review_to_done_enters_confirm_mode() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Review)]);
    app.selection_mut().set_column(3); // Review = nav col 3

    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('L'))));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::ConfirmDone);
    assert_eq!(app.select.pending_done, vec![TaskId(1)]);
    assert!(app.status.message.as_deref().unwrap().contains("Done"));
}

#[test]
fn move_backlog_to_running_no_confirmation() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.selection_mut().set_column(1); // Backlog = nav col 1

    let cmds = app.handle_key(make_key(KeyCode::Char('L')));
    assert_eq!(app.input.mode, InputMode::Normal);
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(1)).unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Task(crate::tui::commands::TaskCommand::Persist(_))
    )));
}

#[test]
fn batch_move_mixed_statuses_moves_non_review_immediately() {
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Running),
        make_task(2, TaskStatus::Review),
    ]);
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleSelect(TaskId(1)),
    ));
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleSelect(TaskId(2)),
    ));

    let cmds = app.update(Message::Task(
        crate::tui::messages::TaskMessage::BatchMove {
            ids: vec![TaskId(1), TaskId(2)],
            direction: MoveDirection::Forward,
        },
    ));
    // Running→Review moved immediately
    let t1 = app.board.tasks.iter().find(|t| t.id == TaskId(1)).unwrap();
    assert_eq!(t1.status, TaskStatus::Review);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::Task(crate::tui::commands::TaskCommand::Persist(t)) if t.id == TaskId(1))));

    // Review→Done waiting for confirmation
    let t2 = app.board.tasks.iter().find(|t| t.id == TaskId(2)).unwrap();
    assert_eq!(t2.status, TaskStatus::Review); // not moved yet
    assert_eq!(app.input.mode, InputMode::ConfirmDone);
    assert_eq!(app.select.pending_done, vec![TaskId(2)]);
}

#[test]
fn key_a_toggles_off_when_all_selected() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('a')));
    assert_eq!(app.select.tasks.len(), 2);
    app.handle_key(make_key(KeyCode::Char('a')));
    assert!(app.select.tasks.is_empty());
}

#[test]
fn navigate_down_from_toggle_exits_to_row_zero() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('k')));
    assert!(app.on_select_all());
    app.handle_key(make_key(KeyCode::Char('j')));
    assert!(!app.on_select_all());
    assert_eq!(app.selected_row()[0], 0);
}

#[test]
fn column_scrolls_to_keep_cursor_visible() {
    // Create 20 backlog tasks — more than fit in a 20-row terminal
    let tasks: Vec<Task> = (1..=20)
        .map(|id| make_task(id, TaskStatus::Backlog))
        .collect();
    let mut app = App::new(tasks);

    // Navigate down to the last task (row 19, past visible area)
    for _ in 0..19 {
        app.update(Message::NavigateRow(1));
    }

    // Render in a small terminal (height 20 with header/detail/status ~10 lines
    // leaves roughly 10 lines for the column, fitting ~4-5 two-line task cards)
    let buf = render_to_buffer(&mut app, 120, 20);

    // The cursor should be on "Task 20" and it should be visible in the buffer
    assert!(
        buffer_contains(&buf, "Task 20"),
        "cursor task should be visible after scrolling down"
    );
}

#[test]
fn column_scrolls_back_up_when_cursor_moves_up() {
    let tasks: Vec<Task> = (1..=20)
        .map(|id| make_task(id, TaskStatus::Backlog))
        .collect();
    let mut app = App::new(tasks);

    // Navigate to the bottom
    for _ in 0..19 {
        app.update(Message::NavigateRow(1));
    }
    // Render once to establish scroll state
    let _ = render_to_buffer(&mut app, 120, 20);

    // Navigate back to the top
    for _ in 0..19 {
        app.update(Message::NavigateRow(-1));
    }
    let buf = render_to_buffer(&mut app, 120, 20);

    assert!(
        buffer_contains(&buf, "Task 1"),
        "first task should be visible after scrolling back up"
    );
}
