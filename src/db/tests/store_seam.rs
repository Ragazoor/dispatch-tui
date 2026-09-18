//! The shared/local store seam — Phase 3 of the SpacetimeDB migration.
//!
//! [`SharedDomainStore`] covers the tables `spacetime::snapshot::SharedTable`
//! names; [`LocalStore`] covers what stays in SQLite. `Database` satisfies both
//! halves today. A second backend has to satisfy only the shared half, which is
//! what makes the store swappable, so every test here exercises the halves
//! through `&dyn` rather than through the concrete type — reaching a method on
//! `Database` proves nothing about the seam.
//!
//! The compile-time direction of the seam (a local method being *unreachable*
//! through the shared half, and the reverse) is pinned by the compile-fail doc
//! tests on the two traits in `src/db/mod.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

/// Every shared table that exists in SQLite today is reachable through the
/// shared half alone. `SharedDomainStore`'s doc comment carries the table-by-
/// table mapping and the one gap (`subscriptions`, which Phase 4 introduces).
#[tokio::test]
async fn shared_half_reaches_every_shared_table() {
    let db = in_memory_db().await;
    let shared: &dyn SharedDomainStore = &db;

    // repo_paths
    shared.save_repo_path("/repo").await.unwrap();
    assert_eq!(shared.list_repo_paths().await.unwrap(), vec!["/repo"]);

    // repo_base_branches
    shared.record_base_branch("/repo", "main").await.unwrap();
    assert_eq!(
        shared.list_all_base_branches().await.unwrap(),
        vec![("/repo".to_string(), "main".to_string())]
    );

    // hosts
    let (host_id, label) = shared.ensure_host_identity().await.unwrap();
    assert!(!host_id.is_empty());
    assert_eq!(label, None);

    // epics
    let epic = shared.create_epic("Epic", "", None).await.unwrap();
    assert!(shared.get_epic(epic.id).await.unwrap().is_some());

    // tasks
    let task_id = shared
        .create_task(CreateTaskRequest {
            title: "Task",
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
    assert!(shared.get_task(task_id).await.unwrap().is_some());

    // todos
    let todo_id = shared.insert_todo(todo("Todo")).await.unwrap();
    assert_eq!(shared.list_todos().await.unwrap()[0].id, todo_id);

    // task_watchers
    let watcher = make_task(&db, "Watcher").await;
    shared
        .create_task_watcher(watcher.id, task_id)
        .await
        .unwrap();
    assert_eq!(
        shared.list_watchers_of(task_id).await.unwrap(),
        [watcher.id]
    );

    // task_subagents
    let now = chrono::Utc::now();
    assert_eq!(
        shared
            .subagent_start(task_id, "agent-1", "session-1", now)
            .await
            .unwrap(),
        1
    );

    // task_shells
    assert_eq!(
        shared
            .shell_start(task_id, "shell-1", "session-1", now)
            .await
            .unwrap(),
        1
    );
}

/// The local half reaches what stays in SQLite: key/value settings, filter
/// presets, managed-feed config, learnings and usage.
#[tokio::test]
async fn local_half_reaches_every_local_table() {
    let db = in_memory_db().await;
    let local: &dyn LocalStore = &db;

    // settings
    local.set_setting_bool("notifications", true).await.unwrap();
    assert_eq!(
        local.get_setting_bool("notifications").await.unwrap(),
        Some(true)
    );

    // managed-feed config, also settings-backed
    local
        .set_reviews_feed_command(Some("gh pr list"))
        .await
        .unwrap();
    assert_eq!(
        local.get_reviews_feed_command().await.unwrap().as_deref(),
        Some("gh pr list")
    );

    // filter_presets
    local
        .save_filter_preset("preset", &["/repo".to_string()], "include")
        .await
        .unwrap();
    assert_eq!(local.list_filter_presets().await.unwrap().len(), 1);

    // learnings
    let learning = local
        .create_learning(CreateLearningRow {
            kind: crate::models::LearningKind::Convention,
            summary: "A convention",
            detail: None,
            scope: crate::models::LearningScope::User,
            scope_ref: None,
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();
    assert!(local.get_learning(learning).await.unwrap().is_some());

    // usage_events
    local
        .query_usage(&crate::db::UsageQuery::default())
        .await
        .unwrap();
}

/// `rescope_epic_learnings` writes the *learnings* table, which stays in
/// SQLite. It sat on `EpicCrud` — a shared trait — so a SpacetimeDB backend
/// would have had to implement a local-table write. It belongs to the local
/// half.
#[tokio::test]
async fn rescoping_epic_learnings_is_a_local_operation() {
    let db = in_memory_db().await;
    let local: &dyn LocalStore = &db;
    let shared: &dyn SharedDomainStore = &db;

    let from = shared.create_epic("From", "", None).await.unwrap();
    let to = shared.create_epic("To", "", None).await.unwrap();
    let learning = local
        .create_learning(CreateLearningRow {
            kind: crate::models::LearningKind::Convention,
            summary: "Scoped to an epic",
            detail: None,
            scope: crate::models::LearningScope::Epic,
            scope_ref: Some(&from.id.0.to_string()),
            tags: &[],
            source_task_id: None,
            embedding: None,
        })
        .await
        .unwrap();

    local.rescope_epic_learnings(from.id, to.id).await.unwrap();

    let moved = local.get_learning(learning).await.unwrap().unwrap();
    assert_eq!(
        moved.scope_ref.as_deref(),
        Some(to.id.0.to_string().as_str())
    );
}
