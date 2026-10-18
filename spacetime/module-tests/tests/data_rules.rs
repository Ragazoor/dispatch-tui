//! Data rules the reducers enforce, run against the in-memory store. Twins of
//! the live-store cases in `tests/spacetime_module/data_rules.rs`.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

/// `mcp-task-tools.allium: UpdateTaskViaMcp`, enforced where the rows are.
#[test]
fn an_illegal_sub_status_is_refused_at_the_reducer_with_the_old_wording() {
    let store = Store::new();
    let epic = store.create_epic("alice", "epic", false);
    let id = store.create("alice", epic_task("task", epic, "backlog"));

    let bad = TaskPatch {
        sub_status: Some("active".into()),
        ..TaskPatch::default()
    };
    let why = store
        .call("alice", |ctx| patch_task(ctx, id, bad))
        .expect_err("active is not a backlog sub-status");
    assert!(
        why.contains("sub_status 'active' is not valid for status 'backlog'"),
        "{why}"
    );

    // Judged against the status the patch moves to, not the one the row has.
    let both = TaskPatch {
        sub_status: Some("active".into()),
        status: Some("running".into()),
        ..TaskPatch::default()
    };
    store
        .call("alice", |ctx| patch_task(ctx, id, both))
        .expect("running accepts active");
}

/// `epics.allium: UpdateEpicViaMcp` / `ReparentEpicFromTui`.
#[test]
fn a_parent_cycle_is_refused_at_the_reducer() {
    let store = Store::new();
    let a = store.create_epic("alice", "a", false);
    let b = store
        .call("alice", |ctx| {
            create_epic(
                ctx,
                Epic {
                    title: "b".into(),
                    parent_epic_id: a,
                    ..blank_epic()
                },
            )
        })
        .map(|()| store.db().epics().iter().map(|e| e.id).max().expect("epic"))
        .expect("create_epic");

    let patch = EpicPatch {
        parent_epic_id: Some(b),
        ..EpicPatch::default()
    };
    let why = store
        .call("alice", |ctx| patch_epic(ctx, a, patch))
        .expect_err("a cycle is refused");
    assert!(
        why.contains("would create a cycle in the epic hierarchy"),
        "{why}"
    );
}

/// `epics.allium`'s routing note: the store places a task of a grouped epic.
#[test]
fn a_task_created_in_a_grouped_epic_lands_in_its_repo_sub_epic() {
    let store = Store::new();
    let root = store
        .call("alice", |ctx| {
            create_epic(
                ctx,
                Epic {
                    title: "root".into(),
                    group_by_repo: true,
                    ..blank_epic()
                },
            )
        })
        .map(|()| store.db().epics().iter().map(|e| e.id).max().expect("epic"))
        .expect("create_epic");
    let task = Task {
        repo_path: "/x/alpha".into(),
        ..epic_task("task", root, "backlog")
    };
    let id = store.create("alice", task);

    let sub = store
        .db()
        .epics()
        .iter()
        .find(|e| e.origin == "repo-group")
        .expect("a sub-epic for the task's repo");
    assert!(sub.title.contains("alpha"), "{}", sub.title);
    assert_eq!(sub.parent_epic_id, root);
    assert_eq!(
        store.task(id).epic_id,
        sub.id,
        "in the sub-epic, not the root"
    );
}
