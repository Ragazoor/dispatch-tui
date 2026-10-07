use super::*;

#[tokio::test]
async fn upsert_feed_tasks_creates_tasks() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![
        make_feed_item("ext-1", "Task One"),
        make_feed_item("ext-2", "Task Two"),
    ];
    let repo_paths = vec!["/repo".to_string(), "/repo".to_string()];
    let branches = main_branches(items.len());

    db.upsert_feed_tasks(epic.id, &items, &repo_paths, &branches)
        .await
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 2);
    let mut titles: Vec<&str> = tasks.iter().map(|t| t.title.as_str()).collect();
    titles.sort();
    assert_eq!(titles, vec!["Task One", "Task Two"]);
    assert!(tasks.iter().all(|t| t.status == TaskStatus::Backlog));
    assert!(tasks
        .iter()
        .all(|t| t.external_id.as_deref() == Some("ext-1")
            || t.external_id.as_deref() == Some("ext-2")));
}

#[tokio::test]
async fn upsert_feed_tasks_rejects_mismatched_slice_lengths() {
    // The three slices are parallel-to-items by contract. A length mismatch
    // would silently truncate via zip and drop feed items, so it must error
    // explicitly instead.
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![
        make_feed_item("ext-1", "Task One"),
        make_feed_item("ext-2", "Task Two"),
    ];

    // repo_paths shorter than items
    let err = db
        .upsert_feed_tasks(epic.id, &items, &["/repo".to_string()], &main_branches(2))
        .await
        .expect_err("mismatched repo_paths length must error");
    assert!(
        err.to_string().contains("length"),
        "error should mention length mismatch, got: {err}"
    );

    // base_branches shorter than items
    let err = db
        .upsert_feed_tasks(
            epic.id,
            &items,
            &["/repo".to_string(), "/repo".to_string()],
            &main_branches(1),
        )
        .await
        .expect_err("mismatched base_branches length must error");
    assert!(
        err.to_string().contains("length"),
        "error should mention length mismatch, got: {err}"
    );

    // No tasks should have been written on either failed call.
    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert!(tasks.is_empty(), "no tasks should be written on mismatch");
}

#[tokio::test]
async fn upsert_feed_tasks_idempotent() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![make_feed_item("ext-1", "Task One")];
    let repo_paths = vec!["/repo".to_string()];
    let branches = main_branches(items.len());

    db.upsert_feed_tasks(epic.id, &items, &repo_paths, &branches)
        .await
        .unwrap();
    db.upsert_feed_tasks(epic.id, &items, &repo_paths, &branches)
        .await
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 1, "second call should not create duplicate");
    assert_eq!(tasks[0].title, "Task One");
}

/// The claim AppendOnlyFeed's "HOW SUCH A TASK IS EVER CLOSED" rests on
/// (feeds.allium): the user completes a feed task and deletes it, the delete
/// writes a RetiredFeedItem keyed on the feed epic, and no later emission of
/// the same external_id inserts a task for it — however many times it recurs.
/// This replaces the archived-row suppression the archive status used to
/// provide: deletion IS suppression now, not its opposite.
#[tokio::test]
async fn upsert_feed_tasks_inserts_no_task_for_a_retired_external_id() {
    let db = in_memory_db().await;
    let epic = feed_epic(&db, "Log warnings").await;
    let items = vec![make_feed_item("log:WARN:mod:a warning", "warn A")];

    upsert(&db, epic.id, &items).await;
    complete_and_delete(&db, epic.id, "log:WARN:mod:a warning").await;

    // The record is still in the log, so the script emits it again — forever.
    for _ in 0..3 {
        upsert(&db, epic.id, &items).await;
    }

    assert!(
        db.list_tasks_for_epic(epic.id).await.unwrap().is_empty(),
        "a deleted feed task's external_id is retired: re-emission must insert nothing"
    );
}

/// The additive path (AppendOnlyFeed, DegradedNonEmptyEmission) inserts
/// through the same UpsertFeedTasks clause, so it refuses a retired id too.
#[tokio::test]
async fn upsert_feed_tasks_additive_inserts_no_task_for_a_retired_external_id() {
    let db = in_memory_db().await;
    let epic = feed_epic(&db, "Append-only").await;
    let items = vec![make_feed_item("ext-1", "One")];

    upsert(&db, epic.id, &items).await;
    complete_and_delete(&db, epic.id, "ext-1").await;

    db.upsert_feed_tasks_additive(epic.id, &items, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    assert!(
        db.list_tasks_for_epic(epic.id).await.unwrap().is_empty(),
        "the additive upsert must refuse a retired id exactly like the reconcile one"
    );
}

/// Retirement suppresses insertion only. A new id in the same emission is
/// inserted as usual beside the refused one.
#[tokio::test]
async fn upsert_feed_tasks_still_inserts_ids_that_are_not_retired() {
    let db = in_memory_db().await;
    let epic = feed_epic(&db, "Feed").await;

    upsert(&db, epic.id, &[make_feed_item("gone", "Gone")]).await;
    complete_and_delete(&db, epic.id, "gone").await;

    upsert(
        &db,
        epic.id,
        &[make_feed_item("gone", "Gone"), make_feed_item("new", "New")],
    )
    .await;

    let ids: Vec<String> = db
        .list_tasks_for_epic(epic.id)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|t| t.external_id)
        .collect();
    assert_eq!(ids, vec!["new".to_string()]);
}

/// "Suppression covers insertion only" (feeds.allium, Retired feed items): a
/// task still on the board with a retired id is matched and refreshed like
/// any other feed task, not filtered out of the emission.
#[tokio::test]
async fn upsert_feed_tasks_refreshes_a_task_still_on_the_board_with_a_retired_id() {
    let db = in_memory_db().await;
    let epic = feed_epic(&db, "Feed").await;

    // Retire "dup" by deleting one task for it, while a second task with the
    // same id — moved in from outside the feed, which the subtree uniqueness
    // triggers allow once the first is gone — stays on the board.
    upsert(&db, epic.id, &[make_feed_item("dup", "Old title")]).await;
    let survivor = make_task(&db, "survivor").await;
    db.patch_task(survivor.id, &TaskPatch::new().external_id(Some("dup")))
        .await
        .unwrap();
    complete_and_delete(&db, epic.id, "dup").await;
    db.set_task_epic_id(survivor.id, Some(epic.id))
        .await
        .unwrap();

    upsert(&db, epic.id, &[make_feed_item("dup", "New title")]).await;

    let tasks = tasks_with_external_id(&db, epic.id, "dup").await;
    assert_eq!(
        tasks.len(),
        1,
        "the surviving row is matched, not duplicated"
    );
    assert_eq!(tasks[0].id, survivor.id);
    assert_eq!(
        tasks[0].title, "New title",
        "the surviving row is refreshed like any other feed task"
    );
}

/// DeleteTask keys the record on the task's `nearest_feed_epic`. For a task
/// in a grouped feed's repo sub-epic — which carries no command of its own —
/// that is the root, so the record suppresses the id under the root's whole
/// subtree, including a sub-epic created afresh after the old one is gone.
#[tokio::test]
async fn delete_task_in_a_feed_sub_epic_retires_under_the_nearest_feed_epic() {
    let db = in_memory_db().await;
    let root = feed_epic(&db, "Grouped feed").await;
    let sub = db.create_epic("repo-a", "", Some(root.id)).await.unwrap();

    upsert(&db, sub.id, &[make_feed_item("pr-1", "PR 1")]).await;
    complete_and_delete(&db, sub.id, "pr-1").await;

    // A second repo sub-epic of the same root: the id is retired under the
    // root, so it is refused here too.
    let other_sub = db.create_epic("repo-b", "", Some(root.id)).await.unwrap();
    upsert(&db, other_sub.id, &[make_feed_item("pr-1", "PR 1")]).await;
    upsert(&db, sub.id, &[make_feed_item("pr-1", "PR 1")]).await;

    assert!(tasks_with_external_id(&db, sub.id, "pr-1").await.is_empty());
    assert!(tasks_with_external_id(&db, other_sub.id, "pr-1")
        .await
        .is_empty());
}

/// A retired id is scoped to the feed epic it was retired under
/// (UniqueRetiredFeedItemPerFeed is per feed_epic): another feed emitting the
/// same id is unaffected.
#[tokio::test]
async fn a_retired_external_id_does_not_suppress_the_same_id_in_another_feed() {
    let db = in_memory_db().await;
    let a = feed_epic(&db, "Feed A").await;
    let b = feed_epic(&db, "Feed B").await;

    upsert(&db, a.id, &[make_feed_item("shared-id", "A's copy")]).await;
    complete_and_delete(&db, a.id, "shared-id").await;

    upsert(&db, b.id, &[make_feed_item("shared-id", "B's copy")]).await;

    assert_eq!(
        tasks_with_external_id(&db, b.id, "shared-id").await.len(),
        1
    );
}

/// "A task not under any feed epic records nothing" (DeleteTask: `retires`
/// needs a non-null feed_epic). An epic with no feed_command anywhere in its
/// chain has no cycle to suppress, so upserting into it afterwards inserts.
#[tokio::test]
async fn delete_task_under_no_feed_epic_retires_nothing() {
    let db = in_memory_db().await;
    let plain = db.create_epic("Not a feed", "", None).await.unwrap();

    upsert(&db, plain.id, &[make_feed_item("ext-1", "One")]).await;
    complete_and_delete(&db, plain.id, "ext-1").await;

    upsert(&db, plain.id, &[make_feed_item("ext-1", "One")]).await;
    assert_eq!(
        tasks_with_external_id(&db, plain.id, "ext-1").await.len(),
        1,
        "no feed epic in the chain means no record, so the id is inserted again"
    );
}

/// "Moving a feed task out of its feed detaches it" (design doc): retirement
/// is keyed by the task's chain AT DELETE TIME. A feed task moved to no epic
/// and then deleted records nothing for the feed it came from.
#[tokio::test]
async fn delete_task_moved_out_of_its_feed_retires_nothing_for_that_feed() {
    let db = in_memory_db().await;
    let epic = feed_epic(&db, "Feed").await;

    upsert(&db, epic.id, &[make_feed_item("ext-1", "One")]).await;
    let id = tasks_with_external_id(&db, epic.id, "ext-1").await[0].id;
    db.set_task_epic_id(id, None).await.unwrap();
    db.patch_task(id, &TaskPatch::new().status(TaskStatus::Done))
        .await
        .unwrap();
    db.delete_task(id).await.unwrap();

    upsert(&db, epic.id, &[make_feed_item("ext-1", "One")]).await;
    assert_eq!(
        tasks_with_external_id(&db, epic.id, "ext-1").await.len(),
        1,
        "a detached task's delete must not retire the id under its old feed"
    );
}

/// UniqueRetiredFeedItemPerFeed (core.allium): "A second delete of the same
/// id under the same feed is a no-op on this table." Reaching a second delete
/// needs a second row carrying the id — here one moved in from outside the
/// feed after the first was retired, the same shape the migration's
/// worktree-holding survivors produce. The second delete must succeed (a
/// unique-constraint error would leave the row on the board) and the id must
/// stay retired.
#[tokio::test]
async fn deleting_a_second_task_for_an_already_retired_id_is_a_no_op_on_the_record() {
    let db = in_memory_db().await;
    let epic = feed_epic(&db, "Feed").await;

    upsert(&db, epic.id, &[make_feed_item("dup", "First")]).await;
    let second = make_task(&db, "second").await;
    db.patch_task(second.id, &TaskPatch::new().external_id(Some("dup")))
        .await
        .unwrap();
    complete_and_delete(&db, epic.id, "dup").await;

    db.set_task_epic_id(second.id, Some(epic.id)).await.unwrap();
    db.patch_task(second.id, &TaskPatch::new().status(TaskStatus::Done))
        .await
        .unwrap();
    db.delete_task(second.id)
        .await
        .expect("retiring an id that is already retired must not fail the delete");
    assert!(db.get_task(second.id).await.unwrap().is_none());

    upsert(&db, epic.id, &[make_feed_item("dup", "Again")]).await;
    assert!(
        tasks_with_external_id(&db, epic.id, "dup").await.is_empty(),
        "the id stays retired after the no-op second retirement"
    );
}
