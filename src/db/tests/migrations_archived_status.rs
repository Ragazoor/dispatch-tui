use super::*;

// ---------------------------------------------------------------------------
// ArchivedStatusMigration (epics.allium) — the one-time migration that retires
// the `archived` status (task #4971).
//
// The fixture is a real, fully migrated board on disk with rows forced into
// `archived` (CHECK constraints bypassed, so the fixture builds on either side
// of the migration), rewound to `user_version = 103` — the last version before
// the archived-status migration — and reopened, so `Database::open` runs that
// migration through the real runner exactly as an upgrading board would. Every
// assertion then goes through the ordinary `Database` API.
// ---------------------------------------------------------------------------

/// The last schema version that still had the `archived` status — v104
/// (drop filter_presets) and v105 (create retired_feed_items) came after this
/// one and before v106 (the archived-status migration itself), so rewinding
/// to this value and reopening exercises all three in sequence, not just
/// v106 alone.
const LAST_VERSION_WITH_ARCHIVED: i64 = 103;

struct ArchivedBoard {
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
    /// Feed root carrying a feed_command.
    feed_root: EpicId,
    /// Repo sub-epic of `feed_root` (no command of its own; not archived).
    feed_sub: EpicId,
    /// Archived feed task in `feed_sub`, no worktree: retired, then deleted.
    feed_archived: TaskId,
    /// Archived feed task in `feed_root` still holding a worktree: retired,
    /// then kept as done.
    feed_archived_with_worktree: TaskId,
    /// Archived manual task in a plain epic, no worktree: deleted.
    manual_archived: TaskId,
    /// A live backlog task in the same plain epic: untouched.
    live: TaskId,
    plain_epic: EpicId,
    /// Archived epic whose only task is archived without a worktree: deleted.
    archived_epic_emptied: EpicId,
    /// Archived epic holding an archived task WITH a worktree: kept, done.
    archived_epic_kept_done: EpicId,
    kept_task: TaskId,
    /// Archived epic still holding a backlog task: kept, set done, then
    /// recalculated — which regresses it to backlog.
    archived_epic_with_open_task: EpicId,
    open_task: TaskId,
    /// Archived parent with an archived, task-less child: both deleted.
    archived_parent: EpicId,
    archived_child: EpicId,
    /// Archived, task-less epic whose child epic is NOT archived (never
    /// touched by force_archived) and itself task-less: epics.allium's
    /// literal phase-4 text ("if epic.subtree_tasks.is_empty(): not exists
    /// epic") deletes the archived epic together with its whole empty
    /// subtree, even though the child epic's own status was never archived.
    archived_epic_with_live_child: EpicId,
    live_child_of_archived: EpicId,
    /// Archived managed reviews_parent root, empty: settings cleared, deleted.
    archived_reviews_root: EpicId,
    /// Archived managed cve root still holding a worktree task: settings
    /// cleared, kept as done.
    archived_cve_root: EpicId,
    /// A learning whose `source_task_id` names `manual_archived` — deleted by
    /// phase 2, so this row's `source_task_id` must be nulled rather than
    /// left dangling.
    orphaned_learning: i64,
}

fn archived_item(external_id: &str) -> crate::models::FeedItem {
    crate::models::FeedItem {
        external_id: external_id.to_string(),
        title: external_id.to_string(),
        description: String::new(),
        url: String::new(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::Bug,
        labels: Vec::new(),
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    }
}

async fn upsert_one(db: &Database, epic: EpicId, external_id: &str) -> TaskId {
    db.upsert_feed_tasks(
        epic,
        &[archived_item(external_id)],
        &["/repo".to_string()],
        &["main".to_string()],
    )
    .await
    .unwrap();
    db.list_tasks_for_epic(epic)
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.external_id.as_deref() == Some(external_id))
        .unwrap()
        .id
}

async fn task_in_epic(db: &Database, title: &str, epic: EpicId) -> TaskId {
    let t = make_task(db, title).await;
    db.set_task_epic_id(t.id, Some(epic)).await.unwrap();
    t.id
}

async fn set_worktree(db: &Database, id: TaskId, path: &str) {
    db.patch_task(id, &TaskPatch::new().worktree(Some(path)))
        .await
        .unwrap();
}

/// Force `archived` onto rows, bypassing the CHECK constraints so the
/// fixture can be built on a schema that no longer admits the value.
async fn force_archived(db: &Database, tasks: Vec<TaskId>, epics: Vec<EpicId>) {
    db.db_call(move |conn| {
        conn.execute_batch("PRAGMA ignore_check_constraints = ON;")?;
        for t in &tasks {
            conn.execute(
                "UPDATE tasks SET status = 'archived', sub_status = 'none' WHERE id = ?1",
                [t.0],
            )?;
        }
        for e in &epics {
            conn.execute("UPDATE epics SET status = 'archived' WHERE id = ?1", [e.0])?;
        }
        conn.execute_batch("PRAGMA ignore_check_constraints = OFF;")?;
        Ok(())
    })
    .await
    .unwrap();
}

async fn build_archived_board() -> ArchivedBoard {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.db");
    let db = Database::open(&path).await.unwrap();
    {
        let feed_root = db.create_epic("Feed", "", None).await.unwrap().id;
        db.patch_epic(feed_root, &EpicPatch::new().feed_command(Some("echo []")))
            .await
            .unwrap();
        let feed_sub = db
            .create_epic("repo-a", "", Some(feed_root))
            .await
            .unwrap()
            .id;
        let feed_archived = upsert_one(&db, feed_sub, "ext-archived").await;
        let feed_archived_with_worktree = upsert_one(&db, feed_root, "ext-worktree").await;
        set_worktree(&db, feed_archived_with_worktree, "/wt/ext-worktree").await;

        let plain_epic = db.create_epic("Plain", "", None).await.unwrap().id;
        let manual_archived = task_in_epic(&db, "manual archived", plain_epic).await;
        let live = task_in_epic(&db, "live", plain_epic).await;

        let archived_epic_emptied = db.create_epic("Emptied", "", None).await.unwrap().id;
        let emptied_task = task_in_epic(&db, "emptied task", archived_epic_emptied).await;

        let archived_epic_kept_done = db.create_epic("Kept", "", None).await.unwrap().id;
        let kept_task = task_in_epic(&db, "kept task", archived_epic_kept_done).await;
        set_worktree(&db, kept_task, "/wt/kept").await;

        let archived_epic_with_open_task = db.create_epic("Open", "", None).await.unwrap().id;
        let open_task = task_in_epic(&db, "open task", archived_epic_with_open_task).await;

        let archived_parent = db.create_epic("Parent", "", None).await.unwrap().id;
        let archived_child = db
            .create_epic("Child", "", Some(archived_parent))
            .await
            .unwrap()
            .id;

        let archived_epic_with_live_child = db
            .create_epic("Parent-with-live-child", "", None)
            .await
            .unwrap()
            .id;
        let live_child_of_archived = db
            .create_epic("Live-child", "", Some(archived_epic_with_live_child))
            .await
            .unwrap()
            .id;

        let archived_reviews_root = db.create_epic("Reviews", "", None).await.unwrap().id;
        db.patch_epic(
            archived_reviews_root,
            &EpicPatch::new().feed_role(crate::models::FeedRole::ReviewsParent),
        )
        .await
        .unwrap();
        db.set_reviews_feed_command(Some("reviews-cmd"))
            .await
            .unwrap();
        db.set_reviews_feed_interval_secs(Some(600)).await.unwrap();

        let archived_cve_root = db.create_epic("CVE", "", None).await.unwrap().id;
        db.patch_epic(
            archived_cve_root,
            &EpicPatch::new().feed_role(crate::models::FeedRole::Cve),
        )
        .await
        .unwrap();
        let cve_task = task_in_epic(&db, "cve task", archived_cve_root).await;
        set_worktree(&db, cve_task, "/wt/cve").await;
        db.set_cve_feed_command(Some("cve-cmd")).await.unwrap();
        db.set_cve_feed_interval_secs(Some(600)).await.unwrap();

        force_archived(
            &db,
            vec![
                feed_archived,
                feed_archived_with_worktree,
                manual_archived,
                emptied_task,
                kept_task,
                cve_task,
            ],
            vec![
                archived_epic_emptied,
                archived_epic_kept_done,
                archived_epic_with_open_task,
                archived_parent,
                archived_child,
                archived_epic_with_live_child,
                archived_reviews_root,
                archived_cve_root,
            ],
        )
        .await;

        let orphaned_learning: i64 = db
            .db_call(move |conn| {
                conn.execute(
                    "INSERT INTO learnings (kind, summary, scope, status, source_task_id) \
                     VALUES ('pitfall', 'orphaned by archive', 'user', 'approved', ?1)",
                    [manual_archived.0],
                )?;
                Ok(conn.last_insert_rowid())
            })
            .await
            .unwrap();

        db.db_call(|conn| {
            conn.pragma_update(None, "user_version", LAST_VERSION_WITH_ARCHIVED)?;
            Ok(())
        })
        .await
        .unwrap();

        ArchivedBoard {
            _dir: dir,
            path,
            feed_root,
            feed_sub,
            feed_archived,
            feed_archived_with_worktree,
            manual_archived,
            live,
            plain_epic,
            archived_epic_emptied,
            archived_epic_kept_done,
            kept_task,
            archived_epic_with_open_task,
            open_task,
            archived_parent,
            archived_child,
            archived_epic_with_live_child,
            live_child_of_archived,
            archived_reviews_root,
            archived_cve_root,
            orphaned_learning,
        }
    }
}

/// Reopen the rewound board, which runs the archived-status migration.
async fn migrate(board: &ArchivedBoard) -> Database {
    Database::open(&board.path).await.unwrap()
}

/// Phase 1: every archived feed task is retired under the feed epic its chain
/// names — the root for a task in a command-less repo sub-epic. Observed
/// through ingest: the id is refused afterwards.
#[tokio::test]
async fn archived_status_migration_phase_1_retires_every_archived_feed_task() {
    let board = build_archived_board().await;
    let db = migrate(&board).await;

    db.upsert_feed_tasks(
        board.feed_sub,
        &[archived_item("ext-archived")],
        &["/repo".to_string()],
        &["main".to_string()],
    )
    .await
    .unwrap();
    assert!(
        db.list_tasks_for_epic(board.feed_sub)
            .await
            .unwrap()
            .iter()
            .all(|t| t.external_id.as_deref() != Some("ext-archived")),
        "an archived feed task must be retired, so the feed does not re-insert it"
    );

    // The worktree-holding survivor is retired too: detach it (which retires
    // nothing), and the feed still refuses its id.
    db.set_task_epic_id(board.feed_archived_with_worktree, None)
        .await
        .unwrap();
    db.upsert_feed_tasks(
        board.feed_root,
        &[archived_item("ext-worktree")],
        &["/repo".to_string()],
        &["main".to_string()],
    )
    .await
    .unwrap();
    assert!(
        db.list_tasks_for_epic(board.feed_root)
            .await
            .unwrap()
            .iter()
            .all(|t| t.external_id.as_deref() != Some("ext-worktree")),
        "the survivor's id is retired under the feed root as well"
    );
}

/// Phase 2: an archived task still holding a worktree becomes done (sub
/// status reset, worktree kept so the board can retry teardown); every other
/// archived task is deleted. Non-archived tasks are untouched.
#[tokio::test]
async fn archived_status_migration_phase_2_keeps_worktree_holders_as_done_and_deletes_the_rest() {
    let board = build_archived_board().await;
    let db = migrate(&board).await;

    let survivor = db
        .get_task(board.feed_archived_with_worktree)
        .await
        .unwrap()
        .expect("a worktree-holding archived task survives");
    assert_eq!(survivor.status, TaskStatus::Done);
    assert_eq!(survivor.sub_status, SubStatus::None);
    assert_eq!(survivor.worktree.as_deref(), Some("/wt/ext-worktree"));

    let kept = db.get_task(board.kept_task).await.unwrap().unwrap();
    assert_eq!(kept.status, TaskStatus::Done);
    assert_eq!(kept.worktree.as_deref(), Some("/wt/kept"));

    assert!(db.get_task(board.feed_archived).await.unwrap().is_none());
    assert!(db.get_task(board.manual_archived).await.unwrap().is_none());

    let live = db.get_task(board.live).await.unwrap().unwrap();
    assert_eq!(live.status, TaskStatus::Backlog);
    assert_eq!(
        db.get_epic(board.plain_epic)
            .await
            .unwrap()
            .map(|e| e.status),
        Some(TaskStatus::Backlog),
        "a non-archived epic is untouched"
    );
}

/// Phase 2 also owes the deleted rows the same learning detachment
/// `delete_task`'s real runtime path gets from `learnings.source_task_id`'s
/// `ON DELETE SET NULL` (migration v47) — disabled here along with every
/// other FK action by the migration runner's `PRAGMA foreign_keys = OFF`, so
/// phase 2 must null it explicitly, the same way it already purges
/// `task_subagents`/`learning_retrievals`/`task_watchers`.
#[tokio::test]
async fn archived_status_migration_phase_2_detaches_learnings_of_deleted_archived_tasks() {
    let board = build_archived_board().await;
    let db = migrate(&board).await;

    let source_task_id: Option<i64> = db
        .db_call(move |conn| {
            Ok(conn.query_row(
                "SELECT source_task_id FROM learnings WHERE id = ?1",
                [board.orphaned_learning],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(
        source_task_id, None,
        "a learning sourced from a deleted archived task must be detached, not left dangling"
    );
}

/// Phase 3: every archived managed ROOT clears its managed-feed command and
/// interval — whether phase 4 deletes it (reviews, empty) or keeps it (cve,
/// still holding a task) — so ProvisionManagedEpics does not re-provision
/// what the user switched off.
#[tokio::test]
async fn archived_status_migration_phase_3_clears_the_config_of_every_archived_managed_root() {
    let board = build_archived_board().await;
    let db = migrate(&board).await;

    assert_eq!(db.get_reviews_feed_command().await.unwrap(), None);
    assert_eq!(db.get_reviews_feed_interval_secs().await.unwrap(), None);
    assert_eq!(db.get_cve_feed_command().await.unwrap(), None);
    assert_eq!(db.get_cve_feed_interval_secs().await.unwrap(), None);
}

/// Phase 3's complement: a managed root that was NOT archived keeps its
/// configuration.
#[tokio::test]
async fn archived_status_migration_phase_3_leaves_a_live_managed_root_configured() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.db");
    {
        let db = Database::open(&path).await.unwrap();
        let root = db.create_epic("Reviews", "", None).await.unwrap().id;
        db.patch_epic(
            root,
            &EpicPatch::new().feed_role(crate::models::FeedRole::ReviewsParent),
        )
        .await
        .unwrap();
        db.set_reviews_feed_command(Some("reviews-cmd"))
            .await
            .unwrap();
        db.db_call(|conn| {
            conn.pragma_update(None, "user_version", LAST_VERSION_WITH_ARCHIVED)?;
            Ok(())
        })
        .await
        .unwrap();
    }
    let db = Database::open(&path).await.unwrap();
    assert_eq!(
        db.get_reviews_feed_command().await.unwrap().as_deref(),
        Some("reviews-cmd")
    );
}

/// Phase 4: an archived epic with no task left in its subtree (after phase 2)
/// is deleted — nested archived sub-epics included; one that still holds a
/// task becomes done and is recalculated, which keeps an all-done epic done
/// and regresses one holding open work to backlog.
#[tokio::test]
async fn archived_status_migration_phase_4_deletes_emptied_epics_and_settles_the_rest() {
    let board = build_archived_board().await;
    let db = migrate(&board).await;

    for gone in [
        board.archived_epic_emptied,
        board.archived_parent,
        board.archived_child,
        board.archived_reviews_root,
    ] {
        assert!(
            db.get_epic(gone).await.unwrap().is_none(),
            "archived epic {gone:?} has no task left and must be deleted"
        );
    }

    // epics.allium's phase 4 reads literally: "if
    // epic.subtree_tasks.is_empty(): not exists epic" — the whole subtree is
    // empty of tasks, so the archived epic is deleted together with it, even
    // though the child epic's own status was never archived (it is not
    // independently reachable by `was_archived`, and a leftover, dangling
    // child would otherwise violate `epics.parent_epic_id`'s foreign key
    // once the parent is gone).
    assert!(
        db.get_epic(board.archived_epic_with_live_child)
            .await
            .unwrap()
            .is_none(),
        "an archived, task-less epic is deleted even when its child epic is not archived"
    );
    assert!(
        db.get_epic(board.live_child_of_archived)
            .await
            .unwrap()
            .is_none(),
        "the non-archived child goes with its emptied, archived parent's subtree"
    );

    let kept = db
        .get_epic(board.archived_epic_kept_done)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(kept.status, TaskStatus::Done);
    let cve = db.get_epic(board.archived_cve_root).await.unwrap().unwrap();
    assert_eq!(
        cve.status,
        TaskStatus::Done,
        "an archived managed root holding a task survives as done"
    );

    let open = db
        .get_epic(board.archived_epic_with_open_task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        open.status,
        TaskStatus::Backlog,
        "done, then recalculated over a backlog child: the regression rule sends it to backlog"
    );
    assert!(db.get_task(board.open_task).await.unwrap().is_some());

    assert!(
        db.get_epic(board.feed_root).await.unwrap().is_some()
            && db.get_epic(board.feed_sub).await.unwrap().is_some(),
        "non-archived epics are untouched"
    );
}

/// Phase 5: the status CHECK constraints are rebuilt without `archived`, so
/// no later write can bring the value back — on either table.
#[tokio::test]
async fn archived_status_migration_phase_5_rebuilds_the_status_checks_without_archived() {
    let board = build_archived_board().await;
    let db = migrate(&board).await;

    let live = board.live.0;
    let task_write = db
        .db_call(move |conn| {
            conn.execute("UPDATE tasks SET status = 'archived' WHERE id = ?1", [live])
                .map_err(anyhow::Error::from)
        })
        .await;
    assert!(
        task_write.is_err(),
        "tasks.status must no longer admit 'archived'"
    );

    let epic = board.plain_epic.0;
    let epic_write = db
        .db_call(move |conn| {
            conn.execute("UPDATE epics SET status = 'archived' WHERE id = ?1", [epic])
                .map_err(anyhow::Error::from)
        })
        .await;
    assert!(
        epic_write.is_err(),
        "epics.status must no longer admit 'archived'"
    );
}

/// A fresh database has the same constraint: the value is gone from the
/// schema, not only from migrated rows.
#[tokio::test]
async fn a_fresh_db_refuses_the_archived_status() {
    let db = unattached_db().await;
    let task_id = make_task(&db, "t").await.id.0;
    let res = db
        .db_call(move |conn| {
            conn.execute(
                "UPDATE tasks SET status = 'archived' WHERE id = ?1",
                [task_id],
            )
            .map_err(anyhow::Error::from)
        })
        .await;
    assert!(res.is_err(), "a fresh schema must not admit 'archived'");
}

/// Regression: phase 4's leaf-first loop used to recalculate a just-settled
/// archived epic (`recalculate_epic_status_inner`) immediately, in the same
/// pass that settled it — before its own ARCHIVED ANCESTOR had been resolved.
/// `recalculate_epic_status_inner` reads that ancestor's row, which still
/// held `status = 'archived'` at that point, and `TaskStatus::parse` no
/// longer accepts the value (task #4971 removed the enum variant): the
/// migration failed outright, so the board never opened. Here the archived
/// child holds a worktree task (settled to done, not deleted) directly under
/// an archived parent that has no tasks of its own.
#[tokio::test]
async fn archived_status_migration_phase_4_recalculation_does_not_choke_on_an_unresolved_archived_ancestor(
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.db");
    let (parent, child) = {
        let db = Database::open(&path).await.unwrap();
        let parent = db.create_epic("Parent", "", None).await.unwrap().id;
        let child = db.create_epic("Child", "", Some(parent)).await.unwrap().id;
        let task = task_in_epic(&db, "kept", child).await;
        set_worktree(&db, task, "/wt/kept").await;
        force_archived(&db, vec![task], vec![parent, child]).await;
        db.db_call(|conn| {
            conn.pragma_update(None, "user_version", LAST_VERSION_WITH_ARCHIVED)?;
            Ok(())
        })
        .await
        .unwrap();
        (parent, child)
    };

    // Reopening runs the migration. It must succeed rather than fail with
    // "unrecognised epic_status value: archived".
    let db = Database::open(&path).await.unwrap();

    let child_epic = db.get_epic(child).await.unwrap().unwrap();
    assert_eq!(
        child_epic.status,
        TaskStatus::Done,
        "the worktree-holding archived child survives as done"
    );
    let parent_epic = db.get_epic(parent).await.unwrap().unwrap();
    assert_eq!(
        parent_epic.status,
        TaskStatus::Done,
        "the parent is recalculated from its one done child, once the child \
         is actually settled"
    );
}

/// The sibling shape of the same regression: a LIVE (never archived) parent
/// with three archived children processed in the same leaf-first round. The
/// first one settled (holding a worktree task) triggered a recalculation
/// that walked up to the live parent and read its sibling epics' statuses —
/// which still held `archived` at that point, since they had not been
/// processed yet within the same round.
#[tokio::test]
async fn archived_status_migration_phase_4_recalculation_does_not_choke_on_an_unresolved_archived_sibling(
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.db");
    let (parent, kept, emptied_1, emptied_2) = {
        let db = Database::open(&path).await.unwrap();
        let parent = db.create_epic("Parent", "", None).await.unwrap().id;
        let kept = db.create_epic("Y", "", Some(parent)).await.unwrap().id;
        let emptied_1 = db.create_epic("Z", "", Some(parent)).await.unwrap().id;
        let emptied_2 = db.create_epic("Z2", "", Some(parent)).await.unwrap().id;
        let task = task_in_epic(&db, "kept", kept).await;
        set_worktree(&db, task, "/wt/kept").await;
        force_archived(&db, vec![task], vec![kept, emptied_1, emptied_2]).await;
        db.db_call(|conn| {
            conn.pragma_update(None, "user_version", LAST_VERSION_WITH_ARCHIVED)?;
            Ok(())
        })
        .await
        .unwrap();
        (parent, kept, emptied_1, emptied_2)
    };

    // Reopening runs the migration. It must succeed rather than fail with
    // "unknown epic status \"archived\" in recalc".
    let db = Database::open(&path).await.unwrap();

    let kept_epic = db.get_epic(kept).await.unwrap().unwrap();
    assert_eq!(
        kept_epic.status,
        TaskStatus::Done,
        "the worktree-holding archived sibling survives as done"
    );
    assert!(
        db.get_epic(emptied_1).await.unwrap().is_none(),
        "an empty archived sibling is deleted"
    );
    assert!(
        db.get_epic(emptied_2).await.unwrap().is_none(),
        "so is the other one"
    );
    let parent_epic = db.get_epic(parent).await.unwrap().unwrap();
    assert_eq!(
        parent_epic.status,
        TaskStatus::Done,
        "the live parent is recalculated from its one surviving, done child \
         once every archived sibling has actually been resolved"
    );
}
