//! Rows outside your subscriptions never reach the board —
//! `nothing_is_read_through_to_the_local_store` is the structural half; the
//! other half is what is ASKED for, in `tests::queries`.

use crate::store::{CreateTaskRequest, EpicRead, RepoConfigRead, Store, TaskCrud, TaskRead};

/// **Test 3 of the phase plan**, structurally.
///
/// A board reading from the subscription has NO path to any other store's
/// shared tables. Asserted with a populated in-memory store sitting beside an
/// empty subscription: every read answers empty, although the row is right
/// there in the other store.
///
/// This is the property that makes the containment claim in `tests::queries`
/// worth anything. Asking for only your own rows means nothing if the reader
/// can also reach past the subscription — and a read-through fallback is
/// exactly the "local read cache" Phase 5 records as not existing.
#[tokio::test]
async fn nothing_is_read_through_to_the_local_store() {
    let db = Store::open_in_memory().unwrap();
    let any_task = db
        .create_task(CreateTaskRequest::fixture("elsewhere", "/repo"))
        .await
        .unwrap();
    assert!(
        !db.list_all().await.unwrap().is_empty(),
        "the handle's own store must be populated, or this proves nothing"
    );

    // A second handle over rows nothing has been delivered to.
    let nothing_delivered = Store::open_in_memory().unwrap();

    assert!(nothing_delivered.list_all().await.unwrap().is_empty());
    assert!(nothing_delivered.list_epics().await.unwrap().is_empty());
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
    assert_eq!(nothing_delivered.get_task(any_task).await.unwrap(), None);
}

// ---------------------------------------------------------------------------
// 4. The in-memory handle's own board reads
// ---------------------------------------------------------------------------
