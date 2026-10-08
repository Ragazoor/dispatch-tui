use super::*;

// ---------------------------------------------------------------------------
// epic_repo_matches
// ---------------------------------------------------------------------------

#[test]
fn epic_repo_matches_no_filter_always_true() {
    let mut app = make_app();
    app.board.epics = vec![make_epic(1)];
    // No filter active → always true regardless of tasks
    assert!(app.view().epic_repo_matches(EpicId(1)));
}

#[test]
fn epic_repo_matches_empty_epic_with_active_filter_true() {
    let mut app = make_app();
    app.board.epics = vec![make_epic(1)];
    // Filter active but no tasks → empty epic is always shown
    app.filter.repos = std::collections::HashSet::from(["/other/repo".to_string()]);
    assert!(app.view().epic_repo_matches(EpicId(1)));
}

#[test]
fn epic_repo_matches_with_matching_task_true() {
    let mut app = make_app();
    app.board.epics = vec![make_epic(1)];
    let mut task = make_task(10, crate::models::TaskStatus::Backlog);
    task.epic_id = Some(EpicId(1));
    task.repo_path = "/my/repo".to_string();
    app.board.tasks = vec![task];
    app.filter.repos = std::collections::HashSet::from(["/my/repo".to_string()]);
    assert!(app.view().epic_repo_matches(EpicId(1)));
}

#[test]
fn epic_repo_matches_with_no_matching_task_false() {
    let mut app = make_app();
    app.board.epics = vec![make_epic(1)];
    let mut task = make_task(10, crate::models::TaskStatus::Backlog);
    task.epic_id = Some(EpicId(1));
    task.repo_path = "/other/repo".to_string();
    app.board.tasks = vec![task];
    app.filter.repos = std::collections::HashSet::from(["/my/repo".to_string()]);
    assert!(!app.view().epic_repo_matches(EpicId(1)));
}

#[test]
fn reparent_epic_input_mode_is_normal_initially() {
    let app = App::new(vec![]);
    assert_eq!(app.input.mode, InputMode::Normal);
    // Verify new variants can be constructed (compile-time check)
    let _a = InputMode::ReparentEpic(EpicId(1));
    let _b = InputMode::ConfirmReparentEpic {
        epic_id: EpicId(1),
        new_parent: None,
    };
    let _c = InputMode::ConfirmReparentEpic {
        epic_id: EpicId(1),
        new_parent: Some(EpicId(2)),
    };
}

#[test]
fn start_reparent_sets_mode_and_picker() {
    use crate::tui::messages::EpicMessage;
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10), make_epic(20)];
    // navigate cursor onto epic 10 (board view, column 0, row 0)
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);

    app.update(Message::Epic(EpicMessage::StartReparent(EpicId(10))));

    assert_eq!(app.input.mode, InputMode::ReparentEpic(EpicId(10)));
    assert!(app.interaction.reparent_picker.is_some());
    let picker = app.interaction.reparent_picker.as_ref().unwrap();
    assert_eq!(picker.epic_id, EpicId(10));
}

#[test]
fn reparent_navigate_does_not_panic() {
    use crate::tui::messages::EpicMessage;
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10), make_epic(20)];
    app.update(Message::Epic(EpicMessage::StartReparent(EpicId(10))));

    // Navigation should not panic even with an empty or default tree state
    app.update(Message::Epic(EpicMessage::ReparentNavigate(
        crate::tui::types::TreeNav::Down,
    )));
    app.update(Message::Epic(EpicMessage::ReparentNavigate(
        crate::tui::types::TreeNav::Up,
    )));
}

#[test]
fn reparent_confirm_with_no_parent_selected_transitions_to_confirm_mode() {
    use crate::tui::messages::EpicMessage;
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10), make_epic(20)];
    app.update(Message::Epic(EpicMessage::StartReparent(EpicId(10))));
    // select_first picks "— no parent —" (our sentinel)
    if let Some(picker) = &app.interaction.reparent_picker {
        picker.tree_state.borrow_mut().select_first();
    }

    app.update(Message::Epic(EpicMessage::ReparentConfirm));

    assert!(
        matches!(
            app.input.mode,
            InputMode::ConfirmReparentEpic {
                epic_id: EpicId(10),
                new_parent: None
            }
        ),
        "expected ConfirmReparentEpic, got {:?}",
        app.input.mode
    );
    assert!(app.status.message.is_some());
}

#[test]
fn reparent_execute_emits_reparent_command_and_resets_state() {
    use crate::tui::messages::EpicMessage;
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10), make_epic(20)];
    app.input.mode = InputMode::ConfirmReparentEpic {
        epic_id: EpicId(10),
        new_parent: Some(EpicId(20)),
    };
    app.interaction.reparent_picker = Some(make_reparent_picker(EpicId(10)));

    let cmds = app.update(Message::Epic(EpicMessage::ReparentExecute));

    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.interaction.reparent_picker.is_none());
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Epic(crate::tui::commands::EpicCommand::Reparent {
            id: EpicId(10),
            new_parent: Some(EpicId(20)),
        })
    )));
}

#[test]
fn reparent_cancel_from_confirm_returns_to_picker() {
    use crate::tui::messages::EpicMessage;
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    app.input.mode = InputMode::ConfirmReparentEpic {
        epic_id: EpicId(10),
        new_parent: None,
    };
    app.interaction.reparent_picker = Some(make_reparent_picker(EpicId(10)));

    app.update(Message::Epic(EpicMessage::ReparentCancel));

    assert_eq!(app.input.mode, InputMode::ReparentEpic(EpicId(10)));
    assert!(app.interaction.reparent_picker.is_some());
}

#[test]
fn reparent_cancel_from_picker_clears_state() {
    use crate::tui::messages::EpicMessage;
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    app.input.mode = InputMode::ReparentEpic(EpicId(10));
    app.interaction.reparent_picker = Some(make_reparent_picker(EpicId(10)));

    app.update(Message::Epic(EpicMessage::ReparentCancel));

    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.interaction.reparent_picker.is_none());
}

// ---------------------------------------------------------------------------
// Task 4: Key binding tests
// ---------------------------------------------------------------------------

#[test]
fn m_key_on_epic_card_opens_reparent_picker() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    app.selection_mut().set_column(1); // Backlog column (no standalone tasks, epic is at row 0)
    app.selection_mut().set_row(1, 0); // first row — the epic

    app.handle_key(make_key(KeyCode::Char('m')));

    assert_eq!(app.input.mode, InputMode::ReparentEpic(EpicId(10)));
    assert!(app.interaction.reparent_picker.is_some());
}

#[test]
fn m_key_on_task_opens_move_to_epic_picker() {
    // `m` on a task card opens the move-to-epic picker (it reparents an epic
    // when the cursor is on an epic card instead — see
    // `m_key_on_epic_card_opens_reparent_picker`).
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.epics = vec![make_epic(10)];
    // cursor on task (row 0), not epic (row 1)
    app.selection_mut().set_column(1); // backlog task column
    app.selection_mut().set_row(1, 0);

    app.handle_key(make_key(KeyCode::Char('m')));

    assert_eq!(app.input.mode, InputMode::MoveTaskToEpic(TaskId(1)));
    assert!(app.interaction.move_task_picker.is_some());
}

#[test]
fn reparent_mode_j_k_dispatch_navigate_messages() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10), make_epic(20)];
    app.input.mode = InputMode::ReparentEpic(EpicId(10));
    app.interaction.reparent_picker = Some(make_reparent_picker(EpicId(10)));

    // j and k should navigate, not crash
    app.handle_key(make_key(KeyCode::Char('j')));
    app.handle_key(make_key(KeyCode::Char('k')));
    app.handle_key(make_key(KeyCode::Down));
    app.handle_key(make_key(KeyCode::Up));
    // mode should remain ReparentEpic
    assert_eq!(app.input.mode, InputMode::ReparentEpic(EpicId(10)));
}

#[test]
fn reparent_mode_esc_cancels() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    app.input.mode = InputMode::ReparentEpic(EpicId(10));
    app.interaction.reparent_picker = Some(make_reparent_picker(EpicId(10)));

    app.handle_key(make_key(KeyCode::Esc));

    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.interaction.reparent_picker.is_none());
}

#[test]
fn confirm_reparent_y_executes() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10), make_epic(20)];
    app.input.mode = InputMode::ConfirmReparentEpic {
        epic_id: EpicId(10),
        new_parent: Some(EpicId(20)),
    };
    app.interaction.reparent_picker = Some(make_reparent_picker(EpicId(10)));

    let cmds = app.handle_key(make_key(KeyCode::Char('y')));

    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Epic(crate::tui::commands::EpicCommand::Reparent {
            id: EpicId(10),
            new_parent: Some(EpicId(20)),
        })
    )));
}

#[test]
fn confirm_reparent_n_returns_to_picker() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    app.input.mode = InputMode::ConfirmReparentEpic {
        epic_id: EpicId(10),
        new_parent: None,
    };
    app.interaction.reparent_picker = Some(make_reparent_picker(EpicId(10)));

    app.handle_key(make_key(KeyCode::Char('n')));

    assert_eq!(app.input.mode, InputMode::ReparentEpic(EpicId(10)));
}

#[test]
fn esc_from_confirm_reparent_cancels_entirely() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10), make_epic(20)];
    app.input.mode = InputMode::ConfirmReparentEpic {
        epic_id: EpicId(10),
        new_parent: Some(EpicId(20)),
    };
    app.interaction.reparent_picker = Some(make_reparent_picker(EpicId(10)));

    app.handle_key(make_key(KeyCode::Esc));

    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.interaction.reparent_picker.is_none());
}

#[test]
fn q_from_confirm_reparent_cancels_entirely() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10), make_epic(20)];
    app.input.mode = InputMode::ConfirmReparentEpic {
        epic_id: EpicId(10),
        new_parent: Some(EpicId(20)),
    };
    app.interaction.reparent_picker = Some(make_reparent_picker(EpicId(10)));

    app.handle_key(make_key(KeyCode::Char('q')));

    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(app.interaction.reparent_picker.is_none());
}

// ---------------------------------------------------------------------------
// reparent_target_epics — picker eligibility filtering (task #1595)
// ---------------------------------------------------------------------------

/// Helper: build an epic with a specific status and parent.
fn epic_with(id: i64, status: TaskStatus, parent: Option<i64>) -> crate::models::Epic {
    crate::models::Epic {
        status,
        parent_epic_id: parent.map(EpicId),
        ..make_epic(id)
    }
}

fn target_ids(app: &App, target: EpicId) -> Vec<i64> {
    app.view()
        .reparent_target_epics(target)
        .iter()
        .map(|e| e.id.0)
        .collect()
}

#[test]
fn reparent_target_epics_excludes_target_and_descendants() {
    let mut app = App::new(vec![]);
    // 10 (target) -> 11 (child) -> 12 (grandchild); 20 unrelated
    app.board.epics = vec![
        epic_with(10, TaskStatus::Backlog, None),
        epic_with(11, TaskStatus::Backlog, Some(10)),
        epic_with(12, TaskStatus::Backlog, Some(11)),
        epic_with(20, TaskStatus::Backlog, None),
    ];
    let ids = target_ids(&app, EpicId(10));
    assert_eq!(
        ids,
        vec![20],
        "target + descendants excluded, unrelated kept"
    );
}

#[test]
fn reparent_target_epics_excludes_done() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![
        epic_with(10, TaskStatus::Backlog, None), // target
        epic_with(20, TaskStatus::Done, None),
        epic_with(40, TaskStatus::Backlog, None),
    ];
    let ids = target_ids(&app, EpicId(10));
    assert_eq!(ids, vec![40], "a Done epic is not a reparent target");
}

#[test]
fn reparent_target_epics_excludes_repo_filtered_epic() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![
        epic_with(10, TaskStatus::Backlog, None), // target
        epic_with(20, TaskStatus::Backlog, None),
        epic_with(30, TaskStatus::Backlog, None),
    ];
    let mut task_b = make_task(1, TaskStatus::Backlog);
    task_b.epic_id = Some(EpicId(20));
    task_b.repo_path = "/repo-b".to_string();
    let mut task_c = make_task(2, TaskStatus::Backlog);
    task_c.epic_id = Some(EpicId(30));
    task_c.repo_path = "/repo-c".to_string();
    app.board.tasks = vec![task_b, task_c];
    app.filter.repos.insert("/repo-b".to_string());

    let ids = target_ids(&app, EpicId(10));
    assert_eq!(ids, vec![20], "epic whose tasks are filtered out is hidden");
}

#[test]
fn reparent_target_epics_excludes_only_active_filtered_epic() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![
        epic_with(10, TaskStatus::Backlog, None), // target
        epic_with(20, TaskStatus::Running, None),
        epic_with(30, TaskStatus::Running, None),
    ];
    let mut active = make_task(1, TaskStatus::Running);
    active.epic_id = Some(EpicId(20));
    active.tmux_window = Some(test_tmux_window("sess:1"));
    let mut inactive = make_task(2, TaskStatus::Running);
    inactive.epic_id = Some(EpicId(30));
    inactive.tmux_window = None; // no live session — what only_active filters on
    app.board.tasks = vec![active, inactive];
    app.filter.only_active = true;

    let ids = target_ids(&app, EpicId(10));
    assert_eq!(
        ids,
        vec![20],
        "only_active hides epic with no active descendant"
    );
}

#[test]
fn reparent_target_epics_keeps_eligible_epics() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![
        epic_with(10, TaskStatus::Backlog, None), // target
        epic_with(20, TaskStatus::Backlog, None),
        epic_with(30, TaskStatus::Running, None),
        epic_with(40, TaskStatus::Review, None),
    ];
    let ids = target_ids(&app, EpicId(10));
    assert_eq!(
        ids,
        vec![20, 30, 40],
        "Backlog/Running/Review epics are all eligible"
    );
}

// ---------------------------------------------------------------------------
// Picker item pre-computation
// ---------------------------------------------------------------------------

#[test]
fn reparent_picker_has_prebuilt_tree_items_on_open() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(1), make_epic(2)];

    app.update(Message::Epic(
        crate::tui::messages::EpicMessage::StartReparent(EpicId(1)),
    ));

    let picker = app
        .interaction
        .reparent_picker
        .as_ref()
        .expect("picker should be set");
    assert!(
        !picker.items.is_empty(),
        "picker items must be prebuilt when picker opens"
    );
    // First item is always the "— no parent —" sentinel
    assert_eq!(
        picker.items[0].identifier(),
        crate::tui::types::REPARENT_NO_PARENT_SENTINEL,
        "first item must be the no-parent sentinel"
    );
}

#[test]
fn move_task_picker_has_prebuilt_tree_items_on_open() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.epics = vec![make_epic(10)];
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    app.update(Message::Task(
        crate::tui::messages::TaskMessage::StartMoveToEpic(TaskId(1)),
    ));

    let picker = app
        .interaction
        .move_task_picker
        .as_ref()
        .expect("picker should be set");
    assert!(
        !picker.items.is_empty(),
        "move-task picker items must be prebuilt when picker opens"
    );
}

#[test]
fn flattened_board_surfaces_done_subtasks() {
    // Done flattens with Running and Review (task #4784). Only Backlog is
    // exempt, so an epic's done subtask surfaces as a card of its own.
    let mut app = App::new(vec![]);
    let standalone = make_task(1, TaskStatus::Done);
    let mut done_subtask = make_task(2, TaskStatus::Done);
    done_subtask.epic_id = Some(EpicId(10));
    let mut running_subtask = make_task(3, TaskStatus::Running);
    running_subtask.epic_id = Some(EpicId(10));
    app.board.tasks = vec![standalone, done_subtask, running_subtask];
    app.board.epics = vec![make_epic(10)];
    app.board.flattened = true;

    let ids = visible_task_ids(&app);
    assert!(
        ids.contains(&TaskId(1)),
        "standalone done task stays visible"
    );
    assert!(
        ids.contains(&TaskId(3)),
        "running subtask surfaces in flat mode"
    );
    assert!(
        ids.contains(&TaskId(2)),
        "done subtask surfaces too — done is no longer exempt from flattening"
    );
}

#[test]
fn flattened_board_drops_epic_cards_from_done() {
    let mut app = App::new(vec![]);
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Done;
    app.board.epics = vec![epic];
    let mut done_subtask = make_task(1, TaskStatus::Done);
    done_subtask.epic_id = Some(EpicId(10));
    app.board.tasks = vec![done_subtask];
    app.board.flattened = true;

    let items = app.view().column_items_for_status(TaskStatus::Done);
    assert!(
        !items
            .iter()
            .any(|i| matches!(i, ColumnItem::Epic(e) if e.id == EpicId(10))),
        "a flattened column draws no epic card"
    );
    assert!(
        items
            .iter()
            .any(|i| matches!(i, ColumnItem::Task(t) if t.id == TaskId(1))),
        "the done subtask surfaces in its place"
    );
}

#[test]
fn flattened_epic_view_done_shows_the_whole_subtree() {
    // In an epic view the Done column now flattens like Running and Review, so
    // it reaches past the epic's own direct tasks into its sub-epics. Backlog
    // is the one column that still stops at the direct children.
    let mut app = App::new(vec![]);
    let mut child_epic = make_epic(20);
    child_epic.parent_epic_id = Some(EpicId(10));
    app.board.epics = vec![make_epic(10), child_epic];

    let mut direct = make_task(1, TaskStatus::Done);
    direct.epic_id = Some(EpicId(10));
    let mut nested = make_task(2, TaskStatus::Done);
    nested.epic_id = Some(EpicId(20));
    let mut nested_running = make_task(3, TaskStatus::Running);
    nested_running.epic_id = Some(EpicId(20));
    app.board.tasks = vec![direct, nested, nested_running];
    app.board.flattened = true;
    app.handle_enter_epic(EpicId(10));

    let ids = visible_task_ids(&app);
    assert!(ids.contains(&TaskId(1)), "direct done child stays visible");
    assert!(
        ids.contains(&TaskId(2)),
        "nested done task surfaces — done flattens too"
    );
    assert!(
        ids.contains(&TaskId(3)),
        "nested running task still surfaces in flat mode"
    );
}
