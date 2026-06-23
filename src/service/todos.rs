use std::sync::Arc;

use crate::db::{CreateTodoRow, TodoPatch, TodoStore};
use crate::models::{Todo, TodoId};

use super::ServiceError;

// ---------------------------------------------------------------------------
// TodoUpdate — service-layer mutation params
// ---------------------------------------------------------------------------

/// Partial update for a todo item. `None` fields are left unchanged.
#[derive(Debug, Clone, Default)]
pub struct TodoUpdate {
    pub title: Option<String>,
    pub done: Option<bool>,
    pub sort_order: Option<i64>,
}

// ---------------------------------------------------------------------------
// TodoService
// ---------------------------------------------------------------------------

pub struct TodoService {
    db: Arc<dyn TodoStore>,
}

impl TodoService {
    pub fn new(db: Arc<dyn TodoStore>) -> Self {
        Self { db }
    }

    pub async fn list_todos(&self) -> Result<Vec<Todo>, ServiceError> {
        self.db.list_todos().await.map_err(ServiceError::from)
    }

    pub async fn create_todo(&self, title: String) -> Result<Todo, ServiceError> {
        if title.trim().is_empty() {
            return Err(ServiceError::Validation("title must not be empty".into()));
        }
        let id = self
            .db
            .insert_todo(CreateTodoRow {
                title: &title,
                task_id: None,
                epic_id: None,
            })
            .await
            .map_err(ServiceError::from)?;
        let todos = self.db.list_todos().await.map_err(ServiceError::from)?;
        todos
            .into_iter()
            .find(|t| t.id == id)
            .ok_or_else(|| ServiceError::NotFound(format!("todo {id:?} not found after insert")))
    }

    pub async fn update_todo(&self, id: TodoId, update: TodoUpdate) -> Result<(), ServiceError> {
        let mut patch = TodoPatch::new();
        if let Some(ref title) = update.title {
            patch = patch.title(title.as_str());
        }
        if let Some(done) = update.done {
            patch = patch.done(done);
        }
        if let Some(sort_order) = update.sort_order {
            patch = patch.sort_order(sort_order);
        }
        if patch.has_changes() {
            self.db
                .patch_todo(id, &patch)
                .await
                .map_err(ServiceError::from)?;
        }
        Ok(())
    }

    pub async fn delete_todo(&self, id: TodoId) -> Result<(), ServiceError> {
        self.db.delete_todo(id).await.map_err(ServiceError::from)
    }

    pub async fn clear_done(&self) -> Result<(), ServiceError> {
        self.db
            .delete_done_todos()
            .await
            .map_err(ServiceError::from)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::sync::Arc;

    use crate::db::Database;
    use crate::models::TodoId;
    use crate::service::todos::{TodoService, TodoUpdate};

    async fn make_service() -> TodoService {
        let db = Database::open_in_memory().await.unwrap();
        TodoService::new(Arc::new(db))
    }

    #[tokio::test]
    async fn create_todo_returns_todo_with_correct_title() {
        let svc = make_service().await;
        let todo = svc.create_todo("Buy milk".into()).await.unwrap();
        assert_eq!(todo.title, "Buy milk");
        assert!(!todo.done);
    }

    #[tokio::test]
    async fn create_todo_empty_title_returns_validation_error() {
        let svc = make_service().await;
        let err = svc.create_todo("   ".into()).await.unwrap_err();
        assert!(
            matches!(err, crate::service::ServiceError::Validation(_)),
            "expected Validation error, got {err:?}"
        );
    }

    #[tokio::test]
    async fn list_todos_returns_all_created() {
        let svc = make_service().await;
        svc.create_todo("A".into()).await.unwrap();
        svc.create_todo("B".into()).await.unwrap();
        let todos = svc.list_todos().await.unwrap();
        assert_eq!(todos.len(), 2);
        assert_eq!(todos[0].title, "A");
        assert_eq!(todos[1].title, "B");
    }

    #[tokio::test]
    async fn update_todo_title() {
        let svc = make_service().await;
        let todo = svc.create_todo("Old title".into()).await.unwrap();
        svc.update_todo(
            todo.id,
            TodoUpdate {
                title: Some("New title".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let todos = svc.list_todos().await.unwrap();
        assert_eq!(todos[0].title, "New title");
    }

    #[tokio::test]
    async fn update_todo_done() {
        let svc = make_service().await;
        let todo = svc.create_todo("Check me".into()).await.unwrap();
        assert!(!todo.done);
        svc.update_todo(
            todo.id,
            TodoUpdate {
                done: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let todos = svc.list_todos().await.unwrap();
        assert!(todos[0].done);
    }

    #[tokio::test]
    async fn update_todo_noop_when_no_changes() {
        let svc = make_service().await;
        let todo = svc.create_todo("Stable".into()).await.unwrap();
        // Should not error even with empty update
        svc.update_todo(todo.id, TodoUpdate::default())
            .await
            .unwrap();
        let todos = svc.list_todos().await.unwrap();
        assert_eq!(todos[0].title, "Stable");
    }

    #[tokio::test]
    async fn delete_todo_removes_it() {
        let svc = make_service().await;
        let todo = svc.create_todo("Temporary".into()).await.unwrap();
        svc.delete_todo(todo.id).await.unwrap();
        let todos = svc.list_todos().await.unwrap();
        assert!(todos.is_empty());
    }

    #[tokio::test]
    async fn clear_done_removes_only_done_todos() {
        let svc = make_service().await;
        let a = svc.create_todo("Keep me".into()).await.unwrap();
        let b = svc.create_todo("Done item".into()).await.unwrap();
        svc.update_todo(
            b.id,
            TodoUpdate {
                done: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        svc.clear_done().await.unwrap();
        let todos = svc.list_todos().await.unwrap();
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].id, a.id);
    }

    #[tokio::test]
    async fn delete_nonexistent_todo_does_not_error() {
        let svc = make_service().await;
        // SQLite DELETE on a non-existent row is a no-op — no error expected
        svc.delete_todo(TodoId(9999)).await.unwrap();
    }
}
