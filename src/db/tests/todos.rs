#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[tokio::test]
async fn migration_v67_creates_todos_table() {
    let db = in_memory_db().await;

    // Verify the todos table exists
    let table_exists: bool = db
        .db_call(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='todos'",
                [],
                |row| {
                    let count: i64 = row.get(0)?;
                    Ok(count > 0)
                },
            )
            .map_err(anyhow::Error::from)
        })
        .await
        .unwrap();

    assert!(table_exists, "todos table should exist after migration v67");
}
