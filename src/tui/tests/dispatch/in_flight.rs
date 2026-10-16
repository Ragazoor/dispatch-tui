use super::*;

#[test]
fn tab_hint_absent_from_tab_bar() {
    let mut app = make_app();
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(
        find_style_of(&buf, "[Tab]").is_none(),
        "[Tab] hint must not appear in the board view"
    );
}

#[test]
fn dispatch_in_flight_blocks_second_dispatch() {
    let mut app = make_app();
    // First dispatch succeeds
    let cmds = app.update(Message::Task(TaskMessage::Dispatch(
        TaskId(1),
        DispatchMode::Dispatch,
    )));
    assert!(matches!(
        cmds[0],
        Command::Task(TaskCommand::DispatchAgent { .. })
    ));
    // Second dispatch of same task is blocked
    let cmds = app.update(Message::Task(TaskMessage::Dispatch(
        TaskId(1),
        DispatchMode::Dispatch,
    )));
    assert!(cmds.is_empty());
}

#[test]
fn dispatched_clears_in_flight() {
    let mut app = make_app();
    // Dispatch task 1
    app.update(Message::Task(TaskMessage::Dispatch(
        TaskId(1),
        DispatchMode::Dispatch,
    )));
    // Dispatched message clears the in-flight guard
    app.update(Message::Task(TaskMessage::Dispatched {
        id: TaskId(1),
        worktree: "/wt".to_string(),
        tmux_window: test_tmux_window("win"),
        switch_focus: false,
    }));
    // Task is now Running, so dispatch is a no-op for a different reason,
    // but the in-flight set should be clear
    assert!(!app.is_dispatching(TaskId(1)));
}

/// The in-flight guard follows the request row: once the row fails, the task
/// is no longer a member and can be dispatched again
/// (`dispatch.allium: DispatchingFeedback`, ClearedWhenEmpty).
#[test]
fn a_failed_request_clears_in_flight() {
    let mut app = make_app();
    app.update(Message::Task(TaskMessage::Dispatch(
        TaskId(1),
        DispatchMode::Dispatch,
    )));
    assert!(app.is_dispatching(TaskId(1)));
    app.update(requests_refreshed(vec![request_row(
        1,
        1,
        crate::models::DispatchRequestState::Requested,
    )]));
    app.update(requests_refreshed(vec![request_row(
        1,
        1,
        crate::models::DispatchRequestState::Failed,
    )]));
    assert!(!app.is_dispatching(TaskId(1)));
    // Can dispatch again
    let cmds = app.update(Message::Task(TaskMessage::Dispatch(
        TaskId(1),
        DispatchMode::Dispatch,
    )));
    assert!(matches!(
        cmds[0],
        Command::Task(TaskCommand::DispatchAgent { .. })
    ));
}

#[test]
fn dispatch_different_tasks_both_succeed() {
    let mut app = make_app();
    // Dispatch task 1
    let cmds = app.update(Message::Task(TaskMessage::Dispatch(
        TaskId(1),
        DispatchMode::Dispatch,
    )));
    assert!(matches!(
        cmds[0],
        Command::Task(TaskCommand::DispatchAgent { .. })
    ));
    // Dispatch task 2 — different task, should succeed
    let cmds = app.update(Message::Task(TaskMessage::Dispatch(
        TaskId(2),
        DispatchMode::Dispatch,
    )));
    assert!(matches!(
        cmds[0],
        Command::Task(TaskCommand::DispatchAgent { .. })
    ));
}

// `dispatch_failed_clears_mark_dispatching_guard` and
// `dispatch_failed_releases_the_claim` pinned `TaskMessage::DispatchFailed` and
// the `ReleaseClaim` it emitted, both retired by store-split M4: a failed
// dispatch reaches the board as its failed request row (pinned above and in
// `src/tui/tests/dispatch_requests.rs`), and no client releases a claim — the
// module does, for every failed request (`store-split.allium:
// ReleaseClaimOfFailedRequest`).

/// A *refused* request must drain the spinner and do nothing else
/// (`dispatch.allium: LostClaimReported`).
///
/// The winner of a contested claim is itself Running with no worktree while it
/// provisions. A loser that wrote anything to the task could hand the winner's
/// task back to Backlog mid-provision and invite a second dispatch of the same
/// work. The loser wrote nothing and holds nothing, so it drains its spinner
/// and stops there.
#[test]
fn dispatch_claim_lost_clears_the_spinner_without_releasing() {
    let mut app = make_app();
    app.update(Message::Task(TaskMessage::MarkDispatching(TaskId(1))));

    let cmds = app.update(Message::Task(TaskMessage::DispatchAbandoned(TaskId(1))));

    assert!(
        !app.is_dispatching(TaskId(1)),
        "a lost claim must still drain the spinner"
    );
    assert!(
        cmds.is_empty(),
        "the loser owns no claim and must touch nothing, got: {cmds:?}"
    );
}

// The in-memory watchdog tests that stood here
// (`dispatch_watchdog_timeout_matches_provision_worst_case`,
// `dispatching_timeout_does_not_release_the_claim`) pinned
// `DispatchingTimeout`, which store-split M4 REPLACES with the request row's
// own deadline (`dispatch.allium: FailedRequestReported`). Its successors are in
// `src/tui/tests/dispatch_requests.rs`.

/// The claim flips the task to Running before provisioning finishes, so the
/// "dispatching…" indicator has to survive that transition. Before the claim
/// existed, a member of `dispatching` was always Backlog; now it is Backlog
/// (pre-claim) or Running-without-worktree (claimed, being provisioned), and the
/// indicator must win over the status-derived ones in both cases or an in-flight
/// dispatch would render as a live agent (`SpansTheClaim` in
/// docs/specs/dispatch.allium).
#[test]
fn claimed_task_still_renders_the_dispatching_indicator() {
    let mut task = make_task(1, TaskStatus::Running);
    task.worktree = None;
    task.tmux_window = None;
    let mut app = App::new(vec![task]);
    app.update(Message::Task(TaskMessage::MarkDispatching(TaskId(1))));

    let buf = render_to_buffer(&mut app, 120, 40);
    assert!(
        buffer_contains(&buf, "dispatching"),
        "a claimed-but-unprovisioned task must still show 'dispatching', not a running indicator"
    );
}

// ---------------------------------------------------------------------------
// Sticky dispatching status — handler-level coverage (Task #500)
// ---------------------------------------------------------------------------

#[test]
fn handle_dispatch_task_sets_sticky_status() {
    let mut app = make_app(); // task 1 is Backlog with title "Task 1"
    let cmds = app.update(Message::Task(TaskMessage::Dispatch(
        TaskId(1),
        DispatchMode::Dispatch,
    )));

    // The DispatchAgent command is still produced.
    assert!(matches!(
        cmds[0],
        Command::Task(TaskCommand::DispatchAgent { .. })
    ));
    // And the sticky status is set so the user sees feedback while
    // git fetch / worktree creation runs in spawn_blocking.
    let msg = app.status.message.as_deref().expect("sticky status set");
    assert!(msg.contains("Dispatching"), "got: {msg}");
    assert!(msg.contains("Task 1"), "got: {msg}");
    assert!(app.status.message_sticky);
    assert!(app.is_dispatching(TaskId(1)));
}

#[test]
fn handle_dispatch_task_rejects_when_already_dispatching() {
    let mut app = make_app();
    // First dispatch sets the in-flight guard
    let cmds = app.update(Message::Task(TaskMessage::Dispatch(
        TaskId(1),
        DispatchMode::Dispatch,
    )));
    assert!(matches!(
        cmds[0],
        Command::Task(TaskCommand::DispatchAgent { .. })
    ));

    // Second dispatch of the same task is debounced — no new command.
    let cmds = app.update(Message::Task(TaskMessage::Dispatch(
        TaskId(1),
        DispatchMode::Dispatch,
    )));
    assert!(cmds.is_empty(), "second dispatch should be rejected");
    // Sticky status remains set (we're still in flight).
    assert!(app.status.message_sticky);
}

// `dispatching_timeout_clears_stuck_task` (the watchdog force-fail) is
// superseded by `a_tick_never_force_fails_a_dispatch` and the
// FailedRequestReported tests in `src/tui/tests/dispatch_requests.rs`.

#[test]
fn dispatching_start_time_recorded_on_mark() {
    let mut app = make_app();
    app.update(Message::Task(TaskMessage::MarkDispatching(TaskId(1))));
    assert!(
        app.dispatching.contains(&TaskId(1)),
        "mark_dispatching should record the start time for the watchdog"
    );
}

#[test]
fn dispatching_start_time_cleared_on_dispatched() {
    let mut app = make_app();
    app.update(Message::Task(TaskMessage::MarkDispatching(TaskId(1))));
    assert!(app.dispatching.contains(&TaskId(1)));

    app.update(Message::Task(TaskMessage::Dispatched {
        id: TaskId(1),
        worktree: "/wt".to_string(),
        tmux_window: test_tmux_window("win-1"),
        switch_focus: false,
    }));

    assert!(
        !app.dispatching.contains(&TaskId(1)),
        "Dispatched should remove the task from the dispatching map"
    );
}

#[test]
fn dispatching_card_renders_with_indicator_text() {
    let mut app = make_app(); // task 1 is Backlog
    app.update(Message::Task(TaskMessage::MarkDispatching(TaskId(1))));

    let buf = render_to_buffer(&mut app, 120, 40);
    assert!(
        buffer_contains(&buf, "dispatching"),
        "Backlog card for task 1 should show the 'dispatching' indicator while in flight"
    );
}

#[test]
fn spinner_tick_advances_only_on_tick_when_dispatching_nonempty() {
    let mut app = make_app();
    app.update(Message::Task(TaskMessage::MarkDispatching(TaskId(1))));
    let before = app.spinner_tick;

    app.update(Message::System(SystemMessage::Tick));

    assert_ne!(
        app.spinner_tick, before,
        "spinner_tick should advance on Tick while dispatching is non-empty"
    );
}

#[test]
fn spinner_tick_does_not_advance_when_dispatching_empty() {
    let mut app = make_app();
    let before = app.spinner_tick;

    app.update(Message::System(SystemMessage::Tick));

    assert_eq!(
        app.spinner_tick, before,
        "spinner_tick should stay frozen on Tick when dispatching is empty"
    );
}

#[test]
fn spinner_tick_wraps_modulo_ten() {
    let mut app = make_app();
    app.update(Message::Task(TaskMessage::MarkDispatching(TaskId(1))));

    // 10 ticks should bring us back to the starting frame.
    let start = app.spinner_tick;
    for _ in 0..10 {
        app.update(Message::System(SystemMessage::Tick));
    }
    assert_eq!(
        app.spinner_tick, start,
        "spinner_tick should wrap mod 10 so the rendered glyph cycles"
    );
}
