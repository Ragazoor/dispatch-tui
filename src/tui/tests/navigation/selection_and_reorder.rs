use super::*;

#[test]
fn test_selection_preserved_when_task_above_cursor_moves() {
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Backlog), // row 0
        make_task(2, TaskStatus::Backlog), // row 1 — cursor here
        make_task(3, TaskStatus::Backlog), // row 2
    ]);
    // App starts at Backlog (nav col 1); navigate down to row 1.
    app.update(Message::NavigateRow(1));
    assert_eq!(app.selection().row(1), 1);

    // Task 1 moves out; Task 2 follows to row 0.
    app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        vec![
            make_task(1, TaskStatus::Running),
            make_task(2, TaskStatus::Backlog),
            make_task(3, TaskStatus::Backlog),
        ],
    )));

    // Anchor follows task 2 — stays in Backlog (nav col 1) at row 0.
    assert_eq!(app.selection().column(), 1);
    assert_eq!(app.selection().row(1), 0);
    let items = app.view().column_items_for_status(TaskStatus::Backlog);
    assert!(matches!(items[0], ColumnItem::Task(t) if t.id == TaskId(2)));
}

#[test]
fn test_selection_follows_task_to_new_column() {
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Backlog), // row 0 — default cursor
        make_task(2, TaskStatus::Backlog),
    ]);
    // App starts at Backlog (nav col 1).
    assert_eq!(app.selection().column(), 1);
    assert_eq!(app.selection().row(1), 0);

    // Task 1 dispatched to Running
    app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        vec![
            make_task(1, TaskStatus::Running),
            make_task(2, TaskStatus::Backlog),
        ],
    )));

    assert_eq!(app.selection().column(), 2); // Running = nav col 2
    assert_eq!(app.selection().row(2), 0);
    // Running is a sectioned column, so its first item is the section header.
    // The cursor's row 0 is the first *selectable* item, which is the task.
    let items = app.view().column_items_for_status(TaskStatus::Running);
    let first_selectable = items
        .iter()
        .find(|i| i.is_selectable())
        .expect("Running holds one task");
    assert!(matches!(first_selectable, ColumnItem::Task(t) if t.id == TaskId(1)));
}

#[test]
fn test_selection_falls_back_when_task_deleted() {
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Backlog),
        make_task(3, TaskStatus::Backlog),
    ]);
    // App starts at Backlog (nav col 1); navigate down to row 2 (Task 3).
    app.update(Message::NavigateRow(1));
    app.update(Message::NavigateRow(1));
    assert_eq!(app.selection().row(1), 2); // Task 3

    // Task 3 deleted
    app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        vec![
            make_task(1, TaskStatus::Backlog),
            make_task(2, TaskStatus::Backlog),
        ],
    )));

    assert_eq!(app.selection().column(), 1);
    assert_eq!(app.selection().row(1), 1); // clamped to last valid row
}

#[test]
fn test_selection_preserved_on_same_data_refresh() {
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Backlog),
        make_task(3, TaskStatus::Backlog),
    ]);
    // App starts at Backlog (nav col 1); navigate down to row 1.
    app.update(Message::NavigateRow(1));
    assert_eq!(app.selection().row(1), 1);

    app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        vec![
            make_task(1, TaskStatus::Backlog),
            make_task(2, TaskStatus::Backlog),
            make_task(3, TaskStatus::Backlog),
        ],
    )));

    assert_eq!(app.selection().row(1), 1);
    let items = app.view().column_items_for_status(TaskStatus::Backlog);
    assert!(matches!(items[1], ColumnItem::Task(t) if t.id == TaskId(2)));
}

#[test]
fn test_selection_falls_back_when_column_empties() {
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Running),
    ]);
    // App starts at Backlog (nav col 1); navigate right to Running (nav col 2).
    app.update(Message::NavigateColumn(1));
    assert_eq!(app.selection().column(), 2);

    // Task 2 deleted — Running column empties, anchor not found
    app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        vec![make_task(1, TaskStatus::Backlog)],
    )));

    // Cursor must be in a valid state: row 0 in the empty Running column
    assert_eq!(app.selection().column(), 2);
    assert_eq!(app.selection().row(2), 0);
}

#[test]
fn board_selection_task_col_row_uses_offset() {
    let mut sel = BoardSelection::new();
    sel.set_row(1, 10); // Backlog = nav col 1 → array index 0
    assert_eq!(sel.row(1), 10);
    sel.set_row(4, 5); // Done = nav col 4 → array index 3
    assert_eq!(sel.row(4), 5);
}

// --- [1,4] column-range navigation tests ---

#[test]
fn navigate_left_at_backlog_is_noop() {
    let mut app = make_app();
    // Board starts at Backlog (col 1), which is the leftmost column.
    assert_eq!(app.selected_column(), 1);
    app.update(Message::NavigateColumn(-1)); // clamp at 1
    assert_eq!(app.selected_column(), 1);
}

// --- update_anchor_from_current with pre-computed EpicStatsMap ---

#[test]
fn update_anchor_from_current_accepts_stats_and_sets_epic_anchor() {
    // Direct API test: update_anchor_from_current() must set the anchor to the epic
    // when the cursor is on row 1 (epic sorts after task id=1).
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.epics = vec![make_epic(10)];
    // Invalidate stale cache so cached_epic_stats() rebuilds with the new epic.
    app.invalidate_layout_cache();
    // Task(id=1) sort key: (priority=0, sort=1, id=1)
    // Epic(id=10) sort key: (priority=0, sort=10, id=10)  → row 1
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 1);

    let _ = app.cached_epic_stats();
    app.update_anchor_from_current();

    assert_eq!(app.selection().anchor, Some(ColumnAnchor::Epic(EpicId(10))));
}

#[test]
fn navigate_row_with_epics_sets_anchor_via_stats() {
    // Integration test: navigating down to an epic (via j key equivalent) must set
    // the anchor to ColumnAnchor::Epic, proving the stats path is wired in.
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.epics = vec![make_epic(10)];
    // Invalidate stale cache so navigation rebuilds with the new epic.
    app.invalidate_layout_cache();
    // Row 0 = Task(1), row 1 = Epic(10)
    app.update(Message::NavigateRow(1));

    assert_eq!(app.selection().row(1), 1);
    assert_eq!(app.selection().anchor, Some(ColumnAnchor::Epic(EpicId(10))));
}

#[test]
fn navigate_column_with_epics_sets_anchor_via_stats() {
    // Integration test: navigating to a column with epics must set the anchor,
    // exercising the stats path in handle_navigate_column.
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.epics = vec![make_epic(10)]; // epic in Backlog (nav col 1)

    // Navigate right then back: anchor after returning to Backlog col should be Task(1)
    app.update(Message::NavigateColumn(1)); // → Running
    app.update(Message::NavigateColumn(-1)); // → Backlog (row 0 = Task(1))
    assert_eq!(app.selection().anchor, Some(ColumnAnchor::Task(TaskId(1))));
}

// --- jump-to-top/bottom ([/]) ---

#[test]
fn bracket_right_jumps_to_last_row() {
    // make_app() gives 2 Backlog tasks (ids 1,2) at rows 0 and 1
    let mut app = make_app();
    app.selection_mut().set_column(1); // Backlog = nav col 1
                                       // Start at row 0 (default); ] should jump to row 1 (last)
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char(']'))));
    assert!(cmds.is_empty());
    assert_eq!(app.selection().row(1), 1, "should jump to last task");
    assert!(!app.on_select_all());
}

#[test]
fn bracket_left_jumps_to_first_row() {
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Backlog),
        make_task(3, TaskStatus::Backlog),
    ]);
    app.selection_mut().set_column(1); // Backlog
    app.update(Message::NavigateRow(1));
    app.update(Message::NavigateRow(1)); // now at row 2
    assert_eq!(app.selection().row(1), 2);
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('['))));
    assert!(cmds.is_empty());
    assert_eq!(app.selection().row(1), 0, "should jump to first task");
    assert!(!app.on_select_all());
}

#[test]
fn bracket_left_clears_on_select_all() {
    let mut app = make_app();
    app.selection_mut().set_column(1);
    // Navigate up from row 0 → on_select_all
    app.update(Message::NavigateRow(-1));
    assert!(app.on_select_all(), "precondition: on select-all toggle");
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char('['))));
    assert!(cmds.is_empty());
    assert!(!app.on_select_all(), "[ should clear on_select_all");
    assert_eq!(app.selection().row(1), 0, "should land on first task");
}

#[test]
fn bracket_right_clears_on_select_all() {
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Backlog),
    ]);
    app.selection_mut().set_column(1);
    app.update(Message::NavigateRow(-1)); // → on_select_all
    assert!(app.on_select_all(), "precondition");
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char(']'))));
    assert!(cmds.is_empty());
    assert!(!app.on_select_all(), "] should clear on_select_all");
    assert_eq!(app.selection().row(1), 1, "should land on last task");
}

#[test]
fn bracket_right_on_empty_column_is_noop() {
    let mut app = App::new(vec![]);
    app.selection_mut().set_column(1); // empty Backlog
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char(']'))));
    assert!(cmds.is_empty());
    assert_eq!(app.selection().row(1), 0, "row unchanged on empty column");
}

#[test]
fn bracket_left_on_empty_column_is_noop() {
    // Use a fresh app with no tasks; to verify true no-op, manually set
    // on_select_all = true to see that [ does NOT clear it when empty.
    let mut app2 = App::new(vec![]);
    app2.selection_mut().set_column(1); // empty Backlog
    app2.selection_mut().on_select_all = true; // non-default state
    let cmds = without_usage(app2.handle_key(make_key(KeyCode::Char('['))));
    assert!(cmds.is_empty());
    // on_select_all should remain unchanged because the column is empty — true no-op
    assert!(
        app2.on_select_all(),
        "[ on empty column must not mutate on_select_all"
    );
    assert_eq!(app2.selection().row(1), 0, "row unchanged on empty column");
}

#[test]
fn bracket_right_on_single_item_stays_at_row_zero() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.selection_mut().set_column(1);
    let cmds = without_usage(app.handle_key(make_key(KeyCode::Char(']'))));
    assert!(cmds.is_empty());
    assert_eq!(
        app.selection().row(1),
        0,
        "single item: row 0 is both first and last"
    );
}

// --- handle_navigate_row / _first / _last: invalid-column defensive guards ---
//
// Columns are normally 0 (unselected/toggle-row sentinel) through
// COLUMN_COUNT (4). Nothing in the app ever drives the selection to column 0
// or past the last column, but the handlers guard against it defensively —
// exercise those guards directly by poking the selection.

#[test]
fn navigate_row_messages_are_noop_for_invalid_columns() {
    for col in [
        0,
        TaskStatus::COLUMN_COUNT + 1,
        TaskStatus::COLUMN_COUNT + 2,
    ] {
        for msg in [
            Message::NavigateRow(1),
            Message::NavigateRowFirst,
            Message::NavigateRowLast,
        ] {
            let mut app = make_app();
            app.selection_mut().set_column(col);
            let cmds = app.update(msg.clone());
            assert!(cmds.is_empty(), "col {col} should be a no-op for {msg:?}");
        }
    }
}

// --- handle_navigate_row: empty-column tail (moving up from an empty column) ---

#[test]
fn navigate_row_up_in_empty_column_moves_to_toggle_row() {
    let mut app = App::new(vec![]);
    app.selection_mut().set_column(1); // empty Backlog
    assert!(!app.on_select_all(), "precondition");

    let cmds = app.update(Message::NavigateRow(-1));
    assert!(cmds.is_empty());
    assert!(
        app.on_select_all(),
        "moving up in an empty column should land on the toggle row"
    );
}

// --- handle_reorder_item: guards that return early ---

#[test]
fn reorder_item_is_noop_for_invalid_columns() {
    for col in [
        0,
        TaskStatus::COLUMN_COUNT + 1,
        TaskStatus::COLUMN_COUNT + 2,
    ] {
        let mut app = make_app();
        app.selection_mut().set_column(col);
        let cmds = app.update(Message::Task(
            crate::tui::messages::TaskMessage::ReorderItem(1),
        ));
        assert!(cmds.is_empty(), "col {col} should be a no-op");
    }
}

// --- handle_reorder_item: epic branches ---
//
// Epics and tasks share a column (Backlog is never flattened), so an epic
// card can sit anywhere in the reorder list — alone, or interleaved with
// task cards.

#[test]
fn reorder_epic_down_swaps_sort_order_and_persists_with_status_none() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10), make_epic(20)];
    app.selection_mut().set_column(1); // Backlog
    app.selection_mut().set_row(1, 0); // cursor on epic 10 (lower effective sort, row 0)

    let cmds = app.update(Message::Task(
        crate::tui::messages::TaskMessage::ReorderItem(1),
    ));

    let e10 = app.board.epics.iter().find(|e| e.id == EpicId(10)).unwrap();
    let e20 = app.board.epics.iter().find(|e| e.id == EpicId(20)).unwrap();
    assert_eq!(e10.sort_order, Some(20));
    assert_eq!(e20.sort_order, Some(10));
    assert_eq!(
        cmds.iter()
            .filter(|c| matches!(
                c,
                Command::Epic(crate::tui::commands::EpicCommand::Persist { status: None, .. })
            ))
            .count(),
        2,
        "both epics should be persisted with status untouched"
    );
    assert_eq!(app.selection().row(1), 1, "cursor follows the moved epic");
}

#[test]
fn reorder_epic_up_swaps_sort_order_and_persists_with_status_none() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10), make_epic(20)];
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 1); // cursor on epic 20 (row 1)

    let cmds = app.update(Message::Task(
        crate::tui::messages::TaskMessage::ReorderItem(-1),
    ));

    let e10 = app.board.epics.iter().find(|e| e.id == EpicId(10)).unwrap();
    let e20 = app.board.epics.iter().find(|e| e.id == EpicId(20)).unwrap();
    assert_eq!(e20.sort_order, Some(10));
    assert_eq!(e10.sort_order, Some(20));
    assert_eq!(
        cmds.iter()
            .filter(|c| matches!(
                c,
                Command::Epic(crate::tui::commands::EpicCommand::Persist { status: None, .. })
            ))
            .count(),
        2
    );
    assert_eq!(app.selection().row(1), 0, "cursor follows the moved epic");
}

#[test]
fn reorder_task_down_into_epic_persists_both_and_swaps_sort_order() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.epics = vec![make_epic(10)];
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0); // cursor on task 1 (lower effective sort than epic 10)

    let cmds = app.update(Message::Task(
        crate::tui::messages::TaskMessage::ReorderItem(1),
    ));

    let task = app.board.find_task(TaskId(1)).unwrap();
    let epic = app.board.epics.iter().find(|e| e.id == EpicId(10)).unwrap();
    assert_eq!(task.sort_order, Some(10));
    assert_eq!(epic.sort_order, Some(1));
    assert_eq!(
        cmds.len(),
        2,
        "swapping across the task/epic boundary persists both"
    );
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Task(crate::tui::commands::TaskCommand::Persist(_))
    )));
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Epic(crate::tui::commands::EpicCommand::Persist { .. })
    )));
    assert_eq!(app.selection().row(1), 1);
}

#[test]
fn reorder_epic_up_into_task_persists_both_and_swaps_sort_order() {
    // task 1 + epic 10 in Backlog, cursor on the epic (higher effective sort).
    let mut app = make_app_with_epic_selected();

    let cmds = app.update(Message::Task(
        crate::tui::messages::TaskMessage::ReorderItem(-1),
    ));

    let task = app.board.find_task(TaskId(1)).unwrap();
    let epic = app.board.epics.iter().find(|e| e.id == EpicId(10)).unwrap();
    assert_eq!(epic.sort_order, Some(1));
    assert_eq!(task.sort_order, Some(10));
    assert_eq!(cmds.len(), 2);
    assert_eq!(app.selection().row(1), 0);
}

// --- handle_reorder_item: equal-effective-sort tie break (`sort_order: None`
// items falling back to `id.0`, the realistic path for freshly-created items) ---

#[test]
fn reorder_tied_effective_sort_moving_down_offsets_mover_by_plus_one() {
    // Task id 7 and epic id 7 both have sort_order: None, so both fall back to
    // an effective sort value of 7 — a genuine tie. Tasks are placed in the
    // item list before epics, so the task sorts first (row 0) on the tie.
    let mut app = App::new(vec![make_task(7, TaskStatus::Backlog)]);
    app.board.epics = vec![make_epic(7)];
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0); // cursor on task 7

    let cmds = app.update(Message::Task(
        crate::tui::messages::TaskMessage::ReorderItem(1),
    ));

    let task = app.board.find_task(TaskId(7)).unwrap();
    let epic = app.board.epics.iter().find(|e| e.id == EpicId(7)).unwrap();
    assert_eq!(task.sort_order, Some(8), "tied mover moving down gets +1");
    assert_eq!(epic.sort_order, Some(7));
    assert_eq!(cmds.len(), 2);
}

#[test]
fn reorder_tied_effective_sort_moving_up_offsets_mover_by_minus_one() {
    let mut app = App::new(vec![make_task(8, TaskStatus::Backlog)]);
    app.board.epics = vec![make_epic(8)];
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 1); // cursor on epic 8 (ties with task 8, sorts second)

    let cmds = app.update(Message::Task(
        crate::tui::messages::TaskMessage::ReorderItem(-1),
    ));

    let task = app.board.find_task(TaskId(8)).unwrap();
    let epic = app.board.epics.iter().find(|e| e.id == EpicId(8)).unwrap();
    assert_eq!(epic.sort_order, Some(7), "tied mover moving up gets -1");
    assert_eq!(task.sort_order, Some(8));
    assert_eq!(cmds.len(), 2);
}
