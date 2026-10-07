//! `OpenInMemoryAttachesStoreOnceComplete` (docs/specs/spacetime-memory-store.allium).
use super::*;

#[tokio::test]
async fn open_in_memory_attaches_every_shared_port_once_store_is_complete() {
    let db = Database::open_in_memory().await.unwrap();
    assert!(db.shared_writer.is_some());
    assert!(db.shared_reader.is_some());
    assert!(db.shared_learning_reader.is_some());
    assert!(db.shared_usage_reader.is_some());
    assert!(db.shared_retired_feed_item_reader.is_some());
}

/// A handle with no store attached has nothing to answer from: every shared
/// read and write refuses, and says why, instead of answering from a table
/// nothing else reads (`storage.allium`).
#[tokio::test]
async fn an_unattached_handle_refuses_every_shared_read_and_write() {
    let db = Database::unattached();
    assert!(db.shared_writer.is_none());
    assert!(db.shared_reader.is_none());

    let read = db.list_all().await.unwrap_err();
    assert!(
        read.to_string().contains("no shared store attached"),
        "{read}"
    );
    let write = db.create_epic("E", "", None).await.unwrap_err();
    assert!(
        write.to_string().contains("no shared store attached"),
        "{write}"
    );
    let learning = db.get_learning(LearningId(1)).await.unwrap_err();
    assert!(
        learning.to_string().contains("no shared store attached"),
        "{learning}"
    );
}

/// Identity lives only in the host file: a handle with none refuses, rather
/// than keeping an identity somewhere nothing else reads.
#[tokio::test]
async fn a_handle_with_no_host_file_refuses_identity() {
    let db = Database::unattached();
    let err = db.ensure_host_identity().await.unwrap_err();
    assert!(err.to_string().contains("no host file attached"), "{err}");
    assert!(db.user_identity().await.is_err());
    assert!(db.adopt_user_identity("user-a").await.is_err());
}

#[tokio::test]
async fn attached_handle_round_trips_a_task_through_the_store_not_sqlite() {
    let db = Database::open_in_memory().await.unwrap();
    let id = db
        .create_task(CreateTaskRequest {
            title: "via store",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();
    assert_eq!(db.get_task(id).await.unwrap().unwrap().title, "via store");
    let other = Database::open_in_memory().await.unwrap();
    assert!(other.list_all().await.unwrap().is_empty());
}
