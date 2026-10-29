use super::*;
use crate::models::test_pane_id;

#[test]
fn w_key_on_non_review_task_is_noop() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);

    app.handle_key(make_key(KeyCode::Char('W')));
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn tick_skips_already_stale_tasks() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)]);
    app.board.tasks[0].tmux_window = Some(test_tmux_window("win-3"));
    app.board.tasks[0].sub_status = SubStatus::Stale;

    let cmds = app.update(Message::System(crate::tui::messages::SystemMessage::Tick));
    // Tick should NOT re-emit PersistTask for already-stale tasks
    // (only CheckWindow and RefreshFromDb expected)
    assert!(!cmds.iter().any(|c| matches!(
        c,
        Command::Task(crate::tui::commands::TaskCommand::Persist(_))
    )));
}

#[test]
fn tick_skips_already_crashed_tasks() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)]);
    app.board.tasks[0].tmux_window = Some(test_tmux_window("win-3"));
    app.board.tasks[0].sub_status = SubStatus::Crashed;

    let cmds = app.update(Message::System(crate::tui::messages::SystemMessage::Tick));
    assert!(!cmds.iter().any(|c| matches!(
        c,
        Command::Task(crate::tui::commands::TaskCommand::Persist(_))
    )));
}

#[test]
fn tick_skips_conflict_tasks_for_stale_detection() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)]);
    app.board.tasks[0].tmux_window = Some(test_tmux_window("win-3"));
    app.board.tasks[0].sub_status = SubStatus::Conflict;

    let cmds = app.update(Message::System(crate::tui::messages::SystemMessage::Tick));
    assert!(!cmds.iter().any(|c| matches!(
        c,
        Command::Task(crate::tui::commands::TaskCommand::Persist(_))
    )));
    assert_eq!(app.board.tasks[0].sub_status, SubStatus::Conflict);
}

#[test]
fn move_task_forward_resets_substatus() {
    let mut app = make_app();
    let id = TaskId(3); // Running
    app.find_task_mut(id).unwrap().sub_status = SubStatus::Stale;
    app.update(Message::Task(crate::tui::messages::TaskMessage::Move {
        id,
        direction: MoveDirection::Forward,
    }));
    let task = app.board.find_task(id).unwrap();
    assert_eq!(task.status, TaskStatus::Review);
    assert_eq!(task.sub_status, SubStatus::AwaitingReview);
}

#[test]
fn move_task_backward_resets_substatus() {
    let mut app = make_app();
    let id = TaskId(3); // Running
    app.update(Message::Task(crate::tui::messages::TaskMessage::Move {
        id,
        direction: MoveDirection::Backward,
    }));
    let task = app.board.find_task(id).unwrap();
    assert_eq!(task.status, TaskStatus::Backlog);
    assert_eq!(task.sub_status, SubStatus::None);
}

#[test]
fn shift_l_with_mixed_selection_moves_tasks_only() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.epics = vec![make_epic(10)];
    app.update(Message::Task(
        crate::tui::messages::TaskMessage::ToggleSelect(TaskId(1)),
    ));
    app.update(Message::Epic(
        crate::tui::messages::EpicMessage::ToggleSelect(EpicId(10)),
    ));
    // Cursor on the task (row 0) so 'm' triggers batch move, not epic move
    app.selection_mut().set_column(1); // Backlog = nav col 1
    app.selection_mut().set_row(1, 0);

    app.handle_key(make_key(KeyCode::Char('L')));
    // Task should move forward
    assert_eq!(
        app.board.find_task(TaskId(1)).unwrap().status,
        TaskStatus::Running
    );
}

#[test]
fn detach_tmux_single_sets_confirm_mode() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Review)]);

    app.update(Message::Task(
        crate::tui::messages::TaskMessage::DetachTmux(TaskId(1)),
    ));

    assert!(
        matches!(&app.input.mode, InputMode::ConfirmDetachTmux(ids) if ids == &[TaskId(1)]),
        "Expected ConfirmDetachTmux([1]), got {:?}",
        app.input.mode
    );
    assert!(app.status.message.is_some());
}

#[test]
fn detach_tmux_running_task_with_window_is_detachable() {
    // Running tasks with a tmux window should pass the detach filter.
    let mut app = App::new(vec![make_task(1, TaskStatus::Running)]);

    app.update(Message::Task(
        crate::tui::messages::TaskMessage::DetachTmux(TaskId(1)),
    ));

    assert!(
        matches!(&app.input.mode, InputMode::ConfirmDetachTmux(ids) if ids == &[TaskId(1)]),
        "Running task with window should set ConfirmDetachTmux, got {:?}",
        app.input.mode
    );
}

#[test]
fn detach_tmux_noop_on_task_without_window() {
    let mut task = make_task(1, TaskStatus::Review);
    task.tmux_window = None;
    let mut app = App::new(vec![task]);

    let cmds = app.update(Message::Task(
        crate::tui::messages::TaskMessage::DetachTmux(TaskId(1)),
    ));

    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds.is_empty(), "should produce no commands");
}

#[test]
fn batch_detach_tmux() {
    let mut app = App::new(vec![
        make_task(1, TaskStatus::Review),
        make_task(2, TaskStatus::Review),
    ]);
    app.board.tasks[0].tmux_window = Some(test_tmux_window("task-1"));
    app.board.tasks[1].tmux_window = Some(test_tmux_window("task-2"));

    app.update(Message::Task(
        crate::tui::messages::TaskMessage::BatchDetachTmux(vec![TaskId(1), TaskId(2)]),
    ));
    let cmds = app.update(Message::Input(
        crate::tui::messages::InputMessage::ConfirmDetachTmux,
    ));

    assert!(
        app.board.tasks[0].tmux_window.is_none(),
        "task 1 window should be cleared"
    );
    assert!(
        app.board.tasks[1].tmux_window.is_none(),
        "task 2 window should be cleared"
    );

    let kill_count = cmds
        .iter()
        .filter(|c| {
            matches!(
                c,
                Command::Task(crate::tui::commands::TaskCommand::KillTmuxWindow { .. })
            )
        })
        .count();
    assert_eq!(kill_count, 2, "should kill 2 windows");

    let persist_count = cmds
        .iter()
        .filter(|c| {
            matches!(
                c,
                Command::Task(crate::tui::commands::TaskCommand::Persist(_))
            )
        })
        .count();
    assert_eq!(persist_count, 2, "should persist 2 tasks");
}

#[test]
fn move_repo_cursor_down_wraps() {
    let mut app = App::new(vec![]);
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    app.input.repo_cursor = 1; // last
    app.handle_key(make_key(KeyCode::Down));
    assert_eq!(app.input.repo_cursor, 0, "should wrap to first");
}

#[test]
fn move_repo_cursor_up_wraps() {
    let mut app = App::new(vec![]);
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    app.input.repo_cursor = 0; // first
    app.handle_key(make_key(KeyCode::Up));
    assert_eq!(app.input.repo_cursor, 1, "should wrap to last");
}

#[test]
fn repo_cursor_resets_on_quick_dispatch_entry() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string()];
    app.input.repo_cursor = 1;
    app.update(Message::Input(
        crate::tui::messages::InputMessage::StartQuickDispatchSelection,
    ));
    assert_eq!(
        app.input.repo_cursor, 0,
        "cursor should reset to 0 on mode entry"
    );
}

#[test]
fn detached_review_task_without_a_pr_shows_parked_header() {
    let mut task = make_task(1, TaskStatus::Review);
    task.sub_status = SubStatus::AwaitingReview;
    task.url = None;
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = None; // detached
    let mut app = App::new(vec![task]);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "parked"),
        "detached review task with no PR should show 'parked' section header"
    );
}

/// Detach state stops mattering once a PR exists — there is no longer a separate
/// "awaiting merge" section.
#[test]
fn detached_review_task_with_a_pr_shows_awaiting_review_header() {
    let mut task = make_task(1, TaskStatus::Review);
    task.sub_status = SubStatus::AwaitingReview;
    task.url = Some(crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/10",
        crate::models::UrlType::Pr,
    ));
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = None; // detached
    let mut app = App::new(vec![task]);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "awaiting review"),
        "detached review task with a PR should show 'awaiting review'"
    );
    assert!(
        !buffer_contains(&buf, "parked"),
        "detached review task with a PR should not be parked"
    );
}

#[test]
fn live_review_task_shows_awaiting_review_header() {
    let mut task = make_task(1, TaskStatus::Review);
    task.sub_status = SubStatus::AwaitingReview;
    task.url = Some(crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/10",
        crate::models::UrlType::Pr,
    ));
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = Some(test_tmux_window("1-fix")); // live
    let mut app = App::new(vec![task]);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "awaiting review"),
        "live review task should show 'awaiting review' section header"
    );
    assert!(
        !buffer_contains(&buf, "parked"),
        "live review task should not show 'parked'"
    );
}

#[test]
fn parked_and_awaiting_review_tasks_get_separate_sections() {
    // Has a PR — awaiting review, regardless of detach state.
    let mut with_pr = make_task(1, TaskStatus::Review);
    with_pr.sub_status = SubStatus::AwaitingReview;
    with_pr.url = Some(crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/10",
        crate::models::UrlType::Pr,
    ));
    with_pr.worktree = Some("/repo/.worktrees/1-fix".to_string());
    with_pr.tmux_window = Some(test_tmux_window("1-fix"));

    // No PR and detached — parked.
    let mut parked = make_task(2, TaskStatus::Review);
    parked.sub_status = SubStatus::AwaitingReview;
    parked.url = None;
    parked.worktree = Some("/repo/.worktrees/2-feat".to_string());
    parked.tmux_window = None;

    let mut app = App::new(vec![with_pr, parked]);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "awaiting review"),
        "should show 'awaiting review' for the task with a PR"
    );
    assert!(
        buffer_contains(&buf, "parked"),
        "should show 'parked' for the detached task with no PR"
    );
}

/// A review decision *I* made on someone else's PR is a different section from
/// the same decision made on mine, and both sort below the sections that still
/// need me. Reads the rendered section order straight off the buffer rather than
/// searching for one label at a time: "changes requested" and "approved" are
/// prefixes of their own by-me labels, so a first-substring-match assertion
/// passes for the wrong reason when the order is broken.
#[test]
fn review_decisions_i_made_render_as_their_own_sections_at_the_bottom() {
    let review_task = |id: i64, sub_status: SubStatus, tag: crate::models::TaskTag| {
        let mut t = make_task(id, TaskStatus::Review);
        t.sub_status = sub_status;
        t.tag = Some(tag);
        t.url = Some(crate::models::TaskUrl::new(
            format!("https://github.com/org/repo/pull/{id}"),
            crate::models::UrlType::Pr,
        ));
        t
    };

    let tasks = vec![
        // On my own PRs — I must fix, then I can merge.
        review_task(
            1,
            SubStatus::ChangesRequested,
            crate::models::TaskTag::Feature,
        ),
        review_task(2, SubStatus::Approved, crate::models::TaskTag::Feature),
        // On someone else's — the ball is with them, or nowhere at all.
        review_task(
            3,
            SubStatus::ChangesRequested,
            crate::models::TaskTag::PrReview,
        ),
        review_task(4, SubStatus::Approved, crate::models::TaskTag::PrReview),
    ];

    let mut app = App::new(tasks);
    let buf = render_to_buffer(&mut app, 120, 24);

    // Longest label first so "approved by me" never reads as "approved".
    let labels = [
        "changes requested by me",
        "approved by me",
        "changes requested",
        "approved",
    ];
    let area = buf.area();
    let rendered: Vec<&str> = (area.top()..area.bottom())
        .filter_map(|y| {
            let line = buffer_line(&buf, y);
            labels.into_iter().find(|l| line.contains(l))
        })
        .collect();

    assert_eq!(
        rendered,
        vec![
            "changes requested",
            "approved",
            "changes requested by me",
            "approved by me",
        ],
        "unexpected Review section order"
    );
}

#[test]
fn is_detached_returns_true_for_review_without_window() {
    let mut task = make_task(1, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = None;
    assert!(task.is_detached());
}

#[test]
fn is_detached_returns_false_with_window() {
    let mut task = make_task(1, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = Some(test_tmux_window("1-fix"));
    assert!(!task.is_detached());
}

/// `is_detached` no longer special-cases conflict — callers that need to
/// treat conflict separately (e.g. card rendering) check `sub_status` first.
#[test]
fn is_detached_returns_true_for_conflict() {
    let mut task = make_task(1, TaskStatus::Running);
    task.sub_status = SubStatus::Conflict;
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = None;
    assert!(task.is_detached());
}

#[test]
fn is_unprovisioned_is_mutually_exclusive_with_is_detached() {
    for status in [TaskStatus::Running, TaskStatus::Review] {
        let mut task = make_task(1, status);
        task.worktree = None;
        task.tmux_window = None;
        assert!(task.is_unprovisioned(), "{status:?} with nothing behind it");
        assert!(!task.is_detached(), "the two are mutually exclusive");

        task.worktree = Some("/repo/.worktrees/1-fix".to_string());
        assert!(!task.is_unprovisioned(), "a worktree means resumable");
        assert!(task.is_detached());
    }
}

#[test]
fn is_unprovisioned_is_false_for_non_live_statuses() {
    for status in [TaskStatus::Backlog, TaskStatus::Done] {
        let mut task = make_task(1, status);
        task.worktree = None;
        task.tmux_window = None;
        assert!(!task.is_unprovisioned(), "{status:?} is not a live status");
    }
}

#[test]
fn mark_dispatching_sets_guard_and_returns_no_commands() {
    let mut app = make_app();
    assert!(!app.is_dispatching(TaskId(1)));
    let cmds = app.update(Message::Task(
        crate::tui::messages::TaskMessage::MarkDispatching(TaskId(1)),
    ));
    assert!(cmds.is_empty());
    assert!(app.is_dispatching(TaskId(1)));
}

#[test]
fn tick_with_active_split_checks_pane() {
    let mut app = make_app();
    app.board.split.active = true;
    app.board.split.right_pane_id = Some(test_pane_id("%42"));
    let cmds = app.update(Message::System(crate::tui::messages::SystemMessage::Tick));
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Split(crate::tui::commands::SplitCommand::CheckPaneExists { pane_id }) if pane_id == "%42"
    )));
}

#[test]
fn tick_without_split_does_not_check_pane() {
    let mut app = make_app();
    let cmds = app.update(Message::System(crate::tui::messages::SystemMessage::Tick));
    assert!(!cmds.iter().any(|c| matches!(
        c,
        Command::Split(crate::tui::commands::SplitCommand::CheckPaneExists { .. })
    )));
}

#[test]
fn tick_skips_capture_for_split_pinned_task() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)]);

    // Pin task 4 in split mode
    app.board.split.active = true;
    app.board.split.right_pane_id = Some(test_pane_id("%42"));
    app.board.split.pinned_task_id = Some(TaskId(4));

    let cmds = app.update(Message::System(crate::tui::messages::SystemMessage::Tick));

    // Should NOT emit CheckWindow for the pinned task (its window is a pane now)
    assert!(
        !cmds.iter().any(|c| matches!(
            c,
            Command::Task(crate::tui::commands::TaskCommand::CheckWindow { id: TaskId(4), .. })
        )),
        "split-pinned task should be excluded from CheckWindow"
    );
}
