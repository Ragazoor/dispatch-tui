use super::*;
use crate::models::test_tmux_window;

// ---------------------------------------------------------------------------
// upsert_feed_tasks
// ---------------------------------------------------------------------------

fn make_feed_item(external_id: &str, title: &str) -> crate::models::FeedItem {
    crate::models::FeedItem {
        external_id: external_id.to_string(),
        title: title.to_string(),
        description: "desc".to_string(),
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

/// Build a parallel vec of "main" base branches for tests that don't
/// exercise the per-task base_branch path.
fn main_branches(n: usize) -> Vec<String> {
    vec!["main".to_string(); n]
}

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

// ---------------------------------------------------------------------------
// Retired feed items (core.allium: RetiredFeedItem; tasks.allium: DeleteTask;
// feeds.allium: IngestSkipsRetiredFeedItems)
// ---------------------------------------------------------------------------
//
// Every assertion here is behavioural: a retired id is observed through the
// only thing it changes, which is whether the next upsert inserts a task for
// it. That keeps these tests independent of how the record is stored.

/// A root epic that carries a `feed_command`, i.e. one whose cycle emits items
/// and so is the `nearest_feed_epic` a deletion retires under.
async fn feed_epic(db: &Database, title: &str) -> Epic {
    let epic = db.create_epic(title, "", None).await.unwrap();
    db.patch_epic(epic.id, &EpicPatch::new().feed_command(Some("echo []")))
        .await
        .unwrap();
    epic
}

/// Upsert `items` into `epic` with placeholder repo paths and base branches.
async fn upsert(db: &Database, epic: EpicId, items: &[crate::models::FeedItem]) {
    db.upsert_feed_tasks(
        epic,
        items,
        &vec!["/repo".to_string(); items.len()],
        &main_branches(items.len()),
    )
    .await
    .unwrap();
}

/// The only id-to-task lookup these tests need: every task under `epic`
/// carrying `external_id`.
async fn tasks_with_external_id(db: &Database, epic: EpicId, external_id: &str) -> Vec<Task> {
    db.list_tasks_for_epic(epic)
        .await
        .unwrap()
        .into_iter()
        .filter(|t| t.external_id.as_deref() == Some(external_id))
        .collect()
}

/// Complete and delete the single task under `epic` that carries
/// `external_id` — the DeleteTask gesture (`x` on a Done card), which requires
/// status = done.
async fn complete_and_delete(db: &Database, epic: EpicId, external_id: &str) -> TaskId {
    let tasks = tasks_with_external_id(db, epic, external_id).await;
    assert_eq!(
        tasks.len(),
        1,
        "fixture: exactly one task for {external_id}"
    );
    let id = tasks[0].id;
    db.patch_task(id, &TaskPatch::new().status(TaskStatus::Done))
        .await
        .unwrap();
    db.delete_task(id).await.unwrap();
    id
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

/// A manual task (no external_id) under a feed epic has nothing to retire;
/// deleting it must still simply remove the row.
#[tokio::test]
async fn delete_task_without_external_id_under_a_feed_epic_just_removes_the_row() {
    let db = unattached_db().await;
    let epic = feed_epic(&db, "Feed").await;
    let manual = make_task(&db, "manual").await;
    db.set_task_epic_id(manual.id, Some(epic.id)).await.unwrap();

    db.delete_task(manual.id).await.unwrap();
    assert!(db.get_task(manual.id).await.unwrap().is_none());
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

#[tokio::test]
async fn upsert_feed_tasks_preserves_status() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![make_feed_item("ext-1", "Original Title")];

    db.upsert_feed_tasks(epic.id, &items, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    // Simulate user moving task to Running
    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    db.patch_task(tasks[0].id, &TaskPatch::new().status(TaskStatus::Running))
        .await
        .unwrap();

    // Re-run upsert with updated title and different status
    let updated = vec![crate::models::FeedItem {
        external_id: "ext-1".to_string(),
        title: "Updated Title".to_string(),
        description: "new desc".to_string(),
        url: String::new(),
        url_type: None,
        status: TaskStatus::Done, // feed says done; user status should be preserved
        tag: crate::models::TaskTag::Bug,
        labels: Vec::new(),
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    }];
    db.upsert_feed_tasks(epic.id, &updated, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "Updated Title", "title should be updated");
    assert_eq!(
        tasks[0].description, "new desc",
        "description should be updated"
    );
    assert_eq!(
        tasks[0].status,
        TaskStatus::Running,
        "user-managed status must be preserved"
    );
}

/// The done exception is gone. `sort_order` used to double as the Done
/// column's completion rank, so a feed's severity-rank re-poll would clobber
/// it and the upsert skipped done tasks to protect it. The rank is
/// `completed_at` now, which no feed field can reach, so the feed's value
/// applies to a done task like any other — and the completion survives
/// untouched beside it.
#[tokio::test]
async fn upsert_feed_tasks_updates_a_done_tasks_sort_order_and_keeps_its_completion() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![make_feed_item("ext-1", "Original Title")];

    db.upsert_feed_tasks(epic.id, &items, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    let finished = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();
    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    db.patch_task(
        tasks[0].id,
        &TaskPatch::new()
            .status(TaskStatus::Done)
            .completed_at(Some(finished)),
    )
    .await
    .unwrap();

    let mut updated_item = make_feed_item("ext-1", "Original Title");
    updated_item.sort_order = Some(1); // feed severity rank
    db.upsert_feed_tasks(
        epic.id,
        &[updated_item],
        &["/repo".to_string()],
        &main_branches(1),
    )
    .await
    .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].sort_order,
        Some(1),
        "the feed's severity rank applies to a done task too"
    );
    assert_eq!(
        tasks[0].completed_at,
        Some(finished),
        "and the completion time is untouched by a re-poll"
    );
}

/// An item that arrives already done is stamped on INSERT, because it never
/// passes through the status transition that would otherwise stamp it. Without
/// this the card sinks to the bottom of the Done column rather than leading it
/// (board-layout.allium, "Done Column Ordering").
#[tokio::test]
async fn upsert_feed_tasks_stamps_a_task_inserted_straight_into_done() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let mut item = make_feed_item("ext-1", "Already finished");
    item.status = TaskStatus::Done;

    let before = chrono::Utc::now() - chrono::Duration::seconds(1);
    db.upsert_feed_tasks(epic.id, &[item], &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();
    let after = chrono::Utc::now() + chrono::Duration::seconds(1);

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    let completed_at = tasks[0]
        .completed_at
        .expect("a card born in Done carries a completion time");
    assert!(
        (before..=after).contains(&completed_at),
        "completed_at {completed_at} should be about now, within [{before}, {after}]"
    );
}

/// A card inserted NOT done takes no stamp.
/// A row inserted straight into Done is stamped by `insert_task_row` too, not
/// only by the feed upsert — "landing in done stamps" has one owner whatever
/// route the row took in. Latent today (every production caller passes
/// Backlog), and here so it stays closed.
#[tokio::test]
async fn create_task_stamps_a_task_created_straight_into_done() {
    let db = in_memory_db().await;
    let before = chrono::Utc::now() - chrono::Duration::seconds(1);
    let id = db
        .create_task(CreateTaskRequest {
            title: "Already finished",
            description: "d",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Done,
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
    let after = chrono::Utc::now() + chrono::Duration::seconds(1);

    let completed_at = db
        .get_task(id)
        .await
        .unwrap()
        .unwrap()
        .completed_at
        .expect("a task created in Done carries a completion time");
    assert!(
        (before..=after).contains(&completed_at),
        "completed_at {completed_at} should be about now, within [{before}, {after}]"
    );
}

/// The complement: a create outside Done takes no stamp.
#[tokio::test]
async fn create_task_does_not_stamp_a_task_created_outside_done() {
    let db = in_memory_db().await;
    let id = db
        .create_task(CreateTaskRequest {
            title: "Open",
            description: "d",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
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

    assert_eq!(db.get_task(id).await.unwrap().unwrap().completed_at, None);
}

#[tokio::test]
async fn upsert_feed_tasks_does_not_stamp_a_task_inserted_outside_done() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();

    db.upsert_feed_tasks(
        epic.id,
        &[make_feed_item("ext-1", "Open")],
        &["/repo".to_string()],
        &main_branches(1),
    )
    .await
    .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks[0].completed_at, None);
}

#[tokio::test]
async fn upsert_feed_tasks_still_updates_sort_order_when_task_is_not_done() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![make_feed_item("ext-1", "Original Title")];

    db.upsert_feed_tasks(epic.id, &items, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    let mut updated_item = make_feed_item("ext-1", "Original Title");
    updated_item.sort_order = Some(7);
    db.upsert_feed_tasks(
        epic.id,
        &[updated_item],
        &["/repo".to_string()],
        &main_branches(1),
    )
    .await
    .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks[0].sort_order, Some(7));
}

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

#[tokio::test]
async fn upsert_feed_tasks_uses_resolved_repo_path() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![make_feed_item("ext-1", "Task One")];
    let repo_paths = vec!["/resolved/local/repo".to_string()];
    let branches = main_branches(items.len());

    db.upsert_feed_tasks(epic.id, &items, &repo_paths, &branches)
        .await
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks[0].repo_path, "/resolved/local/repo");
}

#[tokio::test]
async fn upsert_feed_tasks_stores_empty_sentinel_when_unresolved() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![make_feed_item("ext-1", "Task One")];
    let repo_paths = vec!["".to_string()];
    let branches = main_branches(items.len());

    db.upsert_feed_tasks(epic.id, &items, &repo_paths, &branches)
        .await
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks[0].repo_path, "");
}

#[tokio::test]
async fn upsert_feed_tasks_on_conflict_does_not_update_repo_path() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![make_feed_item("ext-1", "Original")];

    // First upsert: resolved path stored
    db.upsert_feed_tasks(
        epic.id,
        &items,
        &["/first/path".to_string()],
        &main_branches(1),
    )
    .await
    .unwrap();
    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks[0].repo_path, "/first/path");

    // Second upsert: different path provided — ON CONFLICT should NOT update repo_path
    let updated = vec![crate::models::FeedItem {
        external_id: "ext-1".to_string(),
        title: "Updated Title".to_string(),
        description: "new desc".to_string(),
        url: String::new(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::Bug,
        labels: Vec::new(),
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    }];
    db.upsert_feed_tasks(
        epic.id,
        &updated,
        &["/second/path".to_string()],
        &main_branches(1),
    )
    .await
    .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks[0].title, "Updated Title");
    assert_eq!(
        tasks[0].repo_path, "/first/path",
        "repo_path must not be updated on conflict"
    );
}

#[tokio::test]
async fn upsert_feed_tasks_mixed_batch_resolved_and_unresolved() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![
        make_feed_item("ext-1", "Resolved Task"),
        make_feed_item("ext-2", "Unresolved Task"),
    ];
    let repo_paths = vec!["/matched/local/path".to_string(), "".to_string()];
    let branches = main_branches(items.len());

    db.upsert_feed_tasks(epic.id, &items, &repo_paths, &branches)
        .await
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    let resolved = tasks
        .iter()
        .find(|t| t.external_id.as_deref() == Some("ext-1"))
        .unwrap();
    let unresolved = tasks
        .iter()
        .find(|t| t.external_id.as_deref() == Some("ext-2"))
        .unwrap();
    assert_eq!(resolved.repo_path, "/matched/local/path");
    assert_eq!(unresolved.repo_path, "");
}

#[tokio::test]
async fn upsert_feed_tasks_stores_per_task_base_branch() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![
        make_feed_item("ext-1", "Master Task"),
        make_feed_item("ext-2", "Develop Task"),
        make_feed_item("ext-3", "Main Task"),
    ];
    let repo_paths = vec![
        "/repo-a".to_string(),
        "/repo-b".to_string(),
        "/repo-c".to_string(),
    ];
    let base_branches = vec![
        "master".to_string(),
        "develop".to_string(),
        "main".to_string(),
    ];

    db.upsert_feed_tasks(epic.id, &items, &repo_paths, &base_branches)
        .await
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    let by_ext = |ext: &str| {
        tasks
            .iter()
            .find(|t| t.external_id.as_deref() == Some(ext))
            .unwrap()
    };
    assert_eq!(by_ext("ext-1").base_branch, "master");
    assert_eq!(by_ext("ext-2").base_branch, "develop");
    assert_eq!(by_ext("ext-3").base_branch, "main");
}

#[tokio::test]
async fn upsert_feed_tasks_does_not_remove_manual_tasks() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();

    // Manually created task linked to the epic (no external_id)
    let manual_task_id = db
        .create_task(CreateTaskRequest {
            title: "Manual",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: TaskStatus::Backlog,
            base_branch: "main",
            epic_id: Some(epic.id),
            sort_order: None,
            tag: None,
            wrap_up_mode: None,
            auto_run_plan: false,
            phoenix: false,
        })
        .await
        .unwrap();

    // Feed fetch with one item
    db.upsert_feed_tasks(
        epic.id,
        &[make_feed_item("ext-1", "Feed Task")],
        &["/repo".to_string()],
        &main_branches(1),
    )
    .await
    .unwrap();

    // Feed fetch returns nothing — only manual task should survive
    db.upsert_feed_tasks(epic.id, &[], &[], &[]).await.unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(
        tasks.len(),
        1,
        "manual task should survive empty feed fetch"
    );
    assert_eq!(tasks[0].id, manual_task_id);
}

#[tokio::test]
async fn upsert_feed_tasks_persists_tag() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![crate::models::FeedItem {
        external_id: "ext-1".to_string(),
        title: "Tagged".to_string(),
        description: "".to_string(),
        url: String::new(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::PrReview,
        labels: Vec::new(),
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    }];

    db.upsert_feed_tasks(epic.id, &items, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].tag, Some(crate::models::TaskTag::PrReview));
}

#[tokio::test]
async fn upsert_feed_tasks_updates_tag_on_conflict() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let initial = vec![crate::models::FeedItem {
        external_id: "ext-1".to_string(),
        title: "T".to_string(),
        description: "".to_string(),
        url: String::new(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::PrReview,
        labels: Vec::new(),
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    }];
    db.upsert_feed_tasks(epic.id, &initial, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    // Re-emit the same item with a different tag — feed is the source of truth.
    let updated = vec![crate::models::FeedItem {
        external_id: "ext-1".to_string(),
        title: "T".to_string(),
        description: "".to_string(),
        url: String::new(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::Fix,
        labels: Vec::new(),
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    }];
    db.upsert_feed_tasks(epic.id, &updated, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].tag, Some(crate::models::TaskTag::Fix));
}

#[tokio::test]
async fn feed_item_legacy_json_deserializes_with_default_labels_and_sort_order() {
    // Wire-compat: scripts written before labels/sort_order existed must still
    // parse. Both fields are #[serde(default)].
    let legacy_json = r#"{
        "external_id": "ext-1",
        "title": "Legacy",
        "description": "",
        "url": "",
        "status": "backlog",
        "tag": "bug"
    }"#;
    let item: crate::models::FeedItem = serde_json::from_str(legacy_json).unwrap();
    assert!(item.labels.is_empty());
    assert_eq!(item.sort_order, None);
    // wrap_up_mode is #[serde(default)]: absent -> None.
    assert_eq!(item.wrap_up_mode, None);
}

#[tokio::test]
async fn feed_item_deserializes_wrap_up_mode() {
    // A feed script may declare wrap_up_mode; "pr" parses to WrapUpMode::Pr
    // (WrapUpMode derives Deserialize with rename_all = "lowercase").
    let json = r#"{
        "external_id": "cve:org/repo#1",
        "title": "[CRITICAL] repo: CVE-1",
        "description": "",
        "status": "backlog",
        "tag": "fix",
        "wrap_up_mode": "pr"
    }"#;
    let item: crate::models::FeedItem = serde_json::from_str(json).unwrap();
    assert_eq!(item.wrap_up_mode, Some(crate::models::WrapUpMode::Pr));
}

#[tokio::test]
async fn upsert_feed_tasks_writes_labels_and_sort_order_on_insert() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![crate::models::FeedItem {
        external_id: "ext-1".to_string(),
        title: "CRITICAL CVE-1234".to_string(),
        description: "".to_string(),
        url: String::new(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::Fix,
        labels: vec!["scala-common".to_string()],
        sort_order: Some(1),
        signals: vec![],
        wrap_up_mode: None,
    }];
    db.upsert_feed_tasks(epic.id, &items, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].labels, vec!["scala-common".to_string()]);
    assert_eq!(tasks[0].sort_order, Some(1));
}

#[tokio::test]
async fn upsert_feed_tasks_replaces_labels_and_sort_order_on_conflict() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let initial = vec![crate::models::FeedItem {
        external_id: "ext-1".to_string(),
        title: "T".to_string(),
        description: "".to_string(),
        url: String::new(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::Fix,
        labels: vec!["repo-a".to_string()],
        sort_order: Some(3),
        signals: vec![],
        wrap_up_mode: None,
    }];
    db.upsert_feed_tasks(epic.id, &initial, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();
    // Simulate user moving the task — status & repo_path must be preserved.
    let task_id = db.list_tasks_for_epic(epic.id).await.unwrap()[0].id;
    db.patch_task(
        task_id,
        &TaskPatch::new()
            .status(TaskStatus::Running)
            .repo_path("/manually-fixed"),
    )
    .await
    .unwrap();

    let updated = vec![crate::models::FeedItem {
        external_id: "ext-1".to_string(),
        title: "T".to_string(),
        description: "".to_string(),
        url: String::new(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::Fix,
        labels: vec!["repo-a".to_string(), "security".to_string()],
        sort_order: Some(1),
        signals: vec![],
        wrap_up_mode: None,
    }];
    db.upsert_feed_tasks(epic.id, &updated, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    let task = db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(
        task.labels,
        vec!["repo-a".to_string(), "security".to_string()],
        "labels are feed-controlled and replaced on conflict"
    );
    assert_eq!(
        task.sort_order,
        Some(1),
        "sort_order is replaced on conflict"
    );
    // User-owned fields preserved.
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(task.repo_path, "/manually-fixed");
}

#[tokio::test]
async fn upsert_feed_tasks_sets_wrap_up_mode_on_insert() {
    let db = in_memory_db().await;
    let epic = db.create_epic("CVE", "", None).await.unwrap();
    let items = vec![
        crate::models::FeedItem {
            sort_order: Some(1),
            wrap_up_mode: Some(crate::models::WrapUpMode::Pr),
            ..make_feed_item("cve:org/repo#1", "[CRITICAL] repo: CVE-1")
        },
        crate::models::FeedItem {
            sort_order: Some(2),
            // Omitted by the script -> stays NULL on the task.
            wrap_up_mode: None,
            ..make_feed_item("cve:org/repo#2", "[LOW] repo: CVE-2")
        },
    ];
    db.upsert_feed_tasks(
        epic.id,
        &items,
        &["/repo".to_string(), "/repo".to_string()],
        &main_branches(2),
    )
    .await
    .unwrap();

    let mut tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    tasks.sort_by_key(|t| t.sort_order);
    assert_eq!(tasks.len(), 2);
    assert_eq!(
        tasks[0].wrap_up_mode,
        Some(crate::models::WrapUpMode::Pr),
        "declared wrap_up_mode is applied on insert"
    );
    assert_eq!(
        tasks[1].wrap_up_mode, None,
        "omitted wrap_up_mode leaves the task's value NULL"
    );
}

#[tokio::test]
async fn upsert_feed_tasks_preserves_wrap_up_mode_on_conflict() {
    let db = in_memory_db().await;
    let epic = db.create_epic("CVE", "", None).await.unwrap();
    let initial = vec![crate::models::FeedItem {
        wrap_up_mode: Some(crate::models::WrapUpMode::Pr),
        ..make_feed_item("cve:org/repo#1", "T")
    }];
    db.upsert_feed_tasks(epic.id, &initial, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    // User changes the wrap-up choice manually.
    let task_id = db.list_tasks_for_epic(epic.id).await.unwrap()[0].id;
    db.patch_task(
        task_id,
        &TaskPatch::new().wrap_up_mode(Some(crate::models::WrapUpMode::Rebase)),
    )
    .await
    .unwrap();

    // Feed re-polls the same alert, still declaring "pr".
    db.upsert_feed_tasks(epic.id, &initial, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    let task = db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(
        task.wrap_up_mode,
        Some(crate::models::WrapUpMode::Rebase),
        "wrap_up_mode is insert-only; a user's manual change survives feed refreshes"
    );
}

#[tokio::test]
async fn upsert_feed_tasks_sets_pr_url_from_item_url_on_insert() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![
        crate::models::FeedItem {
            external_id: "dep:org/repo#42".to_string(),
            title: "#42 Bump foo".to_string(),
            description: "".to_string(),
            url: "https://github.com/org/repo/pull/42".to_string(),
            url_type: None,
            status: TaskStatus::Backlog,
            tag: crate::models::TaskTag::PrReview,
            labels: vec![],
            sort_order: None,
            signals: vec![],
            wrap_up_mode: None,
        },
        crate::models::FeedItem {
            external_id: "dep:org/repo#43".to_string(),
            title: "#43 Bump bar".to_string(),
            description: "".to_string(),
            url: "https://github.com/org/repo/pull/43".to_string(),
            url_type: None,
            status: TaskStatus::Backlog,
            tag: crate::models::TaskTag::Dependabot,
            labels: vec![],
            sort_order: None,
            signals: vec![],
            wrap_up_mode: None,
        },
        crate::models::FeedItem {
            external_id: "cve:GHSA-xxxx".to_string(),
            title: "CRITICAL CVE-1234".to_string(),
            description: "".to_string(),
            url: "https://github.com/org/repo/security/advisories/GHSA-xxxx".to_string(),
            url_type: None,
            status: TaskStatus::Backlog,
            tag: crate::models::TaskTag::Fix,
            labels: vec![],
            sort_order: None,
            signals: vec![],
            wrap_up_mode: None,
        },
    ];
    db.upsert_feed_tasks(
        epic.id,
        &items,
        &vec!["/repo".to_string(); 3],
        &main_branches(3),
    )
    .await
    .unwrap();

    let mut tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    tasks.sort_by(|a, b| a.external_id.cmp(&b.external_id));
    assert_eq!(tasks.len(), 3);
    assert_eq!(
        tasks[0].url.as_ref().map(|u| u.url.as_str()),
        Some("https://github.com/org/repo/security/advisories/GHSA-xxxx"),
        "non-empty url copied to url regardless of tag (Fix)"
    );
    assert_eq!(
        tasks[0].url.as_ref().map(|u| u.url_type),
        Some(crate::models::UrlType::Other),
        "non-PR/issue url inferred as other"
    );
    assert_eq!(
        tasks[1].url.as_ref().map(|u| u.url.as_str()),
        Some("https://github.com/org/repo/pull/42"),
        "PrReview items keep url-on-insert"
    );
    assert_eq!(
        tasks[1].url.as_ref().map(|u| u.url_type),
        Some(crate::models::UrlType::Pr),
        "pull url inferred as pr"
    );
    assert_eq!(
        tasks[2].url.as_ref().map(|u| u.url.as_str()),
        Some("https://github.com/org/repo/pull/43"),
        "Dependabot items get url-on-insert"
    );
}

#[tokio::test]
async fn upsert_feed_tasks_leaves_pr_url_null_when_item_url_empty() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![crate::models::FeedItem {
        external_id: "ext-no-url".to_string(),
        title: "no url".to_string(),
        description: "".to_string(),
        url: "".to_string(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::Dependabot,
        labels: vec![],
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    }];
    db.upsert_feed_tasks(epic.id, &items, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].url.is_none());
}

#[tokio::test]
async fn upsert_feed_tasks_backfills_null_pr_url_on_conflict() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    // First emission: no URL — task created with url = NULL.
    let initial = vec![crate::models::FeedItem {
        external_id: "dep:org/repo#42".to_string(),
        title: "#42 Bump foo".to_string(),
        description: "".to_string(),
        url: "".to_string(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::Dependabot,
        labels: vec![],
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    }];
    db.upsert_feed_tasks(epic.id, &initial, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();
    let task_id = db.list_tasks_for_epic(epic.id).await.unwrap()[0].id;
    assert!(
        db.get_task(task_id).await.unwrap().unwrap().url.is_none(),
        "precondition: url is null after first upsert"
    );

    // Second emission: same external_id but now with a URL.
    let refreshed = vec![crate::models::FeedItem {
        url: "https://github.com/org/repo/pull/42".to_string(),
        ..initial[0].clone()
    }];
    db.upsert_feed_tasks(
        epic.id,
        &refreshed,
        &["/repo".to_string()],
        &main_branches(1),
    )
    .await
    .unwrap();

    let task = db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(
        task.url.as_ref().map(|u| u.url.as_str()),
        Some("https://github.com/org/repo/pull/42"),
        "null url is backfilled from item.url on conflict"
    );
    assert_eq!(
        task.url.as_ref().map(|u| u.url_type),
        Some(crate::models::UrlType::Pr),
        "backfilled url_type is inferred"
    );
}

#[tokio::test]
async fn upsert_feed_tasks_preserves_pr_url_on_conflict() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let initial = vec![crate::models::FeedItem {
        external_id: "dep:org/repo#42".to_string(),
        title: "#42 Bump foo".to_string(),
        description: "".to_string(),
        url: "https://github.com/org/repo/pull/42".to_string(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::PrReview,
        labels: vec![],
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    }];
    db.upsert_feed_tasks(epic.id, &initial, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();
    let task_id = db.list_tasks_for_epic(epic.id).await.unwrap()[0].id;
    let manual = crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/999",
        crate::models::UrlType::Pr,
    );
    db.patch_task(task_id, &TaskPatch::new().url(Some(&manual)))
        .await
        .unwrap();

    // Re-run upsert; url on the existing task must not be overwritten.
    db.upsert_feed_tasks(epic.id, &initial, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    let task = db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(
        task.url.as_ref().map(|u| u.url.as_str()),
        Some("https://github.com/org/repo/pull/999")
    );
}

#[tokio::test]
async fn feed_upsert_infers_url_type_and_backfills_atomically() {
    use crate::models::{TaskUrl, UrlType};
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();

    let feed_item = |external_id: &str, url: &str| crate::models::FeedItem {
        external_id: external_id.to_string(),
        title: "t".to_string(),
        description: "".to_string(),
        url: url.to_string(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::Dependabot,
        labels: vec![],
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    };
    // First emit: a PR URL is inferred as pr.
    let items = vec![feed_item("ext-1", "https://github.com/o/r/pull/5")];
    db.upsert_feed_tasks(epic.id, &items, &["/r".into()], &main_branches(1))
        .await
        .unwrap();
    let t = db.list_tasks_for_epic(epic.id).await.unwrap().remove(0);
    assert_eq!(
        t.url,
        Some(TaskUrl::new("https://github.com/o/r/pull/5", UrlType::Pr))
    );

    // Conflict re-emit with a DIFFERENT url must NOT clobber the existing pair.
    let items = vec![feed_item("ext-1", "https://github.com/o/r/pull/999")];
    db.upsert_feed_tasks(epic.id, &items, &["/r".into()], &main_branches(1))
        .await
        .unwrap();
    let t = db.list_tasks_for_epic(epic.id).await.unwrap().remove(0);
    assert_eq!(
        t.url,
        Some(TaskUrl::new("https://github.com/o/r/pull/5", UrlType::Pr)),
        "existing url/url_type must be preserved on conflict"
    );
}

#[tokio::test]
async fn upsert_feed_tasks_explicit_url_type_wins_over_inference() {
    use crate::models::{TaskUrl, UrlType};
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();

    // A Dependabot alert URL has no /pull/ or /issues/ segment, so inference
    // would classify it as Other. The declared security_alert must win.
    let alert_url = "https://github.com/org/repo/security/dependabot/7";
    let items = vec![
        crate::models::FeedItem {
            url: alert_url.to_string(),
            url_type: Some(UrlType::SecurityAlert),
            ..make_feed_item("ext-declared", "declared")
        },
        crate::models::FeedItem {
            url: alert_url.to_string(),
            url_type: None,
            ..make_feed_item("ext-inferred", "inferred")
        },
    ];
    db.upsert_feed_tasks(
        epic.id,
        &items,
        &["/repo".to_string(), "/repo".to_string()],
        &main_branches(2),
    )
    .await
    .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    let by_ext = |ext: &str| {
        tasks
            .iter()
            .find(|t| t.external_id.as_deref() == Some(ext))
            .unwrap()
    };
    assert_eq!(
        by_ext("ext-declared").url,
        Some(TaskUrl::new(alert_url, UrlType::SecurityAlert)),
        "explicit url_type is stored verbatim"
    );
    assert_eq!(
        by_ext("ext-inferred").url,
        Some(TaskUrl::new(alert_url, UrlType::Other)),
        "absent url_type falls back to inference"
    );
}

#[tokio::test]
async fn upsert_feed_tasks_backfill_uses_declared_url_type() {
    use crate::models::{TaskUrl, UrlType};
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();

    // First emission: no URL — task created with url = NULL.
    let initial = vec![make_feed_item("ext-1", "alert")];
    db.upsert_feed_tasks(epic.id, &initial, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();
    let task_id = db.list_tasks_for_epic(epic.id).await.unwrap()[0].id;
    assert!(
        db.get_task(task_id).await.unwrap().unwrap().url.is_none(),
        "precondition: url is null after first upsert"
    );

    // Refresh with a URL and a declared type that inference cannot reach.
    let alert_url = "https://github.com/org/repo/security/dependabot/7";
    let refreshed = vec![crate::models::FeedItem {
        url: alert_url.to_string(),
        url_type: Some(UrlType::SecurityAlert),
        ..initial[0].clone()
    }];
    db.upsert_feed_tasks(
        epic.id,
        &refreshed,
        &["/repo".to_string()],
        &main_branches(1),
    )
    .await
    .unwrap();

    let task = db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(
        task.url,
        Some(TaskUrl::new(alert_url, UrlType::SecurityAlert)),
        "backfilled url_type uses the declared type, not inference"
    );
}

#[tokio::test]
async fn upsert_feed_tasks_can_purge_task_with_associated_learning() {
    use crate::models::{LearningKind, LearningScope};

    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();

    // First feed run: creates a task.
    let initial = vec![make_feed_item("ext-1", "first")];
    db.upsert_feed_tasks(epic.id, &initial, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();
    let task_id = db.list_tasks_for_epic(epic.id).await.unwrap()[0].id;

    // The dispatched agent records a learning referencing the task as its source.
    db.create_learning(CreateLearningRow {
        kind: LearningKind::Pitfall,
        summary: "watch out",
        detail: None,
        scope: LearningScope::User,
        scope_ref: None,
        tags: &[],
        source_task_id: Some(task_id),
        embedding: None,
    })
    .await
    .unwrap();

    // Second feed run with a different external_id — the previous task should
    // be purged. Without ON DELETE SET NULL on learnings.source_task_id, this
    // fails with a FK violation.
    let next = vec![make_feed_item("ext-2", "second")];
    db.upsert_feed_tasks(epic.id, &next, &["/repo".to_string()], &main_branches(1))
        .await
        .expect("stale feed task with associated learning should be purgeable");

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].external_id.as_deref(), Some("ext-2"));
}

#[tokio::test]
async fn upsert_feed_tasks_can_purge_stale_task() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();

    let initial = vec![make_feed_item("ext-1", "first")];
    db.upsert_feed_tasks(epic.id, &initial, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    let next = vec![make_feed_item("ext-2", "second")];
    db.upsert_feed_tasks(epic.id, &next, &["/repo".to_string()], &main_branches(1))
        .await
        .expect("stale feed task should be purgeable");
}
