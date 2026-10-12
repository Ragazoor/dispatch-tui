#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Integration test: visible dispatching status across the full
//! mark → in-flight → resolved lifecycle. This test exercises the public
//! `App` API (`status_message`, `is_dispatching`) so a regression that
//! breaks the user-facing feedback contract is caught even if the
//! TUI-internal helpers are refactored.

use dispatch_tui::models::{Task, TaskBuilder, TaskId, TmuxWindow};
use dispatch_tui::tui::{App, Message};

fn make_task(id: i64, title: &str) -> Task {
    TaskBuilder::new(id).title(title).build()
}

fn make_app(task: Task) -> App {
    App::new(vec![task])
}

#[tokio::test]
async fn dispatching_status_visible_across_lifecycle_success() {
    let mut app = make_app(make_task(7, "Fix login bug"));

    // 1. Pre-mark: no status, not dispatching.
    assert!(app.status_message().is_none());
    assert!(!app.is_dispatching(TaskId(7)));

    // 2. Mark dispatching: status mentions task title.
    app.update(Message::Task(
        dispatch_tui::tui::messages::TaskMessage::MarkDispatching(TaskId(7)),
    ));
    assert!(app.is_dispatching(TaskId(7)));
    let msg = app
        .status_message()
        .expect("status set after MarkDispatching");
    assert!(msg.contains("Fix login bug"), "got: {msg}");
    assert!(msg.contains("Dispatching"), "got: {msg}");

    // 3. Tick mid-flight: status persists (sticky).
    app.update(Message::System(
        dispatch_tui::tui::messages::SystemMessage::Tick,
    ));
    let msg = app
        .status_message()
        .expect("sticky status survives Tick during dispatch");
    assert!(msg.contains("Fix login bug"));

    // 4. Dispatched: status clears, set drains.
    app.update(Message::Task(
        dispatch_tui::tui::messages::TaskMessage::Dispatched {
            id: TaskId(7),
            worktree: "/repo/.worktrees/7-fix-login-bug".to_string(),
            tmux_window: TmuxWindow::for_task(TaskId(7)),
            switch_focus: false,
        },
    ));
    assert!(!app.is_dispatching(TaskId(7)));
    assert!(
        app.status_message().is_none(),
        "status should clear when dispatch resolves"
    );
}

/// Since store-split M4 a failed dispatch reaches the board as its request
/// row: the open row keeps the task a member, and the row reaching failed
/// ends the membership, clears the status and raises the row's reason
/// (`dispatch.allium: DispatchingFeedback`, `FailedRequestReported`).
#[tokio::test]
async fn dispatching_status_visible_across_lifecycle_failure() {
    use dispatch_tui::models::{
        DispatchMode, DispatchOrigin, DispatchRequest, DispatchRequestId, DispatchRequestState,
    };
    use dispatch_tui::tui::messages::TaskMessage;

    let mut app = make_app(make_task(8, "Refactor module"));
    app.update(Message::Task(TaskMessage::DispatchRequestsRefreshed(
        vec![],
    )));

    app.update(Message::Task(TaskMessage::MarkDispatching(TaskId(8))));
    assert!(app.is_dispatching(TaskId(8)));
    assert!(app.status_message().is_some());

    let requested_at = chrono::Utc::now();
    let row = |state, reason: Option<&str>| DispatchRequest {
        id: DispatchRequestId(1),
        task_id: TaskId(8),
        host: "this-host".into(),
        requested_by: "me".into(),
        reused_worktree: false,
        mode: DispatchMode::Dispatch,
        origin: DispatchOrigin::User,
        state,
        requested_at,
        claimed_at: Some(requested_at),
        finished_at: reason.map(|_| requested_at),
        reason: reason.map(str::to_string),
        worktree: None,
        tmux_window: None,
        deadline: requested_at + chrono::Duration::seconds(60),
    };
    app.update(Message::Task(TaskMessage::DispatchRequestsRefreshed(vec![
        row(DispatchRequestState::Provisioning, None),
    ])));
    assert!(
        app.is_dispatching(TaskId(8)),
        "the open row keeps it a member"
    );

    app.update(Message::Task(TaskMessage::DispatchRequestsRefreshed(vec![
        row(
            DispatchRequestState::Failed,
            Some("git worktree add failed"),
        ),
    ])));
    assert!(!app.is_dispatching(TaskId(8)));
    assert!(
        app.status_message().is_none(),
        "status should clear when dispatch fails"
    );
    assert!(app
        .error_popup()
        .is_some_and(|p| p.contains("git worktree add failed")));
}

#[tokio::test]
async fn multiple_dispatches_show_pluralized_status() {
    let mut app = App::new(vec![make_task(1, "Task A"), make_task(2, "Task B")]);

    app.update(Message::Task(
        dispatch_tui::tui::messages::TaskMessage::MarkDispatching(TaskId(1)),
    ));
    app.update(Message::Task(
        dispatch_tui::tui::messages::TaskMessage::MarkDispatching(TaskId(2)),
    ));

    let msg = app.status_message().expect("status set");
    assert!(msg.contains("2 tasks"), "expected plural form, got: {msg}");

    // Resolving one transitions back to the singular form.
    app.update(Message::Task(
        dispatch_tui::tui::messages::TaskMessage::Dispatched {
            id: TaskId(1),
            worktree: "/wt/1".to_string(),
            tmux_window: TmuxWindow::for_task(TaskId(1)),
            switch_focus: false,
        },
    ));
    let msg = app.status_message().expect("status still set");
    assert!(
        msg.contains("Task B"),
        "expected singular form referencing remaining task, got: {msg}"
    );
}
