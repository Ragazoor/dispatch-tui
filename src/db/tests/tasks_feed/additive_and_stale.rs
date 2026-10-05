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

/// Insert a `reviews-parent` epic and a `my-reviews` role sub-epic beneath
/// it, returning the sub-epic's id. Plain `create_epic` always defaults to
/// `feed_role = 'none'`, which the v72/v76 subtree-uniqueness triggers
/// ignore entirely — so tests that must exercise those triggers (rather than
/// accidentally bypass them) set up the epic tree via raw SQL instead.
async fn create_role_sub_epic(db: &Database) -> EpicId {
    db.db_call(|conn| {
        conn.execute_batch(
            "INSERT INTO epics (id, title, description, status, feed_role, origin)
             VALUES (1, 'PR Reviews', '', 'backlog', 'reviews-parent', 'manual');
             INSERT INTO epics (id, title, description, status, feed_role, origin, parent_epic_id)
             VALUES (2, 'My Reviews', '', 'backlog', 'my-reviews', 'manual', 1);",
        )
        .map_err(anyhow::Error::from)
    })
    .await
    .unwrap();
    EpicId(2)
}

/// Regression test for the v72 trigger false-positiving on the ON CONFLICT
/// DO UPDATE path: re-upserting an already-tracked task into the SAME role
/// sub-epic must not error, since it resolves via the existing (epic_id,
/// external_id) row, not a genuine cross-epic duplicate.
#[tokio::test]
async fn upsert_feed_tasks_reupsert_into_role_sub_epic_does_not_error() {
    let db = unattached_db().await;
    let epic_id = create_role_sub_epic(&db).await;
    let items = vec![make_feed_item("ext-1", "Task One")];
    let repo_paths = vec!["/repo".to_string()];
    let branches = main_branches(1);

    db.upsert_feed_tasks(epic_id, &items, &repo_paths, &branches)
        .await
        .unwrap();
    db.upsert_feed_tasks(epic_id, &items, &repo_paths, &branches)
        .await
        .expect("re-upserting an already-tracked task in the same role sub-epic must not error");

    let tasks = db.list_tasks_for_epic(epic_id).await.unwrap();
    assert_eq!(tasks.len(), 1, "second call should not create a duplicate");
}

/// A batch containing one already-tracked item and one brand-new item, both
/// targeting the same role sub-epic, must fully succeed: the pre-existing
/// item's self-conflict must not abort the whole transaction and silently
/// drop the new item alongside it.
#[tokio::test]
async fn upsert_feed_tasks_mixed_batch_existing_and_new_item_in_role_sub_epic_succeeds() {
    let db = unattached_db().await;
    let epic_id = create_role_sub_epic(&db).await;

    db.upsert_feed_tasks(
        epic_id,
        &[make_feed_item("ext-1", "First")],
        &["/repo".to_string()],
        &main_branches(1),
    )
    .await
    .unwrap();

    db.upsert_feed_tasks(
        epic_id,
        &[
            make_feed_item("ext-1", "First"),
            make_feed_item("ext-2", "Second"),
        ],
        &["/repo".to_string(), "/repo".to_string()],
        &main_branches(2),
    )
    .await
    .expect("a batch mixing an already-tracked item with a brand-new item must fully succeed");

    let tasks = db.list_tasks_for_epic(epic_id).await.unwrap();
    assert_eq!(
        tasks.len(),
        2,
        "both the existing and the new item must be present"
    );
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

/// The subtree delete must hand back the rows it removed so the caller can tear
/// down their worktrees (`feeds.allium`: `RoleRoutedFeedSync`). Only rows
/// carrying a worktree or tmux window are returned — a plain card has nothing to
/// clean up. Crucially, the DELETE predicate is unchanged: every stale feed task
/// is still removed from the DB, reported or not.
///
/// Two of the deleted rows carry state on purpose: reporting must be *complete*,
/// not merely non-empty. Under-reporting is invisible in the DB (the row is gone
/// either way) and surfaces only as a silently leaked worktree, so a single
/// state-carrying row would let a truncated `RETURNING` drain pass.
#[tokio::test]
async fn delete_stale_subtree_feed_tasks_returns_removed_rows_with_state() {
    let db = unattached_db().await;
    let parent = db.create_epic("Reviews", "", None).await.unwrap();
    let sub = db
        .create_epic("My Reviews", "", Some(parent.id))
        .await
        .unwrap();

    db.upsert_feed_tasks(
        sub.id,
        &[
            make_feed_item("stale-1", "Stale"),
            make_feed_item("stale-2", "Also stale"),
            make_feed_item("plain-3", "Plain"),
            make_feed_item("keep-4", "Kept"),
        ],
        &vec!["/repo/a".to_string(); 4],
        &main_branches(4),
    )
    .await
    .unwrap();

    let tasks = db.list_tasks_for_epic(sub.id).await.unwrap();
    let by_ext = |ext: &str| {
        tasks
            .iter()
            .find(|t| t.external_id.as_deref() == Some(ext))
            .unwrap()
            .id
    };

    // `stale-1` carries both kinds of state; `stale-2` only a worktree, so the
    // two reported rows are not interchangeable. `plain-3` carries neither.
    let stale_1 = by_ext("stale-1");
    let stale_2 = by_ext("stale-2");
    db.patch_task(
        stale_1,
        &TaskPatch::new()
            .worktree(Some("/repo/a/.worktrees/stale-1"))
            .tmux_window(Some(&test_tmux_window("dispatch:stale-1"))),
    )
    .await
    .unwrap();
    db.patch_task(
        stale_2,
        &TaskPatch::new().worktree(Some("/repo/a/.worktrees/stale-2")),
    )
    .await
    .unwrap();

    let removed = db
        .delete_stale_subtree_feed_tasks(parent.id, &["keep-4".to_string()])
        .await
        .unwrap();

    // stale-1, stale-2 and plain-3 are all really gone from the DB...
    let left = db.list_tasks_for_epic(sub.id).await.unwrap();
    assert_eq!(left.len(), 1, "only the kept item survives, got {left:?}");
    assert_eq!(left[0].external_id.as_deref(), Some("keep-4"));

    // ...but only the two rows with state need teardown, and *both* of them do:
    // dropping either would leak a worktree with nothing left to name it.
    assert_eq!(
        removed.len(),
        2,
        "every removed row with state must be reported, got {removed:?}"
    );
    let reported = |id| removed.iter().find(|r| r.id == id).unwrap();
    let one = reported(stale_1);
    assert_eq!(one.repo_path, "/repo/a");
    assert_eq!(one.worktree.as_deref(), Some("/repo/a/.worktrees/stale-1"));
    assert_eq!(
        one.tmux_window.as_ref().map(|w| w.as_str()),
        Some("dispatch:stale-1")
    );
    let two = reported(stale_2);
    assert_eq!(two.repo_path, "/repo/a");
    assert_eq!(two.worktree.as_deref(), Some("/repo/a/.worktrees/stale-2"));
    assert_eq!(two.tmux_window, None);
}

/// The same contract for the flat/grouped path's stale-delete, which runs inside
/// `upsert_feed_tasks` (`feeds.allium`: `UpsertFeedTasks`).
#[tokio::test]
async fn upsert_feed_tasks_returns_removed_rows_with_state() {
    let db = unattached_db().await;
    let epic = db.create_epic("CVE Feed", "", None).await.unwrap();

    db.upsert_feed_tasks(
        epic.id,
        &[
            make_feed_item("gone-1", "Gone"),
            make_feed_item("bare-2", "Bare"),
        ],
        &vec!["/repo/b".to_string(); 2],
        &main_branches(2),
    )
    .await
    .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    let gone = tasks
        .iter()
        .find(|t| t.external_id.as_deref() == Some("gone-1"))
        .unwrap();
    db.patch_task(
        gone.id,
        &TaskPatch::new().worktree(Some("/repo/b/.worktrees/gone-1")),
    )
    .await
    .unwrap();

    // An empty emission clears the epic and reports what it removed.
    let removed = db.upsert_feed_tasks(epic.id, &[], &[], &[]).await.unwrap();

    // The delete really ran — draining RETURNING is what executes it.
    assert!(
        db.list_tasks_for_epic(epic.id).await.unwrap().is_empty(),
        "every stale feed task is deleted, reported or not"
    );

    assert_eq!(removed.len(), 1, "only the row with a worktree is reported");
    assert_eq!(removed[0].id, gone.id);
    assert_eq!(removed[0].repo_path, "/repo/b");
    assert_eq!(
        removed[0].worktree.as_deref(),
        Some("/repo/b/.worktrees/gone-1")
    );
    assert_eq!(removed[0].tmux_window, None, "no tmux window was set");
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
            title: "Manual",
            description: "",
            repo_path: "/repo/a",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: Some(sub.id),
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
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
            title: "Manual",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: Some(child_a.id),
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
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
