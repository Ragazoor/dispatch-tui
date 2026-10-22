//! The new-task signal (`core.allium: UnseenTask`), run against the in-memory
//! store.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

fn unseen_for(store: &Store, subscriber: &str) -> Vec<i64> {
    let mut ids: Vec<i64> = store
        .db()
        .unseen_tasks()
        .iter()
        .filter(|u| u.subscriber == subscriber)
        .map(|u| u.task_id)
        .collect();
    ids.sort_unstable();
    ids
}

fn create_signalled(store: &Store, row: Task) -> i64 {
    let before = store.db().tasks().iter().map(|t| t.id).max().unwrap_or(0);
    store
        .call("alice", |ctx| create_task_signalled(ctx, row))
        .expect("create_task_signalled");
    let id = store.db().tasks().iter().map(|t| t.id).max().unwrap();
    assert!(id > before, "create_task_signalled wrote no row");
    id
}

fn follow(store: &Store, subscriber: &str, epic: i64) {
    store
        .call("alice", |ctx| {
            subscribe_to_epic(ctx, subscriber.into(), epic)
        })
        .expect("subscribe_to_epic");
}

/// Every follower of the epic OR of an ancestor is marked, and nobody else.
#[test]
fn a_signalled_task_is_unseen_for_followers_of_its_epic_and_ancestors() {
    let store = Store::new();
    let root = store.create_epic("alice", "root", false);
    let child = store.create_epic("alice", "child", false);
    store
        .call("alice", |ctx| {
            patch_epic(
                ctx,
                child,
                EpicPatch {
                    parent_epic_id: Some(root),
                    ..EpicPatch::default()
                },
            )
        })
        .expect("reparent");
    follow(&store, "of-root", root);
    follow(&store, "of-child", child);
    follow(&store, "elsewhere", store.create_epic("alice", "other", false));

    let task = create_signalled(&store, epic_task("new", child, "backlog"));

    assert_eq!(unseen_for(&store, "of-root"), vec![task]);
    assert_eq!(unseen_for(&store, "of-child"), vec![task]);
    assert!(unseen_for(&store, "elsewhere").is_empty());
}

/// A plain `create_task` never writes a mark.
#[test]
fn an_ordinary_create_marks_nothing() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    follow(&store, "bob", epic);

    store.create("alice", epic_task("plain", epic, "backlog"));

    assert!(store.db().unseen_tasks().iter().next().is_none());
}

/// Someone who follows after the task exists is not marked for it.
#[test]
fn a_later_follower_is_not_marked() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    create_signalled(&store, epic_task("new", epic, "backlog"));

    follow(&store, "late", epic);

    assert!(unseen_for(&store, "late").is_empty());
}

/// Entering the epic clears the entering person's marks on tasks directly in
/// it, leaves a deeper task's mark, and leaves other people's marks.
#[test]
fn entering_an_epic_clears_only_its_own_tasks_for_that_person() {
    let store = Store::new();
    let root = store.create_epic("alice", "root", false);
    let child = store.create_epic("alice", "child", false);
    store
        .call("alice", |ctx| {
            patch_epic(
                ctx,
                child,
                EpicPatch {
                    parent_epic_id: Some(root),
                    ..EpicPatch::default()
                },
            )
        })
        .expect("reparent");
    follow(&store, "bob", root);
    follow(&store, "carol", root);
    let shallow = create_signalled(&store, epic_task("shallow", root, "backlog"));
    let deep = create_signalled(&store, epic_task("deep", child, "backlog"));

    store
        .call("bob", |ctx| mark_epic_seen(ctx, "bob".into(), root))
        .expect("mark_epic_seen");

    assert_eq!(unseen_for(&store, "bob"), vec![deep]);
    assert_eq!(unseen_for(&store, "carol"), vec![shallow, deep]);
}

/// Marking seen needs an identity, and an epic with nothing unseen is routine.
#[test]
fn mark_epic_seen_refuses_a_blank_identity_and_tolerates_nothing_to_clear() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);

    assert!(store
        .call("alice", |ctx| mark_epic_seen(ctx, "  ".into(), epic))
        .is_err());
    store
        .call("alice", |ctx| mark_epic_seen(ctx, "bob".into(), epic))
        .expect("nothing to clear is not an error");
}

/// Moving the task to another epic ends the mark, and it does not follow.
#[test]
fn moving_a_task_out_of_its_epic_clears_the_mark() {
    let store = Store::new();
    let from = store.create_epic("alice", "from", false);
    let to = store.create_epic("alice", "to", false);
    follow(&store, "bob", from);
    follow(&store, "bob", to);
    let task = create_signalled(&store, epic_task("new", from, "backlog"));

    store
        .call("alice", |ctx| set_task_epic(ctx, task, to, String::new()))
        .expect("set_task_epic");

    assert!(unseen_for(&store, "bob").is_empty());
}

/// Reaching Done ends the mark.
#[test]
fn finishing_a_task_clears_the_mark() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    follow(&store, "bob", epic);
    let task = create_signalled(&store, epic_task("new", epic, "running"));

    store
        .call("alice", |ctx| patch_task(ctx, task, status_patch("done")))
        .expect("patch_task");

    assert!(unseen_for(&store, "bob").is_empty());
}

/// A mark cannot outlive its task.
#[test]
fn deleting_a_task_removes_its_marks() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    follow(&store, "bob", epic);
    let task = create_signalled(&store, epic_task("new", epic, "running"));
    // Done clears the mark first, so re-mark by hand to prove the delete path
    // removes rows on its own.
    store.db().unseen_tasks().insert(UnseenTask {
        id: unseen_id("bob", task),
        task_id: task,
        subscriber: "bob".into(),
    });
    store.db().tasks().id().update(Task {
        status: "done".into(),
        ..store.task(task)
    });

    store
        .call("alice", |ctx| delete_task(ctx, task))
        .expect("delete_task");

    assert!(unseen_for(&store, "bob").is_empty());
}
