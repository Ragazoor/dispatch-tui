use super::*;

// -- store-split M5 (task #53258): wrap-up and exit-session as request rows ---
//
// `store-split.allium`: SessionRequest and its rules (W1-W13). As for M4, what
// a unit test can pin without a store (refusal wording, the token rules, the
// close's writes, the backstop's body on a row whose deadline has passed) is
// pinned in the module's own `tests.rs` ("session_requests"); here it is the
// transaction: that the close, the token's consumption, the epic chain and
// the exit_session row land together, that a refusal writes nothing and leaves
// the token valid, and that the scheduled timer row is created and cancelled
// with its request. THE BACKSTOP ITSELF IS NOT FIRED HERE (no wall-clock waits,
// `scripts/check-no-test-sleep.sh`).

fn request_exit_session(
    instance: &Instance,
    task: i64,
    token: &str,
    action: &str,
    pr_url: &str,
) -> std::process::Output {
    instance.call(
        "request_exit_session",
        &[&task.to_string(), token, action, pr_url],
    )
}

/// wrap_up for `task` through to a done row, as a board does it: written done
/// for done and pr; claimed and reported done for rebase. Returns the token.
fn wrap_up_to_token(instance: &Instance, task: i64, action: &str) -> String {
    let requested = request_wrap_up(instance, task, "host-a", action);
    assert!(requested.status.success(), "{}", describe(&requested));
    let id = session_column(instance, "id", task, "wrap_up");
    if action == "rebase" {
        let claimed = instance.call("claim_session_request", &[&id, "host-a"]);
        assert!(claimed.status.success(), "{}", describe(&claimed));
        let done = instance.call("report_session_request_done", &[&id, "host-a"]);
        assert!(done.status.success(), "{}", describe(&done));
    }
    id
}

/// `RequestWrapUp`, rebase: a requested row on the task's host with its
/// deadline and one backstop timer, and no write to the task.
#[test]
fn a_rebase_wrap_up_writes_a_requested_row_with_a_timer_and_leaves_the_task() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    let before = task_column(&instance, "updated_at", 1);

    let requested = request_wrap_up(&instance, 1, "host-a", "rebase");
    assert!(requested.status.success(), "{}", describe(&requested));

    assert_eq!(session_count(&instance), "1");
    assert_eq!(session_column(&instance, "kind", 1, "wrap_up"), "wrap_up");
    assert_eq!(session_column(&instance, "action", 1, "wrap_up"), "rebase");
    assert_eq!(session_column(&instance, "host", 1, "wrap_up"), "host-a");
    assert_eq!(
        session_column(&instance, "state", 1, "wrap_up"),
        "requested"
    );
    assert_eq!(
        session_column(&instance, "requested_by", 1, "wrap_up"),
        instance.identity()
    );
    assert_eq!(
        session_column(&instance, "requested_at", 1, "wrap_up").len(),
        23
    );
    assert_eq!(
        session_column(&instance, "deadline", 1, "wrap_up").len(),
        23
    );
    assert_eq!(session_column(&instance, "claimed_at", 1, "wrap_up"), "");
    assert_eq!(session_column(&instance, "finished_at", 1, "wrap_up"), "");
    let id = session_column(&instance, "id", 1, "wrap_up");
    assert_eq!(session_timer_count(&instance, &id), "1");
    assert_eq!(task_column(&instance, "updated_at", 1), before);
    assert_eq!(task_column(&instance, "status", 1), "running");
}

/// done and pr: the row is written already done in the same call, is the
/// token at once, and has no timer.
#[test]
fn a_done_or_pr_wrap_up_is_written_already_done_without_a_timer() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", ""), (2, "running", "")]);

    for (task, action) in [(1, "done"), (2, "pr")] {
        let requested = request_wrap_up(&instance, task, "host-a", action);
        assert!(requested.status.success(), "{}", describe(&requested));
        assert_eq!(session_column(&instance, "state", task, "wrap_up"), "done");
        assert_eq!(session_column(&instance, "action", task, "wrap_up"), action);
        assert_eq!(
            session_column(&instance, "finished_at", task, "wrap_up").len(),
            23
        );
        assert_eq!(session_column(&instance, "claimed_at", task, "wrap_up"), "");
        let id = session_column(&instance, "id", task, "wrap_up");
        assert_eq!(session_timer_count(&instance, &id), "0");
    }
}

/// Each refusal names why and writes nothing.
#[test]
fn a_refused_wrap_up_request_writes_nothing() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(
        &instance,
        false,
        &[
            (1, "backlog", ""),
            (2, "running", "pr-review"),
            (3, "running", "dependabot"),
            (4, "running", ""),
            (5, "done", ""),
            (6, "running", "release-please"),
        ],
    );

    for (task, host, why) in [
        (1, "host-a", "cannot be wrapped up"),
        (2, "host-a", "retag"),
        (3, "host-a", "retag"),
        (6, "host-a", "retag"),
        (4, "host-b", "host-a"),
        (5, "host-a", "cannot be wrapped up"),
        (99, "host-a", "99"),
    ] {
        let refused = request_wrap_up(&instance, task, host, "rebase");
        assert!(
            !refused.status.success(),
            "task {task}: {}",
            describe(&refused)
        );
        assert!(
            describe(&refused).contains(why),
            "task {task} must say {why:?}: {}",
            describe(&refused)
        );
    }
    let unknown = request_wrap_up(&instance, 4, "host-a", "merge");
    assert!(!unknown.status.success(), "{}", describe(&unknown));
    assert_eq!(session_count(&instance), "0");
    assert!(no_rows(
        &instance,
        "SELECT request_id FROM session_request_timeouts"
    ));
}

/// `OneOpenSessionRequestPerTask`: a second wrap_up while the rebase row is
/// open is refused; once it finishes a new one is accepted.
#[test]
fn a_second_wrap_up_while_one_is_open_is_refused() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    assert!(request_wrap_up(&instance, 1, "host-a", "rebase")
        .status
        .success());

    let second = request_wrap_up(&instance, 1, "host-a", "done");
    assert!(!second.status.success(), "{}", describe(&second));
    assert!(
        describe(&second).contains("open session request"),
        "{}",
        describe(&second)
    );
    assert_eq!(session_count(&instance), "1");

    let id = session_column(&instance, "id", 1, "wrap_up");
    instance.call("claim_session_request", &[&id, "host-a"]);
    instance.call("report_session_request_done", &[&id, "host-a"]);
    assert!(request_wrap_up(&instance, 1, "host-a", "done")
        .status
        .success());
}

/// `HostClaimsSessionRequest`: only the row's host, only once, and the claim
/// clears a Conflict sub_status in the same transaction.
#[test]
fn the_claim_is_the_rows_hosts_alone_once_and_clears_a_conflict() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    move_task(&instance, 1, "running", "conflict");
    assert!(request_wrap_up(&instance, 1, "host-a", "rebase")
        .status
        .success());
    let id = session_column(&instance, "id", 1, "wrap_up");
    assert_eq!(
        task_column(&instance, "sub_status", 1),
        "conflict",
        "the request alone does not clear it"
    );

    let foreign = instance.call("claim_session_request", &[&id, "host-b"]);
    assert!(!foreign.status.success(), "{}", describe(&foreign));
    assert_eq!(
        session_column(&instance, "state", 1, "wrap_up"),
        "requested"
    );
    assert_eq!(task_column(&instance, "sub_status", 1), "conflict");

    let claimed = instance.call("claim_session_request", &[&id, "host-a"]);
    assert!(claimed.status.success(), "{}", describe(&claimed));
    assert_eq!(session_column(&instance, "state", 1, "wrap_up"), "running");
    assert_eq!(
        session_column(&instance, "claimed_at", 1, "wrap_up").len(),
        23
    );
    assert_eq!(task_column(&instance, "sub_status", 1), "active");

    let again = instance.call("claim_session_request", &[&id, "host-a"]);
    assert!(!again.status.success(), "{}", describe(&again));
}

/// `HostReportsSessionRequestDone`: the row is done (a valid token from now
/// on), the timer is cancelled; a repeat is accepted and changes nothing; a
/// report from another host, or on an unclaimed row, is refused.
#[test]
fn a_done_report_finishes_the_row_cancels_the_timer_and_may_repeat() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    assert!(request_wrap_up(&instance, 1, "host-a", "rebase")
        .status
        .success());
    let id = session_column(&instance, "id", 1, "wrap_up");

    let unclaimed = instance.call("report_session_request_done", &[&id, "host-a"]);
    assert!(!unclaimed.status.success(), "{}", describe(&unclaimed));
    instance.call("claim_session_request", &[&id, "host-a"]);
    let foreign = instance.call("report_session_request_done", &[&id, "host-b"]);
    assert!(!foreign.status.success(), "{}", describe(&foreign));
    assert_eq!(session_column(&instance, "state", 1, "wrap_up"), "running");

    let done = instance.call("report_session_request_done", &[&id, "host-a"]);
    assert!(done.status.success(), "{}", describe(&done));
    assert_eq!(session_column(&instance, "state", 1, "wrap_up"), "done");
    let finished = session_column(&instance, "finished_at", 1, "wrap_up");
    assert_eq!(finished.len(), 23);
    assert_eq!(session_timer_count(&instance, &id), "0");

    let repeat = instance.call("report_session_request_done", &[&id, "host-a"]);
    assert!(repeat.status.success(), "{}", describe(&repeat));
    assert_eq!(
        session_column(&instance, "finished_at", 1, "wrap_up"),
        finished
    );
}

/// `HostReportsSessionRequestFailed`: a conflict report makes the MODULE set
/// the Conflict sub_status; a plain failure leaves the task alone; both
/// record the reason and cancel the timer, and issue no token.
#[test]
fn a_failed_report_records_the_reason_and_a_conflict_sets_the_sub_status() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", ""), (2, "running", "")]);
    for task in [1, 2] {
        assert!(request_wrap_up(&instance, task, "host-a", "rebase")
            .status
            .success());
        let id = session_column(&instance, "id", task, "wrap_up");
        instance.call("claim_session_request", &[&id, "host-a"]);
    }
    let first = session_column(&instance, "id", 1, "wrap_up");
    let second = session_column(&instance, "id", 2, "wrap_up");

    let conflict = instance.call(
        "report_session_request_failed",
        &[&first, "host-a", "conflict in src/a.rs", "true"],
    );
    assert!(conflict.status.success(), "{}", describe(&conflict));
    assert_eq!(session_column(&instance, "state", 1, "wrap_up"), "failed");
    assert_eq!(
        session_column(&instance, "reason", 1, "wrap_up"),
        "conflict in src/a.rs"
    );
    assert_eq!(task_column(&instance, "sub_status", 1), "conflict");
    assert_eq!(session_timer_count(&instance, &first), "0");

    let plain = instance.call(
        "report_session_request_failed",
        &[&second, "host-a", "dirty primary worktree", "false"],
    );
    assert!(plain.status.success(), "{}", describe(&plain));
    assert_eq!(task_column(&instance, "sub_status", 2), "active");

    // A late report on a finished row is refused.
    let late = instance.call("report_session_request_done", &[&first, "host-a"]);
    assert!(!late.status.success(), "{}", describe(&late));
    assert_eq!(session_column(&instance, "state", 1, "wrap_up"), "failed");
}

/// A conflict reported for a task that moved on leaves it alone.
#[test]
fn a_conflict_report_leaves_a_task_that_moved_on_alone() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    assert!(request_wrap_up(&instance, 1, "host-a", "rebase")
        .status
        .success());
    let id = session_column(&instance, "id", 1, "wrap_up");
    instance.call("claim_session_request", &[&id, "host-a"]);
    move_task(&instance, 1, "done", "none");

    let reported = instance.call(
        "report_session_request_failed",
        &[&id, "host-a", "conflict", "true"],
    );
    assert!(reported.status.success(), "{}", describe(&reported));
    assert_eq!(task_column(&instance, "sub_status", 1), "none");
}

/// `RequestExitSession`, rebase: the close lands whole with the exit_session
/// row, the epic rollup and the token's consumption.
#[test]
fn an_exit_session_applies_the_close_and_writes_the_teardown_row() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    let token = wrap_up_to_token(&instance, 1, "rebase");

    let closed = request_exit_session(&instance, 1, &token, "rebase", "");
    assert!(closed.status.success(), "{}", describe(&closed));

    assert_eq!(task_column(&instance, "status", 1), "done");
    assert_eq!(task_column(&instance, "sub_status", 1), "none");
    assert_eq!(task_column(&instance, "tmux_window", 1), "");
    assert_eq!(task_column(&instance, "worktree", 1), "/wt/1-t");
    assert_eq!(task_column(&instance, "completed_at", 1).len(), 23);
    assert_eq!(
        column(&instance, "SELECT status FROM epics WHERE id = 1"),
        "done",
        "the epic rolls up in the same transaction"
    );

    assert_eq!(
        session_column(&instance, "state", 1, "exit_session"),
        "requested"
    );
    assert_eq!(
        session_column(&instance, "host", 1, "exit_session"),
        "host-a"
    );
    assert_eq!(
        session_column(&instance, "action", 1, "exit_session"),
        "rebase"
    );
    assert_eq!(
        session_column(&instance, "tmux_window", 1, "exit_session"),
        "task-1"
    );
    assert_eq!(
        session_column(&instance, "wrap_up_request", 1, "exit_session"),
        token
    );
    assert_eq!(
        session_column(&instance, "chained_task", 1, "exit_session"),
        "0"
    );
    assert_eq!(
        session_column(&instance, "requested_by", 1, "exit_session"),
        instance.identity()
    );
    assert_eq!(
        session_column(&instance, "deadline", 1, "exit_session").len(),
        23
    );
    let id = session_column(&instance, "id", 1, "exit_session");
    assert_eq!(session_timer_count(&instance, &id), "1");
}

/// pr: review with the pr-typed url, and the url recorded on the row.
#[test]
fn a_pr_exit_session_takes_the_task_to_review_with_the_url() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    let token = wrap_up_to_token(&instance, 1, "pr");
    let url = "https://github.com/acme/repo/pull/7";

    let closed = request_exit_session(&instance, 1, &token, "pr", url);
    assert!(closed.status.success(), "{}", describe(&closed));

    assert_eq!(task_column(&instance, "status", 1), "review");
    assert_eq!(task_column(&instance, "sub_status", 1), "awaiting_review");
    assert_eq!(task_column(&instance, "url", 1), url);
    assert_eq!(task_column(&instance, "url_type", 1), "pr");
    assert_eq!(task_column(&instance, "tmux_window", 1), "");
    assert_eq!(task_column(&instance, "completed_at", 1), "");
    assert_eq!(session_column(&instance, "pr_url", 1, "exit_session"), url);
}

/// The token is consumed once: a second call is told to wrap up again.
#[test]
fn an_exit_token_is_consumed_by_the_close() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    let token = wrap_up_to_token(&instance, 1, "done");
    assert!(request_exit_session(&instance, 1, &token, "done", "")
        .status
        .success());
    // Finish the teardown so the open-request check is not what refuses.
    let exit = session_column(&instance, "id", 1, "exit_session");
    instance.call("claim_session_request", &[&exit, "host-a"]);
    instance.call("report_session_request_done", &[&exit, "host-a"]);

    let again = request_exit_session(&instance, 1, &token, "done", "");
    assert!(!again.status.success(), "{}", describe(&again));
    assert!(
        describe(&again).contains("no exit token — call wrap_up first"),
        "{}",
        describe(&again)
    );
    assert_eq!(session_count(&instance), "2");
}

/// Every refusal writes nothing, leaves the task and the token as they were,
/// and says why; the same token then closes once the cause is fixed.
#[test]
fn a_refused_exit_session_leaves_the_task_and_the_token_untouched() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    let token = wrap_up_to_token(&instance, 1, "pr");
    let rows_before = session_count(&instance);

    for (action, token_arg, url, why) in [
        ("pr", "999", "https://x/pull/1", "invalid exit token"),
        (
            "rebase",
            token.as_str(),
            "",
            "exit token was issued for wrap_up(action=\"pr\"), but exit_session was called with action=\"rebase\"",
        ),
        (
            "pr",
            token.as_str(),
            "",
            "pr_url is required for action 'pr'",
        ),
    ] {
        let refused = request_exit_session(&instance, 1, token_arg, action, url);
        assert!(!refused.status.success(), "{why}: {}", describe(&refused));
        assert!(describe(&refused).contains(why), "{why}: {}", describe(&refused));
    }
    assert_eq!(session_count(&instance), rows_before);
    assert_eq!(task_column(&instance, "status", 1), "running");
    assert_eq!(task_column(&instance, "tmux_window", 1), "task-1");

    let retried = request_exit_session(&instance, 1, &token, "pr", "https://x/pull/1");
    assert!(retried.status.success(), "{}", describe(&retried));
    assert_eq!(task_column(&instance, "status", 1), "review");
}

/// No token at all, and a task with no window.
#[test]
fn an_exit_session_without_a_token_or_a_window_is_refused() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", ""), (2, "running", "")]);

    let none = request_exit_session(&instance, 1, "5", "done", "");
    assert!(!none.status.success(), "{}", describe(&none));
    assert!(
        describe(&none).contains("no exit token — call wrap_up first"),
        "{}",
        describe(&none)
    );

    let token = wrap_up_to_token(&instance, 2, "done");
    // The window went away by some other path (the TUI, crash detection).
    let patched = instance.call(
        "patch_task",
        &["2", &patch_setting("tmux_window", "").to_string()],
    );
    assert!(patched.status.success(), "{}", describe(&patched));
    let closed = request_exit_session(&instance, 2, &token, "done", "");
    assert!(!closed.status.success(), "{}", describe(&closed));
    assert!(
        describe(&closed).contains("task #2 has no active session"),
        "{}",
        describe(&closed)
    );
    assert_eq!(task_column(&instance, "status", 2), "running");
}

/// A newer wrap_up supersedes the older token.
#[test]
fn a_newer_wrap_up_supersedes_the_older_token() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    let old = wrap_up_to_token(&instance, 1, "done");
    let newer = request_wrap_up(&instance, 1, "host-a", "pr");
    assert!(newer.status.success(), "{}", describe(&newer));
    let new = column(
        &instance,
        "SELECT id FROM session_requests WHERE task_id = 1 AND action = 'pr'",
    );

    let refused = request_exit_session(&instance, 1, &old, "done", "");
    assert!(!refused.status.success(), "{}", describe(&refused));
    assert!(
        describe(&refused).contains("invalid exit token"),
        "{}",
        describe(&refused)
    );
    let closed = request_exit_session(&instance, 1, &new, "pr", "https://x/pull/1");
    assert!(closed.status.success(), "{}", describe(&closed));
}

/// A LATER DISPATCH VOIDS THE TOKEN: the task went back to backlog and was
/// dispatched again after the wrap-up, so the old token cannot close the new
/// session.
#[test]
fn a_later_dispatch_voids_the_exit_token() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    let token = wrap_up_to_token(&instance, 1, "done");
    move_task(&instance, 1, "backlog", "none");
    assert!(request_dispatch(&instance, 1, "host-a", "user", "")
        .status
        .success());
    let dispatch = request_column(&instance, "id", 1);
    instance.call("claim_dispatch_request", &[&dispatch, "host-a"]);
    let reported = instance.call(
        "report_dispatch_provisioned",
        &[&dispatch, "host-a", "/wt/1-t", "task-1", "false"],
    );
    assert!(reported.status.success(), "{}", describe(&reported));

    let refused = request_exit_session(&instance, 1, &token, "done", "");
    assert!(!refused.status.success(), "{}", describe(&refused));
    assert!(
        describe(&refused).contains("invalid exit token"),
        "{}",
        describe(&refused)
    );
    assert_eq!(task_column(&instance, "status", 1), "running");
}

/// An open session request (a second wrap_up's rebase still running) holds
/// the task: the close is refused and the token stays valid.
#[test]
fn an_open_session_request_refuses_the_close() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    let token = wrap_up_to_token(&instance, 1, "done");
    // A second wrap_up (rebase) that is still open is not a done row, so it
    // does not supersede the token; it holds the task instead.
    assert!(request_wrap_up(&instance, 1, "host-a", "rebase")
        .status
        .success());
    let refused = request_exit_session(&instance, 1, &token, "done", "");
    assert!(!refused.status.success(), "{}", describe(&refused));
    assert_eq!(task_column(&instance, "status", 1), "running");
}

/// D11: the task's host is the row's host, and only its owner may ask.
#[test]
fn a_teammate_may_not_act_for_my_hosts_session_requests() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", ""), (2, "running", "")]);
    register_teammates_host(&instance, "their-host");
    assert!(request_wrap_up(&instance, 1, "host-a", "rebase")
        .status
        .success());
    let id = session_column(&instance, "id", 1, "wrap_up");
    let token = wrap_up_to_token(&instance, 2, "done");

    let teammate = |reducer: &str, args: &[&str]| instance.call_as("teammate", reducer, args);
    for out in [
        teammate("request_wrap_up", &["2", "host-a", "done"]),
        teammate("request_wrap_up", &["2", "their-host", "done"]),
        teammate("claim_session_request", &[&id, "host-a"]),
        teammate("report_session_request_done", &[&id, "host-a"]),
        teammate(
            "report_session_request_failed",
            &[&id, "host-a", "x", "false"],
        ),
        teammate("request_exit_session", &["2", &token, "done", ""]),
        teammate("fail_interrupted_session_requests", &["host-a"]),
    ] {
        assert!(!out.status.success(), "{}", describe(&out));
    }
    assert_eq!(
        session_column(&instance, "state", 1, "wrap_up"),
        "requested"
    );
    assert_eq!(task_column(&instance, "status", 2), "running");

    let ownerless = request_wrap_up(&instance, 2, "nowhere", "done");
    assert!(!ownerless.status.success(), "{}", describe(&ownerless));
    assert!(
        describe(&ownerless).contains("not registered"),
        "{}",
        describe(&ownerless)
    );
}

/// The teardown: the host claims the exit_session row and reports it done, or
/// failed; a failed teardown does NOT revert the close.
#[test]
fn a_failed_teardown_does_not_revert_the_close() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    let token = wrap_up_to_token(&instance, 1, "done");
    assert!(request_exit_session(&instance, 1, &token, "done", "")
        .status
        .success());
    let id = session_column(&instance, "id", 1, "exit_session");

    let claimed = instance.call("claim_session_request", &[&id, "host-a"]);
    assert!(claimed.status.success(), "{}", describe(&claimed));
    assert_eq!(task_column(&instance, "sub_status", 1), "none");
    let failed = instance.call(
        "report_session_request_failed",
        &[&id, "host-a", "kill failed", "false"],
    );
    assert!(failed.status.success(), "{}", describe(&failed));

    assert_eq!(
        session_column(&instance, "state", 1, "exit_session"),
        "failed"
    );
    assert_eq!(
        session_column(&instance, "reason", 1, "exit_session"),
        "kill failed"
    );
    assert_eq!(task_column(&instance, "status", 1), "done");
    assert_eq!(session_timer_count(&instance, &id), "0");
    let conflict = instance.call(
        "report_session_request_failed",
        &[&id, "host-a", "again", "true"],
    );
    assert!(!conflict.status.success(), "{}", describe(&conflict));
}

/// A teardown reported done after its deadline still records what happened;
/// the close stands either way. (The deadline itself is a unit test.)
#[test]
fn a_teardown_reported_done_finishes_the_row_and_cancels_the_timer() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    let token = wrap_up_to_token(&instance, 1, "done");
    request_exit_session(&instance, 1, &token, "done", "");
    let id = session_column(&instance, "id", 1, "exit_session");
    instance.call("claim_session_request", &[&id, "host-a"]);

    let done = instance.call("report_session_request_done", &[&id, "host-a"]);
    assert!(done.status.success(), "{}", describe(&done));
    assert_eq!(
        session_column(&instance, "state", 1, "exit_session"),
        "done"
    );
    assert_eq!(session_timer_count(&instance, &id), "0");
}

/// W6, the epic chain: a close in an auto-dispatch epic requests the first
/// backlog subtask in board order whose epic_chain request is accepted,
/// passing over a phoenix one, on the closed task's host, in the same
/// transaction; the exit_session row records it.
#[test]
fn an_exit_session_chains_the_next_subtask_passing_over_a_phoenix_one() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, true, &[(1, "running", "")]);
    seed_backlog_subtask(&instance, 10, 1, true);
    seed_backlog_subtask(&instance, 11, 2, false);
    seed_backlog_subtask(&instance, 12, 3, false);
    let token = wrap_up_to_token(&instance, 1, "rebase");

    let closed = request_exit_session(&instance, 1, &token, "rebase", "");
    assert!(closed.status.success(), "{}", describe(&closed));

    assert_eq!(
        session_column(&instance, "chained_task", 1, "exit_session"),
        "11"
    );
    assert_eq!(request_column(&instance, "origin", 11), "epic_chain");
    assert_eq!(request_column(&instance, "host", 11), "host-a");
    assert_eq!(request_column(&instance, "state", 11), "requested");
    assert_eq!(
        request_column(&instance, "requested_by", 11),
        instance.identity()
    );
    assert_eq!(task_column(&instance, "status", 11), "running");
    assert_eq!(task_column(&instance, "status", 10), "backlog");
    assert_eq!(task_column(&instance, "status", 12), "backlog");
    assert_eq!(request_count(&instance), "1");
    let dispatch = request_column(&instance, "id", 11);
    assert_eq!(timer_count(&instance, dispatch.parse().unwrap()), "1");
}

/// The chain stops, normally and without failing the close: no auto_dispatch,
/// or nothing but phoenix subtasks left.
#[test]
fn the_chain_stops_quietly_without_auto_dispatch_or_candidates() {
    if !spacetime_available_or_skip() {
        return;
    }
    let off = published_instance();
    seed_wrappable(&off, false, &[(1, "running", "")]);
    seed_backlog_subtask(&off, 11, 2, false);
    let token = wrap_up_to_token(&off, 1, "done");
    assert!(request_exit_session(&off, 1, &token, "done", "")
        .status
        .success());
    assert_eq!(task_column(&off, "status", 11), "backlog");
    assert_eq!(request_count(&off), "0");
    assert_eq!(session_column(&off, "chained_task", 1, "exit_session"), "0");

    let only_phoenix = published_instance();
    seed_wrappable(&only_phoenix, true, &[(1, "running", "")]);
    seed_backlog_subtask(&only_phoenix, 10, 1, true);
    let token = wrap_up_to_token(&only_phoenix, 1, "done");
    let closed = request_exit_session(&only_phoenix, 1, &token, "done", "");
    assert!(closed.status.success(), "{}", describe(&closed));
    assert_eq!(task_column(&only_phoenix, "status", 10), "backlog");
    assert_eq!(request_count(&only_phoenix), "0");
    assert_eq!(task_column(&only_phoenix, "status", 1), "done");
}

/// A candidate whose worktree is on another host is refused by the request
/// and the chain moves on to the next.
#[test]
fn the_chain_passes_over_a_subtask_on_another_host() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, true, &[(1, "running", "")]);
    let mut elsewhere = task_json(10, "next", "backlog", 1, "host-b");
    elsewhere["sort_order"] = serde_json::json!({"some": 1});
    let seeded = instance.call("seed_tasks", &[&serde_json::json!([elsewhere]).to_string()]);
    assert!(seeded.status.success(), "{}", describe(&seeded));
    seed_backlog_subtask(&instance, 11, 2, false);
    let token = wrap_up_to_token(&instance, 1, "done");

    assert!(request_exit_session(&instance, 1, &token, "done", "")
        .status
        .success());
    assert_eq!(
        session_column(&instance, "chained_task", 1, "exit_session"),
        "11"
    );
    assert_eq!(task_column(&instance, "status", 10), "backlog");
}

/// A phoenix task closing into Done respawns its successor in the same
/// transaction, and clears its own flag (`TheFlagIsTheReceipt`); a pr close
/// does not respawn.
#[test]
fn a_phoenix_task_respawns_with_a_done_close_but_not_a_pr_close() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    register_own_hosts(&instance);
    // The successor's id is generated, so the sequence is advanced past the
    // seeded ids first, before any row is loaded (`burn_id_sequence`).
    let burned = instance.call("burn_id_sequence", &["tasks", "10"]);
    assert!(burned.status.success(), "{}", describe(&burned));
    let seeded = instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "running", 0)]).to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));
    let rows: Vec<serde_json::Value> = [1, 2]
        .iter()
        .map(|id| {
            let mut row = task_json(*id, "weekly", "running", 1, "host-a");
            row["worktree"] = serde_json::json!(format!("/wt/{id}-t"));
            row["tmux_window"] = serde_json::json!(format!("task-{id}"));
            row["sub_status"] = serde_json::json!("active");
            row["phoenix"] = serde_json::json!(true);
            row["tag"] = serde_json::json!("chore");
            row
        })
        .collect();
    let seeded = instance.call("seed_tasks", &[&serde_json::json!(rows).to_string()]);
    assert!(seeded.status.success(), "{}", describe(&seeded));

    let token = wrap_up_to_token(&instance, 2, "pr");
    assert!(
        request_exit_session(&instance, 2, &token, "pr", "https://x/pull/1")
            .status
            .success()
    );
    assert_eq!(
        column(&instance, "SELECT COUNT(*) AS n FROM tasks"),
        "2",
        "a pr close lands in review and does not respawn"
    );
    assert_eq!(task_column(&instance, "phoenix", 2), "true");

    let token = wrap_up_to_token(&instance, 1, "done");
    let closed = request_exit_session(&instance, 1, &token, "done", "");
    assert!(closed.status.success(), "{}", describe(&closed));
    assert_eq!(column(&instance, "SELECT COUNT(*) AS n FROM tasks"), "3");
    assert_eq!(task_column(&instance, "phoenix", 1), "false");
    assert_eq!(
        column(
            &instance,
            "SELECT status FROM tasks WHERE id <> 1 AND id <> 2"
        ),
        "backlog"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT phoenix FROM tasks WHERE id <> 1 AND id <> 2"
        ),
        "true"
    );
}

/// `HostRestartFailsInterruptedSessionRequests`: this host's RUNNING rows
/// fail with "host restarted" and cancel their timers; its requested rows and
/// every other host's rows are untouched.
#[test]
fn a_restart_fails_only_this_hosts_running_session_requests() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(
        &instance,
        false,
        &[(1, "running", ""), (2, "running", ""), (3, "running", "")],
    );
    for task in [1, 2] {
        assert!(request_wrap_up(&instance, task, "host-a", "rebase")
            .status
            .success());
    }
    let first = session_column(&instance, "id", 1, "wrap_up");
    instance.call("claim_session_request", &[&first, "host-a"]);

    let swept = instance.call("fail_interrupted_session_requests", &["host-a"]);
    assert!(swept.status.success(), "{}", describe(&swept));

    assert_eq!(session_column(&instance, "state", 1, "wrap_up"), "failed");
    assert_eq!(
        session_column(&instance, "reason", 1, "wrap_up"),
        "host restarted"
    );
    assert_eq!(session_timer_count(&instance, &first), "0");
    assert_eq!(
        session_column(&instance, "state", 2, "wrap_up"),
        "requested"
    );
    let swept_other = instance.call("fail_interrupted_session_requests", &["host-b"]);
    assert!(swept_other.status.success(), "{}", describe(&swept_other));
    assert_eq!(
        session_column(&instance, "state", 2, "wrap_up"),
        "requested"
    );
}

/// The scheduled reducer refuses every client: nobody may time a request out
/// early.
#[test]
fn a_client_may_not_run_the_session_backstop() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    assert!(request_wrap_up(&instance, 1, "host-a", "rebase")
        .status
        .success());
    let id = session_column(&instance, "id", 1, "wrap_up");

    let forged = instance.call(
        "expire_session_request",
        &[&format!(
            "{{\"scheduled_id\": 1, \"scheduled_at\": {{\"Interval\": 0}}, \"request_id\": {id}}}"
        )],
    );
    assert!(!forged.status.success(), "{}", describe(&forged));
    assert_eq!(
        session_column(&instance, "state", 1, "wrap_up"),
        "requested"
    );
}

/// W9, `DeletingATaskFailsItsOpenSessionRequest`, through `delete_task`: the
/// open row fails "task deleted" and its timer is cancelled.
#[test]
fn deleting_a_task_fails_its_open_session_request_and_cancels_its_timer() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", "")]);
    assert!(request_wrap_up(&instance, 1, "host-a", "rebase")
        .status
        .success());
    let id = session_column(&instance, "id", 1, "wrap_up");
    move_task(&instance, 1, "done", "none");

    let deleted = instance.call("delete_task", &["1"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));

    assert!(no_rows(&instance, "SELECT id FROM tasks WHERE id = 1"));
    assert_eq!(session_column(&instance, "state", 1, "wrap_up"), "failed");
    assert_eq!(
        session_column(&instance, "reason", 1, "wrap_up"),
        "task deleted"
    );
    assert_eq!(session_timer_count(&instance, &id), "0");
}

/// The same, through `batch_delete`'s task arm and its epic arm.
#[test]
fn batch_delete_fails_the_open_session_requests_of_the_tasks_it_deletes() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_wrappable(&instance, false, &[(1, "running", ""), (2, "running", "")]);
    for task in [1, 2] {
        assert!(request_wrap_up(&instance, task, "host-a", "rebase")
            .status
            .success());
        move_task(&instance, task, "done", "none");
    }

    let deleted = instance.call("batch_delete", &["[1]", "[]"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));
    assert_eq!(session_column(&instance, "state", 1, "wrap_up"), "failed");
    assert_eq!(
        session_column(&instance, "reason", 1, "wrap_up"),
        "task deleted"
    );
    assert_eq!(
        session_column(&instance, "state", 2, "wrap_up"),
        "requested"
    );

    let deleted = instance.call("batch_delete", &["[]", "[1]"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));
    assert_eq!(session_column(&instance, "state", 2, "wrap_up"), "failed");
    assert_eq!(
        session_column(&instance, "reason", 2, "wrap_up"),
        "task deleted"
    );
    assert!(no_rows(
        &instance,
        "SELECT request_id FROM session_request_timeouts"
    ));
}

/// The same, through `ingest_feed`'s stale delete, which removes a feed task
/// whatever its status.
#[test]
fn a_feed_ingest_that_deletes_a_stale_task_fails_its_open_session_request() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    register_own_hosts(&instance);
    let mut epic = epic_json(1, "Feed", "backlog", 0);
    epic["feed_command"] = serde_json::json!("echo []");
    let seeded = instance.call("seed_epics", &[&serde_json::json!([epic]).to_string()]);
    assert!(seeded.status.success(), "{}", describe(&seeded));
    let upserted = instance.call(
        "ingest_feed",
        &[
            "1",
            &ingest_items(&serde_json::json!([feed_item_json(
                "ext-1", "one", "running"
            )]))
            .to_string(),
            "false",
            "test-creator",
        ],
    );
    assert!(upserted.status.success(), "{}", describe(&upserted));
    let task: i64 = column(
        &instance,
        "SELECT id FROM tasks WHERE external_id = 'ext-1'",
    )
    .parse()
    .expect("task id");
    let patched = instance.call(
        "patch_task",
        &[
            &task.to_string(),
            &{
                let mut patch = patch_setting("worktree", "/wt/feed");
                patch["host"] = serde_json::json!({"some": "host-a"});
                patch
            }
            .to_string(),
        ],
    );
    assert!(patched.status.success(), "{}", describe(&patched));
    let requested = request_wrap_up(&instance, task, "host-a", "rebase");
    assert!(requested.status.success(), "{}", describe(&requested));

    let ingested = instance.call("ingest_feed", &["1", "[]", "false", "test-creator"]);
    assert!(ingested.status.success(), "{}", describe(&ingested));

    assert!(no_rows(
        &instance,
        &format!("SELECT id FROM tasks WHERE id = {task}")
    ));
    assert_eq!(
        session_column(&instance, "state", task, "wrap_up"),
        "failed"
    );
    assert_eq!(
        session_column(&instance, "reason", task, "wrap_up"),
        "task deleted"
    );
    assert!(no_rows(
        &instance,
        "SELECT request_id FROM session_request_timeouts"
    ));
}
