//! The repo-group origin is set by routing and never by a patch
//! (`epics.allium: RepoGroupOriginIsSetByRouting`), and a flatten ends the
//! unseen marks of the tasks it moves (`epics.allium: FlattenEpic`), run
//! against the in-memory store.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

/// A store where `alice` and `bob` have connected.
fn world() -> Store {
    let store = Store::bare();
    for who in ["alice", "bob"] {
        store
            .call(who, |ctx| {
                stamp_rules_version_on_connect(ctx);
                Ok(())
            })
            .expect("connect");
    }
    store
}

fn make_epic(store: &Store, who: &str) -> i64 {
    let row = Epic {
        title: "e".into(),
        ..blank_epic()
    };
    store
        .call(who, |ctx| create_epic(ctx, row))
        .expect("create_epic");
    store.db().epics().iter().map(|e| e.id).max().expect("epic")
}

/// A grouped root and the repo-group sub-epic one task created.
fn grouped(store: &Store) -> (i64, i64, i64) {
    let root = make_epic(store, "alice");
    store
        .call("alice", |ctx| {
            patch_epic(
                ctx,
                root,
                EpicPatch {
                    group_by_repo: Some(true),
                    ..EpicPatch::default()
                },
            )
        })
        .expect("group");
    let task = store.create(
        "alice",
        Task {
            repo_path: "/x/alpha".into(),
            ..epic_task("t", root, "backlog")
        },
    );
    let sub = store
        .db()
        .epics()
        .iter()
        .find(|e| e.origin == "repo-group")
        .expect("repo group")
        .id;
    (root, sub, task)
}

#[test]
fn a_repo_groups_origin_cannot_be_changed() {
    let store = world();
    let (_, sub, _) = grouped(&store);
    let r = store.call("alice", |ctx| {
        patch_epic(
            ctx,
            sub,
            EpicPatch {
                origin: Some(String::new()),
                ..EpicPatch::default()
            },
        )
    });
    assert!(r.is_err());
    assert_eq!(store.epic(sub).origin, "repo-group");
}

#[test]
fn a_hand_made_epic_cannot_be_given_the_repo_group_origin() {
    let store = world();
    let plain = make_epic(&store, "alice");
    let r = store.call("alice", |ctx| {
        patch_epic(
            ctx,
            plain,
            EpicPatch {
                origin: Some("repo-group".into()),
                ..EpicPatch::default()
            },
        )
    });
    assert!(r.is_err());
}

#[test]
fn flattening_a_grouped_epic_clears_the_unseen_marks_of_its_tasks() {
    let store = world();
    let (root, sub, _) = grouped(&store);
    let signalled = {
        let row = Task {
            repo_path: "/x/alpha".into(),
            ..epic_task("s", sub, "backlog")
        };
        store
            .call("alice", |ctx| create_task_signalled(ctx, row))
            .expect("signalled");
        store.db().tasks().iter().map(|t| t.id).max().expect("task")
    };
    assert!(
        store
            .db()
            .unseen_tasks()
            .iter()
            .any(|u| u.task_id == signalled),
        "the signal marks the task"
    );
    store
        .call("alice", |ctx| flatten_epic(ctx, root))
        .expect("flatten");
    assert_eq!(store.task(signalled).epic_id, root);
    assert!(
        !store
            .db()
            .unseen_tasks()
            .iter()
            .any(|u| u.task_id == signalled),
        "the task left its epic, so its mark ends"
    );
}
