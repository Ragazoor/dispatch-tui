use super::*;

// ---------------------------------------------------------------------------
// Feed epic manual trigger — message handling
// ---------------------------------------------------------------------------

#[test]
fn trigger_epic_feed_sets_status_and_returns_command() {
    let mut app = App::new(vec![]);
    let mut epic = make_epic(10);
    epic.feed_command = Some("echo '[]'".to_string());
    app.board.epics = vec![epic];

    let cmds = app.update(Message::Feed(
        crate::tui::messages::FeedMessage::TriggerEpic(EpicId(10)),
    ));

    assert!(
        app.status_message()
            .map(|s| s.contains("Epic 10"))
            .unwrap_or(false),
        "status should mention epic title"
    );
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Feed(crate::tui::commands::FeedCommand::TriggerEpic {
                epic_id,
                ..
            }) if *epic_id == EpicId(10)
        )),
        "should return TriggerEpicFeed command"
    );
}

#[test]
fn trigger_epic_feed_no_feed_command_sets_status_no_command() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)]; // no feed_command

    let cmds = app.update(Message::Feed(
        crate::tui::messages::FeedMessage::TriggerEpic(EpicId(10)),
    ));

    assert!(
        app.status_message().is_some(),
        "should show a status message"
    );
    assert!(
        !cmds.iter().any(|c| matches!(
            c,
            Command::Feed(crate::tui::commands::FeedCommand::TriggerEpic { .. })
        )),
        "should not return TriggerEpicFeed command when no feed_command"
    );
}

#[test]
fn feed_refreshed_sets_status_and_returns_refresh_from_db() {
    let mut app = App::new(vec![]);

    let cmds = app.update(Message::Feed(
        crate::tui::messages::FeedMessage::Refreshed {
            epic_title: "My Feed Epic".to_string(),
            count: 5,
            degraded: None,
        },
    ));

    let status = app.status_message().unwrap_or("");
    assert!(
        status.contains("My Feed Epic"),
        "status should mention epic title, got: {status}"
    );
    assert!(
        status.contains("5"),
        "status should mention task count, got: {status}"
    );
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Task(crate::tui::commands::TaskCommand::RefreshFromDb)
        )),
        "should return RefreshFromDb command"
    );
}

#[test]
fn feed_refreshed_zero_items_still_succeeds() {
    let mut app = App::new(vec![]);

    let cmds = app.update(Message::Feed(
        crate::tui::messages::FeedMessage::Refreshed {
            epic_title: "Empty Feed".to_string(),
            count: 0,
            degraded: None,
        },
    ));

    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Task(crate::tui::commands::TaskCommand::RefreshFromDb)
        )),
        "zero-item refresh should still return RefreshFromDb"
    );

    let status = app.status_message().unwrap_or("");
    assert!(
        !status.contains("stderr"),
        "a genuinely empty feed is not an error and must not be flagged, got: {status}"
    );
}

// feeds.allium: DegradedEmptyEmission — a zero-item emission that wrote to
// stderr now fails before reaching `Refreshed` at all (see
// exec_trigger_epic_feed_fails_on_degraded_empty_emission in
// src/runtime/tests.rs), so the status-bar hint this variant used to carry
// (`wrote_stderr`) is retired along with it. `Refreshed` no longer has a
// stderr-flavoured branch to test.
#[test]
fn feed_refreshed_with_items_has_no_stderr_wording() {
    let mut app = App::new(vec![]);

    app.update(Message::Feed(
        crate::tui::messages::FeedMessage::Refreshed {
            epic_title: "Chatty Feed".to_string(),
            count: 7,
            degraded: None,
        },
    ));

    let status = app.status_message().unwrap_or("");
    assert!(
        !status.contains("stderr"),
        "a successful refresh must never mention stderr, got: {status}"
    );
}

/// feeds.allium: DegradedNonEmptyEmission. An additive refresh IS a success —
/// items synced, nothing failed — but the line must say that nothing was
/// removed, or an unchanged board reads as a reconciled one.
#[test]
fn feed_refreshed_degraded_says_it_removed_nothing() {
    let mut app = App::new(vec![]);

    let cmds = app.update(Message::Feed(
        crate::tui::messages::FeedMessage::Refreshed {
            epic_title: "Reviews".to_string(),
            count: 7,
            degraded: Some(
                "command wrote to stderr, so its omissions are not trusted: gh failed".to_string(),
            ),
        },
    ));

    let status = app.status_message().unwrap_or("");
    assert!(
        status.contains("7 task(s) synced"),
        "an additive refresh still reports its count, got: {status}"
    );
    assert!(
        status.contains("no removals"),
        "and must say nothing was removed, got: {status}"
    );
    assert!(
        status.contains("gh failed"),
        "and must carry the script's own reason, got: {status}"
    );
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Task(crate::tui::commands::TaskCommand::RefreshFromDb)
        )),
        "an additive refresh still wrote items, so the board must reload"
    );
}

#[test]
fn feed_failed_sets_status_no_refresh() {
    let mut app = App::new(vec![]);

    let cmds = app.update(Message::Feed(crate::tui::messages::FeedMessage::Failed {
        epic_title: "Bad Feed".to_string(),
        error: "exit code 1".to_string(),
    }));

    let status = app.status_message().unwrap_or("");
    assert!(
        status.contains("Bad Feed"),
        "status should mention epic title, got: {status}"
    );
    assert!(
        status.contains("exit code 1"),
        "status should mention the error, got: {status}"
    );
    assert!(
        !cmds.iter().any(|c| matches!(
            c,
            Command::Task(crate::tui::commands::TaskCommand::RefreshFromDb)
        )),
        "failed feed should NOT return RefreshFromDb"
    );
}

#[test]
fn epic_action_hints_shows_refresh_for_feed_epic() {
    let mut epic = make_epic(1);
    epic.feed_command = Some("echo '[]'".to_string());
    let hints = ui::epic_action_hints(&epic, ratatui::style::Color::Rgb(122, 162, 247));
    let keys = hint_keys(&hints);
    assert!(
        keys.contains(&"[r]"),
        "feed epic should show [r] refresh hint"
    );
}

#[test]
fn epic_action_hints_no_refresh_for_non_feed_epic() {
    let epic = make_epic(1); // no feed_command
    let hints = ui::epic_action_hints(&epic, ratatui::style::Color::Rgb(122, 162, 247));
    let keys = hint_keys(&hints);
    assert!(
        !keys.contains(&"[r]"),
        "non-feed epic should NOT show [r] refresh hint"
    );
}

#[test]
fn flat_view_inserts_epic_header_before_group() {
    let mut app = App::new(vec![]);
    let epic = make_epic(10);
    app.board.epics = vec![epic];
    let mut t1 = make_task(1, TaskStatus::Running);
    t1.epic_id = Some(EpicId(10));
    let mut t2 = make_task(2, TaskStatus::Running);
    t2.epic_id = Some(EpicId(10));
    app.board.tasks = vec![t1, t2];
    app.board.flattened = true;

    let items = app.column_items_for_status(TaskStatus::Running);
    let selectable: Vec<_> = items.iter().filter(|i| i.is_selectable()).collect();
    assert_eq!(selectable.len(), 2, "2 tasks");
    let headers: Vec<_> = items
        .iter()
        .filter(|i| matches!(i, ColumnItem::EpicHeader(_)))
        .collect();
    assert_eq!(headers.len(), 1, "one EpicHeader");
    assert!(
        matches!(headers[0], ColumnItem::EpicHeader(e) if e.id == EpicId(10)),
        "header must be for epic 10"
    );
}

#[test]
fn flat_view_header_sorts_before_its_tasks() {
    let mut app = App::new(vec![]);
    let mut epic = make_epic(10);
    epic.sort_order = Some(50);
    app.board.epics = vec![epic];
    let mut t = make_task(1, TaskStatus::Running);
    t.epic_id = Some(EpicId(10));
    t.sort_order = Some(100);
    app.board.tasks = vec![t];
    app.board.flattened = true;

    let items = app.column_items_for_status(TaskStatus::Running);
    let header_pos = items
        .iter()
        .position(|i| matches!(i, ColumnItem::EpicHeader(_)));
    let task_pos = items.iter().position(|i| matches!(i, ColumnItem::Task(_)));
    assert!(header_pos.is_some() && task_pos.is_some());
    assert!(
        header_pos.unwrap() < task_pos.unwrap(),
        "header precedes task"
    );
}

#[test]
fn flat_view_standalone_task_interleaves_by_sort_order() {
    let mut app = App::new(vec![]);
    let mut epic = make_epic(10);
    epic.sort_order = Some(200);
    app.board.epics = vec![epic];

    let mut standalone = make_task(1, TaskStatus::Running);
    standalone.sort_order = Some(100); // orphan tasks sort last (epic_sk = i64::MAX)

    let mut subtask = make_task(2, TaskStatus::Running);
    subtask.epic_id = Some(EpicId(10));
    subtask.sort_order = Some(300);

    app.board.tasks = vec![standalone, subtask];
    app.board.flattened = true;

    let items = app.column_items_for_status(TaskStatus::Running);
    // epic group comes first, orphan (no epic in board) sorts last with epic_sk = i64::MAX.
    // An OrphanSeparator is emitted between the epic group and the orphan task.
    assert!(matches!(items[0], ColumnItem::SubstatusLabel(_)));
    assert!(matches!(items[1], ColumnItem::EpicHeader(_)));
    assert!(matches!(items[2], ColumnItem::Task(t) if t.id == TaskId(2)));
    assert!(matches!(items[3], ColumnItem::OrphanSeparator));
    assert!(matches!(items[4], ColumnItem::Task(t) if t.id == TaskId(1)));
}

#[test]
fn flat_view_two_epics_get_two_headers() {
    let mut app = App::new(vec![]);
    let mut epic_a = make_epic(10);
    epic_a.sort_order = Some(10);
    let mut epic_b = make_epic(20);
    epic_b.sort_order = Some(20);
    app.board.epics = vec![epic_a, epic_b];

    let mut ta = make_task(1, TaskStatus::Running);
    ta.epic_id = Some(EpicId(10));
    let mut tb = make_task(2, TaskStatus::Running);
    tb.epic_id = Some(EpicId(20));
    app.board.tasks = vec![ta, tb];
    app.board.flattened = true;

    let items = app.column_items_for_status(TaskStatus::Running);
    let headers: Vec<_> = items
        .iter()
        .filter(|i| matches!(i, ColumnItem::EpicHeader(_)))
        .collect();
    assert_eq!(headers.len(), 2, "one header per epic");
}

#[test]
fn flat_view_no_header_when_epic_has_no_tasks_in_column() {
    let mut app = App::new(vec![]);
    // Epic 10 has a task in Running, not Backlog
    app.board.epics = vec![make_epic(10)];
    let mut t = make_task(1, TaskStatus::Running);
    t.epic_id = Some(EpicId(10));
    app.board.tasks = vec![t];
    app.board.flattened = true;

    let backlog = app.column_items_for_status(TaskStatus::Backlog);
    assert!(
        backlog
            .iter()
            .all(|i| !matches!(i, ColumnItem::EpicHeader(_))),
        "no header in Backlog when epic has no Backlog tasks"
    );
}

#[test]
fn flat_view_orphan_task_treated_as_standalone() {
    let mut app = App::new(vec![]);
    // No epics in board.epics, but a task references epic id 99
    let mut orphan = make_task(1, TaskStatus::Running);
    orphan.epic_id = Some(EpicId(99));
    app.board.tasks = vec![orphan];
    app.board.flattened = true;

    let items = app.column_items_for_status(TaskStatus::Running);
    let selectable: Vec<_> = items.iter().filter(|i| i.is_selectable()).collect();
    assert_eq!(selectable.len(), 1, "orphan renders without a header");
    assert!(matches!(selectable[0], ColumnItem::Task(_)));
}

#[test]
fn flat_view_tie_break_by_epic_id_when_sort_orders_equal() {
    let mut app = App::new(vec![]);
    let mut epic_a = make_epic(10);
    epic_a.sort_order = Some(50);
    let mut epic_b = make_epic(20);
    epic_b.sort_order = Some(50); // same sort_order, tie-broken by id
    app.board.epics = vec![epic_a, epic_b];

    let mut ta = make_task(1, TaskStatus::Running);
    ta.epic_id = Some(EpicId(10));
    let mut tb = make_task(2, TaskStatus::Running);
    tb.epic_id = Some(EpicId(20));
    app.board.tasks = vec![ta, tb];
    app.board.flattened = true;

    let items = app.column_items_for_status(TaskStatus::Running);
    let headers: Vec<EpicId> = items
        .iter()
        .filter_map(|i| {
            if let ColumnItem::EpicHeader(e) = i {
                Some(e.id)
            } else {
                None
            }
        })
        .collect();
    // Headers are emitted inline before their tasks; tie-broken by epic id.
    assert_eq!(headers, vec![EpicId(10), EpicId(20)]);
}

#[test]
fn non_flat_mode_has_no_epic_headers() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    let mut t = make_task(1, TaskStatus::Backlog);
    t.epic_id = Some(EpicId(10));
    app.board.tasks = vec![t];
    // flattened = false (default)

    let items = app.column_items_for_status(TaskStatus::Backlog);
    assert!(
        items
            .iter()
            .all(|i| !matches!(i, ColumnItem::EpicHeader(_))),
        "non-flat mode must never emit EpicHeader"
    );
}

#[test]
fn flat_view_selected_column_item_skips_headers() {
    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic(10)];
    let mut t = make_task(1, TaskStatus::Running);
    t.epic_id = Some(EpicId(10));
    app.board.tasks = vec![t];
    app.board.flattened = true;

    // Items are [SubstatusLabel, EpicHeader(10), Task(1)]. selected_row=0 should give Task(1).
    app.selection_mut().set_column(2); // Running = nav col 2
    app.selection_mut().set_row(2, 0);

    let item = app.selected_column_item();
    assert!(
        matches!(item, Some(ColumnItem::Task(t)) if t.id == TaskId(1)),
        "row 0 should resolve to Task(1), not the header"
    );
}

#[test]
fn flat_view_review_substatus_label_precedes_epic_header() {
    // Approved sorts above AwaitingReview: an approved PR is one keystroke from
    // merging, while one awaiting a decision needs nothing from anyone
    // (`ColumnSection::properties`). Lower priority number = sorts first.
    // The section header must appear BEFORE the EpicHeader in each group.
    use crate::models::{EpicId, SubStatus};
    let mut app = App::new(vec![]);
    let epic = make_epic_with_title(10, "My Epic");
    app.board.epics = vec![epic];

    let mut t1 = make_task(1, TaskStatus::Review);
    t1.epic_id = Some(EpicId(10));
    t1.sub_status = SubStatus::AwaitingReview; // sorts second

    let mut t2 = make_task(2, TaskStatus::Review);
    t2.epic_id = Some(EpicId(10));
    t2.sub_status = SubStatus::Approved; // sorts first

    app.board.tasks = vec![t1, t2];
    app.board.flattened = true;

    let items = app.column_items_for_status(TaskStatus::Review);
    // Expected: SubstatusLabel(approved), EpicHeader, Task(2),
    //           SubstatusLabel(awaiting review), EpicHeader, Task(1)
    assert_eq!(items.len(), 6, "expected 6 items, got {}", items.len());
    // The order the comment above claims, asserted rather than assumed: the
    // shape checks below hold whichever section comes first.
    assert_eq!(
        items
            .iter()
            .filter_map(|i| match i {
                ColumnItem::SubstatusLabel(h) => Some(h.section),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![
            crate::models::ColumnSection::Approved,
            crate::models::ColumnSection::AwaitingReview
        ]
    );
    assert!(
        matches!(items[0], ColumnItem::SubstatusLabel(_)),
        "items[0] must be SubstatusLabel"
    );
    assert!(
        matches!(items[1], ColumnItem::EpicHeader(_)),
        "items[1] must be EpicHeader"
    );
    assert!(
        matches!(items[2], ColumnItem::Task(_)),
        "items[2] must be Task"
    );
    assert!(
        matches!(items[3], ColumnItem::SubstatusLabel(_)),
        "items[3] must be SubstatusLabel"
    );
    assert!(
        matches!(items[4], ColumnItem::EpicHeader(_)),
        "items[4] must be EpicHeader"
    );
    assert!(
        matches!(items[5], ColumnItem::Task(_)),
        "items[5] must be Task"
    );
}

#[test]
fn flat_view_epic_repeated_across_substatus_groups() {
    // Same epic has tasks in two substatus groups.
    // EpicHeader for that epic must appear once per group (twice total).
    use crate::models::{EpicId, SubStatus};
    let mut app = App::new(vec![]);
    let epic = make_epic_with_title(10, "Shared Epic");
    app.board.epics = vec![epic];

    let mut t1 = make_task(1, TaskStatus::Running);
    t1.epic_id = Some(EpicId(10));
    t1.sub_status = SubStatus::NeedsInput; // priority 3

    let mut t2 = make_task(2, TaskStatus::Running);
    t2.epic_id = Some(EpicId(10));
    t2.sub_status = SubStatus::Active; // priority 5

    app.board.tasks = vec![t1, t2];
    app.board.flattened = true;

    let items = app.column_items_for_status(TaskStatus::Running);
    let epic_header_count = items
        .iter()
        .filter(|i| matches!(i, ColumnItem::EpicHeader(_)))
        .count();
    assert_eq!(
        epic_header_count, 2,
        "EpicHeader must appear once per substatus group; got {epic_header_count}"
    );
}

#[test]
fn flat_view_backlog_no_substatus_labels() {
    // Backlog tasks don't have meaningful substatus groups — no SubstatusLabel expected.
    use crate::models::EpicId;
    let mut app = App::new(vec![]);
    let epic = make_epic_with_title(10, "Epic");
    app.board.epics = vec![epic];

    let mut t1 = make_task(1, TaskStatus::Backlog);
    t1.epic_id = Some(EpicId(10));

    app.board.tasks = vec![t1];
    app.board.flattened = true;

    let items = app.column_items_for_status(TaskStatus::Backlog);
    assert!(
        !items
            .iter()
            .any(|i| matches!(i, ColumnItem::SubstatusLabel(_))),
        "Backlog column must not contain SubstatusLabel items"
    );
}

#[test]
fn shift_r_in_epic_view_toggles_group_by_repo() {
    let mut app = App::new(vec![]);
    let mut epic = make_epic(42);
    epic.group_by_repo = false;
    epic.feed_command = Some("echo '[]'".to_string());
    app.board.epics = vec![epic];

    // Enter epic view
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(42),
    )));

    // Press Shift+R — should return ToggleGroupByRepo command with group_by_repo = true
    let cmds = app.handle_key(make_key(KeyCode::Char('R')));
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Epic(crate::tui::commands::EpicCommand::ToggleGroupByRepo {
            id: EpicId(42),
            group_by_repo: true
        })
    )));

    // Also verify in-memory state was updated
    assert!(app.board.epics[0].group_by_repo);
}

#[test]
fn shift_r_outside_epic_view_is_noop() {
    let mut app = App::new(vec![]);
    // Default view mode is not ViewMode::Epic
    let cmds = app.handle_key(make_key(KeyCode::Char('R')));
    assert!(
        cmds.is_empty(),
        "R outside epic view should produce no commands, got {cmds:?}"
    );
}

#[test]
fn epic_view_header_shows_group_by_repo_on_for_feed_epic() {
    let mut app = App::new(vec![]);
    let mut epic = make_epic(1);
    epic.feed_command = Some("echo '[]'".to_string());
    epic.group_by_repo = true;
    app.board.epics = vec![epic];
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(1),
    )));

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "group:on [R]"),
        "Expected 'group:on [R]' in header for feed epic with group_by_repo=true"
    );
}

#[test]
fn epic_view_header_shows_group_by_repo_off_for_feed_epic() {
    let mut app = App::new(vec![]);
    let mut epic = make_epic(1);
    epic.feed_command = Some("echo '[]'".to_string());
    epic.group_by_repo = false;
    app.board.epics = vec![epic];
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(1),
    )));

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "group:off [R]"),
        "Expected 'group:off [R]' in header for feed epic with group_by_repo=false"
    );
}

#[test]
fn epic_view_header_shows_group_indicator_for_non_feed_epic() {
    let mut app = App::new(vec![]);
    let mut epic = make_epic(1);
    epic.feed_command = None;
    epic.group_by_repo = false;
    app.board.epics = vec![epic];
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(1),
    )));

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        buffer_contains(&buf, "group:off [R]"),
        "Expected 'group:off [R]' indicator in header for non-feed epic with group_by_repo=false"
    );
}

/// The append-only marker is asymmetric by design: it appears when the flag is
/// set and renders NOTHING when it is clear, so ordinary epics are not
/// cluttered with an `append:off` badge they can do nothing about
/// (epics.allium: AppendOnlyEpicIndicator).
#[test]
fn epic_header_shows_append_only_marker_when_flag_set() {
    let mut app = App::new(vec![]);
    let mut epic = make_epic(1);
    epic.feed_command = Some("echo '[]'".to_string());
    epic.feed_append_only = true;
    app.board.epics = vec![epic];
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(1),
    )));

    // Assert on the indicator row itself, not the whole buffer: a needle this
    // short could otherwise be satisfied by the hint bar.
    let buf = render_to_buffer(&mut app, 120, 30);
    let row = buffer_line(&buf, 0);
    assert!(
        row.contains("append-only"),
        "Expected 'append-only' in the epic header indicator row, got {row:?}"
    );
}

#[test]
fn epic_header_omits_append_only_marker_when_flag_clear() {
    let mut app = App::new(vec![]);
    let mut epic = make_epic(1);
    epic.feed_command = Some("echo '[]'".to_string());
    epic.feed_append_only = false;
    app.board.epics = vec![epic];
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(1),
    )));

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        !buffer_contains(&buf, "append"),
        "A mirroring epic must carry no append-only marker anywhere on screen"
    );
}

/// No bracketed key hint, unlike `[U]` and `[R]`. A bracket in this row means
/// "press this to toggle", and there is deliberately no keybinding for a flag
/// whose OFF direction can delete every accumulated task (feeds.allium:
/// AppendOnlyFeed).
#[test]
fn append_only_marker_carries_no_key_hint() {
    let mut app = App::new(vec![]);
    let mut epic = make_epic(1);
    epic.feed_command = Some("echo '[]'".to_string());
    epic.feed_append_only = true;
    app.board.epics = vec![epic];
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(1),
    )));

    let buf = render_to_buffer(&mut app, 120, 30);
    let row = buffer_line(&buf, 0);
    let (_, after) = row.split_once("append-only").expect("marker must render");
    assert!(
        !after.trim_start().starts_with('['),
        "append-only must not be followed by a key hint, got {row:?}"
    );
}

#[test]
fn append_only_marker_renders_before_the_group_indicator() {
    let mut app = App::new(vec![]);
    let mut epic = make_epic(1);
    epic.feed_command = Some("echo '[]'".to_string());
    epic.feed_append_only = true;
    app.board.epics = vec![epic];
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(1),
    )));

    let buf = render_to_buffer(&mut app, 120, 30);
    let row = buffer_line(&buf, 0);
    let marker = row.find("append-only").expect("marker must render");
    let group = row.find("group:").expect("group indicator must render");
    assert!(
        marker < group,
        "append-only must sit left of the group indicator, got {row:?}"
    );
}

/// The flag is settable on an epic with no feed command, and that combination is
/// a misconfiguration the user is better off seeing than not — so the marker
/// does not gate on `feed_command`.
#[test]
fn epic_header_shows_append_only_marker_on_non_feed_epic() {
    let mut app = App::new(vec![]);
    let mut epic = make_epic(1);
    epic.feed_command = None;
    epic.feed_append_only = true;
    app.board.epics = vec![epic];
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(1),
    )));

    let buf = render_to_buffer(&mut app, 120, 30);
    let row = buffer_line(&buf, 0);
    assert!(
        row.contains("append-only"),
        "Expected 'append-only' for a non-feed epic carrying the flag, got {row:?}"
    );
}

/// Every indicator in this row is epic-view-only. Board view describes no
/// single epic, so a set flag must not leak into it.
#[test]
fn append_only_marker_is_absent_from_board_view() {
    let mut app = App::new(vec![]);
    let mut epic = make_epic(1);
    epic.feed_command = Some("echo '[]'".to_string());
    epic.feed_append_only = true;
    app.board.epics = vec![epic];

    let buf = render_to_buffer(&mut app, 120, 30);
    assert!(
        !buffer_contains(&buf, "append"),
        "Board view must carry no append-only marker"
    );
}

/// FLATTENED is orthogonal to which view we are in (board-layout.allium: FlattenedView
/// applies inside epic view too, widening the scope to the subtree). It does
/// not change which epic the header describes, so the marker survives it.
#[test]
fn append_only_marker_survives_a_flattened_epic_view() {
    let mut app = App::new(vec![]);
    let mut epic = make_epic(1);
    epic.feed_command = Some("echo '[]'".to_string());
    epic.feed_append_only = true;
    app.board.epics = vec![epic];
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(1),
    )));
    app.board.flattened = true;

    let buf = render_to_buffer(&mut app, 120, 30);
    let row = buffer_line(&buf, 0);
    assert!(
        row.contains("append-only"),
        "Flattening an epic view must not drop the marker, got {row:?}"
    );
}

/// The colour was a deliberate choice, not a default: green is this row's
/// "feature is on" convention, and muted grey would read as "off".
#[test]
fn append_only_marker_is_green() {
    let mut app = App::new(vec![]);
    let mut epic = make_epic(1);
    epic.feed_command = Some("echo '[]'".to_string());
    epic.feed_append_only = true;
    app.board.epics = vec![epic];
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(1),
    )));

    let buf = render_to_buffer(&mut app, 120, 30);
    let style = find_style_of(&buf, "append-only").expect("marker must render");
    assert_eq!(
        style.fg,
        Some(crate::palette::GREEN),
        "append-only must render in GREEN, got {:?}",
        style.fg
    );
}

/// The other half of the ordering guarantee: the marker sits AFTER the
/// feed-role slot. Only forcible on the struct — the write path refuses the
/// pair — but worth pinning so a reorder cannot go unnoticed.
#[test]
fn append_only_marker_renders_after_the_role_indicator() {
    use crate::models::FeedRole;
    let mut app = App::new(vec![]);
    let mut epic = make_epic(1);
    epic.feed_command = Some("echo '[]'".to_string());
    epic.feed_append_only = true;
    epic.feed_role = FeedRole::MyReviews;
    app.board.epics = vec![epic];
    app.update(Message::Epic(crate::tui::messages::EpicMessage::Enter(
        EpicId(1),
    )));

    let buf = render_to_buffer(&mut app, 120, 30);
    let row = buffer_line(&buf, 0);
    let role = row.find("role:").expect("role indicator must render");
    let marker = row.find("append-only").expect("marker must render");
    assert!(
        role < marker,
        "append-only must sit right of the role indicator, got {row:?}"
    );
}

#[test]
fn flat_view_emits_orphan_separator_between_epic_and_orphan_tasks() {
    use crate::models::EpicId;
    use crate::tui::tests::helpers::make_epic_with_title;

    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic_with_title(10, "Epic A")];
    let mut t1 = make_task(1, TaskStatus::Running);
    t1.epic_id = Some(EpicId(10));
    let mut t2 = make_task(2, TaskStatus::Running);
    t2.epic_id = None; // orphan
    app.board.tasks = vec![t1, t2];
    app.board.flattened = true;

    let items = app.column_items_for_status(TaskStatus::Running);

    // Expected order: SubstatusLabel, EpicHeader, Task(epic), OrphanSeparator, Task(orphan)
    let header_pos = items
        .iter()
        .position(|i| matches!(i, ColumnItem::EpicHeader(_)));
    let epic_task_pos = items
        .iter()
        .position(|i| matches!(i, ColumnItem::Task(t) if t.id == TaskId(1)));
    let sep_pos = items
        .iter()
        .position(|i| matches!(i, ColumnItem::OrphanSeparator));
    let orphan_pos = items
        .iter()
        .position(|i| matches!(i, ColumnItem::Task(t) if t.id == TaskId(2)));
    assert!(header_pos.is_some(), "EpicHeader must be present");
    assert!(epic_task_pos.is_some(), "epic task must be present");
    assert!(sep_pos.is_some(), "OrphanSeparator must be present");
    assert!(orphan_pos.is_some(), "orphan task must be present");
    let h = header_pos.unwrap();
    let et = epic_task_pos.unwrap();
    let s = sep_pos.unwrap();
    let o = orphan_pos.unwrap();
    assert!(
        h < et && et < s && s < o,
        "order: header → epic-task → separator → orphan"
    );
}

#[test]
fn flat_view_no_orphan_separator_when_only_orphan_tasks() {
    let mut app = App::new(vec![]);
    let t1 = make_task(1, TaskStatus::Backlog);
    let t2 = make_task(2, TaskStatus::Backlog);
    // both tasks have epic_id = None (make_task default)
    app.board.tasks = vec![t1, t2];
    app.board.flattened = true;

    let items = app.column_items_for_status(TaskStatus::Backlog);

    assert!(
        !items
            .iter()
            .any(|i| matches!(i, ColumnItem::OrphanSeparator)),
        "no separator when no epic tasks precede orphans"
    );
}

#[test]
fn flat_view_orphan_separator_resets_on_substatus_boundary() {
    // Running column: epic task (NeedsInput priority), orphan (NeedsInput),
    // then orphan (Active priority). OrphanSeparator appears once at the
    // NeedsInput epic→orphan transition; the Active band starts fresh
    // (current_epic_id reset to None), so no second separator.
    use crate::models::{EpicId, SubStatus};
    use crate::tui::tests::helpers::make_epic_with_title;

    let mut app = App::new(vec![]);
    app.board.epics = vec![make_epic_with_title(10, "Epic A")];

    let mut t1 = make_task(1, TaskStatus::Running);
    t1.epic_id = Some(EpicId(10));
    t1.sub_status = SubStatus::NeedsInput;

    let mut t2 = make_task(2, TaskStatus::Running);
    t2.epic_id = None;
    t2.sub_status = SubStatus::NeedsInput;

    let mut t3 = make_task(3, TaskStatus::Running);
    t3.epic_id = None;
    t3.sub_status = SubStatus::Active;

    app.board.tasks = vec![t1, t2, t3];
    app.board.flattened = true;

    let items = app.column_items_for_status(TaskStatus::Running);

    let separator_count = items
        .iter()
        .filter(|i| matches!(i, ColumnItem::OrphanSeparator))
        .count();
    assert_eq!(
        separator_count, 1,
        "exactly one separator at the NeedsInput epic→orphan transition"
    );
}
