use super::*;

// --- group_by_repo on role sub-epics ---

/// When a role sub-epic has `group_by_repo = true`, feed items must be
/// routed into per-repo sub-epics rather than into the role sub-epic directly.
#[tokio::test]
async fn role_routed_group_by_repo_routes_into_repo_sub_epic() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();
    db.patch_epic(
        parent.id,
        &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
    )
    .await
    .unwrap();

    // First cycle — creates role sub-epics.
    let items1 = vec![make_signal_item(
        "pr-1",
        "https://github.com/org/myrepo/pull/1",
        vec![Signal::TeamRequest],
    )];
    run_role_routed_feed_sync(&*db, parent.id, entries(&items1, &[""], &["main"]))
        .await
        .unwrap();

    // Enable group_by_repo on Team Reviews.
    let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
    db.patch_epic(team, &EpicPatch::new().group_by_repo(true))
        .await
        .unwrap();

    // Second cycle — same PR. Should now land in a repo-group sub-epic.
    run_role_routed_feed_sync(&*db, parent.id, entries(&items1, &[""], &["main"]))
        .await
        .unwrap();

    let team_direct = db.list_tasks_for_epic(team).await;
    assert!(
        team_direct.is_empty(),
        "Team Reviews must have no direct tasks when group_by_repo is active"
    );

    let repo_subs = db.list_sub_epics(team).await;
    assert_eq!(
        repo_subs.len(),
        1,
        "one repo-group sub-epic under Team Reviews"
    );
    assert_eq!(repo_subs[0].title, "myrepo");
    let repo_tasks = db.list_tasks_for_epic(repo_subs[0].id).await;
    assert_eq!(repo_tasks.len(), 1, "PR landed in the repo-group sub-epic");
    assert_eq!(repo_tasks[0].external_id.as_deref(), Some("pr-1"));
}

/// Re-running the feed when group_by_repo is active must not create
/// duplicate tasks in the role sub-epic — the `existing` map must reach
/// into repo-group sub-epics so the PR is recognised as already present.
#[tokio::test]
async fn role_routed_group_by_repo_no_duplicate_on_resync() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();
    db.patch_epic(
        parent.id,
        &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
    )
    .await
    .unwrap();

    let items = vec![make_signal_item(
        "pr-1",
        "https://github.com/org/myrepo/pull/1",
        vec![Signal::TeamRequest],
    )];

    // First cycle — creates role sub-epics.
    run_role_routed_feed_sync(&*db, parent.id, entries(&items, &[""], &["main"]))
        .await
        .unwrap();

    let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
    db.patch_epic(team, &EpicPatch::new().group_by_repo(true))
        .await
        .unwrap();

    // Second cycle — lands in repo-group sub-epic.
    run_role_routed_feed_sync(&*db, parent.id, entries(&items, &[""], &["main"]))
        .await
        .unwrap();

    // Third cycle — must not duplicate.
    run_role_routed_feed_sync(&*db, parent.id, entries(&items, &[""], &["main"]))
        .await
        .unwrap();

    let repo_subs = db.list_sub_epics(team).await;
    assert_eq!(repo_subs.len(), 1);
    let tasks = db.list_tasks_for_epic(repo_subs[0].id).await;
    assert_eq!(tasks.len(), 1, "exactly one task after three cycles");
    assert!(
        db.list_tasks_for_epic(team).await.is_empty(),
        "no duplicate in role sub-epic"
    );
}

/// When a PR disappears from the feed and group_by_repo is active, the
/// stale deletion must reach into the repo-group sub-epic grandchildren
/// and remove the task.
#[tokio::test]
async fn role_routed_group_by_repo_stale_deletion_reaches_grandchildren() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();
    db.patch_epic(
        parent.id,
        &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
    )
    .await
    .unwrap();

    let items = vec![make_signal_item(
        "pr-1",
        "https://github.com/org/myrepo/pull/1",
        vec![Signal::TeamRequest],
    )];

    // First cycle — creates role sub-epics.
    run_role_routed_feed_sync(&*db, parent.id, entries(&items, &[""], &["main"]))
        .await
        .unwrap();

    let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
    db.patch_epic(team, &EpicPatch::new().group_by_repo(true))
        .await
        .unwrap();

    // Second cycle — PR lands in repo-group sub-epic.
    run_role_routed_feed_sync(&*db, parent.id, entries(&items, &[""], &["main"]))
        .await
        .unwrap();

    let repo_subs = db.list_sub_epics(team).await;
    assert_eq!(
        db.list_tasks_for_epic(repo_subs[0].id).await.len(),
        1,
        "task present before stale deletion cycle"
    );

    // Third cycle — PR absent (merged/closed).
    run_role_routed_feed_sync(&*db, parent.id, entries(&[], &[], &[]))
        .await
        .unwrap();

    let tasks_after = db.list_tasks_for_epic(repo_subs[0].id).await;
    assert!(
        tasks_after.is_empty(),
        "stale PR must be removed from repo-group sub-epic"
    );
}

#[tokio::test]
async fn items_grouped_by_repo_name() {
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

    let subs = db.list_sub_epics(parent.id).await;
    assert_eq!(subs.len(), 2);
    let names: Vec<&str> = subs.iter().map(|e| e.title.as_str()).collect();
    assert!(names.contains(&"repo-a"), "got {names:?}");
    assert!(names.contains(&"repo-b"), "got {names:?}");

    for sub in &subs {
        let tasks = db.list_tasks_for_epic(sub.id).await;
        assert_eq!(tasks.len(), 1, "sub-epic {} should have 1 task", sub.title);
    }

    let parent_tasks = db.list_tasks_for_epic(parent.id).await;
    assert_eq!(parent_tasks.len(), 0, "parent should have no direct tasks");
}

#[tokio::test]
async fn no_url_groups_as_other() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();

    let items = vec![FeedItem {
        external_id: "x".into(),
        title: "X".into(),
        description: String::new(),
        url: String::new(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: TaskTag::Bug,
        labels: vec![],
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    }];

    sync_grouped_feed(&*db, parent.id, entries(&items, &[""], &["main"])).await;

    let subs = db.list_sub_epics(parent.id).await;
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].title, "other");
}

#[tokio::test]
async fn existing_active_sub_epic_reused() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();

    // Pre-create the sub-epic as active.
    let pre_existing = db.create_epic("repo-a", "", Some(parent.id)).await.unwrap();

    let items = vec![make_item("1", "https://github.com/org/repo-a/pull/1")];

    let outcome = sync_grouped_feed(&*db, parent.id, entries(&items, &[""], &["main"])).await;
    // affected_epics leads with the parent; this assertion is about sub-epics.
    let sub_ids: Vec<_> = outcome
        .affected_epics
        .iter()
        .copied()
        .filter(|id| *id != parent.id)
        .collect();

    assert_eq!(
        sub_ids,
        vec![pre_existing.id],
        "should reuse existing active sub-epic"
    );
    let subs = db.list_sub_epics(parent.id).await;
    assert_eq!(subs.len(), 1, "no duplicate sub-epic should be created");
}

#[tokio::test]
async fn run_feed_sync_flat_upserts_to_parent_epic() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let epic = db.create_epic("Feed", "", None).await.unwrap();
    let items = vec![crate::models::FeedItem {
        external_id: "1".into(),
        title: "T".into(),
        description: String::new(),
        url: String::new(),
        url_type: None,
        status: crate::models::TaskStatus::Backlog,
        tag: crate::models::TaskTag::Bug,
        labels: vec![],
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    }];

    let outcome = run_feed_sync(&*db, epic.id, false, entries(&items, &[""], &["main"]))
        .await
        .unwrap();

    assert_eq!(outcome.affected_epics, vec![epic.id]);
    let tasks = db.list_tasks_for_epic(epic.id).await;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].external_id.as_deref(), Some("1"));
}

#[tokio::test]
async fn run_feed_sync_grouped_puts_tasks_in_sub_epics() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let epic = db.create_epic("Reviews", "", None).await.unwrap();
    let items = vec![crate::models::FeedItem {
        external_id: "pr-1".into(),
        title: "PR 1".into(),
        description: String::new(),
        url: "https://github.com/org/repo-a/pull/1".into(),
        url_type: None,
        status: crate::models::TaskStatus::Backlog,
        tag: crate::models::TaskTag::PrReview,
        labels: vec![],
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    }];

    let outcome = run_feed_sync(&*db, epic.id, true, entries(&items, &[""], &["main"]))
        .await
        .unwrap();

    assert!(outcome.affected_epics.contains(&epic.id));
    assert_eq!(outcome.affected_epics.len(), 2, "parent id + 1 sub-epic id");

    let parent_tasks = db.list_tasks_for_epic(epic.id).await;
    assert_eq!(parent_tasks.len(), 0, "parent should have no direct tasks");

    let sub_epics = db.list_sub_epics(epic.id).await;
    assert_eq!(sub_epics.len(), 1);
    assert_eq!(sub_epics[0].title, "repo-a");
    let sub_tasks = db.list_tasks_for_epic(sub_epics[0].id).await;
    assert_eq!(sub_tasks.len(), 1);
}

/// An empty emission must clear feed tasks from EVERY active sub-epic —
/// the feed is the source of truth for the whole grouped subtree, not just
/// the repos present in the current batch. The sub-epic rows themselves
/// remain (not auto-deleted).
#[tokio::test]
async fn sync_grouped_feed_empty_emission_clears_all_sub_epics() {
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

    assert_eq!(db.list_sub_epics(parent.id).await.len(), 2);

    // Second cycle: the feed now returns nothing.
    sync_grouped_feed(&*db, parent.id, vec![]).await;

    let subs = db.list_sub_epics(parent.id).await;
    assert_eq!(
        subs.len(),
        2,
        "sub-epic rows remain, only their tasks clear"
    );
    for sub in &subs {
        let tasks = db.list_tasks_for_epic(sub.id).await;
        assert_eq!(
            tasks.len(),
            0,
            "sub-epic {} should have no feed tasks after empty emission",
            sub.title
        );
    }
}

/// A partial emission clears only the sub-epics whose repo dropped out;
/// repos still present keep their tasks.
#[tokio::test]
async fn sync_grouped_feed_partial_emission_clears_dropped_repo() {
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

    // Second cycle: only repo-a still has an open item.
    let items2 = vec![make_item("1", "https://github.com/org/repo-a/pull/1")];
    sync_grouped_feed(&*db, parent.id, entries(&items2, &[""], &["main"])).await;

    let subs = db.list_sub_epics(parent.id).await;
    let repo_a = subs.iter().find(|e| e.title == "repo-a").unwrap();
    let repo_b = subs.iter().find(|e| e.title == "repo-b").unwrap();
    assert_eq!(
        db.list_tasks_for_epic(repo_a.id).await.len(),
        1,
        "repo-a still in feed, task kept"
    );
    assert_eq!(
        db.list_tasks_for_epic(repo_b.id).await.len(),
        0,
        "repo-b dropped out, task cleared"
    );
}

/// When group_by_repo is toggled OFF, the next feed cycle must move tasks
/// from the orphaned repo-group sub-epic onto the role sub-epic — no duplicate.
#[tokio::test]
async fn role_routed_group_by_repo_off_rehomes_repo_tasks_no_duplicate() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();
    db.patch_epic(
        parent.id,
        &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
    )
    .await
    .unwrap();

    let items = vec![make_signal_item(
        "pr-1",
        "https://github.com/org/myrepo/pull/1",
        vec![Signal::TeamRequest],
    )];

    // Cycle 1 — team_reviews has group_by_repo OFF, task lands flat.
    run_role_routed_feed_sync(&*db, parent.id, entries(&items, &[""], &["main"]))
        .await
        .unwrap();

    let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;

    // Enable group_by_repo → cycle 2 moves task into repo sub-epic.
    db.patch_epic(team, &EpicPatch::new().group_by_repo(true))
        .await
        .unwrap();
    run_role_routed_feed_sync(&*db, parent.id, entries(&items, &[""], &["main"]))
        .await
        .unwrap();

    let repo_subs = db.list_sub_epics(team).await;
    assert_eq!(repo_subs.len(), 1);
    assert_eq!(db.list_tasks_for_epic(repo_subs[0].id).await.len(), 1);
    assert!(db.list_tasks_for_epic(team).await.is_empty());

    // Disable group_by_repo → cycle 3 must re-home task to role sub-epic, no duplicate.
    db.patch_epic(team, &EpicPatch::new().group_by_repo(false))
        .await
        .unwrap();
    run_role_routed_feed_sync(&*db, parent.id, entries(&items, &[""], &["main"]))
        .await
        .unwrap();

    let team_tasks = db.list_tasks_for_epic(team).await;
    assert_eq!(
        team_tasks.len(),
        1,
        "exactly one task on role sub-epic after toggle-off cycle"
    );
    assert_eq!(team_tasks[0].external_id.as_deref(), Some("pr-1"));

    // No tasks remain in any repo-group sub-epic.
    for sub in db.list_sub_epics(team).await {
        assert!(
            db.list_tasks_for_epic(sub.id).await.is_empty(),
            "repo-group sub-epic {} must be empty after group_by_repo turned off",
            sub.title
        );
    }
}

/// Clearing a dropped sub-epic removes only feed tasks (external_id set);
/// a manually-added task (external_id = null) in that sub-epic survives.
#[tokio::test]
async fn sync_grouped_feed_preserves_manual_task_in_dropped_sub_epic() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();

    let items = vec![make_item("1", "https://github.com/org/repo-a/pull/1")];
    sync_grouped_feed(&*db, parent.id, entries(&items, &[""], &["main"])).await;

    let subs = db.list_sub_epics(parent.id).await;
    let repo_a = subs.iter().find(|e| e.title == "repo-a").unwrap();

    // A manual task the user added under the repo sub-epic (no external_id).
    let manual_id = create_manual_task(&db, "Manual", repo_a.id).await;

    // Empty emission clears the feed task but must spare the manual one.
    sync_grouped_feed(&*db, parent.id, vec![]).await;

    let tasks = db.list_tasks_for_epic(repo_a.id).await;
    assert_eq!(tasks.len(), 1, "only the manual task survives");
    assert_eq!(tasks[0].id, manual_id);
}

// --- FlatFeedReconcile (feeds.allium) ---
//
// Toggling group_by_repo OFF on a feed epic only flips the flag; these
// tests cover the flat sync path's reconciliation of pre-existing
// RepoGroup sub-epics (docs/specs/feeds.allium: FlatFeedReconcile).

/// A feed epic with group_by_repo=false and an existing active RepoGroup
/// sub-epic: the flat sync path re-homes the sub-epic's task back to the
/// parent (as the SAME row, not a delete+recreate) and deletes the
/// now-empty sub-epic, then upserts the current emission onto the parent.
#[tokio::test]
async fn flat_sync_rehomes_tasks_from_existing_repo_group_subepic_and_deletes_it() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("CVE", "", None).await.unwrap();

    // Simulate a pre-existing grouped state: a RepoGroup sub-epic holding
    // a feed task, as if group_by_repo had been on for a prior poll.
    // `regroup_epic` is what makes a `repo-group` sub-epic here: the module's
    // grouped ingest names its sub-epics but does not stamp that origin.
    let seed = vec![make_item("cve-1", "https://github.com/org/myrepo/pull/1")];
    db.ingest_feed(
        parent.id,
        &seed,
        &["/repos/myrepo".into()],
        &["main".into()],
        false,
    )
    .await
    .unwrap();
    db.patch_epic(parent.id, &EpicPatch::new().group_by_repo(true))
        .await
        .unwrap();
    db.regroup_epic(parent.id).await.unwrap();
    let sub = db
        .list_sub_epics(parent.id)
        .await
        .into_iter()
        .find(|e| e.title == "myrepo")
        .expect("regroup made the myrepo sub-epic")
        .id;
    let pre_existing = db.list_tasks_for_epic(sub).await.remove(0);

    // Flat sync (group_by_repo=false) with the same item still emitted.
    let items = vec![make_item("cve-1", "https://github.com/org/myrepo/pull/1")];
    run_feed_sync(&*db, parent.id, false, entries(&items, &[""], &["main"]))
        .await
        .unwrap();

    let parent_tasks = db.list_tasks_for_epic(parent.id).await;
    assert_eq!(parent_tasks.len(), 1, "task re-homed onto the parent");
    assert_eq!(
        parent_tasks[0].id, pre_existing.id,
        "re-home is a move (same task row), not a delete+recreate"
    );
    assert_eq!(parent_tasks[0].external_id.as_deref(), Some("cve-1"));

    assert!(
        db.get_epic(sub).await.is_none(),
        "emptied RepoGroup sub-epic is deleted"
    );
}

/// Regression: a feed epic with group_by_repo=false and NO existing
/// RepoGroup sub-epics behaves exactly as a plain flat upsert (no-op
/// reconciliation, not vacuous — the emission still lands on the parent).
#[tokio::test]
async fn flat_sync_with_no_repo_group_subepics_is_unaffected() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("CVE", "", None).await.unwrap();

    let items = vec![make_item("cve-2", "https://github.com/org/other/pull/2")];
    run_feed_sync(&*db, parent.id, false, entries(&items, &[""], &["main"]))
        .await
        .unwrap();

    let parent_tasks = db.list_tasks_for_epic(parent.id).await;
    assert_eq!(parent_tasks.len(), 1, "flat upsert still lands on parent");
    assert_eq!(parent_tasks[0].external_id.as_deref(), Some("cve-2"));
    assert!(
        db.list_sub_epics(parent.id).await.is_empty(),
        "no sub-epics created or left behind"
    );
}

/// A manually-created (non-RepoGroup) sub-epic under a feed epic is never
/// touched by flat-path reconciliation, mirroring
/// `flatten_preserves_manual_sub_epics` in src/store/tests/grouping.rs.
#[tokio::test]
async fn flat_sync_preserves_manual_sub_epic() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("CVE", "", None).await.unwrap();
    let manual = db.create_epic("notes", "", Some(parent.id)).await.unwrap();
    let manual_task = create_manual_task(&db, "Manual note", manual.id).await;

    let items = vec![make_item("cve-3", "https://github.com/org/other/pull/3")];
    run_feed_sync(&*db, parent.id, false, entries(&items, &[""], &["main"]))
        .await
        .unwrap();

    assert!(
        db.get_epic(manual.id).await.is_some(),
        "manual sub-epic survives flat-path reconciliation"
    );
    let manual_tasks = db.list_tasks_for_epic(manual.id).await;
    assert_eq!(manual_tasks.len(), 1, "manual task stays put");
    assert_eq!(manual_tasks[0].id, manual_task);
}
