use super::*;

#[tokio::test]
async fn upsert_feed_tasks_uses_resolved_repo_path() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![make_feed_item("ext-1", "Task One")];
    let repo_paths = vec!["/resolved/local/repo".to_string()];
    let branches = main_branches(items.len());

    db.upsert_feed_tasks(epic.id, &items, &repo_paths, &branches)
        .await
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks[0].repo_path, "/resolved/local/repo");
}

#[tokio::test]
async fn upsert_feed_tasks_stores_empty_sentinel_when_unresolved() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![make_feed_item("ext-1", "Task One")];
    let repo_paths = vec!["".to_string()];
    let branches = main_branches(items.len());

    db.upsert_feed_tasks(epic.id, &items, &repo_paths, &branches)
        .await
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks[0].repo_path, "");
}

#[tokio::test]
async fn upsert_feed_tasks_on_conflict_does_not_update_repo_path() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![make_feed_item("ext-1", "Original")];

    // First upsert: resolved path stored
    db.upsert_feed_tasks(
        epic.id,
        &items,
        &["/first/path".to_string()],
        &main_branches(1),
    )
    .await
    .unwrap();
    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks[0].repo_path, "/first/path");

    // Second upsert: different path provided — ON CONFLICT should NOT update repo_path
    let updated = vec![crate::models::FeedItem {
        external_id: "ext-1".to_string(),
        title: "Updated Title".to_string(),
        description: "new desc".to_string(),
        url: String::new(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::Bug,
        labels: Vec::new(),
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    }];
    db.upsert_feed_tasks(
        epic.id,
        &updated,
        &["/second/path".to_string()],
        &main_branches(1),
    )
    .await
    .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks[0].title, "Updated Title");
    assert_eq!(
        tasks[0].repo_path, "/first/path",
        "repo_path must not be updated on conflict"
    );
}

#[tokio::test]
async fn upsert_feed_tasks_mixed_batch_resolved_and_unresolved() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![
        make_feed_item("ext-1", "Resolved Task"),
        make_feed_item("ext-2", "Unresolved Task"),
    ];
    let repo_paths = vec!["/matched/local/path".to_string(), "".to_string()];
    let branches = main_branches(items.len());

    db.upsert_feed_tasks(epic.id, &items, &repo_paths, &branches)
        .await
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    let resolved = tasks
        .iter()
        .find(|t| t.external_id.as_deref() == Some("ext-1"))
        .unwrap();
    let unresolved = tasks
        .iter()
        .find(|t| t.external_id.as_deref() == Some("ext-2"))
        .unwrap();
    assert_eq!(resolved.repo_path, "/matched/local/path");
    assert_eq!(unresolved.repo_path, "");
}

#[tokio::test]
async fn upsert_feed_tasks_stores_per_task_base_branch() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![
        make_feed_item("ext-1", "Master Task"),
        make_feed_item("ext-2", "Develop Task"),
        make_feed_item("ext-3", "Main Task"),
    ];
    let repo_paths = vec![
        "/repo-a".to_string(),
        "/repo-b".to_string(),
        "/repo-c".to_string(),
    ];
    let base_branches = vec![
        "master".to_string(),
        "develop".to_string(),
        "main".to_string(),
    ];

    db.upsert_feed_tasks(epic.id, &items, &repo_paths, &base_branches)
        .await
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    let by_ext = |ext: &str| {
        tasks
            .iter()
            .find(|t| t.external_id.as_deref() == Some(ext))
            .unwrap()
    };
    assert_eq!(by_ext("ext-1").base_branch, "master");
    assert_eq!(by_ext("ext-2").base_branch, "develop");
    assert_eq!(by_ext("ext-3").base_branch, "main");
}

#[tokio::test]
async fn upsert_feed_tasks_does_not_remove_manual_tasks() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();

    // Manually created task linked to the epic (no external_id)
    let manual_task_id = db
        .create_task(CreateTaskRequest {
            title: "Manual",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: Some(epic.id),
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();

    // Feed fetch with one item
    db.upsert_feed_tasks(
        epic.id,
        &[make_feed_item("ext-1", "Feed Task")],
        &["/repo".to_string()],
        &main_branches(1),
    )
    .await
    .unwrap();

    // Feed fetch returns nothing — only manual task should survive
    db.upsert_feed_tasks(epic.id, &[], &[], &[]).await.unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(
        tasks.len(),
        1,
        "manual task should survive empty feed fetch"
    );
    assert_eq!(tasks[0].id, manual_task_id);
}

#[tokio::test]
async fn upsert_feed_tasks_persists_tag() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![crate::models::FeedItem {
        external_id: "ext-1".to_string(),
        title: "Tagged".to_string(),
        description: "".to_string(),
        url: String::new(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::PrReview,
        labels: Vec::new(),
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    }];

    db.upsert_feed_tasks(epic.id, &items, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].tag, Some(crate::models::TaskTag::PrReview));
}

#[tokio::test]
async fn upsert_feed_tasks_updates_tag_on_conflict() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let initial = vec![crate::models::FeedItem {
        external_id: "ext-1".to_string(),
        title: "T".to_string(),
        description: "".to_string(),
        url: String::new(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::PrReview,
        labels: Vec::new(),
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    }];
    db.upsert_feed_tasks(epic.id, &initial, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    // Re-emit the same item with a different tag — feed is the source of truth.
    let updated = vec![crate::models::FeedItem {
        external_id: "ext-1".to_string(),
        title: "T".to_string(),
        description: "".to_string(),
        url: String::new(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::Fix,
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
    assert_eq!(tasks[0].tag, Some(crate::models::TaskTag::Fix));
}
