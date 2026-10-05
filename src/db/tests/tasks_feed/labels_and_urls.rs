use super::*;

#[tokio::test]
async fn feed_item_legacy_json_deserializes_with_default_labels_and_sort_order() {
    // Wire-compat: scripts written before labels/sort_order existed must still
    // parse. Both fields are #[serde(default)].
    let legacy_json = r#"{
        "external_id": "ext-1",
        "title": "Legacy",
        "description": "",
        "url": "",
        "status": "backlog",
        "tag": "bug"
    }"#;
    let item: crate::models::FeedItem = serde_json::from_str(legacy_json).unwrap();
    assert!(item.labels.is_empty());
    assert_eq!(item.sort_order, None);
    // wrap_up_mode is #[serde(default)]: absent -> None.
    assert_eq!(item.wrap_up_mode, None);
}

#[tokio::test]
async fn feed_item_deserializes_wrap_up_mode() {
    // A feed script may declare wrap_up_mode; "pr" parses to WrapUpMode::Pr
    // (WrapUpMode derives Deserialize with rename_all = "lowercase").
    let json = r#"{
        "external_id": "cve:org/repo#1",
        "title": "[CRITICAL] repo: CVE-1",
        "description": "",
        "status": "backlog",
        "tag": "fix",
        "wrap_up_mode": "pr"
    }"#;
    let item: crate::models::FeedItem = serde_json::from_str(json).unwrap();
    assert_eq!(item.wrap_up_mode, Some(crate::models::WrapUpMode::Pr));
}

#[tokio::test]
async fn upsert_feed_tasks_writes_labels_and_sort_order_on_insert() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![crate::models::FeedItem {
        external_id: "ext-1".to_string(),
        title: "CRITICAL CVE-1234".to_string(),
        description: "".to_string(),
        url: String::new(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::Fix,
        labels: vec!["scala-common".to_string()],
        sort_order: Some(1),
        signals: vec![],
        wrap_up_mode: None,
    }];
    db.upsert_feed_tasks(epic.id, &items, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].labels, vec!["scala-common".to_string()]);
    assert_eq!(tasks[0].sort_order, Some(1));
}

#[tokio::test]
async fn upsert_feed_tasks_replaces_labels_and_sort_order_on_conflict() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let initial = vec![crate::models::FeedItem {
        external_id: "ext-1".to_string(),
        title: "T".to_string(),
        description: "".to_string(),
        url: String::new(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::Fix,
        labels: vec!["repo-a".to_string()],
        sort_order: Some(3),
        signals: vec![],
        wrap_up_mode: None,
    }];
    db.upsert_feed_tasks(epic.id, &initial, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();
    // Simulate user moving the task — status & repo_path must be preserved.
    let task_id = db.list_tasks_for_epic(epic.id).await.unwrap()[0].id;
    db.patch_task(
        task_id,
        &TaskPatch::new()
            .status(TaskStatus::Running)
            .repo_path("/manually-fixed"),
    )
    .await
    .unwrap();

    let updated = vec![crate::models::FeedItem {
        external_id: "ext-1".to_string(),
        title: "T".to_string(),
        description: "".to_string(),
        url: String::new(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::Fix,
        labels: vec!["repo-a".to_string(), "security".to_string()],
        sort_order: Some(1),
        signals: vec![],
        wrap_up_mode: None,
    }];
    db.upsert_feed_tasks(epic.id, &updated, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    let task = db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(
        task.labels,
        vec!["repo-a".to_string(), "security".to_string()],
        "labels are feed-controlled and replaced on conflict"
    );
    assert_eq!(
        task.sort_order,
        Some(1),
        "sort_order is replaced on conflict"
    );
    // User-owned fields preserved.
    assert_eq!(task.status, TaskStatus::Running);
    assert_eq!(task.repo_path, "/manually-fixed");
}

#[tokio::test]
async fn upsert_feed_tasks_sets_wrap_up_mode_on_insert() {
    let db = in_memory_db().await;
    let epic = db.create_epic("CVE", "", None).await.unwrap();
    let items = vec![
        crate::models::FeedItem {
            sort_order: Some(1),
            wrap_up_mode: Some(crate::models::WrapUpMode::Pr),
            ..make_feed_item("cve:org/repo#1", "[CRITICAL] repo: CVE-1")
        },
        crate::models::FeedItem {
            sort_order: Some(2),
            // Omitted by the script -> stays NULL on the task.
            wrap_up_mode: None,
            ..make_feed_item("cve:org/repo#2", "[LOW] repo: CVE-2")
        },
    ];
    db.upsert_feed_tasks(
        epic.id,
        &items,
        &["/repo".to_string(), "/repo".to_string()],
        &main_branches(2),
    )
    .await
    .unwrap();

    let mut tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    tasks.sort_by_key(|t| t.sort_order);
    assert_eq!(tasks.len(), 2);
    assert_eq!(
        tasks[0].wrap_up_mode,
        Some(crate::models::WrapUpMode::Pr),
        "declared wrap_up_mode is applied on insert"
    );
    assert_eq!(
        tasks[1].wrap_up_mode, None,
        "omitted wrap_up_mode leaves the task's value NULL"
    );
}

#[tokio::test]
async fn upsert_feed_tasks_preserves_wrap_up_mode_on_conflict() {
    let db = in_memory_db().await;
    let epic = db.create_epic("CVE", "", None).await.unwrap();
    let initial = vec![crate::models::FeedItem {
        wrap_up_mode: Some(crate::models::WrapUpMode::Pr),
        ..make_feed_item("cve:org/repo#1", "T")
    }];
    db.upsert_feed_tasks(epic.id, &initial, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    // User changes the wrap-up choice manually.
    let task_id = db.list_tasks_for_epic(epic.id).await.unwrap()[0].id;
    db.patch_task(
        task_id,
        &TaskPatch::new().wrap_up_mode(Some(crate::models::WrapUpMode::Rebase)),
    )
    .await
    .unwrap();

    // Feed re-polls the same alert, still declaring "pr".
    db.upsert_feed_tasks(epic.id, &initial, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    let task = db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(
        task.wrap_up_mode,
        Some(crate::models::WrapUpMode::Rebase),
        "wrap_up_mode is insert-only; a user's manual change survives feed refreshes"
    );
}

#[tokio::test]
async fn upsert_feed_tasks_sets_pr_url_from_item_url_on_insert() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![
        crate::models::FeedItem {
            external_id: "dep:org/repo#42".to_string(),
            title: "#42 Bump foo".to_string(),
            description: "".to_string(),
            url: "https://github.com/org/repo/pull/42".to_string(),
            url_type: None,
            status: TaskStatus::Backlog,
            tag: crate::models::TaskTag::PrReview,
            labels: vec![],
            sort_order: None,
            signals: vec![],
            wrap_up_mode: None,
        },
        crate::models::FeedItem {
            external_id: "dep:org/repo#43".to_string(),
            title: "#43 Bump bar".to_string(),
            description: "".to_string(),
            url: "https://github.com/org/repo/pull/43".to_string(),
            url_type: None,
            status: TaskStatus::Backlog,
            tag: crate::models::TaskTag::Dependabot,
            labels: vec![],
            sort_order: None,
            signals: vec![],
            wrap_up_mode: None,
        },
        crate::models::FeedItem {
            external_id: "cve:GHSA-xxxx".to_string(),
            title: "CRITICAL CVE-1234".to_string(),
            description: "".to_string(),
            url: "https://github.com/org/repo/security/advisories/GHSA-xxxx".to_string(),
            url_type: None,
            status: TaskStatus::Backlog,
            tag: crate::models::TaskTag::Fix,
            labels: vec![],
            sort_order: None,
            signals: vec![],
            wrap_up_mode: None,
        },
    ];
    db.upsert_feed_tasks(
        epic.id,
        &items,
        &vec!["/repo".to_string(); 3],
        &main_branches(3),
    )
    .await
    .unwrap();

    let mut tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    tasks.sort_by(|a, b| a.external_id.cmp(&b.external_id));
    assert_eq!(tasks.len(), 3);
    assert_eq!(
        tasks[0].url.as_ref().map(|u| u.url.as_str()),
        Some("https://github.com/org/repo/security/advisories/GHSA-xxxx"),
        "non-empty url copied to url regardless of tag (Fix)"
    );
    assert_eq!(
        tasks[0].url.as_ref().map(|u| u.url_type),
        Some(crate::models::UrlType::Other),
        "non-PR/issue url inferred as other"
    );
    assert_eq!(
        tasks[1].url.as_ref().map(|u| u.url.as_str()),
        Some("https://github.com/org/repo/pull/42"),
        "PrReview items keep url-on-insert"
    );
    assert_eq!(
        tasks[1].url.as_ref().map(|u| u.url_type),
        Some(crate::models::UrlType::Pr),
        "pull url inferred as pr"
    );
    assert_eq!(
        tasks[2].url.as_ref().map(|u| u.url.as_str()),
        Some("https://github.com/org/repo/pull/43"),
        "Dependabot items get url-on-insert"
    );
}

#[tokio::test]
async fn upsert_feed_tasks_leaves_pr_url_null_when_item_url_empty() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let items = vec![crate::models::FeedItem {
        external_id: "ext-no-url".to_string(),
        title: "no url".to_string(),
        description: "".to_string(),
        url: "".to_string(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::Dependabot,
        labels: vec![],
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    }];
    db.upsert_feed_tasks(epic.id, &items, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].url.is_none());
}

#[tokio::test]
async fn upsert_feed_tasks_backfills_null_pr_url_on_conflict() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    // First emission: no URL — task created with url = NULL.
    let initial = vec![crate::models::FeedItem {
        external_id: "dep:org/repo#42".to_string(),
        title: "#42 Bump foo".to_string(),
        description: "".to_string(),
        url: "".to_string(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::Dependabot,
        labels: vec![],
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    }];
    db.upsert_feed_tasks(epic.id, &initial, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();
    let task_id = db.list_tasks_for_epic(epic.id).await.unwrap()[0].id;
    assert!(
        db.get_task(task_id).await.unwrap().unwrap().url.is_none(),
        "precondition: url is null after first upsert"
    );

    // Second emission: same external_id but now with a URL.
    let refreshed = vec![crate::models::FeedItem {
        url: "https://github.com/org/repo/pull/42".to_string(),
        ..initial[0].clone()
    }];
    db.upsert_feed_tasks(
        epic.id,
        &refreshed,
        &["/repo".to_string()],
        &main_branches(1),
    )
    .await
    .unwrap();

    let task = db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(
        task.url.as_ref().map(|u| u.url.as_str()),
        Some("https://github.com/org/repo/pull/42"),
        "null url is backfilled from item.url on conflict"
    );
    assert_eq!(
        task.url.as_ref().map(|u| u.url_type),
        Some(crate::models::UrlType::Pr),
        "backfilled url_type is inferred"
    );
}

#[tokio::test]
async fn upsert_feed_tasks_preserves_pr_url_on_conflict() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();
    let initial = vec![crate::models::FeedItem {
        external_id: "dep:org/repo#42".to_string(),
        title: "#42 Bump foo".to_string(),
        description: "".to_string(),
        url: "https://github.com/org/repo/pull/42".to_string(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::PrReview,
        labels: vec![],
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    }];
    db.upsert_feed_tasks(epic.id, &initial, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();
    let task_id = db.list_tasks_for_epic(epic.id).await.unwrap()[0].id;
    let manual = crate::models::TaskUrl::new(
        "https://github.com/org/repo/pull/999",
        crate::models::UrlType::Pr,
    );
    db.patch_task(task_id, &TaskPatch::new().url(Some(&manual)))
        .await
        .unwrap();

    // Re-run upsert; url on the existing task must not be overwritten.
    db.upsert_feed_tasks(epic.id, &initial, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    let task = db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(
        task.url.as_ref().map(|u| u.url.as_str()),
        Some("https://github.com/org/repo/pull/999")
    );
}

#[tokio::test]
async fn feed_upsert_infers_url_type_and_backfills_atomically() {
    use crate::models::{TaskUrl, UrlType};
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();

    let feed_item = |external_id: &str, url: &str| crate::models::FeedItem {
        external_id: external_id.to_string(),
        title: "t".to_string(),
        description: "".to_string(),
        url: url.to_string(),
        url_type: None,
        status: TaskStatus::Backlog,
        tag: crate::models::TaskTag::Dependabot,
        labels: vec![],
        sort_order: None,
        signals: vec![],
        wrap_up_mode: None,
    };
    // First emit: a PR URL is inferred as pr.
    let items = vec![feed_item("ext-1", "https://github.com/o/r/pull/5")];
    db.upsert_feed_tasks(epic.id, &items, &["/r".into()], &main_branches(1))
        .await
        .unwrap();
    let t = db.list_tasks_for_epic(epic.id).await.unwrap().remove(0);
    assert_eq!(
        t.url,
        Some(TaskUrl::new("https://github.com/o/r/pull/5", UrlType::Pr))
    );

    // Conflict re-emit with a DIFFERENT url must NOT clobber the existing pair.
    let items = vec![feed_item("ext-1", "https://github.com/o/r/pull/999")];
    db.upsert_feed_tasks(epic.id, &items, &["/r".into()], &main_branches(1))
        .await
        .unwrap();
    let t = db.list_tasks_for_epic(epic.id).await.unwrap().remove(0);
    assert_eq!(
        t.url,
        Some(TaskUrl::new("https://github.com/o/r/pull/5", UrlType::Pr)),
        "existing url/url_type must be preserved on conflict"
    );
}

#[tokio::test]
async fn upsert_feed_tasks_explicit_url_type_wins_over_inference() {
    use crate::models::{TaskUrl, UrlType};
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();

    // A Dependabot alert URL has no /pull/ or /issues/ segment, so inference
    // would classify it as Other. The declared security_alert must win.
    let alert_url = "https://github.com/org/repo/security/dependabot/7";
    let items = vec![
        crate::models::FeedItem {
            url: alert_url.to_string(),
            url_type: Some(UrlType::SecurityAlert),
            ..make_feed_item("ext-declared", "declared")
        },
        crate::models::FeedItem {
            url: alert_url.to_string(),
            url_type: None,
            ..make_feed_item("ext-inferred", "inferred")
        },
    ];
    db.upsert_feed_tasks(
        epic.id,
        &items,
        &["/repo".to_string(), "/repo".to_string()],
        &main_branches(2),
    )
    .await
    .unwrap();

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    let by_ext = |ext: &str| {
        tasks
            .iter()
            .find(|t| t.external_id.as_deref() == Some(ext))
            .unwrap()
    };
    assert_eq!(
        by_ext("ext-declared").url,
        Some(TaskUrl::new(alert_url, UrlType::SecurityAlert)),
        "explicit url_type is stored verbatim"
    );
    assert_eq!(
        by_ext("ext-inferred").url,
        Some(TaskUrl::new(alert_url, UrlType::Other)),
        "absent url_type falls back to inference"
    );
}

#[tokio::test]
async fn upsert_feed_tasks_backfill_uses_declared_url_type() {
    use crate::models::{TaskUrl, UrlType};
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();

    // First emission: no URL — task created with url = NULL.
    let initial = vec![make_feed_item("ext-1", "alert")];
    db.upsert_feed_tasks(epic.id, &initial, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();
    let task_id = db.list_tasks_for_epic(epic.id).await.unwrap()[0].id;
    assert!(
        db.get_task(task_id).await.unwrap().unwrap().url.is_none(),
        "precondition: url is null after first upsert"
    );

    // Refresh with a URL and a declared type that inference cannot reach.
    let alert_url = "https://github.com/org/repo/security/dependabot/7";
    let refreshed = vec![crate::models::FeedItem {
        url: alert_url.to_string(),
        url_type: Some(UrlType::SecurityAlert),
        ..initial[0].clone()
    }];
    db.upsert_feed_tasks(
        epic.id,
        &refreshed,
        &["/repo".to_string()],
        &main_branches(1),
    )
    .await
    .unwrap();

    let task = db.get_task(task_id).await.unwrap().unwrap();
    assert_eq!(
        task.url,
        Some(TaskUrl::new(alert_url, UrlType::SecurityAlert)),
        "backfilled url_type uses the declared type, not inference"
    );
}

#[tokio::test]
async fn upsert_feed_tasks_can_purge_task_with_associated_learning() {
    use crate::models::{LearningKind, LearningScope};

    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();

    // First feed run: creates a task.
    let initial = vec![make_feed_item("ext-1", "first")];
    db.upsert_feed_tasks(epic.id, &initial, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();
    let task_id = db.list_tasks_for_epic(epic.id).await.unwrap()[0].id;

    // The dispatched agent records a learning referencing the task as its source.
    db.create_learning(CreateLearningRow {
        kind: LearningKind::Pitfall,
        summary: "watch out",
        detail: None,
        scope: LearningScope::User,
        scope_ref: None,
        tags: &[],
        source_task_id: Some(task_id),
        embedding: None,
    })
    .await
    .unwrap();

    // Second feed run with a different external_id — the previous task should
    // be purged. Without ON DELETE SET NULL on learnings.source_task_id, this
    // fails with a FK violation.
    let next = vec![make_feed_item("ext-2", "second")];
    db.upsert_feed_tasks(epic.id, &next, &["/repo".to_string()], &main_branches(1))
        .await
        .expect("stale feed task with associated learning should be purgeable");

    let tasks = db.list_tasks_for_epic(epic.id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].external_id.as_deref(), Some("ext-2"));
}

#[tokio::test]
async fn upsert_feed_tasks_can_purge_stale_task() {
    let db = in_memory_db().await;
    let epic = db.create_epic("E", "", None).await.unwrap();

    let initial = vec![make_feed_item("ext-1", "first")];
    db.upsert_feed_tasks(epic.id, &initial, &["/repo".to_string()], &main_branches(1))
        .await
        .unwrap();

    let next = vec![make_feed_item("ext-2", "second")];
    db.upsert_feed_tasks(epic.id, &next, &["/repo".to_string()], &main_branches(1))
        .await
        .expect("stale feed task should be purgeable");
}
