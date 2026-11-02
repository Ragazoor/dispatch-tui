//! The new-task signal through the store handle
//! (`mcp-task-tools.allium: SignalMarksTheTaskForEveryoneWhoSeesIt`,
//! `tasks.allium: SeeTaskByLandingOnIt`).
use super::*;

/// An epic the test user sees but does not follow: sight alone earns a mark.
async fn epic_seen(db: &Store) -> crate::models::EpicId {
    db.create_epic("E", "", None).await.unwrap().id
}

fn task_in(epic: crate::models::EpicId, title: &str) -> CreateTaskRequest<'_> {
    CreateTaskRequest {
        epic_id: Some(epic),
        ..CreateTaskRequest::fixture(title, "/repo")
    }
}

#[tokio::test]
async fn a_signalled_task_reads_as_unseen_and_a_plain_one_does_not() {
    let db = Store::open_in_memory().unwrap();
    let epic = epic_seen(&db).await;

    let plain = db.create_task(task_in(epic, "plain")).await.unwrap();
    assert!(!BoardReads::unseen_task_ids(&db).await.contains(&plain));

    let signalled = db
        .create_task_signalled(task_in(epic, "signalled"))
        .await
        .unwrap();
    assert_eq!(BoardReads::unseen_task_ids(&db).await, vec![signalled]);
}

#[tokio::test]
async fn an_epicless_signalled_task_is_created_without_a_mark() {
    let db = Store::open_in_memory().unwrap();

    let id = db
        .create_task_signalled(CreateTaskRequest::fixture("loose", "/repo"))
        .await
        .unwrap();

    assert!(db.get_task(id).await.is_some());
    assert!(BoardReads::unseen_task_ids(&db).await.is_empty());
}

#[tokio::test]
async fn seeing_a_task_clears_its_mark_and_only_its_mark() {
    let db = Store::open_in_memory().unwrap();
    let epic = epic_seen(&db).await;
    let seen = db
        .create_task_signalled(task_in(epic, "seen"))
        .await
        .unwrap();
    let sibling = db
        .create_task_signalled(task_in(epic, "sibling"))
        .await
        .unwrap();

    db.mark_task_seen(seen).await.unwrap();

    assert_eq!(BoardReads::unseen_task_ids(&db).await, vec![sibling]);
}
