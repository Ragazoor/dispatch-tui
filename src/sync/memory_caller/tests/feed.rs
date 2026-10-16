use super::*;
use crate::models::EpicOrigin;

// -- Feed ingestion -----------------------------------------------------------

fn feed_item(external_id: &str) -> bindings::FeedIngestItem {
    bindings::FeedIngestItem {
        signals: vec![],
        item: bindings::FeedTaskUpsertItem {
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
        },
    }
}

#[tokio::test]
async fn ingest_feed_inserts_updates_and_removes_stale_items() {
    let (caller, rows) = caller();
    let epic_id = caller.create_epic(blank_epic()).await.unwrap();

    caller
        .ingest_feed(
            epic_id,
            vec![feed_item("a"), feed_item("b")],
            false,
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
        .ingest_feed(epic_id, vec![feed_item("a")], false, "tester".into())
        .await
        .unwrap();
    let remaining = rows.tasks_for_epic(epic_id);
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].external_id.as_deref(), Some("a"));
}

#[tokio::test]
async fn ingest_feed_refuses_an_unknown_epic() {
    let (caller, _rows) = caller();
    let outcome = caller
        .ingest_feed(EpicId(999), vec![feed_item("a")], false, "tester".into())
        .await
        .unwrap();
    assert!(!outcome.won());
}

fn feed_item_at(external_id: &str, url: &str) -> bindings::FeedIngestItem {
    let mut entry = feed_item(external_id);
    entry.item.url = url.into();
    entry.item.url_type = "pr".into();
    entry
}

/// `GroupedFeedUpsert` creates each repo sub-epic with origin `repo-group`,
/// so that un-grouping the feed (`FlatFeedReconcile`) can find and flatten it.
#[tokio::test]
async fn grouped_ingest_creates_repo_group_sub_epics_that_flat_ingest_flattens() {
    let (caller, rows) = caller();
    let epic_id = caller
        .create_epic(bindings::Epic {
            group_by_repo: true,
            ..blank_epic()
        })
        .await
        .unwrap();
    let items = || vec![feed_item_at("a", "https://github.com/org/alpha/pull/1")];

    caller
        .ingest_feed(epic_id, items(), false, "tester".into())
        .await
        .unwrap();
    let subs = rows.epics_with_parent(Some(epic_id));
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].origin, EpicOrigin::RepoGroup);

    caller
        .patch_epic(
            epic_id,
            bindings::EpicPatch {
                group_by_repo: Some(false),
                ..blank_epic_patch()
            },
        )
        .await
        .unwrap();
    caller
        .ingest_feed(epic_id, items(), false, "tester".into())
        .await
        .unwrap();

    assert!(rows.epics_with_parent(Some(epic_id)).is_empty());
    let tasks = rows.tasks_for_epic(epic_id);
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].external_id.as_deref(), Some("a"));
}
