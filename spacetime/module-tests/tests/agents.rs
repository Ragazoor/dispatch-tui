//! Agent rows, the name ledger and the live-session facts that live on the
//! current agent (`agents.allium`, agents A1 and A4), run against the in-memory
//! store. Each test drives the reducers a board calls and reads the rows back.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

const T0: &str = "2026-10-16T10:00:00Z";

fn agents_of(store: &Store, task: i64) -> Vec<Agent> {
    store
        .db()
        .agents()
        .iter()
        .filter(|a| a.task_id == task)
        .collect()
}

fn current_agent(store: &Store, task: i64) -> Agent {
    agents_of(store, task)
        .into_iter()
        .find(|a| a.state == "current")
        .expect("the task has a current agent")
}

fn current_count(store: &Store, task: i64) -> usize {
    agents_of(store, task)
        .iter()
        .filter(|a| a.state == "current")
        .count()
}

fn ledger_count(store: &Store) -> usize {
    store.db().handed_out_agent_names().iter().count()
}

fn subagent_rows(store: &Store, task: i64) -> Vec<TaskSubagent> {
    store
        .db()
        .task_subagents()
        .iter()
        .filter(|e| e.task_id == task)
        .collect()
}

/// A backlog task on alice's board, with both hosts hers.
fn backlog_task() -> (Store, i64) {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    store.register_host_for(HOST_B, "alice");
    let id = store.create("alice", user_task("t", "alice"));
    (store, id)
}

fn dispatch(store: &Store, task: i64, mode: &str) -> Result<(), String> {
    store.call("alice", |ctx| {
        request_dispatch(ctx, task, HOST_A.into(), "user".into(), mode.into())
    })
}

fn open_request(store: &Store, task: i64) -> i64 {
    store
        .db()
        .dispatch_requests()
        .iter()
        .filter(|r| r.task_id == task)
        .map(|r| r.id)
        .max()
        .expect("a dispatch request")
}

fn claim(store: &Store, request: i64) {
    store
        .call("alice", |ctx| {
            claim_dispatch_request(ctx, request, HOST_A.into())
        })
        .expect("claim");
}

fn report_provisioned(store: &Store, request: i64, task: i64) {
    store
        .call("alice", |ctx| {
            report_dispatch_provisioned(
                ctx,
                request,
                HOST_A.into(),
                format!("/wt/{task}-t"),
                format!("task-{task}"),
                false,
            )
        })
        .expect("report provisioned");
}

/// Request, claim and report `task` provisioned on `HOST_A`.
fn dispatch_and_provision(store: &Store, task: i64) {
    dispatch(store, task, "research").expect("request_dispatch");
    let request = open_request(store, task);
    claim(store, request);
    report_provisioned(store, request, task);
}

fn patch(store: &Store, task: i64, patch: TaskPatch) -> Result<(), String> {
    store.call("alice", |ctx| patch_task(ctx, task, patch))
}

fn move_task(store: &Store, task: i64, status: &str, sub_status: &str) {
    patch(
        store,
        task,
        TaskPatch {
            status: Some(status.into()),
            sub_status: Some(sub_status.into()),
            ..TaskPatch::default()
        },
    )
    .expect("patch_task");
}

fn set_window(store: &Store, task: i64, window: &str) {
    patch(
        store,
        task,
        TaskPatch {
            tmux_window: Some(window.into()),
            ..TaskPatch::default()
        },
    )
    .expect("patch_task");
}

fn set_stop_pending(store: &Store, task: i64, pending: bool) {
    patch(
        store,
        task,
        TaskPatch {
            stop_pending: Some(pending),
            ..TaskPatch::default()
        },
    )
    .expect("patch_task");
}

fn subagent_start_call(store: &Store, task: i64) {
    store
        .call("alice", |ctx| {
            subagent_start(ctx, task, "a1".into(), "s1".into(), T0.into())
        })
        .expect("subagent_start");
}

// -- agents A1: agent rows and the name ledger ---------------------------------

/// `StartAgentOnFreshLaunch`: the claim creates a current agent and its ledger
/// row in the same transaction, and the provisioned report's window is written
/// on the agent, never on the Task row (agents A4).
#[test]
fn a_dispatch_starts_a_named_agent_that_holds_the_window() {
    let (store, id) = backlog_task();

    dispatch(&store, id, "research").expect("request_dispatch");
    assert_eq!(current_count(&store, id), 1);
    let agent = current_agent(&store, id);
    assert_eq!(agent.host, HOST_A);
    assert_eq!(agent.start_mode, "research");
    assert_eq!(agent.tmux_window, "");
    let name = agent.name;
    assert!(AGENT_NAMES.contains(&name.as_str()), "{name}");
    assert_eq!(
        store
            .db()
            .handed_out_agent_names()
            .iter()
            .filter(|n| n.name == name)
            .count(),
        1
    );

    let request = open_request(&store, id);
    claim(&store, request);
    report_provisioned(&store, request, id);
    assert_eq!(current_agent(&store, id).tmux_window, format!("task-{id}"));

    // A pause and a resume are window writes on the task; the agent stays.
    set_window(&store, id, "");
    assert_eq!(current_agent(&store, id).tmux_window, "");
    set_window(&store, id, "task-x");
    assert_eq!(current_agent(&store, id).tmux_window, "task-x");
    assert_eq!(current_agent(&store, id).name, name);
    assert_eq!(agents_of(&store, id).len(), 1);
}

/// A refused claim starts no agent and hands out no name.
#[test]
fn a_refused_dispatch_starts_no_agent() {
    let (store, id) = backlog_task();
    move_task(&store, id, "review", "awaiting_review");

    assert!(dispatch(&store, id, "").is_err());
    assert!(agents_of(&store, id).is_empty());
    assert_eq!(ledger_count(&store), 0);
}

/// `EndAgentWhenLaunchFails`: a failed request that releases the bare claim
/// ends the agent as launch_failed.
#[test]
fn a_failed_launch_ends_the_agent() {
    let (store, id) = backlog_task();
    dispatch(&store, id, "").expect("request_dispatch");
    let request = open_request(&store, id);
    claim(&store, request);

    store
        .call("alice", |ctx| {
            report_dispatch_failed(ctx, request, HOST_A.into(), "boom".into())
        })
        .expect("report_dispatch_failed");

    assert_eq!(store.task(id).status, "backlog");
    assert_eq!(current_count(&store, id), 0);
    assert_eq!(agents_of(&store, id)[0].end_reason, "launch_failed");
}

/// `EndAgentWhenWorktreeGoes`, then the relaunch: RetryFresh's pointer clear
/// ends the agent, and the next dispatch starts a new one under a new name.
#[test]
fn clearing_the_worktree_ends_the_agent_and_the_relaunch_starts_another() {
    let (store, id) = backlog_task();
    dispatch_and_provision(&store, id);
    let first = current_agent(&store, id).name;

    // RetryFresh: the pointer clear, then back to backlog.
    patch(
        &store,
        id,
        TaskPatch {
            worktree: Some(String::new()),
            tmux_window: Some(String::new()),
            host: Some(String::new()),
            ..TaskPatch::default()
        },
    )
    .expect("pointer clear");
    assert_eq!(current_count(&store, id), 0);
    assert_eq!(agents_of(&store, id)[0].end_reason, "worktree_gone");
    move_task(&store, id, "backlog", "none");

    dispatch_and_provision(&store, id);
    assert_eq!(agents_of(&store, id).len(), 2);
    assert_eq!(current_count(&store, id), 1);
    assert_ne!(current_agent(&store, id).name, first);
}

/// A dispatch over a worktree MoveTaskBackward kept replaces the agent.
#[test]
fn a_dispatch_over_a_kept_worktree_replaces_the_agent() {
    let (store, id) = backlog_task();
    dispatch_and_provision(&store, id);
    let first = current_agent(&store, id).name;
    move_task(&store, id, "backlog", "none");

    dispatch(&store, id, "").expect("request_dispatch");
    let old = agents_of(&store, id)
        .into_iter()
        .find(|a| a.name == first)
        .expect("the first agent is kept");
    assert_eq!(old.end_reason, "replaced");
    assert_eq!(current_count(&store, id), 1);
}

/// `AgentsGoWithTheirTask`: a delete removes every agent the task had, and
/// their names stay in the ledger.
#[test]
fn deleting_a_task_removes_its_agents_and_keeps_their_names() {
    let (store, id) = backlog_task();
    dispatch_and_provision(&store, id);
    move_task(&store, id, "done", "none");

    store
        .call("alice", |ctx| delete_task(ctx, id))
        .expect("delete_task");
    assert!(agents_of(&store, id).is_empty());
    assert_eq!(ledger_count(&store), 1);
}

/// A task as a store published before agents held it: it has a worktree and
/// host on the row, and `facts` stamps the session facts it then carried.
fn legacy_task(id: i64, status: &str, worktree: &str) -> Task {
    Task {
        id,
        title: "t".into(),
        repo_path: "/repo".into(),
        status: status.into(),
        owner: identity_of("alice"),
        host: if worktree.is_empty() { "" } else { HOST_A }.into(),
        worktree: worktree.into(),
        ..blank_task()
    }
}

fn seed_tasks_call(store: &Store, rows: Vec<Task>) {
    store
        .call("alice", |ctx| seed_tasks(ctx, rows))
        .expect("seed_tasks");
}

fn backfill(store: &Store) {
    store
        .call("alice", |ctx| backfill_agents(ctx))
        .expect("backfill_agents");
}

/// `BackfillCurrentAgents`: every task with a worktree and no current agent
/// gets one, once.
#[test]
fn the_backfill_names_every_task_with_a_worktree() {
    let store = Store::new();
    store.register_host_for(HOST_A, "alice");
    seed_tasks_call(
        &store,
        vec![
            legacy_task(1, "done", "/wt/1-t"),
            legacy_task(2, "backlog", ""),
        ],
    );

    backfill(&store);
    backfill(&store);

    assert_eq!(current_count(&store, 1), 1);
    assert_eq!(current_agent(&store, 1).host, HOST_A);
    assert_eq!(current_agent(&store, 1).start_mode, "dispatch");
    assert!(agents_of(&store, 2).is_empty());
    assert_eq!(ledger_count(&store), 1);
}

/// A seeded agent's name enters the ledger, so the picker never hands it out
/// again (`AgentNameIsNeverReused`).
#[test]
fn a_seeded_agent_takes_its_name_out_of_the_pool() {
    let store = Store::new();
    let agent = Agent {
        id: 5,
        task_id: 9,
        name: "ada".into(),
        host: HOST_A.into(),
        start_mode: "dispatch".into(),
        state: "ended".into(),
        started_at: "2026-10-16 10:00:00.000".into(),
        ended_at: "2026-10-16 11:00:00.000".into(),
        end_reason: "replaced".into(),
        tmux_window: String::new(),
        last_pre_tool_use_at: String::new(),
        last_notification_at: String::new(),
        live_subagents: 0,
        stop_pending: false,
        stop_pending_at: String::new(),
    };
    store
        .call("alice", |ctx| seed_agents(ctx, vec![agent]))
        .expect("seed_agents");
    assert_eq!(
        store
            .db()
            .handed_out_agent_names()
            .iter()
            .filter(|n| n.name == "ada")
            .count(),
        1
    );
}

// -- agents A4: the live-session facts move onto the agent ---------------------

/// The six session facts live on the current agent; the Task row's own columns
/// are never written, whatever a patch or a hook names.
#[test]
fn the_session_facts_are_the_agents_and_the_task_columns_stay_blank() {
    let (store, id) = backlog_task();
    dispatch_and_provision(&store, id);
    move_task(&store, id, "running", "active");

    // The claim stamped the new agent's activity, and the window is its own.
    assert_ne!(current_agent(&store, id).last_pre_tool_use_at, "");
    assert_eq!(current_agent(&store, id).tmux_window, format!("task-{id}"));
    assert_eq!(store.task(id).tmux_window, "");

    // A patch naming the facts lands on the agent.
    patch(
        &store,
        id,
        TaskPatch {
            live_subagents: Some(0),
            stop_pending: Some(true),
            ..TaskPatch::default()
        },
    )
    .expect("patch_task");
    assert!(current_agent(&store, id).stop_pending);

    // Hooks write the agent, not the task.
    subagent_start_call(&store, id);
    assert_eq!(current_agent(&store, id).live_subagents, 1);
    store
        .call("alice", |ctx| {
            record_pre_tool_use(ctx, id, "active".into(), "2026-10-16 10:00:05".into())
        })
        .expect("record_pre_tool_use");
    assert_eq!(
        current_agent(&store, id).last_pre_tool_use_at,
        "2026-10-16 10:00:05"
    );
    let task = store.task(id);
    assert_eq!(task.live_subagents, 0);
    assert!(!task.stop_pending);
    assert_eq!(task.last_pre_tool_use_at, "");
    assert_eq!(task.last_notification_at, "");
    assert_eq!(task.tmux_window, "");
    // The entry belongs to the agent that was current when the hook fired.
    assert_eq!(
        subagent_rows(&store, id)[0].agent_row,
        current_agent(&store, id).id
    );
}

/// An agent that ends takes its subagent entries and its pending Stop with it
/// (`EndedAgentHoldsNothing`), and a fresh launch starts clean.
#[test]
fn a_replaced_agent_holds_nothing_and_the_new_one_starts_clean() {
    let (store, id) = backlog_task();
    dispatch_and_provision(&store, id);
    move_task(&store, id, "running", "active");
    subagent_start_call(&store, id);
    set_stop_pending(&store, id, true);

    // The task goes back to backlog with its worktree, then is dispatched afresh.
    move_task(&store, id, "backlog", "none");
    dispatch(&store, id, "").expect("request_dispatch");

    let ended = agents_of(&store, id)
        .into_iter()
        .find(|a| a.state == "ended")
        .expect("the old agent ended");
    assert_eq!(ended.live_subagents, 0);
    assert!(!ended.stop_pending);
    assert!(subagent_rows(&store, id).is_empty());
    let fresh = current_agent(&store, id);
    assert_eq!(fresh.live_subagents, 0);
    assert!(!fresh.stop_pending);
    assert_eq!(fresh.tmux_window, "");
}

/// A hook for a task with no current agent writes no agent fact and starts no
/// agent (decision A15); a patch that SETS a fact is refused, one that clears
/// is a no-op.
#[test]
fn a_session_fact_for_a_task_with_no_agent_is_refused_or_ignored() {
    let (store, id) = backlog_task();

    subagent_start_call(&store, id);
    assert_eq!(store.db().task_subagents().iter().count(), 0);
    assert!(agents_of(&store, id).is_empty());

    assert!(patch(
        &store,
        id,
        TaskPatch {
            tmux_window: Some("task-1".into()),
            ..TaskPatch::default()
        }
    )
    .is_err());
    set_window(&store, id, "");
}
