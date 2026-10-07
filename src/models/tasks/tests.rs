use super::*;

fn ts(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(seconds, 0).unwrap()
}

#[test]
fn entering_done_stamps_the_completion_time() {
    let now = ts(1_700_000_000);
    let result = completed_at_for_status_transition(TaskStatus::Review, TaskStatus::Done, now);
    assert_eq!(result, Some(now));
}

/// `completed_at` records the last completion, not the current status, so
/// leaving Done must not clear it. This is the deliberate difference from
/// the `sort_order` completion rank it replaced — see `ConfirmDone` in
/// `docs/specs/tasks.allium`.
#[test]
fn leaving_done_writes_nothing() {
    let now = ts(1_700_000_000);
    for next in [TaskStatus::Review, TaskStatus::Running, TaskStatus::Backlog] {
        assert_eq!(
            completed_at_for_status_transition(TaskStatus::Done, next, now),
            None,
            "done -> {next:?} must leave completed_at alone"
        );
    }
}

#[test]
fn staying_in_done_is_untouched() {
    let now = ts(1_700_000_000);
    let result = completed_at_for_status_transition(TaskStatus::Done, TaskStatus::Done, now);
    assert_eq!(result, None);
}

#[test]
fn staying_outside_done_is_untouched() {
    let now = ts(1_700_000_000);
    let result = completed_at_for_status_transition(TaskStatus::Backlog, TaskStatus::Running, now);
    assert_eq!(result, None);
}

/// Every non-Done prior status stamps.
#[test]
fn every_route_into_done_stamps() {
    let now = ts(1_700_000_000);
    for prior in [TaskStatus::Backlog, TaskStatus::Running, TaskStatus::Review] {
        assert_eq!(
            completed_at_for_status_transition(prior, TaskStatus::Done, now),
            Some(now),
            "{prior:?} -> done must stamp"
        );
    }
}

#[test]
fn leaving_running_clears_the_pending_stop() {
    for next in [TaskStatus::Review, TaskStatus::Backlog, TaskStatus::Done] {
        assert!(
            clears_pending_stop(TaskStatus::Running, next),
            "running -> {next:?} must void a deferred Stop"
        );
    }
}

#[test]
fn a_transition_that_does_not_leave_running_keeps_the_pending_stop() {
    // Arriving in Running is not a clear point either: only HookStop sets
    // the bit and it requires Running, so there is nothing to clear on the
    // way in.
    for (prior, next) in [
        (TaskStatus::Running, TaskStatus::Running),
        (TaskStatus::Backlog, TaskStatus::Running),
        (TaskStatus::Review, TaskStatus::Running),
        (TaskStatus::Review, TaskStatus::Done),
    ] {
        assert!(
            !clears_pending_stop(prior, next),
            "{prior:?} -> {next:?} must not void a deferred Stop"
        );
    }
}

fn done_at(seconds: i64) -> Task {
    Task {
        status: TaskStatus::Done,
        completed_at: Some(ts(seconds)),
        ..Default::default()
    }
}

/// The Done column reads newest-first, so folding keeps the MAXIMUM.
#[test]
fn folding_completions_keeps_the_newest() {
    let best = fold_newest_completion(None, &done_at(100));
    assert_eq!(best, Some(ts(100)));
    let best = fold_newest_completion(best, &done_at(300));
    assert_eq!(best, Some(ts(300)), "the newer completion wins");
    let best = fold_newest_completion(best, &done_at(200));
    assert_eq!(best, Some(ts(300)), "an older one does not displace it");
}

/// A task that is not Done never feeds the key, however recent its stamp —
/// a task that left Done keeps `completed_at`, and the Done column is not
/// showing it.
#[test]
fn folding_ignores_a_task_that_is_not_done() {
    let stale = Task {
        status: TaskStatus::Running,
        completed_at: Some(ts(900)),
        ..Default::default()
    };
    assert_eq!(fold_newest_completion(Some(ts(100)), &stale), Some(ts(100)));
    assert_eq!(fold_newest_completion(None, &stale), None);
}

/// A Done task with no completion time leaves the running best untouched,
/// in both directions.
#[test]
fn folding_ignores_an_undated_done_task() {
    let undated = Task {
        status: TaskStatus::Done,
        completed_at: None,
        ..Default::default()
    };
    assert_eq!(
        fold_newest_completion(Some(ts(100)), &undated),
        Some(ts(100))
    );
    assert_eq!(fold_newest_completion(None, &undated), None);
}

/// Pins the exempt set against `board-layout.allium`'s
/// `FlattenedView.unflattened_statuses`. Changing one without the other is
/// a behaviour change, so make it fail here rather than drift silently.
#[test]
fn unflattened_is_backlog_alone() {
    assert_eq!(TaskStatus::UNFLATTENED, &[TaskStatus::Backlog]);
    for status in TaskStatus::ALL {
        assert_eq!(
            status.is_unflattened(),
            matches!(status, TaskStatus::Backlog),
            "{status:?}"
        );
    }
}
