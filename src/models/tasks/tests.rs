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

mod default_tests {
    use super::*;

    #[test]
    fn default_task_has_sensible_placeholder_values() {
        let task = Task::default();
        assert_eq!(task.id, TaskId(0));
        assert_eq!(task.title, "");
        assert_eq!(task.description, "");
        assert_eq!(task.repo_path, "/repo");
        assert_eq!(task.status, TaskStatus::Backlog);
        assert_eq!(task.sub_status, SubStatus::None);
        assert_eq!(task.base_branch, "main");
        assert!(task.labels.is_empty());
        assert!(task.worktree.is_none());
        assert!(task.tmux_window.is_none());
        assert!(task.host.is_none());
        assert!(task.plan_path.is_none());
        assert!(task.epic_id.is_none());
        assert!(task.url.is_none());
        assert!(task.tag.is_none());
        assert!(task.sort_order.is_none());
        assert!(task.external_id.is_none());
        assert!(task.last_pre_tool_use_at.is_none());
        assert!(task.last_notification_at.is_none());
        assert!(task.last_peer_message_sent_at.is_none());
        assert!(task.last_peer_message_received_at.is_none());
        assert!(task.wrap_up_mode.is_none());
        assert!(!task.auto_run_plan);
        assert!(!task.phoenix);
        assert_eq!(task.live_subagents, 0);
        assert!(!task.stop_pending);
    }
}

// ---------------------------------------------------------------------------
// Task::is_locally_owned (task #4812 distributed-dispatch foundations)
// ---------------------------------------------------------------------------
//
// Mirrors core/Task::is_locally_owned in docs/specs/core.allium: true when
// `host` is null (nothing to conflict over) or names this machine, false
// only when it names a different one.
mod is_locally_owned_tests {
    use super::*;

    #[test]
    fn no_host_is_locally_owned() {
        let task = Task {
            host: None,
            ..Task::default()
        };
        assert!(task.is_locally_owned(Some("this-machine")));
    }

    #[test]
    fn host_matching_local_id_is_locally_owned() {
        let task = Task {
            host: Some("this-machine".to_string()),
            ..Task::default()
        };
        assert!(task.is_locally_owned(Some("this-machine")));
    }

    #[test]
    fn host_naming_another_machine_is_not_locally_owned() {
        let task = Task {
            host: Some("other-machine".to_string()),
            ..Task::default()
        };
        assert!(!task.is_locally_owned(Some("this-machine")));
    }

    #[test]
    fn a_held_task_is_not_locally_owned_when_this_install_has_no_id_yet() {
        let task = Task {
            host: Some("some-machine".to_string()),
            ..Task::default()
        };
        assert!(!task.is_locally_owned(None));
    }

    #[test]
    fn an_unheld_task_is_locally_owned_even_when_this_install_has_no_id_yet() {
        let task = Task {
            host: None,
            ..Task::default()
        };
        assert!(task.is_locally_owned(None));
    }
}

mod wrap_up_mode_tests {
    use super::*;

    #[test]
    fn wrap_up_mode_roundtrip() {
        for mode in [WrapUpMode::Rebase, WrapUpMode::Pr, WrapUpMode::Done] {
            let s = mode.as_str();
            let parsed = WrapUpMode::parse(s).expect("parse should succeed");
            assert_eq!(parsed, mode);
        }
    }

    /// `WrapUpMode::ALL` backs the create_task/update_task MCP schema's
    /// wrap_up_mode enum (dispatch.rs) — a variant added there without
    /// updating `ALL` would silently under-advertise it.
    #[test]
    fn wrap_up_mode_all_has_every_variant() {
        assert_eq!(WrapUpMode::ALL.len(), 3);
    }

    #[test]
    fn wrap_up_mode_from_str() {
        assert_eq!("rebase".parse::<WrapUpMode>().unwrap(), WrapUpMode::Rebase);
        assert_eq!("pr".parse::<WrapUpMode>().unwrap(), WrapUpMode::Pr);
        assert_eq!("done".parse::<WrapUpMode>().unwrap(), WrapUpMode::Done);
        assert!("unknown".parse::<WrapUpMode>().is_err());
    }

    #[test]
    fn wrap_up_mode_display() {
        assert_eq!(WrapUpMode::Rebase.to_string(), "rebase");
        assert_eq!(WrapUpMode::Pr.to_string(), "pr");
        assert_eq!(WrapUpMode::Done.to_string(), "done");
    }
}
