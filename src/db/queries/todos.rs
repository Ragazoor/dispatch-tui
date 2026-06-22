use anyhow::{Context, Result};
use rusqlite::params;

use crate::models::{Todo, TodoId};

use super::super::{Database, TodoPatch};
use super::parse_datetime;

const TODO_COLUMNS: &str = "id, title, done, sort_order, created_at";

fn row_to_todo(row: &rusqlite::Row<'_>) -> rusqlite::Result<Todo> {
    let created_str: String = row.get(4)?;
    Ok(Todo {
        id: TodoId(row.get::<_, i64>(0)?),
        title: row.get(1)?,
        done: row.get(2)?,
        sort_order: row.get(3)?,
        created_at: parse_datetime(&created_str)?,
    })
}

#[async_trait::async_trait]
impl super::super::TodoStore for Database {
    async fn list_todos(&self) -> Result<Vec<Todo>> {
        self.db_call(move |conn| {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {TODO_COLUMNS} FROM todos ORDER BY sort_order ASC"
                ))
                .context("Failed to prepare list_todos")?;
            let rows = stmt
                .query_map([], row_to_todo)
                .context("Failed to query todos")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("Failed to collect todos")?;
            Ok(rows)
        })
        .await
    }

    async fn insert_todo(&self, title: &str) -> Result<TodoId> {
        let title = title.to_owned();
        self.db_call(move |conn| {
            conn.execute(
                "INSERT INTO todos (title, sort_order)
                 VALUES (?1, COALESCE((SELECT MAX(sort_order) FROM todos), -1) + 1)",
                params![title],
            )
            .context("Failed to insert todo")?;
            Ok(TodoId(conn.last_insert_rowid()))
        })
        .await
    }

    async fn patch_todo(&self, id: TodoId, patch: &TodoPatch<'_>) -> Result<()> {
        if !patch.has_changes() {
            return Ok(());
        }
        let title = patch.title.map(|s| s.to_owned());
        let done = patch.done;
        let sort_order = patch.sort_order;

        self.db_call(move |conn| {
            let mut sets: Vec<String> = Vec::new();
            let mut bind: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

            if let Some(t) = title {
                sets.push(format!("title = ?{}", bind.len() + 1));
                bind.push(Box::new(t));
            }
            if let Some(d) = done {
                sets.push(format!("done = ?{}", bind.len() + 1));
                bind.push(Box::new(d));
            }
            if let Some(s) = sort_order {
                sets.push(format!("sort_order = ?{}", bind.len() + 1));
                bind.push(Box::new(s));
            }

            bind.push(Box::new(id.0));
            let sql = format!(
                "UPDATE todos SET {} WHERE id = ?{}",
                sets.join(", "),
                bind.len()
            );

            let params_refs: Vec<&dyn rusqlite::ToSql> = bind.iter().map(|b| b.as_ref()).collect();
            conn.execute(&sql, params_refs.as_slice())
                .context("Failed to patch todo")?;
            Ok(())
        })
        .await
    }

    async fn delete_todo(&self, id: TodoId) -> Result<()> {
        self.db_call(move |conn| {
            conn.execute("DELETE FROM todos WHERE id = ?1", params![id.0])
                .context("Failed to delete todo")?;
            Ok(())
        })
        .await
    }

    async fn delete_done_todos(&self) -> Result<()> {
        self.db_call(move |conn| {
            conn.execute("DELETE FROM todos WHERE done = 1", [])
                .context("Failed to delete done todos")?;
            Ok(())
        })
        .await
    }
}
