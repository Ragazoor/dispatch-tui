use super::*;
use crate::models::test_pane_id;

#[test]
fn tick_checks_window_for_review_task_with_live_window() {
    let mut app = App::new(vec![make_task(5, TaskStatus::Review)]);

    let cmds = app.update(Message::System(SystemMessage::Tick));

    assert!(cmds.iter().any(|c| {
        if let Command::Task(TaskCommand::BatchCheckWindows { windows }) = c {
            windows.iter().any(|(id, _)| *id == TaskId(5))
        } else {
            false
        }
    }));
}

#[test]
fn stale_agent_detected_when_last_pre_tool_use_old() {
    let mut task = make_task(4, TaskStatus::Running);
    // Pre-tool-use older than ACTIVE_THRESHOLD → classifier returns Stale.
    task.last_pre_tool_use_at = Some(chrono::Utc::now() - chrono::Duration::minutes(10));
    let mut app = App::new(vec![task]);

    let cmds = app.update(Message::System(SystemMessage::Tick));
    assert!(app.is_stale(TaskId(4)));
    assert!(cmds.iter().any(|c| {
        if let Command::Task(TaskCommand::BatchCheckWindows { windows }) = c {
            windows.iter().any(|(id, _)| *id == TaskId(4))
        } else {
            false
        }
    }));
    assert!(cmds.iter().any(|c| {
        if let Command::Task(TaskCommand::BatchPatchSubStatus { updates }) = c {
            updates.iter().any(|(id, _)| *id == TaskId(4))
        } else {
            false
        }
    }));
}

#[test]
fn window_gone_on_running_task_marks_crashed() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)]);

    let cmds = app.update(Message::Task(TaskMessage::WindowGone(TaskId(4))));
    assert!(app.is_crashed(TaskId(4)));
    // tmux_window should be cleared — the window is gone by definition
    assert!(app.board.tasks[0].tmux_window.is_none());
    // Should emit PersistTask with cleared tmux_window
    assert!(cmds.iter().any(
        |c| matches!(c, Command::Task(TaskCommand::Persist(t)) if t.id == TaskId(4) && t.tmux_window.is_none())
    ));
}

#[test]
fn window_gone_on_review_task_clears_window() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Review)]);

    let cmds = app.update(Message::Task(TaskMessage::WindowGone(TaskId(4))));
    assert!(!app.is_crashed(TaskId(4)));
    assert!(app.board.tasks[0].tmux_window.is_none());
    assert!(matches!(&cmds[0], Command::Task(TaskCommand::Persist(_))));
}

#[test]
fn tick_reclassifies_running_task_to_stale_when_pre_tool_use_is_old() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)]);
    app.board.tasks[0].tmux_window = Some(test_tmux_window("win-3"));
    app.board.tasks[0].sub_status = SubStatus::Active;
    let old = chrono::Utc::now() - ACTIVE_THRESHOLD - chrono::Duration::seconds(5);
    app.board.tasks[0].last_pre_tool_use_at = Some(old);

    let cmds = app.update(Message::System(SystemMessage::Tick));
    let task = app.board.find_task(TaskId(3)).unwrap();
    assert_eq!(task.sub_status, SubStatus::Stale);
    assert!(cmds.iter().any(|c| {
        if let Command::Task(TaskCommand::BatchPatchSubStatus { updates }) = c {
            updates
                .iter()
                .any(|(id, ss)| *id == TaskId(3) && *ss == SubStatus::Stale)
        } else {
            false
        }
    }));
}

#[test]
fn tick_reclassifies_running_task_to_needs_input_when_notification_newer() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)]);
    app.board.tasks[0].tmux_window = Some(test_tmux_window("win-3"));
    app.board.tasks[0].sub_status = SubStatus::Active;
    let now = chrono::Utc::now();
    app.board.tasks[0].last_pre_tool_use_at = Some(now - chrono::Duration::seconds(30));
    app.board.tasks[0].last_notification_at = Some(now - chrono::Duration::seconds(5));

    app.update(Message::System(SystemMessage::Tick));
    let task = app.board.find_task(TaskId(3)).unwrap();
    assert_eq!(task.sub_status, SubStatus::NeedsInput);
}

#[test]
fn tick_does_not_overwrite_crashed() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)]);
    app.board.tasks[0].tmux_window = Some(test_tmux_window("win-3"));
    app.board.tasks[0].sub_status = SubStatus::Crashed;
    let old = chrono::Utc::now() - chrono::Duration::minutes(5);
    app.board.tasks[0].last_pre_tool_use_at = Some(old);

    app.update(Message::System(SystemMessage::Tick));
    let task = app.board.find_task(TaskId(3)).unwrap();
    assert_eq!(task.sub_status, SubStatus::Crashed);
}

#[test]
fn tick_does_not_overwrite_conflict() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)]);
    app.board.tasks[0].tmux_window = Some(test_tmux_window("win-3"));
    app.board.tasks[0].sub_status = SubStatus::Conflict;
    let old = chrono::Utc::now() - chrono::Duration::minutes(5);
    app.board.tasks[0].last_pre_tool_use_at = Some(old);

    app.update(Message::System(SystemMessage::Tick));
    let task = app.board.find_task(TaskId(3)).unwrap();
    assert_eq!(task.sub_status, SubStatus::Conflict);
}

#[test]
fn crashed_detection_sets_substatus_and_persists() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)]);
    app.board.tasks[0].tmux_window = Some(test_tmux_window("win-3"));

    let cmds = app.update(Message::Task(TaskMessage::AgentCrashed(TaskId(3))));
    let task = app.board.find_task(TaskId(3)).unwrap();
    assert_eq!(task.sub_status, SubStatus::Crashed);
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::Task(TaskCommand::Persist(t)) if t.id == TaskId(3))));
}

#[test]
fn crash_emits_a_non_draining_subagent_clear() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Running)]);
    if let Some(t) = app.find_task_mut(TaskId(1)) {
        t.live_subagents = 2;
        t.stop_pending = true;
    }

    let cmds = app.handle_agent_crashed(TaskId(1));

    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Task(TaskCommand::ClearSubagents {
                id,
                mode: crate::models::DrainMode::NoDrain,
            }) if *id == TaskId(1)
        )),
        "a crash must clear subagents without draining to Review"
    );
    assert_eq!(
        app.board.find_task(TaskId(1)).unwrap().live_subagents,
        0,
        "the board repaints immediately, without waiting for the DB round trip"
    );
    assert!(!app.board.find_task(TaskId(1)).unwrap().stop_pending);
}

#[test]
fn crashed_skips_non_running_task() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Review)]);

    let cmds = app.update(Message::Task(TaskMessage::AgentCrashed(TaskId(3))));
    let task = app.board.find_task(TaskId(3)).unwrap();
    assert_eq!(task.sub_status, SubStatus::AwaitingReview); // unchanged
    assert!(cmds.is_empty());
}

#[test]
fn crashed_notification_sent_urgent_when_enabled() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)]);
    app.board.tasks[0].tmux_window = Some(test_tmux_window("win-3"));
    app.set_notifications_enabled(true);

    let cmds = app.update(Message::Task(TaskMessage::AgentCrashed(TaskId(3))));
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::System(SystemCommand::SendNotification { urgent: true, .. })
    )));
}

#[test]
fn crashed_notification_not_sent_when_disabled() {
    let mut app = App::new(vec![make_task(3, TaskStatus::Running)]);
    app.board.tasks[0].tmux_window = Some(test_tmux_window("win-3"));
    app.set_notifications_enabled(false);

    let cmds = app.update(Message::Task(TaskMessage::AgentCrashed(TaskId(3))));
    assert!(!cmds
        .iter()
        .any(|c| matches!(c, Command::System(SystemCommand::SendNotification { .. }))));
}

#[test]
fn window_gone_ignored_for_split_pinned_task() {
    let mut app = App::new(vec![make_task(4, TaskStatus::Running)]);

    // Pin task 4 in split mode
    app.board.split.active = true;
    app.board.split.right_pane_id = Some(test_pane_id("%42"));
    app.board.split.pinned_task_id = Some(TaskId(4));

    // Even if WindowGone fires for the pinned task, it should NOT crash
    app.update(Message::Task(TaskMessage::WindowGone(TaskId(4))));
    assert!(
        !app.is_crashed(TaskId(4)),
        "split-pinned task should not be marked as crashed"
    );
}
