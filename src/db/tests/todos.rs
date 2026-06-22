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

#[tokio::test]
async fn insert_and_list_todos() {
    let db = in_memory_db().await;

    let id1 = db.insert_todo("Buy milk").await.unwrap();
    let id2 = db.insert_todo("Write tests").await.unwrap();

    let todos = db.list_todos().await.unwrap();
    assert_eq!(todos.len(), 2);
    assert_eq!(todos[0].id, id1);
    assert_eq!(todos[0].title, "Buy milk");
    assert!(!todos[0].done);
    assert_eq!(todos[1].id, id2);
    assert_eq!(todos[1].title, "Write tests");
}

#[tokio::test]
async fn sort_order_auto_increments() {
    let db = in_memory_db().await;

    db.insert_todo("First").await.unwrap();
    db.insert_todo("Second").await.unwrap();
    db.insert_todo("Third").await.unwrap();

    let todos = db.list_todos().await.unwrap();
    assert_eq!(todos.len(), 3);
    // Each new item gets sort_order = previous max + 1, starting at 0
    assert_eq!(todos[0].sort_order, 0);
    assert_eq!(todos[1].sort_order, 1);
    assert_eq!(todos[2].sort_order, 2);
}

#[tokio::test]
async fn patch_todo_title() {
    let db = in_memory_db().await;
    let id = db.insert_todo("Old title").await.unwrap();

    db.patch_todo(id, &TodoPatch::new().title("New title"))
        .await
        .unwrap();

    let todos = db.list_todos().await.unwrap();
    assert_eq!(todos[0].title, "New title");
}

#[tokio::test]
async fn patch_todo_done() {
    let db = in_memory_db().await;
    let id = db.insert_todo("Task").await.unwrap();

    db.patch_todo(id, &TodoPatch::new().done(true))
        .await
        .unwrap();

    let todos = db.list_todos().await.unwrap();
    assert!(todos[0].done);
}

#[tokio::test]
async fn patch_todo_sort_order() {
    let db = in_memory_db().await;
    let id1 = db.insert_todo("First").await.unwrap();
    let id2 = db.insert_todo("Second").await.unwrap();

    // Swap sort_orders
    db.patch_todo(id1, &TodoPatch::new().sort_order(10))
        .await
        .unwrap();
    db.patch_todo(id2, &TodoPatch::new().sort_order(0))
        .await
        .unwrap();

    let todos = db.list_todos().await.unwrap();
    // After swap, id2 has lower sort_order so comes first
    assert_eq!(todos[0].id, id2);
    assert_eq!(todos[1].id, id1);
}

#[tokio::test]
async fn patch_todo_no_changes_is_noop() {
    let db = in_memory_db().await;
    let id = db.insert_todo("Unchanged").await.unwrap();

    // patch with no fields set — should not error
    db.patch_todo(id, &TodoPatch::new()).await.unwrap();

    let todos = db.list_todos().await.unwrap();
    assert_eq!(todos[0].title, "Unchanged");
}

#[tokio::test]
async fn delete_todo() {
    let db = in_memory_db().await;
    let id = db.insert_todo("To delete").await.unwrap();
    db.insert_todo("To keep").await.unwrap();

    db.delete_todo(id).await.unwrap();

    let todos = db.list_todos().await.unwrap();
    assert_eq!(todos.len(), 1);
    assert_eq!(todos[0].title, "To keep");
}

#[tokio::test]
async fn delete_done_todos() {
    let db = in_memory_db().await;
    let id1 = db.insert_todo("Done item").await.unwrap();
    db.insert_todo("Not done").await.unwrap();
    db.patch_todo(id1, &TodoPatch::new().done(true))
        .await
        .unwrap();

    db.delete_done_todos().await.unwrap();

    let todos = db.list_todos().await.unwrap();
    assert_eq!(todos.len(), 1);
    assert_eq!(todos[0].title, "Not done");
}

#[tokio::test]
async fn list_todos_ordered_by_sort_order() {
    let db = in_memory_db().await;
    let id1 = db.insert_todo("A").await.unwrap();
    let id2 = db.insert_todo("B").await.unwrap();
    let id3 = db.insert_todo("C").await.unwrap();

    // Reorder: C first, A second, B third
    db.patch_todo(id3, &TodoPatch::new().sort_order(0))
        .await
        .unwrap();
    db.patch_todo(id1, &TodoPatch::new().sort_order(1))
        .await
        .unwrap();
    db.patch_todo(id2, &TodoPatch::new().sort_order(2))
        .await
        .unwrap();

    let todos = db.list_todos().await.unwrap();
    assert_eq!(todos[0].id, id3);
    assert_eq!(todos[1].id, id1);
    assert_eq!(todos[2].id, id2);
}
