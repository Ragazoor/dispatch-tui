use super::*;

// -- store-split M4 (task #53257): dispatch as request rows ------------------
//
// `store-split.allium`: DispatchRequest and its rules. Each test drives the
// reducers a board and its peers call, and reads the rows back over SQL. What
// a unit test can pin without a store (the refusal wording, mode derivation,
// the backstop's body on a row whose deadline has passed) is pinned in the
// module's own `tests.rs` ("dispatch_requests"); here it is the transaction:
// that the claim and the row land together, that a refusal writes nothing, and
// that the scheduled timer row is created and cancelled with its request.
//
// THE BACKSTOP ITSELF IS NOT FIRED HERE. It is due 70 seconds after the
// request, and the suite does not wait on a wall clock
// (`scripts/check-no-test-sleep.sh`). Its schedule row is asserted to exist and
// to go away when the request finishes; what it does when it fires is the
// module unit test `the_backstop_fails_an_open_row_as_timed_out`.

/// One backlog task per `(id, tag, plan_path, host)`, all in epic 1, with
/// `host-a` and `host-b` registered to this instance's own identity.
fn seed_dispatchable(instance: &Instance, tasks: &[(i64, &str, &str, &str)]) {
    register_own_hosts(instance);
    let seeded = instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "backlog", 0)]).to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));
    let rows: Vec<serde_json::Value> = tasks
        .iter()
        .map(|(id, tag, plan, host)| {
            let mut row = task_json(*id, "t", "backlog", 1, host);
            row["tag"] = serde_json::json!(tag);
            row["plan_path"] = serde_json::json!(plan);
            row
        })
        .collect();
    let seeded = instance.call("seed_tasks", &[&serde_json::json!(rows).to_string()]);
    assert!(seeded.status.success(), "{}", describe(&seeded));
}

/// `RequestDispatch`: the claim and the row land in ONE transaction, and the
/// request's backstop timer is scheduled with it.
#[test]
fn a_dispatch_request_claims_the_task_and_writes_a_requested_row() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);

    let requested = request_dispatch(&instance, 1, "host-a", "user", "");
    assert!(requested.status.success(), "{}", describe(&requested));

    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "running"
    );
    assert_eq!(
        column(&instance, "SELECT sub_status FROM tasks WHERE id = 1"),
        "active"
    );
    assert_ne!(
        current_agent_column(&instance, "last_pre_tool_use_at", 1),
        "",
        "the claim stamps last_pre_tool_use_at"
    );

    assert_eq!(request_count(&instance), "1");
    assert_eq!(request_column(&instance, "kind", 1), "dispatch");
    assert_eq!(request_column(&instance, "host", 1), "host-a");
    assert_eq!(request_column(&instance, "origin", 1), "user");
    assert_eq!(request_column(&instance, "mode", 1), "dispatch");
    assert_eq!(request_column(&instance, "state", 1), "requested");
    assert_eq!(
        request_column(&instance, "requested_at", 1).len(),
        23,
        "requested_at is the module clock, in the store's timestamp spelling"
    );
    assert_eq!(request_column(&instance, "claimed_at", 1), "");
    assert_eq!(request_column(&instance, "finished_at", 1), "");
    let id: i64 = request_column(&instance, "id", 1).parse().expect("id");
    assert_eq!(
        timer_count(&instance, id),
        "1",
        "one backstop timer per request"
    );
}

/// `DispatchRequest.deadline` is stored on the row by the module: its own
/// `requested_at` plus `config.dispatch_request_timeout` (60 seconds), in the
/// store's timestamp spelling, so the host reads it straight off the row.
#[test]
fn a_dispatch_request_stores_its_deadline_sixty_seconds_after_it_was_requested() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    let requested = request_dispatch(&instance, 1, "host-a", "user", "");
    assert!(requested.status.success(), "{}", describe(&requested));

    let format = "%Y-%m-%d %H:%M:%S%.3f";
    let parse = |s: &str| {
        chrono::NaiveDateTime::parse_from_str(s, format)
            .unwrap_or_else(|e| panic!("{s:?} is not a store timestamp: {e}"))
    };
    let requested_at = parse(&request_column(&instance, "requested_at", 1));
    let deadline = parse(&request_column(&instance, "deadline", 1));
    assert_eq!(deadline - requested_at, chrono::Duration::seconds(60));
}

/// `RequestDispatch` moves the dispatch-claim subagent clear point into the
/// same transaction: entries gone, count zero, no stop pending.
#[test]
fn a_dispatch_request_clears_the_tasks_subagent_state() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    let started = instance.call(
        "subagent_start",
        &["1", "agent-1", "session-1", "2026-10-10T10:00:00Z"],
    );
    assert!(started.status.success(), "{}", describe(&started));

    let requested = request_dispatch(&instance, 1, "host-a", "user", "");
    assert!(requested.status.success(), "{}", describe(&requested));

    assert!(no_rows(
        &instance,
        "SELECT agent_id FROM task_subagents WHERE task_id = 1"
    ));
    assert_eq!(current_agent_column(&instance, "live_subagents", 1), "0");
    assert_eq!(current_agent_column(&instance, "stop_pending", 1), "false");
}

/// `DispatchClaimExclusive`, through the request reducer: the second request
/// is REFUSED, says why, and writes no second row.
#[test]
fn a_second_request_for_a_claimed_task_is_refused_and_writes_nothing() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);

    let first = request_dispatch(&instance, 1, "host-a", "user", "");
    assert!(first.status.success(), "{}", describe(&first));
    let second = request_dispatch(&instance, 1, "host-b", "mcp", "");
    assert!(!second.status.success(), "{}", describe(&second));
    assert!(
        describe(&second).contains("already claimed"),
        "the refusal must say why: {}",
        describe(&second)
    );
    assert_eq!(request_count(&instance), "1");
}

/// `requires: task.host = null or task.host = requester`.
#[test]
fn a_request_for_a_task_on_another_host_is_refused_and_writes_nothing() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "host-a")]);

    let refused = request_dispatch(&instance, 1, "host-b", "user", "");
    assert!(!refused.status.success(), "{}", describe(&refused));
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "backlog"
    );
    assert!(no_rows(&instance, "SELECT id FROM dispatch_requests"));
    assert!(no_rows(
        &instance,
        "SELECT request_id FROM dispatch_request_timeouts"
    ));

    let won = request_dispatch(&instance, 1, "host-a", "user", "");
    assert!(won.status.success(), "{}", describe(&won));
}

/// `OneOpenDispatchRequestPerTask`: a task moved back to backlog by hand while
/// its request is still open is not requested a second time.
#[test]
fn a_backlog_task_with_an_open_request_is_refused() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    let first = request_dispatch(&instance, 1, "host-a", "user", "");
    assert!(first.status.success(), "{}", describe(&first));
    move_task(&instance, 1, "backlog", "none");

    let refused = request_dispatch(&instance, 1, "host-a", "user", "");
    assert!(!refused.status.success(), "{}", describe(&refused));
    assert!(
        describe(&refused).contains("open dispatch request"),
        "{}",
        describe(&refused)
    );
    assert_eq!(request_count(&instance), "1");
}

/// D3: the caller's mode when named; otherwise plan -> dispatch, research tag
/// -> research, else dispatch. quick only when named.
#[test]
fn the_request_mode_is_derived_when_omitted_and_kept_when_named() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(
        &instance,
        &[
            (1, "", "", ""),
            (2, "research", "", ""),
            (3, "research", "/plans/p.md", ""),
            (4, "", "", ""),
            (5, "", "", ""),
        ],
    );
    for (task, origin, mode) in [
        (1, "user", ""),
        (2, "user", ""),
        (3, "user", ""),
        (4, "quick", "quick"),
        (5, "user", "research"),
    ] {
        let out = request_dispatch(&instance, task, "host-a", origin, mode);
        assert!(out.status.success(), "task {task}: {}", describe(&out));
    }
    assert_eq!(request_column(&instance, "mode", 1), "dispatch");
    assert_eq!(request_column(&instance, "mode", 2), "research");
    assert_eq!(request_column(&instance, "mode", 3), "dispatch");
    assert_eq!(request_column(&instance, "mode", 4), "quick");
    assert_eq!(request_column(&instance, "mode", 5), "research");
    assert_eq!(request_column(&instance, "origin", 4), "quick");
}

/// `HostClaimsDispatch`: only the row's host claims it, and only once.
#[test]
fn only_the_rows_host_claims_a_request_and_only_once() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    request_dispatch(&instance, 1, "host-a", "user", "");
    let id = request_column(&instance, "id", 1);

    let foreign = instance.call("claim_dispatch_request", &[&id, "host-b"]);
    assert!(!foreign.status.success(), "{}", describe(&foreign));
    assert_eq!(request_column(&instance, "state", 1), "requested");

    let claimed = instance.call("claim_dispatch_request", &[&id, "host-a"]);
    assert!(claimed.status.success(), "{}", describe(&claimed));
    assert_eq!(request_column(&instance, "state", 1), "provisioning");
    assert_eq!(request_column(&instance, "claimed_at", 1).len(), 23);

    let again = instance.call("claim_dispatch_request", &[&id, "host-a"]);
    assert!(
        !again.status.success(),
        "a second claim of the same row must be refused: {}",
        describe(&again)
    );
}

/// `HostReportsDispatched`: the row records the result, the task gets its
/// worktree, window and host, and the timer is cancelled.
#[test]
fn a_dispatched_report_records_the_result_on_the_row_and_the_task() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    request_dispatch(&instance, 1, "host-a", "user", "");
    let id = request_column(&instance, "id", 1);
    instance.call("claim_dispatch_request", &[&id, "host-a"]);

    let reported = instance.call(
        "report_dispatch_provisioned",
        &[&id, "host-a", "/wt/1-t", "task-1", "true"],
    );
    assert!(reported.status.success(), "{}", describe(&reported));

    assert_eq!(request_column(&instance, "state", 1), "dispatched");
    assert_eq!(request_column(&instance, "finished_at", 1).len(), 23);
    assert_eq!(request_column(&instance, "worktree", 1), "/wt/1-t");
    assert_eq!(request_column(&instance, "tmux_window", 1), "task-1");
    assert_eq!(
        request_column(&instance, "reused_worktree", 1),
        "true",
        "the host's reused-worktree answer is kept on the row"
    );
    assert_eq!(
        column(&instance, "SELECT worktree FROM tasks WHERE id = 1"),
        "/wt/1-t"
    );
    assert_eq!(current_agent_column(&instance, "tmux_window", 1), "task-1");
    assert_eq!(
        column(&instance, "SELECT host FROM tasks WHERE id = 1"),
        "host-a"
    );
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "running"
    );
    assert_eq!(timer_count(&instance, id.parse().unwrap()), "0");
}

/// `HostRepeatsDispatchedReport`: the same dispatched report sent again — a
/// host whose first report got no answer — is accepted and changes nothing;
/// a report naming another worktree is still refused.
#[test]
fn a_repeated_dispatched_report_is_accepted_and_changes_nothing() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    request_dispatch(&instance, 1, "host-a", "user", "");
    let id = request_column(&instance, "id", 1);
    instance.call("claim_dispatch_request", &[&id, "host-a"]);
    let first = instance.call(
        "report_dispatch_provisioned",
        &[&id, "host-a", "/wt/1-t", "task-1", "false"],
    );
    assert!(first.status.success(), "{}", describe(&first));
    let finished_at = request_column(&instance, "finished_at", 1);
    let updated_at = column(&instance, "SELECT updated_at FROM tasks WHERE id = 1");

    let again = instance.call(
        "report_dispatch_provisioned",
        &[&id, "host-a", "/wt/1-t", "task-1", "false"],
    );
    assert!(again.status.success(), "{}", describe(&again));
    assert_eq!(request_column(&instance, "finished_at", 1), finished_at);
    assert_eq!(
        column(&instance, "SELECT updated_at FROM tasks WHERE id = 1"),
        updated_at
    );

    let other = instance.call(
        "report_dispatch_provisioned",
        &[&id, "host-a", "/wt/other", "task-1", "false"],
    );
    assert!(!other.status.success(), "{}", describe(&other));
    assert_eq!(request_column(&instance, "worktree", 1), "/wt/1-t");
}

/// `epics.allium: PhoenixIsNeverChained`, enforced by `RequestDispatch`: the
/// epic chain's request for a phoenix task is refused and writes nothing;
/// a request from the TUI takes it.
#[test]
fn the_epic_chain_may_not_request_a_phoenix_task() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    let mut patch = empty_task_patch();
    patch["phoenix"] = serde_json::json!({"some": true});
    let marked = instance.call("patch_task", &["1", &patch.to_string()]);
    assert!(marked.status.success(), "{}", describe(&marked));

    let chained = request_dispatch(&instance, 1, "host-a", "epic_chain", "");
    assert!(!chained.status.success(), "{}", describe(&chained));
    assert!(
        describe(&chained).contains("phoenix"),
        "{}",
        describe(&chained)
    );
    assert_eq!(request_count(&instance), "0");
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "backlog"
    );

    let by_hand = request_dispatch(&instance, 1, "host-a", "user", "");
    assert!(by_hand.status.success(), "{}", describe(&by_hand));
}

/// `requires: reporter = request.host` and `requires: request.state =
/// provisioning`, for both reports.
#[test]
fn a_report_from_another_host_or_on_an_unclaimed_row_is_refused() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    request_dispatch(&instance, 1, "host-a", "user", "");
    let id = request_column(&instance, "id", 1);

    let unclaimed = instance.call(
        "report_dispatch_provisioned",
        &[&id, "host-a", "/wt/1-t", "task-1", "false"],
    );
    assert!(!unclaimed.status.success(), "{}", describe(&unclaimed));
    let unclaimed = instance.call("report_dispatch_failed", &[&id, "host-a", "boom"]);
    assert!(!unclaimed.status.success(), "{}", describe(&unclaimed));

    instance.call("claim_dispatch_request", &[&id, "host-a"]);
    let foreign = instance.call(
        "report_dispatch_provisioned",
        &[&id, "host-b", "/wt/1-t", "task-1", "false"],
    );
    assert!(!foreign.status.success(), "{}", describe(&foreign));
    let foreign = instance.call("report_dispatch_failed", &[&id, "host-b", "boom"]);
    assert!(!foreign.status.success(), "{}", describe(&foreign));

    assert_eq!(request_column(&instance, "state", 1), "provisioning");
    assert_eq!(
        column(&instance, "SELECT worktree FROM tasks WHERE id = 1"),
        ""
    );
}

/// `HostReportsDispatchFailed` + `ReleaseClaimOfFailedRequest`: the row fails
/// with the host's reason, the task goes back to backlog with its claim stamp
/// cleared, and the timer is cancelled.
#[test]
fn a_failed_report_fails_the_row_and_releases_the_claim() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    request_dispatch(&instance, 1, "host-a", "user", "");
    let id = request_column(&instance, "id", 1);
    instance.call("claim_dispatch_request", &[&id, "host-a"]);

    let failed = instance.call(
        "report_dispatch_failed",
        &[&id, "host-a", "deadline passed"],
    );
    assert!(failed.status.success(), "{}", describe(&failed));

    assert_eq!(request_column(&instance, "state", 1), "failed");
    assert_eq!(request_column(&instance, "reason", 1), "deadline passed");
    assert_eq!(request_column(&instance, "finished_at", 1).len(), 23);
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "backlog"
    );
    assert_eq!(
        column(&instance, "SELECT sub_status FROM tasks WHERE id = 1"),
        "none"
    );
    // The agent the claim started ends with the release, holding nothing.
    assert_eq!(
        column(&instance, "SELECT state FROM agents WHERE task_id = 1"),
        "ended"
    );
    assert_eq!(timer_count(&instance, id.parse().unwrap()), "0");

    // Released, so it can be requested afresh.
    let again = request_dispatch(&instance, 1, "host-a", "user", "");
    assert!(again.status.success(), "{}", describe(&again));
}

/// The release is CONDITIONAL: a task that got a worktree meanwhile stays
/// running even though its request failed.
#[test]
fn a_failed_report_leaves_a_task_that_got_a_worktree_alone() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    request_dispatch(&instance, 1, "host-a", "user", "");
    let id = request_column(&instance, "id", 1);
    instance.call("claim_dispatch_request", &[&id, "host-a"]);
    let patched = instance.call(
        "patch_task",
        &["1", &patch_setting("worktree", "/wt/1-t").to_string()],
    );
    assert!(patched.status.success(), "{}", describe(&patched));

    let failed = instance.call("report_dispatch_failed", &[&id, "host-a", "boom"]);
    assert!(failed.status.success(), "{}", describe(&failed));
    assert_eq!(request_column(&instance, "state", 1), "failed");
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "running"
    );
}

/// A LATE REPORT IS REFUSED: once the row has failed, a dispatched report
/// writes neither the row nor the task.
#[test]
fn a_dispatched_report_after_the_row_failed_is_refused() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    request_dispatch(&instance, 1, "host-a", "user", "");
    let id = request_column(&instance, "id", 1);
    instance.call("claim_dispatch_request", &[&id, "host-a"]);
    // A restart is the failure a test can trigger without a clock; the
    // backstop's "timed out" lands on the row the same way.
    let interrupted = instance.call("fail_interrupted_dispatch_requests", &["host-a"]);
    assert!(interrupted.status.success(), "{}", describe(&interrupted));

    let late = instance.call(
        "report_dispatch_provisioned",
        &[&id, "host-a", "/wt/1-t", "task-1", "false"],
    );
    assert!(!late.status.success(), "{}", describe(&late));
    assert_eq!(request_column(&instance, "state", 1), "failed");
    assert_eq!(request_column(&instance, "worktree", 1), "");
    assert_eq!(
        column(&instance, "SELECT worktree FROM tasks WHERE id = 1"),
        ""
    );
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "backlog"
    );
}

/// `HostRestartFailsInterrupted`: this host's PROVISIONING rows fail with
/// "host restarted" and release their claims; its requested rows and every
/// other host's rows are untouched.
#[test]
fn a_restart_fails_only_this_hosts_provisioning_requests() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(
        &instance,
        &[(1, "", "", ""), (2, "", "", ""), (3, "", "", "")],
    );
    request_dispatch(&instance, 1, "host-a", "user", "");
    request_dispatch(&instance, 2, "host-a", "user", "");
    request_dispatch(&instance, 3, "host-b", "user", "");
    let first = request_column(&instance, "id", 1);
    let third = request_column(&instance, "id", 3);
    instance.call("claim_dispatch_request", &[&first, "host-a"]);
    instance.call("claim_dispatch_request", &[&third, "host-b"]);

    let interrupted = instance.call("fail_interrupted_dispatch_requests", &["host-a"]);
    assert!(interrupted.status.success(), "{}", describe(&interrupted));

    assert_eq!(request_column(&instance, "state", 1), "failed");
    assert_eq!(request_column(&instance, "reason", 1), "host restarted");
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "backlog"
    );
    assert_eq!(timer_count(&instance, first.parse().unwrap()), "0");
    assert_eq!(request_column(&instance, "state", 2), "requested");
    assert_eq!(request_column(&instance, "state", 3), "provisioning");
}

/// `DeletingATaskFailsItsOpenRequest`, through `delete_task`: the open request
/// fails "task deleted" in the same reducer and its timer is cancelled.
#[test]
fn deleting_a_task_fails_its_open_request_and_cancels_its_timer() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    request_dispatch(&instance, 1, "host-a", "user", "");
    let id = request_column(&instance, "id", 1);
    // delete_task requires done; a manual move leaves the request open.
    move_task(&instance, 1, "done", "none");

    let deleted = instance.call("delete_task", &["1"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));

    assert!(no_rows(&instance, "SELECT id FROM tasks WHERE id = 1"));
    assert_eq!(request_column(&instance, "state", 1), "failed");
    assert_eq!(request_column(&instance, "reason", 1), "task deleted");
    assert_eq!(timer_count(&instance, id.parse().unwrap()), "0");
}

/// The same, through `batch_delete`.
#[test]
fn batch_delete_fails_the_open_requests_of_the_tasks_it_deletes() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", ""), (2, "", "", "")]);
    request_dispatch(&instance, 1, "host-a", "user", "");
    request_dispatch(&instance, 2, "host-a", "user", "");
    let second = request_column(&instance, "id", 2);
    instance.call("claim_dispatch_request", &[&second, "host-a"]);
    move_task(&instance, 1, "done", "none");
    move_task(&instance, 2, "done", "none");

    let deleted = instance.call("batch_delete", &["[1, 2]", "[]"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));

    for task in [1, 2] {
        assert_eq!(request_column(&instance, "state", task), "failed");
        assert_eq!(request_column(&instance, "reason", task), "task deleted");
    }
    assert!(no_rows(
        &instance,
        "SELECT request_id FROM dispatch_request_timeouts"
    ));
}

/// The same, through `batch_delete`'s epic arm: a deleted epic's subtree tasks
/// fail their open requests too.
#[test]
fn deleting_an_epic_fails_the_open_requests_of_its_subtree_tasks() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    request_dispatch(&instance, 1, "host-a", "user", "");
    move_task(&instance, 1, "done", "none");

    let deleted = instance.call("batch_delete", &["[]", "[1]"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));

    assert_eq!(request_column(&instance, "state", 1), "failed");
    assert_eq!(request_column(&instance, "reason", 1), "task deleted");
    assert!(no_rows(
        &instance,
        "SELECT request_id FROM dispatch_request_timeouts"
    ));
}

/// The same, through `ingest_feed`'s stale delete, which removes a feed task
/// whatever its status — so here the request is still open on a running task.
#[test]
fn a_feed_ingest_that_deletes_a_stale_task_fails_its_open_request() {
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
                "ext-1", "one", "backlog"
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
    let requested = request_dispatch(&instance, task, "host-a", "user", "");
    assert!(requested.status.success(), "{}", describe(&requested));

    let ingested = instance.call("ingest_feed", &["1", "[]", "false", "test-creator"]);
    assert!(ingested.status.success(), "{}", describe(&ingested));

    assert!(no_rows(
        &instance,
        &format!("SELECT id FROM tasks WHERE id = {task}")
    ));
    assert_eq!(request_column(&instance, "state", task), "failed");
    assert_eq!(request_column(&instance, "reason", task), "task deleted");
    assert!(no_rows(
        &instance,
        "SELECT request_id FROM dispatch_request_timeouts"
    ));
}

// -- The epic rollup follows every request outcome ------------------------------
//
// `store-split.allium`: `ReleaseClaimOfFailedRequest` and
// `HostReportsDispatched` run the epic status rollup for the task's epic
// chain in the same reducer, so an epic never waits on a board to recalculate
// it after the module moved its subtask. Observed through a STALE epic: one
// seeded `done` over an unfinished subtask, which any rollup returns to
// backlog. The backstop timer takes the same failure path
// (`fail_dispatch_request`) but is not fired here — see the section header
// above.

/// [`seed_dispatchable`]'s one task, under an epic left `done` by a rollup
/// that never ran.
fn seed_under_a_stale_done_epic(instance: &Instance) {
    register_own_hosts(instance);
    let seeded = instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "done", 0)]).to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));
    let seeded = instance.call(
        "seed_tasks",
        &[&serde_json::json!([task_json(1, "t", "backlog", 1, "")]).to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));
}

/// A requested and claimed row for task 1 on `host-a`, under an epic made
/// stale AFTER the request — the request rolls the chain up itself, so the
/// stale epic is seeded once the row is provisioning. Returns the request id.
fn provisioning_under_a_stale_done_epic(instance: &Instance) -> String {
    register_own_hosts(instance);
    let seeded = instance.call(
        "seed_tasks",
        &[&serde_json::json!([task_json(1, "t", "backlog", 1, "")]).to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));
    let requested = request_dispatch(instance, 1, "host-a", "user", "");
    assert!(requested.status.success(), "{}", describe(&requested));
    let id = request_column(instance, "id", 1);
    let claimed = instance.call("claim_dispatch_request", &[&id, "host-a"]);
    assert!(claimed.status.success(), "{}", describe(&claimed));
    let seeded = instance.call(
        "seed_epics",
        &[&serde_json::json!([epic_json(1, "E", "done", 0)]).to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));
    assert_eq!(
        column(instance, "SELECT status FROM epics WHERE id = 1"),
        "done",
        "the fixture's epic must still be stale when the outcome lands"
    );
    id
}

/// `RequestDispatch` rolls up the claimed task's epic chain in the same
/// reducer.
#[test]
fn a_request_rolls_up_the_claimed_subtasks_epic() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_under_a_stale_done_epic(&instance);

    let requested = request_dispatch(&instance, 1, "host-a", "user", "");
    assert!(requested.status.success(), "{}", describe(&requested));

    assert_eq!(
        column(&instance, "SELECT status FROM epics WHERE id = 1"),
        "backlog"
    );
}

#[test]
fn a_failed_report_rolls_up_the_released_subtasks_epic() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let id = provisioning_under_a_stale_done_epic(&instance);

    let failed = instance.call("report_dispatch_failed", &[&id, "host-a", "boom"]);
    assert!(failed.status.success(), "{}", describe(&failed));

    assert_eq!(
        column(&instance, "SELECT status FROM epics WHERE id = 1"),
        "backlog"
    );
}

#[test]
fn a_restart_sweep_rolls_up_the_released_subtasks_epic() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    provisioning_under_a_stale_done_epic(&instance);

    let swept = instance.call("fail_interrupted_dispatch_requests", &["host-a"]);
    assert!(swept.status.success(), "{}", describe(&swept));

    assert_eq!(
        column(&instance, "SELECT status FROM epics WHERE id = 1"),
        "backlog"
    );
}

#[test]
fn a_dispatched_report_rolls_up_the_subtasks_epic() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let id = provisioning_under_a_stale_done_epic(&instance);

    let reported = instance.call(
        "report_dispatch_provisioned",
        &[&id, "host-a", "/wt/1-t", "task-1", "false"],
    );
    assert!(reported.status.success(), "{}", describe(&reported));

    assert_eq!(
        column(&instance, "SELECT status FROM epics WHERE id = 1"),
        "backlog"
    );
}

// -- D11: only a host's owner may act for it ------------------------------------
//
// `store-split.allium`, D11: request_dispatch, claim_dispatch_request, both
// reports and the restart sweep refuse unless the host they name is
// registered with an owner and that owner is the caller. Each refusal writes
// nothing.

/// A request records who made it, from the call.
#[test]
fn a_request_records_its_caller_as_requested_by() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);

    let requested = request_dispatch(&instance, 1, "host-a", "user", "");
    assert!(requested.status.success(), "{}", describe(&requested));

    assert_eq!(
        request_column(&instance, "requested_by", 1),
        instance.identity()
    );
}

/// A request naming a host somebody else owns is refused, and writes nothing:
/// no claim, no row.
#[test]
fn a_request_naming_another_persons_host_is_refused() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    register_teammates_host(&instance, "their-host");

    let refused = request_dispatch(&instance, 1, "their-host", "user", "");

    assert!(!refused.status.success(), "{}", describe(&refused));
    assert!(
        describe(&refused).contains("their-host"),
        "{}",
        describe(&refused)
    );
    assert_eq!(request_count(&instance), "0");
    assert_eq!(
        column(&instance, "SELECT status FROM tasks WHERE id = 1"),
        "backlog"
    );
}

/// A NEW host may not be registered ownerless (`store-split.allium`, D11):
/// host ids are public, so an ownerless row would go to whoever registered
/// the id next as their own. The refused registration writes no row, and a
/// host with no row cannot be named. (An ownerless LEGACY row is refused for
/// everyone too; no reducer can write one any more, so that branch is the
/// module unit test `nobody_may_act_for_an_ownerless_host`.)
#[test]
fn a_request_naming_an_ownerless_or_unregistered_host_is_refused() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    let ownerless = instance.call("register_host", &["fresh-host", "", ""]);
    assert!(!ownerless.status.success(), "{}", describe(&ownerless));
    assert!(
        describe(&ownerless).contains("owner"),
        "{}",
        describe(&ownerless)
    );
    assert!(no_rows(
        &instance,
        "SELECT id FROM hosts WHERE id = 'fresh-host'"
    ));

    for host in ["fresh-host", "nowhere"] {
        let refused = request_dispatch(&instance, 1, host, "user", "");
        assert!(!refused.status.success(), "{}", describe(&refused));
        assert!(
            describe(&refused).contains("not registered"),
            "{}",
            describe(&refused)
        );
    }
    assert_eq!(request_count(&instance), "0");
}

/// The claim, both reports and the restart sweep name the host too, so a
/// teammate can do none of them to this host's rows.
#[test]
fn claims_reports_and_the_sweep_are_refused_for_another_persons_host() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", ""), (2, "", "", "")]);
    request_dispatch(&instance, 1, "host-a", "user", "");
    request_dispatch(&instance, 2, "host-a", "user", "");
    let requested = request_column(&instance, "id", 1);
    let provisioning = request_column(&instance, "id", 2);
    let claimed = instance.call("claim_dispatch_request", &[&provisioning, "host-a"]);
    assert!(claimed.status.success(), "{}", describe(&claimed));

    let teammate = |reducer: &str, args: &[&str]| instance.call_as("teammate", reducer, args);
    let claim = teammate("claim_dispatch_request", &[&requested, "host-a"]);
    assert!(!claim.status.success(), "{}", describe(&claim));
    let dispatched = teammate(
        "report_dispatch_provisioned",
        &[&provisioning, "host-a", "/wt/2-t", "task-2", "false"],
    );
    assert!(!dispatched.status.success(), "{}", describe(&dispatched));
    let failed = teammate("report_dispatch_failed", &[&provisioning, "host-a", "x"]);
    assert!(!failed.status.success(), "{}", describe(&failed));
    let swept = teammate("fail_interrupted_dispatch_requests", &["host-a"]);
    assert!(!swept.status.success(), "{}", describe(&swept));

    assert_eq!(request_column(&instance, "state", 1), "requested");
    assert_eq!(request_column(&instance, "state", 2), "provisioning");
}

/// ...and nobody may sweep or claim for a host that has no owned row — the
/// ownerless registration that would have made one is refused.
#[test]
fn the_sweep_and_a_claim_are_refused_for_an_ownerless_host() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_dispatchable(&instance, &[(1, "", "", "")]);
    request_dispatch(&instance, 1, "host-a", "user", "");
    let id = request_column(&instance, "id", 1);
    let ownerless = instance.call("register_host", &["fresh-host", "", ""]);
    assert!(!ownerless.status.success(), "{}", describe(&ownerless));

    let swept = instance.call("fail_interrupted_dispatch_requests", &["fresh-host"]);
    assert!(!swept.status.success(), "{}", describe(&swept));
    assert!(
        describe(&swept).contains("not registered"),
        "{}",
        describe(&swept)
    );
    let claim = instance.call("claim_dispatch_request", &[&id, "fresh-host"]);
    assert!(!claim.status.success(), "{}", describe(&claim));
    assert_eq!(request_column(&instance, "state", 1), "requested");
}

// -- LocalHostOwnerIsWrittenOnce, enforced by register_host ---------------------

/// A teammate can neither take over this host nor relabel it.
#[test]
fn a_teammate_cannot_overwrite_another_persons_host() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let me = instance.identity();
    register_host(&instance, "host-a", &me);
    instance.call_as("teammate", "register_host", &["scratch", "", ""]);
    let them = instance.identity_as("teammate");

    let takeover = instance.call_as("teammate", "register_host", &["host-a", "", &them]);
    assert!(!takeover.status.success(), "{}", describe(&takeover));
    let relabel = instance.call_as("teammate", "register_host", &["host-a", "mine now", &me]);
    assert!(!relabel.status.success(), "{}", describe(&relabel));

    assert_eq!(
        column(&instance, "SELECT owner FROM hosts WHERE id = 'host-a'"),
        me
    );
}

/// Nobody may name another identity as a host's owner, new host or not.
#[test]
fn a_host_cannot_be_registered_to_someone_elses_identity() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let them = register_teammates_host(&instance, "their-host");

    let gifted = instance.call("register_host", &["new-host", "", &them]);
    assert!(!gifted.status.success(), "{}", describe(&gifted));
    assert!(no_rows(
        &instance,
        "SELECT id FROM hosts WHERE id = 'new-host'"
    ));
}

/// The owner's own re-register — a rename — still works, and an owned host's
/// owner cannot be cleared.
#[test]
fn the_owner_may_relabel_but_not_disown_its_host() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let me = instance.identity();
    register_host(&instance, "host-a", &me);

    let renamed = instance.call("register_host", &["host-a", "laptop", &me]);
    assert!(renamed.status.success(), "{}", describe(&renamed));
    let disowned = instance.call("register_host", &["host-a", "laptop", ""]);
    assert!(!disowned.status.success(), "{}", describe(&disowned));

    assert_eq!(
        column(&instance, "SELECT label FROM hosts WHERE id = 'host-a'"),
        "laptop"
    );
    assert_eq!(
        column(&instance, "SELECT owner FROM hosts WHERE id = 'host-a'"),
        me
    );
}
