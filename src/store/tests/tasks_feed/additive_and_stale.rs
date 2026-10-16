use super::*;

#[tokio::test]
async fn upsert_feed_tasks_adds_new_items() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();

    db.ingest_feed(
        epic.id,
        &[make_feed_item("ext-1", "First")],
        &["/repo".to_string()],
        &main_branches(1),
        false,
    )
    .await
    .unwrap();

    db.ingest_feed(
        epic.id,
        &[
            make_feed_item("ext-1", "First"),
            make_feed_item("ext-2", "Second"),
        ],
        &["/repo".to_string(), "/repo".to_string()],
        &main_branches(2),
        false,
    )
    .await
    .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await;
    assert_eq!(tasks.len(), 2, "new item should be created on second call");
}

#[tokio::test]
async fn upsert_feed_tasks_removes_stale_items() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();

    // First fetch: two items
    db.ingest_feed(
        epic.id,
        &[
            make_feed_item("ext-1", "First"),
            make_feed_item("ext-2", "Second"),
        ],
        &["/repo".to_string(), "/repo".to_string()],
        &main_branches(2),
        false,
    )
    .await
    .unwrap();
    assert_eq!(db.list_tasks_for_epic(epic.id).await.len(), 2);

    // Second fetch: only ext-1 remains in the feed
    db.ingest_feed(
        epic.id,
        &[make_feed_item("ext-1", "First")],
        &["/repo".to_string()],
        &main_branches(1),
        false,
    )
    .await
    .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await;
    assert_eq!(tasks.len(), 1, "stale feed task should be removed");
    assert_eq!(tasks[0].external_id.as_deref(), Some("ext-1"));
}
