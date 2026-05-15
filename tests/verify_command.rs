//! Integration: verify_command stored under a path string equal to
//! task.repo_path is found by the dispatch lookup.

use dispatch_tui::db::{Database, SettingsStore};
use dispatch_tui::dispatch::fetch_verify_command;

#[tokio::test]
async fn verify_command_lookup_matches_task_repo_path() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let db = Database::open(tmp.path()).await.unwrap();

    let path = "/home/me/repo";
    db.set_verify_command(path, Some("cargo test")).await.unwrap();

    // Simulate what exec_dispatch_agent does:
    let fetched = fetch_verify_command(&db, path).await;
    assert_eq!(fetched.as_deref(), Some("cargo test"));
}

#[tokio::test]
async fn verify_command_lookup_returns_none_for_unregistered_path() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let db = Database::open(tmp.path()).await.unwrap();

    let fetched = fetch_verify_command(&db, "/not/registered").await;
    assert_eq!(fetched, None);
}
