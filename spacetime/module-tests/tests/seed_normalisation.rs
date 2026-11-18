//! A restore meets the one-time migrations' normalisation
//! (`spacetime-seed.allium: SeedsNormaliseLikeMigrations`), run against the
//! in-memory store.

mod common;

use common::*;
use dispatch_spacetime_module::*;
#[allow(unused_imports)]
use spacetimedb::Table;

fn learning(id: i64, scope: &str, scope_ref: Option<&str>, author: &str) -> Learning {
    Learning {
        id,
        kind: "pitfall".into(),
        summary: "s".into(),
        detail: None,
        scope: scope.into(),
        scope_ref: scope_ref.map(String::from),
        tags: "[]".into(),
        status: "approved".into(),
        source_task_id: None,
        upvote_count: 0,
        last_upvoted_at: None,
        created_at: "2026-09-19 10:00:00".into(),
        updated_at: "2026-09-19 10:00:00".into(),
        embedding: None,
        workspace_id: 7,
        author: author.into(),
    }
}

/// K1: a task-scoped learning in a snapshot lands retired, not readable by the
/// workspace.
#[test]
fn a_restored_task_scoped_learning_lands_retired() {
    let store = Store::bare();
    store
        .call_as_scheduler(|ctx| seed_learnings(ctx, vec![learning(1, "task", Some("404"), "alice")]))
        .expect("seed_learnings");
    let row = store.db().learnings().id().find(1).expect("row");
    assert_eq!(row.scope, "repo");
    assert_eq!(row.scope_ref.as_deref(), Some(""));
    assert_eq!(row.status, "archived");
}

/// K1: a learning of a task in a private epic loses its workspace, as the
/// migration does it.
#[test]
fn a_restored_task_learning_of_a_private_epic_loses_its_workspace() {
    let store = Store::bare();
    store
        .call_as_scheduler(|ctx| {
            seed_epics(
                ctx,
                vec![Epic {
                    id: 5,
                    title: "e".into(),
                    visibility: "private".into(),
                    workspace_id: 7,
                    ..blank_epic()
                }],
            )
        })
        .expect("seed_epics");
    store
        .call_as_scheduler(|ctx| {
            seed_tasks(
                ctx,
                vec![Task {
                    id: 9,
                    title: "t".into(),
                    repo_path: "/repo".into(),
                    epic_id: 5,
                    owner: String::new(),
                    ..blank_task()
                }],
            )
        })
        .expect("seed_tasks");
    store
        .call_as_scheduler(|ctx| seed_learnings(ctx, vec![learning(1, "task", Some("9"), "alice")]))
        .expect("seed_learnings");
    let row = store.db().learnings().id().find(1).expect("row");
    assert_eq!(row.workspace_id, 0);
    assert_eq!(row.author, "alice");
    assert_eq!(row.scope_ref.as_deref(), Some("/repo"));
}

/// A learning of any other scope is stored as it came.
#[test]
fn a_restored_repo_learning_is_untouched() {
    let store = Store::bare();
    store
        .call_as_scheduler(|ctx| seed_learnings(ctx, vec![learning(1, "repo", Some("/r"), "alice")]))
        .expect("seed_learnings");
    let row = store.db().learnings().id().find(1).expect("row");
    assert_eq!((row.status.as_str(), row.workspace_id), ("approved", 7));
}

/// K2: a snapshot task whose sub_status its status does not allow is restored
/// with the status's default.
#[test]
fn a_restored_task_with_an_unfit_sub_status_is_reset() {
    let store = Store::bare();
    store
        .call_as_scheduler(|ctx| {
            seed_tasks(
                ctx,
                vec![Task {
                    id: 3,
                    title: "t".into(),
                    repo_path: "/repo".into(),
                    status: "backlog".into(),
                    sub_status: "approved".into(),
                    owner: identity_of("alice"),
                    ..blank_task()
                }],
            )
        })
        .expect("an unfit pair is reset, not refused");
    let row = store.db().tasks().id().find(3).expect("row");
    assert_eq!(row.sub_status, default_sub_status("backlog"));
}
