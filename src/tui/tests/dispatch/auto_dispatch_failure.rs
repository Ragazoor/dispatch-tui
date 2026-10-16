use super::*;

/// Fail subtask `id`'s chained dispatch the way the board learns of it since
/// store-split M4: its `epic_chain` request row reaching failed
/// (`epics.allium: ReportChainedDispatchFailure`). Puts the task in epic 7
/// first, and delivers the open row before the failed one so the failure is
/// not mistaken for baseline history.
fn fail_chain(app: &mut App, id: i64) -> Vec<Command> {
    use crate::models::{DispatchOrigin, DispatchRequestState};
    if let Some(task) = app.board.tasks.iter_mut().find(|t| t.id == TaskId(id)) {
        task.epic_id = Some(EpicId(7));
    }
    let chained = |state| crate::models::DispatchRequest {
        origin: DispatchOrigin::EpicChain,
        ..request_row(100 + id, id, state)
    };
    app.update(requests_refreshed(vec![chained(
        DispatchRequestState::Provisioning,
    )]));
    app.update(requests_refreshed(vec![chained(
        DispatchRequestState::Failed,
    )]))
}

/// The marker is what survives once the status message and the notification
/// have gone. Without it a chain that died overnight is indistinguishable from
/// an epic that simply ran out of subtasks.
#[test]
fn auto_dispatch_failure_marks_the_subtask() {
    let mut app = make_app();
    assert!(!app.auto_dispatch_failed(TaskId(1)));

    fail_chain(&mut app, 1);

    assert!(app.auto_dispatch_failed(TaskId(1)));
    assert!(
        !app.auto_dispatch_failed(TaskId(2)),
        "only the subtask that failed is marked"
    );
}

/// StatusMessageNamesTheSubtask: an operator watching the board must learn
/// which chain stopped, not merely that something went wrong.
#[test]
fn auto_dispatch_failure_sets_a_status_message_naming_the_task() {
    let mut app = make_app();

    fail_chain(&mut app, 1);

    let status = app.status_message().unwrap_or_default().to_string();
    assert!(
        status.contains("#1"),
        "status message must name the subtask, got: {status}"
    );
    assert!(
        status.contains("git fetch origin main failed"),
        "status message must carry the reason, got: {status}"
    );
}

/// Gated on `notifications_enabled`, exactly as NotifyNeedsInput / NotifyReview
/// are (docs/specs/agent-health.allium).
#[test]
fn auto_dispatch_failure_notifies_when_notifications_are_enabled() {
    let mut app = make_app();
    app.set_notifications_enabled(true);

    let cmds = fail_chain(&mut app, 1);

    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::System(SystemCommand::SendNotification { urgent, .. })
                if *urgent
        )),
        "an auto-dispatch failure must raise an urgent notification, got: {cmds:?}"
    );
}

#[test]
fn auto_dispatch_failure_does_not_notify_when_notifications_are_disabled() {
    let mut app = make_app();
    app.set_notifications_enabled(false);

    let cmds = fail_chain(&mut app, 1);

    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Command::System(SystemCommand::SendNotification { .. }))),
        "notifications are gated on the setting, got: {cmds:?}"
    );
    assert!(
        app.auto_dispatch_failed(TaskId(1)),
        "the board marker is not gated on the notification setting"
    );
}

/// PersistsUntilRedispatched: a retry is the resolution, so starting one clears
/// the marker.
#[test]
fn auto_dispatch_failure_marker_cleared_by_a_new_dispatch() {
    let mut app = make_app();
    fail_chain(&mut app, 1);

    app.update(Message::Task(TaskMessage::MarkDispatching(TaskId(1))));

    assert!(!app.auto_dispatch_failed(TaskId(1)));
}

/// The other half of PersistsUntilRedispatched: a subtask that left backlog by
/// any route is no longer stalled, and the board learns that from the refreshed
/// row rather than from a message of its own.
#[test]
fn auto_dispatch_failure_marker_cleared_when_the_task_leaves_backlog() {
    let mut app = make_app();
    fail_chain(&mut app, 1);

    app.update(Message::Task(TaskMessage::Updated(Box::new(make_task(
        1,
        TaskStatus::Running,
    )))));

    assert!(!app.auto_dispatch_failed(TaskId(1)));
}

/// A refreshed row that is still in backlog is still stalled — the update must
/// not clear the marker just because the row was reloaded (which it is, right
/// after the failure, by the chain's own TaskChanged event).
#[test]
fn auto_dispatch_failure_marker_survives_a_backlog_refresh() {
    let mut app = make_app();
    fail_chain(&mut app, 1);

    app.update(Message::Task(TaskMessage::Updated(Box::new(make_task(
        1,
        TaskStatus::Backlog,
    )))));

    assert!(app.auto_dispatch_failed(TaskId(1)));
}

/// `@guarantee DistinctFromOrdinaryBacklog` in docs/specs/epics.allium — a
/// subtask released by a failed chain is otherwise byte-identical to one that
/// was never dispatched.
#[test]
fn stalled_chain_card_shows_auto_dispatch_failed() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    // The marker as the chain's failure leaves it, set directly so the card
    // stays an ordinary board card for the render (a chained subtask's own
    // path to it is `fail_chain`, above).
    app.handle_auto_dispatch_failed(TaskId(1), EpicId(7), "git fetch origin main failed".into());

    let buf = render_to_buffer(&mut app, 120, 20);

    assert!(
        buffer_contains(&buf, "\u{26a0} auto-dispatch failed"),
        "expected '⚠ auto-dispatch failed'"
    );
}

/// `@guarantee DispatchingOutranksIt`: retrying is the resolution, so the
/// retry's own spinner must not be masked by the failure it is resolving.
#[test]
fn stalled_chain_card_yields_to_the_dispatching_spinner() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Backlog)]);
    app.handle_auto_dispatch_failed(TaskId(1), EpicId(7), "git fetch origin main failed".into());
    // Re-insert the marker behind the spinner: a real retry clears it, so this
    // asserts the render precedence rather than the clearing.
    app.mark_dispatching(TaskId(1));
    app.agents
        .auto_dispatch_failed
        .insert(TaskId(1), "stale".to_string());

    let buf = render_to_buffer(&mut app, 120, 20);

    assert!(
        buffer_contains(&buf, "dispatching"),
        "an in-flight dispatch must still show its spinner"
    );
    assert!(
        !buffer_contains(&buf, "auto-dispatch failed"),
        "the stale failure must not mask the retry"
    );
}
