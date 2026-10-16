use super::*;

// --- run_role_routed_feed_sync (WP3) ---

/// Task 2 (B0): an emitted PR routes into the sub-epic for its role and the
/// other role sub-epics stay empty.
#[tokio::test]
async fn route_routed_inserts_into_role_sub_epic() {
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
        "https://github.com/org/repo/pull/1",
        vec![Signal::DirectRequest],
    )];

    run_role_routed_feed_sync(&*db, parent.id, entries(&items, &[""], &["main"]))
        .await
        .unwrap();

    let my = role_sub_epic(&db, parent.id, FeedRole::MyReviews).await;
    let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
    let bots = role_sub_epic(&db, parent.id, FeedRole::Bots).await;

    let my_tasks = db.list_tasks_for_epic(my).await;
    assert_eq!(my_tasks.len(), 1, "direct-request PR lands in My Reviews");
    assert_eq!(my_tasks[0].external_id.as_deref(), Some("pr-1"));
    assert!(db.list_tasks_for_epic(team).await.is_empty());
    assert!(db.list_tasks_for_epic(bots).await.is_empty());
}

/// Task 3 (B2): a PR whose role changes is MOVED, preserving its in-flight
/// status, sub_status, worktree, and tmux_window (agent session survives).
#[tokio::test]
async fn route_routed_moves_task_preserving_state() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();

    // Cycle 1: a team-requested PR lands in Team Reviews.
    let cycle1 = vec![make_signal_item(
        "pr-1",
        "https://github.com/org/repo/pull/1",
        vec![Signal::TeamRequest],
    )];
    run_role_routed_feed_sync(&*db, parent.id, entries(&cycle1, &[""], &["main"]))
        .await
        .unwrap();

    let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
    let task = db.list_tasks_for_epic(team).await.remove(0);

    // Simulate in-flight dispatched work on the task.
    db.patch_task(
        task.id,
        &TaskPatch::new()
            .status(TaskStatus::Running)
            .sub_status(crate::models::SubStatus::Active)
            .worktree(Some("/tmp/wt-pr-1"))
            .tmux_window(Some(&test_tmux_window("dispatch:7"))),
    )
    .await
    .unwrap();

    // Cycle 2: the same PR is now also reviewed -> routes to My Reviews.
    let cycle2 = vec![make_signal_item(
        "pr-1",
        "https://github.com/org/repo/pull/1",
        vec![Signal::TeamRequest, Signal::Reviewed],
    )];
    run_role_routed_feed_sync(&*db, parent.id, entries(&cycle2, &[""], &["main"]))
        .await
        .unwrap();

    let my = role_sub_epic(&db, parent.id, FeedRole::MyReviews).await;
    let my_tasks = db.list_tasks_for_epic(my).await;
    assert_eq!(my_tasks.len(), 1, "exactly one task, moved into My Reviews");
    let moved = &my_tasks[0];
    assert_eq!(moved.id, task.id, "same task row, not a recreate");
    assert_eq!(moved.external_id.as_deref(), Some("pr-1"));
    assert_eq!(moved.status, TaskStatus::Running, "status preserved");
    assert_eq!(
        moved.sub_status,
        crate::models::SubStatus::Active,
        "sub_status preserved"
    );
    assert_eq!(moved.worktree.as_deref(), Some("/tmp/wt-pr-1"));
    assert_eq!(
        moved.tmux_window.as_ref().map(|w| w.as_str()),
        Some("dispatch:7")
    );

    assert!(
        db.list_tasks_for_epic(team).await.is_empty(),
        "old role sub-epic no longer holds the moved task"
    );
}

/// Task 4 (B1): the moved task is NOT deleted by the same cycle even though
/// it is absent from its losing role's group.
#[tokio::test]
async fn route_routed_move_not_deleted_same_cycle() {
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

    let cycle2 = vec![make_signal_item(
        "pr-1",
        "https://github.com/org/repo/pull/1",
        vec![Signal::Reviewed],
    )];
    run_role_routed_feed_sync(&*db, parent.id, entries(&cycle2, &[""], &["main"]))
        .await
        .unwrap();

    let my = role_sub_epic(&db, parent.id, FeedRole::MyReviews).await;
    let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
    assert_eq!(
        db.list_tasks_for_epic(my).await.len(),
        1,
        "moved PR survives in its new role"
    );
    assert!(db.list_tasks_for_epic(team).await.is_empty());
}

/// A task MOVED between role sub-epics during a sync must NEVER appear in the
/// removed set. The ordering that makes this true is not compiler-enforced:
/// the module's `ingest_role_routed` lands the move before its per-group
/// upsert, subtree delete and parent sweep, each of which filters on the
/// task's CURRENT `epic_id`. Before feed-task teardown existed a
/// mis-ordering merely deleted a row; now it would force-remove a live review
/// agent's worktree and kill its tmux window. Pin it.
///
/// `pr-2` stays in Team Reviews on purpose: it keeps a non-empty group on the
/// LOSING role sub-epic, so the per-epic stale-delete would
/// actually sweep `pr-1` out of Team if the move had not already landed. Drop
/// `pr-2` and the test stops detecting the mis-ordering.
#[tokio::test]
async fn moved_task_is_never_reported_as_removed() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();
    db.patch_epic(
        parent.id,
        &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
    )
    .await
    .unwrap();

    // Cycle 1: both PRs are team-requested, so both land in Team Reviews.
    let cycle1 = vec![
        make_signal_item(
            "pr-1",
            "https://github.com/org/repo/pull/1",
            vec![Signal::TeamRequest],
        ),
        make_signal_item(
            "pr-2",
            "https://github.com/org/repo/pull/2",
            vec![Signal::TeamRequest],
        ),
    ];
    let outcome = run_feed_sync_by_role(
        &*db,
        parent.id,
        FeedRole::ReviewsParent,
        false,
        entries(&cycle1, &["", ""], &["main", "main"]),
    )
    .await
    .unwrap();
    assert!(
        outcome.removed.is_empty(),
        "nothing removed on first sight, got: {:?}",
        outcome.removed
    );

    // Give pr-1 an in-flight worktree and tmux window, as a dispatched review
    // agent would.
    let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
    let task = db
        .list_tasks_for_epic(team)
        .await
        .into_iter()
        .find(|t| t.external_id.as_deref() == Some("pr-1"))
        .expect("pr-1 landed in Team Reviews");
    db.patch_task(
        task.id,
        &TaskPatch::new()
            .worktree(Some("/repo/a/.worktrees/7-pr-1"))
            .tmux_window(Some(&test_tmux_window("dispatch:pr-1"))),
    )
    .await
    .unwrap();

    // Cycle 2: pr-1 is now also reviewed, so it routes to My Reviews — a MOVE
    // across role sub-epics, not a delete-and-reinsert. pr-2 stays in Team.
    let cycle2 = vec![
        make_signal_item(
            "pr-1",
            "https://github.com/org/repo/pull/1",
            vec![Signal::TeamRequest, Signal::Reviewed],
        ),
        make_signal_item(
            "pr-2",
            "https://github.com/org/repo/pull/2",
            vec![Signal::TeamRequest],
        ),
    ];
    let outcome = run_feed_sync_by_role(
        &*db,
        parent.id,
        FeedRole::ReviewsParent,
        false,
        entries(&cycle2, &["", ""], &["main", "main"]),
    )
    .await
    .unwrap();

    assert!(
        outcome.removed.is_empty(),
        "a moved task must not be reported for teardown, got: {:?}",
        outcome.removed
    );

    let my = role_sub_epic(&db, parent.id, FeedRole::MyReviews).await;
    let moved = db.list_tasks_for_epic(my).await.remove(0);
    assert_eq!(moved.id, task.id, "same task row, not a recreate");
    assert_eq!(
        moved.worktree.as_deref(),
        Some("/repo/a/.worktrees/7-pr-1"),
        "the move must preserve the in-flight worktree"
    );
    assert_eq!(
        moved.tmux_window.as_ref().map(|w| w.as_str()),
        Some("dispatch:pr-1")
    );
}

/// A PR that genuinely leaves the emission (merged/closed) IS reported, so the
/// caller can tear its worktree down — the counterpart to the move case above.
#[tokio::test]
async fn absent_task_with_worktree_is_reported_as_removed() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();
    db.patch_epic(
        parent.id,
        &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
    )
    .await
    .unwrap();

    let cycle1 = vec![make_signal_item(
        "pr-1",
        "https://github.com/org/repo/pull/1",
        vec![Signal::TeamRequest],
    )];
    run_feed_sync_by_role(
        &*db,
        parent.id,
        FeedRole::ReviewsParent,
        false,
        entries(&cycle1, &[""], &["main"]),
    )
    .await
    .unwrap();

    let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
    let task = db.list_tasks_for_epic(team).await.remove(0);
    db.patch_task(
        task.id,
        &TaskPatch::new()
            .worktree(Some("/repo/a/.worktrees/7-pr-1"))
            .tmux_window(Some(&test_tmux_window("dispatch:pr-1"))),
    )
    .await
    .unwrap();

    // Cycle 2: the PR merged, so it is gone from the emission.
    let outcome = run_feed_sync_by_role(&*db, parent.id, FeedRole::ReviewsParent, false, vec![])
        .await
        .unwrap();

    assert_eq!(
        outcome.removed.len(),
        1,
        "the merged PR's worktree must be reported for teardown, got: {:?}",
        outcome.removed
    );
    assert_eq!(outcome.removed[0].id, task.id);
    assert_eq!(
        outcome.removed[0].worktree.as_deref(),
        Some("/repo/a/.worktrees/7-pr-1")
    );
    assert_eq!(
        outcome.removed[0].tmux_window.as_ref().map(|w| w.as_str()),
        Some("dispatch:pr-1")
    );
}

/// The flat (non-reviews_parent) path reports its removals too — the flat
/// branch of `run_feed_sync` is a `RemovedFeedTask` producer like the rest.
#[tokio::test]
async fn flat_sync_reports_removed_task_with_worktree() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let epic = db.create_epic("Flat Feed", "", None).await.unwrap();

    let items = vec![make_item("pr-1", "https://github.com/org/repo/pull/1")];
    run_feed_sync(&*db, epic.id, false, entries(&items, &[""], &["main"]))
        .await
        .unwrap();

    let task = db.list_tasks_for_epic(epic.id).await.remove(0);
    db.patch_task(
        task.id,
        &TaskPatch::new()
            .worktree(Some("/repo/a/.worktrees/7-pr-1"))
            .tmux_window(Some(&test_tmux_window("dispatch:pr-1"))),
    )
    .await
    .unwrap();

    let outcome = run_feed_sync(&*db, epic.id, false, vec![]).await.unwrap();

    assert_eq!(
        outcome.removed.len(),
        1,
        "flat stale-delete must report its removal, got: {:?}",
        outcome.removed
    );
    assert_eq!(outcome.removed[0].id, task.id);
}

/// Count feed-managed tasks (external_id set) sitting DIRECTLY on an epic.
async fn flat_feed_task_count(db: &Store, epic: EpicId) -> usize {
    db.list_tasks_for_epic(epic)
        .await
        .into_iter()
        .filter(|t| t.external_id.is_some())
        .count()
}

/// Bug B (parent-stranded rescue): a feed task sitting flat on the
/// reviews_parent epic itself is MOVED down into its routed role sub-epic —
/// same row, in-flight state preserved — not left to deadlock the
/// subtree-uniqueness trigger. Enforces NoFlatFeedTasksOnReviewsParent.
#[tokio::test]
async fn route_routed_rescues_flat_task_stranded_on_parent() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();

    // Strand a flat feed task directly on the parent — exactly what an
    // out-of-band flat upsert (the manual-trigger bug, or an older binary)
    // produces. Inserting into the reviews_parent epic does not fire the
    // subtree-uniqueness trigger, so this is a valid starting state.
    let item = make_signal_item(
        "pr-1",
        "https://github.com/org/repo/pull/1",
        vec![Signal::DirectRequest],
    );
    db.ingest_feed(
        parent.id,
        std::slice::from_ref(&item),
        &["".into()],
        &["main".into()],
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
    let stranded = db.list_tasks_for_epic(parent.id).await.remove(0);

    // Simulate in-flight dispatched work on the stranded task.
    db.patch_task(
        stranded.id,
        &TaskPatch::new()
            .status(TaskStatus::Running)
            .sub_status(crate::models::SubStatus::Active)
            .worktree(Some("/tmp/wt-pr-1"))
            .tmux_window(Some(&test_tmux_window("dispatch:7"))),
    )
    .await
    .unwrap();

    // Reconcile with the same PR present in the emission.
    run_role_routed_feed_sync(&*db, parent.id, entries(&[item], &[""], &["main"]))
        .await
        .unwrap();

    assert_eq!(
        flat_feed_task_count(&db, parent.id).await,
        0,
        "reviews_parent must hold no flat feed task after reconcile"
    );

    let my = role_sub_epic(&db, parent.id, FeedRole::MyReviews).await;
    let my_tasks = db.list_tasks_for_epic(my).await;
    assert_eq!(my_tasks.len(), 1, "the rescued PR lands in My Reviews once");
    let moved = &my_tasks[0];
    assert_eq!(moved.id, stranded.id, "same task row, not delete+recreate");
    assert_eq!(moved.status, TaskStatus::Running, "status preserved");
    assert_eq!(
        moved.sub_status,
        crate::models::SubStatus::Active,
        "sub_status preserved"
    );
    assert_eq!(moved.worktree.as_deref(), Some("/tmp/wt-pr-1"));
    assert_eq!(
        moved.tmux_window.as_ref().map(|w| w.as_str()),
        Some("dispatch:7")
    );
}

/// Bug B (parent-stranded stale delete): a feed task stranded on the parent
/// that no current item names is removed as stale by the subtree delete,
/// whose scope must include the parent epic itself.
#[tokio::test]
async fn route_routed_deletes_stale_flat_task_on_parent() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();

    let gone = make_item("pr-gone", "https://github.com/org/repo/pull/9");
    db.ingest_feed(parent.id, &[gone], &["".into()], &["main".into()], false)
        .await
        .unwrap();
    // The role is set only after the seed: ingest routes a role-carrying epic, so a flat task on the parent can only be planted while it has no role.
    db.patch_epic(
        parent.id,
        &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
    )
    .await
    .unwrap();
    assert_eq!(flat_feed_task_count(&db, parent.id).await, 1);

    // Emission no longer contains pr-gone (merged/closed).
    run_role_routed_feed_sync(&*db, parent.id, entries(&[], &[], &[]))
        .await
        .unwrap();

    assert_eq!(
        flat_feed_task_count(&db, parent.id).await,
        0,
        "stale parent-stranded feed task must be deleted"
    );
}

/// Bug B guard: a MANUAL task (external_id = null) on the parent is never
/// touched by the parent-inclusive reconcile — only feed-managed tasks are.
#[tokio::test]
async fn route_routed_preserves_manual_task_on_parent() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();
    db.patch_epic(
        parent.id,
        &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
    )
    .await
    .unwrap();

    let manual_id = create_manual_task(&db, "Manual note on parent", parent.id).await;

    run_role_routed_feed_sync(&*db, parent.id, entries(&[], &[], &[]))
        .await
        .unwrap();

    let survivors = db.list_tasks_for_epic(parent.id).await;
    assert_eq!(survivors.len(), 1, "manual task must survive");
    assert_eq!(survivors[0].id, manual_id);
    assert!(
        survivors[0].external_id.is_none(),
        "the survivor is the manual (non-feed) task"
    );
}

/// Bug B (legacy duplicate convergence): the corrupt state the old
/// flat-upsert bug produced — the SAME PR present BOTH flat on the parent
/// AND routed in a role sub-epic. The reconcile must converge to a single
/// copy in the sub-epic and clear the parent duplicate.
#[tokio::test]
async fn route_routed_clears_parent_duplicate_when_canonical_in_sub_epic() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();
    db.patch_epic(
        parent.id,
        &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
    )
    .await
    .unwrap();

    let item = make_signal_item(
        "pr-1",
        "https://github.com/org/repo/pull/1",
        vec![Signal::DirectRequest],
    );

    // Cycle 1: route the PR into My Reviews (the canonical copy).
    run_role_routed_feed_sync(
        &*db,
        parent.id,
        entries(std::slice::from_ref(&item), &[""], &["main"]),
    )
    .await
    .unwrap();
    let my = role_sub_epic(&db, parent.id, FeedRole::MyReviews).await;
    assert_eq!(db.list_tasks_for_epic(my).await.len(), 1);

    // Corrupt the state: plant a duplicate flat copy on the parent, as the
    // old manual-trigger flat upsert did (inserting onto a reviews_parent
    // epic does not fire the subtree-uniqueness trigger).
    // Ingest routes a role-carrying epic, so the flat copy is planted while the
    // parent briefly has no role.
    db.patch_epic(parent.id, &EpicPatch::new().feed_role(FeedRole::None))
        .await
        .unwrap();
    db.ingest_feed(
        parent.id,
        std::slice::from_ref(&item),
        &["".into()],
        &["main".into()],
        false,
    )
    .await
    .unwrap();
    db.patch_epic(
        parent.id,
        &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
    )
    .await
    .unwrap();
    assert_eq!(
        flat_feed_task_count(&db, parent.id).await,
        1,
        "duplicate planted"
    );

    // Cycle 2: reconcile with the same PR present. Must converge.
    run_role_routed_feed_sync(
        &*db,
        parent.id,
        entries(std::slice::from_ref(&item), &[""], &["main"]),
    )
    .await
    .unwrap();

    assert_eq!(
        flat_feed_task_count(&db, parent.id).await,
        0,
        "parent duplicate cleared"
    );
    assert_eq!(
        db.list_tasks_for_epic(my).await.len(),
        1,
        "exactly one canonical copy remains in My Reviews"
    );
}

/// Task 4: a PR present in cycle 1 but absent from cycle 2 (merged/closed)
/// is removed from the subtree; a manual task (external_id NULL) survives.
#[tokio::test]
async fn route_routed_removes_merged_pr_keeps_manual() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();

    let cycle1 = vec![
        make_signal_item(
            "pr-1",
            "https://github.com/org/repo/pull/1",
            vec![Signal::DirectRequest],
        ),
        make_signal_item(
            "pr-2",
            "https://github.com/org/repo/pull/2",
            vec![Signal::TeamRequest],
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
    // A manual task the user added under a role sub-epic.
    let manual_id = create_manual_task(&db, "Manual", my).await;

    // Cycle 2: pr-2 merged/closed (absent). pr-1 still direct-requested.
    let cycle2 = vec![make_signal_item(
        "pr-1",
        "https://github.com/org/repo/pull/1",
        vec![Signal::DirectRequest],
    )];
    run_role_routed_feed_sync(&*db, parent.id, entries(&cycle2, &[""], &["main"]))
        .await
        .unwrap();

    let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
    assert!(
        db.list_tasks_for_epic(team).await.is_empty(),
        "merged pr-2 removed from Team Reviews"
    );

    let my_tasks = db.list_tasks_for_epic(my).await;
    assert!(
        my_tasks.iter().any(|t| t.id == manual_id),
        "manual task survives reconcile"
    );
    assert!(
        my_tasks
            .iter()
            .any(|t| t.external_id.as_deref() == Some("pr-1")),
        "still-open pr-1 retained"
    );
}

/// WP2 regression: each item must land with ITS OWN repo_path/base_branch,
/// never a neighbour's. Three items across three different roles (so they
/// land in three different sub-epics) each carry a distinct repo_path and
/// base_branch; a mis-paired zip (the footgun the old parallel-slice
/// length guard only detected after the fact) would surface here as a
/// task holding the wrong branch or repo_path.
#[tokio::test]
async fn route_routed_preserves_per_item_repo_path_and_base_branch() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();
    db.patch_epic(
        parent.id,
        &EpicPatch::new().feed_role(FeedRole::ReviewsParent),
    )
    .await
    .unwrap();

    let items = vec![
        make_signal_item(
            "pr-my",
            "https://github.com/org/repo-my/pull/1",
            vec![Signal::DirectRequest],
        ),
        make_signal_item(
            "pr-team",
            "https://github.com/org/repo-team/pull/2",
            vec![Signal::TeamRequest],
        ),
        make_signal_item(
            "pr-bots",
            "https://github.com/org/repo-bots/pull/3",
            vec![Signal::AuthorBot],
        ),
    ];
    let entries = entries(
        &items,
        &["/repo-my", "/repo-team", "/repo-bots"],
        &["my-branch", "team-branch", "bots-branch"],
    );
    run_role_routed_feed_sync(&*db, parent.id, entries)
        .await
        .unwrap();

    let my = role_sub_epic(&db, parent.id, FeedRole::MyReviews).await;
    let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
    let bots = role_sub_epic(&db, parent.id, FeedRole::Bots).await;

    let task_by_ext = |tasks: &[crate::models::Task], ext: &str| {
        tasks
            .iter()
            .find(|t| t.external_id.as_deref() == Some(ext))
            .unwrap()
            .clone()
    };

    let my_tasks = db.list_tasks_for_epic(my).await;
    let my_task = task_by_ext(&my_tasks, "pr-my");
    assert_eq!(my_task.repo_path, "/repo-my");
    assert_eq!(my_task.base_branch, "my-branch");

    let team_tasks = db.list_tasks_for_epic(team).await;
    let team_task = task_by_ext(&team_tasks, "pr-team");
    assert_eq!(team_task.repo_path, "/repo-team");
    assert_eq!(team_task.base_branch, "team-branch");

    let bots_tasks = db.list_tasks_for_epic(bots).await;
    let bots_task = task_by_ext(&bots_tasks, "pr-bots");
    assert_eq!(bots_task.repo_path, "/repo-bots");
    assert_eq!(bots_task.base_branch, "bots-branch");
}
