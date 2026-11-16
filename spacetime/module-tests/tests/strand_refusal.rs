//! `workspaces.allium: SightChangeNeverStrandsAHost` (task #99226, L1): every
//! path that would take a task out of the sight of a person whose host runs its
//! agent is refused, as a detach is. Twin of `src/sync/memory_caller/tests/
//! strand_refusal.rs`; `tests/memory_caller_conformance/loss_of_sight.rs` holds
//! the two sides to the same answers.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

/// `root` is super-admin; `boss` administers A. `alice` and `bob` are members of
/// A; `alice` and `bob` also belong to B. Each has a host `h-<name>`.
/// Returns (store, A, B).
fn world() -> (Store, i64, i64) {
    let store = Store::bare();
    for who in ["root", "alice", "bob", "boss"] {
        store
            .call(who, |ctx| {
                stamp_rules_version_on_connect(ctx);
                Ok(())
            })
            .expect("connect");
    }
    store
        .call("root", |ctx| claim_super_admin(ctx))
        .expect("claim");
    let mut ids = Vec::new();
    for name in ["A", "B"] {
        store
            .call("root", |ctx| create_workspace(ctx, name.into()))
            .expect("workspace");
        ids.push(
            store
                .db()
                .workspaces()
                .iter()
                .find(|w| w.name == name)
                .expect("ws")
                .id,
        );
    }
    store
        .call("root", |ctx| {
            add_member(ctx, ids[0], identity_of("boss"), "admin".into())
        })
        .expect("admin");
    for (ws, who) in [
        (ids[0], "alice"),
        (ids[0], "bob"),
        (ids[1], "alice"),
        (ids[1], "boss"),
    ] {
        store
            .call("root", |ctx| {
                add_member(ctx, ws, identity_of(who), "member".into())
            })
            .expect("member");
    }
    for who in ["alice", "bob"] {
        store.register_host_for(&format!("h-{who}"), who);
    }
    (store, ids[0], ids[1])
}

fn make_epic(store: &Store, who: &str, row: Epic) -> i64 {
    store
        .call(who, |ctx| create_epic(ctx, row))
        .expect("create_epic");
    store.db().epics().iter().map(|e| e.id).max().expect("epic")
}

/// A current agent of `task` on `host`.
fn run_agent(store: &Store, task: i64, host: &str) {
    store.db().agents().insert(Agent {
        id: 0,
        task_id: task,
        name: "agent".into(),
        host: host.into(),
        start_mode: "dispatch".into(),
        state: "current".into(),
        started_at: String::new(),
        ended_at: String::new(),
        end_reason: String::new(),
        tmux_window: String::new(),
        last_pre_tool_use_at: String::new(),
        last_notification_at: String::new(),
        live_subagents: 0,
        stop_pending: false,
        stop_pending_at: String::new(),
        background_shells: 0,
        background_workflows: 0,
        background_monitors: 0,
        background_mcp_tasks: 0,
    });
}

/// A public epic of alice's in `a` holding one task, with bob's agent on it.
fn bobs_agent_in_alices_epic(store: &Store, a: i64) -> (i64, i64) {
    let epic = make_epic(
        store,
        "alice",
        Epic {
            workspace_id: a,
            ..blank_epic()
        },
    );
    let task = store.create("alice", epic_task("t", epic, "backlog"));
    run_agent(store, task, "h-bob");
    (epic, task)
}

fn assert_strand_refusal(why: &str) {
    assert!(why.contains("host is running"), "{why}");
}

#[test]
fn making_an_epic_private_is_refused_while_another_persons_host_runs_a_task_in_it() {
    let (store, a, _) = world();
    let (epic, _) = bobs_agent_in_alices_epic(&store, a);
    let why = store
        .call("alice", |ctx| set_epic_visibility(ctx, epic, "private".into()))
        .unwrap_err();
    assert_strand_refusal(&why);
    assert_eq!(store.epic(epic).visibility, "public", "a refusal writes nothing");
}

#[test]
fn making_an_epic_private_is_allowed_when_only_the_holders_host_runs_in_it() {
    let (store, a, _) = world();
    let epic = make_epic(
        &store,
        "alice",
        Epic {
            workspace_id: a,
            ..blank_epic()
        },
    );
    let task = store.create("alice", epic_task("t", epic, "backlog"));
    run_agent(&store, task, "h-alice");
    store
        .call("alice", |ctx| set_epic_visibility(ctx, epic, "private".into()))
        .expect("the holder's own agent is not stranded");
}

#[test]
fn a_task_move_into_a_private_epic_is_refused_while_another_persons_host_runs_it() {
    let (store, a, _) = world();
    let (epic, task) = bobs_agent_in_alices_epic(&store, a);
    let private = make_epic(
        &store,
        "boss",
        Epic {
            workspace_id: a,
            visibility: "private".into(),
            ..blank_epic()
        },
    );
    let why = store
        .call("boss", |ctx| set_task_epic(ctx, task, private, String::new()))
        .unwrap_err();
    assert_strand_refusal(&why);
    assert_eq!(store.task(task).epic_id, epic);
}

#[test]
fn a_reparent_under_a_private_epic_is_refused_while_another_persons_host_runs_in_it() {
    let (store, a, _) = world();
    let (epic, _) = bobs_agent_in_alices_epic(&store, a);
    let private = make_epic(
        &store,
        "alice",
        Epic {
            workspace_id: a,
            visibility: "private".into(),
            ..blank_epic()
        },
    );
    let why = store
        .call("alice", |ctx| {
            patch_epic(
                ctx,
                epic,
                EpicPatch {
                    parent_epic_id: Some(private),
                    ..EpicPatch::default()
                },
            )
        })
        .unwrap_err();
    assert_strand_refusal(&why);
    assert_eq!(store.epic(epic).parent_epic_id, 0);
}

#[test]
fn an_epic_workspace_move_is_refused_while_a_host_of_a_non_member_runs_in_it() {
    let (store, a, b) = world();
    // bob is not a member of B.
    let (epic, _) = bobs_agent_in_alices_epic(&store, a);
    let why = store
        .call("root", |ctx| move_epic_to_workspace(ctx, epic, b))
        .unwrap_err();
    assert_strand_refusal(&why);
    assert_eq!(store.epic(epic).workspace_id, a);
}

#[test]
fn removing_a_member_is_refused_while_their_host_runs_a_task_of_the_workspace() {
    let (store, a, _) = world();
    bobs_agent_in_alices_epic(&store, a);
    let membership = store
        .db()
        .memberships()
        .iter()
        .find(|m| m.workspace_id == a && m.user == identity_of("bob"))
        .expect("membership")
        .id;
    let why = store
        .call("root", |ctx| remove_member(ctx, membership))
        .unwrap_err();
    assert_strand_refusal(&why);
    assert!(store
        .db()
        .memberships()
        .iter()
        .any(|m| m.id == membership));
}

#[test]
fn an_ended_agent_strands_nobody() {
    let (store, a, _) = world();
    let (epic, task) = bobs_agent_in_alices_epic(&store, a);
    for agent in store.db().agents().iter().filter(|a| a.task_id == task) {
        store.db().agents().id().update(Agent {
            state: "ended".into(),
            ..agent
        });
    }
    store
        .call("alice", |ctx| set_epic_visibility(ctx, epic, "private".into()))
        .expect("nothing runs, so nothing is stranded");
}

/// V5: the super-admin's delete of an unseen item checks the subtree for a
/// private epic someone still sees BEFORE it judges the tasks, so its refusal
/// names no task inside a private sub-epic.
#[test]
fn the_unseen_delete_names_no_task_inside_a_private_sub_epic() {
    let (store, a, _) = world();
    // boss's root epic is private and unseen (boss leaves A); it holds a
    // private sub-epic of bob's, who is still a member.
    let root = make_epic(
        &store,
        "boss",
        Epic {
            workspace_id: a,
            ..blank_epic()
        },
    );
    let inner = make_epic(
        &store,
        "bob",
        Epic {
            workspace_id: a,
            parent_epic_id: root,
            visibility: "private".into(),
            ..blank_epic()
        },
    );
    let secret = store.create("bob", epic_task("bobs-secret", inner, "backlog"));
    store
        .call("boss", |ctx| set_epic_visibility(ctx, root, "private".into()))
        .expect("boss makes the root private");
    let membership = store
        .db()
        .memberships()
        .iter()
        .find(|m| m.workspace_id == a && m.user == identity_of("boss"))
        .expect("membership")
        .id;
    store
        .call("root", |ctx| remove_member(ctx, membership))
        .expect("boss leaves A");
    let why = store
        .call("root", |ctx| {
            super_admin_delete_unseen_item(ctx, "epic".into(), root)
        })
        .unwrap_err();
    assert!(
        !why.contains(&secret.to_string()),
        "the refusal leaked the private task id: {why}"
    );
    assert!(why.contains("someone still sees"), "{why}");
}
