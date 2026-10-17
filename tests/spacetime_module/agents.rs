use super::*;

// -- agents A1 (task #90115): agent rows and the name ledger ------------------
//
// `agents.allium`. Each test drives the reducers a board calls and reads the
// agent rows back over SQL. The rules' decisions are pinned without a store in
// the module's own `tests/agents.rs`; here it is the transaction: that the
// agent lands with the claim, follows the task's writes and goes with it.

fn agent_count(instance: &Instance, filter: &str) -> String {
    column(
        instance,
        &format!("SELECT COUNT(*) AS n FROM agents WHERE {filter}"),
    )
}

fn ledger_count(instance: &Instance) -> String {
    column(instance, "SELECT COUNT(*) AS n FROM handed_out_agent_names")
}

/// Task 1 in backlog, epic-less and owned, with both hosts registered.
fn seed_backlog_task(instance: &Instance) {
    register_own_hosts(instance);
    let mut row = task_json(1, "t", "backlog", 0, "");
    row["owner"] = serde_json::json!("user-me");
    let seeded = instance.call("seed_tasks", &[&serde_json::json!([row]).to_string()]);
    assert!(seeded.status.success(), "{}", describe(&seeded));
}

/// Request, claim and report task 1 provisioned at `/wt/1-t` in `task-1`.
fn dispatch_and_provision(instance: &Instance) {
    let requested = request_dispatch(instance, 1, "host-a", "user", "research");
    assert!(requested.status.success(), "{}", describe(&requested));
    let id = column(
        instance,
        "SELECT id FROM dispatch_requests WHERE task_id = 1 AND state = 'requested'",
    );
    let claimed = instance.call("claim_dispatch_request", &[&id, "host-a"]);
    assert!(claimed.status.success(), "{}", describe(&claimed));
    let reported = instance.call(
        "report_dispatch_provisioned",
        &[&id, "host-a", "/wt/1-t", "task-1", "false"],
    );
    assert!(reported.status.success(), "{}", describe(&reported));
}

/// `StartAgentOnFreshLaunch`: the claim creates a current agent and its ledger
/// row in the same transaction, and the provisioned report's window is written
/// on the agent, never on the Task row (agents A4).
#[test]
fn a_dispatch_starts_a_named_agent_that_holds_the_window() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_backlog_task(&instance);

    let requested = request_dispatch(&instance, 1, "host-a", "user", "research");
    assert!(requested.status.success(), "{}", describe(&requested));
    assert_eq!(
        agent_count(&instance, "task_id = 1 AND state = 'current'"),
        "1"
    );
    assert_eq!(current_agent_column(&instance, "host", 1), "host-a");
    assert_eq!(current_agent_column(&instance, "start_mode", 1), "research");
    assert_eq!(current_agent_column(&instance, "tmux_window", 1), "");
    let name = current_agent_column(&instance, "name", 1);
    assert!(
        dispatch_spacetime_module::AGENT_NAMES.contains(&name.as_str()),
        "{name}"
    );
    assert_eq!(
        column(
            &instance,
            &format!("SELECT COUNT(*) AS n FROM handed_out_agent_names WHERE name = '{name}'")
        ),
        "1"
    );

    let id = request_column(&instance, "id", 1);
    instance.call("claim_dispatch_request", &[&id, "host-a"]);
    let reported = instance.call(
        "report_dispatch_provisioned",
        &[&id, "host-a", "/wt/1-t", "task-1", "false"],
    );
    assert!(reported.status.success(), "{}", describe(&reported));
    assert_eq!(current_agent_column(&instance, "tmux_window", 1), "task-1");

    // A pause and a resume are window writes on the task; the agent stays.
    set_task_field(&instance, 1, "tmux_window", serde_json::json!(""));
    assert_eq!(current_agent_column(&instance, "tmux_window", 1), "");
    set_task_field(&instance, 1, "tmux_window", serde_json::json!("task-1"));
    assert_eq!(current_agent_column(&instance, "tmux_window", 1), "task-1");
    assert_eq!(current_agent_column(&instance, "name", 1), name);
    assert_eq!(agent_count(&instance, "task_id = 1"), "1");
}

/// A refused claim starts no agent and hands out no name.
#[test]
fn a_refused_dispatch_starts_no_agent() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_backlog_task(&instance);
    move_task(&instance, 1, "review", "awaiting_review");

    let refused = request_dispatch(&instance, 1, "host-a", "user", "");
    assert!(!refused.status.success(), "{}", describe(&refused));
    assert_eq!(agent_count(&instance, "task_id = 1"), "0");
    assert_eq!(ledger_count(&instance), "0");
}

/// `EndAgentWhenLaunchFails`: a failed request that releases the bare claim
/// ends the agent as launch_failed.
#[test]
fn a_failed_launch_ends_the_agent() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_backlog_task(&instance);
    request_dispatch(&instance, 1, "host-a", "user", "");
    let id = request_column(&instance, "id", 1);
    instance.call("claim_dispatch_request", &[&id, "host-a"]);

    let failed = instance.call("report_dispatch_failed", &[&id, "host-a", "boom"]);
    assert!(failed.status.success(), "{}", describe(&failed));
    assert_eq!(task_column(&instance, "status", 1), "backlog");
    assert_eq!(
        agent_count(&instance, "task_id = 1 AND state = 'current'"),
        "0"
    );
    assert_eq!(
        column(&instance, "SELECT end_reason FROM agents WHERE task_id = 1"),
        "launch_failed"
    );
}

/// `EndAgentWhenWorktreeGoes`, then the relaunch: RetryFresh's pointer clear
/// ends the agent, and the next dispatch starts a new one under a new name.
#[test]
fn clearing_the_worktree_ends_the_agent_and_the_relaunch_starts_another() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_backlog_task(&instance);
    dispatch_and_provision(&instance);
    let first = current_agent_column(&instance, "name", 1);

    // RetryFresh: the pointer clear, then back to backlog.
    let mut patch = empty_task_patch();
    patch["worktree"] = serde_json::json!({"some": ""});
    patch["tmux_window"] = serde_json::json!({"some": ""});
    patch["host"] = serde_json::json!({"some": ""});
    let cleared = instance.call("patch_task", &["1", &patch.to_string()]);
    assert!(cleared.status.success(), "{}", describe(&cleared));
    assert_eq!(
        agent_count(&instance, "task_id = 1 AND state = 'current'"),
        "0"
    );
    assert_eq!(
        column(&instance, "SELECT end_reason FROM agents WHERE task_id = 1"),
        "worktree_gone"
    );
    move_task(&instance, 1, "backlog", "none");

    dispatch_and_provision(&instance);
    assert_eq!(agent_count(&instance, "task_id = 1"), "2");
    assert_eq!(
        agent_count(&instance, "task_id = 1 AND state = 'current'"),
        "1"
    );
    assert_ne!(current_agent_column(&instance, "name", 1), first);
}

/// A dispatch over a worktree MoveTaskBackward kept replaces the agent.
#[test]
fn a_dispatch_over_a_kept_worktree_replaces_the_agent() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_backlog_task(&instance);
    dispatch_and_provision(&instance);
    let first = current_agent_column(&instance, "name", 1);
    move_task(&instance, 1, "backlog", "none");

    let requested = request_dispatch(&instance, 1, "host-a", "user", "");
    assert!(requested.status.success(), "{}", describe(&requested));
    assert_eq!(
        column(
            &instance,
            &format!("SELECT end_reason FROM agents WHERE name = '{first}'")
        ),
        "replaced"
    );
    assert_eq!(
        agent_count(&instance, "task_id = 1 AND state = 'current'"),
        "1"
    );
}

/// `AgentsGoWithTheirTask`: a delete removes every agent the task had, and
/// their names stay in the ledger.
#[test]
fn deleting_a_task_removes_its_agents_and_keeps_their_names() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_backlog_task(&instance);
    dispatch_and_provision(&instance);
    move_task(&instance, 1, "done", "none");

    let deleted = instance.call("delete_task", &["1"]);
    assert!(deleted.status.success(), "{}", describe(&deleted));
    assert_eq!(agent_count(&instance, "task_id = 1"), "0");
    assert_eq!(ledger_count(&instance), "1");
}

/// `BackfillCurrentAgents`: every task with a worktree and no current agent
/// gets one, once.
#[test]
fn the_backfill_names_every_task_with_a_worktree() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    register_own_hosts(&instance);
    let mut with_worktree = task_json(1, "t", "done", 0, "host-a");
    with_worktree["owner"] = serde_json::json!("user-me");
    with_worktree["worktree"] = serde_json::json!("/wt/1-t");
    let mut without = task_json(2, "t", "backlog", 0, "");
    without["owner"] = serde_json::json!("user-me");
    let seeded = instance.call(
        "seed_tasks",
        &[&serde_json::json!([with_worktree, without]).to_string()],
    );
    assert!(seeded.status.success(), "{}", describe(&seeded));

    for _ in 0..2 {
        let backfilled = instance.call("backfill_agents", &[]);
        assert!(backfilled.status.success(), "{}", describe(&backfilled));
    }
    assert_eq!(
        agent_count(&instance, "task_id = 1 AND state = 'current'"),
        "1"
    );
    assert_eq!(current_agent_column(&instance, "host", 1), "host-a");
    assert_eq!(current_agent_column(&instance, "start_mode", 1), "dispatch");
    assert_eq!(agent_count(&instance, "task_id = 2"), "0");
    assert_eq!(ledger_count(&instance), "1");
}

/// A seeded agent's name enters the ledger, so the picker never hands it out
/// again (`AgentNameIsNeverReused`).
#[test]
fn a_seeded_agent_takes_its_name_out_of_the_pool() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let agent = serde_json::json!([{
        "id": 5, "task_id": 9, "name": "ada", "host": "host-a",
        "start_mode": "dispatch", "state": "ended",
        "started_at": "2026-10-16 10:00:00.000", "ended_at": "2026-10-16 11:00:00.000",
        "end_reason": "replaced", "tmux_window": "",
        "last_pre_tool_use_at": "", "last_notification_at": "", "live_subagents": 0,
        "stop_pending": false, "stop_pending_at": "",
    }]);
    let seeded = instance.call("seed_agents", &[&agent.to_string()]);
    assert!(seeded.status.success(), "{}", describe(&seeded));
    assert_eq!(
        column(
            &instance,
            "SELECT COUNT(*) AS n FROM handed_out_agent_names WHERE name = 'ada'"
        ),
        "1"
    );
}

// -- agents A4 (task #90118): the live-session facts move onto the agent -----

/// The six session facts live on the current agent; the Task row's own columns
/// are never written, whatever a patch or a hook names.
#[test]
fn the_session_facts_are_the_agents_and_the_task_columns_stay_blank() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_backlog_task(&instance);
    dispatch_and_provision(&instance);
    move_task(&instance, 1, "running", "active");

    // The claim stamped the new agent's activity, and the window is its own.
    assert_ne!(
        current_agent_column(&instance, "last_pre_tool_use_at", 1),
        ""
    );
    assert_eq!(current_agent_column(&instance, "tmux_window", 1), "task-1");
    assert_eq!(task_column(&instance, "tmux_window", 1), "");

    // A patch naming the facts lands on the agent.
    set_task_field(&instance, 1, "live_subagents", serde_json::json!(0));
    set_task_field(&instance, 1, "stop_pending", serde_json::json!(true));
    assert_eq!(current_agent_column(&instance, "stop_pending", 1), "true");

    // Hooks write the agent, not the task.
    let started = instance.call("subagent_start", &["1", "a1", "s1", "2026-10-16T10:00:00Z"]);
    assert!(started.status.success(), "{}", describe(&started));
    assert_eq!(current_agent_column(&instance, "live_subagents", 1), "1");
    let tool = instance.call(
        "record_pre_tool_use",
        &["1", "active", "2026-10-16 10:00:05"],
    );
    assert!(tool.status.success(), "{}", describe(&tool));
    assert_eq!(
        current_agent_column(&instance, "last_pre_tool_use_at", 1),
        "2026-10-16 10:00:05"
    );
    for column_name in [
        "live_subagents",
        "stop_pending",
        "last_pre_tool_use_at",
        "last_notification_at",
        "tmux_window",
    ] {
        let blank = if column_name == "live_subagents" {
            "0"
        } else if column_name == "stop_pending" {
            "false"
        } else {
            ""
        };
        assert_eq!(
            task_column(&instance, column_name, 1),
            blank,
            "{column_name}"
        );
    }
    // The entry belongs to the agent that was current when the hook fired.
    assert_eq!(
        column(
            &instance,
            "SELECT agent_row FROM task_subagents WHERE task_id = 1"
        ),
        column(&instance, "SELECT id FROM agents WHERE task_id = 1")
    );
}

/// An agent that ends takes its subagent entries and its pending Stop with it
/// (`EndedAgentHoldsNothing`), and a fresh launch starts clean.
#[test]
fn a_replaced_agent_holds_nothing_and_the_new_one_starts_clean() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_backlog_task(&instance);
    dispatch_and_provision(&instance);
    move_task(&instance, 1, "running", "active");
    let started = instance.call("subagent_start", &["1", "a1", "s1", "2026-10-16T10:00:00Z"]);
    assert!(started.status.success(), "{}", describe(&started));
    set_task_field(&instance, 1, "stop_pending", serde_json::json!(true));

    // The task goes back to backlog with its worktree, then is dispatched afresh.
    move_task(&instance, 1, "backlog", "none");
    let again = request_dispatch(&instance, 1, "host-a", "user", "");
    assert!(again.status.success(), "{}", describe(&again));

    assert_eq!(
        column(
            &instance,
            "SELECT live_subagents FROM agents WHERE task_id = 1 AND state = 'ended'"
        ),
        "0"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT stop_pending FROM agents WHERE task_id = 1 AND state = 'ended'"
        ),
        "false"
    );
    assert_eq!(
        column(
            &instance,
            "SELECT COUNT(*) AS n FROM task_subagents WHERE task_id = 1"
        ),
        "0"
    );
    assert_eq!(current_agent_column(&instance, "live_subagents", 1), "0");
    assert_eq!(current_agent_column(&instance, "stop_pending", 1), "false");
    assert_eq!(current_agent_column(&instance, "tmux_window", 1), "");
}

/// A hook for a task with no current agent writes no agent fact and starts no
/// agent (decision A15); a patch that SETS a fact is refused, one that clears
/// is a no-op.
#[test]
fn a_session_fact_for_a_task_with_no_agent_is_refused_or_ignored() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    seed_backlog_task(&instance);

    let started = instance.call("subagent_start", &["1", "a1", "s1", "2026-10-16T10:00:00Z"]);
    assert!(started.status.success(), "{}", describe(&started));
    assert_eq!(
        column(&instance, "SELECT COUNT(*) AS n FROM task_subagents"),
        "0"
    );
    assert_eq!(agent_count(&instance, "task_id = 1"), "0");

    let mut set = empty_task_patch();
    set["tmux_window"] = serde_json::json!({"some": "task-1"});
    let refused = instance.call("patch_task", &["1", &set.to_string()]);
    assert!(!refused.status.success(), "{}", describe(&refused));
    set_task_field(&instance, 1, "tmux_window", serde_json::json!(""));
}

/// `BackfillCurrentAgents`, A4: a store published before A4 holds the facts on
/// its Task rows. The backfill moves them onto the current agent, points the
/// task's subagent entries at it, and blanks the Task. A second run changes
/// nothing, and the agent's own newer fact is not overwritten.
#[test]
fn the_backfill_moves_a_pre_a4_tasks_session_facts_onto_its_agent() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    register_own_hosts(&instance);
    let mut row = task_json(1, "t", "running", 0, "host-a");
    row["owner"] = serde_json::json!("user-me");
    row["worktree"] = serde_json::json!("/wt/1-t");
    row["tmux_window"] = serde_json::json!("task-1");
    row["last_pre_tool_use_at"] = serde_json::json!("2026-10-16 11:00:00");
    row["last_notification_at"] = serde_json::json!("2026-10-16 11:30:00");
    row["live_subagents"] = serde_json::json!(2);
    row["stop_pending"] = serde_json::json!(true);
    row["stop_pending_at"] = serde_json::json!("2026-10-16 11:45:00.000");
    let seeded = instance.call("seed_tasks", &[&serde_json::json!([row]).to_string()]);
    assert!(seeded.status.success(), "{}", describe(&seeded));
    let entries = serde_json::json!([
        {"task_id": 1, "agent_id": "a1", "session_id": "s1", "started_at": "t", "agent_row": 0},
        {"task_id": 1, "agent_id": "a2", "session_id": "s1", "started_at": "t", "agent_row": 0},
    ]);
    let seeded = instance.call("seed_task_subagents", &[&entries.to_string()]);
    assert!(seeded.status.success(), "{}", describe(&seeded));

    backfill_agents(&instance);

    assert_eq!(current_agent_column(&instance, "tmux_window", 1), "task-1");
    assert_eq!(
        current_agent_column(&instance, "last_pre_tool_use_at", 1),
        "2026-10-16 11:00:00"
    );
    assert_eq!(
        current_agent_column(&instance, "last_notification_at", 1),
        "2026-10-16 11:30:00"
    );
    assert_eq!(current_agent_column(&instance, "live_subagents", 1), "2");
    assert_eq!(current_agent_column(&instance, "stop_pending", 1), "true");
    assert_eq!(
        current_agent_column(&instance, "stop_pending_at", 1),
        "2026-10-16 11:45:00.000"
    );
    assert_eq!(task_column(&instance, "tmux_window", 1), "");
    assert_eq!(task_column(&instance, "live_subagents", 1), "0");
    assert_eq!(
        column(
            &instance,
            "SELECT COUNT(*) AS n FROM task_subagents WHERE agent_row = 0"
        ),
        "0"
    );

    // A newer agent fact survives a second run: the Task is blank now.
    let tool = instance.call(
        "record_pre_tool_use",
        &["1", "active", "2026-10-16 12:00:00"],
    );
    assert!(tool.status.success(), "{}", describe(&tool));
    backfill_agents(&instance);
    assert_eq!(
        current_agent_column(&instance, "last_pre_tool_use_at", 1),
        "2026-10-16 12:00:00"
    );
}
