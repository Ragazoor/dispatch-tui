//! The three tests this phase is defined by.
//!
//! 1. Every board read answers identically from the subscription and from
//!    SQLite — `the_two_backings_answer_identically` below, over the whole
//!    `BoardReads` surface, on a board with both arms of every sentinel.
//! 2. A teammate's change appears without polling — `a_row_that_arrives_wakes_
//!    the_board`.
//! 3. Rows outside your subscriptions never reach the board —
//!    `nothing_is_read_through_to_the_local_store`, which is the structural
//!    half; the other half is what is ASKED for, in `tests::queries`.

use std::sync::Arc;

use super::decode::{
    as_epic, as_host, as_repo_base_branch, as_repo_path, as_task, as_todo, populated_board, rows,
};
use crate::db::{Database, RepoConfigStore, TaskCrud, TaskPatch, TaskRead};
use crate::models::{EpicId, TaskId};
use crate::spacetime::{dump_from_sqlite, SharedTable, Snapshot};
use crate::sync::{BoardReads, LocalBoardReads, SharedRows, SubscriptionBoardReads};

/// Fill a [`SharedRows`] from a dump, exactly as a subscription delivering
/// every row of every subscribed table would.
///
/// This is the seam the phase turns on, so the fixture goes through the
/// production dump and the production sentinel table rather than hand-built
/// rows: what is being compared is two readings of the SAME board, not two
/// fixtures that happen to match.
fn deliver(snapshot: &Snapshot) -> Arc<SharedRows> {
    let shared = Arc::new(SharedRows::new());
    for row in rows(snapshot, SharedTable::Tasks) {
        shared.upsert_task(&as_task(&row));
    }
    for row in rows(snapshot, SharedTable::Epics) {
        shared.upsert_epic(&as_epic(&row));
    }
    for row in rows(snapshot, SharedTable::Todos) {
        shared.upsert_todo(&as_todo(&row));
    }
    for row in rows(snapshot, SharedTable::RepoPaths) {
        shared.upsert_repo_path(&as_repo_path(&row));
    }
    for row in rows(snapshot, SharedTable::RepoBaseBranches) {
        shared.upsert_repo_base_branch(&as_repo_base_branch(&row));
    }
    for row in rows(snapshot, SharedTable::Hosts) {
        shared.upsert_host(&as_host(&row));
    }
    shared
}

fn local(db: &Arc<Database>) -> LocalBoardReads {
    LocalBoardReads::new(db.clone(), db.clone())
}

/// A board with repos, branches and the task/epic/todo fixture behind it.
async fn board() -> Arc<Database> {
    let db = Arc::new(populated_board().await);
    // `list_repo_paths` orders by `last_used DESC`, so several paths are worth
    // having: one row can be in any order and still look sorted.
    for path in ["/repo", "/other", "/third"] {
        db.save_repo_path(path).await.unwrap();
        db.record_base_branch(path, "main").await.unwrap();
        db.record_base_branch(path, "develop").await.unwrap();
    }
    db.set_verify_command("/repo", Some("cargo test"))
        .await
        .unwrap();
    db
}

// ---------------------------------------------------------------------------
// 1. Identical reads
// ---------------------------------------------------------------------------

/// **Test 1 of the phase plan.** Every board read, both ways, same answer.
///
/// Every board view is a pure function of these reads, so a view that renders
/// differently would need one of them to differ — which is what this asserts
/// away. The snapshot tests then pin the rendering itself, unchanged and
/// unaware that the rows came from anywhere new.
#[tokio::test]
async fn the_two_backings_answer_identically() {
    let db = board().await;
    let snapshot = dump_from_sqlite(&db).await.unwrap();

    let from_sqlite = local(&db);
    let from_store = SubscriptionBoardReads::new(deliver(&snapshot));

    let tasks = from_sqlite.list_tasks().await.unwrap();
    assert!(tasks.len() >= 2, "the fixture must have tasks to compare");
    assert_eq!(tasks, from_store.list_tasks().await.unwrap());

    let epics = from_sqlite.list_epics().await.unwrap();
    assert!(epics.len() >= 2);
    assert_eq!(epics, from_store.list_epics().await.unwrap());

    assert_eq!(
        from_sqlite.list_todos().await.unwrap(),
        from_store.list_todos().await.unwrap()
    );
    assert_eq!(
        from_sqlite.list_repo_paths().await.unwrap(),
        from_store.list_repo_paths().await.unwrap()
    );
    assert_eq!(
        from_sqlite.list_all_base_branches().await.unwrap(),
        from_store.list_all_base_branches().await.unwrap()
    );

    for task in &tasks {
        assert_eq!(
            from_sqlite.get_task(task.id).await.unwrap(),
            from_store.get_task(task.id).await.unwrap()
        );
    }
    for epic in &epics {
        assert_eq!(
            from_sqlite.get_epic(epic.id).await.unwrap(),
            from_store.get_epic(epic.id).await.unwrap()
        );
        assert_eq!(
            from_sqlite.list_tasks_for_epic(epic.id).await.unwrap(),
            from_store.list_tasks_for_epic(epic.id).await.unwrap(),
            "epic {:?}",
            epic.id
        );
    }
}

/// An id neither store has answers `None` from both, rather than one answering
/// `None` and the other erroring.
#[tokio::test]
async fn a_missing_row_is_absent_the_same_way_from_both() {
    let db = board().await;
    let snapshot = dump_from_sqlite(&db).await.unwrap();
    let from_store = SubscriptionBoardReads::new(deliver(&snapshot));

    assert_eq!(local(&db).get_task(TaskId(9_999)).await.unwrap(), None);
    assert_eq!(from_store.get_task(TaskId(9_999)).await.unwrap(), None);
    assert_eq!(local(&db).get_epic(EpicId(9_999)).await.unwrap(), None);
    assert_eq!(from_store.get_epic(EpicId(9_999)).await.unwrap(), None);
    assert!(from_store
        .list_tasks_for_epic(EpicId(9_999))
        .await
        .unwrap()
        .is_empty());
}

/// Order is part of the answer, not an accident of it.
///
/// `list_tasks` is `COALESCE(sort_order, id) ASC, id ASC`, and a board that
/// returned the same tasks in a different order would draw the same cards in
/// the wrong columns' order — a difference the set comparison above would miss.
#[tokio::test]
async fn ordering_survives_the_crossing() {
    let db = board().await;
    // Sort orders that disagree with id order, so the two keys cannot be
    // confused for one another.
    let ids: Vec<TaskId> = db.list_all().await.unwrap().iter().map(|t| t.id).collect();
    for (position, id) in ids.iter().rev().enumerate() {
        db.patch_task(*id, &TaskPatch::new().sort_order(Some(position as i64)))
            .await
            .unwrap();
    }

    let snapshot = dump_from_sqlite(&db).await.unwrap();
    let expected = local(&db).list_tasks().await.unwrap();
    let actual = SubscriptionBoardReads::new(deliver(&snapshot))
        .list_tasks()
        .await
        .unwrap();

    assert_ne!(
        expected.iter().map(|t| t.id).collect::<Vec<_>>(),
        ids,
        "the fixture must make sort_order disagree with id order"
    );
    assert_eq!(
        expected.iter().map(|t| t.id).collect::<Vec<_>>(),
        actual.iter().map(|t| t.id).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// 2. Arriving without being asked
// ---------------------------------------------------------------------------

/// **Test 2 of the phase plan.** A teammate's change appears without polling.
///
/// The board waits on the change signal rather than re-reading on a timer, so
/// this asserts the wake-up — not that a read eventually returns the new row,
/// which a poll would also satisfy. The wait has no timeout and no sleep: if
/// the signal does not fire the test hangs and the harness kills it, which is a
/// louder failure than a slept-through assertion.
#[tokio::test]
async fn a_row_that_arrives_wakes_the_board() {
    let db = board().await;
    let snapshot = dump_from_sqlite(&db).await.unwrap();
    let shared = deliver(&snapshot);
    let reads = SubscriptionBoardReads::new(shared.clone());

    let before = reads.list_tasks().await.unwrap();
    let mut wake = shared.changed();
    wake.mark_unchanged();

    // A teammate edits a task on their machine. Nothing here asked for it.
    let mut theirs = as_task(&rows(&snapshot, SharedTable::Tasks)[0]);
    theirs.title = "Retitled by a teammate".into();
    let edited = TaskId(theirs.id);
    shared.upsert_task(&theirs);

    wake.changed().await.expect("the board must be woken");

    let after = reads.list_tasks().await.unwrap();
    assert_eq!(after.len(), before.len(), "an edit is not an insert");
    assert_eq!(
        after.iter().find(|t| t.id == edited).unwrap().title,
        "Retitled by a teammate"
    );
}

/// A deletion wakes the board too, and takes the card with it.
#[tokio::test]
async fn a_row_that_leaves_wakes_the_board() {
    let db = board().await;
    let snapshot = dump_from_sqlite(&db).await.unwrap();
    let shared = deliver(&snapshot);
    let reads = SubscriptionBoardReads::new(shared.clone());

    let doomed = reads.list_tasks().await.unwrap()[0].id;
    let mut wake = shared.changed();
    wake.mark_unchanged();

    shared.remove_task(doomed);

    wake.changed().await.expect("the board must be woken");
    assert!(reads
        .list_tasks()
        .await
        .unwrap()
        .iter()
        .all(|t| t.id != doomed));
}

/// The revision moves with the rows, so the tick-driven refresh can still skip
/// a read that would change nothing.
#[tokio::test]
async fn the_revision_advances_only_when_rows_move() {
    let db = board().await;
    let snapshot = dump_from_sqlite(&db).await.unwrap();
    let shared = deliver(&snapshot);
    let reads = SubscriptionBoardReads::new(shared.clone());

    let quiet = reads.revision().await;
    assert_eq!(reads.revision().await, quiet, "reading changes nothing");

    shared.upsert_task(&as_task(&rows(&snapshot, SharedTable::Tasks)[0]));
    assert_ne!(reads.revision().await, quiet);
}

// ---------------------------------------------------------------------------
// 3. Nothing arrives that was not subscribed to
// ---------------------------------------------------------------------------

/// **Test 3 of the phase plan**, structurally.
///
/// A board reading from the subscription has NO path to the local store's
/// shared tables. Asserted with a fully populated SQLite database sitting
/// beside an empty subscription: every read answers empty, although every row
/// is right there on disk.
///
/// This is the property that makes the containment claim in `tests::queries`
/// worth anything. Asking for only your own rows means nothing if the reader
/// can also reach past the subscription — and a read-through fallback is
/// exactly the "local read cache" Phase 5 records as not existing.
#[tokio::test]
async fn nothing_is_read_through_to_the_local_store() {
    let db = board().await;
    assert!(
        !local(&db).list_tasks().await.unwrap().is_empty(),
        "the local store must be populated, or this proves nothing"
    );

    let nothing_delivered = SubscriptionBoardReads::new(Arc::new(SharedRows::new()));

    assert!(nothing_delivered.list_tasks().await.unwrap().is_empty());
    assert!(nothing_delivered.list_epics().await.unwrap().is_empty());
    assert!(nothing_delivered.list_todos().await.unwrap().is_empty());
    assert!(nothing_delivered
        .list_repo_paths()
        .await
        .unwrap()
        .is_empty());
    assert!(nothing_delivered
        .list_all_base_branches()
        .await
        .unwrap()
        .is_empty());
    let any_task = local(&db).list_tasks().await.unwrap()[0].id;
    assert_eq!(nothing_delivered.get_task(any_task).await.unwrap(), None);
}

/// Only what was delivered is held. A subscription that brings one epic's tasks
/// brings that epic's tasks and nothing else, however many the store has.
#[tokio::test]
async fn only_the_delivered_rows_are_held() {
    let db = board().await;
    let snapshot = dump_from_sqlite(&db).await.unwrap();
    let all = rows(&snapshot, SharedTable::Tasks);
    assert!(all.len() >= 2);

    let shared = Arc::new(SharedRows::new());
    shared.upsert_task(&as_task(&all[0]));
    let reads = SubscriptionBoardReads::new(shared);

    let held = reads.list_tasks().await.unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].id.0, as_task(&all[0]).id);
}

/// A connection going down takes its rows with it.
///
/// They belonged to that subscription and the next one re-delivers them. Left
/// in place they would be a board's stale contents on screen with nothing
/// behind them live — which is the read-through this design does not have,
/// arrived at by accident instead of by design.
#[tokio::test]
async fn a_dropped_connection_leaves_no_rows_behind() {
    let db = board().await;
    let snapshot = dump_from_sqlite(&db).await.unwrap();
    let shared = deliver(&snapshot);
    let reads = SubscriptionBoardReads::new(shared.clone());
    assert!(!reads.list_tasks().await.unwrap().is_empty());

    shared.clear();

    assert!(reads.list_tasks().await.unwrap().is_empty());
    assert!(reads.list_epics().await.unwrap().is_empty());
    assert!(reads.list_repo_paths().await.unwrap().is_empty());
}

/// An undecodable row is dropped, and the rows around it are not.
///
/// The alternative — failing the whole delivery — would take a board off screen
/// because one task carried a status this binary does not know.
#[tokio::test]
async fn one_bad_row_does_not_take_the_others_with_it() {
    let db = board().await;
    let snapshot = dump_from_sqlite(&db).await.unwrap();
    let all = rows(&snapshot, SharedTable::Tasks);

    let shared = Arc::new(SharedRows::new());
    let mut broken = as_task(&all[0]);
    broken.status = "teleported".into();
    shared.upsert_task(&broken);
    shared.upsert_task(&as_task(&all[1]));

    let held = SubscriptionBoardReads::new(shared)
        .list_tasks()
        .await
        .unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].id.0, as_task(&all[1]).id);
}
