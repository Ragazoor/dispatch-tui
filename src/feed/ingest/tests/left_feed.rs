//! `feeds.allium: KeepWorkedOnFeedTask` and the coming-back clause of
//! `UpsertFeedTasks`' match arm: a trusted cycle deletes only an untouched
//! (backlog) feed task whose item left; a worked-on one moves to done and
//! returns to backlog when its item comes back.

use super::*;
use crate::models::{SessionRequestKind, Task};
use crate::store::{SessionRequestRead, TaskRead};

const PR_1: &str = "https://github.com/org/repo/pull/1";

/// A flat feed epic holding one task for `pr-1`, as the first poll leaves it.
async fn flat_feed_with_pr_1(db: &Store) -> (EpicId, Task) {
    let epic = db.create_epic("Flat Feed", "", None).await.unwrap();
    let items = vec![make_item("pr-1", PR_1)];
    run_feed_sync(db, epic.id, false, entries(&items, &[""], &["main"]))
        .await
        .unwrap();
    let task = db.list_tasks_for_epic(epic.id).await.remove(0);
    (epic.id, task)
}

async fn poll_with_pr_1(db: &Store, epic: EpicId) -> FeedSyncOutcome {
    let items = vec![make_item("pr-1", PR_1)];
    run_feed_sync(db, epic, false, entries(&items, &[""], &["main"]))
        .await
        .unwrap()
}

async fn poll_empty(db: &Store, epic: EpicId) -> FeedSyncOutcome {
    run_feed_sync(db, epic, false, vec![]).await.unwrap()
}

async fn reread(db: &Store, task: TaskId) -> Task {
    db.get_task(task)
        .await
        .expect("the task is still on the board")
}

/// Give `task` a running agent with a worktree and a window, as dispatching a
/// review agent does.
async fn dispatch(db: &Store, task: TaskId) {
    db.give_agent(task);
    db.patch_task(
        task,
        &TaskPatch::new()
            .status(TaskStatus::Running)
            .worktree(Some("/repo/a/.worktrees/7-pr-1"))
            .tmux_window(Some(&test_tmux_window("dispatch:pr-1"))),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn untouched_backlog_task_is_still_deleted_when_its_item_leaves() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let (epic, task) = flat_feed_with_pr_1(&db).await;

    let outcome = poll_empty(&db, epic).await;

    assert_eq!(outcome.removed.len(), 1);
    assert!(db.get_task(task.id).await.is_none());
}

#[tokio::test]
async fn running_task_moves_to_done_when_its_item_leaves() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let (epic, task) = flat_feed_with_pr_1(&db).await;
    dispatch(&db, task.id).await;

    let outcome = poll_empty(&db, epic).await;

    assert!(
        outcome.removed.is_empty(),
        "a kept task is not reported for teardown, got {:?}",
        outcome.removed
    );
    let kept = reread(&db, task.id).await;
    assert_eq!(kept.status, TaskStatus::Done);
    assert!(kept.completed_at.is_some(), "stamped like any completion");
    assert_eq!(
        kept.worktree.as_deref(),
        Some("/repo/a/.worktrees/7-pr-1"),
        "the worktree stays"
    );
    assert!(kept.tmux_window.is_none(), "the window is released");
}

#[tokio::test]
async fn kept_task_with_a_window_gets_a_close_row_for_it() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let (epic, task) = flat_feed_with_pr_1(&db).await;
    dispatch(&db, task.id).await;

    poll_empty(&db, epic).await;

    let closes: Vec<_> = db
        .session_requests()
        .await
        .into_iter()
        .filter(|r| r.task_id == task.id && r.kind == SessionRequestKind::Close)
        .collect();
    assert_eq!(closes.len(), 1, "one close row, got {closes:?}");
    assert_eq!(
        closes[0].tmux_window.as_ref().map(|w| w.as_str()),
        Some("dispatch:pr-1"),
        "the close row names the window the host must kill"
    );
}

#[tokio::test]
async fn review_task_moves_to_done_when_its_item_leaves() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let (epic, task) = flat_feed_with_pr_1(&db).await;
    db.patch_task(task.id, &TaskPatch::new().status(TaskStatus::Review))
        .await
        .unwrap();

    poll_empty(&db, epic).await;

    assert_eq!(reread(&db, task.id).await.status, TaskStatus::Done);
}

#[tokio::test]
async fn a_later_absent_poll_rewrites_nothing() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let (epic, task) = flat_feed_with_pr_1(&db).await;
    dispatch(&db, task.id).await;
    poll_empty(&db, epic).await;
    let first = reread(&db, task.id).await;
    let requests_before = db.session_requests().await.len();

    poll_empty(&db, epic).await;

    let second = reread(&db, task.id).await;
    assert_eq!(second, first, "idempotent: no second write");
    assert_eq!(db.session_requests().await.len(), requests_before);
}

#[tokio::test]
async fn kept_task_returns_to_backlog_when_its_item_comes_back() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let (epic, task) = flat_feed_with_pr_1(&db).await;
    dispatch(&db, task.id).await;
    poll_empty(&db, epic).await;

    poll_with_pr_1(&db, epic).await;

    let back = reread(&db, task.id).await;
    assert_eq!(back.status, TaskStatus::Backlog, "new review work");
    assert_eq!(
        back.worktree.as_deref(),
        Some("/repo/a/.worktrees/7-pr-1"),
        "the card returns with its worktree"
    );

    // Present on the next poll too: nothing moves it again.
    poll_with_pr_1(&db, epic).await;
    assert_eq!(reread(&db, task.id).await.status, TaskStatus::Backlog);
}

#[tokio::test]
async fn task_marked_done_while_its_item_is_present_stays_done() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let (epic, task) = flat_feed_with_pr_1(&db).await;
    db.patch_task(task.id, &TaskPatch::new().status(TaskStatus::Done))
        .await
        .unwrap();

    poll_with_pr_1(&db, epic).await;
    poll_with_pr_1(&db, epic).await;

    assert_eq!(reread(&db, task.id).await.status, TaskStatus::Done);
}

#[tokio::test]
async fn task_marked_done_by_hand_is_kept_and_reopens_when_its_item_returns() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let (epic, task) = flat_feed_with_pr_1(&db).await;
    db.patch_task(task.id, &TaskPatch::new().status(TaskStatus::Done))
        .await
        .unwrap();
    let done = reread(&db, task.id).await;

    poll_empty(&db, epic).await;
    let kept = reread(&db, task.id).await;
    assert_eq!(kept.status, TaskStatus::Done);
    assert_eq!(kept.completed_at, done.completed_at, "not re-stamped");

    poll_with_pr_1(&db, epic).await;
    assert_eq!(reread(&db, task.id).await.status, TaskStatus::Backlog);
}

#[tokio::test]
async fn task_moved_out_of_done_by_hand_stays_put_when_its_item_returns() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let (epic, task) = flat_feed_with_pr_1(&db).await;
    db.patch_task(task.id, &TaskPatch::new().status(TaskStatus::Review))
        .await
        .unwrap();
    poll_empty(&db, epic).await;
    db.patch_task(task.id, &TaskPatch::new().status(TaskStatus::Review))
        .await
        .unwrap();

    poll_with_pr_1(&db, epic).await;

    assert_eq!(reread(&db, task.id).await.status, TaskStatus::Review);
}

#[tokio::test]
async fn additive_cycle_keeps_a_worked_on_task_where_it_is() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let (epic, task) = flat_feed_with_pr_1(&db).await;
    dispatch(&db, task.id).await;

    run_feed_sync_with_mode(&*db, epic, false, vec![], SyncMode::Additive)
        .await
        .unwrap();

    assert_eq!(reread(&db, task.id).await.status, TaskStatus::Running);
}

/// The role-routed path judges "left" against the filtered set: an approved
/// PR (ExcludeFromReviews) has left, so a worked-on review moves to done.
#[tokio::test]
async fn approved_review_moves_to_done_on_the_role_routed_path() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();
    let requested = vec![make_signal_item("pr-1", PR_1, vec![Signal::TeamRequest])];
    run_role_routed_feed_sync(&*db, parent.id, entries(&requested, &[""], &["main"]))
        .await
        .unwrap();
    let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
    let task = db.list_tasks_for_epic(team).await.remove(0);
    dispatch(&db, task.id).await;

    let approved = vec![make_signal_item("pr-1", PR_1, vec![Signal::Approved])];
    let outcome = run_role_routed_feed_sync(&*db, parent.id, entries(&approved, &[""], &["main"]))
        .await
        .unwrap();

    assert!(outcome.removed.is_empty(), "got {:?}", outcome.removed);
    assert_eq!(reread(&db, task.id).await.status, TaskStatus::Done);

    // Review re-requested: the same card returns to backlog.
    run_role_routed_feed_sync(&*db, parent.id, entries(&requested, &[""], &["main"]))
        .await
        .unwrap();
    assert_eq!(reread(&db, task.id).await.status, TaskStatus::Backlog);
}

/// A grouped feed keeps a worked-on task in its repo sub-epic.
#[tokio::test]
async fn grouped_feed_keeps_a_worked_on_task() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Grouped", "", None).await.unwrap();
    let items = vec![make_item("pr-1", PR_1)];
    sync_grouped_feed(&*db, parent.id, entries(&items, &[""], &["main"])).await;
    let sub = db.list_sub_epics(parent.id).await.remove(0);
    let task = db.list_tasks_for_epic(sub.id).await.remove(0);
    dispatch(&db, task.id).await;

    sync_grouped_feed(&*db, parent.id, vec![]).await;

    assert_eq!(reread(&db, task.id).await.status, TaskStatus::Done);
}

/// `NewReviewRequestIsSignalled`: a kept review that comes back is new review
/// work, so it gets the new-task mark like an insert. A mere refresh does not.
#[tokio::test]
async fn reopened_review_is_signalled_like_a_new_request() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let parent = db.create_epic("Reviews", "", None).await.unwrap();
    let requested = vec![make_signal_item("pr-1", PR_1, vec![Signal::TeamRequest])];
    run_role_routed_feed_sync(&*db, parent.id, entries(&requested, &[""], &["main"]))
        .await
        .unwrap();
    let team = role_sub_epic(&db, parent.id, FeedRole::TeamReviews).await;
    let task = db.list_tasks_for_epic(team).await.remove(0);
    db.mark_task_seen(task.id).await.unwrap();
    dispatch(&db, task.id).await;
    run_role_routed_feed_sync(&*db, parent.id, vec![])
        .await
        .unwrap();
    assert_eq!(reread(&db, task.id).await.status, TaskStatus::Done);
    let unseen = || async { crate::store::BoardReads::unseen_task_ids(&*db).await };
    assert!(unseen().await.is_empty());

    run_role_routed_feed_sync(&*db, parent.id, entries(&requested, &[""], &["main"]))
        .await
        .unwrap();

    assert_eq!(unseen().await, vec![task.id], "the reopen is signalled");
    db.mark_task_seen(task.id).await.unwrap();
    run_role_routed_feed_sync(&*db, parent.id, entries(&requested, &[""], &["main"]))
        .await
        .unwrap();
    assert!(unseen().await.is_empty(), "a refresh after it is not");
}
