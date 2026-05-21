#![allow(clippy::unwrap_used)]
use super::*;

#[tokio::test]
async fn test_usage_events_table_created() {
    let db = in_memory_db().await;
    let cols: Vec<String> = db
        .db_call(|conn| {
            let mut stmt =
                conn.prepare("SELECT name FROM pragma_table_info('usage_events') ORDER BY cid")?;
            let cols = stmt
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(cols)
        })
        .await
        .unwrap();
    assert_eq!(
        cols,
        vec!["id", "recorded_at", "category", "action", "detail", "actor"]
    );
}

#[tokio::test]
async fn test_record_and_query_usage() {
    use crate::db::UsageStore;
    use crate::models::{UsageActor, UsageCategory, UsageEvent};

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

    let query = crate::db::UsageQuery::default();
    let results = db.query_usage(&query).await.unwrap();

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].action, "create_task");
    assert_eq!(results[0].count, 1);
    assert_eq!(results[1].action, "dispatch_task");
    assert_eq!(results[1].count, 2);
}

#[tokio::test]
async fn test_usage_cap_enforcement() {
    use crate::db::{UsageCap, UsageStore};
    use crate::models::{UsageActor, UsageCategory, UsageEvent};

    let db = in_memory_db().await;
    let small_cap = UsageCap(3);

    for i in 0..5u32 {
        db.record_usage_event_with_cap(
            &UsageEvent {
                category: UsageCategory::Keybinding,
                action: format!("action_{i}"),
                detail: None,
                actor: UsageActor::Human,
            },
            small_cap,
        )
        .await
        .unwrap();
    }

    let count: i64 = db
        .db_call(|conn| {
            conn.query_row("SELECT COUNT(*) FROM usage_events", [], |r| r.get(0))
                .map_err(Into::into)
        })
        .await
        .unwrap();
    assert_eq!(count, 3);
}
