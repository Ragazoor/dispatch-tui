use super::*;

#[tokio::test]
async fn upsert_feed_tasks_preserves_status() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![make_feed_item("ext-1", "Original Title")];

    db.upsert_feed_tasks(epic.id, &items, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    // Simulate user moving task to Running
    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    db.patch_task(tasks[0].id, &TaskPatch::new().status(TaskStatus::Running))
        .await
        .unwrap();

    // Re-run upsert with updated title and different status
    let updated = vec![crate::models::FeedItem {
        external_id: "ext-1".to_string(),
        title: "Updated Title".to_string(),
        description: "new desc".to_string(),
        url: String::new(),
        url_type: None,
        status: TaskStatus::Done, // feed says done; user status should be preserved
        tag: crate::models::TaskTag::Bug,
        labels: Vec::new(),
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    }];
    db.upsert_feed_tasks(epic.id, &updated, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "Updated Title", "title should be updated");
    assert_eq!(
        tasks[0].description, "new desc",
        "description should be updated"
    );
    assert_eq!(
        tasks[0].status,
        TaskStatus::Running,
        "user-managed status must be preserved"
    );
}

/// The done exception is gone. `sort_order` used to double as the Done
/// column's completion rank, so a feed's severity-rank re-poll would clobber
/// it and the upsert skipped done tasks to protect it. The rank is
/// `completed_at` now, which no feed field can reach, so the feed's value
/// applies to a done task like any other — and the completion survives
/// untouched beside it.
#[tokio::test]
async fn upsert_feed_tasks_updates_a_done_tasks_sort_order_and_keeps_its_completion() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![make_feed_item("ext-1", "Original Title")];

    db.upsert_feed_tasks(epic.id, &items, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    let finished = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();
    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    db.patch_task(
        tasks[0].id,
        &TaskPatch::new()
            .status(TaskStatus::Done)
            .completed_at(Some(finished)),
    )
    .await
    .unwrap();

    let mut updated_item = make_feed_item("ext-1", "Original Title");
    updated_item.sort_order = Some(1); // feed severity rank
    db.upsert_feed_tasks(
        epic.id,
        &[updated_item],
        &["/repo".to_string()],
        &main_branches(1),
    )
    .await
    .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].sort_order,
        Some(1),
        "the feed's severity rank applies to a done task too"
    );
    assert_eq!(
        tasks[0].completed_at,
        Some(finished),
        "and the completion time is untouched by a re-poll"
    );
}

/// An item that arrives already done is stamped on INSERT, because it never
/// passes through the status transition that would otherwise stamp it. Without
/// this the card sinks to the bottom of the Done column rather than leading it
/// (board-layout.allium, "Done Column Ordering").
#[tokio::test]
async fn upsert_feed_tasks_stamps_a_task_inserted_straight_into_done() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let mut item = make_feed_item("ext-1", "Already finished");
    item.status = TaskStatus::Done;

    let before = chrono::Utc::now() - chrono::Duration::seconds(1);
    db.upsert_feed_tasks(epic.id, &[item], &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();
    let after = chrono::Utc::now() + chrono::Duration::seconds(1);

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    let completed_at = tasks[0]
        .completed_at
        .expect("a card born in Done carries a completion time");
    assert!(
        (before..=after).contains(&completed_at),
        "completed_at {completed_at} should be about now, within [{before}, {after}]"
    );
}

/// A card inserted NOT done takes no stamp.
/// A row inserted straight into Done is stamped by the create path too, not
/// only by the feed upsert — "landing in done stamps" has one owner whatever
/// route the row took in. Latent today (every production caller passes
/// Backlog), and here so it stays closed.
#[tokio::test]
async fn create_task_stamps_a_task_created_straight_into_done() {
    let db = in_memory_db().await;
    let before = chrono::Utc::now() - chrono::Duration::seconds(1);
    let id = db
        .create_task(CreateTaskRequest {
            description: "d",
            status: TaskStatus::Done,
            ..CreateTaskRequest::fixture("Already finished", "/repo")
        })
        .await
        .unwrap();
    let after = chrono::Utc::now() + chrono::Duration::seconds(1);

    let completed_at = db
        .get_task(id)
        .await
        .unwrap()
        .unwrap()
        .completed_at
        .expect("a task created in Done carries a completion time");
    assert!(
        (before..=after).contains(&completed_at),
        "completed_at {completed_at} should be about now, within [{before}, {after}]"
    );
}

/// The complement: a create outside Done takes no stamp.
#[tokio::test]
async fn create_task_does_not_stamp_a_task_created_outside_done() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            description: "d",
            ..CreateTaskRequest::fixture("Open", "/repo")
        })
        .await
        .unwrap();

    assert_eq!(db.get_task(id).await.unwrap().unwrap().completed_at, None);
}

#[tokio::test]
async fn upsert_feed_tasks_does_not_stamp_a_task_inserted_outside_done() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();

    db.upsert_feed_tasks(
        epic.id,
        &[make_feed_item("ext-1", "Open")],
        &["/repo".to_string()],
        &main_branches(1),
    )
    .await
    .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks[0].completed_at, None);
}

#[tokio::test]
async fn upsert_feed_tasks_still_updates_sort_order_when_task_is_not_done() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![make_feed_item("ext-1", "Original Title")];

    db.upsert_feed_tasks(epic.id, &items, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    let mut updated_item = make_feed_item("ext-1", "Original Title");
    updated_item.sort_order = Some(7);
    db.upsert_feed_tasks(
        epic.id,
        &[updated_item],
        &["/repo".to_string()],
        &main_branches(1),
    )
    .await
    .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks[0].sort_order, Some(7));
}
