use super::*;

#[tokio::test]
async fn upsert_feed_tasks_adds_new_items() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();

    db.upsert_feed_tasks(
        epic.id,
        &[make_feed_item("ext-1", "First")],
        &["/repo".to_string()],
        &main_branches(1),
    )
    .await
    .unwrap();

    db.upsert_feed_tasks(
        epic.id,
        &[
            make_feed_item("ext-1", "First"),
            make_feed_item("ext-2", "Second"),
        ],
        &["/repo".to_string(), "/repo".to_string()],
        &main_branches(2),
    )
    .await
    .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 2, "new item should be created on second call");
}

#[tokio::test]
async fn upsert_feed_tasks_removes_stale_items() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();

    // First fetch: two items
    db.upsert_feed_tasks(
        epic.id,
        &[
            make_feed_item("ext-1", "First"),
            make_feed_item("ext-2", "Second"),
        ],
        &["/repo".to_string(), "/repo".to_string()],
        &main_branches(2),
    )
    .await
    .unwrap();
    assert_eq!(db.list_tasks_for_epic(epic.id).await.unwrap().len(), 2);

    // Second fetch: only ext-1 remains in the feed
    db.upsert_feed_tasks(
        epic.id,
        &[make_feed_item("ext-1", "First")],
        &["/repo".to_string()],
        &main_branches(1),
    )
    .await
    .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 1, "stale feed task should be removed");
    assert_eq!(tasks[0].external_id.as_deref(), Some("ext-1"));
}

/// The additive variant writes everything `upsert_feed_tasks` writes and deletes
/// nothing (`feeds.allium`: `DegradedNonEmptyEmission`). This is the DB half of
/// the partial-degradation guard: a tainted emission's omissions must not reach
/// a `DELETE`, so there is no row to report and nothing for the teardown fan-out
/// to destroy.
#[tokio::test]
async fn upsert_feed_tasks_additive_upserts_without_deleting_absent_tasks() {
    let db = in_memory_db().await;
    let epic = db.create_epic("Reviews", "", None).await.unwrap();

    db.upsert_feed_tasks(
        epic.id,
        &[
            make_feed_item("live-1", "Under review"),
            make_feed_item("keep-2", "Still open"),
        ],
        &vec!["/repo/a".to_string(); 2],
        &main_branches(2),
    )
    .await
    .unwrap();

    // `live-1` carries an agent's worktree — the row a stale delete would
    // report and the fan-out would then force-remove.
    let live = db
        .list_tasks_for_epic(epic.id)
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.external_id.as_deref() == Some("live-1"))
        .unwrap();
    db.patch_task(
        live.id,
        &TaskPatch::new().worktree(Some("/repo/a/.worktrees/live-1")),
    )
    .await
    .unwrap();

    // A partially degraded emission: `live-1` dropped out, one item is new.
    let removed = db
        .upsert_feed_tasks_additive(
            epic.id,
            &[
                make_feed_item("keep-2", "Still open, retitled"),
                make_feed_item("new-3", "Newly seen"),
            ],
            &vec!["/repo/a".to_string(); 2],
            &main_branches(2),
        )
        .await
        .unwrap();
    assert!(
        removed.is_empty(),
        "the additive variant reports nothing for teardown because it removes \
         nothing, got {removed:?}"
    );

    let left = db.list_tasks_for_epic(epic.id).await.unwrap();
    let ext = |t: &crate::models::Task| t.external_id.clone().unwrap_or_default();
    let mut ids: Vec<String> = left.iter().map(ext).collect();
    ids.sort();
    assert_eq!(
        ids,
        vec![
            "keep-2".to_string(),
            "live-1".to_string(),
            "new-3".to_string()
        ],
        "the omitted task must survive and the new one must be inserted"
    );

    // The omitted task keeps its state, so the agent in it is untouched.
    let survivor = left
        .iter()
        .find(|t| t.external_id.as_deref() == Some("live-1"))
        .unwrap();
    assert_eq!(
        survivor.worktree.as_deref(),
        Some("/repo/a/.worktrees/live-1")
    );

    // The present items are still refreshed — additive means "no removals",
    // not "no writes".
    let kept = left
        .iter()
        .find(|t| t.external_id.as_deref() == Some("keep-2"))
        .unwrap();
    assert_eq!(kept.title, "Still open, retitled");
}

/// An empty additive emission is a no-op, not a clear. The reconciling variant
/// treats `[]` as "delete everything"; the additive one must treat it as "learn
/// nothing", or the guard would leak the exact wipe it exists to prevent.
#[tokio::test]
async fn upsert_feed_tasks_additive_with_no_items_deletes_nothing() {
    let db = in_memory_db().await;
    let epic = db.create_epic("Reviews", "", None).await.unwrap();
    db.upsert_feed_tasks(
        epic.id,
        &[make_feed_item("ext-1", "One")],
        &["/repo/a".to_string()],
        &main_branches(1),
    )
    .await
    .unwrap();

    db.upsert_feed_tasks_additive(epic.id, &[], &[], &[])
        .await
        .unwrap();

    assert_eq!(
        db.list_tasks_for_epic(epic.id).await.unwrap().len(),
        1,
        "an empty additive emission must not clear the epic"
    );
}

/// A manual task (`external_id IS NULL`) is never deleted and never reported,
/// even when it carries a worktree.
#[tokio::test]
async fn delete_stale_subtree_feed_tasks_never_reports_manual_tasks() {
    let db = in_memory_db().await;
    let parent = db.create_epic("Reviews", "", None).await.unwrap();
    let sub = db
        .create_epic("My Reviews", "", Some(parent.id))
        .await
        .unwrap();

    let manual = db
        .create_task(CreateTaskRequest {
            epic_id: Some(sub.id),
            ..CreateTaskRequest::fixture("Manual", "/repo/a")
        })
        .await
        .unwrap();
    db.patch_task(
        manual,
        &TaskPatch::new().worktree(Some("/repo/a/.worktrees/manual")),
    )
    .await
    .unwrap();

    let removed = db
        .delete_stale_subtree_feed_tasks(parent.id, &[])
        .await
        .unwrap();

    assert!(
        removed.is_empty(),
        "manual tasks are neither deleted nor reported, got {removed:?}"
    );
    assert_eq!(db.list_tasks_for_epic(sub.id).await.unwrap().len(), 1);
}

/// `delete_stale_subtree_feed_tasks` deletes feed tasks (external_id set) across
/// the WHOLE subtree of a parent epic, except those in the keep-set. It must:
/// - keep a feed task whose external_id is in the keep-set (even in another child);
/// - delete a feed task absent from the keep-set;
/// - preserve manual tasks (external_id IS NULL) regardless of the keep-set.
#[tokio::test]
async fn delete_stale_subtree_feed_tasks_scopes_to_subtree_and_keeps_set() {
    let db = in_memory_db().await;
    let parent = db.create_epic("Reviews", "", None).await.unwrap();
    let child_a = db.create_epic("A", "", Some(parent.id)).await.unwrap();
    let child_b = db.create_epic("B", "", Some(parent.id)).await.unwrap();

    // Feed tasks in both children.
    db.upsert_feed_tasks(
        child_a.id,
        &[make_feed_item("keep-1", "Kept")],
        &["/repo".to_string()],
        &main_branches(1),
    )
    .await
    .unwrap();
    db.upsert_feed_tasks(
        child_b.id,
        &[make_feed_item("stale-1", "Stale")],
        &["/repo".to_string()],
        &main_branches(1),
    )
    .await
    .unwrap();

    // A manual task (no external_id) in child_a must survive.
    let manual_id = db
        .create_task(CreateTaskRequest {
            epic_id: Some(child_a.id),
            ..CreateTaskRequest::fixture("Manual", "/repo")
        })
        .await
        .unwrap();

    db.delete_stale_subtree_feed_tasks(parent.id, &["keep-1".to_string()])
        .await
        .unwrap();

    let a_tasks = db.list_tasks_for_epic(child_a.id).await.unwrap();
    assert_eq!(a_tasks.len(), 2, "kept feed task + manual task survive");
    assert!(a_tasks
        .iter()
        .any(|t| t.external_id.as_deref() == Some("keep-1")));
    assert!(a_tasks.iter().any(|t| t.id == manual_id));

    let b_tasks = db.list_tasks_for_epic(child_b.id).await.unwrap();
    assert!(
        b_tasks.is_empty(),
        "stale feed task absent from keep-set is deleted, got {b_tasks:?}"
    );
}
