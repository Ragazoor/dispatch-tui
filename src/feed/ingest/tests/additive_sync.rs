use super::*;

// --- SyncMode::Additive (feeds.allium: DegradedNonEmptyEmission) ---
//
// One test per removal mechanism, because they are three separate code paths
// that happen to share an outcome: the role-routed subtree delete, the parent
// sweep, the grouped absent-sub-epic clear, and the flat/per-epic stale delete
// inside the module's upsert. A single end-to-end test would leave three of them
// free to regress independently.

/// The role-routed subtree delete is skipped: a PR omitted by a partially
/// degraded emission keeps its row AND its agent state, and is not reported for
/// teardown. This is the exact scenario #4095 exists for — `pr-1` is a live
/// review agent whose PR one soft-failed sub-query dropped.
#[tokio::test]
async fn additive_role_routed_sync_keeps_a_task_absent_from_the_emission() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();
    db.patch_epic(
        parent.id,
        &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
    )
    .await
    .unwrap();

    let cycle1 = vec![
        make_signal_item(
            "pr-1",
            "https://github.com/org/repo/pull/1",
            vec![Signal::DirectRequest],
        ),
        make_signal_item(
            "pr-2",
            "https://github.com/org/repo/pull/2",
            vec![Signal::DirectRequest],
        ),
    ];
    run_role_routed_feed_sync(
        &*db,
        parent.id,
        entries(&cycle1, &["", ""], &["main", "main"]),
    )
    .await
    .unwrap();

    let my = role_sub_epic(&db, parent.id, FeedRole::MyReviews).await;
    let live = db
        .list_tasks_for_epic(my)
        .await
        .into_iter()
        .find(|t| t.external_id.as_deref() == Some("pr-1"))
        .unwrap();
    db.patch_task(
        live.id,
        &TaskPatch::new()
            .status(TaskStatus::Running)
            .sub_status(crate::models::SubStatus::Active)
            .worktree(Some("/tmp/wt-pr-1"))
            .tmux_window(Some(&test_tmux_window("dispatch:7"))),
    )
    .await
    .unwrap();

    // Cycle 2: a degraded emission that lost pr-1.
    let cycle2 = vec![make_signal_item(
        "pr-2",
        "https://github.com/org/repo/pull/2",
        vec![Signal::DirectRequest],
    )];
    let outcome = run_role_routed_feed_sync_with_mode(
        &*db,
        parent.id,
        entries(&cycle2, &[""], &["main"]),
        SyncMode::Additive,
    )
    .await
    .unwrap();

    let tasks = db.list_tasks_for_epic(my).await;
    let survivor = tasks
        .iter()
        .find(|t| t.external_id.as_deref() == Some("pr-1"))
        .expect("a task omitted by a degraded emission must not be deleted");
    assert_eq!(survivor.worktree.as_deref(), Some("/tmp/wt-pr-1"));
    assert_eq!(survivor.status, TaskStatus::Running);
    assert!(
        outcome.removed.is_empty(),
        "an additive sync reports nothing for teardown, got {:?}",
        outcome.removed
    );
}

/// The parent sweep is skipped too. It deletes on position alone — every feed
/// task sitting directly on the reviews_parent — so it would wipe a stranded
/// task on a degraded cycle without ever consulting the emission.
#[tokio::test]
async fn additive_role_routed_sync_keeps_parent_stranded_tasks() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();

    // A feed task stranded flat on the parent, absent from the next emission.
    db.ingest_feed(
        parent.id,
        &[make_item("stranded", "https://github.com/org/repo/pull/9")],
        &["".to_string()],
        &["main".to_string()],
        false,
    )
    .await
    .unwrap();
    // The role is set only after the seed: ingest routes a role-carrying epic, so a flat task on the parent can only be planted while it has no role.
    db.patch_epic(
        parent.id,
        &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
    )
    .await
    .unwrap();

    let items = vec![make_signal_item(
        "pr-2",
        "https://github.com/org/repo/pull/2",
        vec![Signal::DirectRequest],
    )];
    run_role_routed_feed_sync_with_mode(
        &*db,
        parent.id,
        entries(&items, &[""], &["main"]),
        SyncMode::Additive,
    )
    .await
    .unwrap();

    let on_parent = db.list_tasks_for_epic(parent.id).await;
    assert_eq!(
        on_parent.len(),
        1,
        "the parent sweep must not run on a degraded cycle, got {on_parent:?}"
    );
    assert_eq!(on_parent[0].external_id.as_deref(), Some("stranded"));
}

/// Additive is "no removals", not "no writes": a cross-role MOVE still happens,
/// because it follows from what the emission CONTAINS. Withholding moves would
/// make a degraded cycle actively wrong rather than merely conservative.
#[tokio::test]
async fn additive_role_routed_sync_still_moves_and_inserts() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();

    let cycle1 = vec![make_signal_item(
        "pr-1",
        "https://github.com/org/repo/pull/1",
        vec![Signal::TeamRequest],
    )];
    run_role_routed_feed_sync(&*db, parent.id, entries(&cycle1, &[""], &["main"]))
        .await
        .unwrap();

    let cycle2 = vec![
        make_signal_item(
            "pr-1",
            "https://github.com/org/repo/pull/1",
            vec![Signal::Reviewed],
        ),
        make_signal_item(
            "pr-new",
            "https://github.com/org/repo/pull/5",
            vec![Signal::DirectRequest],
        ),
    ];
    run_role_routed_feed_sync_with_mode(
        &*db,
        parent.id,
        entries(&cycle2, &["", ""], &["main", "main"]),
        SyncMode::Additive,
    )
    .await
    .unwrap();

    let my = role_sub_epic(&db, parent.id, FeedRole::MyReviews).await;
    let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
    let my_ids: Vec<String> = db
        .list_tasks_for_epic(my)
        .await
        .into_iter()
        .filter_map(|t| t.external_id)
        .collect();
    assert!(my_ids.contains(&"pr-1".to_string()), "move still applied");
    assert!(
        my_ids.contains(&"pr-new".to_string()),
        "first-sight insert still applied"
    );
    assert!(
        db.list_tasks_for_epic(team).await.is_empty(),
        "the moved task left its old role sub-epic, and nothing was deleted to \
         achieve that"
    );
}

/// The grouped path's two clearing passes — absent sub-epics and the parent's
/// flat tasks — are both skipped.
#[tokio::test]
async fn additive_grouped_sync_keeps_tasks_in_a_dropped_repo_sub_epic() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();

    let items = vec![
        make_item("1", "https://github.com/org/repo-a/pull/1"),
        make_item("2", "https://github.com/org/repo-b/pull/1"),
    ];
    sync_grouped_feed(
        &*db,
        parent.id,
        entries(&items, &["", ""], &["main", "main"]),
    )
    .await;

    // Degraded cycle: repo-b's query soft-failed, so only repo-a is emitted.
    let degraded = vec![make_item("1", "https://github.com/org/repo-a/pull/1")];
    let outcome = sync_grouped_feed_with_mode(
        &*db,
        parent.id,
        entries(&degraded, &[""], &["main"]),
        SyncMode::Additive,
    )
    .await;

    let subs = db.list_sub_epics(parent.id).await;
    let repo_b = subs.iter().find(|e| e.title == "repo-b").unwrap();
    assert_eq!(
        db.list_tasks_for_epic(repo_b.id).await.len(),
        1,
        "a repo absent from a degraded emission keeps its tasks"
    );
    assert!(outcome.removed.is_empty());
}

/// The flat path's stale delete lives inside the module's upsert, so it is
/// gated by the additive variant of the ingest call rather than by a skipped step.
#[tokio::test]
async fn additive_flat_sync_keeps_a_task_absent_from_the_emission() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let epic = db.create_epic("CVE", "", None).await.unwrap();

    let items = vec![make_item("cve-1", ""), make_item("cve-2", "")];
    run_feed_sync(
        &*db,
        epic.id,
        false,
        entries(&items, &["", ""], &["main", "main"]),
    )
    .await
    .unwrap();

    let degraded = vec![make_item("cve-2", "")];
    let outcome = run_feed_sync_with_mode(
        &*db,
        epic.id,
        false,
        entries(&degraded, &[""], &["main"]),
        SyncMode::Additive,
    )
    .await
    .unwrap();

    let ids: Vec<String> = db
        .list_tasks_for_epic(epic.id)
        .await
        .into_iter()
        .filter_map(|t| t.external_id)
        .collect();
    assert_eq!(ids.len(), 2, "nothing removed on a degraded flat sync");
    assert!(ids.contains(&"cve-1".to_string()));
    assert!(outcome.removed.is_empty());
}
