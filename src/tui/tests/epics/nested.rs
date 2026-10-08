use super::*;

#[test]
fn epic_view_header_shows_auto_dispatch_indicator() {
    let mut app = App::new(vec![]);
    let mut epic = make_epic(1);
    epic.auto_dispatch = true;
    app.board.epics = vec![epic];
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(1),
    )));

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "auto dispatch [U]"),
        "Expected 'auto dispatch [U]' in header"
    );
}

#[test]
fn epic_view_header_shows_manual_dispatch_indicator() {
    let mut app = App::new(vec![]);
    let mut epic = make_epic(1);
    epic.auto_dispatch = false;
    app.board.epics = vec![epic];
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(1),
    )));

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "manual dispatch [U]"),
        "Expected 'manual dispatch [U]' in header"
    );
}

#[test]
fn exit_sub_epic_returns_to_parent_epic() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(1), make_epic(2)];
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(1),
    )));
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(2),
    )));

    app.update(Message::Epic(crate::tui::messages::EpicMessage::Exit));

    match &app.board.view_mode {
        ViewMode::Epic { epic_id, .. } => {
            assert_eq!(*epic_id, EpicId(1), "should return to parent epic 1");
        }
        _ => panic!("Expected ViewMode::Epic after exiting sub-epic"),
    }
}

#[test]
fn exit_from_root_epic_returns_to_board() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(1)];
    app.selection_mut().set_column(3);
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(1),
    )));
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Exit));

    match &app.board.view_mode {
        ViewMode::Board(sel) => {
            assert_eq!(sel.column(), 3, "board column should be restored");
        }
        _ => panic!("Expected ViewMode::Board"),
    }
}

#[test]
fn board_view_excludes_sub_epics() {
    let mut app = App::new(vec![]);
    let mut sub = make_epic(20);
    sub.parent_epic_id = Some(EpicId(10));
    app.board.epics = vec![make_epic(10), sub];

    let items = app.view().column_items_for_status(TaskStatus::Backlog);
    // Only root epic (id=10) should appear; sub-epic (id=20) must not
    let epic_ids: Vec<i64> = items
        .iter()
        .filter_map(|i| {
            if let ColumnItem::Epic(e) = i {
                Some(e.id.0)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(epic_ids, vec![10], "only root epic should appear on board");
}

#[test]
fn epic_view_includes_sub_epics_as_column_items() {
    let mut app = App::new(vec![]);
    let mut sub = make_epic(20);
    sub.parent_epic_id = Some(EpicId(10));
    app.board.epics = vec![make_epic(10), sub];

    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(10),
    )));

    let items = app.view().column_items_for_status(TaskStatus::Backlog);
    // sub-epic (id=20) should appear as an Epic column item
    let epic_ids: Vec<i64> = items
        .iter()
        .filter_map(|i| {
            if let ColumnItem::Epic(e) = i {
                Some(e.id.0)
            } else {
                None
            }
        })
        .collect();
    assert!(
        epic_ids.contains(&20),
        "sub-epic should appear inside parent epic view"
    );
}

#[test]
fn epic_view_breadcrumb_shows_parent_and_child_title() {
    let mut app = App::new(vec![]);
    let parent_epic = make_epic_with_title(1, "Root Epic");
    let child_epic = make_epic_with_title(2, "Child Epic");
    app.board.epics = vec![parent_epic.clone(), child_epic.clone()];

    // Nested: viewing child epic, parent is another epic view
    app.board.view_mode = ViewMode::Epic {
        epic_id: child_epic.id,
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Epic {
            epic_id: parent_epic.id,
            selection: BoardSelection::new_for_epic(),
            parent: Box::new(ViewMode::Board(BoardSelection::new())),
        }),
    };

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Root Epic"),
        "breadcrumb should show parent epic title"
    );
    assert!(
        buffer_contains(&buf, "Child Epic"),
        "breadcrumb should show current epic title"
    );
    // The separator between parent and child
    assert!(
        buffer_contains(&buf, "›"),
        "breadcrumb should show › separator between parent and child"
    );
}

#[test]
fn epic_view_no_breadcrumb_when_parent_is_board() {
    let mut app = App::new(vec![]);
    let epic = make_epic_with_title(1, "Only Epic");
    app.board.epics = vec![epic.clone()];
    app.board.view_mode = ViewMode::Epic {
        epic_id: epic.id,
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "Only Epic"),
        "title should show current epic title"
    );
}

#[test]
fn create_epic_in_epic_view_inherits_parent() {
    let mut app = App::new(vec![]);
    let parent_id = EpicId(42);
    app.board.view_mode = ViewMode::Epic {
        epic_id: parent_id,
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };

    // Enter the epic creation flow
    app.update(Message::Epic(crate::tui::messages::EpicMessage::StartNew));
    assert_eq!(app.input.mode, InputMode::InputEpicTitle);

    // The draft should already know about the parent
    let draft_parent = app.input.epic_draft.as_ref().and_then(|d| d.parent_epic_id);
    assert_eq!(
        draft_parent,
        Some(parent_id),
        "epic_draft.parent_epic_id should be set to current epic's id"
    );

    // Submit title then description — description submit now completes creation
    app.update(Message::Epic(
        crate::tui::messages::EpicMessage::SubmitTitle("Sub Epic".to_string()),
    ));
    let cmds = app.update(Message::Epic(
        crate::tui::messages::EpicMessage::SubmitDescription("desc".to_string()),
    ));

    let draft = cmds
        .iter()
        .find_map(|c| {
            if let Command::Epic(crate::tui::commands::EpicCommand::Insert(d)) = c {
                Some(d)
            } else {
                None
            }
        })
        .expect("expected Command::InsertEpic");

    assert_eq!(
        draft.parent_epic_id,
        Some(parent_id),
        "InsertEpic draft must carry parent_epic_id"
    );
}

#[test]
fn breadcrumb_shows_three_levels() {
    let mut app = App::new(vec![]);
    let grandparent = make_epic_with_title(1, "Grandparent");
    let parent = make_epic_with_title(2, "ParentEpic");
    let child = make_epic_with_title(3, "ChildEpic");
    app.board.epics = vec![grandparent, parent, child];

    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(3),
        selection: BoardSelection::new_for_epic(),
        parent: Box::new(ViewMode::Epic {
            epic_id: EpicId(2),
            selection: BoardSelection::new_for_epic(),
            parent: Box::new(ViewMode::Epic {
                epic_id: EpicId(1),
                selection: BoardSelection::new_for_epic(),
                parent: Box::new(ViewMode::Board(BoardSelection::new())),
            }),
        }),
    };

    let buf = render_to_buffer(&mut app, 120, 40);
    assert!(
        buffer_contains(&buf, "Grandparent"),
        "breadcrumb should show grandparent title"
    );
    assert!(
        buffer_contains(&buf, "ParentEpic"),
        "breadcrumb should show parent title"
    );
    assert!(
        buffer_contains(&buf, "ChildEpic"),
        "breadcrumb should show child title"
    );
}

#[test]
fn test_epic_anchor_preserved_on_refresh() {
    let tasks = vec![make_task(1, TaskStatus::Backlog)];
    let epics = vec![make_epic(1)];
    let mut app = App::new(tasks.clone());
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Refresh(
        epics.clone(),
    )));

    let items = app.view().column_items_for_status(TaskStatus::Backlog);
    let epic_row = items
        .iter()
        .position(|i| matches!(i, ColumnItem::Epic(_)))
        .expect("epic should be in Backlog column");
    for _ in 0..epic_row {
        app.update(Message::NavigateRow(1));
    }
    assert!(matches!(
        app.view().column_items_for_status(TaskStatus::Backlog)[app.selection().row(1)],
        ColumnItem::Epic(_)
    ));

    // Refresh same data
    app.update(Message::Task(crate::tui::messages::TaskMessage::Refresh(
        tasks,
    )));
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Refresh(
        epics,
    )));

    // Still on the epic
    assert!(matches!(
        app.view().column_items_for_status(TaskStatus::Backlog)[app.selection().row(1)],
        ColumnItem::Epic(_)
    ));
}

#[test]
fn epic_view_navigation_stays_within_projects_and_done() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(10),
    )));
    assert!(matches!(app.board.view_mode, ViewMode::Epic { .. }));

    // Starts at Backlog (column 1)
    assert_eq!(app.selected_column(), 1);

    // Navigate left past Backlog — should not enter Projects (col 0)
    app.update(Message::NavigateColumn(-1));
    assert_eq!(
        app.selected_column(),
        1,
        "should not enter Projects (col 0) from epic view"
    );

    // Navigate right to Done (col 4)
    for _ in 0..3 {
        app.update(Message::NavigateColumn(1));
    }
    assert_eq!(app.selected_column(), 4);

    // Navigate right past Done — there is no fifth column to enter
    app.update(Message::NavigateColumn(1));
    assert_eq!(
        app.selected_column(),
        4,
        "col 4 (Done) is the rightmost column in epic view"
    );
}

#[test]
fn test_selection_survives_flatten_toggle() {
    // Use task IDs > 1 so they sort after Epic(1) in the column.
    // Column order: [Task(1), Epic(1), Task(2)] — tasks inserted before epics,
    // stable sort keeps Task(1) before Epic(1) when both have key (5,1,1).
    // Navigate +2 to reach Task(2) at row 2.
    let tasks = vec![
        make_task(1, TaskStatus::Backlog),
        make_task(2, TaskStatus::Backlog),
    ];
    let epics = vec![make_epic(1)];
    let mut app = App::new(tasks.clone());
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Refresh(
        epics.clone(),
    )));

    app.update(Message::NavigateRow(1)); // row 1 — Epic(1)
    app.update(Message::NavigateRow(1)); // row 2 — Task(2)
    let items = app.view().column_items_for_status(TaskStatus::Backlog);
    let pre_id: TaskId = match &items[app.selection().row(0)] {
        ColumnItem::Task(t) => t.id,
        _ => panic!("expected task at cursor"),
    };

    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleFlattened,
    ));
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleFlattened,
    ));

    let items = app.view().column_items_for_status(TaskStatus::Backlog);
    let post_id: TaskId = match &items[app.selection().row(0)] {
        ColumnItem::Task(t) => t.id,
        _ => panic!("expected task at cursor"),
    };
    assert_eq!(pre_id, post_id);
}

#[test]
fn reparent_epic_message_and_command_variants_compile() {
    use crate::tui::commands::EpicCommand;
    use crate::tui::messages::EpicMessage;
    use crate::tui::types::TreeNav;

    let _msgs = [
        EpicMessage::StartReparent(EpicId(1)),
        EpicMessage::ReparentNavigate(TreeNav::Down),
        EpicMessage::ReparentConfirm,
        EpicMessage::ReparentExecute,
        EpicMessage::ReparentCancel,
    ];
    let _cmd = EpicCommand::Reparent {
        id: EpicId(1),
        new_parent: Some(EpicId(2)),
    };
    let _cmd_root = EpicCommand::Reparent {
        id: EpicId(1),
        new_parent: None,
    };
}
