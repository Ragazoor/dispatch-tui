//! Rows outside your subscriptions never reach the board —
//! `nothing_is_read_through_to_the_local_store` is the structural half; the
//! other half is what is ASKED for, in `tests::queries`.

use std::sync::Arc;

use crate::store::{CreateTaskRequest, Database, TaskCrud};
use crate::sync::{BoardReads, SharedRows, SubscriptionBoardReads};

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
    let db = Database::open_in_memory().await.unwrap();
    let any_task = db
        .create_task(CreateTaskRequest {
            title: "elsewhere",
            description: "",
            repo_path: "/repo",
            plan: None,
            status: crate::models::TaskStatus::Backlog,
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
    assert!(
        !db.board_reads()
            .unwrap()
            .list_tasks()
            .await
            .unwrap()
            .is_empty(),
        "the handle's own store must be populated, or this proves nothing"
    );

    let nothing_delivered = SubscriptionBoardReads::new(Arc::new(SharedRows::new()));

    assert!(nothing_delivered.list_tasks().await.unwrap().is_empty());
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
