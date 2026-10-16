
use super::*;

const HOST_A: &str = "host-a";
const HOST_B: &str = "host-b";
const NOW: &str = "2026-10-10 12:00:00.000";
const LATER: &str = "2026-10-10 12:00:20.000";
const PAST_TEARDOWN_DEADLINE: &str = "2026-10-10 12:00:30.000";
const PAST_REBASE_DEADLINE: &str = "2026-10-10 12:02:00.000";

/// A running task on `HOST_A` with a worktree and a live window.
fn running_task() -> Task {
    Task {
        id: 1,
        status: "running".into(),
        sub_status: "active".into(),
        worktree: "/wt/1-t".into(),
        tmux_window: "task-1".into(),
        host: HOST_A.into(),
        epic_id: 5,
        ..blank_task()
    }
}

/// A session row for task 1 on `HOST_A`.
fn session(id: i64, kind: &str, action: &str, state: &str) -> SessionRequest {
    SessionRequest {
        id,
        task_id: 1,
        kind: kind.into(),
        action: action.into(),
        host: HOST_A.into(),
        state: state.into(),
        requested_at: NOW.into(),
        ..blank_session_request()
    }
}

/// A done wrap_up row, finished at `finished_at`: a candidate exit token.
fn done_wrap_up(id: i64, action: &str, finished_at: &str) -> SessionRequest {
    SessionRequest {
        finished_at: finished_at.into(),
        ..session(id, "wrap_up", action, "done")
    }
}

/// The exit_session row that consumed `token`.
fn consumed(id: i64, token: i64) -> SessionRequest {
    SessionRequest {
        wrap_up_request: token,
        ..session(id, "exit_session", "rebase", "requested")
    }
}

fn dispatched(finished_at: &str) -> DispatchRequest {
    DispatchRequest {
        id: 1,
        task_id: 1,
        state: "dispatched".into(),
        finished_at: finished_at.into(),
        ..blank_dispatch_request()
    }
}

// -- config (store-split.allium: config) ------------------------------

#[test]
fn the_session_timeouts_are_120_and_30_seconds_with_a_ten_second_grace() {
    assert_eq!(WRAP_UP_REBASE_TIMEOUT_SECS, 120);
    assert_eq!(EXIT_SESSION_TEARDOWN_TIMEOUT_SECS, 30);
    assert_eq!(SESSION_REQUEST_BACKSTOP_GRACE_SECS, 10);
}

/// `SessionRequest.deadline`: ONE deadline from `requested_at`, which
/// depends on the kind.
#[test]
fn the_deadline_is_requested_at_plus_the_timeout_of_the_kind() {
    assert_eq!(
        session_request_deadline("wrap_up", "2026-09-19 12:34:56.789"),
        "2026-09-19 12:36:56.789"
    );
    assert_eq!(
        session_request_deadline("exit_session", "2026-09-19 12:34:56.789"),
        "2026-09-19 12:35:26.789"
    );
    assert_eq!(session_request_deadline("wrap_up", "yesterday"), "");
}

/// `SessionRequestTimesOut` fires at deadline + grace, not at the
/// deadline: a live host's own stop must land first.
#[test]
fn the_backstop_timer_is_due_after_the_deadline_plus_the_grace() {
    let requested = 1_789_821_296_789_000_i64;
    assert_eq!(
        session_request_backstop_due_micros("wrap_up", requested),
        requested + 130 * 1_000_000
    );
    assert_eq!(
        session_request_backstop_due_micros("exit_session", requested),
        requested + 40 * 1_000_000
    );
}

#[test]
fn requested_and_running_are_open_and_the_finished_states_are_not() {
    assert!(session_request_is_open("requested"));
    assert!(session_request_is_open("running"));
    assert!(!session_request_is_open("done"));
    assert!(!session_request_is_open("failed"));
}

#[test]
fn the_failure_reasons_and_refusals_are_the_specs_wording() {
    assert_eq!(SESSION_REASON_TIMED_OUT, "timed out");
    assert_eq!(SESSION_REASON_HOST_RESTARTED, "host restarted");
    assert_eq!(SESSION_REASON_TASK_DELETED, "task deleted");
    assert_eq!(EXIT_TOKEN_MISSING, "no exit token — call wrap_up first");
    assert_eq!(EXIT_TOKEN_INVALID, "invalid exit token");
}

// -- RequestWrapUp ------------------------------------------------------

#[test]
fn only_the_three_wrap_up_actions_are_actions() {
    for action in ["rebase", "done", "pr"] {
        assert!(validate_wrap_up_action(action).is_ok(), "{action}");
    }
    for action in ["", "merge", "Rebase"] {
        assert!(validate_wrap_up_action(action).is_err(), "{action:?}");
    }
}

#[test]
fn a_running_or_review_task_with_a_worktree_on_the_host_may_be_wrapped_up() {
    for status in ["running", "review"] {
        let task = Task {
            status: status.into(),
            ..running_task()
        };
        assert!(
            validate_wrap_up_request(&task, HOST_A, false).is_ok(),
            "{status}"
        );
    }
}

#[test]
fn a_task_that_is_not_running_or_review_or_has_no_worktree_is_refused() {
    for status in ["backlog", "done"] {
        let task = Task {
            status: status.into(),
            ..running_task()
        };
        let why = validate_wrap_up_request(&task, HOST_A, false).unwrap_err();
        assert!(
            why.contains("cannot be wrapped up") && why.contains("Requires Running or Review"),
            "{status}: {why}"
        );
    }
    let bare = Task {
        worktree: String::new(),
        ..running_task()
    };
    let why = validate_wrap_up_request(&bare, HOST_A, false).unwrap_err();
    assert!(why.contains("cannot be wrapped up"), "{why}");
}

/// `ReviewTasksAreNotWrappedUp`, the module's by decision: the refusal
/// carries the tag, that a review task ends at its PR, and the retag way
/// out.
#[test]
fn a_review_tagged_task_is_refused_with_the_retag_way_out() {
    for tag in ["pr-review", "dependabot", "release-please"] {
        let task = Task {
            tag: tag.into(),
            ..running_task()
        };
        let why = validate_wrap_up_request(&task, HOST_A, false).unwrap_err();
        assert!(
            why.contains(&format!("`{tag}`")) && why.contains("retag"),
            "{tag}: {why}"
        );
    }
    let chore = Task {
        tag: "chore".into(),
        ..running_task()
    };
    assert!(validate_wrap_up_request(&chore, HOST_A, false).is_ok());
}

/// The rebase runs where the worktree is, so only the task's own host may
/// ask.
#[test]
fn a_task_whose_worktree_is_on_another_host_is_refused() {
    let why = validate_wrap_up_request(&running_task(), HOST_B, false).unwrap_err();
    assert!(why.contains(HOST_A) && why.contains(HOST_B), "{why}");
    let unhosted = Task {
        host: String::new(),
        ..running_task()
    };
    assert!(validate_wrap_up_request(&unhosted, HOST_A, false).is_err());
}

/// `OneOpenSessionRequestPerTask`.
#[test]
fn a_task_with_an_open_session_request_is_refused() {
    let why = validate_wrap_up_request(&running_task(), HOST_A, true).unwrap_err();
    assert!(why.contains("open session request"), "{why}");
}

/// A rebase row is written requested, for its host to claim, with the
/// wrap-up deadline.
#[test]
fn a_rebase_wrap_up_row_is_requested_with_a_deadline() {
    let row = new_wrap_up_request(1, HOST_A, "rebase", NOW);
    assert_eq!(row.task_id, 1);
    assert_eq!(row.kind, "wrap_up");
    assert_eq!(row.action, "rebase");
    assert_eq!(row.host, HOST_A);
    assert_eq!(row.state, "requested");
    assert_eq!(row.requested_at, NOW);
    assert_eq!(row.claimed_at, "");
    assert_eq!(row.finished_at, "");
    assert_eq!(row.deadline, "2026-10-10 12:02:00.000");
    assert_eq!(row.wrap_up_request, 0);
    assert_eq!(row.tmux_window, "");
    assert_eq!(row.chained_task, 0);
    assert!(session_request_has_host_work(&row));
}

/// done and pr need no host work: written already done in the same call,
/// with no claim, no deadline and so no timer.
#[test]
fn a_done_or_pr_wrap_up_row_is_written_already_done() {
    for action in ["done", "pr"] {
        let row = new_wrap_up_request(1, HOST_A, action, NOW);
        assert_eq!(row.state, "done", "{action}");
        assert_eq!(row.finished_at, NOW, "{action}");
        assert_eq!(row.claimed_at, "", "{action}");
        assert_eq!(row.deadline, "", "{action}");
        assert!(!session_request_has_host_work(&row), "{action}");
    }
}

// -- HostClaimsSessionRequest -----------------------------------------

#[test]
fn the_rows_own_host_claims_a_requested_row() {
    let row = SessionRequest {
        deadline: session_request_deadline("wrap_up", NOW),
        ..session(7, "wrap_up", "rebase", "requested")
    };
    assert!(validate_session_request_claim(&row, HOST_A, LATER).is_ok());
    let claimed = claim_session_request_row(row, LATER);
    assert_eq!(claimed.state, "running");
    assert_eq!(claimed.claimed_at, LATER);
    assert_eq!(claimed.requested_at, NOW);
}

#[test]
fn another_host_cannot_claim_the_row() {
    let row = session(7, "wrap_up", "rebase", "requested");
    assert!(validate_session_request_claim(&row, HOST_B, LATER).is_err());
}

/// Of two boards, or a row delivered twice, exactly one claim is accepted.
#[test]
fn a_row_that_is_not_requested_cannot_be_claimed() {
    for state in ["running", "done", "failed"] {
        let row = session(7, "wrap_up", "rebase", state);
        assert!(
            validate_session_request_claim(&row, HOST_A, LATER).is_err(),
            "{state}"
        );
    }
}

/// The host enforces the deadline, so a row past it is not started; the
/// backstop fails it.
#[test]
fn a_row_at_or_past_its_deadline_cannot_be_claimed() {
    let rebase = SessionRequest {
        deadline: session_request_deadline("wrap_up", NOW),
        ..session(7, "wrap_up", "rebase", "requested")
    };
    assert!(validate_session_request_claim(&rebase, HOST_A, PAST_REBASE_DEADLINE).is_err());
    let teardown = SessionRequest {
        deadline: session_request_deadline("exit_session", NOW),
        ..session(8, "exit_session", "rebase", "requested")
    };
    assert!(validate_session_request_claim(&teardown, HOST_A, "2026-10-10 12:00:29.999").is_ok());
    assert!(validate_session_request_claim(&teardown, HOST_A, PAST_TEARDOWN_DEADLINE).is_err());
}

/// Today's optimistic clear, moved into the claim: a Conflict sub_status
/// is cleared to the status's default when a wrap_up row is claimed.
#[test]
fn claiming_a_wrap_up_clears_a_conflict() {
    let conflicted = Task {
        sub_status: "conflict".into(),
        ..running_task()
    };
    let cleared = clear_conflict_on_claim("wrap_up", conflicted, LATER).expect("was conflict");
    assert_eq!(cleared.sub_status, "active");
    assert_eq!(cleared.updated_at, LATER);

    let in_review = Task {
        status: "review".into(),
        sub_status: "conflict".into(),
        ..running_task()
    };
    let cleared = clear_conflict_on_claim("wrap_up", in_review, LATER).expect("was conflict");
    assert_eq!(cleared.sub_status, "awaiting_review");
}

#[test]
fn claiming_leaves_a_task_without_a_conflict_alone() {
    assert!(clear_conflict_on_claim("wrap_up", running_task(), LATER).is_none());
}

/// Only a wrap_up claim clears it: a teardown claim never touches the
/// task.
#[test]
fn claiming_a_teardown_never_clears_a_conflict() {
    let conflicted = Task {
        sub_status: "conflict".into(),
        ..running_task()
    };
    assert!(clear_conflict_on_claim("exit_session", conflicted, LATER).is_none());
}

#[test]
fn the_default_sub_status_follows_the_status() {
    assert_eq!(default_sub_status("backlog"), "none");
    assert_eq!(default_sub_status("running"), "active");
    assert_eq!(default_sub_status("review"), "awaiting_review");
    assert_eq!(default_sub_status("done"), "none");
}

// -- HostReportsSessionRequestDone / Repeats / Failed -----------------

#[test]
fn the_rows_host_reports_on_a_running_row() {
    let row = session(7, "wrap_up", "rebase", "running");
    assert!(validate_session_report(&row, HOST_A).is_ok());
    assert!(validate_session_report(&row, HOST_B).is_err());
}

/// A LATE REPORT IS REFUSED: on a row that is no longer running.
#[test]
fn a_report_on_a_row_that_is_not_running_is_refused() {
    for state in ["requested", "done", "failed"] {
        let row = session(7, "wrap_up", "rebase", state);
        assert!(validate_session_report(&row, HOST_A).is_err(), "{state}");
    }
}

/// A rebase reported done after its deadline, by the module's clock, is
/// too late and issues no token. A teardown is not held to it: the close
/// stands whatever the teardown does.
#[test]
fn a_rebase_done_report_past_the_deadline_is_refused_but_a_teardown_is_not() {
    let rebase = SessionRequest {
        deadline: session_request_deadline("wrap_up", NOW),
        ..session(7, "wrap_up", "rebase", "running")
    };
    assert!(validate_session_done_in_time(&rebase, LATER).is_ok());
    assert!(validate_session_done_in_time(&rebase, PAST_REBASE_DEADLINE).is_err());

    let teardown = SessionRequest {
        deadline: session_request_deadline("exit_session", NOW),
        ..session(8, "exit_session", "rebase", "running")
    };
    assert!(validate_session_done_in_time(&teardown, PAST_REBASE_DEADLINE).is_ok());
}

#[test]
fn a_done_report_records_the_finish_on_the_row() {
    let row = done_session_request_row(session(7, "wrap_up", "rebase", "running"), LATER);
    assert_eq!(row.state, "done");
    assert_eq!(row.finished_at, LATER);
    assert_eq!(row.reason, "");
}

/// `HostRepeatsSessionRequestDone`: only a row this host claimed and
/// reported; a wrap_up row written done had no report to repeat.
#[test]
fn a_repeat_of_the_done_report_is_recognised() {
    let reported = SessionRequest {
        claimed_at: NOW.into(),
        finished_at: LATER.into(),
        ..session(7, "wrap_up", "rebase", "done")
    };
    assert!(session_report_is_repeat(&reported, HOST_A));
    assert!(!session_report_is_repeat(&reported, HOST_B));

    let written_done = done_wrap_up(8, "done", NOW);
    assert!(!session_report_is_repeat(&written_done, HOST_A));

    let failed = session(9, "wrap_up", "rebase", "failed");
    assert!(!session_report_is_repeat(&failed, HOST_A));
}

#[test]
fn a_failed_report_records_the_reason_on_the_row() {
    let row = failed_session_request_row(
        session(7, "wrap_up", "rebase", "running"),
        "conflict in src/a.rs",
        LATER,
    );
    assert_eq!(row.state, "failed");
    assert_eq!(row.finished_at, LATER);
    assert_eq!(row.reason, "conflict in src/a.rs");
}

/// Only a rebase can conflict.
#[test]
fn only_a_wrap_up_can_report_a_conflict() {
    let wrap_up = session(7, "wrap_up", "rebase", "running");
    assert!(validate_session_failure_report(&wrap_up, true).is_ok());
    assert!(validate_session_failure_report(&wrap_up, false).is_ok());
    let teardown = session(8, "exit_session", "rebase", "running");
    assert!(validate_session_failure_report(&teardown, false).is_ok());
    assert!(validate_session_failure_report(&teardown, true).is_err());
}

/// `FinishTaskConflict`: Conflict is legal only under running and review,
/// so a task moved on meanwhile is left alone.
#[test]
fn a_conflict_sets_the_sub_status_only_while_running_or_review() {
    let running = apply_rebase_conflict(running_task(), LATER).expect("running");
    assert_eq!(running.sub_status, "conflict");
    assert_eq!(running.status, "running");
    assert_eq!(running.updated_at, LATER);

    let review = Task {
        status: "review".into(),
        sub_status: "awaiting_review".into(),
        ..running_task()
    };
    assert_eq!(
        apply_rebase_conflict(review, LATER)
            .expect("review")
            .sub_status,
        "conflict"
    );

    for status in ["backlog", "done"] {
        let moved = Task {
            status: status.into(),
            sub_status: "none".into(),
            ..running_task()
        };
        assert!(apply_rebase_conflict(moved, LATER).is_none(), "{status}");
    }
}

// -- SessionRequestTimesOut (the scheduled reducer's body) ------------

#[test]
fn the_backstop_fails_an_open_row_as_timed_out() {
    for state in ["requested", "running"] {
        let failed = expire_session_request_row(
            session(7, "wrap_up", "rebase", state),
            PAST_REBASE_DEADLINE,
        )
        .unwrap_or_else(|| panic!("{state} is open and must be failed"));
        assert_eq!(failed.state, "failed");
        assert_eq!(failed.reason, "timed out");
        assert_eq!(failed.finished_at, PAST_REBASE_DEADLINE);
    }
}

#[test]
fn the_backstop_leaves_a_finished_row_alone() {
    for state in ["done", "failed"] {
        assert!(
            expire_session_request_row(session(7, "wrap_up", "rebase", state), LATER).is_none(),
            "{state}"
        );
    }
}

// -- HostRestartFailsInterruptedSessionRequests -----------------------

/// Only RUNNING rows, over rows already read by the restarting host's
/// index: a requested row is picked up by the starting board.
#[test]
fn a_restart_interrupts_only_running_rows() {
    let rows = vec![
        session(1, "wrap_up", "rebase", "running"),
        session(2, "wrap_up", "rebase", "requested"),
        session(3, "exit_session", "rebase", "running"),
        session(4, "wrap_up", "rebase", "done"),
        session(5, "wrap_up", "rebase", "failed"),
    ];
    let ids: Vec<i64> = interrupted_session_requests(rows)
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(ids, vec![1, 3]);
}

// -- the exit token (W3) ----------------------------------------------

#[test]
fn the_most_recent_done_wrap_up_row_is_a_valid_token() {
    let rows = vec![done_wrap_up(10, "rebase", LATER)];
    let token = check_exit_token(1, 10, &rows, &[]).expect("valid");
    assert_eq!(token.id, 10);
    assert_eq!(token.action, "rebase");
}

/// No done wrap_up row at all: the agent never wrapped up.
#[test]
fn no_done_wrap_up_row_means_no_exit_token() {
    assert_eq!(
        check_exit_token(1, 10, &[], &[]).unwrap_err(),
        EXIT_TOKEN_MISSING
    );
    let not_done = vec![
        session(10, "wrap_up", "rebase", "requested"),
        session(11, "wrap_up", "rebase", "running"),
        session(12, "wrap_up", "rebase", "failed"),
    ];
    for token in [10, 11, 12] {
        assert_eq!(
            check_exit_token(1, token, &not_done, &[]).unwrap_err(),
            EXIT_TOKEN_MISSING,
            "{token}"
        );
    }
}

/// Each token is consumed once: the exit_session row that names it ends
/// its validity, and a second call is told to wrap up again.
#[test]
fn a_consumed_token_means_no_exit_token() {
    let rows = vec![done_wrap_up(10, "rebase", LATER), consumed(11, 10)];
    assert_eq!(
        check_exit_token(1, 10, &rows, &[]).unwrap_err(),
        EXIT_TOKEN_MISSING
    );
}

/// A token that names some other row is "invalid", not "missing".
#[test]
fn a_token_naming_another_row_is_invalid() {
    let rows = vec![
        done_wrap_up(10, "rebase", LATER),
        SessionRequest {
            task_id: 2,
            ..done_wrap_up(20, "rebase", LATER)
        },
        consumed(11, 99),
    ];
    // No such row, another task's row, and an exit_session row.
    for token in [999, 20, 11] {
        assert_eq!(
            check_exit_token(1, token, &rows, &[]).unwrap_err(),
            EXIT_TOKEN_INVALID,
            "{token}"
        );
    }
}

/// Only the most recent done wrap_up row is valid, as a newer token
/// replaces an older one today; ties on requested_at go to the higher id.
#[test]
fn a_newer_wrap_up_row_supersedes_an_older_token() {
    let newer = SessionRequest {
        requested_at: LATER.into(),
        ..done_wrap_up(11, "done", LATER)
    };
    let rows = vec![done_wrap_up(10, "rebase", NOW), newer];
    assert_eq!(
        check_exit_token(1, 10, &rows, &[]).unwrap_err(),
        EXIT_TOKEN_INVALID
    );
    assert!(check_exit_token(1, 11, &rows, &[]).is_ok());

    let tied = vec![
        done_wrap_up(10, "rebase", LATER),
        done_wrap_up(11, "done", LATER),
    ];
    assert_eq!(
        check_exit_token(1, 10, &tied, &[]).unwrap_err(),
        EXIT_TOKEN_INVALID
    );
    assert!(check_exit_token(1, 11, &tied, &[]).is_ok());
}

/// A LATER DISPATCH VOIDS THE TOKEN: a dispatched request of the task that
/// finished at or after the token's row.
#[test]
fn a_later_dispatch_voids_the_token() {
    let rows = vec![done_wrap_up(10, "rebase", LATER)];
    for finished in [LATER, PAST_TEARDOWN_DEADLINE] {
        assert_eq!(
            check_exit_token(1, 10, &rows, &[dispatched(finished)]).unwrap_err(),
            EXIT_TOKEN_INVALID,
            "{finished}"
        );
    }
}

/// An earlier dispatch is the session the token closes, and a dispatch
/// that failed never started a new one.
#[test]
fn an_earlier_or_failed_dispatch_leaves_the_token_valid() {
    let rows = vec![done_wrap_up(10, "rebase", LATER)];
    assert!(check_exit_token(1, 10, &rows, &[dispatched(NOW)]).is_ok());
    let failed = DispatchRequest {
        state: "failed".into(),
        ..dispatched(PAST_TEARDOWN_DEADLINE)
    };
    assert!(check_exit_token(1, 10, &rows, &[failed]).is_ok());
}

// -- RequestExitSession: the refusals ----------------------------------

fn token(action: &str) -> SessionRequest {
    done_wrap_up(10, action, LATER)
}

#[test]
fn a_matching_action_and_a_live_window_are_accepted() {
    assert!(validate_exit_session(&running_task(), &token("rebase"), "rebase", "").is_ok());
    assert!(validate_exit_session(&running_task(), &token("done"), "done", "").is_ok());
    assert!(validate_exit_session(
        &running_task(),
        &token("pr"),
        "pr",
        "https://github.com/acme/repo/pull/7"
    )
    .is_ok());
}

/// The error names both actions: it stops an agent doing one thing and
/// closing as another.
#[test]
fn an_action_that_differs_from_the_tokens_is_refused_naming_both() {
    let why = validate_exit_session(&running_task(), &token("rebase"), "pr", "u").unwrap_err();
    assert_eq!(
            why,
            "exit token was issued for wrap_up(action=\"rebase\"), but exit_session was called with action=\"pr\""
        );
}

#[test]
fn a_task_with_no_window_has_no_active_session() {
    let closed = Task {
        tmux_window: String::new(),
        ..running_task()
    };
    let why = validate_exit_session(&closed, &token("done"), "done", "").unwrap_err();
    assert_eq!(why, "task #1 has no active session");
}

#[test]
fn a_pr_close_requires_a_pr_url() {
    for url in ["", "  "] {
        let why = validate_exit_session(&running_task(), &token("pr"), "pr", url).unwrap_err();
        assert_eq!(
            why, "pr_url is required for action 'pr' — pass the URL returned by `gh pr create`",
            "{url:?}"
        );
    }
}

/// The checks run in the order the spec lists them: action, window, url.
#[test]
fn the_refusals_come_in_the_specs_order() {
    let closed = Task {
        tmux_window: String::new(),
        ..running_task()
    };
    let why = validate_exit_session(&closed, &token("rebase"), "pr", "").unwrap_err();
    assert!(why.contains("exit token was issued"), "{why}");
    let why = validate_exit_session(&closed, &token("pr"), "pr", "").unwrap_err();
    assert!(why.contains("no active session"), "{why}");
}

#[test]
fn an_open_session_request_refuses_a_close() {
    let why = validate_no_open_session_request(1, true).unwrap_err();
    assert!(why.contains("open session request"), "{why}");
    assert!(validate_no_open_session_request(1, false).is_ok());
}

// -- ExitSession: the close's writes -------------------------------------

#[test]
fn a_rebase_or_done_close_takes_the_task_to_done() {
    for action in ["rebase", "done"] {
        let closed = apply_session_close(
            Task {
                stop_pending: true,
                ..running_task()
            },
            action,
            "",
            LATER,
        );
        assert_eq!(closed.status, "done", "{action}");
        assert_eq!(closed.sub_status, "none", "{action}");
        assert_eq!(closed.tmux_window, "", "{action}");
        assert_eq!(closed.completed_at, LATER, "{action}");
        assert_eq!(closed.updated_at, LATER, "{action}");
        assert!(!closed.stop_pending, "{action}");
        assert_eq!(
            closed.worktree, "/wt/1-t",
            "the worktree survives the close"
        );
        assert_eq!(closed.url, "");
    }
}

/// pr lands in review with a pr-typed url, and does not complete the task.
#[test]
fn a_pr_close_takes_the_task_to_review_with_the_url() {
    let closed = apply_session_close(
        running_task(),
        "pr",
        "https://github.com/acme/repo/pull/7",
        LATER,
    );
    assert_eq!(closed.status, "review");
    assert_eq!(closed.sub_status, "awaiting_review");
    assert_eq!(closed.tmux_window, "");
    assert_eq!(closed.url, "https://github.com/acme/repo/pull/7");
    assert_eq!(closed.url_type, "pr");
    assert_eq!(closed.completed_at, "");
    assert_eq!(closed.updated_at, LATER);
}

/// Both branches leave running, so both clear a deferred Stop; a close
/// from review has none to clear and does not touch the flag.
#[test]
fn the_deferred_stop_is_cleared_only_when_the_close_leaves_running() {
    let reviewing = Task {
        status: "review".into(),
        sub_status: "awaiting_review".into(),
        stop_pending: true,
        ..running_task()
    };
    let closed = apply_session_close(reviewing, "done", "", LATER);
    assert!(closed.stop_pending);
}

/// pr_url is ignored for rebase and done.
#[test]
fn a_done_close_ignores_a_pr_url() {
    let closed = apply_session_close(running_task(), "done", "https://x/pull/1", LATER);
    assert_eq!(closed.url, "");
    assert_eq!(closed.url_type, "");
}

// -- ExitSession: the exit_session row -----------------------------------

#[test]
fn an_exit_session_row_names_the_token_window_host_and_chained_subtask() {
    let row = new_exit_session_request(&running_task(), 10, "pr", "https://x/pull/1", 42, NOW);
    assert_eq!(row.task_id, 1);
    assert_eq!(row.kind, "exit_session");
    assert_eq!(row.action, "pr");
    assert_eq!(row.host, HOST_A);
    assert_eq!(row.state, "requested");
    assert_eq!(row.requested_at, NOW);
    assert_eq!(row.deadline, "2026-10-10 12:00:30.000");
    assert_eq!(row.pr_url, "https://x/pull/1");
    assert_eq!(row.wrap_up_request, 10);
    assert_eq!(row.tmux_window, "task-1");
    assert_eq!(row.chained_task, 42);
    assert_eq!(row.claimed_at, "");
    assert_eq!(row.finished_at, "");
    assert!(session_request_has_host_work(&row));
}

/// pr_url is recorded only for action pr; no chained subtask is 0.
#[test]
fn an_exit_session_row_records_the_pr_url_only_for_pr() {
    let row = new_exit_session_request(&running_task(), 10, "done", "https://x/pull/1", 0, NOW);
    assert_eq!(row.pr_url, "");
    assert_eq!(row.chained_task, 0);
}

// -- PhoenixRespawn through the close -------------------------------------

fn phoenix_task() -> Task {
    Task {
        title: "weekly".into(),
        description: "d".into(),
        repo_path: "/repo".into(),
        plan_path: "/plans/p.md".into(),
        tag: "chore".into(),
        labels: "[\"x\"]".into(),
        base_branch: "develop".into(),
        wrap_up_mode: "rebase".into(),
        auto_run_plan: true,
        phoenix: true,
        ..running_task()
    }
}

#[test]
fn a_phoenix_task_respawns_when_a_close_takes_it_into_done() {
    let prior = phoenix_task();
    let closed = apply_session_close(prior.clone(), "done", "", LATER);
    assert!(should_respawn_phoenix(&prior.status, &closed));
}

/// pr lands in review, and the respawn waits for the PR to merge. A feed
/// row is exempt, and so is a task that was already done.
#[test]
fn a_phoenix_task_does_not_respawn_in_the_other_cases() {
    let prior = phoenix_task();
    let to_review = apply_session_close(prior.clone(), "pr", "u", LATER);
    assert!(!should_respawn_phoenix(&prior.status, &to_review));

    let feed = Task {
        external_id: "ext-1".into(),
        ..phoenix_task()
    };
    let closed = apply_session_close(feed.clone(), "done", "", LATER);
    assert!(!should_respawn_phoenix(&feed.status, &closed));

    let ordinary = apply_session_close(running_task(), "done", "", LATER);
    assert!(!should_respawn_phoenix("running", &ordinary));

    let closed = apply_session_close(prior, "done", "", LATER);
    assert!(!should_respawn_phoenix("done", &closed));
}

/// WhatTheSuccessorInherits: everything the operator configured and
/// nothing the previous run produced.
#[test]
fn the_successor_inherits_the_configuration_and_none_of_the_run() {
    let prior = Task {
        url: "https://x/pull/1".into(),
        url_type: "pr".into(),
        completed_at: LATER.into(),
        sort_order: Some(3),
        last_pre_tool_use_at: LATER.into(),
        live_subagents: 2,
        ..phoenix_task()
    };
    let next = phoenix_successor(&prior, 9, NOW, "creator");
    assert_eq!(next.id, 0);
    assert_eq!(next.title, "weekly");
    assert_eq!(next.description, "d");
    assert_eq!(next.repo_path, "/repo");
    assert_eq!(next.plan_path, "/plans/p.md");
    assert_eq!(next.tag, "chore");
    assert_eq!(next.labels, "[\"x\"]");
    assert_eq!(next.base_branch, "develop");
    assert_eq!(next.wrap_up_mode, "rebase");
    assert!(next.auto_run_plan);
    assert!(next.phoenix);
    assert_eq!(next.epic_id, 9);
    assert_eq!(next.status, "backlog");
    assert_eq!(next.sub_status, "none");
    assert_eq!(next.created_at, NOW);
    assert_eq!(next.updated_at, NOW);
    assert_eq!(next.created_by, "creator");
    assert_eq!(next.sort_order, None);
    for empty in [
        &next.worktree,
        &next.tmux_window,
        &next.url,
        &next.url_type,
        &next.completed_at,
        &next.last_pre_tool_use_at,
        &next.external_id,
        &next.host,
    ] {
        assert_eq!(empty, "");
    }
    assert_eq!(next.live_subagents, 0);
}

/// A task on a user board (no epic) keeps its owner; one in an epic has
/// none (`OwnerTracksUserBoardTask`).
#[test]
fn the_successor_owner_follows_the_epic() {
    let owned = Task {
        epic_id: 0,
        owner: "alice".into(),
        ..phoenix_task()
    };
    assert_eq!(phoenix_successor(&owned, 0, NOW, "c").owner, "alice");
    assert_eq!(phoenix_successor(&owned, 9, NOW, "c").owner, "");
}

// -- AutoDispatchNextSubtask: candidate selection --------------------------

fn subtask(id: i64, status: &str, sort_order: Option<i64>) -> Task {
    Task {
        id,
        status: status.into(),
        sort_order,
        ..blank_task()
    }
}

/// First backlog subtask ordered by sort_order (nulls fall back to the
/// task id), then by id.
#[test]
fn candidates_are_the_backlog_subtasks_in_board_order() {
    let tasks = vec![
        subtask(3, "backlog", None),
        subtask(1, "backlog", Some(5)),
        subtask(2, "backlog", Some(1)),
        subtask(4, "running", Some(0)),
        subtask(5, "done", None),
        subtask(6, "backlog", Some(3)),
    ];
    let ids: Vec<i64> = chain_candidates(tasks).into_iter().map(|t| t.id).collect();
    assert_eq!(ids, vec![2, 3, 6, 1]);
}

/// A phoenix subtask is a candidate in the list and refused by the request
/// itself (`PhoenixIsNeverChained`), so the chain passes over it.
#[test]
fn the_chains_request_refuses_a_phoenix_candidate_and_one_on_another_host() {
    let phoenix = Task {
        phoenix: true,
        host: String::new(),
        ..subtask(1, "backlog", None)
    };
    assert!(validate_dispatch_request(&phoenix, HOST_A, "epic_chain", false).is_err());
    let elsewhere = Task {
        host: HOST_B.into(),
        ..subtask(2, "backlog", None)
    };
    assert!(validate_dispatch_request(&elsewhere, HOST_A, "epic_chain", false).is_err());
    assert!(
        validate_dispatch_request(&subtask(3, "backlog", None), HOST_A, "epic_chain", false)
            .is_ok()
    );
}

// -- PR state (store-split.allium: ReportPrStatus, ReportPrUnreadable) ----

fn pr_task(sub_status: &str) -> Task {
    Task {
        id: 1,
        status: "review".into(),
        sub_status: sub_status.into(),
        url: "https://github.com/acme/repo/pull/7".into(),
        url_type: "pr".into(),
        tmux_window: "task-1".into(),
        epic_id: 5,
        ..blank_task()
    }
}

#[test]
fn a_merged_pr_takes_the_task_to_done() {
    let done = apply_pr_status(pr_task("approved"), "MERGED", "APPROVED", LATER).unwrap();
    assert_eq!(done.status, "done");
    assert_eq!(done.sub_status, "none");
    assert_eq!(done.tmux_window, "");
    assert_eq!(done.completed_at, LATER);
    assert_eq!(done.updated_at, LATER);
}

#[test]
fn gh_letter_case_does_not_matter() {
    assert!(validate_pr_status_report("merged", "approved").is_ok());
    assert_eq!(
        apply_pr_status(pr_task("approved"), "merged", "", LATER)
            .unwrap()
            .status,
        "done"
    );
}

#[test]
fn a_closed_pr_marks_pr_closed_and_overrides_conflict() {
    for from in ["awaiting_review", "conflict"] {
        let closed = apply_pr_status(pr_task(from), "CLOSED", "", LATER).unwrap();
        assert_eq!(closed.status, "review", "{from}");
        assert_eq!(closed.sub_status, "pr_closed", "{from}");
        assert_eq!(closed.updated_at, LATER);
    }
    assert!(apply_pr_status(pr_task("pr_closed"), "CLOSED", "", LATER).is_none());
}

#[test]
fn an_open_pr_follows_the_review_decision() {
    for (decision, expected) in [
        ("APPROVED", "approved"),
        ("CHANGES_REQUESTED", "changes_requested"),
        ("REVIEW_REQUIRED", "awaiting_review"),
        ("", "awaiting_review"),
    ] {
        let got = apply_pr_status(pr_task("pr_unreachable"), "OPEN", decision, LATER).unwrap();
        assert_eq!(got.sub_status, expected, "{decision:?}");
    }
    assert!(apply_pr_status(pr_task("approved"), "OPEN", "APPROVED", LATER).is_none());
}

#[test]
fn an_open_pr_never_overwrites_conflict() {
    assert!(apply_pr_status(pr_task("conflict"), "OPEN", "APPROVED", LATER).is_none());
}

#[test]
fn a_report_for_a_task_outside_review_or_without_a_pr_changes_nothing() {
    let running = Task {
        status: "running".into(),
        sub_status: "active".into(),
        ..pr_task("")
    };
    assert!(apply_pr_status(running, "MERGED", "", LATER).is_none());
    let issue = Task {
        url_type: "issue".into(),
        ..pr_task("awaiting_review")
    };
    assert!(apply_pr_status(issue, "MERGED", "", LATER).is_none());
}

#[test]
fn unknown_raw_strings_are_refused() {
    assert!(validate_pr_status_report("DRAFT", "").is_err());
    assert!(validate_pr_status_report("", "").is_err());
    assert!(validate_pr_status_report("OPEN", "MAYBE").is_err());
    assert!(validate_pr_status_report("OPEN", "").is_ok());
}

#[test]
fn an_unreadable_pr_marks_pr_unreachable_once() {
    let got = apply_pr_unreadable(pr_task("awaiting_review"), LATER).unwrap();
    assert_eq!(got.sub_status, "pr_unreachable");
    assert_eq!(got.updated_at, LATER);
    assert!(apply_pr_unreadable(pr_task("pr_unreachable"), LATER).is_none());
    let backlog = Task {
        status: "done".into(),
        sub_status: "none".into(),
        ..pr_task("")
    };
    assert!(apply_pr_unreadable(backlog, LATER).is_none());
}

// -- RequestCloseTask (store-split.allium, M7 X1) --------------------------

#[test]
fn a_close_row_has_host_work_like_an_exit_session_row() {
    assert!(session_request_has_host_work(&session(
        1,
        "close",
        "done",
        "requested"
    )));
    assert_eq!(
        session_request_deadline("close", "2026-09-19 12:34:56.789"),
        "2026-09-19 12:35:26.789"
    );
    assert_eq!(
        session_request_backstop_due_micros("close", 1_000_000),
        1_000_000 + 40 * 1_000_000
    );
}

#[test]
fn a_done_close_of_a_running_task_stamps_completion_and_clears_window_and_stop() {
    let closed = apply_done_close(
        Task {
            stop_pending: true,
            ..running_task()
        },
        LATER,
    );
    assert_eq!(closed.status, "done");
    assert_eq!(closed.sub_status, "none");
    assert_eq!(closed.tmux_window, "");
    assert_eq!(closed.completed_at, LATER);
    assert_eq!(closed.updated_at, LATER);
    assert!(!closed.stop_pending);
}

/// A done -> done re-close is no transition: completed_at is kept.
#[test]
fn a_done_to_done_reclose_keeps_completed_at() {
    let done = Task {
        status: "done".into(),
        sub_status: "none".into(),
        completed_at: NOW.into(),
        tmux_window: "task-1".into(),
        ..running_task()
    };
    let closed = apply_done_close(done, LATER);
    assert_eq!(closed.completed_at, NOW);
    assert_eq!(closed.updated_at, LATER);
    assert_eq!(closed.tmux_window, "");
}

#[test]
fn a_close_needs_a_teardown_only_for_a_window_or_a_pr_url() {
    assert!(close_has_host_work(&running_task()));
    let no_window = Task {
        tmux_window: String::new(),
        ..running_task()
    };
    assert!(!close_has_host_work(&no_window));
    let with_pr = Task {
        url: "https://github.com/a/b/pull/1".into(),
        url_type: "pr".into(),
        ..no_window.clone()
    };
    assert!(close_has_host_work(&with_pr));
    let other_url = Task {
        url: "https://example.com".into(),
        url_type: "issue".into(),
        ..no_window
    };
    assert!(!close_has_host_work(&other_url));
}

#[test]
fn a_close_row_carries_the_window_read_and_no_token_chain_or_pr_url() {
    let row = new_close_request(&running_task(), HOST_B, NOW);
    assert_eq!(row.kind, "close");
    assert_eq!(row.action, "done");
    assert_eq!(row.host, HOST_A, "the task's own host wins");
    assert_eq!(row.state, "requested");
    assert_eq!(row.tmux_window, "task-1");
    assert_eq!(row.wrap_up_request, 0);
    assert_eq!(row.chained_task, 0);
    assert_eq!(row.pr_url, "");
    assert_eq!(row.requested_at, NOW);
    assert_eq!(row.deadline, session_request_deadline("close", NOW));
}

#[test]
fn a_close_row_falls_back_to_the_callers_host_when_the_task_has_none() {
    let hostless = Task {
        host: String::new(),
        tmux_window: String::new(),
        url: "https://github.com/a/b/pull/1".into(),
        url_type: "pr".into(),
        ..running_task()
    };
    let row = new_close_request(&hostless, HOST_B, NOW);
    assert_eq!(row.host, HOST_B);
    assert_eq!(row.tmux_window, "");
}

/// A close row consumes no token: neither a close row nor the open-ness of
/// one changes what check_exit_token answers.
#[test]
fn a_close_row_is_not_an_exit_token_consumer() {
    let rows = vec![
        done_wrap_up(3, "rebase", LATER),
        SessionRequest {
            id: 9,
            ..session(9, "close", "done", "requested")
        },
    ];
    assert!(check_exit_token(1, 3, &rows, &[]).is_ok());
}

// -- patch_task's transition rules (M7 X2) -----------------------------------

#[test]
fn leaving_running_clears_a_pending_stop_unless_the_patch_names_it() {
    let mut row = Task {
        status: "review".into(),
        stop_pending: true,
        ..running_task()
    };
    apply_patch_status_rules("running", false, &mut row);
    assert!(!row.stop_pending);

    let mut named = Task {
        status: "review".into(),
        stop_pending: true,
        ..running_task()
    };
    apply_patch_status_rules("running", true, &mut named);
    assert!(named.stop_pending);

    let mut stays = Task {
        stop_pending: true,
        ..running_task()
    };
    apply_patch_status_rules("running", false, &mut stays);
    assert!(stays.stop_pending);

    let mut from_review = Task {
        status: "done".into(),
        stop_pending: true,
        ..running_task()
    };
    apply_patch_status_rules("review", false, &mut from_review);
    assert!(
        from_review.stop_pending,
        "only a move out of running clears"
    );
}

// -- the rules version (sync.allium: config.rules_version) ----------------

/// W12: M5 moves the close and the wrap-up request rules into the module,
/// so the version is 4. The board's own copy is pinned equal to this one
/// by `src/sync/rules_gate.rs`'s test.
#[test]
fn the_rules_version_is_six_for_m7() {
    assert_eq!(RULES_VERSION, 6);
}
