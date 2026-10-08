//! `OpenInMemoryAttachesStoreOnceComplete` (docs/specs/spacetime-memory-store.allium).
use super::*;

/// The board's card reads and the store's own reads are one path over one
/// set of rows: a write through the handle is visible through both, with no
/// second copy to keep in step (`sync.allium: BoardReadsFromTheSubscription`).
#[tokio::test]
async fn the_board_reads_and_the_store_answer_from_the_same_rows() {
    let db = Arc::new(Store::open_in_memory().unwrap());
    let board: Arc<dyn crate::store::BoardReads> = db.clone();
    let before = board.revision().await;

    let id = db
        .create_task(CreateTaskRequest::fixture("drawn", "/repo"))
        .await
        .unwrap();

    assert_eq!(board.get_task(id).await.unwrap().unwrap().title, "drawn");
    assert_eq!(board.list_all().await.unwrap().len(), 1);
    assert_ne!(
        board.revision().await,
        before,
        "a write must move the revision the redraw guard compares"
    );
}

/// Identity lives only in the host file: a handle whose data directory holds
/// none refuses, rather than keeping an identity somewhere nothing else reads.
#[tokio::test]
async fn a_handle_with_no_host_file_refuses_identity() {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::in_memory_with_host_file(dir.path());
    assert!(db.ensure_host_identity().await.is_err());
    assert!(db.user_identity().await.is_err());
    assert!(db.adopt_user_identity("user-a").await.is_err());
}

#[tokio::test]
async fn attached_handle_round_trips_a_task_through_the_store() {
    let db = Store::open_in_memory().unwrap();
    let id = db
        .create_task(CreateTaskRequest::fixture("via store", "/repo"))
        .await
        .unwrap();
    assert_eq!(db.get_task(id).await.unwrap().unwrap().title, "via store");
    let other = Store::open_in_memory().unwrap();
    assert!(other.list_all().await.unwrap().is_empty());
}
