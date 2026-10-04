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

#[tokio::test]
async fn open_in_memory_unattached_leaves_every_shared_port_empty() {
    let db = Database::open_in_memory_unattached().await.unwrap();
    assert!(db.shared_writer.is_none());
    assert!(db.shared_reader.is_none());
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
