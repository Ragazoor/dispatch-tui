use super::*;

// `dispatch_task_release_claim_returns_the_task_to_backlog` pinned
// `TaskCommand::ReleaseClaim`, which store-split M4 retired: no client releases
// a claim, a failed request does (`store-split.allium:
// ReleaseClaimOfFailedRequest`). Its guarantee — a failed dispatch returns the
// task to backlog with the seeded stamp cleared — is pinned against the
// request flow in `src/service/tasks/tests/crud/claim_and_close.rs`
// (`a_failed_request_returns_the_task_to_backlog`).

#[tokio::test]
async fn dispatch_task_clear_subagents_drops_the_live_count() {
    let mut h = quiet_harness().await;
    let task = create_task_returning(
        &**h.rt.db_write(),
        "Busy",
        "desc",
        "/repo",
        None,
        models::TaskStatus::Running,
    )
    .await
    .unwrap();
    h.rt.db_write().give_agent(task.id);
    h.rt.task_svc
        .record_subagent_event(
            task.id,
            models::SubagentEvent::Start {
                agent_id: "a1".into(),
                session_id: "s1".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        h.rt.database
            .get_task(task.id)
            .await
            .unwrap()
            .live_subagents,
        1,
        "precondition: one live subagent"
    );

    h.dispatch(Command::Task(TaskCommand::ClearSubagents {
        id: task.id,
        mode: models::DrainMode::Drain,
    }))
    .await;

    assert_eq!(
        h.rt.database
            .get_task(task.id)
            .await
            .unwrap()
            .live_subagents,
        0
    );
}

#[tokio::test]
async fn dispatch_task_check_window_reports_a_window_that_is_gone() {
    let mut h = harness(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"other-window\n"),
    ]))
    .await;

    h.dispatch(Command::Task(TaskCommand::CheckWindow {
        id: TaskId(1),
        window: test_tmux_window("gone-window"),
    }))
    .await;

    assert!(matches!(
        h.next_msg().await,
        Message::Task(crate::tui::messages::TaskMessage::WindowGone(TaskId(1)))
    ));
}

/// The batch arm's distinguishing property: one tmux call for N windows, and
/// a `WindowGone` for the absent one only.
#[tokio::test]
async fn dispatch_task_batch_check_windows_reports_only_the_absent_one() {
    let mut h = harness(MockProcessRunner::new(vec![
        MockProcessRunner::ok_with_stdout(b"task-1\nother-window\n"),
    ]))
    .await;

    h.dispatch(Command::Task(TaskCommand::BatchCheckWindows {
        windows: vec![
            (TaskId(1), TmuxWindow::for_task(TaskId(1))),
            (TaskId(2), TmuxWindow::for_task(TaskId(2))),
        ],
    }))
    .await;

    assert!(matches!(
        h.next_msg().await,
        Message::Task(crate::tui::messages::TaskMessage::WindowGone(TaskId(2)))
    ));
    assert_eq!(
        h.mock.recorded_calls().len(),
        1,
        "the batch arm issues one tmux call, not one per window"
    );
}

#[tokio::test]
async fn dispatch_task_resume_relaunches_the_agent_window() {
    let mut h = harness(DispatchScript::resume().runner()).await;
    let task = create_task_returning(
        &**h.rt.db_write(),
        "Resume Me",
        "desc",
        "/repo",
        None,
        models::TaskStatus::Backlog,
    )
    .await
    .unwrap();
    let id = task.id;
    give_current_agent(&h.rt, id).await;
    let worktree = Some("/repo/.worktrees/1-resume-me".to_string());

    h.dispatch(Command::Task(TaskCommand::Resume { id, worktree }))
        .await;

    match h.next_msg().await {
        Message::Task(crate::tui::messages::TaskMessage::Resumed {
            id: tid,
            tmux_window,
        }) => {
            assert_eq!(tid, id);
            assert_eq!(tmux_window, TmuxWindow::for_task(id));
        }
        other => panic!("expected Resumed, got {other:?}"),
    }
}

#[tokio::test]
async fn dispatch_task_jump_to_tmux_selects_the_window() {
    let mut h =
        harness(MockProcessRunner::new(vec![MockProcessRunner::ok()]).with_windows(&["task-1"]))
            .await;

    h.dispatch(Command::Task(TaskCommand::JumpToTmux {
        window: test_tmux_window("task-1"),
    }))
    .await;

    // Synchronous, not spawned: the call is already recorded on return.
    let calls = h.mock.flattened_calls();
    assert!(
        calls.iter().any(|c| c.contains("select-window")),
        "got: {calls:?}"
    );
    assert!(h.app.error_popup().is_none());
}

#[tokio::test]
async fn dispatch_task_jump_to_tmux_surfaces_a_failed_jump() {
    let mut h = harness(MockProcessRunner::new(vec![MockProcessRunner::fail(
        "no such window",
    )]))
    .await;

    h.dispatch(Command::Task(TaskCommand::JumpToTmux {
        window: test_tmux_window("task-1"),
    }))
    .await;

    let err = h.app.error_popup().unwrap_or_default();
    assert!(err.contains("Jump failed"), "got: {err:?}");
}

#[tokio::test]
async fn dispatch_task_cleanup_carries_its_follow_up_back_on_success() {
    let mut h = harness(MockProcessRunner::new(vec![
        MockProcessRunner::ok(), // git worktree remove
        MockProcessRunner::ok(), // git branch -D
    ]))
    .await;

    h.dispatch(Command::Task(TaskCommand::Cleanup {
        id: TaskId(1),
        repo_path: "/repo".into(),
        worktree: Some("/repo/.worktrees/1-doomed".into()),
        tmux_window: None,
        follow_up: crate::tui::commands::CleanupFollowUp::DeleteRow,
        guard: None,
    }))
    .await;

    let msg = h.next_msg().await;
    assert!(
        matches!(
            msg,
            Message::Task(crate::tui::messages::TaskMessage::CleanupSucceeded {
                id: TaskId(1),
                follow_up: crate::tui::commands::CleanupFollowUp::DeleteRow,
            })
        ),
        "the follow-up must ride the completion path, got: {msg:?}"
    );
}

/// No message at all on either outcome — the kill is best-effort — so the
/// runner announcement is the only signal this arm produces.
#[tokio::test]
async fn dispatch_task_kill_tmux_window_kills_the_window() {
    let mut h =
        harness(MockProcessRunner::new(vec![MockProcessRunner::ok()]).with_windows(&["task-1"]))
            .await;

    h.dispatch(Command::Task(TaskCommand::KillTmuxWindow {
        window: test_tmux_window("task-1"),
    }))
    .await;

    let call = h.await_call("kill-window").await;
    assert!(
        call.contains(&h.mock.pane_id_of("task-1")),
        "targeted by resolved pane ID, not by name (see `tmux::window_target`): {call}"
    );
}
