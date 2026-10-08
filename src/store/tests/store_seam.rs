//! One store — Phase 12a of the SpacetimeDB migration (task #4916).
//!
//! Phase 3 split the store into a shared half and a local half so a second
//! backend could implement one of them. With the store mandatory there is one
//! backend, and [`TaskStore`] is the one complete store. Every test here
//! reaches the tables through `&dyn TaskStore` rather than through the
//! concrete type — reaching a method on `Store` proves nothing about the
//! trait. The compile-time half (the old local half is gone, and is not a
//! complete store) is the compile-fail doc tests on `TaskStore` in
//! `src/store/mod.rs`.

use super::*;

/// Every shared table is reachable through the one store handle.
/// `TaskStore`'s doc comment carries the table-by-table mapping.
#[tokio::test]
async fn the_store_reaches_every_shared_table() {
    let db = in_memory_db().await;
    let shared: &dyn TaskStore = &db;

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
            epic_id: Some(epic.id),
            ..CreateTaskRequest::fixture("Task", "/repo")
        })
        .await
        .unwrap();
    assert!(shared.get_task(task_id).await.unwrap().is_some());

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

    // learnings — moved here in Phase 10 (task #4914): the knowledge base is
    // genuinely team-shared, not per-machine, so it belongs on the shared half.
    let learning = shared
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
    assert!(shared.get_learning(learning).await.unwrap().is_some());

    // learning_retrievals
    shared
        .record_retrieval(
            task_id,
            learning,
            crate::models::RetrievalSource::QueryLearnings,
        )
        .await
        .unwrap();
    assert_eq!(
        shared
            .list_retrievals_for_task(task_id)
            .await
            .unwrap()
            .len(),
        1
    );

    // usage_events — moved here in Phase 11 (task #4915): telemetry with no
    // user-observable rule beyond "recorded", same reasoning as learnings.
    shared
        .record_usage_event(&crate::models::UsageEvent {
            category: crate::models::UsageCategory::Keybinding,
            action: "dispatch_task".to_string(),
            detail: None,
            actor: crate::models::UsageActor::Human,
        })
        .await
        .unwrap();
    assert_eq!(
        shared
            .query_usage(&crate::store::UsageQuery::default())
            .await
            .unwrap()
            .len(),
        1
    );
}

/// The same handle reaches the tables the old local half covered: key/value
/// settings and managed-feed config.
#[tokio::test]
async fn the_store_reaches_the_settings_tables() {
    let db = in_memory_db().await;
    let local: &dyn TaskStore = &db;

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
}

/// `rescope_epic_learnings` writes the *learnings* table. Both `learnings`
/// and `epics` are shared tables as of Phase 10 (task #4914), so this is an
/// ordinary store operation now — no longer a local write reached
/// through epic-shaped arguments, which was the anomaly this test used to
/// document (see `docs/conventions.md`'s store-seam section).
#[tokio::test]
async fn rescoping_epic_learnings_is_a_shared_operation() {
    let db = in_memory_db().await;
    let shared: &dyn TaskStore = &db;

    let from = shared.create_epic("From", "", None).await.unwrap();
    let to = shared.create_epic("To", "", None).await.unwrap();
    let learning = shared
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

    shared.rescope_epic_learnings(from.id, to.id).await.unwrap();

    let moved = shared.get_learning(learning).await.unwrap().unwrap();
    assert_eq!(
        moved.scope_ref.as_deref(),
        Some(to.id.0.to_string().as_str())
    );
}
