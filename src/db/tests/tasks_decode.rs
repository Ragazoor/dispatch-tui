use super::*;
use crate::models::test_tmux_window;

// ---------------------------------------------------------------------------
// Decode-failure policy: skip-and-warn for bulk reads, fail-loud for
// single-entity reads. See the decode-failure-policy section of
// docs/conventions.md.
// ---------------------------------------------------------------------------

/// Plant an undecodable task row (unrecognised `status`) alongside a healthy
/// one and return the healthy task's id.
async fn db_with_undecodable_status_row() -> (Database, TaskId) {
    let db = unattached_db().await;
    let good = create_task_returning(&db, "healthy", "", "/repo", None, TaskStatus::Backlog)
        .await
        .unwrap();
    write_corrupt_row(
        &db,
        "INSERT INTO tasks (id, title, description, repo_path, status, sub_status,
                            base_branch, created_at, updated_at)
         VALUES (9001, 'corrupt', '', '/repo', 'not_a_status', 'none', 'main',
                 '2026-01-01 00:00:00', '2026-01-01 00:00:00');",
    )
    .await;
    (db, good.id)
}

#[tokio::test]
async fn list_all_skips_row_with_unrecognised_status() {
    // Covers the skip-and-warn behaviour for every bulk task read: they all
    // funnel their `query_map` iterator through the same `collect_decodable`
    // row decoder (see "Bulk reads skip and warn" in docs/conventions.md).
    let (db, good_id) = db_with_undecodable_status_row().await;
    let before = crate::db::decode_fallback_count();

    let tasks = db
        .list_all()
        .await
        .expect("one undecodable row must not fail the whole board load");

    assert_eq!(
        tasks.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![good_id],
        "the healthy row must still load; the corrupt row must be skipped"
    );
    assert!(
        crate::db::decode_fallback_count() > before,
        "skipping a row must bump the decode-fallback counter"
    );
}

/// A `tmux_window` value that is not a window name — a pane id, or the empty
/// string — soft-fails to `None` rather than failing the row. The task then
/// reads back as owning no window, which is exactly what a task whose agent is
/// gone looks like; failing the row would drop the card from every bulk read.
#[tokio::test]
async fn malformed_stored_tmux_window_decodes_to_none() {
    for stored in ["%3", ""] {
        let db = unattached_db().await;
        let task = create_task_returning(&db, "windowed", "", "/repo", None, TaskStatus::Running)
            .await
            .unwrap();
        db.db_call(move |conn| {
            conn.execute(
                "UPDATE tasks SET tmux_window = ?1 WHERE id = ?2",
                rusqlite::params![stored, task.id.0],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        let before = crate::db::decode_fallback_count();

        let read = db.get_task(task.id).await.unwrap().unwrap();

        assert_eq!(read.tmux_window, None, "stored {stored:?}");
        assert_eq!(read.title, "windowed", "the rest of the row must survive");
        // The empty string reads back as SQL NULL-equivalent "no window" too,
        // but only a non-empty malformed value is a decode fallback worth
        // counting — an empty column is indistinguishable from an unset one.
        if !stored.is_empty() {
            assert!(
                crate::db::decode_fallback_count() > before,
                "a malformed stored window must bump the decode-fallback counter"
            );
        }
    }
}

/// A window name written by an older binary that this one has no opinion about
/// still round-trips: `parse` only rejects the two strings that are not window
/// names, so an unfamiliar-but-valid name is preserved verbatim.
#[tokio::test]
async fn unfamiliar_stored_tmux_window_round_trips() {
    let db = unattached_db().await;
    let task = create_task_returning(&db, "windowed", "", "/repo", None, TaskStatus::Running)
        .await
        .unwrap();
    db.db_call(move |conn| {
        conn.execute(
            "UPDATE tasks SET tmux_window = 'session:1-legacy' WHERE id = ?1",
            rusqlite::params![task.id.0],
        )?;
        Ok(())
    })
    .await
    .unwrap();

    let read = db.get_task(task.id).await.unwrap().unwrap();

    assert_eq!(
        read.tmux_window.as_ref().map(|w| w.as_str()),
        Some("session:1-legacy")
    );
}

#[tokio::test]
async fn get_task_errors_on_unrecognised_status() {
    let (db, _) = db_with_undecodable_status_row().await;
    let result = db.get_task(TaskId(9001)).await;
    assert!(
        result.is_err(),
        "a single-entity read must fail loudly for the row the caller asked for, got {result:?}"
    );
    let msg = format!("{:#}", result.unwrap_err());
    assert!(
        msg.contains("not_a_status"),
        "error must name the offending value, got: {msg}"
    );
}

/// Plant a task row whose `url`/`url_type` pair is inconsistent — a state the
/// application can never write, but which a partially-applied migration could
/// leave behind.
async fn db_with_inconsistent_url_row() -> (Database, TaskId, TaskId) {
    let db = unattached_db().await;
    let good = create_task_returning(&db, "healthy", "", "/repo", None, TaskStatus::Backlog)
        .await
        .unwrap();
    let bad = create_task_returning(&db, "corrupt", "", "/repo", None, TaskStatus::Backlog)
        .await
        .unwrap();
    let bad_id = bad.id.0;
    db.db_call(move |conn| {
        conn.execute(
            "UPDATE tasks SET url = 'https://example.com/pull/1', url_type = NULL WHERE id = ?1",
            rusqlite::params![bad_id],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    (db, good.id, bad.id)
}

#[tokio::test]
async fn list_all_skips_row_with_inconsistent_url_pair() {
    let (db, good_id, _) = db_with_inconsistent_url_row().await;
    let tasks = db
        .list_all()
        .await
        .expect("an inconsistent url/url_type pair must not fail the whole board load");
    assert_eq!(
        tasks.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![good_id]
    );
}

#[tokio::test]
async fn get_task_errors_on_inconsistent_url_pair() {
    let (db, _, bad_id) = db_with_inconsistent_url_row().await;
    let result = db.get_task(bad_id).await;
    assert!(
        result.is_err(),
        "a single-entity read must fail loudly on a corrupt url/url_type pair, got {result:?}"
    );
}

#[tokio::test]
async fn find_task_by_plan_errors_on_undecodable_row() {
    let db = unattached_db().await;
    write_corrupt_row(
        &db,
        "INSERT INTO tasks (id, title, description, repo_path, status, sub_status,
                            base_branch, plan_path, created_at, updated_at)
         VALUES (9002, 'corrupt', '', '/repo', 'not_a_status', 'none', 'main', '/p/plan.md',
                 '2026-01-01 00:00:00', '2026-01-01 00:00:00');",
    )
    .await;
    let result = db.find_task_by_plan("/p/plan.md").await;
    assert!(
        result.is_err(),
        "find_task_by_plan targets one row, so it must fail loudly, got {result:?}"
    );
}

/// `pr_unreachable` needs the tasks table's `(status, sub_status)` CHECK
/// constraint to admit it for `review`, which migration v96 adds. Without that
/// migration the patch below fails at the DB layer even though
/// `SubStatus::is_valid_for` allows it (pr-workflow.allium: PrPollGaveUp).
#[tokio::test]
async fn task_sub_status_pr_unreachable_persists_for_review() {
    let db = Database::open_in_memory().await.unwrap();
    let id = db
        .create_task(CreateTaskRequest {
            title: "Test",
            description: "desc",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Review,
            base_branch: "main",
            epic_id: None,
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();
    db.patch_task(
        id,
        &TaskPatch::default().sub_status(SubStatus::PrUnreachable),
    )
    .await
    .unwrap();
    let task = db.get_task(id).await.unwrap().unwrap();
    assert_eq!(task.sub_status, SubStatus::PrUnreachable);
}

/// `list_live_agent_tasks` is the SQL mirror of `Task::is_live_agent`: every
/// task it returns satisfies the predicate, every task the predicate accepts
/// is returned, and they come back ordered by id.
#[tokio::test]
async fn list_live_agent_tasks_returns_exactly_the_live_agents_by_id() {
    let db = in_memory_db().await;
    let mut ids = Vec::new();
    for (status, window) in [
        (TaskStatus::Review, true),
        (TaskStatus::Running, true),
        (TaskStatus::Running, false),
        (TaskStatus::Backlog, true),
        (TaskStatus::Done, true),
    ] {
        let task = create_task_returning(&db, "t", "d", "/repo", None, status)
            .await
            .unwrap();
        if window {
            let window = test_tmux_window(&format!("task-{}", task.id));
            db.patch_task(task.id, &TaskPatch::new().tmux_window(Some(&window)))
                .await
                .unwrap();
        }
        ids.push(task.id);
    }

    let live = db.list_live_agent_tasks().await.unwrap();
    assert_eq!(
        live.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![ids[0], ids[1]]
    );
    let all = db.list_all().await.unwrap();
    let expected: Vec<_> = all
        .iter()
        .filter(|t| t.is_live_agent())
        .map(|t| t.id)
        .collect();
    assert_eq!(live.iter().map(|t| t.id).collect::<Vec<_>>(), expected);
}

/// `DeleteEpicRefused` (epics.allium): the delete pre-check must be able to
/// ask which tasks of an epic the bulk read skipped, since a skipped row is
/// still in the subtree and its status is unknown.
#[tokio::test]
async fn undecodable_task_ids_for_epic_lists_the_skipped_rows_of_that_epic() {
    let db = unattached_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    write_corrupt_row(
        &db,
        "INSERT INTO tasks (id, title, description, repo_path, status, sub_status,
                            base_branch, created_at, updated_at, epic_id)
         VALUES (9001, 'corrupt', '', '/repo', 'not_a_status', 'none', 'main',
                 '2026-01-01 00:00:00', '2026-01-01 00:00:00', (SELECT MIN(id) FROM epics));",
    )
    .await;
    let healthy = create_task_returning(&db, "ok", "", "/repo", None, TaskStatus::Done)
        .await
        .unwrap();

    let ids = db
        .list_undecodable_task_ids_for_epic(epic.id)
        .await
        .unwrap();

    assert_eq!(ids, vec![TaskId(9001)]);
    assert!(!ids.contains(&healthy.id));
}

#[tokio::test]
async fn epic_delete_refusal_counts_an_undecodable_subtree_task() {
    use crate::service::{EpicService, ServiceError};
    let db = std::sync::Arc::new(unattached_db().await);
    let epic = db.create_epic("E", "", None).await.unwrap();
    write_corrupt_row(
        &db,
        "INSERT INTO tasks (id, title, description, repo_path, status, sub_status,
                            base_branch, created_at, updated_at, epic_id)
         VALUES (9001, 'corrupt', '', '/repo', 'not_a_status', 'none', 'main',
                 '2026-01-01 00:00:00', '2026-01-01 00:00:00', (SELECT MIN(id) FROM epics));",
    )
    .await;
    let svc = EpicService::new(db.clone(), db.clone());

    let err = svc.ensure_deletable(epic.id).await.unwrap_err();

    let ServiceError::Validation(msg) = err else {
        panic!("expected Validation, got {err:?}");
    };
    assert!(msg.contains("could not be read"), "{msg}");
    assert!(msg.contains("#9001"), "{msg}");
}
