//! Task and epic reducers, run against the in-memory store. Twins of the
//! live-store cases in `tests/spacetime_module/mutations.rs` that need no
//! publish: create, patch, delete and the epic status they drive.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

#[test]
fn create_task_ignores_a_supplied_id_and_stamps_nothing_else_it_was_not_given() {
    let store = Store::new();
    let row = Task {
        id: 99,
        ..user_task("first", "alice")
    };
    let id = store.create("alice", row);
    assert_ne!(id, 99, "the store reserves ids, the caller does not");
    assert_eq!(store.task(id).title, "first");
}

#[test]
fn the_store_refuses_a_user_board_task_with_no_owner() {
    let store = Store::new();
    let row = Task {
        owner: String::new(),
        ..user_task("orphan", "alice")
    };
    // A person's epic-less task is stamped with them (`identity_stamping.rs`),
    // so only the store itself, which names an owner, can leave one blank.
    let refused = store.call_as_scheduler(|ctx| create_task(ctx, row));
    assert!(refused.is_err(), "an epic-less task needs an owner");
    assert_eq!(store.db().tasks().iter().count(), 0, "nothing was written");
}

#[test]
fn an_epics_status_is_derived_from_every_hosts_children() {
    let store = Store::new();
    let epic = store.create_epic("alice", "Shared", false);
    let a = store.create("alice", epic_task("finished on host-a", epic, "done"));
    let b = store.create("bob", epic_task("still going on host-b", epic, "running"));
    assert_ne!(a, b);
    assert_eq!(
        store.epic(epic).status,
        "backlog",
        "one child is still running elsewhere, so the epic is not done"
    );

    store
        .call("bob", |ctx| patch_task(ctx, b, status_patch("done")))
        .expect("patch_task");
    assert_eq!(store.epic(epic).status, "done");
}

#[test]
fn patching_a_task_to_done_stamps_completion_from_the_stores_clock() {
    let store = Store::new();
    let id = store.create("alice", user_task("t", "alice"));
    assert_eq!(store.task(id).completed_at, "");

    store.advance_secs(90);
    store
        .call("alice", |ctx| patch_task(ctx, id, status_patch("done")))
        .expect("patch_task");
    assert_eq!(store.task(id).completed_at, "2026-10-10 12:01:30.000");
}

#[test]
fn patching_a_missing_task_is_a_silent_no_op() {
    let store = Store::new();
    store
        .call("alice", |ctx| patch_task(ctx, 404, status_patch("done")))
        .expect("a missing id is not an error");
}

#[test]
fn deleting_an_undone_task_is_refused_at_the_reducer() {
    let store = Store::new();
    let id = store.create("alice", user_task("still running", "alice"));
    store
        .call("alice", |ctx| patch_task(ctx, id, status_patch("running")))
        .expect("patch_task");

    let refused = store.call("alice", |ctx| delete_task(ctx, id));
    let why = refused.expect_err("a task that is not done must be refused");
    assert!(why.contains("not done"), "the refusal says why: {why}");
    assert!(store.db().tasks().id().find(id).is_some());

    store
        .call("alice", |ctx| patch_task(ctx, id, status_patch("done")))
        .expect("patch_task");
    store
        .call("alice", |ctx| delete_task(ctx, id))
        .expect("a done task deletes");
    assert!(store.db().tasks().id().find(id).is_none());
}

#[test]
fn deleting_the_last_unfinished_child_lets_the_epic_finish() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    let done = store.create("alice", epic_task("done", epic, "done"));
    store.create("alice", epic_task("open", epic, "backlog"));
    assert_eq!(store.epic(epic).status, "backlog");

    // Finish and delete the open one: only the done child remains.
    let open = store
        .db()
        .tasks()
        .iter()
        .find(|t| t.status == "backlog")
        .expect("the open child")
        .id;
    store
        .call("alice", |ctx| patch_task(ctx, open, status_patch("done")))
        .expect("patch_task");
    store
        .call("alice", |ctx| delete_task(ctx, open))
        .expect("delete_task");
    assert_eq!(store.task(done).status, "done");
    assert_eq!(store.epic(epic).status, "done");
}

#[test]
fn moving_a_task_into_an_epic_that_does_not_exist_is_refused() {
    let store = Store::new();
    let id = store.create("alice", user_task("stays put", "alice"));
    let refused = store.call("alice", |ctx| set_task_epic(ctx, id, 9_999, String::new()));
    let why = refused.expect_err("a missing epic is refused");
    assert!(why.contains("does not exist"), "{why}");
    assert_eq!(store.task(id).epic_id, 0, "the task did not move");
}

#[test]
fn moving_a_task_that_does_not_exist_is_refused() {
    let store = Store::new();
    let epic = store.create_epic("alice", "Shared", false);
    let why = store
        .call("alice", |ctx| set_task_epic(ctx, 404, epic, String::new()))
        .expect_err("the caller learns the task is gone");
    assert!(why.contains("not found"), "{why}");
}

fn blank_learning(id: i64, summary: &str, source_task_id: Option<i64>) -> Learning {
    Learning {
        id,
        kind: "pitfall".into(),
        summary: summary.into(),
        detail: None,
        scope: "repo".into(),
        scope_ref: None,
        tags: "[]".into(),
        status: "approved".into(),
        source_task_id,
        upvote_count: 0,
        last_upvoted_at: None,
        created_at: "2026-09-19 10:00:00".into(),
        updated_at: "2026-09-19 10:00:00".into(),
        embedding: None,
        workspace_id: 0,
        author: String::new(),
    }
}

fn retrieval(id: i64, task_id: i64, learning_id: i64) -> LearningRetrieval {
    LearningRetrieval {
        id,
        task_id,
        learning_id,
        source: "prompt_injection".into(),
        retrieved_at: "2026-09-19 10:00:00".into(),
    }
}

fn usage_event(action: &str) -> UsageEvent {
    UsageEvent {
        id: 0,
        recorded_at: "2026-09-26 10:00:00".into(),
        category: "keybinding".into(),
        action: action.into(),
        detail: None,
        actor: "human".into(),
        // The store stamps the caller over whatever is sent.
        recorded_by: "someone-else".into(),
    }
}

#[test]
fn finishing_an_epic_stamps_a_completion_and_reopening_keeps_it() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    let id = store.create("alice", epic_task("t", epic, "running"));
    assert_eq!(store.epic(epic).completed_at, "");

    store.advance_secs(30);
    store
        .call("alice", |ctx| patch_task(ctx, id, status_patch("done")))
        .expect("patch_task");
    let stamped = store.epic(epic).completed_at;
    assert_eq!(stamped, "2026-10-10 12:00:30.000");

    store.advance_secs(30);
    store
        .call("alice", |ctx| patch_task(ctx, id, status_patch("running")))
        .expect("patch_task");
    assert_eq!(store.epic(epic).status, "backlog");
    assert_eq!(
        store.epic(epic).completed_at,
        stamped,
        "a regression must not clear the last completion"
    );
}

#[test]
fn two_hosts_patching_different_fields_of_one_task_converge() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    let id = store.create("alice", epic_task("original", epic, "backlog"));

    let rename = TaskPatch {
        title: Some("renamed by host-a".into()),
        ..TaskPatch::default()
    };
    store
        .call("alice", |ctx| patch_task(ctx, id, rename))
        .expect("patch_task");
    let move_it = TaskPatch {
        worktree: Some("/wt/host-b".into()),
        ..TaskPatch::default()
    };
    store
        .call("bob", |ctx| patch_task(ctx, id, move_it))
        .expect("patch_task");

    assert_eq!(
        store.task(id).title,
        "renamed by host-a",
        "host B's write must not revert a field it never named"
    );
    assert_eq!(store.task(id).worktree, "/wt/host-b");
}

#[test]
fn a_task_created_in_done_is_stamped_as_completed() {
    let store = Store::new();
    store.advance_secs(1);
    let done = Task {
        status: "done".into(),
        ..user_task("born finished", "alice")
    };
    let born = store.create("alice", done);
    assert_eq!(store.task(born).completed_at, "2026-10-10 12:00:01.000");
}

#[test]
fn deleting_a_learning_cascades_its_retrievals_and_refuses_when_missing() {
    let store = Store::new();
    store
        .call_as_scheduler(|ctx| seed_learnings(ctx, vec![blank_learning(1, "A learning", None)]))
        .expect("seed_learnings");
    store
        .call_as_scheduler(|ctx| seed_learning_retrievals(ctx, vec![retrieval(1, 7, 1)]))
        .expect("seed_learning_retrievals");

    store
        .call("alice", |ctx| delete_learning(ctx, 1))
        .expect("delete_learning");
    assert_eq!(store.db().learnings().iter().count(), 0);
    assert_eq!(store.db().learning_retrievals().iter().count(), 0);

    let refused = store.call("alice", |ctx| delete_learning(ctx, 1));
    assert!(refused.is_err(), "a missing id is refused, not a no-op");
}

#[test]
fn recording_usage_events_prunes_beyond_the_cap() {
    let store = Store::new();
    for i in 1..=5 {
        store
            .call("alice", |ctx| {
                record_usage_event(ctx, usage_event(&format!("action_{i}")), 3)
            })
            .expect("record_usage_event");
    }
    let actions: Vec<String> = store.db().usage_events().iter().map(|e| e.action).collect();
    assert_eq!(actions.len(), 3, "the table never exceeds the cap");
    assert!(!actions.contains(&"action_1".to_string()), "oldest pruned");
    assert!(actions.contains(&"action_5".to_string()), "newest kept");
    assert!(
        store
            .db()
            .usage_events()
            .iter()
            .all(|e| e.recorded_by == identity_of("alice")),
        "every event is stamped with whoever recorded it"
    );
}

// -- update_task: the relink and the patch are one transaction ---------------

#[test]
fn update_task_relinks_and_patches_in_one_call() {
    let store = Store::new();
    let epic = store.create_epic("alice", "Shared", false);
    let id = store.create("alice", user_task("t", "alice"));
    let patch = TaskPatch {
        title: Some("renamed".into()),
        ..TaskPatch::default()
    };
    store
        .call("alice", |ctx| {
            update_task(ctx, id, Some(epic), String::new(), String::new(), patch, String::new(), None)
        })
        .expect("update_task");
    let task = store.task(id);
    assert_eq!(task.epic_id, epic);
    assert_eq!(task.title, "renamed");
}

#[test]
fn update_task_with_an_illegal_patch_leaves_the_relink_undone() {
    let store = Store::new();
    let epic = store.create_epic("alice", "Shared", false);
    let id = store.create("alice", user_task("t", "alice"));
    // Backlog allows only `none`; `repo_path` makes the call relink first.
    let patch = TaskPatch {
        repo_path: Some("/other".into()),
        sub_status: Some("active".into()),
        ..TaskPatch::default()
    };
    let why = store
        .call("alice", |ctx| {
            update_task(ctx, id, Some(epic), String::new(), String::new(), patch, String::new(), None)
        })
        .expect_err("the sub_status is illegal for backlog");
    assert!(why.contains("not valid for status"), "{why}");
    let task = store.task(id);
    assert_eq!(task.epic_id, 0, "the task did not move");
    assert_eq!(task.repo_path, "/repo", "the repo did not change");
}

#[test]
fn update_task_into_a_missing_epic_changes_nothing() {
    let store = Store::new();
    let id = store.create("alice", user_task("t", "alice"));
    let patch = TaskPatch {
        title: Some("renamed".into()),
        ..TaskPatch::default()
    };
    let why = store
        .call("alice", |ctx| {
            update_task(ctx, id, Some(9_999), String::new(), String::new(), patch, String::new(), None)
        })
        .expect_err("a missing epic is refused");
    assert!(why.contains("does not exist"), "{why}");
    assert_eq!(store.task(id).title, "t");
}

#[test]
fn update_task_on_a_missing_task_is_refused() {
    let store = Store::new();
    let why = store
        .call("alice", |ctx| {
            update_task(
                ctx,
                404,
                None,
                String::new(),
                String::new(),
                status_patch("done"), String::new(), None,
            )
        })
        .expect_err("the caller learns the task is gone");
    assert!(why.contains("not found"), "{why}");
}

#[test]
fn update_task_stamps_done_from_the_store_clock_over_a_named_completion() {
    let store = Store::new();
    let id = store.create("alice", user_task("t", "alice"));
    store.advance_secs(90);
    let patch = TaskPatch {
        status: Some("done".into()),
        completed_at: Some("2001-01-01 00:00:00.000".into()),
        ..TaskPatch::default()
    };
    store
        .call("alice", |ctx| {
            update_task(ctx, id, None, String::new(), String::new(), patch, String::new(), None)
        })
        .expect("update_task");
    assert_eq!(store.task(id).completed_at, "2026-10-10 12:01:30.000");
}

#[test]
fn update_task_keeps_a_named_completion_on_a_task_already_done() {
    let store = Store::new();
    let id = store.create("alice", user_task("t", "alice"));
    store
        .call("alice", |ctx| patch_task(ctx, id, status_patch("done")))
        .expect("patch_task");
    let patch = TaskPatch {
        completed_at: Some("2001-01-01 00:00:00.000".into()),
        ..TaskPatch::default()
    };
    store
        .call("alice", |ctx| {
            update_task(ctx, id, None, String::new(), String::new(), patch, String::new(), None)
        })
        .expect("update_task");
    assert_eq!(store.task(id).completed_at, "2001-01-01 00:00:00.000");
}

// -- An epic's own patch recalculates its parent chain, never the epic itself
// (epics.allium: RecalculateEpicStatus, MoveEpicForward, MoveEpicBackward).

fn sub_epic(store: &Store, who: &str, parent: i64) -> i64 {
    let row = Epic {
        title: format!("sub of {parent}"),
        workspace_id: TEST_WORKSPACE,
        parent_epic_id: parent,
        ..blank_epic()
    };
    store
        .call(who, |ctx| create_epic(ctx, row))
        .expect("create_epic");
    store.db().epics().iter().map(|e| e.id).max().expect("epic")
}

fn set_epic(store: &Store, id: i64, patch: EpicPatch) {
    store
        .call("alice", |ctx| patch_epic(ctx, id, patch))
        .expect("patch_epic");
}

fn epic_status(status: &str) -> EpicPatch {
    EpicPatch {
        status: Some(status.into()),
        ..EpicPatch::default()
    }
}

#[test]
fn a_manual_move_into_done_with_an_unfinished_child_stays_done() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    store.create("alice", epic_task("open", epic, "running"));

    set_epic(&store, epic, epic_status("done"));
    assert_eq!(store.epic(epic).status, "done", "the placement stands");
}

#[test]
fn a_done_epic_moved_back_with_every_child_done_stays_where_it_was_put() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    store.create("alice", epic_task("finished", epic, "done"));
    assert_eq!(store.epic(epic).status, "done");
    let finished_at = store.epic(epic).completed_at;

    store.advance_secs(30);
    set_epic(&store, epic, epic_status("review"));
    let row = store.epic(epic);
    assert_eq!(row.status, "review", "the placement stands");
    assert_eq!(row.completed_at, finished_at, "no new completion");
}

#[test]
fn renaming_an_epic_keeps_its_manual_placement() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E", false);
    store.create("alice", epic_task("open", epic, "running"));
    set_epic(&store, epic, epic_status("done"));

    set_epic(
        &store,
        epic,
        EpicPatch {
            title: Some("Renamed".into()),
            ..EpicPatch::default()
        },
    );
    assert_eq!(store.epic(epic).status, "done");
}

#[test]
fn a_sub_epics_status_move_still_recalculates_its_parent() {
    let store = Store::new();
    let parent = store.create_epic("alice", "P", false);
    let sub = sub_epic(&store, "alice", parent);
    assert_eq!(store.epic(parent).status, "backlog");

    set_epic(&store, sub, epic_status("done"));
    assert_eq!(store.epic(parent).status, "done", "its only child is done");
}

#[test]
fn reparenting_an_epic_recalculates_both_parents() {
    let store = Store::new();
    let from = store.create_epic("alice", "From", false);
    store.create("alice", epic_task("done in from", from, "done"));
    let moved = sub_epic(&store, "alice", from);
    set_epic(&store, moved, epic_status("running"));
    assert_eq!(store.epic(from).status, "backlog");

    let to = store.create_epic("alice", "To", false);
    store.create("alice", epic_task("done in to", to, "done"));
    assert_eq!(store.epic(to).status, "done");

    set_epic(
        &store,
        moved,
        EpicPatch {
            parent_epic_id: Some(to),
            ..EpicPatch::default()
        },
    );
    assert_eq!(store.epic(from).status, "done", "only its done task is left");
    assert_eq!(store.epic(to).status, "backlog", "a running child arrived");
    assert_eq!(store.epic(moved).status, "running", "the moved epic is untouched");
}
