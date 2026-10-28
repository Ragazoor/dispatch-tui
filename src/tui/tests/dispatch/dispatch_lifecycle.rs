use super::*;

#[test]
fn dispatched_sets_fields_and_transitions_to_running() {
    let mut task = make_task(3, TaskStatus::Backlog);
    task.plan_path = Some("plan.md".into());
    let mut app = App::new(vec![task]);
    app.set_local_host_id("this-machine".to_string());
    let cmds = app.update(Message::Task(TaskMessage::Dispatched {
        id: TaskId(3),
        worktree: "/wt".to_string(),
        tmux_window: test_tmux_window("win"),
        switch_focus: false,
    }));
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(3)).unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(task.worktree.as_deref(), Some("/wt"));
    assert_eq!(task.tmux_window.as_ref().map(|w| w.as_str()), Some("win"));
    // Paired with `worktree` per core/Task's `HostTracksWorktree` invariant
    // (docs/specs/core.allium) — this write records the worktree, so it owes
    // the host (`DispatchTask` in docs/specs/dispatch.allium).
    assert_eq!(task.host.as_deref(), Some("this-machine"));
    // Stamped so ClassifyAgentActivity sees a recent PreToolUse and
    // does not flicker the freshly dispatched task into Stale.
    let stamped = task.last_pre_tool_use_at.expect("last_pre_tool_use_at set");
    assert!(
        chrono::Utc::now()
            .signed_duration_since(stamped)
            .num_seconds()
            < 5
    );
    // Only the trailing repo-sync refresh
    // (docs/specs/repo-sync.allium: RefreshRepoSyncStateAfterDispatch). No
    // Persist: since store-split M4 the module wrote the worktree, window and
    // host when the host's report landed (HostReportsDispatched), so the board
    // only follows. No SeedActivity: the claim already wrote the activity
    // stamp, so re-writing it here would be redundant — and would clobber a
    // real hook stamp if this handling ever ran later than it does.
    assert_eq!(cmds.len(), 1, "got: {cmds:?}");
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Command::Task(TaskCommand::Persist(_)))),
        "the module records the dispatch; the board writes nothing"
    );
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Command::Task(TaskCommand::SeedActivity { .. }))),
        "the claim owns the activity stamp on the dispatch path"
    );
}

/// If bootstrap's mint failed, `App.local_host_id` is left at its empty-string
/// default (see `App::local_host_id`'s doc comment). Stamping that literal
/// value as a task's host would be worse than leaving it null: once a real id
/// is later minted, the task would look permanently foreign-owned on its own
/// machine.
#[test]
fn dispatched_with_no_local_host_id_leaves_host_null() {
    let mut task = make_task(3, TaskStatus::Backlog);
    task.plan_path = Some("plan.md".into());
    let mut app = App::new(vec![task]);
    // Deliberately not calling set_local_host_id — simulating a bootstrap
    // whose mint failed.

    app.update(Message::Task(TaskMessage::Dispatched {
        id: TaskId(3),
        worktree: "/wt".to_string(),
        tmux_window: test_tmux_window("win"),
        switch_focus: false,
    }));

    let task = app.board.tasks.iter().find(|t| t.id == TaskId(3)).unwrap();
    assert_eq!(task.host, None);
}

#[test]
fn dispatched_with_switch_focus_emits_jump() {
    let mut task = make_task(3, TaskStatus::Backlog);
    task.plan_path = Some("plan.md".into());
    let mut app = App::new(vec![task]);
    let cmds = app.update(Message::Task(TaskMessage::Dispatched {
        id: TaskId(3),
        worktree: "/wt".to_string(),
        tmux_window: test_tmux_window("win"),
        switch_focus: true,
    }));
    // JumpToTmux, and the trailing repo-sync refresh.
    assert_eq!(cmds.len(), 2, "got: {cmds:?}");
    assert!(
        matches!(&cmds[0], Command::Task(TaskCommand::JumpToTmux { window }) if window == "win")
    );
}

#[test]
fn dispatched_unknown_id_is_noop() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    let cmds = app.update(Message::Task(TaskMessage::Dispatched {
        id: TaskId(999),
        worktree: "/wt".to_string(),
        tmux_window: test_tmux_window("win"),
        switch_focus: false,
    }));
    assert!(cmds.is_empty());
    assert_eq!(app.board.tasks[0].status, TaskStatus::Backlog);
}

#[test]
fn kill_and_retry_enters_confirm_mode() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)]);
    app.board.tasks[0].sub_status = SubStatus::Stale;

    app.update(Message::Task(TaskMessage::KillAndRetry(TaskId(4))));
    assert!(matches!(app.input.mode, InputMode::ConfirmRetry(TaskId(4))));
}

#[test]
fn retry_resume_emits_kill_and_resume() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)]);
    app.board.tasks[0].sub_status = SubStatus::Stale;
    // Pretend this stale task's last activity was 5 minutes ago — well past
    // ACTIVE_THRESHOLD. Without the seed in handle_retry_resume, a tick that
    // fires before the Resumed message arrives would flip the task back to
    // Stale on the basis of this old timestamp.
    app.board.tasks[0].last_pre_tool_use_at =
        Some(chrono::Utc::now() - chrono::Duration::minutes(5));
    app.input.mode = InputMode::ConfirmRetry(TaskId(4));

    let cmds = app.update(Message::Task(TaskMessage::RetryResume(TaskId(4))));

    // After retry resume, sub_status is no longer stale/crashed
    assert!(!app.is_stale(TaskId(4)));
    assert!(!app.is_crashed(TaskId(4)));
    // last_pre_tool_use_at must be seeded so the tick classifier sees a
    // fresh activity stamp through the ACTIVE_THRESHOLD window.
    let stamped = app.board.tasks[0]
        .last_pre_tool_use_at
        .expect("last_pre_tool_use_at seeded on retry resume");
    assert!(
        chrono::Utc::now()
            .signed_duration_since(stamped)
            .num_seconds()
            < 5,
        "expected fresh seed, got {stamped}"
    );
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::Task(TaskCommand::KillTmuxWindow { .. }))));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::Task(TaskCommand::Resume { .. }))));
}

#[test]
fn retry_fresh_emits_cleanup_and_dispatch() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)]);
    app.board.tasks[0].sub_status = SubStatus::Stale;
    app.input.mode = InputMode::ConfirmRetry(TaskId(4));

    let cmds = app.update(Message::Task(TaskMessage::RetryFresh(TaskId(4))));

    assert!(!app.is_stale(TaskId(4)));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert_eq!(app.board.tasks[0].status, TaskStatus::Backlog);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::Task(TaskCommand::Cleanup { .. }))));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::Task(TaskCommand::DispatchAgent { .. }))));
}

/// #4096 on the retry path: a crashed task whose worktree pointer is already
/// clear still owns a window, and re-dispatching over it must reclaim it rather
/// than leave a second window behind
/// (`TeardownIsOwedWheneverThereIsSomethingToRelease`).
#[test]
fn retry_fresh_tears_down_a_window_with_no_worktree() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)]);
    app.board.tasks[0].worktree = None;
    app.board.tasks[0].sub_status = SubStatus::Crashed;
    app.input.mode = InputMode::ConfirmRetry(TaskId(4));

    let cmds = app.update(Message::Task(TaskMessage::RetryFresh(TaskId(4))));

    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Task(TaskCommand::Cleanup {
                worktree: None,
                tmux_window: Some(w),
                ..
            }) if w == "task-4"
        )),
        "the stale window must be torn down before the re-dispatch, got: {cmds:?}"
    );
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::Task(TaskCommand::DispatchAgent { .. }))));
}

// -- host gating (task #4812 distributed-dispatch foundations) --------------
// `RetryResume`/`RetryFresh`'s `requires: task.is_locally_owned`
// (docs/specs/dispatch.allium): a task whose worktree lives on another
// machine must be refused, not acted on.

#[test]
fn retry_resume_refuses_a_foreign_owned_task() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)]);
    app.set_local_host_id("this-machine".to_string());
    app.board.tasks[0].worktree = Some("/repo/.worktrees/4-task-4".to_string());
    app.board.tasks[0].host = Some("other-machine".to_string());
    app.board.tasks[0].sub_status = SubStatus::Stale;
    app.input.mode = InputMode::ConfirmRetry(TaskId(4));

    let cmds = app.update(Message::Task(TaskMessage::RetryResume(TaskId(4))));

    assert!(
        cmds.is_empty(),
        "a foreign-owned task must not be resumed, got: {cmds:?}"
    );
    assert_eq!(
        app.board.tasks[0].tmux_window,
        Some(test_tmux_window("task-4")),
        "nothing about the task changes when the gate refuses it"
    );
}

#[test]
fn retry_resume_allows_a_task_owned_by_this_host() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)]);
    app.set_local_host_id("this-machine".to_string());
    app.board.tasks[0].worktree = Some("/repo/.worktrees/4-task-4".to_string());
    app.board.tasks[0].host = Some("this-machine".to_string());
    app.board.tasks[0].sub_status = SubStatus::Stale;
    app.input.mode = InputMode::ConfirmRetry(TaskId(4));

    let cmds = app.update(Message::Task(TaskMessage::RetryResume(TaskId(4))));

    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::Task(TaskCommand::Resume { .. }))));
}

#[test]
fn retry_fresh_refuses_a_foreign_owned_task() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)]);
    app.set_local_host_id("this-machine".to_string());
    app.board.tasks[0].worktree = Some("/repo/.worktrees/4-task-4".to_string());
    app.board.tasks[0].host = Some("other-machine".to_string());
    app.board.tasks[0].sub_status = SubStatus::Stale;
    app.input.mode = InputMode::ConfirmRetry(TaskId(4));

    let cmds = app.update(Message::Task(TaskMessage::RetryFresh(TaskId(4))));

    assert!(
        cmds.is_empty(),
        "a foreign-owned task must not be torn down and re-dispatched from here, got: {cmds:?}"
    );
    assert_eq!(
        app.board.tasks[0].status,
        TaskStatus::Running,
        "the refused task is left exactly as it was"
    );
    assert_eq!(app.board.tasks[0].host.as_deref(), Some("other-machine"));
}

/// `ResumeTask`'s `requires: task.is_locally_owned` (docs/specs/dispatch.allium),
/// gated directly in `handle_resume_task` rather than only by its one caller
/// today (`run_activation`'s priority-0 branch) — see the sibling
/// RetryResume/RetryFresh handlers, which both gate themselves too.
#[test]
fn resume_task_refuses_a_foreign_owned_task() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)]);
    app.set_local_host_id("this-machine".to_string());
    app.board.tasks[0].tmux_window = None;
    app.board.tasks[0].worktree = Some("/repo/.worktrees/4-task-4".to_string());
    app.board.tasks[0].host = Some("other-machine".to_string());

    let cmds = app.update(Message::Task(TaskMessage::Resume(TaskId(4))));

    assert!(
        cmds.is_empty(),
        "a foreign-owned task must not be resumed, got: {cmds:?}"
    );
}

#[test]
fn resume_task_allows_a_task_owned_by_this_host() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)]);
    app.set_local_host_id("this-machine".to_string());
    app.board.tasks[0].tmux_window = None;
    app.board.tasks[0].worktree = Some("/repo/.worktrees/4-task-4".to_string());
    app.board.tasks[0].host = Some("this-machine".to_string());

    let cmds = app.update(Message::Task(TaskMessage::Resume(TaskId(4))));

    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::Task(TaskCommand::Resume { .. }))));
}

/// `resume_task` (`mcp-task-tools.allium: ResumeTaskViaMcp`) reaches the same
/// launch as Space, carrying the caller's message.
#[test]
fn resume_requested_launches_the_resume_with_the_message() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)]);
    app.set_local_host_id("this-machine".to_string());
    app.board.tasks[0].tmux_window = None;
    app.board.tasks[0].worktree = Some("/repo/.worktrees/4-task-4".to_string());
    app.board.tasks[0].host = Some("this-machine".to_string());

    let cmds = app.update(Message::Task(TaskMessage::ResumeRequested {
        id: TaskId(4),
        message: Some("rebase on main".to_string()),
    }));

    match cmds.as_slice() {
        [Command::Task(TaskCommand::Resume { id, message, .. })] => {
            assert_eq!(*id, TaskId(4));
            assert_eq!(message.as_deref(), Some("rebase on main"));
        }
        other => panic!("expected one Resume command, got: {other:?}"),
    }
}

#[test]
fn resume_requested_for_a_foreign_task_says_so_instead_of_staying_silent() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)]);
    app.set_local_host_id("this-machine".to_string());
    app.board.tasks[0].tmux_window = None;
    app.board.tasks[0].worktree = Some("/repo/.worktrees/4-task-4".to_string());
    app.board.tasks[0].host = Some("other-machine".to_string());

    let cmds = app.update(Message::Task(TaskMessage::ResumeRequested {
        id: TaskId(4),
        message: None,
    }));

    assert!(cmds.is_empty());
    assert!(app
        .status_message()
        .is_some_and(|m| m.contains("another machine")));
}

#[test]
fn crashed_card_with_no_window_shows_detached_not_crashed() {
    // Detached out-prioritizes Crashed when tmux_window is None
    let mut task = make_task(1, TaskStatus::Running);
    task.sub_status = SubStatus::Crashed;
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = None;
    let mut app = App::new(vec![task]);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(buffer_contains(&buf, "○ detached"), "expected '○ detached'");
    assert!(
        !buffer_contains(&buf, "\u{26a0} crashed"),
        "should not show ⚠ crashed"
    );
}

/// `@guarantee ShownForWindowlessRunning` in docs/specs/dispatch.allium — a
/// Running task with neither worktree nor window is otherwise indistinguishable
/// from a healthy live agent.
#[test]
fn unprovisioned_running_card_shows_no_worktree() {
    let task = make_unprovisioned_task(1, TaskStatus::Running);
    let mut app = App::new(vec![task]);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(
        buffer_contains(&buf, "\u{26a0} no worktree"),
        "expected '⚠ no worktree'"
    );
    assert!(
        !buffer_contains(&buf, "\u{25c9} running"),
        "should not render as an ordinary running card"
    );
}

#[test]
fn resumed_seeds_last_pre_tool_use_at() {
    // ResumeTask spec ensures last_pre_tool_use_at = now so the resumed
    // task is classified Active until the agent's first PreToolUse hook,
    // matching DispatchTask behaviour.
    let mut t = make_task(1, TaskStatus::Running);
    t.worktree = Some("/repo/.worktrees/1-t".to_string());
    let mut app = App::new(vec![t]);
    let cmds = app.update(Message::Task(TaskMessage::Resumed {
        id: TaskId(1),
        tmux_window: test_tmux_window("task-1"),
    }));
    let task = app.board.find_task(TaskId(1)).unwrap();
    let stamp = task
        .last_pre_tool_use_at
        .expect("resume should seed last_pre_tool_use_at");
    assert!(
        chrono::Utc::now()
            .signed_duration_since(stamp)
            .num_seconds()
            < 5
    );
    assert!(matches!(&cmds[0], Command::Task(TaskCommand::Persist(_))));
}

// `TaskMessage::FinishFailed` no longer exists — the TUI wrap-up entry
// point (`W`) is gone, so the board never learns about a rebase conflict
// via a TUI message. The MCP `wrap_up` tool sets `sub_status` directly via
// `TaskService`, and the board picks it up on its next DB refresh. These
// tests set the conflict flag directly to model that refresh.

#[test]
fn conflict_flag_clears_on_dispatch() {
    let mut app = App::new(vec![{ make_task(1, TaskStatus::Review) }]);
    app.find_task_mut(TaskId(1)).unwrap().sub_status = SubStatus::Conflict;
    assert!(app
        .board
        .find_task(TaskId(1))
        .is_some_and(|t| t.sub_status == SubStatus::Conflict));

    app.update(Message::Task(TaskMessage::Resumed {
        id: TaskId(1),
        tmux_window: test_tmux_window("task-1"),
    }));
    assert!(!app
        .board
        .find_task(TaskId(1))
        .is_some_and(|t| t.sub_status == SubStatus::Conflict));
}

#[test]
fn conflict_flag_clears_on_move_backward() {
    let mut app = App::new(vec![{ make_task(1, TaskStatus::Review) }]);
    app.find_task_mut(TaskId(1)).unwrap().sub_status = SubStatus::Conflict;

    app.update(Message::Task(TaskMessage::Move {
        id: TaskId(1),
        direction: MoveDirection::Backward,
    }));
    assert!(!app
        .board
        .find_task(TaskId(1))
        .is_some_and(|t| t.sub_status == SubStatus::Conflict));
}

#[test]
fn manual_move_review_to_running_seeds_last_pre_tool_use_at() {
    let mut task = make_task(1, TaskStatus::Review);
    task.last_pre_tool_use_at = None;
    task.last_notification_at = None;
    task.sub_status = SubStatus::AwaitingReview;
    let mut app = App::new(vec![task]);

    let cmds = app.update(Message::Task(TaskMessage::Move {
        id: TaskId(1),
        direction: MoveDirection::Backward,
    }));

    let t = app.board.find_task(TaskId(1)).expect("task");
    assert_eq!(t.status, TaskStatus::Running);
    assert!(t.last_pre_tool_use_at.is_some(), "seed missing");
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Task(TaskCommand::SeedActivity { id: TaskId(1), .. })
        )),
        "no SeedActivity in {cmds:?}",
    );
}
