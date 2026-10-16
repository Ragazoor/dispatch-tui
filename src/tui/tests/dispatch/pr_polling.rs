use super::*;

/// The module has already moved the task to Done (`ReportPrStatus`); what is
/// left for the board is host work: kill the window read before the report,
/// notify and leave the worktree alone. The board writes no task field.
#[test]
fn pr_merged_kills_the_window_and_notifies_without_writing_the_task() {
    let mut task = make_task(1, TaskStatus::Review);
    task.url = Some(crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/42",
        crate::models::UrlType::Pr,
    ));
    let mut app = App::new(vec![task]);
    app.set_notifications_enabled(true);

    let cmds = app.update(Message::Pr(PrMessage::Merged {
        id: TaskId(1),
        window: Some(test_tmux_window("task-1")),
    }));

    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::Task(TaskCommand::KillTmuxWindow { .. }))));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::System(SystemCommand::SendNotification { .. }))));
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Command::Task(TaskCommand::Persist(_)))),
        "the module owns the task write, got: {cmds:?}"
    );
    let task = app.board.find_task(TaskId(1)).unwrap();
    assert!(task.worktree.is_some(), "worktree should be preserved");
    assert!(!app.status_message().unwrap_or_default().is_empty());
}

/// The store's Done row can reach the board before the message does.
#[test]
fn pr_merged_still_acts_when_the_store_row_arrived_first() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Done)]);

    let cmds = app.update(Message::Pr(PrMessage::Merged {
        id: TaskId(1),
        window: Some(test_tmux_window("task-1")),
    }));

    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::Task(TaskCommand::KillTmuxWindow { .. }))));
}

#[test]
fn pr_merged_ignores_a_task_still_running() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Running)]);

    let cmds = app.update(Message::Pr(PrMessage::Merged {
        id: TaskId(1),
        window: None,
    }));

    assert!(cmds.is_empty());
}

#[test]
fn pr_merged_preserves_worktree() {
    let mut task = make_task(1, TaskStatus::Review);
    task.url = Some(crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/42",
        crate::models::UrlType::Pr,
    ));
    let mut app = App::new(vec![task]);

    let cmds = app.update(Message::Pr(PrMessage::Merged {
        id: TaskId(1),
        window: None,
    }));

    // Should NOT emit a Cleanup command
    assert!(!cmds
        .iter()
        .any(|c| matches!(c, Command::Task(TaskCommand::Cleanup { .. }))));
}

#[test]
fn pr_closed_notifies_and_leaves_the_task_to_the_module() {
    let mut task = make_task(1, TaskStatus::Review);
    task.url = Some(crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/42",
        crate::models::UrlType::Pr,
    ));
    let mut app = App::new(vec![task]);
    app.set_notifications_enabled(true);

    let cmds = app.update(Message::Pr(PrMessage::Closed(TaskId(1))));

    let task = app.board.find_task(TaskId(1)).unwrap();
    assert_eq!(
        task.status,
        TaskStatus::Review,
        "task should stay in review"
    );
    assert_eq!(
        task.tmux_window.as_ref().map(|w| w.as_str()),
        Some("task-1"),
        "tmux window should not be torn down — the task isn't finishing"
    );
    assert!(task.worktree.is_some(), "worktree should be preserved");
    assert!(task.url.is_some(), "url should be preserved");
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Command::Task(TaskCommand::Persist(_)))),
        "the module sets pr_closed, the board writes nothing"
    );
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::System(SystemCommand::SendNotification { .. }))));
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Command::Task(TaskCommand::KillTmuxWindow { .. }))),
        "no tmux window teardown for a PR close — the task isn't done"
    );
}

/// The store's `pr_closed` row can reach the board before the message does;
/// the notice must not be lost to it.
#[test]
fn pr_closed_still_notifies_when_the_store_row_arrived_first() {
    let mut task = make_task(1, TaskStatus::Review);
    task.sub_status = SubStatus::PrClosed;
    let mut app = App::new(vec![task]);
    app.set_notifications_enabled(true);

    let cmds = app.update(Message::Pr(PrMessage::Closed(TaskId(1))));

    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::System(SystemCommand::SendNotification { .. }))));
}

#[test]
fn pr_closed_status_message_says_closed_not_merged() {
    let mut task = make_task(1, TaskStatus::Review);
    task.url = Some(crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/42",
        crate::models::UrlType::Pr,
    ));
    let mut app = App::new(vec![task]);

    app.update(Message::Pr(PrMessage::Closed(TaskId(1))));

    let status = app.status_message().unwrap_or_default();
    assert!(
        status.contains("closed"),
        "expected 'closed' in status bar, got: {status}"
    );
    assert!(
        !status.contains("merged"),
        "status bar should not say 'merged' for a closed PR, got: {status}"
    );
}

#[test]
fn pr_closed_no_notification_when_disabled() {
    let mut task = make_task(1, TaskStatus::Review);
    task.url = Some(crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/42",
        crate::models::UrlType::Pr,
    ));
    let mut app = App::new(vec![task]);
    // notifications_enabled is false by default in tests

    let cmds = app.update(Message::Pr(PrMessage::Closed(TaskId(1))));

    assert!(!cmds
        .iter()
        .any(|c| matches!(c, Command::System(SystemCommand::SendNotification { .. }))));
}

#[test]
fn pr_closed_ignores_non_review_task() {
    let mut app = App::new(vec![make_task(1, TaskStatus::Done)]);

    let cmds = app.update(Message::Pr(PrMessage::Closed(TaskId(1))));

    let task = app.board.find_task(TaskId(1)).unwrap();
    assert_eq!(task.status, TaskStatus::Done, "status should be unchanged");
    assert!(cmds.is_empty(), "no commands expected for non-review task");
}

#[test]
fn pr_polling_skips_done_tasks() {
    let mut task = make_task(1, TaskStatus::Done);
    task.url = Some(crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/42",
        crate::models::UrlType::Pr,
    ));
    let mut app = App::new(vec![task]);

    let cmds = app.update(Message::System(SystemMessage::Tick));
    // Should NOT contain any CheckPrStatus command
    assert!(!cmds
        .iter()
        .any(|c| matches!(c, Command::Pr(PrCommand::CheckStatus { .. }))));
}

#[test]
fn pr_polling_emits_check_for_review_tasks() {
    let mut task = make_task(1, TaskStatus::Review);
    task.url = Some(crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/42",
        crate::models::UrlType::Pr,
    ));
    let mut app = App::new(vec![task]);

    let cmds = app.update(Message::System(SystemMessage::Tick));
    // Host-less (no worktree — `make_task` leaves `host: None`), so this
    // routes through the ownership-checked command
    // (pr-workflow.allium: PollPrStatus, "Host scoping").
    assert!(cmds.iter().any(|c| matches!(c, Command::Pr(PrCommand::CheckStatusIfOwned { ref url, .. }) if url == "https://github.com/org/repo/pull/42")));
}

#[test]
fn pr_polling_only_targets_pr_typed_urls() {
    // PR-typed review task — should be polled.
    let mut pr_task = make_task(1, TaskStatus::Review);
    pr_task.url = Some(crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/42",
        crate::models::UrlType::Pr,
    ));
    // Issue-typed review task — must NOT be polled.
    let mut issue_task = make_task(2, TaskStatus::Review);
    issue_task.url = Some(crate::models::TaskUrl::new(
        "https://github.com/org/repo/issues/7",
        crate::models::UrlType::Issue,
    ));
    // Review task with no url — must NOT be polled.
    let url_less_task = make_task(3, TaskStatus::Review);

    let mut app = App::new(vec![pr_task, issue_task, url_less_task]);

    let cmds = app.update(Message::System(SystemMessage::Tick));

    let polled_ids: Vec<TaskId> = cmds
        .iter()
        .filter_map(|c| match c {
            // Host-less (no worktree), so this is `CheckStatusIfOwned` — see
            // `pr_polling_emits_check_for_review_tasks`.
            Command::Pr(PrCommand::CheckStatusIfOwned { id, .. }) => Some(*id),
            _ => None,
        })
        .collect();

    assert_eq!(polled_ids, vec![TaskId(1)]);
}

/// An open PR: the module sets the review sub-status
/// (`ReportPrStatus`), so the board does nothing but retire the failures.
#[test]
fn pr_review_state_writes_nothing_and_retires_failures() {
    let mut app = make_app();
    let id = TaskId(3);
    app.find_task_mut(id).unwrap().status = TaskStatus::Review;
    app.agents
        .pr_poll
        .entry(id)
        .or_default()
        .consecutive_transient_failures = 2;

    let cmds = app.update(Message::Pr(PrMessage::ReviewState(id)));

    assert!(cmds.is_empty());
    assert_eq!(
        app.agents
            .pr_poll
            .get(&id)
            .unwrap()
            .consecutive_transient_failures,
        0
    );
}

// --- PR poll give-up and backoff (pr-workflow.allium: PollPrStatus) ---

/// Build a review task carrying a pr-typed url, the shape `tick_pr_poll` polls.
fn review_task_with_pr(id: i64) -> crate::models::Task {
    let mut task = make_task(id, TaskStatus::Review);
    task.url = Some(crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/42",
        crate::models::UrlType::Pr,
    ));
    task
}

/// `review_task_with_pr` is host-less, so a poll surfaces as
/// `CheckStatusIfOwned`, not `CheckStatus` — matched here too so this helper
/// still answers "was this task polled at all" regardless of which command
/// carries it.
fn polled_ids(cmds: &[Command]) -> Vec<TaskId> {
    cmds.iter()
        .filter_map(|c| match c {
            Command::Pr(
                PrCommand::CheckStatus { id, .. } | PrCommand::CheckStatusIfOwned { id, .. },
            ) => Some(*id),
            _ => None,
        })
        .collect()
}

/// The whole point of the card: once polling has given up on a task, it stops
/// issuing calls. Before this, five unreadable PRs were polled every 30s for
/// five months.
#[test]
fn pr_polling_skips_a_task_it_gave_up_on() {
    let mut app = App::new(vec![review_task_with_pr(1)]);
    app.agents
        .pr_poll
        .entry(TaskId(1))
        .or_default()
        .consecutive_permanent_failures = crate::tui::PR_POLL_PERMANENT_FAILURE_THRESHOLD;

    let cmds = app.update(Message::System(SystemMessage::Tick));

    assert!(
        polled_ids(&cmds).is_empty(),
        "a task polling gave up on must not be polled again"
    );
}

/// A transient failure defers the next attempt rather than stopping it.
#[test]
fn pr_polling_skips_a_task_inside_its_backoff_window() {
    let mut app = App::new(vec![review_task_with_pr(1)]);
    app.agents
        .pr_poll
        .entry(TaskId(1))
        .or_default()
        .next_poll_at = Some(Instant::now() + Duration::from_secs(600));

    let cmds = app.update(Message::System(SystemMessage::Tick));

    assert!(
        polled_ids(&cmds).is_empty(),
        "a task still inside its backoff window must not be polled"
    );
}

/// A backoff deadline that has passed makes the task eligible again.
#[test]
fn pr_polling_resumes_once_the_backoff_deadline_passes() {
    let mut app = App::new(vec![review_task_with_pr(1)]);
    app.agents
        .pr_poll
        .entry(TaskId(1))
        .or_default()
        .next_poll_at = Some(Instant::now() - Duration::from_secs(1));

    let cmds = app.update(Message::System(SystemMessage::Tick));

    assert_eq!(polled_ids(&cmds), vec![TaskId(1)]);
}

/// Permanent failures below the threshold must not strand the task: a GitHub
/// incident briefly reporting a real repository as unresolvable is exactly the
/// case the threshold exists to absorb.
#[test]
fn a_permanent_failure_below_the_threshold_does_not_give_up() {
    let mut app = App::new(vec![review_task_with_pr(1)]);

    app.update(Message::Pr(PrMessage::CheckFailed {
        id: TaskId(1),
        permanent: true,
        error: "Could not resolve to a Repository".to_string(),
    }));

    let state = app.agents.pr_poll.get(&TaskId(1)).expect("state recorded");
    assert_eq!(state.consecutive_permanent_failures, 1);
    assert!(!state.gave_up(), "one failure must not give up");
    assert_eq!(
        app.board.find_task(TaskId(1)).unwrap().sub_status,
        SubStatus::AwaitingReview,
        "sub_status must not change before the threshold"
    );
}

/// At the threshold, polling gives up and the board says so.
#[test]
fn the_threshold_permanent_failure_gives_up_and_reports_the_pr_unreadable() {
    let mut app = App::new(vec![review_task_with_pr(1)]);

    let mut cmds = Vec::new();
    for _ in 0..crate::tui::PR_POLL_PERMANENT_FAILURE_THRESHOLD {
        cmds = app.update(Message::Pr(PrMessage::CheckFailed {
            id: TaskId(1),
            permanent: true,
            error: "Could not resolve to a Repository".to_string(),
        }));
    }

    let state = app.agents.pr_poll.get(&TaskId(1)).expect("state recorded");
    assert!(state.gave_up(), "the threshold failure must give up");
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Pr(PrCommand::ReportUnreadable { id: TaskId(1), .. })
        )),
        "giving up must tell the store, which marks the task pr_unreachable, got: {cmds:?}"
    );
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Command::Task(TaskCommand::Persist(_)))),
        "the module owns the task write, got: {cmds:?}"
    );
}

/// Transient failures must never accumulate towards a give-up, or a long GitHub
/// outage would strand every review task on the board.
#[test]
fn transient_failures_never_accumulate_towards_giving_up() {
    let mut app = App::new(vec![review_task_with_pr(1)]);

    for _ in 0..(crate::tui::PR_POLL_PERMANENT_FAILURE_THRESHOLD * 3) {
        app.update(Message::Pr(PrMessage::CheckFailed {
            id: TaskId(1),
            permanent: false,
            error: "error connecting to api.github.com".to_string(),
        }));
    }

    let state = app.agents.pr_poll.get(&TaskId(1)).expect("state recorded");
    assert_eq!(
        state.consecutive_permanent_failures, 0,
        "transient failures must not touch the permanent counter"
    );
    assert!(!state.gave_up(), "transient failures must never give up");
    assert!(
        state.next_poll_at.is_some(),
        "a transient failure must set a backoff deadline"
    );
}

/// The backoff widens rather than retrying at a fixed interval.
#[test]
fn successive_transient_failures_widen_the_backoff() {
    let mut app = App::new(vec![review_task_with_pr(1)]);
    let mut waits = Vec::new();

    for _ in 0..3 {
        let before = Instant::now();
        app.update(Message::Pr(PrMessage::CheckFailed {
            id: TaskId(1),
            permanent: false,
            error: "error connecting to api.github.com".to_string(),
        }));
        let deadline = app.agents.pr_poll[&TaskId(1)]
            .next_poll_at
            .expect("deadline set");
        waits.push(deadline.saturating_duration_since(before));
    }

    for pair in waits.windows(2) {
        assert!(
            pair[1] > pair[0],
            "each transient failure should wait longer than the last, got {waits:?}"
        );
    }
}

/// Recovery needs no user action on the task, because for the failure that
/// drives this card ("no account has access") there is none available.
#[test]
fn a_successful_poll_clears_give_up_state() {
    let mut app = App::new(vec![review_task_with_pr(1)]);
    let state = app.agents.pr_poll.entry(TaskId(1)).or_default();
    state.consecutive_permanent_failures = 9;
    state.next_poll_at = Some(Instant::now() + Duration::from_secs(600));

    app.update(Message::Pr(PrMessage::ReviewState(TaskId(1))));

    let state = app.agents.pr_poll.get(&TaskId(1)).expect("state kept");
    assert!(!state.gave_up(), "a successful poll must clear the give-up");
    assert_eq!(state.consecutive_permanent_failures, 0);
    assert!(state.next_poll_at.is_none(), "backoff must be cleared");
}
