//! The new-task signal (`core.allium: UnseenTask`), run against the in-memory
//! store.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

fn unseen_for(store: &Store, who: &str) -> Vec<i64> {
    let subscriber = identity_of(who);
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

/// Everyone who can see the epic is marked, with no subscription anywhere
/// (`mcp-task-tools.allium: SignalMarksTheTaskForEveryoneWhoSeesIt`), and
/// nobody outside the workspace is.
#[test]
fn a_signalled_task_is_unseen_for_everyone_who_sees_its_epic() {
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
    store.enrol("bob");

    let task = store.create_signalled("alice", epic_task("new", child, "backlog"));

    assert!(store.db().subscriptions().iter().next().is_none());
    assert_eq!(unseen_for(&store, "alice"), vec![task]);
    assert_eq!(unseen_for(&store, "bob"), vec![task]);
    assert!(unseen_for(&store, "outsider").is_empty());
}

/// A private epic is seen by its holder only, so only the holder is marked.
#[test]
fn a_signalled_task_in_a_private_epic_marks_only_its_holder() {
    let store = Store::new();
    let epic = store.create_epic("alice", "mine", false);
    store
        .call("alice", |ctx| {
            set_epic_visibility(ctx, epic, VISIBILITY_PRIVATE.into())
        })
        .expect("set_epic_visibility");
    store.enrol("bob");

    let task = store.create_signalled("alice", epic_task("new", epic, "backlog"));

    assert_eq!(unseen_for(&store, "alice"), vec![task]);
    assert!(unseen_for(&store, "bob").is_empty());
}

/// A plain `create_task` never writes a mark.
#[test]
fn an_ordinary_create_marks_nothing() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    store.enrol("bob");

    store.create("alice", epic_task("plain", epic, "backlog"));

    assert!(store.db().unseen_tasks().iter().next().is_none());
}

/// Someone who joins after the task exists is not marked for it.
#[test]
fn a_later_member_is_not_marked() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    store.create_signalled("alice", epic_task("new", epic, "backlog"));

    store.enrol("late");

    assert!(unseen_for(&store, "late").is_empty());
}

/// The cursor resting on a task card clears that person's mark on that task
/// only: its sibling, a deeper task and other people's marks stay
/// (`tasks.allium: SeeTaskByLandingOnIt`).
#[test]
fn seeing_a_task_clears_only_that_task_for_that_person() {
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
    store.enrol("bob");
    store.enrol("carol");
    let shallow = store.create_signalled("alice", epic_task("shallow", root, "backlog"));
    let sibling = store.create_signalled("alice", epic_task("sibling", root, "backlog"));
    let deep = store.create_signalled("alice", epic_task("deep", child, "backlog"));

    store
        .call("bob", |ctx| mark_task_seen(ctx, identity_of("bob"), shallow))
        .expect("mark_task_seen");

    assert_eq!(unseen_for(&store, "bob"), vec![sibling, deep]);
    assert_eq!(unseen_for(&store, "carol"), vec![shallow, sibling, deep]);
}

/// Marking seen needs an identity; a task with no mark, or one that does not
/// exist, is routine.
#[test]
fn mark_task_seen_refuses_a_blank_identity_and_tolerates_nothing_to_clear() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    let task = store.create_signalled("alice", epic_task("t", epic, "backlog"));

    assert!(store
        .call("alice", |ctx| mark_task_seen(ctx, "  ".into(), task))
        .is_err());
    for id in [task, task, 9_999] {
        store
            .call("alice", |ctx| mark_task_seen(ctx, identity_of("alice"), id))
            .expect("nothing to clear is not an error");
    }
}

/// Moving the task to another epic ends the mark, and it does not follow.
#[test]
fn moving_a_task_out_of_its_epic_clears_the_mark() {
    let store = Store::new();
    let from = store.create_epic("alice", "from", false);
    let to = store.create_epic("alice", "to", false);
    store.enrol("bob");
    let task = store.create_signalled("alice", epic_task("new", from, "backlog"));

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
    store.enrol("bob");
    let task = store.create_signalled("alice", epic_task("new", epic, "running"));

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
    store.enrol("bob");
    let task = store.create_signalled("alice", epic_task("new", epic, "running"));
    // Done clears the mark first, so re-mark by hand to prove the delete path
    // removes rows on its own.
    store.db().unseen_tasks().insert(UnseenTask {
        id: unseen_id(&identity_of("bob"), task),
        task_id: task,
        subscriber: identity_of("bob"),
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
