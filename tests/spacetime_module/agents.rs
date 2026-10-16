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

fn current_agent_column(instance: &Instance, field: &str, task: i64) -> String {
    column(
        instance,
        &format!("SELECT {field} FROM agents WHERE task_id = {task} AND state = 'current'"),
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
/// row in the same transaction, and the provisioned report's window reaches
/// the agent (`CurrentAgentMirrorsTheWindow`).
#[test]
fn a_dispatch_starts_a_named_agent_that_mirrors_the_window() {
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
