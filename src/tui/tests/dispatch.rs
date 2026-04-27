#![allow(unused_imports)]

use super::*;
use crate::models::{
    DispatchMode, Epic, EpicId, SubStatus, TaskId, TaskStatus, TaskTag, DEFAULT_QUICK_TASK_TITLE,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Modifier},
    Terminal,
};
use std::time::{Duration, Instant};

#[test]
fn dispatch_only_backlog_tasks() {
    let mut app = make_app();

    // Task 1 is Backlog — should dispatch
    let cmds = app.update(Message::DispatchTask(TaskId(1), DispatchMode::Dispatch));
    assert!(matches!(cmds[0], Command::DispatchAgent { .. }));

    // Task 3 is Running — should not dispatch
    let cmds = app.update(Message::DispatchTask(TaskId(3), DispatchMode::Dispatch));
    assert!(cmds.is_empty());

    // Task 4 is Done — should not dispatch
    let cmds = app.update(Message::DispatchTask(TaskId(4), DispatchMode::Dispatch));
    assert!(cmds.is_empty());
}

#[test]
fn tick_captures_review_task_with_live_window() {
    let mut task = make_task(5, TaskStatus::Review);
    task.tmux_window = Some("task-5".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    let cmds = app.update(Message::Tick);

    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::CaptureTmux { id: TaskId(5), .. })));
}

#[test]
fn dispatch_from_running_is_noop() {
    let mut task = make_task(4, TaskStatus::Running);
    task.worktree = Some("/repo/.worktrees/4-task-4".to_string());
    task.tmux_window = Some("task-4".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    let cmds = app.update(Message::DispatchTask(TaskId(4), DispatchMode::Dispatch));
    assert!(cmds.is_empty());
}

#[test]
fn dispatch_from_review_is_noop() {
    let mut task = make_task(5, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/5-task-5".to_string());
    task.tmux_window = Some("task-5".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    let cmds = app.update(Message::DispatchTask(TaskId(5), DispatchMode::Dispatch));
    assert!(cmds.is_empty());
}

#[test]
fn d_key_on_backlog_with_plan_dispatches() {
    let mut task = make_task(3, TaskStatus::Backlog);
    task.plan_path = Some("plan.md".into());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.selection_mut().set_column(0); // Backlog column
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(matches!(&cmds[0], Command::DispatchAgent { .. }));
}

#[test]
fn d_key_on_running_with_window_shows_warning() {
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some("task-4".to_string());
    task.worktree = Some("/repo/.worktrees/4-task-4".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.selection_mut().set_column(1); // Running column
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(cmds.is_empty());
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap()
        .contains("already running"));
}

#[test]
fn d_key_on_running_no_window_resumes() {
    let mut task = make_task(4, TaskStatus::Running);
    task.worktree = Some("/repo/.worktrees/4-task-4".to_string());
    task.tmux_window = None;
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.selection_mut().set_column(1); // Running column
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(matches!(&cmds[0], Command::Resume { .. }));
}

#[test]
fn d_key_on_backlog_brainstorms() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.tag = Some(TaskTag::Epic); // tag=epic triggers brainstorm
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.selection_mut().set_column(0); // Backlog column
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::DispatchAgent { task, mode: DispatchMode::Brainstorm } if task.id == TaskId(1))
    );
}

#[test]
fn d_key_on_done_shows_warning() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Done)], TEST_TIMEOUT);
    app.selection_mut().set_column(3); // Done column
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(cmds.is_empty());
    assert!(app.status.message.is_some());
}

#[test]
fn d_key_on_running_no_worktree_no_window_shows_warning() {
    let mut task = make_task(4, TaskStatus::Running);
    task.worktree = None;
    task.tmux_window = None;
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.selection_mut().set_column(1); // Running column
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(cmds.is_empty());
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap()
        .contains("No worktree"));
}

#[test]
fn brainstorm_only_backlog_tasks() {
    let mut app = make_app();

    // Task 1 is Backlog — should brainstorm
    let cmds = app.update(Message::DispatchTask(TaskId(1), DispatchMode::Brainstorm));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::DispatchAgent { task, mode: DispatchMode::Brainstorm } if task.id == TaskId(1))
    );

    // Task 3 is Running — should not brainstorm
    let cmds = app.update(Message::DispatchTask(TaskId(3), DispatchMode::Brainstorm));
    assert!(cmds.is_empty());

    // Task 4 is Done — should not brainstorm
    let cmds = app.update(Message::DispatchTask(TaskId(4), DispatchMode::Brainstorm));
    assert!(cmds.is_empty());
}

#[test]
fn shift_d_with_one_repo_emits_quick_dispatch() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo".to_string()];
    let cmds = app.handle_key(make_shift_key(KeyCode::Char('D')));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::QuickDispatch { ref draft, epic_id: None } if draft.repo_path == "/repo")
    );
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn shift_d_with_no_repos_shows_error() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.repo_paths = vec![];
    let cmds = app.handle_key(make_shift_key(KeyCode::Char('D')));
    assert!(cmds.is_empty());
    assert!(app.status.message.is_some());
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn shift_d_with_multiple_repos_enters_quick_dispatch_mode() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo1".to_string(), "/repo2".to_string()];
    let cmds = app.handle_key(make_shift_key(KeyCode::Char('D')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::QuickDispatch);
}

#[test]
fn quick_dispatch_mode_number_selects_repo() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo1".to_string(), "/repo2".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    let cmds = app.handle_key(make_key(KeyCode::Char('2')));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::QuickDispatch { ref draft, epic_id: None } if draft.repo_path == "/repo2")
    );
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn quick_dispatch_mode_esc_cancels() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo1".to_string(), "/repo2".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    let cmds = app.handle_key(make_key(KeyCode::Esc));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn quick_dispatch_mode_invalid_number_is_noop() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo1".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    let cmds = app.handle_key(make_key(KeyCode::Char('3')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::QuickDispatch);
}

#[test]
fn quick_dispatch_message_emits_command() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let cmds = app.update(Message::QuickDispatch {
        repo_path: "/my/repo".to_string(),
        epic_id: None,
    });
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::QuickDispatch { ref draft, epic_id: None }
        if draft.title == DEFAULT_QUICK_TASK_TITLE && draft.repo_path == "/my/repo")
    );
}

#[test]
fn shift_d_in_epic_view_quick_dispatches_subtask_single_repo() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let epic = make_epic(10);
    app.board.epics = vec![epic];
    app.board.repo_paths = vec!["/my/repo".to_string()];
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };
    let cmds = app.handle_key(make_shift_key(KeyCode::Char('D')));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0],
        Command::QuickDispatch { ref draft, epic_id: Some(EpicId(10)) }
        if draft.repo_path == "/my/repo"
    ));
}

#[test]
fn shift_d_in_epic_view_shows_repo_selection_with_multiple_repos() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let epic = make_epic(10);
    app.board.epics = vec![epic];
    app.board.repo_paths = vec!["/repo/a".to_string(), "/repo/b".to_string()];
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };
    let cmds = app.handle_key(make_shift_key(KeyCode::Char('D')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::QuickDispatch);
    assert_eq!(app.input.pending_epic_id, Some(EpicId(10)));
}

#[test]
fn shift_d_in_epic_view_repo_selection_dispatches_with_epic_id() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let epic = make_epic(10);
    app.board.epics = vec![epic];
    app.board.repo_paths = vec!["/repo/a".to_string(), "/repo/b".to_string()];
    app.board.view_mode = ViewMode::Epic {
        epic_id: EpicId(10),
        selection: BoardSelection::new(),
        parent: Box::new(ViewMode::Board(BoardSelection::new())),
    };
    // Enter selection mode
    app.handle_key(make_shift_key(KeyCode::Char('D')));
    // Select second repo
    let cmds = app.handle_key(make_key(KeyCode::Char('2')));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0],
        Command::QuickDispatch { ref draft, epic_id: Some(EpicId(10)) }
        if draft.repo_path == "/repo/b"
    ));
}

#[test]
fn stale_agent_detected_after_timeout() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());
    app.agents
        .last_active_at
        .insert(TaskId(4), Instant::now() - Duration::from_secs(301));

    let cmds = app.update(Message::Tick);
    assert!(app.is_stale(TaskId(4)));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::CaptureTmux { id: TaskId(4), .. })));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistTask(t) if t.id == TaskId(4))));
}

#[test]
fn window_gone_on_running_task_marks_crashed() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());

    let cmds = app.update(Message::WindowGone(TaskId(4)));
    assert!(app.is_crashed(TaskId(4)));
    // tmux_window should be cleared — the window is gone by definition
    assert!(app.board.tasks[0].tmux_window.is_none());
    // Should emit PersistTask with cleared tmux_window
    assert!(cmds.iter().any(
        |c| matches!(c, Command::PersistTask(t) if t.id == TaskId(4) && t.tmux_window.is_none())
    ));
}

#[test]
fn window_gone_on_review_task_clears_window() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Review)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());

    let cmds = app.update(Message::WindowGone(TaskId(4)));
    assert!(!app.is_crashed(TaskId(4)));
    assert!(app.board.tasks[0].tmux_window.is_none());
    assert!(matches!(&cmds[0], Command::PersistTask(_)));
}

#[test]
fn tmux_output_change_resets_staleness_timer() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());
    app.agents
        .last_active_at
        .insert(TaskId(4), Instant::now() - Duration::from_secs(301));
    app.agents.prev_tmux_activity.insert(TaskId(4), 1000);

    app.update(Message::TmuxOutput {
        id: TaskId(4),
        output: "output".to_string(),
        activity_ts: 1001,
    });
    let elapsed = app.agents.last_active_at[&TaskId(4)].elapsed();
    assert!(elapsed < Duration::from_secs(1));
}

#[test]
fn tmux_output_same_activity_does_not_reset_timer() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());
    let old_instant = Instant::now() - Duration::from_secs(200);
    app.agents.last_active_at.insert(TaskId(4), old_instant);
    app.agents.prev_tmux_activity.insert(TaskId(4), 1000);

    app.update(Message::TmuxOutput {
        id: TaskId(4),
        output: "output".to_string(),
        activity_ts: 1000,
    });
    let elapsed = app.agents.last_active_at[&TaskId(4)].elapsed();
    assert!(elapsed >= Duration::from_secs(199));
}

#[test]
fn activity_ts_change_with_same_output_resets_timer() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());
    app.agents
        .last_active_at
        .insert(TaskId(4), Instant::now() - Duration::from_secs(301));
    app.agents.prev_tmux_activity.insert(TaskId(4), 1000);
    app.agents
        .tmux_outputs
        .insert(TaskId(4), "same output".to_string());

    // Same display text, but tmux reports new activity
    app.update(Message::TmuxOutput {
        id: TaskId(4),
        output: "same output".to_string(),
        activity_ts: 1001,
    });
    let elapsed = app.agents.last_active_at[&TaskId(4)].elapsed();
    assert!(elapsed < Duration::from_secs(1));
}

#[test]
fn activity_ts_same_with_different_output_no_reset() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());
    let old_instant = Instant::now() - Duration::from_secs(200);
    app.agents.last_active_at.insert(TaskId(4), old_instant);
    app.agents.prev_tmux_activity.insert(TaskId(4), 1000);
    app.agents
        .tmux_outputs
        .insert(TaskId(4), "old text".to_string());

    // Different display text, but same activity timestamp
    app.update(Message::TmuxOutput {
        id: TaskId(4),
        output: "new text".to_string(),
        activity_ts: 1000,
    });
    let elapsed = app.agents.last_active_at[&TaskId(4)].elapsed();
    assert!(elapsed >= Duration::from_secs(199));
    // Display output is still updated for rendering
    assert_eq!(app.agents.tmux_outputs.get(&TaskId(4)).unwrap(), "new text");
}

#[test]
fn dispatched_sets_fields_and_transitions_to_running() {
    let mut task = make_task(3, TaskStatus::Backlog);
    task.plan_path = Some("plan.md".into());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    let cmds = app.update(Message::Dispatched {
        id: TaskId(3),
        worktree: "/wt".to_string(),
        tmux_window: "win".to_string(),
        switch_focus: false,
    });
    let task = app.board.tasks.iter().find(|t| t.id == TaskId(3)).unwrap();
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(task.worktree.as_deref(), Some("/wt"));
    assert_eq!(task.tmux_window.as_deref(), Some("win"));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(&cmds[0], Command::PersistTask(_)));
}

#[test]
fn dispatched_with_switch_focus_emits_jump() {
    let mut task = make_task(3, TaskStatus::Backlog);
    task.plan_path = Some("plan.md".into());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    let cmds = app.update(Message::Dispatched {
        id: TaskId(3),
        worktree: "/wt".to_string(),
        tmux_window: "win".to_string(),
        switch_focus: true,
    });
    assert_eq!(cmds.len(), 2);
    assert!(matches!(&cmds[0], Command::PersistTask(_)));
    assert!(matches!(&cmds[1], Command::JumpToTmux { window } if window == "win"));
}

#[test]
fn dispatched_unknown_id_is_noop() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    let cmds = app.update(Message::Dispatched {
        id: TaskId(999),
        worktree: "/wt".to_string(),
        tmux_window: "win".to_string(),
        switch_focus: false,
    });
    assert!(cmds.is_empty());
    assert_eq!(app.board.tasks[0].status, TaskStatus::Backlog);
}

#[test]
fn tmux_output_stores_in_map() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Running)], TEST_TIMEOUT);
    let cmds = app.update(Message::TmuxOutput {
        id: TaskId(1),
        output: "hello".to_string(),
        activity_ts: 1000,
    });
    assert_eq!(app.agents.tmux_outputs.get(&TaskId(1)).unwrap(), "hello");
    assert!(cmds.is_empty());
}

#[test]
fn tmux_output_overwrites_previous() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Running)], TEST_TIMEOUT);
    app.update(Message::TmuxOutput {
        id: TaskId(1),
        output: "first".to_string(),
        activity_ts: 1000,
    });
    app.update(Message::TmuxOutput {
        id: TaskId(1),
        output: "second".to_string(),
        activity_ts: 1001,
    });
    assert_eq!(app.agents.tmux_outputs.get(&TaskId(1)).unwrap(), "second");
}

#[test]
fn d_key_on_review_with_window_shows_warning() {
    let mut task = make_task(5, TaskStatus::Review);
    task.tmux_window = Some("task-5".to_string());
    task.worktree = Some("/repo/.worktrees/5-task-5".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.selection_mut().set_column(2); // Review column
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(cmds.is_empty());
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap()
        .contains("already running"));
}

#[test]
fn d_key_on_review_no_window_with_worktree_resumes() {
    let mut task = make_task(5, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/5-task-5".to_string());
    task.tmux_window = None;
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.selection_mut().set_column(2); // Review column
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(matches!(&cmds[0], Command::Resume { .. }));
}

#[test]
fn d_key_on_review_no_worktree_no_window_shows_warning() {
    let mut task = make_task(5, TaskStatus::Review);
    task.worktree = None;
    task.tmux_window = None;
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.selection_mut().set_column(2); // Review column
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(cmds.is_empty());
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap()
        .contains("No worktree"));
}

#[test]
fn d_key_on_empty_column_is_noop() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.selection_mut().set_column(0);
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(cmds.is_empty());
}

#[test]
fn new_app_has_empty_agent_tracking() {
    let app = App::new(vec![], TEST_TIMEOUT);
    // stale/crashed state is now on the task's sub_status field, not in AgentTracking
    assert!(app.agents.prev_tmux_activity.is_empty());
}

#[test]
fn kill_and_retry_enters_confirm_mode() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());
    app.board.tasks[0].sub_status = SubStatus::Stale;

    app.update(Message::KillAndRetry(TaskId(4)));
    assert!(matches!(app.input.mode, InputMode::ConfirmRetry(TaskId(4))));
}

#[test]
fn retry_resume_emits_kill_and_resume() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());
    app.board.tasks[0].worktree = Some("/repo/.worktrees/4-task-4".to_string());
    app.board.tasks[0].sub_status = SubStatus::Stale;
    app.input.mode = InputMode::ConfirmRetry(TaskId(4));

    let cmds = app.update(Message::RetryResume(TaskId(4)));

    // After retry resume, sub_status is no longer stale/crashed
    assert!(!app.is_stale(TaskId(4)));
    assert!(!app.is_crashed(TaskId(4)));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::KillTmuxWindow { .. })));
    assert!(cmds.iter().any(|c| matches!(c, Command::Resume { .. })));
}

#[test]
fn retry_fresh_emits_cleanup_and_dispatch() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());
    app.board.tasks[0].worktree = Some("/repo/.worktrees/4-task-4".to_string());
    app.board.tasks[0].sub_status = SubStatus::Stale;
    app.input.mode = InputMode::ConfirmRetry(TaskId(4));

    let cmds = app.update(Message::RetryFresh(TaskId(4)));

    assert!(!app.is_stale(TaskId(4)));
    assert_eq!(app.input.mode, InputMode::Normal);
    assert_eq!(app.board.tasks[0].status, TaskStatus::Backlog);
    assert!(cmds.iter().any(|c| matches!(c, Command::Cleanup { .. })));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchAgent { .. })));
}

#[test]
fn d_key_on_stale_running_task_enters_retry_mode() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());
    app.board.tasks[0].sub_status = SubStatus::Stale;
    // Navigate to Running column (index 1)
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    app.handle_key(make_key(KeyCode::Char('d')));
    assert!(matches!(app.input.mode, InputMode::ConfirmRetry(TaskId(4))));
}

#[test]
fn d_key_on_crashed_running_task_enters_retry_mode() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());
    app.board.tasks[0].sub_status = SubStatus::Crashed;
    // Navigate to Running column (index 1)
    app.selection_mut().set_column(1);
    app.selection_mut().set_row(1, 0);

    app.handle_key(make_key(KeyCode::Char('d')));
    assert!(matches!(app.input.mode, InputMode::ConfirmRetry(TaskId(4))));
}

#[test]
fn crashed_card_with_no_window_shows_detached_not_crashed() {
    // Detached out-prioritizes Crashed when tmux_window is None
    let mut task = make_task(1, TaskStatus::Running);
    task.sub_status = SubStatus::Crashed;
    task.worktree = Some("/repo/.worktrees/1-fix".to_string());
    task.tmux_window = None;
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    let buf = render_to_buffer(&mut app, 120, 20);
    assert!(buffer_contains(&buf, "○ detached"), "expected '○ detached'");
    assert!(
        !buffer_contains(&buf, "\u{26a0} crashed"),
        "should not show ⚠ crashed"
    );
}

#[test]
fn d_key_on_backlog_epic_dispatches_epic() {
    let mut app = make_app_with_epic_selected(); // epic at row 1 in Backlog
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(cmds[0], Command::DispatchEpic { ref epic } if epic.id == EpicId(10)));
}

#[test]
fn d_key_in_epic_view_with_no_subtasks_dispatches_epic() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let epic = make_epic(10);
    app.board.epics = vec![epic];
    app.update(Message::EnterEpic(EpicId(10)));

    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchEpic { ref epic } if epic.id == EpicId(10))));
}

#[test]
fn dispatch_epic_on_backlog_epic_produces_command() {
    let mut app = make_app_with_epic_selected(); // epic at row 1, Backlog column
    let cmds = app.update(Message::DispatchEpic(EpicId(10)));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(cmds[0], Command::DispatchEpic { ref epic } if epic.id == EpicId(10)));
}

#[test]
fn dispatch_epic_on_non_backlog_shows_status() {
    let mut app = App::new(
        vec![{
            let mut t = make_task(1, TaskStatus::Running);
            t.epic_id = Some(EpicId(10));
            t
        }],
        TEST_TIMEOUT,
    );
    let mut epic = make_epic(10);
    epic.status = TaskStatus::Running;
    app.board.epics = vec![epic];

    // Epic status is Running (not Backlog) — dispatch should be rejected
    let cmds = app.update(Message::DispatchEpic(EpicId(10)));
    assert!(cmds.is_empty());
    assert!(app
        .status
        .message
        .as_ref()
        .unwrap()
        .contains("No backlog tasks"));
}

#[test]
fn dispatch_epic_with_plan_dispatches_next_backlog_subtask() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.plan_path = Some("docs/plan.md".to_string());
    app.board.epics = vec![epic];

    // Add two backlog subtasks for this epic
    let mut task1 = make_task(1, TaskStatus::Backlog);
    task1.epic_id = Some(EpicId(10));
    task1.plan_path = Some("plan1.md".to_string());
    let mut task2 = make_task(2, TaskStatus::Backlog);
    task2.epic_id = Some(EpicId(10));

    app.board.tasks = vec![task1.clone(), task2];

    // Select the epic (only item in backlog column at row 0)
    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);

    let cmds = app.update(Message::DispatchEpic(EpicId(10)));
    // Should dispatch task1 (first backlog subtask, has plan)
    assert_eq!(cmds.len(), 1);
    assert!(matches!(cmds[0], Command::DispatchAgent { ref task, .. } if task.id == TaskId(1)));
}

#[test]
fn dispatch_epic_with_plan_brainstorms_subtask_without_plan() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.plan_path = Some("docs/plan.md".to_string());
    app.board.epics = vec![epic];

    // Subtask without a plan, tagged as "epic" to trigger brainstorm
    let mut task1 = make_task(1, TaskStatus::Backlog);
    task1.epic_id = Some(EpicId(10));
    task1.tag = Some(TaskTag::Epic);
    app.board.tasks = vec![task1];

    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);

    let cmds = app.update(Message::DispatchEpic(EpicId(10)));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(cmds[0], Command::DispatchAgent { ref task, mode: DispatchMode::Brainstorm } if task.id == TaskId(1))
    );
}

#[test]
fn dispatch_epic_with_plan_no_backlog_subtasks_shows_status() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    let mut epic = make_epic(10);
    epic.plan_path = Some("docs/plan.md".to_string());
    app.board.epics = vec![epic];

    // Only an archived subtask — archived tasks are excluded from epic_status
    // so the epic stays Backlog, but there are no backlog subtasks to dispatch
    let mut task1 = make_task(1, TaskStatus::Archived);
    task1.epic_id = Some(EpicId(10));
    app.board.tasks = vec![task1];

    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);

    let cmds = app.update(Message::DispatchEpic(EpicId(10)));
    assert!(cmds.is_empty());
}

#[test]
fn quick_dispatch_zero_is_noop() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    let cmds = app.handle_key(make_key(KeyCode::Char('0')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::QuickDispatch);
}

#[test]
fn quick_dispatch_non_digit_is_noop() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/repo".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    let cmds = app.handle_key(make_key(KeyCode::Char('a')));
    assert!(cmds.is_empty());
    assert_eq!(app.input.mode, InputMode::QuickDispatch);
}

#[test]
fn conflict_flag_clears_on_dispatch() {
    let mut app = App::new(
        vec![{
            let mut t = make_task(1, TaskStatus::Review);
            t.worktree = Some("/repo/.worktrees/1-task-1".to_string());
            t
        }],
        TEST_TIMEOUT,
    );

    app.update(Message::FinishFailed {
        id: TaskId(1),
        error: "conflict".to_string(),
        is_conflict: true,
    });
    assert!(app
        .find_task(TaskId(1))
        .is_some_and(|t| t.sub_status == SubStatus::Conflict));

    app.update(Message::Resumed {
        id: TaskId(1),
        tmux_window: "task-1".to_string(),
    });
    assert!(!app
        .find_task(TaskId(1))
        .is_some_and(|t| t.sub_status == SubStatus::Conflict));
}

#[test]
fn conflict_flag_clears_on_move_backward() {
    let mut app = App::new(
        vec![{
            let mut t = make_task(1, TaskStatus::Review);
            t.worktree = Some("/repo/.worktrees/1-task-1".to_string());
            t
        }],
        TEST_TIMEOUT,
    );

    app.update(Message::FinishFailed {
        id: TaskId(1),
        error: "conflict".to_string(),
        is_conflict: true,
    });

    app.update(Message::MoveTask {
        id: TaskId(1),
        direction: MoveDirection::Backward,
    });
    assert!(!app
        .find_task(TaskId(1))
        .is_some_and(|t| t.sub_status == SubStatus::Conflict));
}

#[test]
fn dispatch_is_noop_when_on_select_all() {
    let mut app = make_app();
    app.handle_key(make_key(KeyCode::Char('k')));
    let cmds = app.handle_key(make_key(KeyCode::Char('d')));
    assert!(cmds.is_empty());
}

#[test]
fn pr_created_stores_url() {
    let task = make_task(1, TaskStatus::Review);
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    let cmds = app.update(Message::PrCreated {
        id: TaskId(1),
        pr_url: "https://github.com/org/repo/pull/42".to_string(),
    });

    let task = app.find_task(TaskId(1)).unwrap();
    assert_eq!(
        task.pr_url.as_deref(),
        Some("https://github.com/org/repo/pull/42")
    );
    assert!(cmds.iter().any(|c| matches!(c, Command::PersistTask(_))));
}

#[test]
fn pr_failed_shows_error() {
    let task = make_task(1, TaskStatus::Review);
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    app.update(Message::PrFailed {
        id: TaskId(1),
        error: "Push failed".to_string(),
    });

    assert!(app
        .status
        .message
        .as_deref()
        .unwrap()
        .contains("Push failed"));
}

#[test]
fn pr_merged_moves_to_done_and_detaches() {
    let mut task = make_task(1, TaskStatus::Review);
    task.tmux_window = Some("task-1".to_string());
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.pr_url = Some("https://github.com/org/repo/pull/42".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.set_notifications_enabled(true);

    let cmds = app.update(Message::PrMerged(TaskId(1)));

    let task = app.find_task(TaskId(1)).unwrap();
    assert_eq!(task.status, TaskStatus::Done);
    assert!(task.tmux_window.is_none(), "tmux window should be cleared");
    assert!(task.worktree.is_some(), "worktree should be preserved");
    assert!(task.pr_url.is_some(), "pr_url should be preserved");
    assert!(cmds.iter().any(|c| matches!(c, Command::PersistTask(_))));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::SendNotification { .. })));
}

#[test]
fn pr_merged_preserves_worktree() {
    let mut task = make_task(1, TaskStatus::Review);
    task.worktree = Some("/repo/.worktrees/1-task-1".to_string());
    task.pr_url = Some("https://github.com/org/repo/pull/42".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    let cmds = app.update(Message::PrMerged(TaskId(1)));

    // Should NOT emit a Cleanup command
    assert!(!cmds.iter().any(|c| matches!(c, Command::Cleanup { .. })));
}

#[test]
fn pr_polling_skips_done_tasks() {
    let mut task = make_task(1, TaskStatus::Done);
    task.pr_url = Some("https://github.com/org/repo/pull/42".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    let cmds = app.update(Message::Tick);
    // Should NOT contain any CheckPrStatus command
    assert!(!cmds
        .iter()
        .any(|c| matches!(c, Command::CheckPrStatus { .. })));
}

#[test]
fn pr_polling_emits_check_for_review_tasks() {
    let mut task = make_task(1, TaskStatus::Review);
    task.pr_url = Some("https://github.com/org/repo/pull/42".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    let cmds = app.update(Message::Tick);
    assert!(cmds.iter().any(|c| matches!(c, Command::CheckPrStatus { ref pr_url, .. } if pr_url == "https://github.com/org/repo/pull/42")));
}

#[test]
fn dispatch_epic_with_backlog_subtasks_dispatches_first_by_sort_order() {
    let mut app = make_app();

    // Create epic with a plan so subtask dispatch path is taken
    let mut epic = make_epic(1);
    epic.plan_path = Some("docs/plans/epic-1.md".to_string());
    app.board.epics = vec![epic];

    // Create two backlog subtasks with different sort orders (both have plans)
    let mut t1 = make_task(10, TaskStatus::Backlog);
    t1.epic_id = Some(EpicId(1));
    t1.sort_order = Some(200);
    t1.title = "Second task".to_string();
    t1.plan_path = Some("docs/plans/task-10.md".to_string());
    let mut t2 = make_task(11, TaskStatus::Backlog);
    t2.epic_id = Some(EpicId(1));
    t2.sort_order = Some(100);
    t2.title = "First task".to_string();
    t2.plan_path = Some("docs/plans/task-11.md".to_string());
    app.board.tasks = vec![t1, t2];

    let cmds = app.update(Message::DispatchEpic(EpicId(1)));

    // Should dispatch the task with lower sort_order (task 11, sort_order=100)
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchAgent { task, .. } if task.id == TaskId(11))));
}

#[test]
fn dispatch_epic_no_subtasks_falls_back_to_planning() {
    let mut app = make_app();

    let epic = make_epic(1);
    app.board.epics = vec![epic];
    // No subtasks

    let cmds = app.update(Message::DispatchEpic(EpicId(1)));

    // Should fall back to planning dispatch
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::DispatchEpic { .. })));
}

#[test]
fn dispatch_epic_no_plan_with_subtasks_does_not_create_planning() {
    let mut app = make_app();

    let epic = make_epic(1); // no plan
    app.board.epics = vec![epic];

    // Epic has an active (running) subtask — should not spawn planning
    let mut t1 = make_task(10, TaskStatus::Running);
    t1.epic_id = Some(EpicId(1));
    app.board.tasks = vec![t1];

    let cmds = app.update(Message::DispatchEpic(EpicId(1)));
    // Epic status is Running, so it's blocked by the Backlog check
    assert!(cmds.is_empty());
}

#[test]
fn dispatch_epic_no_plan_with_backlog_subtask_does_not_create_planning() {
    let mut app = make_app();

    let epic = make_epic(1); // no plan
    app.board.epics = vec![epic];

    // Epic has a backlog subtask — epic status is Backlog but has subtasks
    let mut t1 = make_task(10, TaskStatus::Backlog);
    t1.epic_id = Some(EpicId(1));
    app.board.tasks = vec![t1];

    app.selection_mut().set_column(0);
    app.selection_mut().set_row(0, 0);

    let cmds = app.update(Message::DispatchEpic(EpicId(1)));
    // Should NOT create planning subtask since subtasks already exist
    assert!(cmds.is_empty());
    assert!(app.status.message.as_deref().unwrap().contains("no plan"));
}

#[test]
fn dispatch_epic_all_done_shows_message() {
    let mut app = make_app();

    let mut epic = make_epic(1);
    epic.status = TaskStatus::Done;
    app.board.epics = vec![epic];

    let mut t1 = make_task(10, TaskStatus::Done);
    t1.epic_id = Some(EpicId(1));
    app.board.tasks = vec![t1];

    let cmds = app.update(Message::DispatchEpic(EpicId(1)));

    // Epic status is Done — should not dispatch
    assert!(cmds.is_empty());
    assert!(app
        .status
        .message
        .as_deref()
        .unwrap()
        .contains("No backlog tasks"));
}

#[test]
fn stale_detection_sets_substatus_and_persists() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("win-3".to_string());

    let cmds = app.update(Message::StaleAgent(TaskId(3)));
    let task = app.find_task(TaskId(3)).unwrap();
    assert_eq!(task.sub_status, SubStatus::Stale);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistTask(t) if t.id == TaskId(3))));
}

#[test]
fn crashed_detection_sets_substatus_and_persists() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("win-3".to_string());

    let cmds = app.update(Message::AgentCrashed(TaskId(3)));
    let task = app.find_task(TaskId(3)).unwrap();
    assert_eq!(task.sub_status, SubStatus::Crashed);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::PersistTask(t) if t.id == TaskId(3))));
}

#[test]
fn stale_does_not_overwrite_crashed() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("win-3".to_string());
    app.board.tasks[0].sub_status = SubStatus::Crashed;

    let cmds = app.update(Message::StaleAgent(TaskId(3)));
    let task = app.find_task(TaskId(3)).unwrap();
    assert_eq!(task.sub_status, SubStatus::Crashed); // unchanged
    assert!(cmds.is_empty()); // no persist needed
}

#[test]
fn stale_skips_non_running_task() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Backlog)], TEST_TIMEOUT);

    let cmds = app.update(Message::StaleAgent(TaskId(3)));
    let task = app.find_task(TaskId(3)).unwrap();
    assert_eq!(task.sub_status, SubStatus::None); // unchanged
    assert!(cmds.is_empty());
}

#[test]
fn crashed_skips_non_running_task() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Review)], TEST_TIMEOUT);

    let cmds = app.update(Message::AgentCrashed(TaskId(3)));
    let task = app.find_task(TaskId(3)).unwrap();
    assert_eq!(task.sub_status, SubStatus::AwaitingReview); // unchanged
    assert!(cmds.is_empty());
}

#[test]
fn stale_notification_sent_when_enabled() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("win-3".to_string());
    app.set_notifications_enabled(true);

    let cmds = app.update(Message::StaleAgent(TaskId(3)));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::SendNotification { urgent: false, .. })));
}

#[test]
fn stale_notification_not_sent_when_disabled() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("win-3".to_string());
    app.set_notifications_enabled(false);

    let cmds = app.update(Message::StaleAgent(TaskId(3)));
    assert!(!cmds
        .iter()
        .any(|c| matches!(c, Command::SendNotification { .. })));
}

#[test]
fn crashed_notification_sent_urgent_when_enabled() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("win-3".to_string());
    app.set_notifications_enabled(true);

    let cmds = app.update(Message::AgentCrashed(TaskId(3)));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::SendNotification { urgent: true, .. })));
}

#[test]
fn crashed_notification_not_sent_when_disabled() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("win-3".to_string());
    app.set_notifications_enabled(false);

    let cmds = app.update(Message::AgentCrashed(TaskId(3)));
    assert!(!cmds
        .iter()
        .any(|c| matches!(c, Command::SendNotification { .. })));
}

#[test]
fn pr_review_state_updates_substatus() {
    let mut app = make_app();
    let id = TaskId(3);
    app.find_task_mut(id).unwrap().status = TaskStatus::Review;
    app.find_task_mut(id).unwrap().sub_status = SubStatus::AwaitingReview;
    let cmds = app.update(Message::PrReviewState {
        id,
        review_decision: Some(ReviewDecision::Approved),
    });
    let task = app.find_task(id).unwrap();
    assert_eq!(task.sub_status, SubStatus::Approved);
    assert!(cmds.iter().any(|c| matches!(c, Command::PersistTask(_))));
}

#[test]
fn pr_review_state_noop_when_unchanged() {
    let mut app = make_app();
    let id = TaskId(3);
    app.find_task_mut(id).unwrap().status = TaskStatus::Review;
    app.find_task_mut(id).unwrap().sub_status = SubStatus::AwaitingReview;
    let cmds = app.update(Message::PrReviewState {
        id,
        review_decision: None, // maps to AwaitingReview
    });
    assert!(cmds.is_empty()); // no change, no persist
}

#[test]
fn pr_review_state_changes_requested() {
    let mut app = make_app();
    let id = TaskId(3);
    app.find_task_mut(id).unwrap().status = TaskStatus::Review;
    app.find_task_mut(id).unwrap().sub_status = SubStatus::AwaitingReview;
    let cmds = app.update(Message::PrReviewState {
        id,
        review_decision: Some(ReviewDecision::ChangesRequested),
    });
    let task = app.find_task(id).unwrap();
    assert_eq!(task.sub_status, SubStatus::ChangesRequested);
    assert!(cmds.iter().any(|c| matches!(c, Command::PersistTask(_))));
}

#[test]
fn pr_review_state_ignores_non_review_task() {
    let mut app = make_app();
    let id = TaskId(3);
    // Task 3 is Running by default in make_app
    assert_eq!(app.find_task(id).unwrap().status, TaskStatus::Running);
    let cmds = app.update(Message::PrReviewState {
        id,
        review_decision: Some(ReviewDecision::Approved),
    });
    assert!(cmds.is_empty());
    // sub_status should not have changed
    assert_ne!(app.find_task(id).unwrap().sub_status, SubStatus::Approved);
}

#[test]
fn pr_review_state_preserves_conflict_substatus() {
    let mut app = make_app();
    let id = TaskId(3);
    app.find_task_mut(id).unwrap().status = TaskStatus::Review;
    app.find_task_mut(id).unwrap().sub_status = SubStatus::Conflict;
    let cmds = app.update(Message::PrReviewState {
        id,
        review_decision: Some(ReviewDecision::Approved),
    });
    assert!(cmds.is_empty());
    assert_eq!(app.find_task(id).unwrap().sub_status, SubStatus::Conflict);
}

#[test]
fn quick_dispatch_j_moves_cursor_down() {
    let mut app = App::new(vec![], TEST_TIMEOUT);
    app.board.repo_paths = vec!["/a".to_string(), "/b".to_string(), "/c".to_string()];
    app.input.mode = InputMode::QuickDispatch;
    app.input.repo_cursor = 0;
    app.handle_key(make_key(KeyCode::Char('j')));
    assert_eq!(app.input.repo_cursor, 1);
}

#[test]
fn quick_dispatch_enter_selects_cursor_repo() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)], TEST_TIMEOUT);
    app.board.repo_paths = vec![
        "/repo1".to_string(),
        "/repo2".to_string(),
        "/repo3".to_string(),
    ];
    app.input.mode = InputMode::QuickDispatch;
    app.input.repo_cursor = 2; // third repo
    let cmds = app.handle_key(make_key(KeyCode::Enter));
    assert_eq!(cmds.len(), 1);
    assert!(
        matches!(&cmds[0], Command::QuickDispatch { ref draft, epic_id: None } if draft.repo_path == "/repo3")
    );
    assert_eq!(app.input.mode, InputMode::Normal);
}

#[test]
fn tab_bar_board_mode_highlights_tab_key() {
    let mut app = make_app();
    let buf = render_to_buffer(&mut app, 100, 30);
    let style = find_style_of(&buf, "[Tab]").expect("[Tab] text not found in buffer");
    assert!(
        style.add_modifier.contains(Modifier::BOLD),
        "[Tab] should be bold"
    );
    assert_eq!(
        style.fg,
        Some(Color::Rgb(120, 124, 153)),
        "[Tab] should use MUTED_LIGHT color"
    );
}

#[test]
fn dispatch_in_flight_blocks_second_dispatch() {
    let mut app = make_app();
    // First dispatch succeeds
    let cmds = app.update(Message::DispatchTask(TaskId(1), DispatchMode::Dispatch));
    assert!(matches!(cmds[0], Command::DispatchAgent { .. }));
    // Second dispatch of same task is blocked
    let cmds = app.update(Message::DispatchTask(TaskId(1), DispatchMode::Dispatch));
    assert!(cmds.is_empty());
}

#[test]
fn brainstorm_in_flight_blocks_second_brainstorm() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.tag = Some(TaskTag::Feature);
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    // First brainstorm succeeds (feature without plan → brainstorm)
    let cmds = app.update(Message::DispatchTask(TaskId(1), DispatchMode::Brainstorm));
    assert!(matches!(
        cmds[0],
        Command::DispatchAgent {
            mode: DispatchMode::Brainstorm,
            ..
        }
    ));
    // Second brainstorm of same task is blocked
    let cmds = app.update(Message::DispatchTask(TaskId(1), DispatchMode::Brainstorm));
    assert!(cmds.is_empty());
}

#[test]
fn plan_in_flight_blocks_second_plan() {
    let mut task = make_task(1, TaskStatus::Backlog);
    task.tag = Some(TaskTag::Feature);
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    let cmds = app.update(Message::DispatchTask(TaskId(1), DispatchMode::Plan));
    assert!(matches!(
        cmds[0],
        Command::DispatchAgent {
            mode: DispatchMode::Plan,
            ..
        }
    ));
    // Second plan of same task is blocked
    let cmds = app.update(Message::DispatchTask(TaskId(1), DispatchMode::Plan));
    assert!(cmds.is_empty());
}

#[test]
fn dispatched_clears_in_flight() {
    let mut app = make_app();
    // Dispatch task 1
    app.update(Message::DispatchTask(TaskId(1), DispatchMode::Dispatch));
    // Dispatched message clears the in-flight guard
    app.update(Message::Dispatched {
        id: TaskId(1),
        worktree: "/wt".to_string(),
        tmux_window: "win".to_string(),
        switch_focus: false,
    });
    // Task is now Running, so dispatch is a no-op for a different reason,
    // but the in-flight set should be clear
    assert!(!app.is_dispatching(TaskId(1)));
}

#[test]
fn dispatch_failed_clears_in_flight() {
    let mut app = make_app();
    // Dispatch task 1
    app.update(Message::DispatchTask(TaskId(1), DispatchMode::Dispatch));
    assert!(app.is_dispatching(TaskId(1)));
    // DispatchFailed clears the in-flight guard
    app.update(Message::DispatchFailed(TaskId(1)));
    assert!(!app.is_dispatching(TaskId(1)));
    // Can dispatch again
    let cmds = app.update(Message::DispatchTask(TaskId(1), DispatchMode::Dispatch));
    assert!(matches!(cmds[0], Command::DispatchAgent { .. }));
}

#[test]
fn dispatch_different_tasks_both_succeed() {
    let mut app = make_app();
    // Dispatch task 1
    let cmds = app.update(Message::DispatchTask(TaskId(1), DispatchMode::Dispatch));
    assert!(matches!(cmds[0], Command::DispatchAgent { .. }));
    // Dispatch task 2 — different task, should succeed
    let cmds = app.update(Message::DispatchTask(TaskId(2), DispatchMode::Dispatch));
    assert!(matches!(cmds[0], Command::DispatchAgent { .. }));
}

#[test]
fn dispatch_failed_clears_mark_dispatching_guard() {
    let mut app = make_app();
    app.update(Message::MarkDispatching(TaskId(99)));
    assert!(app.is_dispatching(TaskId(99)));
    app.update(Message::DispatchFailed(TaskId(99)));
    assert!(!app.is_dispatching(TaskId(99)));
}

#[test]
fn window_gone_ignored_for_split_pinned_task() {
    let mut task = make_task(4, TaskStatus::Running);
    task.tmux_window = Some("task-4".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);

    // Pin task 4 in split mode
    app.board.split.active = true;
    app.board.split.right_pane_id = Some("%42".to_string());
    app.board.split.pinned_task_id = Some(TaskId(4));

    // Even if WindowGone fires for the pinned task, it should NOT crash
    app.update(Message::WindowGone(TaskId(4)));
    assert!(
        !app.is_crashed(TaskId(4)),
        "split-pinned task should not be marked as crashed"
    );
}

#[test]
fn agent_crashed_stores_last_error_from_tmux_output() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)], TEST_TIMEOUT);
    app.board.tasks[0].tmux_window = Some("task-4".to_string());
    app.agents.tmux_outputs.insert(
        TaskId(4),
        "Error: connection refused\npanicked at main.rs:42".to_string(),
    );

    app.update(Message::AgentCrashed(TaskId(4)));

    assert_eq!(
        app.agents.last_error.get(&TaskId(4)).map(|s| s.as_str()),
        Some("Error: connection refused\npanicked at main.rs:42"),
    );
}

#[test]
fn retry_fresh_clears_last_error() {
    let mut task = make_task(4, TaskStatus::Running);
    task.worktree = Some("/repo/.worktrees/4-task-4".to_string());
    let mut app = App::new(vec![task], TEST_TIMEOUT);
    app.agents
        .last_error
        .insert(TaskId(4), "some crash".to_string());
    app.input.mode = InputMode::ConfirmRetry(TaskId(4));

    app.update(Message::RetryFresh(TaskId(4)));

    assert!(!app.agents.last_error.contains_key(&TaskId(4)));
}

#[test]
fn workflow_column_round_trips_column_index() {
    use crate::models::SecurityWorkflowColumn;
    for col in [
        SecurityWorkflowColumn::Backlog,
        SecurityWorkflowColumn::InProgress,
        SecurityWorkflowColumn::Review,
    ] {
        assert_eq!(
            SecurityWorkflowColumn::from_column_index(col.column_index()),
            Some(col)
        );
    }
}

#[test]
fn workflow_column_labels() {
    use crate::models::SecurityWorkflowColumn;
    assert_eq!(SecurityWorkflowColumn::Backlog.label(), "Backlog");
    assert_eq!(SecurityWorkflowColumn::InProgress.label(), "In Progress");
    assert_eq!(SecurityWorkflowColumn::Review.label(), "Review");
}

#[test]
fn workflow_column_count_is_three() {
    assert_eq!(crate::models::SecurityWorkflowColumn::COLUMN_COUNT, 3);
}
