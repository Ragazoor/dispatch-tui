//! Who records and who sees a learning (`workspaces.allium`:
//! `LearningVisibility`, D8), run against the in-memory store.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

/// `root` is super-admin; `alice` and `bob` are members of workspace A,
/// `carol` of workspace B. Returns (store, A, B).
fn world() -> (Store, i64, i64) {
    let store = Store::bare();
    for who in ["root", "alice", "bob", "carol"] {
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
    for (ws, who) in [(ids[0], "alice"), (ids[0], "bob"), (ids[1], "carol")] {
        store
            .call("root", |ctx| {
                add_member(ctx, ws, identity_of(who), "member".into())
            })
            .expect("member");
    }
    (store, ids[0], ids[1])
}

fn epic_in(store: &Store, who: &str, workspace_id: i64) -> i64 {
    let row = Epic {
        title: "e".into(),
        workspace_id,
        visibility: "public".into(),
        ..blank_epic()
    };
    store
        .call(who, |ctx| create_epic(ctx, row))
        .expect("create_epic");
    store.db().epics().iter().map(|e| e.id).max().expect("epic")
}

/// A learning as a client sends it: no author, and a workspace it may not
/// choose.
fn row(scope: &str, scope_ref: Option<String>, source_task_id: Option<i64>) -> Learning {
    Learning {
        id: 0,
        kind: "pitfall".into(),
        summary: "a learning".into(),
        detail: None,
        scope: scope.into(),
        scope_ref,
        tags: "[]".into(),
        status: "approved".into(),
        source_task_id,
        upvote_count: 0,
        last_upvoted_at: None,
        created_at: "2026-10-10 12:00:00".into(),
        updated_at: "2026-10-10 12:00:00".into(),
        embedding: None,
        workspace_id: 0,
        author: String::new(),
    }
}

fn record(store: &Store, who: &str, row: Learning) -> Result<Learning, String> {
    store.call(who, |ctx| create_learning(ctx, row))?;
    Ok(store
        .db()
        .learnings()
        .iter()
        .max_by_key(|l| l.id)
        .expect("a learning"))
}

fn can_see(store: &Store, who: &str, id: i64) -> bool {
    // delete_learning answers an unseen learning as a missing one.
    store.call(who, |ctx| Ok(visible_learning(ctx, id).is_some())) == Ok(true)
}

#[test]
fn a_user_scoped_learning_has_no_workspace_and_is_private_to_its_author() {
    let (store, _a, _b) = world();
    let l = record(&store, "alice", row("user", None, None)).expect("record");
    assert_eq!(l.workspace_id, 0);
    assert_eq!(l.author, identity_of("alice"));
    assert!(can_see(&store, "alice", l.id));
    assert!(!can_see(&store, "bob", l.id), "a workspace mate is not the author");
    assert!(store.call("bob", |ctx| delete_learning(ctx, l.id)).is_err());
}

#[test]
fn an_epic_scoped_learning_takes_its_epics_workspace() {
    let (store, a, _b) = world();
    let epic = epic_in(&store, "alice", a);
    let l = record(&store, "alice", row("epic", Some(epic.to_string()), None)).expect("record");
    assert_eq!(l.workspace_id, a);
    assert!(can_see(&store, "bob", l.id));
    assert!(!can_see(&store, "carol", l.id));
}

#[test]
fn a_client_cannot_choose_the_workspace_or_the_author() {
    let (store, a, b) = world();
    let epic = epic_in(&store, "alice", a);
    let mut sent = row("epic", Some(epic.to_string()), None);
    sent.workspace_id = b;
    sent.author = identity_of("bob");
    let l = record(&store, "alice", sent).expect("record");
    assert_eq!(l.workspace_id, a);
    assert_eq!(l.author, identity_of("alice"));
}

#[test]
fn a_learning_on_an_epic_the_caller_cannot_see_is_refused() {
    let (store, a, _b) = world();
    let epic = epic_in(&store, "alice", a);
    assert!(record(&store, "carol", row("epic", Some(epic.to_string()), None)).is_err());
}

#[test]
fn a_task_scoped_learning_follows_its_tasks_epic() {
    let (store, a, _b) = world();
    let epic = epic_in(&store, "alice", a);
    let task = store.create("alice", epic_task("t", epic, "backlog"));
    let l = record(&store, "alice", row("task", Some(task.to_string()), None)).expect("record");
    assert_eq!(l.workspace_id, a);
    assert!(can_see(&store, "bob", l.id));
    assert!(!can_see(&store, "carol", l.id));
}

#[test]
fn a_task_scoped_learning_on_a_main_board_task_is_private_to_its_author() {
    let (store, _a, _b) = world();
    let task = store.create("alice", user_task("mine", "alice"));
    let l = record(&store, "alice", row("task", Some(task.to_string()), None)).expect("record");
    assert_eq!(l.workspace_id, 0);
    assert!(can_see(&store, "alice", l.id));
    assert!(!can_see(&store, "bob", l.id));
}

#[test]
fn a_repo_learning_takes_the_workspace_of_the_recording_tasks_epic() {
    let (store, a, _b) = world();
    let epic = epic_in(&store, "alice", a);
    let task = store.create("alice", epic_task("t", epic, "backlog"));
    let l = record(&store, "alice", row("repo", Some("/repo".into()), Some(task))).expect("record");
    assert_eq!(l.workspace_id, a);
    assert!(can_see(&store, "bob", l.id));
    assert!(!can_see(&store, "carol", l.id));
}

#[test]
fn a_repo_learning_from_a_main_board_task_is_private_to_its_author() {
    let (store, _a, _b) = world();
    let task = store.create("alice", user_task("mine", "alice"));
    let l = record(&store, "alice", row("repo", Some("/repo".into()), Some(task))).expect("record");
    assert_eq!(l.workspace_id, 0);
    assert_eq!(l.author, identity_of("alice"));
    assert!(!can_see(&store, "bob", l.id));
}

#[test]
fn a_repo_learning_with_no_recording_task_is_private_to_its_author() {
    let (store, _a, _b) = world();
    let l = record(&store, "alice", row("repo", Some("/repo".into()), None)).expect("record");
    assert_eq!(l.workspace_id, 0);
    assert!(!can_see(&store, "bob", l.id));
}

#[test]
fn a_recording_task_the_caller_cannot_see_gives_no_workspace() {
    let (store, a, _b) = world();
    let epic = epic_in(&store, "alice", a);
    let task = store.create("alice", epic_task("t", epic, "backlog"));
    // carol is not in A: naming alice's task must not put her learning there.
    let l = record(&store, "carol", row("repo", Some("/repo".into()), Some(task))).expect("record");
    assert_eq!(l.workspace_id, 0);
    assert!(!can_see(&store, "alice", l.id));
}

#[test]
fn a_learning_from_before_workspaces_stays_visible_to_everyone() {
    let (store, _a, _b) = world();
    let mut old = row("user", None, None);
    old.id = 900;
    store
        .call("root", |ctx| seed_learnings(ctx, vec![old]))
        .expect("seed");
    assert!(can_see(&store, "alice", 900));
    assert!(can_see(&store, "carol", 900));
}
