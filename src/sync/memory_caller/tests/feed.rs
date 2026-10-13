use super::*;

// -- Feed ingestion -----------------------------------------------------------

fn feed_item(external_id: &str) -> bindings::FeedTaskUpsertItem {
    bindings::FeedTaskUpsertItem {
        external_id: external_id.into(),
        title: "feed task".into(),
        description: String::new(),
        repo_path: "/repo".into(),
        status: "backlog".into(),
        sub_status: "none".into(),
        base_branch: "main".into(),
        tag: String::new(),
        labels: "[]".into(),
        sort_order: None,
        url: String::new(),
        url_type: String::new(),
        wrap_up_mode: String::new(),
    }
}

#[tokio::test]
async fn upsert_feed_tasks_inserts_updates_and_removes_stale_items() {
    let (caller, rows) = caller();
    let epic_id = caller.create_epic(blank_epic()).await.unwrap();

    caller
        .upsert_feed_tasks(
            epic_id,
            vec![feed_item("a"), feed_item("b")],
            "tester".into(),
        )
        .await
        .unwrap();
    assert_eq!(rows.tasks_for_epic(epic_id).len(), 2);

    // Re-upsert with only "a": "b" is stale and removed. "a" must be
    // `done` first, or the reducer's own conformance with `delete_task`'s
    // guard would refuse to remove it — but a feed's stale-delete is a
    // raw row delete, not a guarded `delete_task`, so it removes "b"
    // regardless of status.
    caller
        .upsert_feed_tasks(epic_id, vec![feed_item("a")], "tester".into())
        .await
        .unwrap();
    let remaining = rows.tasks_for_epic(epic_id);
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].external_id.as_deref(), Some("a"));
}

#[tokio::test]
async fn upsert_feed_tasks_refuses_an_unknown_epic() {
    let (caller, _rows) = caller();
    let outcome = caller
        .upsert_feed_tasks(EpicId(999), vec![feed_item("a")], "tester".into())
        .await
        .unwrap();
    assert!(!outcome.won());
}

#[tokio::test]
async fn create_repo_group_sub_epic_is_find_or_create() {
    let (caller, rows) = caller();
    let parent_id = caller.create_epic(blank_epic()).await.unwrap();
    let first = caller
        .create_repo_group_sub_epic(parent_id, "repo-a".into(), "tester".into())
        .await
        .unwrap();
    let second = caller
        .create_repo_group_sub_epic(parent_id, "repo-a".into(), "someone-else".into())
        .await
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(rows.epics_with_parent(Some(parent_id)).len(), 1);
}

#[tokio::test]
async fn create_managed_role_epic_is_find_or_create() {
    let (caller, rows) = caller();
    let parent_id = caller.create_epic(blank_epic()).await.unwrap();
    let first = caller
        .create_managed_role_epic(
            "reviewer".into(),
            Some(parent_id),
            "review".into(),
            "cmd".into(),
            60,
            "tester".into(),
        )
        .await
        .unwrap();
    let second = caller
        .create_managed_role_epic(
            "reviewer".into(),
            Some(parent_id),
            "review".into(),
            "cmd".into(),
            60,
            "tester".into(),
        )
        .await
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(rows.epics_with_parent(Some(parent_id)).len(), 1);
}
