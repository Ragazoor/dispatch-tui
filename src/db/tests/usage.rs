#![allow(clippy::unwrap_used)]
use super::*;

#[tokio::test]
async fn test_usage_events_table_created() {
    let db = in_memory_db().await;
    let cols: Vec<String> = db
        .db_call(|conn| {
            let mut stmt = conn.prepare(
                "SELECT name FROM pragma_table_info('usage_events') ORDER BY cid",
            )?;
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
