use super::*;

#[tokio::test]
async fn test_record_and_query_usage() {
    use crate::models::{UsageActor, UsageCategory, UsageEvent};
    use crate::store::UsageStore;

    let db = in_memory_db().await;

    db.record_usage_event(&UsageEvent {
        category: UsageCategory::Keybinding,
        action: "dispatch_task".to_string(),
        detail: Some("d".to_string()),
        actor: UsageActor::Human,
    })
    .await
    .unwrap();

    db.record_usage_event(&UsageEvent {
        category: UsageCategory::Keybinding,
        action: "dispatch_task".to_string(),
        detail: Some("d".to_string()),
        actor: UsageActor::Human,
    })
    .await
    .unwrap();

    db.record_usage_event(&UsageEvent {
        category: UsageCategory::McpTool,
        action: "create_task".to_string(),
        detail: Some("create_task".to_string()),
        actor: UsageActor::Agent,
    })
    .await
    .unwrap();

    let query = crate::store::UsageQuery::default();
    let results = db.query_usage(&query).await.unwrap();

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].action, "create_task");
    assert_eq!(results[0].count, 1);
    assert_eq!(results[1].action, "dispatch_task");
    assert_eq!(results[1].count, 2);
}

#[tokio::test]
async fn test_query_usage_filters_by_category_and_actor() {
    use crate::models::{UsageActor, UsageCategory, UsageEvent};
    use crate::store::{UsageQuery, UsageStore};

    let db = in_memory_db().await;

    for ev in [
        UsageEvent {
            category: UsageCategory::Keybinding,
            action: "a".into(),
            detail: None,
            actor: UsageActor::Human,
        },
        UsageEvent {
            category: UsageCategory::McpTool,
            action: "b".into(),
            detail: None,
            actor: UsageActor::Agent,
        },
        UsageEvent {
            category: UsageCategory::McpTool,
            action: "c".into(),
            detail: None,
            actor: UsageActor::Agent,
        },
    ] {
        db.record_usage_event(&ev).await.unwrap();
    }

    let only_mcp = db
        .query_usage(&UsageQuery {
            category: Some("mcp_tool".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(only_mcp.len(), 2);
    assert!(only_mcp.iter().all(|r| r.category == "mcp_tool"));

    let only_human = db
        .query_usage(&UsageQuery {
            actor: Some("human".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(only_human.len(), 1);
    assert_eq!(only_human[0].action, "a");
    assert_eq!(only_human[0].actor, "human");
}
