//! Soft delete: a person's delete keeps a read-only `ArchivedTask`
//! (`core.allium: ArchivedTask`, `tasks.allium: ArchiveDeletedTask`).

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

fn done_user_task(store: &Store, title: &str, owner: &str) -> i64 {
    let id = store.create(
        owner,
        Task {
            description: format!("{title} description"),
            base_branch: "main".into(),
            ..user_task(title, owner)
        },
    );
    store
        .call(owner, |ctx| patch_task(ctx, id, status_patch("done")))
        .expect("patch_task");
    id
}

fn archived(store: &Store, id: i64) -> Option<ArchivedTask> {
    store.db().archived_tasks().id().find(id)
}

fn read_as(store: &Store, who: &str, id: i64) -> Option<ArchivedTask> {
    store
        .call(who, |ctx| Ok(ctx.archived_task(id)))
        .expect("read")
}

fn agent_name_of(store: &Store, task: i64) -> String {
    store
        .db()
        .agents()
        .task_id()
        .filter(&task)
        .next()
        .expect("a created task has an assigned agent")
        .name
}

#[test]
fn deleting_a_done_task_archives_a_copy_of_it() {
    let store = Store::new();
    let id = done_user_task(&store, "Ship it", "alice");
    let agent = agent_name_of(&store, id);

    store
        .call("alice", |ctx| delete_task(ctx, id))
        .expect("delete_task");

    assert!(store.db().tasks().id().find(id).is_none(), "the row is gone");
    let record = archived(&store, id).expect("an archive record");
    assert_eq!(record.title, "Ship it");
    assert_eq!(record.description, "Ship it description");
    assert_eq!(record.repo_path, "/repo");
    assert_eq!(record.base_branch, "main");
    assert_eq!(record.owner, identity_of("alice"));
    assert_eq!(record.epic_id, 0);
    assert_eq!(record.agent_name, agent, "the freed agent name is kept");
    assert_eq!(record.archived_by, identity_of("alice"));
    assert!(!record.archived_at.is_empty());
    assert!(!record.completed_at.is_empty());
}

#[test]
fn a_refused_delete_archives_nothing() {
    let store = Store::new();
    let id = store.create("alice", user_task("running", "alice"));
    store
        .call("alice", |ctx| delete_task(ctx, id))
        .expect_err("not done");
    assert!(archived(&store, id).is_none());
}

#[test]
fn an_archived_task_keeps_its_learnings_retrievals_and_verdicts() {
    let store = Store::new();
    let id = done_user_task(&store, "source", "alice");
    store
        .call_as_scheduler(|ctx| {
            seed_learnings(
                ctx,
                vec![Learning {
                    id: 1,
                    kind: "pitfall".into(),
                    summary: "A learning".into(),
                    detail: None,
                    scope: "repo".into(),
                    scope_ref: None,
                    tags: "[]".into(),
                    status: "approved".into(),
                    source_task_id: Some(id),
                    upvote_count: 0,
                    last_upvoted_at: None,
                    created_at: "2026-09-19 10:00:00".into(),
                    updated_at: "2026-09-19 10:00:00".into(),
                    embedding: None,
                    workspace_id: 0,
                    author: String::new(),
                }],
            )
        })
        .expect("seed_learnings");
    store
        .call_as_scheduler(|ctx| {
            seed_learning_retrievals(
                ctx,
                vec![LearningRetrieval {
                    id: 1,
                    task_id: id,
                    learning_id: 1,
                    source: "prompt_injection".into(),
                    retrieved_at: "2026-09-19 10:00:00".into(),
                }],
            )
        })
        .expect("seed_learning_retrievals");
    store
        .call_as_scheduler(|ctx| {
            seed_learning_verdicts(
                ctx,
                vec![LearningVerdict {
                    id: 1,
                    task_id: id,
                    learning_id: 1,
                    verdict: "helped".into(),
                }],
            )
        })
        .expect("seed_learning_verdicts");

    store
        .call("alice", |ctx| delete_task(ctx, id))
        .expect("delete_task");

    let learning = store.db().learnings().id().find(1).expect("it survives");
    assert_eq!(learning.source_task_id, Some(id), "still points at the task");
    assert_eq!(store.db().learning_retrievals().iter().count(), 1);
    assert_eq!(store.db().learning_verdicts().iter().count(), 1);
}

#[test]
fn deleting_an_epic_archives_every_task_of_its_subtree_with_the_epic_title() {
    let store = Store::new();
    let epic = store.create_epic("alice", "Parent");
    store
        .call("alice", |ctx| {
            create_epic(
                ctx,
                Epic {
                    title: "Child".into(),
                    parent_epic_id: epic,
                    ..blank_epic()
                },
            )
        })
        .expect("create child epic");
    let child = store.db().epics().iter().map(|e| e.id).max().expect("epic");
    let a = store.create("alice", epic_task("a", epic, "done"));
    let b = store.create("alice", epic_task("b", child, "done"));

    store
        .call("alice", |ctx| delete_epic(ctx, epic))
        .expect("delete_epic");

    let ra = archived(&store, a).expect("a archived");
    let rb = archived(&store, b).expect("b archived");
    assert_eq!((ra.epic_id, ra.epic_title.as_str()), (epic, "Parent"));
    assert_eq!((rb.epic_id, rb.epic_title.as_str()), (child, "Child"));
    // The dead columns hold their defaults (R8).
    assert_eq!(ra.workspace_id, 0);
    assert!(!ra.private);
    assert!(ra.private_holder.is_empty());
}

#[test]
fn a_batch_delete_archives_its_plain_tasks_and_epic_subtrees() {
    let store = Store::new();
    let plain = done_user_task(&store, "plain", "alice");
    let epic = store.create_epic("alice", "E");
    let inner = store.create("alice", epic_task("inner", epic, "done"));

    store
        .call("alice", |ctx| batch_delete(ctx, vec![plain], vec![epic]))
        .expect("batch_delete");

    assert!(archived(&store, plain).is_some());
    assert!(archived(&store, inner).is_some());
}

/// R2: no record is withheld; anyone reads and lists every record, another
/// person's epic-less one included.
#[test]
fn every_record_is_read_and_listed_by_anyone() {
    let store = Store::new();
    let id = done_user_task(&store, "mine", "alice");
    store
        .call("alice", |ctx| delete_task(ctx, id))
        .expect("delete_task");

    assert!(read_as(&store, "alice", id).is_some());
    assert!(read_as(&store, "bob", id).is_some());
    let listed = store
        .call("bob", |ctx| Ok(archived_tasks_for(ctx, 0, &[])))
        .expect("list");
    assert_eq!(listed.iter().map(|a| a.id).collect::<Vec<_>>(), vec![id]);
}

#[test]
fn the_listing_filters_by_epic_and_repo() {
    let store = Store::new();
    let epic = store.create_epic("alice", "E");
    let in_epic = store.create("alice", epic_task("in epic", epic, "done"));
    let other = store.create(
        "alice",
        Task {
            repo_path: "/other".into(),
            ..epic_task("other repo", epic, "done")
        },
    );
    store
        .call("alice", |ctx| delete_epic(ctx, epic))
        .expect("delete_epic");

    let by_epic = store
        .call("alice", |ctx| Ok(archived_tasks_for(ctx, epic, &[])))
        .expect("list");
    let mut ids: Vec<i64> = by_epic.iter().map(|r| r.id).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![in_epic, other]);

    let by_repo = store
        .call("alice", |ctx| {
            Ok(archived_tasks_for(ctx, 0, &["/other".to_string()]))
        })
        .expect("list");
    assert_eq!(by_repo.iter().map(|r| r.id).collect::<Vec<_>>(), vec![other]);
}
