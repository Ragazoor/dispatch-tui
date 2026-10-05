use super::*;

#[test]
fn epic_edited_splices_all_fields_including_feed_settings() {
    // Regression guard: handle_epic_edited previously only copied
    // title/description/updated_at into the in-memory board row, dropping
    // feed_command/feed_interval_secs until the next DB refresh (~10s). In
    // that window the 'r' feed-refresh key checks the stale in-memory value.
    let mut app = App::new(vec![]);
    app.board.epics.push(make_epic(1));

    let mut edited = make_epic(1);
    edited.title = "New Title".to_string();
    edited.description = "New description".to_string();
    edited.feed_command = Some("gh api repos/x/pulls".to_string());
    edited.feed_interval_secs = Some(300);

    app.update(Message::Epic(crate::tui::messages::EpicMessage::Edited(
        edited.clone(),
    )));

    let spliced = app.board.epics.iter().find(|e| e.id == EpicId(1)).unwrap();
    assert_eq!(spliced.feed_command, edited.feed_command);
    assert_eq!(spliced.feed_interval_secs, edited.feed_interval_secs);
    assert_eq!(spliced.title, "New Title");
    assert_eq!(spliced.description, "New description");
}

#[test]
fn toggle_flattened_message_flips_state() {
    let mut app = App::new(vec![]);
    assert!(!app.board.flattened);
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleFlattened,
    ));
    assert!(app.board.flattened);
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleFlattened,
    ));
    assert!(!app.board.flattened);
}

#[test]
fn epic_action_hints_not_done() {
    let epic = make_epic(1);
    let hints = ui::epic_action_hints(&epic, Color::Rgb(122, 162, 247));
    let keys: Vec<&str> = hints
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
        .map(|s| s.content.as_ref())
        .collect();
    assert!(keys.contains(&"[Enter]"), "epic shows detail");
    assert!(keys.contains(&"[L]"), "epic shows status forward");
    assert!(keys.contains(&"[H]"), "epic shows status backward");
    assert!(keys.contains(&"[x]"), "epic shows delete");
}

#[test]
fn epic_action_hints_done() {
    let mut epic = make_epic(1);
    epic.status = TaskStatus::Done;
    let hints = ui::epic_action_hints(&epic, Color::Rgb(122, 162, 247));
    let keys: Vec<&str> = hints
        .iter()
        .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
        .map(|s| s.content.as_ref())
        .collect();
    assert!(keys.contains(&"[L]"), "done epic shows status forward");
    assert!(keys.contains(&"[H]"), "done epic shows status backward");
}

#[test]
fn action_hints_no_ctrl_g_outside_epic() {
    let task = make_task(1, TaskStatus::Backlog);
    let hints = ui::action_hints(Some(&task), false, Color::Rgb(122, 162, 247));
    let keys = hint_keys(&hints);
    assert!(
        !keys.contains(&"[^g]"),
        "should not show ^g back outside epic view"
    );
}

/// `Space` on an epic card enters the epic (`EpicMessage::Enter` in
/// `run_activation`) — it does not go back to the board.
#[test]
fn epic_action_hints_labels_space_as_enter() {
    let epic = make_epic(1);
    let hints = ui::epic_action_hints(&epic, Color::Rgb(122, 162, 247));
    let text: String = hints.iter().map(|s| s.content.as_ref()).collect();
    assert!(
        text.contains("[Space] enter"),
        "Space should be labelled 'enter', got {text:?}"
    );
    assert!(
        !text.contains("board"),
        "Space must not be labelled 'board', got {text:?}"
    );
}

/// `U` requires `current_epic_id()`, i.e. `ViewMode::Epic` — on a board epic card
/// it is a dead key, so the footer must not advertise it.
#[test]
fn epic_action_hints_omits_auto_dispatch() {
    let epic = make_epic(1);
    let hints = ui::epic_action_hints(&epic, Color::Rgb(122, 162, 247));
    let keys = hint_keys(&hints);
    assert!(
        !keys.contains(&"[U]"),
        "epic card must not advertise the inert [U] key, got {keys:?}"
    );
    let text: String = hints.iter().map(|s| s.content.as_ref()).collect();
    assert!(
        !text.contains("auto dispatch"),
        "epic card must not advertise auto dispatch, got {text:?}"
    );
}

/// The rendered footer, not just the helper: a selected epic card on the board
/// shows `[Space] enter` and no `[U]` hint.
#[test]
fn board_footer_for_selected_epic_card() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(1)];
    app.selection_mut().set_column(1); // Backlog column, epic card selected

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "[Space] enter"),
        "footer should label Space as 'enter' for a selected epic card"
    );
    assert!(
        !buffer_contains(&buf, "[U] auto dispatch"),
        "footer should not advertise [U] for a selected epic card"
    );
}

#[test]
fn epic_action_hints_shows_filter_help() {
    let epic = make_epic(1);
    let hints = ui::epic_action_hints(&epic, Color::Rgb(122, 162, 247));
    let keys = hint_keys(&hints);
    assert!(keys.contains(&"[f]"), "epic should show filter hint");
    assert!(keys.contains(&"[?]"), "epic should show help hint");
}

#[test]
fn description_editor_result_for_epic() {
    let mut app = App::new(vec![]);
    app.input.mode = InputMode::InputEpicDescription;
    app.input.epic_draft = Some(EpicDraft {
        title: "E".to_string(),
        ..Default::default()
    });
    let cmds = app.update(Message::Editor(
        crate::tui::messages::EditorMessage::DescriptionResult("epic desc\nline 2".to_string()),
    ));
    // Description submit now completes creation immediately
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Epic(crate::tui::commands::EpicCommand::Insert(_))
    )));
}

#[test]
fn tasks_for_current_view_board_excludes_epic_tasks() {
    let mut app = App::new(vec![]);
    let standalone = make_task(1, TaskStatus::Backlog);
    let mut subtask = make_task(2, TaskStatus::Backlog);
    subtask.epic_id = Some(EpicId(10));
    app.board.tasks = vec![standalone, subtask];

    let visible = app.tasks_for_current_view();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, TaskId(1));
}

#[test]
fn tasks_for_current_view_epic_shows_only_subtasks() {
    let mut app = App::new(vec![]);
    let standalone = make_task(1, TaskStatus::Backlog);
    let mut subtask = make_task(2, TaskStatus::Running);
    subtask.epic_id = Some(EpicId(10));
    app.board.tasks = vec![standalone, subtask];

    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };

    let visible = app.tasks_for_current_view();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, TaskId(2));
}

#[test]
fn flattened_board_shows_running_subtasks_but_not_backlog() {
    let mut app = App::new(vec![]);
    let standalone = make_task(1, TaskStatus::Backlog);
    let mut backlog_subtask = make_task(2, TaskStatus::Backlog);
    backlog_subtask.epic_id = Some(EpicId(10));
    let mut running_subtask = make_task(3, TaskStatus::Running);
    running_subtask.epic_id = Some(EpicId(10));
    app.board.tasks = vec![standalone, backlog_subtask, running_subtask];
    app.board.epics = vec![make_epic(10)];
    app.board.flattened = true;

    let ids = visible_task_ids(&app);
    // Standalone backlog task is always visible
    assert!(ids.contains(&TaskId(1)));
    // Running subtask surfaces in flat mode
    assert!(ids.contains(&TaskId(3)));
    // Backlog subtask does NOT surface — backlog is excluded from flattening
    assert!(!ids.contains(&TaskId(2)));
}

#[test]
fn flattened_board_is_recursive_through_nested_epics() {
    let mut app = App::new(vec![]);
    // Epic tree: root(10) -> child(20)
    let mut child_epic = make_epic(20);
    child_epic.parent_epic_id = Some(EpicId(10));
    app.board.epics = vec![make_epic(10), child_epic];

    let mut t_root = make_task(1, TaskStatus::Backlog);
    t_root.epic_id = Some(EpicId(10));
    let mut t_leaf = make_task(2, TaskStatus::Running);
    t_leaf.epic_id = Some(EpicId(20));
    app.board.tasks = vec![t_root, t_leaf];
    app.board.flattened = true;

    // Backlog is NOT flattened: the root epic card shows, not the individual task
    let backlog = app.column_items_for_status(TaskStatus::Backlog);
    assert!(
        backlog
            .iter()
            .any(|i| matches!(i, ColumnItem::Epic(e) if e.id == EpicId(10))),
        "root epic card should appear in backlog in flat mode"
    );
    assert!(
        !backlog
            .iter()
            .any(|i| matches!(i, ColumnItem::Task(t) if t.id == TaskId(1))),
        "backlog subtask should NOT surface in flat mode"
    );

    // Running IS flattened: t_leaf bubbles up from the nested epic
    let running = app.column_items_for_status(TaskStatus::Running);
    assert!(running
        .iter()
        .any(|i| matches!(i, ColumnItem::Task(t) if t.id == TaskId(2))));
}

#[test]
fn flattened_board_hides_epic_cards_in_active_columns_only() {
    let mut app = App::new(vec![]);
    let mut child = make_epic(20);
    child.parent_epic_id = Some(EpicId(10));
    app.board.epics = vec![make_epic(10), child];
    app.board.flattened = true;

    // Running/Review columns: epic cards are hidden (tasks surface via EpicHeader)
    for status in [TaskStatus::Running, TaskStatus::Review] {
        let items = app.column_items_for_status(status);
        assert!(
            items.iter().all(|i| matches!(
                i,
                ColumnItem::Task(_)
                    | ColumnItem::EpicHeader(_)
                    | ColumnItem::SubstatusLabel(_)
                    | ColumnItem::OrphanSeparator
            )),
            "flattened view should emit no navigable Epic cards in {status:?} column"
        );
    }

    // Backlog column: epic cards remain visible (backlog is excluded from
    // flattening). Done is excluded too, but
    // `flattened_board_shows_epic_cards_in_done` owns that assertion.
    let backlog_items = app.column_items_for_status(TaskStatus::Backlog);
    assert!(
        backlog_items
            .iter()
            .any(|i| matches!(i, ColumnItem::Epic(_))),
        "backlog column should still show epic cards in flat mode"
    );
}

#[test]
fn flattened_epic_view_shows_only_that_subtree() {
    let mut app = App::new(vec![]);
    // Two root epics with tasks under each
    app.board.epics = vec![make_epic(10), make_epic(20)];
    let mut a = make_task(1, TaskStatus::Backlog);
    a.epic_id = Some(EpicId(10));
    let mut b = make_task(2, TaskStatus::Backlog);
    b.epic_id = Some(EpicId(20));
    app.board.tasks = vec![a, b];
    app.board.flattened = true;
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };

    let ids = visible_task_ids(&app);
    assert!(ids.contains(&TaskId(1)));
    assert!(!ids.contains(&TaskId(2)));
}

#[test]
fn shift_f_key_toggles_flattened() {
    let mut app = App::new(vec![]);
    assert!(!app.board.flattened);
    app.handle_key(KeyEvent::new(KeyCode::Char('F'), KeyModifiers::SHIFT));
    assert!(app.board.flattened);
    app.handle_key(KeyEvent::new(KeyCode::Char('F'), KeyModifiers::SHIFT));
    assert!(!app.board.flattened);
}

#[test]
fn shift_f_toggles_flattened_inside_epic_view() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(10),
    )));
    app.handle_key(KeyEvent::new(KeyCode::Char('F'), KeyModifiers::SHIFT));
    assert!(app.board.flattened);
}

#[test]
fn toggle_flattened_clamps_selection_in_backlog() {
    let mut app = App::new(vec![]);
    // Board with one root epic and one subtask inside. No standalone tasks.
    app.board.epics = vec![make_epic(10)];
    let mut subtask = make_task(1, TaskStatus::Backlog);
    subtask.epic_id = Some(EpicId(10));
    app.board.tasks = vec![subtask];

    // Select the (only) item in the backlog column: the epic card at row 0.
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    // Toggle flatten: backlog is excluded from flattening, so epic card stays.
    // Count stays 1 and row 0 remains valid.
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleFlattened,
    ));
    assert!(app.board.flattened);
    assert_eq!(app.selected_row()[0], 0);

    // Set row out-of-bounds, then toggle back to non-flat. Clamping still fires.
    app.selection_mut().set_row(1, 5);
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleFlattened,
    ));
    assert!(!app.board.flattened);
    let count = app.column_items_for_status(TaskStatus::Backlog).len();
    assert!(count > 0);
    assert!(app.selected_row()[0] < count);
}

#[test]
fn flattened_survives_enter_and_exit_epic() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleFlattened,
    ));
    assert!(app.board.flattened);

    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(10),
    )));
    assert!(app.board.flattened, "flatten should persist into epic view");

    app.update(Message::Epic(crate::tui::messages::EpicMessage::Exit));
    assert!(app.board.flattened, "flatten should persist back to board");
}

#[test]
fn flattened_survives_refresh_tasks() {
    let mut app = App::new(vec![]);
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleFlattened,
    ));
    assert!(app.board.flattened);

    app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        vec![make_task(1, TaskStatus::Backlog)],
    )));
    assert!(app.board.flattened);
}
