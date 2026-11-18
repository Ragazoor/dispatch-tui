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
    assert!(
        !can_see(&store, "bob", l.id),
        "a workspace mate is not the author"
    );
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
fn a_task_scoped_learning_is_refused() {
    let (store, a, _b) = world();
    let epic = epic_in(&store, "alice", a);
    let task = store.create("alice", epic_task("t", epic, "backlog"));
    let why = record(&store, "alice", row("task", Some(task.to_string()), None))
        .expect_err("scope=task is gone");
    assert!(why.contains("scope=repo or scope=epic"), "{why}");
}

#[test]
fn a_repo_learning_takes_the_workspace_of_the_recording_tasks_epic() {
    let (store, a, _b) = world();
    let epic = epic_in(&store, "alice", a);
    let task = store.create("alice", epic_task("t", epic, "backlog"));
    let l = record(
        &store,
        "alice",
        row("repo", Some("/repo".into()), Some(task)),
    )
    .expect("record");
    assert_eq!(l.workspace_id, a);
    assert!(can_see(&store, "bob", l.id));
    assert!(!can_see(&store, "carol", l.id));
}

#[test]
fn a_repo_learning_from_a_main_board_task_is_private_to_its_author() {
    let (store, _a, _b) = world();
    let task = store.create("alice", user_task("mine", "alice"));
    let l = record(
        &store,
        "alice",
        row("repo", Some("/repo".into()), Some(task)),
    )
    .expect("record");
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
    let l = record(
        &store,
        "carol",
        row("repo", Some("/repo".into()), Some(task)),
    )
    .expect("record");
    assert_eq!(l.workspace_id, 0);
    assert!(!can_see(&store, "alice", l.id));
}

#[test]
fn a_learning_from_before_workspaces_stays_visible_to_everyone() {
    let (store, _a, _b) = world();
    let mut old = row("user", None, None);
    old.id = 900;
    store
        .call_as_scheduler(|ctx| seed_learnings(ctx, vec![old]))
        .expect("seed");
    assert!(can_see(&store, "alice", 900));
    assert!(can_see(&store, "carol", 900));
}

// -- task #99287: scope_ref refusals, the repair write, manual re-scope -------

#[test]
fn a_blank_repo_scope_ref_is_refused() {
    let (store, _a, _b) = world();
    for blank in ["", "   "] {
        let why = record(&store, "alice", row("repo", Some(blank.into()), None))
            .expect_err("a blank repo ref would cover every repo");
        assert!(why.contains("non-blank scope_ref"), "{why}");
    }
}

#[test]
fn an_epic_scope_ref_that_names_no_epic_is_refused_like_an_unseen_one() {
    let (store, a, _b) = world();
    let epic = epic_in(&store, "alice", a);
    let unseen = record(&store, "carol", row("epic", Some(epic.to_string()), None))
        .expect_err("carol cannot see alice's epic");
    assert_eq!(unseen, format!("epic {epic} does not exist"));
    for missing in ["9999", "abc"] {
        let why = record(&store, "alice", row("epic", Some(missing.into()), None))
            .expect_err("no epic answers to this ref");
        assert_eq!(why, format!("epic {missing} does not exist"));
    }
}

#[test]
fn an_embedding_only_patch_leaves_updated_at_and_a_summary_patch_moves_it() {
    let (store, _a, _b) = world();
    let l = record(&store, "alice", row("user", None, None)).expect("record");
    let stamp = l.updated_at.clone();
    store
        .call("alice", |ctx| {
            patch_learning(
                ctx,
                l.id,
                LearningPatch {
                    embedding: Some(Some(vec![1, 2, 3])),
                    ..LearningPatch::default()
                },
            )
        })
        .expect("embed");
    let after = store.db().learnings().id().find(l.id).expect("row");
    assert_eq!(after.embedding, Some(vec![1, 2, 3]));
    assert_eq!(
        after.updated_at, stamp,
        "the repair pass changes nothing else"
    );
    store
        .call("alice", |ctx| {
            patch_learning(
                ctx,
                l.id,
                LearningPatch {
                    summary: Some("revised".into()),
                    ..LearningPatch::default()
                },
            )
        })
        .expect("revise");
    let revised = store.db().learnings().id().find(l.id).expect("row");
    assert_ne!(revised.updated_at, stamp);
}

/// A grouped root in `ws`, its repo-group sub-epic holding one done task,
/// and a learning scoped to the sub-epic. Returns (root, sub, learning).
fn grouped_with_learning(store: &Store, ws: i64) -> (i64, i64, i64) {
    let root = epic_in(store, "alice", ws);
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
    store.create(
        "alice",
        Task {
            repo_path: "/x/alpha".into(),
            ..epic_task("t", root, "done")
        },
    );
    let sub = store
        .db()
        .epics()
        .iter()
        .find(|e| e.origin == "repo-group" && e.parent_epic_id == root)
        .expect("repo group")
        .id;
    let l = record(store, "alice", row("epic", Some(sub.to_string()), None)).expect("record");
    (root, sub, l.id)
}

fn scope_ref_of(store: &Store, id: i64) -> Option<String> {
    store.db().learnings().id().find(id).expect("row").scope_ref
}

#[test]
fn deleting_a_repo_group_sub_epic_by_hand_moves_its_learnings_to_the_parent() {
    let (store, a, _b) = world();
    let (root, sub, l) = grouped_with_learning(&store, a);
    store
        .call("alice", |ctx| delete_epic(ctx, sub))
        .expect("a repo-group sub-epic can be deleted by hand");
    assert!(store.db().epics().id().find(sub).is_none());
    assert_eq!(scope_ref_of(&store, l), Some(root.to_string()));
}

#[test]
fn batch_deleting_a_repo_group_sub_epic_moves_its_learnings_to_the_parent() {
    let (store, a, _b) = world();
    let (root, sub, l) = grouped_with_learning(&store, a);
    store
        .call("alice", |ctx| batch_delete(ctx, vec![], vec![sub]))
        .expect("batch delete");
    assert_eq!(scope_ref_of(&store, l), Some(root.to_string()));
}

#[test]
fn a_batch_that_also_deletes_the_parent_re_scopes_nothing() {
    let (store, a, _b) = world();
    let (root, sub, l) = grouped_with_learning(&store, a);
    store
        .call("alice", |ctx| batch_delete(ctx, vec![], vec![sub, root]))
        .expect("batch delete");
    assert_eq!(scope_ref_of(&store, l), Some(sub.to_string()));
}

#[test]
fn deleting_a_hand_made_sub_epic_re_scopes_nothing() {
    let (store, a, _b) = world();
    let root = epic_in(&store, "alice", a);
    let sub = store.create_sub_epic("alice", root);
    let l = record(&store, "alice", row("epic", Some(sub.to_string()), None)).expect("record");
    store
        .call("alice", |ctx| delete_epic(ctx, sub))
        .expect("delete");
    assert_eq!(scope_ref_of(&store, l.id), Some(sub.to_string()));
}
