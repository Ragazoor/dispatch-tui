//! What the store records for a learning, run against the in-memory store:
//! the author stamp (`store-split.allium: IdentitiesAreTheCallers`), the scope
//! refusals and the re-scope of a deleted epic's learnings
//! (`learnings.allium: ReScopeLearningsOnEpicDelete`). One person uses the store: every learning is served
//! to everyone (`docs/plans/2026-11-23-remove-workspaces.md`, R2).

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

fn epic_in(store: &Store, who: &str) -> i64 {
    let row = Epic {
        title: "e".into(),
        ..blank_epic()
    };
    store
        .call(who, |ctx| create_epic(ctx, row))
        .expect("create_epic");
    store.db().epics().iter().map(|e| e.id).max().expect("epic")
}

/// A learning as a client sends it: no author.
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

#[test]
fn a_client_cannot_choose_the_author_and_the_dead_workspace_stays_zero() {
    let store = world();
    let epic = epic_in(&store, "alice");
    let mut sent = row("epic", Some(epic.to_string()), None);
    sent.workspace_id = 7;
    sent.author = identity_of("bob");
    let l = record(&store, "alice", sent).expect("record");
    assert_eq!(l.workspace_id, 0);
    assert_eq!(l.author, identity_of("alice"));
}

#[test]
fn anyone_may_delete_a_learning_someone_else_recorded() {
    let store = world();
    let l = record(&store, "alice", row("user", None, None)).expect("record");
    store
        .call("bob", |ctx| delete_learning(ctx, l.id))
        .expect("R2: no learning is withheld");
    assert!(store.db().learnings().id().find(l.id).is_none());
}

/// `visible_learnings` serves every learning, whoever recorded it: its full
/// read over a live table context (`StoreReads::all_learnings`).
#[test]
fn every_learning_is_served_to_everyone_through_the_view_body() {
    let store = world();
    let l = record(&store, "alice", row("user", None, None)).expect("record");
    let seen = store
        .call("bob", |ctx| {
            Ok(learnings_visible_to(ctx))
        })
        .expect("read");
    assert!(seen.iter().any(|s| s.id == l.id), "R2: no learning is withheld");
}

#[test]
fn a_task_scoped_learning_is_refused() {
    let store = world();
    let epic = epic_in(&store, "alice");
    let task = store.create("alice", epic_task("t", epic, "backlog"));
    let why = record(&store, "alice", row("task", Some(task.to_string()), None))
        .expect_err("scope=task is gone");
    assert!(why.contains("scope=repo or scope=epic"), "{why}");
}

// -- task #99287: scope_ref refusals, the repair write, manual re-scope -------

#[test]
fn a_blank_repo_scope_ref_is_refused() {
    let store = world();
    for blank in ["", "   "] {
        let why = record(&store, "alice", row("repo", Some(blank.into()), None))
            .expect_err("a blank repo ref would cover every repo");
        assert!(why.contains("non-blank scope_ref"), "{why}");
    }
}

#[test]
fn an_epic_scope_ref_that_names_no_epic_is_refused() {
    let store = world();
    for missing in ["9999", "abc"] {
        let why = record(&store, "alice", row("epic", Some(missing.into()), None))
            .expect_err("no epic answers to this ref");
        assert_eq!(why, format!("epic {missing} does not exist"));
    }
}

#[test]
fn an_embedding_only_patch_leaves_updated_at_and_a_summary_patch_moves_it() {
    let store = world();
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

/// A grouped root, its repo-group sub-epic holding one done task,
/// and a learning scoped to the sub-epic. Returns (root, sub, learning).
fn grouped_with_learning(store: &Store) -> (i64, i64, i64) {
    let root = epic_in(store, "alice");
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
    let store = world();
    let (root, sub, l) = grouped_with_learning(&store);
    store
        .call("alice", |ctx| delete_epic(ctx, sub))
        .expect("a repo-group sub-epic can be deleted by hand");
    assert!(store.db().epics().id().find(sub).is_none());
    assert_eq!(scope_ref_of(&store, l), Some(root.to_string()));
}

#[test]
fn batch_deleting_a_repo_group_sub_epic_moves_its_learnings_to_the_parent() {
    let store = world();
    let (root, sub, l) = grouped_with_learning(&store);
    store
        .call("alice", |ctx| batch_delete(ctx, vec![], vec![sub]))
        .expect("batch delete");
    assert_eq!(scope_ref_of(&store, l), Some(root.to_string()));
}

fn status_of(store: &Store, id: i64) -> String {
    store.db().learnings().id().find(id).expect("row").status
}

#[test]
fn a_batch_that_also_deletes_the_root_parent_archives_the_learnings() {
    let store = world();
    let (root, sub, l) = grouped_with_learning(&store);
    store
        .call("alice", |ctx| batch_delete(ctx, vec![], vec![sub, root]))
        .expect("batch delete");
    assert_eq!(scope_ref_of(&store, l), Some(sub.to_string()));
    assert_eq!(status_of(&store, l), "archived");
}

#[test]
fn deleting_a_hand_made_sub_epic_moves_its_learnings_to_the_parent() {
    let store = world();
    let root = epic_in(&store, "alice");
    let sub = store.create_sub_epic("alice", root);
    let l = record(&store, "alice", row("epic", Some(sub.to_string()), None)).expect("record");
    store
        .call("alice", |ctx| delete_epic(ctx, sub))
        .expect("delete");
    assert_eq!(scope_ref_of(&store, l.id), Some(root.to_string()));
    assert_eq!(status_of(&store, l.id), "approved");
}

#[test]
fn deleting_an_epic_moves_the_learnings_of_its_whole_subtree_up() {
    let store = world();
    let top = epic_in(&store, "alice");
    let middle = store.create_sub_epic("alice", top);
    let leaf = store.create_sub_epic("alice", middle);
    let l = record(&store, "alice", row("epic", Some(leaf.to_string()), None)).expect("record");
    store
        .call("alice", |ctx| delete_epic(ctx, middle))
        .expect("delete");
    assert_eq!(scope_ref_of(&store, l.id), Some(top.to_string()));
}

#[test]
fn deleting_a_root_epic_archives_its_learnings_and_its_sub_epics() {
    let store = world();
    let root = epic_in(&store, "alice");
    let sub = store.create_sub_epic("alice", root);
    let on_root = record(&store, "alice", row("epic", Some(root.to_string()), None)).expect("record");
    let on_sub = record(&store, "alice", row("epic", Some(sub.to_string()), None)).expect("record");
    let elsewhere = record(&store, "alice", row("user", None, None)).expect("record");
    store
        .call("alice", |ctx| delete_epic(ctx, root))
        .expect("delete");
    assert_eq!(status_of(&store, on_root.id), "archived");
    assert_eq!(status_of(&store, on_sub.id), "archived");
    assert_eq!(status_of(&store, elsewhere.id), "approved");
}

#[test]
fn a_batch_of_an_epic_and_its_parent_moves_the_learnings_past_both() {
    let store = world();
    let top = epic_in(&store, "alice");
    let middle = store.create_sub_epic("alice", top);
    let leaf = store.create_sub_epic("alice", middle);
    let l = record(&store, "alice", row("epic", Some(leaf.to_string()), None)).expect("record");
    store
        .call("alice", |ctx| batch_delete(ctx, vec![], vec![leaf, middle]))
        .expect("batch delete");
    assert_eq!(scope_ref_of(&store, l.id), Some(top.to_string()));
}

#[test]
fn an_older_store_archives_the_epic_learnings_earlier_deletes_left_behind() {
    let store = world();
    let kept_epic = epic_in(&store, "alice");
    let kept = record(&store, "alice", row("epic", Some(kept_epic.to_string()), None)).expect("record");
    let orphan = Learning {
        id: 9_001,
        ..row("epic", Some("404".into()), None)
    };
    store.db().learnings().insert(orphan);
    store
        .db()
        .rules_version()
        .id()
        .update(RulesVersion { id: 1, version: ORPHANED_EPIC_LEARNINGS_RULES_VERSION - 1 });
    store
        .call("alice", |ctx| {
            stamp_rules_version_on_connect(ctx);
            Ok(())
        })
        .expect("connect");
    assert_eq!(status_of(&store, 9_001), "archived");
    assert_eq!(status_of(&store, kept.id), "approved");
}
