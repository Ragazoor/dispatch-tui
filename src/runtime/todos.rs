use super::*;

impl TuiRuntime {
    pub(super) async fn exec_load_todos(&self, app: &mut App) {
        match self.todo_svc.list_todos().await {
            Ok(todos) => {
                app.update(Message::Todo(crate::tui::messages::TodoMessage::Show(todos)));
            }
            Err(e) => tracing::warn!("failed to load todos: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::db::Database;
    use crate::tui::ViewMode;
    #[cfg(test)]
    use crate::service::embeddings::EmbeddingService;

    fn make_runtime(db: std::sync::Arc<Database>) -> TuiRuntime {
        let (tx, _rx) = mpsc::unbounded_channel();
        let (feed_tx, _) = mpsc::unbounded_channel();
        let db_arc: std::sync::Arc<dyn crate::db::TaskStore> = db.clone();
        let runner: std::sync::Arc<dyn crate::process::ProcessRunner> =
            std::sync::Arc::new(crate::process::MockProcessRunner::new(vec![]));
        TuiRuntime {
            task_svc: std::sync::Arc::new(crate::service::TaskService::new(db_arc.clone())),
            epic_svc: std::sync::Arc::new(crate::service::EpicService::new(db_arc.clone())),
            todo_svc: std::sync::Arc::new(crate::service::TodoService::new(db.clone())),
            feed_runner: Some(crate::feed::FeedRunner::new(
                db_arc.clone(),
                feed_tx,
                runner.clone(),
            )),
            feed_invalidate_tx: None,
            database: db_arc,
            msg_tx: tx,
            runner,
            editor_session: std::sync::Arc::new(std::sync::Mutex::new(None)),
            emb_svc: EmbeddingService::new_noop(),
        }
    }

    #[tokio::test]
    async fn todo_open_is_two_phase_load_then_show() {
        let db = std::sync::Arc::new(Database::open_in_memory().await.unwrap());
        // Insert a todo via the service
        let todo_svc = crate::service::TodoService::new(db.clone());
        todo_svc.create_todo("Write tests".to_string()).await.unwrap();

        let rt = make_runtime(db.clone());
        let mut app = App::new(vec![]);

        // Simulate the two-phase open: Open -> Load command -> exec_load_todos -> Show
        // Phase 1: Open emits Load command (tested in tui::tests::todos::open_returns_load_command)
        // Phase 2: exec_load_todos directly applies Show to app
        rt.exec_load_todos(&mut app).await;

        // After exec_load_todos, app should be in Todos view mode with the loaded item
        assert!(
            matches!(app.view_mode(), ViewMode::Todos { todos, .. } if todos.len() == 1),
            "expected Todos view with 1 todo, got {:?}",
            app.view_mode()
        );
    }

    #[tokio::test]
    async fn q_restores_previous_view() {
        // This test verifies the Close message restores the prior view (Board).
        // The input routing (q -> Close) is tested via handle_key_todos at the TUI layer.
        let db = std::sync::Arc::new(Database::open_in_memory().await.unwrap());
        let rt = make_runtime(db.clone());
        let mut app = App::new(vec![]);

        // Open the todos view
        rt.exec_load_todos(&mut app).await;
        assert!(
            matches!(app.view_mode(), ViewMode::Todos { .. }),
            "expected Todos view after load"
        );

        // Close should restore the board view
        app.update(Message::Todo(crate::tui::messages::TodoMessage::Close));
        assert!(
            matches!(app.view_mode(), ViewMode::Board(_)),
            "expected Board view after close, got {:?}",
            app.view_mode()
        );
    }
}
