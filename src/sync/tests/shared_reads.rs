//! A board with a store reads back what it writes — Phase 12a (task #4916).
//!
//! Spec: `docs/specs/sync.allium`'s `BoardReadsFromTheSubscription`.
//!
//! Before this, a store-backed `Database` sent every shared WRITE through
//! `db::SharedWriter` but answered most shared READS from its own SQLite file
//! — which, once writes stopped landing there, held nothing new. Only the
//! card-drawing reads (`BoardReads`), learnings and usage had a store path.
//! Everything else — an MCP `get_task`, the watcher fan-out, a setting read
//! back after it was saved — answered from a table the board no longer wrote.
//!
//! The shape of the proof is `board_reads.rs`'s: fill SQLite, dump it through
//! the production dump, deliver the dump into [`SharedRows`] as a subscription
//! would, and ask the same question two ways. The store-backed handle is a
//! FRESH, EMPTY database with the reader attached, so a read that fell through
//! to SQLite answers from nothing and the comparison fails — there is no
//! agreeing-by-accident.

use std::sync::Arc;

use super::decode::{
    as_epic, as_repo_base_branch, as_repo_path, as_setting, as_subscription, as_task,
    as_task_watcher, populated_board, rows,
};
use crate::db::{
    Database, EpicCrud, EpicRead, HostStore, RepoConfigRead, RepoConfigStore, SettingsStore,
    SubscriptionStore, TaskCrud, TaskPatch, TaskRead,
};
use crate::models::{EpicId, TaskId, TaskStatus, TmuxWindow};
use crate::spacetime::{dump_from_sqlite, SharedTable, Snapshot};
use crate::sync::{SharedRows, SubscriptionBoardReads};

const SUBSCRIBER: &str = "c200e1f4bcae4a1b9f0e7d2a3c5b8e60";

/// Everything a subscription delivers for the tables these reads cover.
fn deliver(snapshot: &Snapshot) -> Arc<SharedRows> {
    let shared = Arc::new(SharedRows::new());
    for row in rows(snapshot, SharedTable::Tasks) {
        shared.upsert_task(&as_task(&row));
    }
    for row in rows(snapshot, SharedTable::Epics) {
        shared.upsert_epic(&as_epic(&row));
    }
    for row in rows(snapshot, SharedTable::RepoPaths) {
        shared.upsert_repo_path(&as_repo_path(&row));
    }
    for row in rows(snapshot, SharedTable::RepoBaseBranches) {
        shared.upsert_repo_base_branch(&as_repo_base_branch(&row));
    }
    for row in rows(snapshot, SharedTable::TaskWatchers) {
        shared.upsert_task_watcher(&as_task_watcher(&row));
    }
    for row in rows(snapshot, SharedTable::Subscriptions) {
        shared.upsert_subscription(&as_subscription(&row));
    }
    for row in rows(snapshot, SharedTable::Settings) {
        shared.upsert_setting(&as_setting(&row));
    }
    shared
}

/// A board with something in every table the routed reads cover, including a
/// live agent task (Running, with a window) and a sub-epic.
async fn board() -> Database {
    let db = populated_board().await;
    // Settings and presets are dumped under this install's host id, so a
    // board that never minted one would dump none of them.
    db.ensure_host_identity().await.unwrap();
    for path in ["/repo", "/other"] {
        db.save_repo_path(path).await.unwrap();
        db.record_base_branch(path, "main").await.unwrap();
    }
    db.set_verify_command("/repo", Some("cargo test"))
        .await
        .unwrap();

    let tasks = db.list_all().await.unwrap();
    let (a, b) = (tasks[0].id, tasks[1].id);
    db.create_task_watcher(a, b).await.unwrap();
    db.patch_task(
        b,
        &TaskPatch::new()
            .status(TaskStatus::Running)
            .tmux_window(Some(&TmuxWindow::for_task(b))),
    )
    .await
    .unwrap();

    let epics = db.list_epics().await.unwrap();
    let child = db
        .create_epic("A child epic", "", Some(epics[0].id))
        .await
        .unwrap();
    db.subscribe_to_epic(SUBSCRIBER, epics[0].id.0)
        .await
        .unwrap();
    db.subscribe_to_epic(SUBSCRIBER, child.id.0).await.unwrap();

    db.set_setting_string("some_key", "some value")
        .await
        .unwrap();
    db.set_setting_bool("a_flag", true).await.unwrap();
    db
}

/// A store-backed handle over nothing but what `snapshot` delivered.
async fn store_backed(snapshot: &Snapshot) -> Database {
    Database::open_in_memory_unattached()
        .await
        .unwrap()
        .with_shared_reader(Arc::new(SubscriptionBoardReads::new(deliver(snapshot))))
}

/// **Every routed read answers from the store, and answers what SQLite did.**
#[tokio::test]
async fn a_store_backed_board_reads_back_what_the_store_holds() {
    let sqlite = board().await;
    let snapshot = dump_from_sqlite(&sqlite).await.unwrap();
    let store = store_backed(&snapshot).await;

    // Tasks.
    let tasks = sqlite.list_all().await.unwrap();
    assert!(tasks.len() >= 2);
    assert_eq!(tasks, store.list_all().await.unwrap());
    let live = sqlite.list_live_agent_tasks().await.unwrap();
    assert!(!live.is_empty(), "the fixture must have a live agent task");
    assert_eq!(live, store.list_live_agent_tasks().await.unwrap());
    for task in &tasks {
        assert_eq!(
            sqlite.get_task(task.id).await.unwrap(),
            store.get_task(task.id).await.unwrap()
        );
        assert!(store.task_exists(task.id).await.unwrap());
        assert_eq!(
            sqlite.list_watchers_of(task.id).await.unwrap(),
            store.list_watchers_of(task.id).await.unwrap()
        );
    }
    assert!(!store.task_exists(TaskId(9_999)).await.unwrap());
    assert_eq!(
        sqlite.find_task_by_plan("/plans/full.md").await.unwrap(),
        store.find_task_by_plan("/plans/full.md").await.unwrap()
    );
    assert!(store
        .find_task_by_plan("/plans/full.md")
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        sqlite.list_all_tasks_with_epic_id().await.unwrap(),
        store.list_all_tasks_with_epic_id().await.unwrap()
    );

    // Epics.
    let epics = sqlite.list_epics().await.unwrap();
    assert!(epics.len() >= 3);
    assert_eq!(epics, store.list_epics().await.unwrap());
    assert_eq!(
        sqlite.list_root_epics().await.unwrap(),
        store.list_root_epics().await.unwrap()
    );
    for epic in &epics {
        assert_eq!(
            sqlite.get_epic(epic.id).await.unwrap(),
            store.get_epic(epic.id).await.unwrap()
        );
        assert_eq!(
            sqlite.list_sub_epics(epic.id).await.unwrap(),
            store.list_sub_epics(epic.id).await.unwrap()
        );
        assert_eq!(
            sqlite.list_tasks_for_epic(epic.id).await.unwrap(),
            store.list_tasks_for_epic(epic.id).await.unwrap()
        );
    }
    assert_eq!(store.get_epic(EpicId(9_999)).await.unwrap(), None);

    // Repo configuration.
    assert_eq!(
        sqlite.list_repo_paths().await.unwrap(),
        store.list_repo_paths().await.unwrap()
    );
    assert_eq!(
        sqlite.list_all_base_branches().await.unwrap(),
        store.list_all_base_branches().await.unwrap()
    );
    for path in ["/repo", "/other", "/nowhere"] {
        assert_eq!(
            sqlite.get_verify_command(path).await.unwrap(),
            store.get_verify_command(path).await.unwrap(),
            "{path}"
        );
    }

    // Subscriptions.
    let subscribed = sqlite.subscribed_epics(SUBSCRIBER).await.unwrap();
    assert_eq!(subscribed.len(), 2);
    assert_eq!(
        subscribed,
        store.subscribed_epics(SUBSCRIBER).await.unwrap()
    );
    assert!(store.subscribed_epics("ffff").await.unwrap().is_empty());

    // Settings.
    assert_eq!(
        store
            .get_setting_string("some_key")
            .await
            .unwrap()
            .as_deref(),
        Some("some value")
    );
    assert_eq!(store.get_setting_bool("a_flag").await.unwrap(), Some(true));
    assert_eq!(store.get_setting_string("absent").await.unwrap(), None);
}

/// A row that lives only in the handle's own SQLite file is invisible once a
/// reader is attached. The store is the one copy; a local row that nothing
/// writes any more must not leak back onto a board that reads from the store.
#[tokio::test]
async fn a_store_backed_board_never_reads_its_own_sqlite_file() {
    let snapshot = dump_from_sqlite(&Database::open_in_memory_unattached().await.unwrap())
        .await
        .unwrap();
    let local = board().await;
    let local_task = local.list_all().await.unwrap()[0].id;
    let store = local.with_shared_reader(Arc::new(SubscriptionBoardReads::new(deliver(&snapshot))));

    assert!(store.list_all().await.unwrap().is_empty());
    assert_eq!(store.get_task(local_task).await.unwrap(), None);
    assert!(store.list_epics().await.unwrap().is_empty());
    assert!(store.list_repo_paths().await.unwrap().is_empty());
    assert!(store.subscribed_epics(SUBSCRIBER).await.unwrap().is_empty());
    assert_eq!(store.get_setting_string("some_key").await.unwrap(), None);
    assert!(store.list_watchers_of(local_task).await.unwrap().is_empty());
}
