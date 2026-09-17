//! The trap this whole subsystem exists to avoid, and the burn that avoids it.
//!
//! See `spacetime-seed.allium`: `IdSequence.SequenceClearsEveryRow` and the
//! `BurnIdSequences` rule.
//!
//! # What these tests prove, and what they do not
//!
//! They run against [`MemoryStore`], which models one rule of the real shared
//! store: **a generated id is handed out only when the caller supplies none, so
//! inserting an explicit id does not advance the counter.** That model is code
//! we wrote, so a test passing here is not evidence about SpacetimeDB's own
//! behaviour — it would pass just as happily if the real store worked some
//! other way.
//!
//! What they *are* evidence for is our side of the contract: that `restore`
//! burns, that it burns every generating table rather than the first one, that
//! it burns past rather than up to the ceiling, and that a later restore does
//! not undo it. Those are the four ways this goes wrong in code, and all four
//! are regressions a fake catches.
//!
//! The rule the fake encodes was verified against a real `spacetime` instance —
//! see the "SpacetimeDB" section of `docs/reference.md` for the run and how to
//! repeat it. If that rule ever changes upstream, [`explicit_ids_do_not_advance_the_sequence`]
//! is the single place to change it, and every test below inherits the fix.

use super::snapshot_of_a_populated_board;
use crate::spacetime::{restore, MemoryStore, SharedStore, SharedTable};

/// The trap itself, stated as a test so the model cannot drift away from the
/// behaviour it is modelling without something going red.
///
/// This is the premise every other test in this file rests on. If it ever fails,
/// nothing else here means what it claims to.
#[tokio::test]
async fn explicit_ids_do_not_advance_the_sequence() {
    let store = MemoryStore::new();

    store
        .upsert_rows(SharedTable::Tasks, &[super::row_with_id(5000)])
        .await
        .unwrap();

    assert_eq!(
        store.next_generated_id(SharedTable::Tasks),
        1,
        "inserting an explicit id must leave the counter untouched — if this \
         fails, the model no longer models the trap and every other test in \
         this file is vacuous"
    );
}

/// **The load-bearing test of Phase 0.**
///
/// Not "the burn loop ran" — that would prove the code calls itself. This
/// creates a task the way the board creates one, immediately after a restore,
/// and asserts it does not land on top of a restored task.
#[tokio::test]
async fn a_task_created_after_a_restore_cannot_collide() {
    let snapshot = snapshot_of_a_populated_board().await;
    let highest_restored = snapshot.highest_id(SharedTable::Tasks);
    assert_eq!(
        highest_restored, 4096,
        "fixture sanity: the board must carry a task well above the others, or \
         a naive counter would pass this test by accident"
    );

    let store = super::store_for(&snapshot);
    restore(&store, &snapshot).await.unwrap();

    let fresh = store
        .insert_generating_id(SharedTable::Tasks)
        .await
        .unwrap();

    assert!(
        fresh > highest_restored,
        "a task created straight after a restore got id {fresh}, which collides \
         with restored task {highest_restored}"
    );
}

/// The counter must pass the ceiling, not reach it. A counter sitting exactly
/// on the highest restored id hands that id out next, which is the same
/// collision one iteration later.
#[tokio::test]
async fn the_burn_passes_the_ceiling_rather_than_reaching_it() {
    let snapshot = snapshot_of_a_populated_board().await;
    let store = super::store_for(&snapshot);
    restore(&store, &snapshot).await.unwrap();

    for table in SharedTable::ALL
        .iter()
        .copied()
        .filter(|t| t.generates_ids())
    {
        assert!(
            store.next_generated_id(table) > snapshot.highest_id(table),
            "{}: counter sits at {}, ceiling is {} — strictly greater is the \
             whole point",
            table.name(),
            store.next_generated_id(table),
            snapshot.highest_id(table),
        );
    }
}

/// Every table with a generated id gets burned, not just `tasks`. Each table
/// has its own counter; burning one does nothing for another, and a single
/// forgotten table is a collision with no test watching for it.
#[tokio::test]
async fn every_generating_table_is_burned_not_just_tasks() {
    let snapshot = snapshot_of_a_populated_board().await;
    let store = super::store_for(&snapshot);
    restore(&store, &snapshot).await.unwrap();

    for table in SharedTable::ALL
        .iter()
        .copied()
        .filter(|t| t.generates_ids())
    {
        let fresh = store.insert_generating_id(table).await.unwrap();
        assert!(
            fresh > snapshot.highest_id(table),
            "{}: a row created after the restore got id {fresh}, at or below \
             the restored ceiling {}",
            table.name(),
            snapshot.highest_id(table),
        );
    }
}

/// A table whose extract is empty has ceiling 0 and terminates immediately,
/// rather than spinning on a ceiling it has already passed.
///
/// It is not free: the real store gives no way to read the counter, so learning
/// where it stands costs one generated id, and the model does the same. What
/// this pins is termination and the property, not a particular counter value.
#[tokio::test]
async fn an_empty_table_terminates_the_burn_at_once() {
    let snapshot = crate::spacetime::Snapshot::empty(super::TEST_SCHEMA_VERSION);
    let store = MemoryStore::new();

    restore(&store, &snapshot).await.unwrap();

    for table in SharedTable::ALL
        .iter()
        .copied()
        .filter(|t| t.generates_ids())
    {
        assert_eq!(
            store.next_generated_id(table),
            2,
            "{}: an empty table should cost exactly the one id it takes to \
             read the counter",
            table.name()
        );
        assert!(
            store.rows(table).await.unwrap().is_empty(),
            "throwaway rows survived"
        );
    }
}

/// Restoring on top of a store that has already been restored into — the
/// recover-from-a-failed-recovery case — must leave the counter clear, not
/// reset it to where the first restore found it.
#[tokio::test]
async fn a_second_restore_leaves_the_counter_clear() {
    let snapshot = snapshot_of_a_populated_board().await;
    let store = super::store_for(&snapshot);

    restore(&store, &snapshot).await.unwrap();
    let after_first = store.next_generated_id(SharedTable::Tasks);
    restore(&store, &snapshot).await.unwrap();

    assert!(
        store.next_generated_id(SharedTable::Tasks) >= after_first,
        "the second restore walked the counter backwards"
    );
    let fresh = store
        .insert_generating_id(SharedTable::Tasks)
        .await
        .unwrap();
    assert!(fresh > snapshot.highest_id(SharedTable::Tasks));
}

/// Work done between two restores must not be undone by the second one's burn.
/// A burn that *sets* the counter rather than advancing it would pass every
/// test above and fail this one.
#[tokio::test]
async fn a_restore_does_not_walk_back_over_ids_handed_out_since() {
    let snapshot = snapshot_of_a_populated_board().await;
    let store = super::store_for(&snapshot);
    restore(&store, &snapshot).await.unwrap();

    let created_between = store
        .insert_generating_id(SharedTable::Tasks)
        .await
        .unwrap();
    restore(&store, &snapshot).await.unwrap();
    let created_after = store
        .insert_generating_id(SharedTable::Tasks)
        .await
        .unwrap();

    assert!(
        created_after > created_between,
        "the second restore re-issued id {created_after}, which is at or below \
         {created_between} — a task created between the two restores"
    );
}

/// **The burn runs before the rows are written, and that order is not a
/// preference.**
///
/// Burning means asking the store to generate ids 1, 2, 3 and so on — exactly
/// the ids a restore is about to write. Against a table that already holds
/// them, every one of those inserts lands on an existing primary key, the store
/// rejects it, and a reducer whose insert is rejected aborts. There is no way
/// to advance the counter past a row without generating that row's id, so
/// burning a loaded table is not slow, it is impossible.
///
/// The migration design says to burn "after loading". It is wrong, and this is
/// the test that says so.
#[tokio::test]
async fn burning_a_table_that_already_holds_low_ids_is_impossible() {
    let snapshot = snapshot_of_a_populated_board().await;
    let store = super::store_for(&snapshot);

    // Load first, the way the design describes...
    store
        .upsert_rows(
            SharedTable::Tasks,
            &snapshot.extract(SharedTable::Tasks).unwrap().rows,
        )
        .await
        .unwrap();

    // ...and the burn cannot get past the rows it just wrote.
    let failure = store
        .advance_id_sequence_past(SharedTable::Tasks, snapshot.highest_id(SharedTable::Tasks))
        .await
        .unwrap_err();

    assert!(
        failure.to_string().contains("already taken"),
        "expected the burn to collide with a restored row, got: {failure}"
    );
}

/// The same sequence in the order `restore` actually uses completes, which is
/// the positive half of the statement above.
#[tokio::test]
async fn burning_before_loading_completes() {
    let snapshot = snapshot_of_a_populated_board().await;
    let store = super::store_for(&snapshot);

    store
        .advance_id_sequence_past(SharedTable::Tasks, snapshot.highest_id(SharedTable::Tasks))
        .await
        .unwrap();
    store
        .upsert_rows(
            SharedTable::Tasks,
            &snapshot.extract(SharedTable::Tasks).unwrap().rows,
        )
        .await
        .unwrap();

    assert_eq!(
        store.row_ids(SharedTable::Tasks).await.unwrap(),
        vec![3, 4, 11, 4096],
        "the burn's throwaway rows were not cleaned up"
    );
}
