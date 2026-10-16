use super::*;

// -- store-split M7 (task #53260): close_task, patch_task's transition rules and
// ensure_managed_epics ----------------------------------------------------------
//
// `store-split.allium`: RequestCloseTask (X1), X2 and X4. As for M5, the unit
// tests in `tests.rs` pin the pure rules; here it is the transaction.

fn close_task(instance: &Instance, task: i64, host: &str) -> std::process::Output {
    instance.call("close_task", &[&task.to_string(), host])
}

/// A running task with a window closes whole: Done, window cleared, completed,
/// the epic rolled up, and ONE close row with its timer; no token is touched.
#[test]
fn close_task_applies_the_done_close_and_writes_a_close_row() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    set_task_field(&instance, 1, "stop_pending", serde_json::json!(true));

    let closed = close_task(&instance, 1, "host-b");
    assert!(closed.status.success(), "{}", describe(&closed));

    assert_eq!(task_column(&instance, "status", 1), "done");
    assert_eq!(task_column(&instance, "sub_status", 1), "none");
    assert_eq!(task_column(&instance, "tmux_window", 1), "");
    assert_eq!(task_column(&instance, "completed_at", 1).len(), 23);
    assert_eq!(task_column(&instance, "stop_pending", 1), "false");
    assert_eq!(
        column(&instance, "SELECT status FROM epics WHERE id = 1"),
        "done"
    );

    assert_eq!(session_count(&instance), "1");
    assert_eq!(session_column(&instance, "state", 1, "close"), "requested");
    assert_eq!(session_column(&instance, "action", 1, "close"), "done");
    assert_eq!(
        session_column(&instance, "host", 1, "close"),
        "host-a",
        "the task's own host, not the argument"
    );
    assert_eq!(
        session_column(&instance, "tmux_window", 1, "close"),
        "task-1"
    );
    assert_eq!(
        session_column(&instance, "wrap_up_request", 1, "close"),
        "0"
    );
    assert_eq!(session_column(&instance, "chained_task", 1, "close"), "0");
    assert_eq!(session_column(&instance, "pr_url", 1, "close"), "");
    assert_eq!(
        session_column(&instance, "requested_by", 1, "close"),
        instance.identity()
    );
    assert_eq!(session_column(&instance, "deadline", 1, "close").len(), 23);
    let id = session_column(&instance, "id", 1, "close");
    assert_eq!(session_timer_count(&instance, &id), "1");
}

/// The teardown of a close row is served like an exit_session's: claim, done,
/// and the timer goes.
#[test]
fn a_close_row_is_claimed_and_reported_like_a_teardown() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    assert!(close_task(&instance, 1, "host-a").status.success());
    let id = session_column(&instance, "id", 1, "close");

    let claimed = instance.call("claim_session_request", &[&id, "host-a"]);
    assert!(claimed.status.success(), "{}", describe(&claimed));
    let done = instance.call("report_session_request_done", &[&id, "host-a"]);
    assert!(done.status.success(), "{}", describe(&done));
    assert_eq!(session_column(&instance, "state", 1, "close"), "done");
    assert_eq!(session_timer_count(&instance, &id), "0");
    assert_eq!(task_column(&instance, "status", 1), "done");
}

/// No window and no pr url: nothing for a host to do, so no row.
#[test]
fn close_task_without_host_work_writes_no_row() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "review", "")]);
    set_task_field(&instance, 1, "tmux_window", serde_json::json!(""));

    let closed = close_task(&instance, 1, "host-a");
    assert!(closed.status.success(), "{}", describe(&closed));
    assert_eq!(task_column(&instance, "status", 1), "done");
    assert_eq!(session_count(&instance), "0");
}

/// A pr-typed url alone is host work, on the caller's host when the task has none.
#[test]
fn close_task_with_a_pr_url_and_no_host_writes_a_row_on_the_argument_host() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "review", "")]);
    set_task_field(&instance, 1, "tmux_window", serde_json::json!(""));
    set_task_field(&instance, 1, "host", serde_json::json!(""));
    set_task_field(
        &instance,
        1,
        "url",
        serde_json::json!("https://github.com/a/b/pull/1"),
    );
    set_task_field(&instance, 1, "url_type", serde_json::json!("pr"));

    let closed = close_task(&instance, 1, "host-b");
    assert!(closed.status.success(), "{}", describe(&closed));
    assert_eq!(session_column(&instance, "host", 1, "close"), "host-b");
    assert_eq!(session_column(&instance, "tmux_window", 1, "close"), "");
}

/// D11: a teammate's host is not the caller's to tear down. The task still closes.
#[test]
fn close_task_on_a_teammates_host_closes_without_a_row() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    register_teammates_host(&instance, "their-host");
    set_task_field(&instance, 1, "host", serde_json::json!("their-host"));

    let closed = close_task(&instance, 1, "host-a");
    assert!(closed.status.success(), "{}", describe(&closed));
    assert_eq!(task_column(&instance, "status", 1), "done");
    assert_eq!(task_column(&instance, "tmux_window", 1), "");
    assert_eq!(session_count(&instance), "0");
}

/// A stuck wrap_up does not block closing: it is failed "task closed", its
/// timer cancelled, and the close row written after.
#[test]
fn close_task_fails_the_open_session_request() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    assert!(request_wrap_up(&instance, 1, "host-a", "rebase")
        .status
        .success());
    let wrap = session_column(&instance, "id", 1, "wrap_up");
    assert_eq!(session_timer_count(&instance, &wrap), "1");

    let closed = close_task(&instance, 1, "host-a");
    assert!(closed.status.success(), "{}", describe(&closed));

    assert_eq!(session_column(&instance, "state", 1, "wrap_up"), "failed");
    assert_eq!(
        session_column(&instance, "reason", 1, "wrap_up"),
        "task closed"
    );
    assert_eq!(session_timer_count(&instance, &wrap), "0");
    assert_eq!(session_column(&instance, "state", 1, "close"), "requested");
    // A late report from the failed rebase is refused.
    let late = instance.call("report_session_request_done", &[&wrap, "host-a"]);
    assert!(!late.status.success(), "{}", describe(&late));
}

/// An earlier close still open is failed by a re-close, so at most one is open.
#[test]
fn a_reclose_fails_the_earlier_open_close_row_and_keeps_completed_at() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    assert!(close_task(&instance, 1, "host-a").status.success());
    let first = session_column(&instance, "id", 1, "close");
    let completed = task_column(&instance, "completed_at", 1);
    // Give the task a window again, as a hand-edit would, so the re-close has work.
    set_task_field(&instance, 1, "tmux_window", serde_json::json!("task-1"));

    assert!(close_task(&instance, 1, "host-a").status.success());

    assert_eq!(task_column(&instance, "completed_at", 1), completed);
    assert_eq!(
        column(
            &instance,
            &format!("SELECT state FROM session_requests WHERE id = {first}")
        ),
        "failed"
    );
    assert_eq!(session_timer_count(&instance, &first), "0");
    assert_eq!(
        column(
            &instance,
            "SELECT COUNT(*) AS n FROM session_requests WHERE state = 'requested'"
        ),
        "1"
    );
}

/// No token and no chain: an auto-dispatch epic's backlog subtask is not requested.
#[test]
fn close_task_runs_no_epic_chain() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, true, &[(1, "running", "")]);
    seed_backlog_subtask(&instance, 2, 1, false);

    assert!(close_task(&instance, 1, "host-a").status.success());

    assert_eq!(
        column(&instance, "SELECT COUNT(*) AS n FROM dispatch_requests"),
        "0"
    );
    assert_eq!(task_column(&instance, "status", 2), "backlog");
}

#[test]
fn close_task_of_a_missing_task_is_refused() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    let refused = close_task(&instance, 99, "host-a");
    assert!(!refused.status.success(), "{}", describe(&refused));
    assert_eq!(session_count(&instance), "0");
}

/// PhoenixRespawn on a close that enters Done; the flag is the receipt, and a
/// done -> done re-close does not respawn again.
#[test]
fn close_task_respawns_a_phoenix_task_once() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    // A respawn inserts a task, so the id counter must be past the seeded ids.
    let burned = instance.call("burn_id_sequence", &["tasks", "10"]);
    assert!(burned.status.success(), "{}", describe(&burned));
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    set_task_field(&instance, 1, "phoenix", serde_json::json!(true));
    let count = || column(&instance, "SELECT COUNT(*) AS n FROM tasks");
    assert_eq!(count(), "1");

    assert!(close_task(&instance, 1, "host-a").status.success());
    assert_eq!(count(), "2");
    assert_eq!(task_column(&instance, "phoenix", 1), "false");

    assert!(close_task(&instance, 1, "host-a").status.success());
    assert_eq!(count(), "2");
}

/// X2: a patch out of running clears stop_pending, unless it names the field.
#[test]
fn patch_task_leaving_running_clears_stop_pending() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", ""), (2, "running", "")]);
    set_task_field(&instance, 1, "stop_pending", serde_json::json!(true));
    set_task_field(&instance, 2, "stop_pending", serde_json::json!(true));

    set_task_field(&instance, 1, "status", serde_json::json!("review"));
    assert_eq!(task_column(&instance, "stop_pending", 1), "false");

    let mut patch = empty_task_patch();
    patch["status"] = serde_json::json!({"some": "review"});
    patch["stop_pending"] = serde_json::json!({"some": true});
    let out = instance.call("patch_task", &["2", &patch.to_string()]);
    assert!(out.status.success(), "{}", describe(&out));
    assert_eq!(task_column(&instance, "stop_pending", 2), "true");
}

/// X2: a patch into Done from another status respawns a phoenix task, in the
/// same transaction; a feed-owned task and a patch that stays done do not.
#[test]
fn patch_task_into_done_respawns_a_phoenix_task() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    // A respawn inserts a task, so the id counter must be past the seeded ids.
    let burned = instance.call("burn_id_sequence", &["tasks", "10"]);
    assert!(burned.status.success(), "{}", describe(&burned));
    seed_wrappable(&instance, false, &[(1, "running", ""), (2, "running", "")]);
    set_task_field(&instance, 1, "phoenix", serde_json::json!(true));
    set_task_field(&instance, 2, "phoenix", serde_json::json!(true));
    set_task_field(&instance, 2, "external_id", serde_json::json!("ext-2"));
    let count = || column(&instance, "SELECT COUNT(*) AS n FROM tasks");

    set_task_field(&instance, 1, "status", serde_json::json!("done"));
    assert_eq!(count(), "3");
    assert_eq!(task_column(&instance, "phoenix", 1), "false");
    assert_eq!(
        column(
            &instance,
            "SELECT status FROM tasks WHERE title = 't' AND id > 10"
        ),
        "backlog"
    );

    set_task_field(&instance, 1, "title", serde_json::json!("renamed"));
    assert_eq!(count(), "3", "no transition, no respawn");

    set_task_field(&instance, 2, "status", serde_json::json!("done"));
    assert_eq!(count(), "3", "a feed-owned task is exempt");
}
