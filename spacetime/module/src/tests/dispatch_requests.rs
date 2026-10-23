use super::*;

const HOST_A: &str = "host-a";
const HOST_B: &str = "host-b";
const NOW: &str = "2026-10-10 12:00:00.000";
const LATER: &str = "2026-10-10 12:00:30.000";

fn backlog_task() -> Task {
    Task {
        id: 1,
        ..blank_task()
    }
}

/// A request row in `state`, for task 1, provisioned by `host`.
fn request(state: &str, host: &str) -> DispatchRequest {
    DispatchRequest {
        id: 7,
        task_id: 1,
        host: host.into(),
        state: state.into(),
        requested_at: NOW.into(),
        ..blank_dispatch_request()
    }
}

// -- config (store-split.allium: config) ------------------------------

#[test]
fn the_request_timeout_is_sixty_seconds_and_the_backstop_grace_ten() {
    assert_eq!(DISPATCH_REQUEST_TIMEOUT_SECS, 60);
    assert_eq!(DISPATCH_REQUEST_BACKSTOP_GRACE_SECS, 10);
}

/// `DispatchRequest.deadline`: ONE deadline, from `requested_at`, in the
/// store's timestamp spelling so the host reads it straight off the row.
#[test]
fn the_deadline_is_requested_at_plus_the_timeout() {
    assert_eq!(
        dispatch_request_deadline("2026-09-19 12:34:56.789"),
        "2026-09-19 12:35:56.789"
    );
}

/// `DispatchRequestTimesOut` fires at deadline + grace, not at the deadline:
/// a live host's own stop must land first.
#[test]
fn the_backstop_timer_is_due_after_the_deadline_plus_the_grace() {
    let requested = 1_789_821_296_789_000_i64;
    assert_eq!(
        dispatch_request_backstop_due_micros(requested),
        requested + 70 * 1_000_000
    );
}

// -- the row's lifecycle ---------------------------------------------

#[test]
fn requested_and_provisioning_are_open_and_the_finished_states_are_not() {
    assert!(dispatch_request_is_open("requested"));
    assert!(dispatch_request_is_open("provisioning"));
    assert!(!dispatch_request_is_open("dispatched"));
    assert!(!dispatch_request_is_open("failed"));
}

/// `RequestDispatch`'s `DispatchRequest.created(...)`: kind dispatch, the
/// requester as host, state requested, requested_at from the module clock,
/// and every result column absent.
#[test]
fn a_new_request_is_requested_on_the_requesting_host() {
    let row = new_dispatch_request(1, HOST_A, "research", "mcp", NOW);
    assert_eq!(row.task_id, 1);
    assert_eq!(row.kind, "dispatch");
    assert_eq!(row.host, HOST_A);
    assert_eq!(row.mode, "research");
    assert_eq!(row.origin, "mcp");
    assert_eq!(row.state, "requested");
    assert_eq!(row.requested_at, NOW);
    assert_eq!(row.claimed_at, "");
    assert_eq!(row.finished_at, "");
    assert_eq!(row.reason, "");
    assert_eq!(row.worktree, "");
    assert_eq!(row.tmux_window, "");
}

/// The deadline is STORED on the row, computed by the module from the
/// row's own `requested_at`, so the host reads it rather than re-deriving
/// it from a copy of the config (D4).
#[test]
fn a_new_request_stores_its_deadline() {
    let row = new_dispatch_request(1, HOST_A, "dispatch", "user", NOW);
    assert_eq!(row.deadline, "2026-10-10 12:01:00.000");
    assert_eq!(row.deadline, dispatch_request_deadline(&row.requested_at));
}

#[test]
fn an_unparseable_requested_at_has_no_deadline() {
    assert_eq!(dispatch_request_deadline(""), "");
    assert_eq!(dispatch_request_deadline("yesterday"), "");
}

#[test]
fn only_the_four_launch_paths_are_origins() {
    for origin in ["user", "mcp", "epic_chain", "quick"] {
        assert!(validate_dispatch_origin(origin).is_ok(), "{origin}");
    }
    for origin in ["", "tui", "Quick"] {
        assert!(validate_dispatch_origin(origin).is_err(), "{origin:?}");
    }
}

// -- RequestDispatch: the refusals -----------------------------------

#[test]
fn a_backlog_task_with_no_open_request_may_be_requested() {
    assert!(validate_dispatch_request(&backlog_task(), HOST_A, "user", false, NOW).is_ok());
}

/// `DispatchClaimExclusive`, through the request reducer: a claimed task is
/// refused, and the refusal says so.
#[test]
fn a_task_that_is_not_in_backlog_is_refused() {
    for status in ["running", "review", "done"] {
        let task = Task {
            status: status.into(),
            ..backlog_task()
        };
        let why = validate_dispatch_request(&task, HOST_A, "user", false, NOW).unwrap_err();
        assert!(why.contains("already claimed"), "{status}: {why}");
    }
}

/// `requires: task.host = null or task.host = requester`.
#[test]
fn a_task_whose_worktree_is_on_another_host_is_refused() {
    let task = Task {
        host: HOST_A.into(),
        ..backlog_task()
    };
    assert!(validate_dispatch_request(&task, HOST_A, "user", false, NOW).is_ok());
    let why = validate_dispatch_request(&task, HOST_B, "user", false, NOW).unwrap_err();
    assert!(why.contains(HOST_A), "{why}");
}

/// `OneOpenDispatchRequestPerTask`: a task moved back to backlog by hand
/// during provisioning still holds its open request, and a second one is
/// refused rather than written beside it.
#[test]
fn a_task_with_an_open_request_is_refused_even_in_backlog() {
    let why = validate_dispatch_request(&backlog_task(), HOST_A, "user", true, NOW).unwrap_err();
    assert!(why.contains("open dispatch request"), "{why}");
}

/// `epics.allium: PhoenixIsNeverChained`, enforced by the request: the
/// epic chain's request for a phoenix task is refused with that reason,
/// and every other origin may take it.
#[test]
fn the_epic_chain_may_not_request_a_phoenix_task() {
    let phoenix = Task {
        phoenix: true,
        ..backlog_task()
    };
    let why = validate_dispatch_request(&phoenix, HOST_A, "epic_chain", false, NOW).unwrap_err();
    assert!(why.contains("phoenix"), "{why}");
    for origin in ["user", "mcp", "quick"] {
        assert!(
            validate_dispatch_request(&phoenix, HOST_A, origin, false, NOW).is_ok(),
            "{origin}"
        );
    }
    assert!(validate_dispatch_request(&backlog_task(), HOST_A, "epic_chain", false, NOW).is_ok());
}

// -- RequestDispatch: the mode (D3) ----------------------------------

#[test]
fn a_named_mode_is_kept() {
    for mode in ["dispatch", "research", "quick"] {
        assert_eq!(
            choose_dispatch_mode(&backlog_task(), mode).unwrap(),
            mode,
            "{mode}"
        );
    }
}

#[test]
fn an_unknown_mode_is_refused() {
    assert!(choose_dispatch_mode(&backlog_task(), "brainstorm").is_err());
}

/// Omitted (`""`) derives: a plan wins, then the research tag, else dispatch.
#[test]
fn an_omitted_mode_is_derived_by_todays_rule() {
    assert_eq!(
        choose_dispatch_mode(&backlog_task(), "").unwrap(),
        "dispatch"
    );

    let research = Task {
        tag: "research".into(),
        ..backlog_task()
    };
    assert_eq!(choose_dispatch_mode(&research, "").unwrap(), "research");

    let planned_research = Task {
        plan_path: "/plans/p.md".into(),
        ..research
    };
    assert_eq!(
        choose_dispatch_mode(&planned_research, "").unwrap(),
        "dispatch"
    );
}

/// quick is only ever named by its caller, never derived.
#[test]
fn quick_is_never_derived() {
    for tag in ["", "research", "chore", "quick"] {
        let task = Task {
            tag: tag.into(),
            ..backlog_task()
        };
        assert_ne!(choose_dispatch_mode(&task, "").unwrap(), "quick", "{tag}");
    }
}

// -- RequestDispatch: the claim --------------------------------------

/// The claim's writes, as `RequestDispatch` ensures them.
#[test]
fn the_request_claims_the_task() {
    let claimed = apply_dispatch_claim(backlog_task(), NOW);
    assert_eq!(claimed.status, "running");
    // core/SubStatus.default_for(running)
    assert_eq!(claimed.sub_status, "active");
    // The activity stamp and the subagent clear point are the new agent's
    // (`start_agent_on_fresh_launch`), not the Task's.
    assert_eq!(claimed.last_pre_tool_use_at, "");
    assert_eq!(claimed.id, 1);
}

// -- HostClaimsDispatch -----------------------------------------------

#[test]
fn the_rows_own_host_claims_a_requested_row() {
    assert!(validate_dispatch_request_claim(&request("requested", HOST_A), HOST_A).is_ok());
    let claimed = claim_dispatch_request_row(request("requested", HOST_A), LATER);
    assert_eq!(claimed.state, "provisioning");
    assert_eq!(claimed.claimed_at, LATER);
    assert_eq!(claimed.requested_at, NOW);
}

#[test]
fn another_host_cannot_claim_the_row() {
    assert!(validate_dispatch_request_claim(&request("requested", HOST_A), HOST_B).is_err());
}

/// Of two boards, or of a row delivered twice, exactly one claim is
/// accepted: the second finds the row no longer requested.
#[test]
fn a_row_that_is_not_requested_cannot_be_claimed() {
    for state in ["provisioning", "dispatched", "failed"] {
        assert!(
            validate_dispatch_request_claim(&request(state, HOST_A), HOST_A).is_err(),
            "{state}"
        );
    }
}

// -- HostReportsDispatched / HostReportsDispatchFailed ---------------

#[test]
fn the_rows_host_reports_on_a_provisioning_row() {
    assert!(validate_dispatch_report(&request("provisioning", HOST_A), HOST_A).is_ok());
}

#[test]
fn a_report_from_another_host_is_refused() {
    assert!(validate_dispatch_report(&request("provisioning", HOST_A), HOST_B).is_err());
}

/// A LATE REPORT IS REFUSED: after the timer (or a deletion) failed the
/// row, nothing the host says lands on it or on the task.
#[test]
fn a_report_on_a_row_that_is_not_provisioning_is_refused() {
    for state in ["requested", "dispatched", "failed"] {
        assert!(
            validate_dispatch_report(&request(state, HOST_A), HOST_A).is_err(),
            "{state}"
        );
    }
}

#[test]
fn a_dispatched_report_records_the_result_on_the_row() {
    let row = dispatched_request_row(
        request("provisioning", HOST_A),
        "/wt/1-task",
        "task-1",
        false,
        LATER,
    );
    assert_eq!(row.state, "dispatched");
    assert_eq!(row.finished_at, LATER);
    assert_eq!(row.worktree, "/wt/1-task");
    assert_eq!(row.tmux_window, "task-1");
    assert_eq!(row.reason, "");
}

/// The host's report of whether provisioning reused the worktree is kept
/// on the row (`DispatchRequest.reused_worktree`).
#[test]
fn a_dispatched_report_records_whether_the_worktree_was_reused() {
    let reused = dispatched_request_row(
        request("provisioning", HOST_A),
        "/wt",
        "task-1",
        true,
        LATER,
    );
    assert!(reused.reused_worktree);
    let created = dispatched_request_row(
        request("provisioning", HOST_A),
        "/wt",
        "task-1",
        false,
        LATER,
    );
    assert!(!created.reused_worktree);
    assert!(
        !request("requested", HOST_A).reused_worktree,
        "false until then"
    );
}

/// D4: a dispatched report is refused once the row's deadline has passed
/// by the module's clock; before it, it is in time.
#[test]
fn a_dispatched_report_past_the_deadline_is_refused() {
    let row = DispatchRequest {
        deadline: dispatch_request_deadline(NOW),
        ..request("provisioning", HOST_A)
    };
    assert!(validate_dispatched_report_in_time(&row, LATER).is_ok());
    let why = validate_dispatched_report_in_time(&row, "2026-10-10 12:01:00.000").unwrap_err();
    assert!(why.contains("deadline"), "{why}");
    assert!(validate_dispatched_report_in_time(&row, "2026-10-10 12:05:00.000").is_err());
}

/// `HostRepeatsDispatchedReport`: the same report on a row it already
/// dispatched is a repeat; any other report is not.
#[test]
fn a_repeat_of_the_dispatched_report_is_recognised() {
    let dispatched = dispatched_request_row(
        request("provisioning", HOST_A),
        "/wt",
        "task-1",
        false,
        LATER,
    );
    assert!(dispatch_report_is_repeat(
        &dispatched,
        HOST_A,
        "/wt",
        "task-1"
    ));
    assert!(!dispatch_report_is_repeat(
        &dispatched,
        HOST_B,
        "/wt",
        "task-1"
    ));
    assert!(!dispatch_report_is_repeat(
        &dispatched,
        HOST_A,
        "/other",
        "task-1"
    ));
    assert!(!dispatch_report_is_repeat(
        &dispatched,
        HOST_A,
        "/wt",
        "task-2"
    ));
    let failed = DispatchRequest {
        worktree: "/wt".into(),
        tmux_window: "task-1".into(),
        ..request("failed", HOST_A)
    };
    assert!(!dispatch_report_is_repeat(&failed, HOST_A, "/wt", "task-1"));
    assert!(!dispatch_report_is_repeat(
        &request("provisioning", HOST_A),
        HOST_A,
        "",
        ""
    ));
}

/// The writes today's post-provision patch made: worktree, window, and the
/// host paired with the worktree (`HostTracksWorktree`).
#[test]
fn a_dispatched_report_writes_the_tasks_worktree_and_host_and_the_window_goes_to_the_agent() {
    let running = apply_dispatch_claim(backlog_task(), NOW);
    let row = dispatched_request_row(
        request("provisioning", HOST_A),
        "/wt/1-task",
        "task-1",
        false,
        LATER,
    );
    let task = apply_dispatched_to_task(running, &row);
    assert_eq!(task.worktree, "/wt/1-task");
    assert_eq!(task.tmux_window, "", "the window is the agent's");
    assert_eq!(row.tmux_window, "task-1");
    assert_eq!(task.host, HOST_A);
    assert_eq!(task.status, "running");
}

#[test]
fn a_failed_report_records_the_reason_on_the_row() {
    let row = failed_request_row(
        request("provisioning", HOST_A),
        "git worktree add failed",
        LATER,
    );
    assert_eq!(row.state, "failed");
    assert_eq!(row.finished_at, LATER);
    assert_eq!(row.reason, "git worktree add failed");
    assert_eq!(row.worktree, "");
    assert_eq!(row.tmux_window, "");
}

// -- ReleaseClaimOfFailedRequest -------------------------------------

#[test]
fn a_failure_releases_a_claim_that_never_got_a_worktree() {
    let claimed = apply_dispatch_claim(backlog_task(), NOW);
    let released = release_claim_after_failure(claimed).expect("still claimed");
    assert_eq!(released.status, "backlog");
    // core/SubStatus.default_for(backlog)
    assert_eq!(released.sub_status, "none");
    assert_eq!(released.last_pre_tool_use_at, "");
}

/// Conditional, as today: a task that was provisioned meanwhile is left
/// alone rather than stomped back to backlog.
#[test]
fn a_failure_leaves_a_task_with_a_worktree_alone() {
    let provisioned = Task {
        worktree: "/wt/1-task".into(),
        ..apply_dispatch_claim(backlog_task(), NOW)
    };
    assert!(release_claim_after_failure(provisioned).is_none());
}

/// ...and one that moved on (back to backlog by hand, or into review).
#[test]
fn a_failure_leaves_a_task_that_is_not_running_alone() {
    for status in ["backlog", "review", "done"] {
        let moved = Task {
            status: status.into(),
            ..backlog_task()
        };
        assert!(release_claim_after_failure(moved).is_none(), "{status}");
    }
}

// -- DispatchRequestTimesOut (the scheduled reducer's body) ----------

/// The scheduled reducer fires only once its row is due (deadline +
/// grace), so its body is "fail the row 'timed out' if it is still open";
/// a row the host already finished is left as it is (its own report won).
#[test]
fn the_backstop_fails_an_open_row_as_timed_out() {
    for state in ["requested", "provisioning"] {
        let row = request(state, HOST_A);
        assert!(dispatch_request_is_open(&row.state), "{state}");
        let failed = failed_request_row(row, DISPATCH_REASON_TIMED_OUT, LATER);
        assert_eq!(failed.state, "failed");
        assert_eq!(failed.reason, "timed out");
        assert_eq!(failed.finished_at, LATER);
    }
    for state in ["dispatched", "failed"] {
        assert!(!dispatch_request_is_open(state), "{state}");
    }
}

// -- HostRestartFailsInterrupted -------------------------------------

/// Only the PROVISIONING rows of the host's own (the reducer reads them
/// by its `host` index): a requested row is picked up by the starting
/// board, and finished rows are history.
#[test]
fn a_restart_interrupts_only_provisioning_rows() {
    let rows = vec![
        DispatchRequest {
            id: 1,
            ..request("provisioning", HOST_A)
        },
        DispatchRequest {
            id: 2,
            ..request("requested", HOST_A)
        },
        DispatchRequest {
            id: 4,
            ..request("dispatched", HOST_A)
        },
        DispatchRequest {
            id: 5,
            ..request("failed", HOST_A)
        },
    ];
    let ids: Vec<i64> = interrupted_dispatch_requests(rows)
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(ids, vec![1]);
}

#[test]
fn the_failure_reasons_are_the_specs_wording() {
    assert_eq!(DISPATCH_REASON_TIMED_OUT, "timed out");
    assert_eq!(DISPATCH_REASON_HOST_RESTARTED, "host restarted");
    assert_eq!(DISPATCH_REASON_TASK_DELETED, "task deleted");
}

// -- the rules version (sync.allium: config.rules_version) -----------

// -- D11: who may act for a host --------------------------------------

fn host(owner: &str) -> Host {
    Host {
        id: HOST_A.into(),
        label: String::new(),
        owner: owner.into(),
    }
}

/// The host's own owner may name it.
#[test]
fn the_hosts_owner_may_act_for_it() {
    assert_eq!(
        validate_host_acting_for(Some(&host("c0ffee")), HOST_A, "c0ffee"),
        Ok(())
    );
}

/// Another identity may not, and the refusal says whose host it is.
#[test]
fn another_identity_may_not_act_for_a_host() {
    let why = validate_host_acting_for(Some(&host("c0ffee")), HOST_A, "beef").unwrap_err();
    assert!(why.contains(HOST_A) && why.contains("beef"), "{why}");
}

/// A host with no owner yet is refused for everyone: there is no one it
/// is known to belong to.
#[test]
fn nobody_may_act_for_an_ownerless_host() {
    let why = validate_host_acting_for(Some(&host("")), HOST_A, "c0ffee").unwrap_err();
    assert!(why.contains("no owner"), "{why}");
}

// -- LocalHostOwnerIsWrittenOnce, enforced by register_host -------------

/// A caller may register a host as its own, and re-register it (a label
/// change) as often as it likes.
#[test]
fn a_caller_may_register_and_re_register_its_own_host() {
    assert_eq!(validate_host_registration(None, "c0ffee", "c0ffee"), Ok(()));
    assert_eq!(
        validate_host_registration(Some(&host("c0ffee")), "c0ffee", "c0ffee"),
        Ok(())
    );
}

/// A NEW host row must name its owner: host ids are public, so an
/// ownerless row would be adoptable by whichever caller registered the id
/// next — a teammate could lock a new machine out of its own host.
#[test]
fn a_new_host_may_not_be_registered_ownerless() {
    let why = validate_host_registration(None, "", "c0ffee").unwrap_err();
    assert!(why.contains("owner"), "{why}");
}

/// A legacy row written before owners were enforced may still be adopted
/// by the first caller naming itself, and an ownerless re-register of it
/// changes no ownership.
#[test]
fn a_legacy_ownerless_row_may_still_be_adopted() {
    assert_eq!(
        validate_host_registration(Some(&host("")), "c0ffee", "c0ffee"),
        Ok(())
    );
    assert_eq!(
        validate_host_registration(Some(&host("")), "", "c0ffee"),
        Ok(())
    );
}

/// Nobody may name another identity as a host's owner.
#[test]
fn a_caller_may_not_name_another_identity_as_owner() {
    let why = validate_host_registration(None, "beef", "c0ffee").unwrap_err();
    assert!(why.contains("beef") && why.contains("c0ffee"), "{why}");
}

/// An owned host's owner never changes — not to the caller, not to empty.
#[test]
fn an_owned_hosts_owner_is_never_rewritten() {
    let why = validate_host_registration(Some(&host("beef")), "c0ffee", "c0ffee").unwrap_err();
    assert!(why.contains("beef"), "{why}");
    let why = validate_host_registration(Some(&host("c0ffee")), "", "c0ffee").unwrap_err();
    assert!(why.contains("c0ffee"), "{why}");
}

/// A host missing from the registry is refused.
#[test]
fn nobody_may_act_for_an_unregistered_host() {
    let why = validate_host_acting_for(None, HOST_B, "c0ffee").unwrap_err();
    assert!(
        why.contains(HOST_B) && why.contains("not registered"),
        "{why}"
    );
}
